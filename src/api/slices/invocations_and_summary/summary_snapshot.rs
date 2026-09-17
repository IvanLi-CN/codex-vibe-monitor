#[derive(Debug, Clone, Copy)]
enum SummarySnapshotTrigger {
    Cadence,
    Mutation,
}

fn summary_snapshot_trigger_marks_dirty(trigger: SummarySnapshotTrigger, has_owner: bool) -> bool {
    matches!(trigger, SummarySnapshotTrigger::Mutation)
        || (matches!(trigger, SummarySnapshotTrigger::Cadence) && has_owner)
}

async fn summary_snapshot_cadence_trigger(state: &AppState) -> Option<SummarySnapshotTrigger> {
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
    tokio::spawn(async move {
        let mut receiver = state.broadcaster.subscribe();
        let mut mutation_receiver = state.subscription_hub.runtime_mutation_bus().subscribe();
        let mut cadence = tokio::time::interval(SUMMARY_SNAPSHOT_REFRESH_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_refresh_attempt = None;
        let mut retry_not_before = None;
        let mut dirty = false;
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
            if !dirty {
                continue;
            }
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
            last_refresh_attempt = Some(Instant::now());
            match refresh_summary_snapshots(state.as_ref()).await {
                Ok(()) => {
                    dirty = false;
                    retry_not_before = None;
                }
                Err(error) => {
                    retry_not_before = Some(summary_snapshot_retry_not_before(Instant::now()));
                    warn!(
                        ?error,
                        retry_after_secs = SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF.as_secs(),
                        "summary snapshot maintenance failed; retaining last-good responses before a bounded retry"
                    );
                }
            }
        }
    });
}

fn summary_snapshot_runtime_mutation_is_dirty(
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

fn summary_snapshot_retry_not_before(failed_at: Instant) -> Instant {
    failed_at + SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF
}

fn summary_snapshot_refresh_is_due(
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
    fn telemetry_route(self) -> &'static str {
        match self {
            Self::Http => "summary_http",
            Self::Topic => "summary_topic",
            Self::Background => "summary_background",
        }
    }

    fn closed_window_delivery_mode(self) -> &'static str {
        match self {
            Self::Http => "http_closed",
            Self::Topic => "sse_topic",
            Self::Background => "background_snapshot",
        }
    }
}

fn summary_window_is_closed(window: &SummaryWindow) -> bool {
    matches!(window, SummaryWindow::Calendar(spec) if spec == "yesterday")
        || matches!(window, SummaryWindow::PreviousFullDays(_))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryRangeBuildTelemetry {
    route: SummaryBuildRoute,
    window_kind: &'static str,
    window_name: &'static str,
    closed_window: bool,
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

    fn route_label(self) -> &'static str {
        self.route.telemetry_route()
    }
}

#[derive(Debug, Clone, Copy)]
struct UsageBreakdownBuildTelemetry {
    route: &'static str,
    window_kind: &'static str,
    window_name: &'static str,
    closed_window: bool,
}

impl UsageBreakdownBuildTelemetry {
    fn from_summary(telemetry: SummaryRangeBuildTelemetry) -> Self {
        Self {
            route: telemetry.route_label(),
            window_kind: telemetry.window_kind,
            window_name: telemetry.window_name,
            closed_window: telemetry.closed_window,
        }
    }

