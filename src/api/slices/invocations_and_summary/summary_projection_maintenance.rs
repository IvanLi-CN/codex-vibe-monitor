use super::*;
#[derive(Debug, Clone, Copy)]
pub(crate) enum SummarySnapshotTrigger {
    Cadence,
    Mutation,
}

pub(crate) fn summary_snapshot_trigger_marks_dirty(
    trigger: SummarySnapshotTrigger,
    has_owner: bool,
) -> bool {
    matches!(trigger, SummarySnapshotTrigger::Mutation)
        || (matches!(trigger, SummarySnapshotTrigger::Cadence) && has_owner)
}

pub(crate) async fn summary_snapshot_cadence_trigger(
    state: &AppState,
) -> Option<SummarySnapshotTrigger> {
    // Historical authority recovery has its own single-flight maintenance worker. Keep this
    // cadence focused on the availability-critical rolling Projection so V2 backfill cannot
    // hold current, rolling, or calendar selections past their freshness deadline.
    if !state.subscription_hub.has_summary_owner().await {
        return None;
    }
    let has_all_time_owner = state.subscription_hub.has_summary_all_time_owner().await;
    let needs_attention = state
        .subscription_hub
        .summary_projection()
        .await
        .is_none_or(|projection| projection.needs_cadence_refresh(has_all_time_owner));
    needs_attention.then_some(SummarySnapshotTrigger::Cadence)
}

pub(crate) fn spawn_summary_snapshot_maintenance(state: Arc<AppState>) {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let mut receiver = state.broadcaster.subscribe();
        let mut mutation_receiver = state.subscription_hub.runtime_mutation_bus().subscribe();
        let mut cadence = tokio::time::interval(SUMMARY_SNAPSHOT_REFRESH_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_refresh_attempt = None;
        let mut retry_not_before = None;
        let mut dirty = false;
        // The journal is a retained overlay rather than a per-refresh queue. Keep a local
        // identity set so a successful refresh counts each accepted terminal contribution once.
        let mut counted_summary_snapshot_contributions = state
            .subscription_hub
            .summary_delta_journal_unabsorbed_identities()
            .await;
        loop {
            let trigger_refresh = tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => summary_snapshot_cadence_trigger(state.as_ref()).await,
                payload = receiver.recv() => matches!(
                    payload,
                    Ok(BroadcastPayload::DashboardCurrentSlice { .. })
                        | Ok(BroadcastPayload::DashboardTerminalSlice { .. })
                        | Ok(BroadcastPayload::DashboardActivityLive { .. })
                        | Err(broadcast::error::RecvError::Lagged(_))
                ).then_some(SummarySnapshotTrigger::Mutation),
                mutation = mutation_receiver.recv() => summary_snapshot_runtime_mutation_is_dirty(mutation)
                    .then_some(SummarySnapshotTrigger::Mutation),
            };
            let Some(trigger) = trigger_refresh else {
                continue;
            };
            let has_owner = state.subscription_hub.has_summary_owner().await;
            // Mutations remain coalesced while idle so the next real consumer receives a
            // bounded off-request rebuild. A due cadence is meaningful only for a live owner;
            // otherwise it would renew a full SQLite/archive hydration indefinitely.
            dirty |= summary_snapshot_trigger_marks_dirty(trigger, has_owner);
            if !has_owner {
                continue;
            }
            if crate::maintenance_store::legacy_worker_should_skip("summary_snapshot").await {
                continue;
            }
            if !dirty {
                continue;
            }
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution("summary_snapshot")
            else {
                continue;
            };
            let now = Instant::now();
            if !summary_snapshot_refresh_is_due(last_refresh_attempt, retry_not_before, now) {
                continue;
            }
            // Merge mutation bursts before starting the expensive off-request baseline. A due
            // cadence already spent its full minimum interval and must start immediately to
            // retain the strict hot-read freshness budget.
            if matches!(trigger, SummarySnapshotTrigger::Mutation) {
                tokio::time::sleep(SUMMARY_SNAPSHOT_EVENT_DEBOUNCE).await;
            }
            while receiver.try_recv().is_ok() {}
            while mutation_receiver.try_recv().is_ok() {}
            let observation = crate::TaskExecutionObservation::begin(
                "summary_snapshot",
                &crate::maintenance_store::task_title_for_observation("summary_snapshot"),
                match trigger {
                    SummarySnapshotTrigger::Mutation => "event",
                    SummarySnapshotTrigger::Cadence => "interval",
                },
                crate::maintenance_store::task_execution_class("summary_snapshot"),
                "processing",
            );
            last_refresh_attempt = Some(Instant::now());
            let journal_before = state
                .subscription_hub
                .summary_delta_journal_unabsorbed_identities()
                .await;
            let refresh_result = crate::with_managed_task_observation(observation.clone(), async {
                tokio::select! {
                    biased;
                    _ = state.shutdown.cancelled() => None,
                    result = refresh_summary_snapshots(state.as_ref()) => Some(result),
                }
            })
            .await;
            let Some(refresh_result) = refresh_result else {
                return;
            };
            let status = match refresh_result {
                Ok(()) => {
                    let journal_after = state
                        .subscription_hub
                        .summary_delta_journal_unabsorbed_identities()
                        .await;
                    let contribution_count = journal_before
                        .difference(&counted_summary_snapshot_contributions)
                        .count();
                    counted_summary_snapshot_contributions
                        .retain(|identity| journal_after.contains(identity));
                    counted_summary_snapshot_contributions.extend(journal_before);
                    crate::record_managed_task_discovered_work(
                        &["summary_snapshot"],
                        i64::try_from(contribution_count).unwrap_or(i64::MAX),
                    );
                    crate::record_managed_task_processed_work(
                        &["summary_snapshot"],
                        i64::try_from(contribution_count).unwrap_or(i64::MAX),
                    );
                    dirty = false;
                    retry_not_before = None;
                    "success"
                }
                Err(error) => {
                    retry_not_before = Some(summary_snapshot_retry_not_before(Instant::now()));
                    warn!(
                        ?error,
                        retry_after_secs = SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF.as_secs(),
                        "summary snapshot maintenance failed; retaining last-good responses before a bounded retry"
                    );
                    "failed"
                }
            };
            observation.finish_with_status(status);
        }
    });
}