    fn from_range_route(route: &'static str, range_name: &str) -> Self {
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

struct SummaryRangeBuilderTelemetryInput {
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
}

fn emit_summary_range_builder_telemetry(input: SummaryRangeBuilderTelemetryInput) {
    let SummaryRangeBuilderTelemetryInput {
        telemetry,
        builder,
        source_scope,
        upstream_account_id,
        aggregation_mode,
        archive_overlap_strategy,
        live_group_row_count,
        archive_group_row_count,
        live_row_scan_mode,
        live_id_overlap_scan_mode,
        elapsed_ms,
    } = input;
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

struct UsageBreakdownBuilderTelemetryInput {
    telemetry: UsageBreakdownBuildTelemetry,
    source_scope: InvocationSourceScope,
    full_hour_bucket_count: usize,
    rollup_row_count: usize,
    partial_hour_row_count: usize,
    archive_batch_count: usize,
    fallback_reason: &'static str,
    legacy_pruned_payload_mode: &'static str,
    elapsed_ms: u64,
}

fn emit_usage_breakdown_builder_telemetry(input: UsageBreakdownBuilderTelemetryInput) {
    let UsageBreakdownBuilderTelemetryInput {
        telemetry,
        source_scope,
        full_hour_bucket_count,
        rollup_row_count,
        partial_hour_row_count,
        archive_batch_count,
        fallback_reason,
        legacy_pruned_payload_mode,
        elapsed_ms,
    } = input;
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
    in_progress_conversation_count: Option<i64>,
    in_progress_retry_conversation_count: Option<i64>,
    in_progress_avg_wait_ms: Option<f64>,
    in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    non_success_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryLiveAugmentationPolicy {
    include_in_progress: bool,
    include_non_success_tokens: bool,
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
    in_progress_count: i64,
    retry_count: i64,
    avg_wait_ms: Option<f64>,
    phase_counts: InvocationPhaseCountsResponse,
}

#[derive(Debug, FromRow)]
struct InProgressRuntimeKeyRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    retry_count: i64,
    upstream_ttfb_ms: Option<f64>,
    live_phase: Option<String>,
}

struct InProgressDatabaseSnapshot {
    in_progress_count: i64,
    retry_count: i64,
    avg_wait_ms: Option<f64>,
    avg_wait_sample_count: i64,
    runtime_rows: Vec<InProgressRuntimeKeyRow>,
    phase_counts: InvocationPhaseCountsResponse,
}

async fn query_in_progress_summary_database(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<InProgressDatabaseSnapshot, ApiError> {
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
        "SELECT COALESCE(COUNT(*), 0) AS in_progress_count, COALESCE(SUM(",
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
    append_in_progress_source_filters(
        &mut query,
        source_scope,
        upstream_account_id,
        resolved_upstream_account_id_sql.as_str(),
    );
    let (in_progress_count, retry_count, avg_wait_ms, avg_wait_sample_count) = query
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
    append_in_progress_source_filters(
        &mut db_key_query,
        source_scope,
        upstream_account_id,
        resolved_upstream_account_id_sql.as_str(),
    );
    let runtime_rows = db_key_query
        .build_query_as::<InProgressRuntimeKeyRow>()
        .fetch_all(&state.pool)
        .await?;
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    for row in &runtime_rows {
        phase_counts.increment_phase_name(row.live_phase.as_deref());
    }
    Ok(InProgressDatabaseSnapshot {
        in_progress_count,
        retry_count,
        avg_wait_ms,
        avg_wait_sample_count,
        runtime_rows,
        phase_counts,
    })
}

fn append_in_progress_source_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    resolved_upstream_account_id_sql: &str,
) {
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND live.source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql)
            .push(" = ")
            .push_bind(upstream_account_id);
    }
}

struct ReconciledInProgressDatabase {
    in_progress_count: i64,
    retry_count: i64,
    ttfb_sum: f64,
    ttfb_sample_count: i64,
    phase_counts: InvocationPhaseCountsResponse,
    runtime_keys: HashMap<(String, String), bool>,
}

fn reconcile_in_progress_database_snapshot(
    snapshot: InProgressDatabaseSnapshot,
    runtime_by_key: &HashMap<(String, String), &ApiInvocation>,
    filter_without_upstream_account: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> ReconciledInProgressDatabase {
    let InProgressDatabaseSnapshot {
        mut in_progress_count,
        mut retry_count,
        avg_wait_ms,
        mut avg_wait_sample_count,
        runtime_rows,
        mut phase_counts,
    } = snapshot;
    let mut db_ttfb_sum = avg_wait_ms.unwrap_or_default() * avg_wait_sample_count as f64;
    for row in &runtime_rows {
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
                filter_without_upstream_account,
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
                db_ttfb_sum = (db_ttfb_sum
                    - normalized_wait_ms(row.upstream_ttfb_ms).unwrap_or_default())
                .max(0.0)
                    + normalized_wait_ms(runtime_record.t_upstream_ttfb_ms).unwrap_or_default();
                avg_wait_sample_count =
                    avg_wait_sample_count.saturating_sub(i64::from(
                        normalized_wait_ms(row.upstream_ttfb_ms).is_some(),
                    )) + i64::from(normalized_wait_ms(runtime_record.t_upstream_ttfb_ms).is_some());
            }
            continue;
        }
        in_progress_count = in_progress_count.saturating_sub(1);
        phase_counts.decrement_phase_name(row.live_phase.as_deref());
        if row.retry_count > 0 {
            retry_count = retry_count.saturating_sub(1);
        }
        if let Some(ttfb_ms) = row
            .upstream_ttfb_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            db_ttfb_sum -= ttfb_ms;
            avg_wait_sample_count = avg_wait_sample_count.saturating_sub(1);
        }
    }
    let runtime_keys = runtime_rows
        .into_iter()
        .map(|row| ((row.invoke_id, row.occurred_at), row.retry_count > 0))
        .collect();
    ReconciledInProgressDatabase {
        in_progress_count,
        retry_count,
        ttfb_sum: db_ttfb_sum,
        ttfb_sample_count: avg_wait_sample_count,
        phase_counts,
        runtime_keys,
    }
}