pub(crate) fn summary_snapshot_runtime_mutation_is_dirty(
    mutation: Result<SequencedRuntimeMutation, broadcast::error::RecvError>,
) -> bool {
    match mutation {
        Ok(event) => matches!(
            event.mutation,
            RuntimeMutation::Invocation(_) | RuntimeMutation::AttemptChanged { .. }
        ),
        Err(broadcast::error::RecvError::Lagged(_)) => true,
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

pub(crate) fn summary_snapshot_retry_not_before(failed_at: Instant) -> Instant {
    failed_at + SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF
}

pub(crate) fn summary_snapshot_refresh_is_due(
    last_refresh_attempt: Option<Instant>,
    retry_not_before: Option<Instant>,
    now: Instant,
) -> bool {
    last_refresh_attempt
        .is_none_or(|attempt| now.duration_since(attempt) >= SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL)
        && retry_not_before.is_none_or(|retry_at| now >= retry_at)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SummaryBuildRoute {
    Http,
    Topic,
    Background,
}

impl SummaryBuildRoute {
    pub(crate) fn telemetry_route(self) -> &'static str {
        match self {
            Self::Http => "summary_http",
            Self::Topic => "summary_topic",
            Self::Background => "summary_background",
        }
    }

    pub(crate) fn closed_window_delivery_mode(self) -> &'static str {
        match self {
            Self::Http => "http_closed",
            Self::Topic => "sse_topic",
            Self::Background => "background_snapshot",
        }
    }
}

pub(crate) fn summary_window_is_closed(window: &SummaryWindow) -> bool {
    matches!(window, SummaryWindow::Calendar(spec) if matches!(spec.as_str(), "yesterday" | "previous1d" | "previous24h"))
        || matches!(window, SummaryWindow::PreviousFullDays(_))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryRangeBuildTelemetry {
    pub(crate) route: SummaryBuildRoute,
    pub(crate) window_kind: &'static str,
    pub(crate) window_name: &'static str,
    pub(crate) closed_window: bool,
}

impl SummaryRangeBuildTelemetry {
    pub(crate) fn new(
        route: SummaryBuildRoute,
        window: &SummaryWindow,
        requested_window: Option<&str>,
    ) -> Self {
        let window_kind = match window {
            SummaryWindow::Duration(_) => "duration",
            SummaryWindow::Calendar(_) => "calendar",
            SummaryWindow::PreviousFullDays(_) => "previous_full_days",
            SummaryWindow::All => "all",
            SummaryWindow::Current(_) => "current",
        };
        let window_name = match window {
            SummaryWindow::Duration(_) => match requested_window {
                Some("7d") => "7d",
                Some("1d") | None => "1d",
                _ => "range",
            },
            SummaryWindow::Calendar(spec) => match spec.as_str() {
                "today" => "today",
                "yesterday" => "yesterday",
                _ => "range",
            },
            SummaryWindow::PreviousFullDays(7) => "previous7d",
            _ => "range",
        };
        Self {
            route,
            window_kind,
            window_name,
            closed_window: summary_window_is_closed(window),
        }
    }

    pub(crate) fn route_label(self) -> &'static str {
        self.route.telemetry_route()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UsageBreakdownBuildTelemetry {
    pub(crate) route: &'static str,
    pub(crate) window_kind: &'static str,
    pub(crate) window_name: &'static str,
    pub(crate) closed_window: bool,
}

impl UsageBreakdownBuildTelemetry {
    pub(crate) fn from_summary(telemetry: SummaryRangeBuildTelemetry) -> Self {
        Self {
            route: telemetry.route_label(),
            window_kind: telemetry.window_kind,
            window_name: telemetry.window_name,
            closed_window: telemetry.closed_window,
        }
    }

    pub(crate) fn from_range_route(route: &'static str, range_name: &str) -> Self {
        let (window_kind, window_name, closed_window) = match range_name {
            "today" => ("calendar", "today", false),
            "yesterday" => ("calendar", "yesterday", true),
            "1d" => ("duration", "1d", false),
            "7d" => ("duration", "7d", false),
            "previous7d" => ("previous_full_days", "previous7d", true),
            _ => ("range", "range", false),
        };
        Self {
            route,
            window_kind,
            window_name,
            closed_window,
        }
    }
}

pub(crate) fn emit_summary_range_builder_telemetry(
    telemetry: SummaryRangeBuildTelemetry,
    builder: &'static str,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    aggregation_mode: &'static str,
    archive_overlap_strategy: &'static str,
    live_group_row_count: usize,
    archive_group_row_count: usize,
    live_row_scan_mode: &'static str,
    live_id_overlap_scan_mode: &'static str,
    elapsed_ms: u64,
) {
    let route = telemetry.route_label();
    let group_row_count = live_group_row_count + archive_group_row_count;
    if elapsed_ms >= 250 {
        tracing::warn!(
            route,
            builder,
            ?source_scope,
            upstream_account_id,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            closed_window = telemetry.closed_window,
            aggregation_mode,
            archive_overlap_strategy,
            live_group_row_count,
            archive_group_row_count,
            group_row_count,
            live_row_scan_mode,
            live_id_overlap_scan_mode,
            elapsed_ms,
            "summary range builder exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route,
            builder,
            ?source_scope,
            upstream_account_id,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            closed_window = telemetry.closed_window,
            aggregation_mode,
            archive_overlap_strategy,
            live_group_row_count,
            archive_group_row_count,
            group_row_count,
            live_row_scan_mode,
            live_id_overlap_scan_mode,
            elapsed_ms,
            "summary range builder completed"
        );
    }
}

pub(crate) fn emit_usage_breakdown_builder_telemetry(
    telemetry: UsageBreakdownBuildTelemetry,
    source_scope: InvocationSourceScope,
    full_hour_bucket_count: usize,
    rollup_row_count: usize,
    partial_hour_row_count: usize,
    archive_batch_count: usize,
    fallback_reason: &'static str,
    legacy_pruned_payload_mode: &'static str,
    elapsed_ms: u64,
) {
    let builder = if archive_batch_count > 0 {
        "usage_breakdown_archive_fallback"
    } else if full_hour_bucket_count == 0 {
        "usage_breakdown_exact_tail"
    } else {
        "usage_breakdown_rollup"
    };
    let archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN;
    let fallback_batch_count_after_rollup = archive_batch_count;
    if elapsed_ms >= 250 {
        tracing::warn!(
            route = telemetry.route,
            builder,
            ?source_scope,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            closed_window = telemetry.closed_window,
            full_hour_bucket_count,
            rollup_row_count,
            partial_hour_row_count,
            archive_batch_count,
            fallback_reason,
            legacy_pruned_payload_mode,
            archive_replay_target,
            fallback_batch_count_after_rollup,
            elapsed_ms,
            "usage breakdown builder exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route = telemetry.route,
            builder,
            ?source_scope,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            closed_window = telemetry.closed_window,
            full_hour_bucket_count,
            rollup_row_count,
            partial_hour_row_count,
            archive_batch_count,
            fallback_reason,
            legacy_pruned_payload_mode,
            archive_replay_target,
            fallback_batch_count_after_rollup,
            elapsed_ms,
            "usage breakdown builder completed"
        );
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SummaryLiveAugmentation {
    pub(crate) in_progress_conversation_count: Option<i64>,
    pub(crate) in_progress_retry_conversation_count: Option<i64>,
    pub(crate) in_progress_avg_wait_ms: Option<f64>,
    pub(crate) in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    pub(crate) non_success_tokens: Option<i64>,
}

#[cfg(test)]
impl SummaryLiveAugmentation {
    pub(crate) fn test_fixture() -> Self {
        Self {
            in_progress_conversation_count: Some(2),
            in_progress_retry_conversation_count: Some(1),
            in_progress_avg_wait_ms: Some(12.5),
            in_progress_phase_counts: Some(InvocationPhaseCountsResponse {
                queued: 1,
                requesting: 1,
                responding: 0,
            }),
            non_success_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryLiveAugmentationPolicy {
    pub(crate) include_in_progress: bool,
    pub(crate) include_non_success_tokens: bool,
}

pub(crate) fn summary_window_range(
    window: &SummaryWindow,
    reporting_tz: Tz,
    now: DateTime<Utc>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, ApiError> {
    match window {
        SummaryWindow::All | SummaryWindow::Current(_) => Ok(None),
        SummaryWindow::Duration(duration) => Ok(Some((now - *duration, now))),
        SummaryWindow::Calendar(spec) => {
            let range =
                resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            Ok(Some((range.start, range.end)))
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(*day_count, now, reporting_tz)
                .ok_or_else(|| {
                    ApiError::bad_request(anyhow!("invalid previous full days window"))
                })?;
            Ok(Some((start, end)))
        }
    }
}

pub(crate) async fn load_summary_live_augmentation(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    policy: SummaryLiveAugmentationPolicy,
    range_telemetry: Option<SummaryRangeBuildTelemetry>,
) -> Result<SummaryLiveAugmentation, ApiError> {
    let in_progress = if policy.include_in_progress {
        let snapshot =
            load_in_progress_summary_snapshot(state, source_scope, upstream_account_id).await?;
        (
            Some(snapshot.in_progress_count),
            Some(snapshot.retry_count),
            snapshot.avg_wait_ms,
            Some(snapshot.phase_counts),
        )
    } else {
        (None, None, None, None)
    };
    let non_success_tokens = if policy.include_non_success_tokens {
        if let Some((start, end)) = range {
            let telemetry = range_telemetry.ok_or_else(|| {
                ApiError::from(anyhow!(
                    "summary range telemetry is required for non-success token augmentation"
                ))
            })?;
            load_non_success_tokens_snapshot(
                state,
                source_scope,
                upstream_account_id,
                ExactUtcRange { start, end },
                telemetry,
            )
            .await?
        } else {
            None
        }
    } else {
        None
    };

    Ok(SummaryLiveAugmentation {
        in_progress_conversation_count: in_progress.0,
        in_progress_retry_conversation_count: in_progress.1,
        in_progress_avg_wait_ms: in_progress.2,
        in_progress_phase_counts: in_progress.3,
        non_success_tokens,
    })
}

pub(crate) fn apply_summary_live_augmentation(
    response: &mut StatsResponse,
    augmentation: SummaryLiveAugmentation,
) {
    response.in_progress_conversation_count = augmentation.in_progress_conversation_count;
    response.in_progress_retry_conversation_count =
        augmentation.in_progress_retry_conversation_count;
    response.in_progress_avg_wait_ms = augmentation.in_progress_avg_wait_ms;
    response.in_progress_phase_counts = augmentation.in_progress_phase_counts;
    response.non_success_tokens = augmentation.non_success_tokens;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InProgressSummarySnapshot {
    pub(crate) in_progress_count: i64,
    pub(crate) retry_count: i64,
    pub(crate) avg_wait_ms: Option<f64>,
    pub(crate) phase_counts: InvocationPhaseCountsResponse,
}

pub(crate) async fn load_in_progress_summary_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<InProgressSummarySnapshot, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: Option<i64>,
        retry_count: i64,
        upstream_ttfb_ms: Option<f64>,
        live_phase: Option<String>,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = if upstream_account_id.is_some() {
        invocation_account_retry_after_failure_with_attempt_fallback_sql(
            resolved_upstream_account_id_sql.as_str(),
            source_scope,
        )
    } else {
        match source_scope {
            InvocationSourceScope::All => "live.is_retry_after_failure_all".to_string(),
            InvocationSourceScope::ProxyOnly => {
                "live.is_retry_after_failure_proxy_only".to_string()
            }
        }
    };
    let final_first_token_timing_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
    let final_live_phase_sql =
        invocation_live_phase_sql_with_timing_sql("inv", final_first_token_timing_sql.as_str());
    let live_ttfb_nonnegative_sql = sqlite_nonnegative_timing_sql("live.upstream_ttfb_ms");
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            COALESCE(COUNT(*), 0) AS in_progress_count, \
            COALESCE(SUM(",
    );
    query.push(retry_sql.as_str()).push(
        format!(
            "), 0) AS retry_count, \
         AVG(CASE WHEN {live_ttfb_nonnegative_sql} THEN live.upstream_ttfb_ms END) AS avg_wait_ms, \
         COUNT(CASE WHEN {live_ttfb_nonnegative_sql} THEN 1 END) AS avg_wait_sample_count \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
        )
        .as_str(),
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND live.source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str())
            .push(" = ")
            .push_bind(upstream_account_id);
    }

    let (mut db_in_progress_count, mut retry_count, avg_wait_ms, mut avg_wait_sample_count) = query
        .build_query_as::<(i64, i64, Option<f64>, i64)>()
        .fetch_one(&state.pool)
        .await?;
    let mut db_key_query = QueryBuilder::<Sqlite>::new(
        "SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ",
    );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(" AS retry_count, live.upstream_ttfb_ms AS upstream_ttfb_ms, ");
    db_key_query.push(final_live_phase_sql.as_str()).push(
        " AS live_phase \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        db_key_query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str())
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    let db_runtime_rows = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(&state.pool)
        .await?;
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    for row in &db_runtime_rows {
        phase_counts.increment_phase_name(row.live_phase.as_deref());
    }
    let runtime_snapshot = state.proxy_runtime_invocations.snapshot();
    let db_terminal_keys =
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_snapshot, None).await?;
    let runtime_by_key = runtime_snapshot
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let filter = InvocationRecordsFilters {
        upstream_account_id,
        ..InvocationRecordsFilters::default()
    };
    let filter_without_upstream_account = InvocationRecordsFilters {
        upstream_account_id: None,
        ..filter.clone()
    };
    let mut db_ttfb_sum = summary_live_sum_from_average(avg_wait_ms, avg_wait_sample_count)?;
    for row in &db_runtime_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        let Some(runtime_record) = runtime_by_key.get(&key) else {
            continue;
        };
        let account_matches = upstream_account_id
            .map(|account_id| row.upstream_account_id == Some(account_id))
            .unwrap_or(true);
        if account_matches
            && runtime_in_flight_record_matches_filters(
                runtime_record,
                &filter_without_upstream_account,
                source_scope,
            )
        {
            let runtime_phase = runtime_record_live_phase(runtime_record);
            if runtime_phase != row.live_phase.as_deref() {
                phase_counts.decrement_phase_name(row.live_phase.as_deref());
                phase_counts.increment_phase_name(runtime_phase);
            }
            if runtime_record_ttfb_differs(runtime_record.t_upstream_ttfb_ms, row.upstream_ttfb_ms)
            {
                checked_summary_live_subtract(
                    &mut db_ttfb_sum,
                    normalized_wait_ms(row.upstream_ttfb_ms).unwrap_or_default(),
                    "runtime replacement wait sum",
                )?;
                checked_summary_live_add(
                    &mut db_ttfb_sum,
                    normalized_wait_ms(runtime_record.t_upstream_ttfb_ms).unwrap_or_default(),
                    "runtime replacement wait sum",
                )?;
                avg_wait_sample_count =
                    avg_wait_sample_count.saturating_sub(i64::from(
                        normalized_wait_ms(row.upstream_ttfb_ms).is_some(),
                    )) + i64::from(normalized_wait_ms(runtime_record.t_upstream_ttfb_ms).is_some());
            }
            continue;
        }
        db_in_progress_count = db_in_progress_count.saturating_sub(1);
        phase_counts.decrement_phase_name(row.live_phase.as_deref());
        if row.retry_count > 0 {
            retry_count = retry_count.saturating_sub(1);
        }
        if let Some(ttfb_ms) = row
            .upstream_ttfb_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            checked_summary_live_subtract(&mut db_ttfb_sum, ttfb_ms, "runtime wait sum")?;
            avg_wait_sample_count = avg_wait_sample_count.saturating_sub(1);
        }
    }
    let db_runtime_keys = db_runtime_rows
        .into_iter()
        .map(|row| ((row.invoke_id, row.occurred_at), row.retry_count > 0))
        .collect::<HashMap<_, _>>();
    let runtime_records = runtime_snapshot
        .into_iter()
        .filter(|record| {
            !db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter(|record| runtime_in_flight_record_matches_filters(record, &filter, source_scope))
        .collect::<Vec<_>>();
    let runtime_in_progress_count = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .count() as i64;
    let runtime_retry_count = runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            runtime_record_is_retry(record) && !db_runtime_keys.get(&key).copied().unwrap_or(false)
        })
        .count() as i64;
    let runtime_phase_counts = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .fold(
            InvocationPhaseCountsResponse::default(),
            |mut counts, record| {
                counts.increment_phase_name(runtime_record_live_phase(record));
                counts
            },
        );
    let mut ttfb_sum = 0.0;
    let mut ttfb_count = 0_i64;
    for value in runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter_map(|record| {
            record
                .t_upstream_ttfb_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        })
    {
        checked_summary_live_add(&mut ttfb_sum, value, "runtime wait sum")?;
        ttfb_count = ttfb_count
            .checked_add(1)
            .ok_or_else(|| summary_live_numeric_overflow("runtime wait sample count"))?;
    }
    let combined_avg_wait_ms = match (avg_wait_sample_count, ttfb_count) {
        (db_sample_count, runtime_count) if db_sample_count > 0 || runtime_count > 0 => {
            let sample_count = db_sample_count
                .checked_add(runtime_count)
                .ok_or_else(|| summary_live_numeric_overflow("combined wait sample count"))?;
            let mut total_sum = db_ttfb_sum;
            checked_summary_live_add(&mut total_sum, ttfb_sum, "combined wait sum")?;
            let average = total_sum / sample_count as f64;
            if !average.is_finite() || average < 0.0 {
                return Err(summary_live_numeric_overflow("combined wait average"));
            }
            Some(average)
        }
        _ => None,
    };
    if runtime_in_progress_count > 0 {
        debug!(
            endpoint = "/api/stats/summary",
            runtime_overlay_row_count = runtime_in_progress_count,
            upstream_account_id,
            "overlayed memory runtime in-progress records into summary live augmentation"
        );
    }
    phase_counts.queued += runtime_phase_counts.queued;
    phase_counts.requesting += runtime_phase_counts.requesting;
    phase_counts.responding += runtime_phase_counts.responding;
    Ok(InProgressSummarySnapshot {
        in_progress_count: db_in_progress_count + runtime_in_progress_count,
        retry_count: retry_count + runtime_retry_count,
        avg_wait_ms: combined_avg_wait_ms,
        phase_counts,
    })
}

pub(crate) async fn load_live_invocation_ids_in_range(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<std::collections::HashSet<i64>, ApiError> {
    #[derive(Debug, FromRow)]
    struct IdRow {
        id: i64,
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE occurred_at >= ");
    query
        .push_bind(db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(end));

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(
                crate::api::invocation_upstream_account_id_with_attempt_fallback_sql(
                    "codex_invocations",
                ),
            )
            .push(" = ")
            .push_bind(upstream_account_id);
    }

    Ok(query
        .build_query_as::<IdRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

pub(crate) fn summary_live_augmentation_policy(
    window: &SummaryWindow,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    now: DateTime<Utc>,
) -> SummaryLiveAugmentationPolicy {
    let closed_named_window = matches!(
        window,
        SummaryWindow::Calendar(_) | SummaryWindow::PreviousFullDays(_)
    ) && range.is_some_and(|(_, end)| end < now);

    SummaryLiveAugmentationPolicy {
        include_in_progress: !closed_named_window,
        include_non_success_tokens: !closed_named_window && range.is_some(),
    }
}

pub(crate) async fn query_account_activity_v2_non_success_tokens(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    bucket_epochs: &[i64],
) -> Result<i64, ApiError> {
    if bucket_epochs.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(SUM(COALESCE(activity_v2_non_success_tokens, 0)), 0) \
         FROM upstream_account_stats_hourly WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(*bucket_epoch);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(tx).await?)
}

pub(crate) async fn query_non_success_tokens_exact_tail(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
) -> Result<i64, ApiError> {
    let normalized_status_sql = INVOCATION_STATUS_NORMALIZED_SQL;
    let resolved_failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let non_success_sql = format!(
        "({normalized_status_sql} = 'interrupted' OR \
         ({normalized_status_sql} NOT IN ('running', 'pending') \
          AND ({resolved_failure_class_sql}) <> 'none'))"
    );
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) \
         FROM codex_invocations WHERE occurred_at >= "
    ));
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(invocation_upstream_account_id_with_attempt_fallback_sql(
                "codex_invocations",
            ))
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    Ok(query
        .build_query_scalar::<i64>()
        .fetch_one(executor)
        .await?)
}

pub(crate) async fn query_non_success_tokens_unreplayed_live_tail(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    covered_hours: &[i64],
    repair_cursor: i64,
    bucket_watermarks: &BTreeMap<i64, i64>,
) -> Result<i64, ApiError> {
    if covered_hours.is_empty() {
        return Ok(0);
    }
    let normalized_status_sql = INVOCATION_STATUS_NORMALIZED_SQL;
    let resolved_failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let non_success_sql = format!(
        "({normalized_status_sql} = 'interrupted' OR \
         ({normalized_status_sql} NOT IN ('running', 'pending') \
          AND ({resolved_failure_class_sql}) <> 'none'))"
    );
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) \
         FROM codex_invocations WHERE ("
    ));
    for (index, bucket_epoch) in covered_hours.iter().enumerate() {
        let start = Utc
            .timestamp_opt(*bucket_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid covered-hour start epoch")))?;
        let end = start + ChronoDuration::hours(1);
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(id > ")
            .push_bind(
                repair_cursor.max(
                    bucket_watermarks
                        .get(bucket_epoch)
                        .copied()
                        .unwrap_or_default(),
                ),
            )
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(end))
            .push(")");
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(invocation_upstream_account_id_with_attempt_fallback_sql(
                "codex_invocations",
            ))
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(tx).await?)
}

pub(crate) fn summary_non_success_tokens_lock_fallback(
    telemetry: SummaryRangeBuildTelemetry,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    stage: &'static str,
) -> Result<Option<i64>, ApiError> {
    tracing::warn!(
        route = telemetry.route_label(),
        builder = "summary_non_success_tokens",
        ?source_scope,
        upstream_account_id,
        window_kind = telemetry.window_kind,
        window_name = telemetry.window_name,
        aggregation_mode = "fallback",
        archive_overlap_strategy = "fallback",
        live_row_scan_mode = "fallback",
        live_id_overlap_scan_mode = "none",
        stage,
        start = %range.start,
        end = %range.end,
        "summary non-success token augmentation skipped because sqlite is locked"
    );
    Ok(None)
}

pub(crate) async fn load_non_success_tokens_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<Option<i64>, ApiError> {
    let started_at = Instant::now();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let live_range = ExactUtcRange {
        start: range.start.max(retention_cutoff),
        end: range.end,
    };
    let mut live_tokens = 0_i64;
    let mut live_group_row_count = 0_usize;
    let mut covered_hour_count = 0_usize;
    let mut boundary_tail_count = 0_usize;
    let mut fallback_hour_count = 0_usize;
    let mut live_row_scan_mode = "none";
    let historical_live_end = range.end.min(retention_cutoff);
    if range.start < historical_live_end {
        let historical_tokens = match query_non_success_tokens_exact_tail(
            &state.pool,
            source_scope,
            upstream_account_id,
            ExactUtcRange {
                start: range.start,
                end: historical_live_end,
            },
        )
        .await
        {
            Ok(tokens) => tokens,
            Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "historical_exact_tail",
                );
            }
            Err(err) => return Err(err),
        };
        live_tokens += historical_tokens;
        live_row_scan_mode = "fallback";
    }
    if live_range.start < live_range.end {
        let mut tx = match state.pool.begin().await {
            Ok(tx) => tx,
            Err(err) if crate::is_sqlite_lock_error(&anyhow!(err.to_string())) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "live_snapshot",
                );
            }
            Err(err) => return Err(err.into()),
        };
        let plan = match plan_account_activity_range_tx(state, tx.as_mut(), live_range).await {
            Ok(plan) => plan,
            Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "coverage_planner",
                );
            }
            Err(err) => return Err(err),
        };
        covered_hour_count = plan.covered_hours.len();
        fallback_hour_count = plan.uncovered_hours.len();
        boundary_tail_count = plan.boundary_tail_count;
        let rollup_tokens = match query_account_activity_v2_non_success_tokens(
            tx.as_mut(),
            source_scope,
            upstream_account_id,
            &plan.covered_hours,
        )
        .await
        {
            Ok(tokens) => tokens,
            Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "hourly_rollup",
                );
            }
            Err(err) => return Err(err),
        };
        live_tokens += rollup_tokens;
        live_group_row_count = plan.covered_hours.len();
        let repair_cursor = match load_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
        )
        .await
        {
            Ok(cursor) => cursor,
            Err(err) => {
                if crate::is_sqlite_lock_error(&err) {
                    return summary_non_success_tokens_lock_fallback(
                        telemetry,
                        source_scope,
                        upstream_account_id,
                        range,
                        "v2_repair_cursor",
                    );
                }
                return Err(err.into());
            }
        };
        let bucket_watermarks = match load_account_activity_v2_bucket_repair_watermarks(
            tx.as_mut(),
            &plan.covered_hours,
        )
        .await
        {
            Ok(watermarks) => watermarks,
            Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "v2_bucket_watermarks",
                );
            }
            Err(err) => return Err(err),
        };
        let unreplayed_live_tail = match query_non_success_tokens_unreplayed_live_tail(
            tx.as_mut(),
            source_scope,
            upstream_account_id,
            &plan.covered_hours,
            repair_cursor,
            &bucket_watermarks,
        )
        .await
        {
            Ok(tokens) => tokens,
            Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                return summary_non_success_tokens_lock_fallback(
                    telemetry,
                    source_scope,
                    upstream_account_id,
                    range,
                    "unreplayed_live_tail",
                );
            }
            Err(err) => return Err(err),
        };
        live_tokens += unreplayed_live_tail;
        if unreplayed_live_tail != 0 {
            tracing::debug!(
                route = telemetry.route_label(),
                builder = "summary_non_success_tokens",
                purpose = "unreplayed_live_tail",
                repair_cursor,
                covered_hour_count = plan.covered_hours.len(),
                "summary non-success token rollup included unreplayed live tail"
            );
        }
        for (index, exact_range) in plan.exact_ranges.into_iter().enumerate() {
            let exact_tokens = match query_non_success_tokens_exact_tail(
                tx.as_mut(),
                source_scope,
                upstream_account_id,
                exact_range,
            )
            .await
            {
                Ok(tokens) => tokens,
                Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
                    return summary_non_success_tokens_lock_fallback(
                        telemetry,
                        source_scope,
                        upstream_account_id,
                        range,
                        "exact_tail",
                    );
                }
                Err(err) => return Err(err),
            };
            live_tokens += exact_tokens;
            live_row_scan_mode = "fallback";
            if index >= boundary_tail_count {
                tracing::debug!(
                    route = telemetry.route_label(),
                    builder = "summary_non_success_tokens",
                    purpose = "coverage_hole",
                    source_scope = ?source_scope,
                    start = %exact_range.start,
                    end = %exact_range.end,
                    "summary non-success token exact fallback covered an hourly hole"
                );
            }
        }
        tx.commit().await?;
    }

    if range.start >= retention_cutoff {
        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        tracing::debug!(
            route = telemetry.route_label(),
            builder = "summary_non_success_tokens",
            ?source_scope,
            upstream_account_id,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            aggregation_mode = "rollup_plus_boundary",
            archive_overlap_strategy = "aggregate_merge",
            covered_hour_count,
            fallback_hour_count,
            boundary_tail_count,
            live_group_row_count,
            archive_group_row_count = 0_usize,
            live_row_scan_mode,
            live_id_overlap_scan_mode = "none",
            elapsed_ms,
            "summary non-success token rollup completed"
        );
        return Ok(Some(live_tokens));
    }

    let archive_rows = match query_completed_invocation_archive_activity_aggregate_rows(
        &state.pool,
        source_scope,
        range,
    )
    .await
    {
        Ok(rows) => rows,
        Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
            tracing::warn!(
                route = telemetry.route_label(),
                builder = "summary_non_success_tokens",
                ?source_scope,
                upstream_account_id,
                window_kind = telemetry.window_kind,
                window_name = telemetry.window_name,
                aggregation_mode = "fallback",
                archive_overlap_strategy = "fallback",
                live_group_row_count,
                archive_group_row_count = 0_usize,
                live_row_scan_mode,
                live_id_overlap_scan_mode = "none",
                start = %range.start,
                end = %range.end,
                "summary archive aggregate overlap merge skipped because sqlite is locked"
            );
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let archive_group_row_count = archive_rows.aggregates.len();
    if !archive_rows.skipped_materialized_ranges.is_empty() {
        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        emit_summary_range_builder_telemetry(
            telemetry,
            "summary_non_success_tokens",
            source_scope,
            upstream_account_id,
            "fallback",
            "fallback",
            live_group_row_count,
            archive_group_row_count,
            live_row_scan_mode,
            "none",
            elapsed_ms,
        );
        return Ok(None);
    }
    let archived_tokens = archive_rows
        .aggregates
        .into_iter()
        .filter(|row| {
            upstream_account_id.is_none_or(|account_id| row.upstream_account_id == Some(account_id))
        })
        .map(|row| row.non_success_tokens)
        .sum::<i64>();
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_summary_range_builder_telemetry(
        telemetry,
        "summary_non_success_tokens",
        source_scope,
        upstream_account_id,
        "rollup_plus_boundary",
        "aggregate_merge",
        live_group_row_count,
        archive_group_row_count,
        live_row_scan_mode,
        "none",
        elapsed_ms,
    );
    Ok(Some(live_tokens + archived_tokens))
}

pub(crate) async fn build_empty_summary_response(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<StatsResponse, ApiError> {
    let mut response = StatsResponse {
        total_count: 0,
        success_count: 0,
        failure_count: 0,
        total_cost: 0.0,
        total_tokens: 0,
        usage_breakdown: None,
        in_progress_conversation_count: None,
        in_progress_retry_conversation_count: None,
        in_progress_avg_wait_ms: None,
        in_progress_phase_counts: None,
        non_success_cost: Some(0.0),
        non_success_tokens: None,
        maintenance: Some(load_stats_maintenance_response(state).await?),
        data_quality: None,
    };
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        upstream_account_id,
        None,
        SummaryLiveAugmentationPolicy {
            include_in_progress: true,
            include_non_success_tokens: false,
        },
        None,
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    Ok(response)
}