struct InProgressRuntimeOverlay {
    in_progress_count: i64,
    retry_count: i64,
    phase_counts: InvocationPhaseCountsResponse,
    ttfb_sum: f64,
    ttfb_count: i64,
}

fn build_in_progress_runtime_overlay(
    runtime_snapshot: Vec<ApiInvocation>,
    db_terminal_keys: &HashSet<(String, String)>,
    db_runtime_keys: &HashMap<(String, String), bool>,
    filter: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> InProgressRuntimeOverlay {
    let runtime_records = runtime_snapshot
        .into_iter()
        .filter(|record| {
            !db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter(|record| runtime_in_flight_record_matches_filters(record, filter, source_scope))
        .collect::<Vec<_>>();
    let new_records = runtime_records.iter().filter(|record| {
        !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
    });
    let in_progress_count = new_records.clone().count() as i64;
    let retry_count = new_records
        .clone()
        .filter(|record| runtime_record_is_retry(record))
        .count() as i64;
    let phase_counts = new_records.clone().fold(
        InvocationPhaseCountsResponse::default(),
        |mut counts, record| {
            counts.increment_phase_name(runtime_record_live_phase(record));
            counts
        },
    );
    let (ttfb_sum, ttfb_count) = new_records
        .filter_map(|record| {
            record
                .t_upstream_ttfb_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        })
        .fold((0.0, 0_i64), |(sum, count), value| (sum + value, count + 1));
    InProgressRuntimeOverlay {
        in_progress_count,
        retry_count,
        phase_counts,
        ttfb_sum,
        ttfb_count,
    }
}

pub(crate) async fn load_in_progress_summary_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<InProgressSummarySnapshot, ApiError> {
    let db_snapshot =
        query_in_progress_summary_database(state, source_scope, upstream_account_id).await?;
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
    let reconciled = reconcile_in_progress_database_snapshot(
        db_snapshot,
        &runtime_by_key,
        &filter_without_upstream_account,
        source_scope,
        upstream_account_id,
    );
    let db_ttfb_sum = reconciled.ttfb_sum;
    let avg_wait_sample_count = reconciled.ttfb_sample_count;
    let mut phase_counts = reconciled.phase_counts;
    let db_runtime_keys = reconciled.runtime_keys;
    let runtime_overlay = build_in_progress_runtime_overlay(
        runtime_snapshot,
        &db_terminal_keys,
        &db_runtime_keys,
        &filter,
        source_scope,
    );
    let combined_avg_wait_ms = match (avg_wait_sample_count, runtime_overlay.ttfb_count) {
        (db_sample_count, runtime_count) if db_sample_count > 0 || runtime_count > 0 => Some(
            (db_ttfb_sum + runtime_overlay.ttfb_sum) / (db_sample_count + runtime_count) as f64,
        ),
        _ => None,
    };
    if runtime_overlay.in_progress_count > 0 {
        debug!(
            endpoint = "/api/stats/summary",
            runtime_overlay_row_count = runtime_overlay.in_progress_count,
            upstream_account_id,
            "overlayed memory runtime in-progress records into summary live augmentation"
        );
    }
    phase_counts.queued += runtime_overlay.phase_counts.queued;
    phase_counts.requesting += runtime_overlay.phase_counts.requesting;
    phase_counts.responding += runtime_overlay.phase_counts.responding;
    Ok(InProgressSummarySnapshot {
        in_progress_count: reconciled.in_progress_count + runtime_overlay.in_progress_count,
        retry_count: reconciled.retry_count + runtime_overlay.retry_count,
        avg_wait_ms: combined_avg_wait_ms,
        phase_counts,
    })
}

async fn query_account_activity_v2_non_success_tokens(
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

async fn query_non_success_tokens_exact_tail(
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
            .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    Ok(query
        .build_query_scalar::<i64>()
        .fetch_one(executor)
        .await?)
}

async fn query_non_success_tokens_unreplayed_live_tail(
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
            .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(tx).await?)
}

fn summary_non_success_tokens_lock_fallback(
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
