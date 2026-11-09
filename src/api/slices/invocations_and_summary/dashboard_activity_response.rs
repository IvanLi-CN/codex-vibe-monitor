use super::*;
pub(crate) fn build_dashboard_activity_snapshot_selection(
    range: &str,
    exact_range: ExactUtcRange,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
) -> DashboardActivitySnapshotSelection {
    DashboardActivitySnapshotSelection {
        range: range.to_string(),
        range_anchor: dashboard_activity_snapshot_selection_anchor(
            range,
            exact_range,
            reporting_tz,
        ),
        time_zone: reporting_tz.to_string(),
        source_scope: dashboard_activity_source_scope_cache_key(source_scope).to_string(),
        recent_limit,
        include_accounts,
        include_recent,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivityTerminalPayload {
    pub(crate) summary: DashboardActivitySummaryResponse,
    pub(crate) accounts: Vec<DashboardActivityAccountResponse>,
}

pub(crate) async fn dashboard_activity_terminal_payload_from_memory(
    state: &AppState,
    selection: &DashboardActivitySnapshotSelection,
) -> Option<DashboardActivityTerminalPayload> {
    let cache = state.dashboard_activity_snapshot_cache.lock().await;
    let entry = cache.entries.get(selection)?;
    Some(DashboardActivityTerminalPayload {
        summary: entry.response.summary.clone(),
        accounts: entry.response.accounts.clone(),
    })
}

pub(crate) fn dashboard_activity_snapshot_selection_anchor(
    range: &str,
    exact_range: ExactUtcRange,
    reporting_tz: Tz,
) -> String {
    if parse_duration_spec(range).is_ok() {
        return "rolling".to_string();
    }
    exact_range
        .start
        .with_timezone(&reporting_tz)
        .format("%Y-%m-%d")
        .to_string()
}

pub(crate) fn dashboard_activity_snapshot_cache_ttl(_range_name: &str) -> Duration {
    Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS)
}

pub(crate) fn dashboard_activity_snapshot_cache_can_track_expiry(
    range_name: &str,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> bool {
    parse_duration_spec(range_name).is_err() || range.start >= retention_cutoff
}

pub(crate) fn dashboard_activity_entry_expiry_covers(
    entry: &DashboardActivitySnapshotCacheEntry,
    range: ExactUtcRange,
) -> bool {
    entry
        .expiry_covered_until
        .is_none_or(|covered_until| range.start <= covered_until)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardActivitySnapshotReuseMode {
    Current,
    LastGoodReconcileBackoff,
}

pub(crate) fn dashboard_activity_snapshot_reuse_mode(
    entry: &DashboardActivitySnapshotCacheEntry,
    range: ExactUtcRange,
    cache_ttl: Duration,
) -> Option<DashboardActivitySnapshotReuseMode> {
    let within_reconcile_deadline = entry.last_reconcile_attempted_at.elapsed() <= cache_ttl;
    if dashboard_activity_entry_expiry_covers(entry, range) {
        return within_reconcile_deadline.then_some(DashboardActivitySnapshotReuseMode::Current);
    }
    if !entry.last_reconcile_failed || !within_reconcile_deadline {
        return None;
    }
    Some(DashboardActivitySnapshotReuseMode::LastGoodReconcileBackoff)
}

pub(crate) fn mark_dashboard_activity_reconcile_failed(
    entry: &mut DashboardActivitySnapshotCacheEntry,
) {
    entry.last_reconcile_attempted_at = Instant::now();
    entry.last_reconcile_failed = true;
}

pub(crate) fn dashboard_activity_pressure_reconcile_deferred(
    entry: &DashboardActivitySnapshotCacheEntry,
) -> bool {
    entry.cached_at.elapsed() <= DASHBOARD_ACTIVITY_PRESSURE_RECONCILE_MAX_AGE
}

pub(crate) fn insert_dashboard_activity_expiry_delta(
    deltas: &mut VecDeque<DashboardActivityTerminalDelta>,
    estimated_bytes: &mut usize,
    expiry_covered_until: Option<DateTime<Utc>>,
    delta: &DashboardActivityTerminalDelta,
    occurred_at: DateTime<Utc>,
) -> Result<(), &'static str> {
    if expiry_covered_until.is_none_or(|covered_until| occurred_at >= covered_until) {
        return Ok(());
    }
    if deltas.len() >= DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS {
        return Err("expiry_count_limit");
    }
    if estimated_bytes.saturating_add(delta.estimated_bytes)
        > DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES
    {
        return Err("expiry_byte_limit");
    }
    let insert_at = deltas
        .iter()
        .position(|existing| {
            parse_to_utc_datetime(&existing.occurred_at)
                .is_some_and(|existing_at| existing_at > occurred_at)
        })
        .unwrap_or(deltas.len());
    deltas.insert(insert_at, delta.clone());
    *estimated_bytes = estimated_bytes.saturating_add(delta.estimated_bytes);
    Ok(())
}

pub(crate) async fn load_dashboard_activity_expiry_deltas(
    pool: &Pool<Sqlite>,
    selection: &DashboardActivitySnapshotSelection,
    range: ExactUtcRange,
    baseline_cursor: i64,
) -> Result<
    (
        VecDeque<DashboardActivityTerminalDelta>,
        Option<DateTime<Utc>>,
        usize,
        Option<&'static str>,
    ),
    ApiError,
> {
    if parse_duration_spec(&selection.range).is_err() {
        return Ok((VecDeque::new(), None, 0, None));
    }
    // Resolve the moving range immediately before the query. The resulting upper bound is also
    // stored on the cache entry, so time spent querying can only shorten reuse instead of leaving
    // an uncovered expiry interval after a slow baseline build.
    let expiry_covered_until = resolve_dashboard_activity_cached_range(
        &selection.range,
        selection
            .time_zone
            .parse::<Tz>()
            .map_err(|_| ApiError::from(anyhow!("invalid dashboard activity cache timezone")))?,
    )?
    .start
        + ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64);
    let mut query = build_invocation_select_query();
    query
        .push(" AND id <= ")
        .push_bind(baseline_cursor)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(expiry_covered_until))
        .push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    if selection.source_scope == "proxy_only" {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at ASC, id ASC");
    let mut rows = query.build_query_as::<ApiInvocation>().fetch(pool);
    let mut deltas = VecDeque::new();
    let mut estimated_bytes = 0usize;
    while let Some(record) = rows.try_next().await? {
        let delta = persisted_dashboard_activity_terminal_delta(&record);
        if deltas.len() >= DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS {
            return Ok((VecDeque::new(), None, 0, Some("expiry_count_limit")));
        }
        if estimated_bytes.saturating_add(delta.estimated_bytes)
            > DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES
        {
            return Ok((VecDeque::new(), None, 0, Some("expiry_byte_limit")));
        }
        estimated_bytes = estimated_bytes.saturating_add(delta.estimated_bytes);
        deltas.push_back(delta);
    }
    Ok((deltas, Some(expiry_covered_until), estimated_bytes, None))
}

pub(crate) fn resolve_dashboard_activity_exact_range(
    range_name: &str,
    reporting_tz: Tz,
) -> Result<ExactUtcRange, ApiError> {
    let range_window = resolve_range_window(range_name, reporting_tz).map_err(ApiError::from)?;
    Ok(ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    })
}

pub(crate) fn resolve_dashboard_activity_cached_range(
    range_name: &str,
    reporting_tz: Tz,
) -> Result<ExactUtcRange, ApiError> {
    resolve_dashboard_activity_exact_range(range_name, reporting_tz)
}

pub(crate) fn dashboard_activity_full_hour_exact_range(
    full_hour_range: Option<(i64, i64)>,
) -> Result<Option<ExactUtcRange>, ApiError> {
    let Some((start_epoch, end_epoch)) = full_hour_range else {
        return Ok(None);
    };
    Ok(Some(ExactUtcRange {
        start: Utc.timestamp_opt(start_epoch, 0).single().ok_or_else(|| {
            ApiError::from(anyhow!("invalid dashboard activity full-hour start epoch"))
        })?,
        end: Utc.timestamp_opt(end_epoch, 0).single().ok_or_else(|| {
            ApiError::from(anyhow!("invalid dashboard activity full-hour end epoch"))
        })?,
    }))
}

pub(crate) fn sort_dashboard_activity_accounts(accounts: &mut [DashboardActivityAccountResponse]) {
    accounts.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| {
                right
                    .recent_invocations
                    .first()
                    .map(|row| row.occurred_at.as_str())
                    .cmp(
                        &left
                            .recent_invocations
                            .first()
                            .map(|row| row.occurred_at.as_str()),
                    )
            })
            .then_with(|| {
                right
                    .upstream_account_id
                    .unwrap_or(i64::MIN)
                    .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
            })
    });
}

pub(crate) fn dashboard_activity_account_from_live(
    live_account: &DashboardActivityLiveAccount,
    meta: Option<&UpstreamAccountActivityMetaRow>,
    range: ExactUtcRange,
    current_snapshot: DashboardActivityCurrentSnapshot,
    model_performance_available: bool,
    effective_routing_rule: Option<crate::upstream_accounts::EffectiveRoutingRule>,
    recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
) -> DashboardActivityAccountResponse {
    let status_fields =
        meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
    let display_name_hint = recent_invocations
        .iter()
        .find_map(|invocation| {
            invocation
                .upstream_account_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| live_account.upstream_account_name.clone());
    let plan_type_hint = recent_invocations.iter().find_map(|invocation| {
        invocation
            .upstream_account_plan_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let display_name = live_account
        .upstream_account_id
        .map(|id| {
            resolve_upstream_account_activity_display_name(id, meta, display_name_hint.as_deref())
        })
        .unwrap_or_else(|| "未分配上游账号".to_string());

    DashboardActivityAccountResponse {
        account_key: live_account.account_key.clone(),
        upstream_account_id: live_account.upstream_account_id,
        display_name,
        is_unassigned: live_account.upstream_account_id.is_none(),
        latest_conversation_created_at: None,
        last_invocation_at: None,
        group_name: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.group_name.clone()),
        ),
        plan_type: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.plan_type.clone())
                .or(plan_type_hint),
        ),
        enabled: status_fields.as_ref().map(|fields| fields.enabled),
        display_status: status_fields
            .as_ref()
            .map(|fields| fields.display_status.clone()),
        enable_status: status_fields
            .as_ref()
            .map(|fields| fields.enable_status.clone()),
        work_status: status_fields
            .as_ref()
            .map(|fields| fields.work_status.clone()),
        health_status: status_fields
            .as_ref()
            .map(|fields| fields.health_status.clone()),
        sync_state: status_fields
            .as_ref()
            .map(|fields| fields.sync_state.clone()),
        last_error: status_fields
            .as_ref()
            .and_then(|fields| fields.last_error.clone()),
        last_action_reason_message: status_fields
            .as_ref()
            .and_then(|fields| fields.last_action_reason_message.clone()),
        request_count: live_account.in_progress_invocation_count.max(0),
        success_count: 0,
        failure_count: 0,
        non_success_count: 0,
        total_tokens: 0,
        success_tokens: 0,
        non_success_tokens: 0,
        failure_tokens: 0,
        failure_cost: 0.0,
        non_success_cost: 0.0,
        total_cost: 0.0,
        usage_breakdown: UsageBreakdownAccumulator::default().into_response(),
        model_performance: ModelPerformanceAccumulator::default()
            .into_response(range, model_performance_available),
        cache_hit_rate: None,
        tokens_per_minute: Some(current_snapshot.qualified_tokens.max(0) as f64),
        spend_rate: Some(current_snapshot.total_cost.max(0.0)),
        first_byte_avg_ms: None,
        first_response_byte_total_avg_ms: None,
        first_token_avg_ms: None,
        avg_total_ms: None,
        current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
        current_first_response_byte_total_avg_ms: current_snapshot
            .first_response_byte_total_avg_ms(),
        current_avg_total_ms: current_snapshot.avg_total_ms(),
        current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
        in_progress_invocation_count: Some(live_account.in_progress_invocation_count),
        in_progress_phase_counts: Some(live_account.in_progress_phase_counts),
        retry_invocation_count: Some(live_account.retry_invocation_count),
        upload_bytes_per_second: live_account.upload_bytes_per_second,
        download_bytes_per_second: live_account.download_bytes_per_second,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        effective_routing_rule,
        recent_invocations,
    }
}

pub(crate) fn merge_dashboard_activity_recent_invocations(
    mut recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    existing_recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    recent_limit: usize,
) -> Vec<PromptCacheConversationInvocationPreviewResponse> {
    recent_invocations.extend(existing_recent_invocations);
    let mut seen_keys = HashSet::with_capacity(recent_invocations.len());
    recent_invocations.retain(|invocation| {
        seen_keys.insert((invocation.invoke_id.clone(), invocation.occurred_at.clone()))
    });
    recent_invocations.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    recent_invocations.truncate(recent_limit);
    recent_invocations
}

pub(crate) async fn load_dashboard_activity_live_recent_invocations_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    recent_limit: usize,
) -> Result<HashMap<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>, ApiError> {
    let mut rows = query_live_upstream_account_activity_preview_rows_per_account_limit(
        &state.pool,
        source_scope,
        range,
        recent_limit,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard",
            builder: "live_recent_overlay",
            purpose: "bounded_per_account_recent",
        },
    )
    .await?;
    overlay_runtime_upstream_account_activity_preview_rows(state, &mut rows, source_scope, range);
    overlay_runtime_terminal_upstream_account_activity_preview_rows(
        state,
        &mut rows,
        source_scope,
        range,
    )
    .await?;

    let mut recent_invocations_by_account =
        HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
    for row in rows {
        recent_invocations_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(upstream_account_invocation_preview_from_row(row));
    }
    for recent_invocations in recent_invocations_by_account.values_mut() {
        *recent_invocations = merge_dashboard_activity_recent_invocations(
            std::mem::take(recent_invocations),
            Vec::new(),
            recent_limit,
        );
    }

    Ok(recent_invocations_by_account)
}

pub(crate) async fn overlay_dashboard_activity_live_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    live: DashboardActivityLiveSnapshot,
    request_range: ExactUtcRange,
    include_accounts: bool,
    include_recent: bool,
    recent_limit: usize,
) -> Result<(), ApiError> {
    snapshot.summary.stats.in_progress_conversation_count = Some(live.in_progress_invocation_count);
    snapshot.summary.stats.in_progress_retry_conversation_count = Some(live.retry_invocation_count);
    snapshot.summary.stats.in_progress_phase_counts = Some(live.in_progress_phase_counts);

    let current_snapshot_by_account = state
        .dashboard_network_speed_cache
        .snapshot_dashboard_activity_accounts(Utc::now());
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    snapshot.summary.tokens_per_minute =
        Some(current_snapshot_summary.qualified_tokens.max(0) as f64);
    snapshot.summary.spend_rate = Some(current_snapshot_summary.total_cost.max(0.0));
    snapshot.summary.current_first_response_byte_total_avg_ms = current_snapshot_summary
        .first_response_byte_total_avg_ms()
        .or(snapshot.summary.current_first_response_byte_total_avg_ms);
    snapshot.summary.current_first_token_avg_ms = current_snapshot_summary.first_token_avg_ms();
    snapshot.summary.current_avg_total_ms = current_snapshot_summary
        .avg_total_ms()
        .or(snapshot.summary.current_avg_total_ms);
    snapshot.summary.current_avg_response_ms = current_snapshot_summary
        .avg_response_duration_ms()
        .or(snapshot.summary.current_avg_response_ms);

    if !include_accounts {
        return Ok(());
    }

    let recent_source_scope = if include_recent {
        Some(resolve_default_source_scope(&state.pool).await?)
    } else {
        None
    };
    let mut refreshed_recent_invocations_by_account =
        if let Some(source_scope) = recent_source_scope {
            Some(
                load_dashboard_activity_live_recent_invocations_by_account(
                    state,
                    source_scope,
                    request_range,
                    recent_limit,
                )
                .await?,
            )
        } else {
            None
        };
    let live_accounts = live
        .accounts
        .into_iter()
        .map(|account| (account.account_key.clone(), account))
        .collect::<HashMap<_, _>>();
    let metadata_refresh_ids = snapshot
        .accounts
        .iter()
        .filter(|account| account.enabled.is_none() && account.upstream_account_id.is_some())
        .filter_map(|account| account.upstream_account_id)
        .collect::<Vec<_>>();
    let metadata_refresh = if metadata_refresh_ids.is_empty() {
        (HashMap::new(), HashMap::new())
    } else {
        (
            query_upstream_account_activity_meta(&state.pool, &metadata_refresh_ids).await?,
            crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                &state.pool,
                &metadata_refresh_ids,
            )
            .await?,
        )
    };
    for account in &mut snapshot.accounts {
        if let Some(account_id) = account.upstream_account_id
            && let Some(meta) = metadata_refresh.0.get(&account_id)
        {
            let status_fields = build_upstream_account_activity_status_fields(meta, Utc::now());
            account.display_name = resolve_upstream_account_activity_display_name(
                account_id,
                Some(meta),
                Some(account.display_name.as_str()),
            );
            account.group_name = normalize_trimmed_optional_string_local(meta.group_name.clone());
            account.plan_type = normalize_trimmed_optional_string_local(meta.plan_type.clone());
            account.enabled = Some(status_fields.enabled);
            account.display_status = Some(status_fields.display_status);
            account.enable_status = Some(status_fields.enable_status);
            account.work_status = Some(status_fields.work_status);
            account.health_status = Some(status_fields.health_status);
            account.sync_state = Some(status_fields.sync_state);
            account.last_error = status_fields.last_error;
            account.last_action_reason_message = status_fields.last_action_reason_message;
            account.effective_routing_rule = Some(
                metadata_refresh
                    .1
                    .get(&account_id)
                    .cloned()
                    .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule),
            );
        }
        let live_account = live_accounts.get(&account.account_key);
        account.in_progress_invocation_count =
            Some(live_account.map_or(0, |row| row.in_progress_invocation_count));
        account.in_progress_phase_counts = Some(
            live_account
                .map(|row| row.in_progress_phase_counts)
                .unwrap_or_default(),
        );
        account.retry_invocation_count =
            Some(live_account.map_or(0, |row| row.retry_invocation_count));
        if let Some(live_account) = live_account {
            account.request_count = account
                .request_count
                .max(live_account.in_progress_invocation_count.max(0));
        }
        account.upload_bytes_per_second =
            live_account.map_or(0.0, |row| row.upload_bytes_per_second);
        account.download_bytes_per_second =
            live_account.map_or(0.0, |row| row.download_bytes_per_second);
        let current_snapshot = current_snapshot_by_account
            .get(&account.upstream_account_id)
            .copied()
            .unwrap_or_default();
        account.tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
        account.spend_rate = Some(current_snapshot.total_cost.max(0.0));
        account.current_first_response_byte_total_avg_ms = current_snapshot
            .first_response_byte_total_avg_ms()
            .or(account.current_first_response_byte_total_avg_ms);
        account.current_first_token_avg_ms = current_snapshot.first_token_avg_ms();
        account.current_avg_total_ms = current_snapshot
            .avg_total_ms()
            .or(account.current_avg_total_ms);
        account.current_avg_response_ms = current_snapshot
            .avg_response_duration_ms()
            .or(account.current_avg_response_ms);
        if let Some(refreshed_recent_invocations_by_account) =
            refreshed_recent_invocations_by_account.as_mut()
            && let Some(recent_invocations) =
                refreshed_recent_invocations_by_account.remove(&account.upstream_account_id)
        {
            let existing_recent_invocations = std::mem::take(&mut account.recent_invocations);
            account.recent_invocations = merge_dashboard_activity_recent_invocations(
                recent_invocations,
                existing_recent_invocations,
                recent_limit,
            );
        }
    }

    let existing_account_keys = snapshot
        .accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    let missing_live_accounts = live_accounts
        .values()
        .filter(|account| !existing_account_keys.contains(&account.account_key))
        .collect::<Vec<_>>();

    if !missing_live_accounts.is_empty() {
        let missing_account_ids = missing_live_accounts
            .iter()
            .filter_map(|account| account.upstream_account_id)
            .collect::<Vec<_>>();
        let account_meta =
            query_upstream_account_activity_meta(&state.pool, &missing_account_ids).await?;
        let effective_routing_rules =
            crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                &state.pool,
                &missing_account_ids,
            )
            .await?;
        for live_account in missing_live_accounts {
            let meta = live_account
                .upstream_account_id
                .and_then(|id| account_meta.get(&id));
            let effective_routing_rule = live_account.upstream_account_id.map(|id| {
                effective_routing_rules
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
            });
            let current_snapshot = current_snapshot_by_account
                .get(&live_account.upstream_account_id)
                .copied()
                .unwrap_or_default();
            let recent_invocations = if let Some(refreshed_recent_invocations_by_account) =
                refreshed_recent_invocations_by_account.as_mut()
            {
                refreshed_recent_invocations_by_account
                    .remove(&live_account.upstream_account_id)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            snapshot.accounts.push(dashboard_activity_account_from_live(
                live_account,
                meta,
                request_range,
                current_snapshot,
                snapshot.summary.model_performance.available,
                effective_routing_rule,
                recent_invocations,
            ));
        }
        sort_dashboard_activity_accounts(&mut snapshot.accounts);
    }

    if let Some(refreshed_recent_invocations_by_account) = refreshed_recent_invocations_by_account {
        let existing_account_ids = snapshot
            .accounts
            .iter()
            .map(|account| account.upstream_account_id)
            .collect::<HashSet<_>>();
        let missing_terminal_account_ids = refreshed_recent_invocations_by_account
            .keys()
            .copied()
            .filter(|account_id| !existing_account_ids.contains(account_id))
            .collect::<Vec<_>>();

        if !missing_terminal_account_ids.is_empty() {
            let missing_terminal_account_meta_ids = missing_terminal_account_ids
                .iter()
                .filter_map(|account_id| *account_id)
                .collect::<Vec<_>>();
            let account_meta = if missing_terminal_account_meta_ids.is_empty() {
                HashMap::new()
            } else {
                query_upstream_account_activity_meta(
                    &state.pool,
                    &missing_terminal_account_meta_ids,
                )
                .await?
            };
            let effective_routing_rules = if missing_terminal_account_meta_ids.is_empty() {
                HashMap::new()
            } else {
                crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                    &state.pool,
                    &missing_terminal_account_meta_ids,
                )
                .await?
            };
            let mut refreshed_recent_invocations_by_account =
                refreshed_recent_invocations_by_account;
            for upstream_account_id in missing_terminal_account_ids {
                let recent_invocations = refreshed_recent_invocations_by_account
                    .remove(&upstream_account_id)
                    .unwrap_or_default();
                if recent_invocations.is_empty() {
                    continue;
                }

                let live_account = DashboardActivityLiveAccount {
                    account_key: upstream_account_id
                        .map(|id| format!("upstream:{id}"))
                        .unwrap_or_else(|| "unassigned".to_string()),
                    upstream_account_id,
                    upstream_account_name: None,
                    in_progress_invocation_count: 0,
                    in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                    retry_invocation_count: 0,
                    in_progress_wait_sum_ms: 0.0,
                    in_progress_wait_sample_count: 0,
                    upload_bytes_per_second: 0.0,
                    download_bytes_per_second: 0.0,
                    network_live_bucket: None,
                };
                let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
                let effective_routing_rule = upstream_account_id.map(|id| {
                    effective_routing_rules
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
                });
                let current_snapshot = current_snapshot_by_account
                    .get(&upstream_account_id)
                    .copied()
                    .unwrap_or_default();
                snapshot.accounts.push(dashboard_activity_account_from_live(
                    &live_account,
                    meta,
                    request_range,
                    current_snapshot,
                    snapshot.summary.model_performance.available,
                    effective_routing_rule,
                    recent_invocations,
                ));
            }
            sort_dashboard_activity_accounts(&mut snapshot.accounts);
        }
    }

    snapshot.summary = build_dashboard_activity_summary(
        &snapshot.accounts,
        true,
        current_snapshot_summary,
        snapshot.summary.current_first_response_byte_total_avg_ms,
        snapshot.summary.current_avg_total_ms,
        snapshot.summary.model_performance.clone(),
    );
    dashboard_activity_apply_materialized_archive_fallback_to_stats(
        &mut snapshot.summary.stats,
        snapshot.materialized_archive_fallback_totals,
    );
    if snapshot.materialized_archive_details_limited {
        dashboard_activity_clear_materialized_archive_detail_fields(&mut snapshot.summary.stats);
    }

    Ok(())
}

#[derive(Debug, FromRow)]
pub(crate) struct DashboardActivityRateUsageRow {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

pub(crate) fn validate_dashboard_activity_params(
    endpoint: &str,
    range: &str,
    recent_limit: Option<i64>,
) -> Result<usize, ApiError> {
    if !matches!(range, "today" | "yesterday" | "1d" | "7d") {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported {endpoint} range: {range}",
        )));
    }

    match recent_limit {
        Some(value) if !(1..=16).contains(&value) => Err(ApiError::bad_request(anyhow!(
            "recentLimit must be between 1 and 16"
        ))),
        Some(value) => Ok(value as usize),
        None => Ok(4),
    }
}

pub(crate) fn sum_optional_rates(
    accounts: &[DashboardActivityAccountResponse],
    value_of: impl Fn(&DashboardActivityAccountResponse) -> Option<f64>,
) -> Option<f64> {
    let mut saw_value = false;
    let mut total = 0.0;
    for account in accounts {
        if let Some(value) = value_of(account).filter(|value| value.is_finite()) {
            saw_value = true;
            total += value;
        }
    }
    saw_value.then_some(total)
}

pub(crate) async fn load_dashboard_activity_current_minute_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let current_window = dashboard_activity_last_complete_minute_window(range);
    let mut rows = query_live_upstream_account_activity_preview_rows_with_limit(
        &state.pool,
        source_scope,
        current_window,
        None,
        None,
        None,
        false,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard",
            builder: "current_minute",
            purpose: "current_minute_preview_rows",
        },
    )
    .await?;
    let mut rows_by_key = rows
        .drain(..)
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<HashMap<_, _>>();
    let mut unresolved_runtime_account_keys = HashSet::new();

    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(mut row) =
            runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, true)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at < current_window.start || occurred_at >= current_window.end {
            continue;
        }
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows_by_key.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = existing.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = existing.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
            }
        }
        if row.upstream_account_id.is_none() {
            unresolved_runtime_account_keys.insert(key.clone());
        }
        rows_by_key.insert(key, row);
    }

    if !unresolved_runtime_account_keys.is_empty() {
        for fallback_row in query_runtime_recent_account_fallback_rows(
            &state.pool,
            source_scope,
            &unresolved_runtime_account_keys,
        )
        .await?
        {
            let key = (
                fallback_row.invoke_id.clone(),
                fallback_row.occurred_at.clone(),
            );
            if let Some(row) = rows_by_key.get_mut(&key) {
                if row.upstream_account_id.is_none() {
                    row.upstream_account_id = fallback_row.upstream_account_id;
                }
                if row.upstream_account_name.is_none() {
                    row.upstream_account_name = fallback_row.upstream_account_name;
                }
                if row.upstream_account_plan_type.is_none() {
                    row.upstream_account_plan_type = fallback_row.upstream_account_plan_type;
                }
            }
        }
    }

    Ok(rows_by_key.into_values().collect())
}

pub(crate) async fn load_dashboard_activity_current_minute_accumulators_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, DashboardActivityCurrentMinuteAccumulator>, ApiError> {
    let mut accumulators = HashMap::<Option<i64>, DashboardActivityCurrentMinuteAccumulator>::new();
    for row in load_dashboard_activity_current_minute_rows(state, source_scope, range).await? {
        accumulators
            .entry(row.upstream_account_id)
            .or_default()
            .add_row(&row);
    }
    Ok(accumulators)
}

pub(crate) fn sum_dashboard_activity_current_minute_accumulators(
    accumulators: impl IntoIterator<Item = DashboardActivityCurrentMinuteAccumulator>,
) -> DashboardActivityCurrentMinuteAccumulator {
    let mut total = DashboardActivityCurrentMinuteAccumulator::default();
    for accumulator in accumulators {
        total.merge(accumulator);
    }
    total
}

pub(crate) fn sum_dashboard_activity_current_snapshots(
    snapshots: impl IntoIterator<Item = DashboardActivityCurrentSnapshot>,
) -> DashboardActivityCurrentSnapshot {
    let mut total = DashboardActivityCurrentSnapshot::default();
    for snapshot in snapshots {
        total.add_assign(snapshot);
    }
    total
}

pub(crate) fn build_dashboard_activity_summary(
    accounts: &[DashboardActivityAccountResponse],
    include_live_counts: bool,
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
    model_performance: ModelPerformanceResponse,
) -> DashboardActivitySummaryResponse {
    let ttfb_sum = accounts
        .iter()
        .map(|account| account.in_progress_wait_sum_ms)
        .sum::<f64>();
    let ttfb_count = accounts
        .iter()
        .map(|account| account.in_progress_wait_sample_count)
        .sum::<i64>();

    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    for account in accounts {
        usage_breakdown.merge_response(&account.usage_breakdown);
    }
    let stats = StatsResponse {
        total_count: accounts.iter().map(|account| account.request_count).sum(),
        success_count: accounts.iter().map(|account| account.success_count).sum(),
        failure_count: accounts.iter().map(|account| account.failure_count).sum(),
        total_cost: accounts.iter().map(|account| account.total_cost).sum(),
        total_tokens: accounts.iter().map(|account| account.total_tokens).sum(),
        usage_breakdown: Some(usage_breakdown.into_response()),
        in_progress_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_retry_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.retry_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_avg_wait_ms: (ttfb_count > 0).then_some(ttfb_sum / ttfb_count as f64),
        in_progress_phase_counts: include_live_counts.then(|| {
            accounts.iter().fold(
                InvocationPhaseCountsResponse::default(),
                |mut total, account| {
                    if let Some(counts) = account.in_progress_phase_counts {
                        total.queued += counts.queued;
                        total.requesting += counts.requesting;
                        total.responding += counts.responding;
                    }
                    total
                },
            )
        }),
        non_success_cost: Some(
            accounts
                .iter()
                .map(|account| account.non_success_cost)
                .sum(),
        ),
        non_success_tokens: Some(
            accounts
                .iter()
                .map(|account| account.non_success_tokens)
                .sum(),
        ),
        maintenance: None,
        data_quality: None,
    };

    DashboardActivitySummaryResponse {
        stats,
        tokens_per_minute: Some(
            sum_optional_rates(accounts, |account| account.tokens_per_minute)
                .unwrap_or(current_snapshot.qualified_tokens.max(0) as f64),
        ),
        spend_rate: Some(
            sum_optional_rates(accounts, |account| account.spend_rate)
                .unwrap_or(current_snapshot.total_cost.max(0.0)),
        ),
        current_first_response_byte_total_avg_ms: current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
        current_avg_total_ms: current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
        current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
        model_performance,
    }
}

pub(crate) fn build_dashboard_activity_latency_summary(
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    (
        current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
    )
}

pub(crate) async fn load_dashboard_activity_current_latency_fallback(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<(Option<f64>, Option<f64>), ApiError> {
    let metric_range = ExactUtcRange {
        start: range
            .start
            .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES)),
        end: range.end,
    };
    let mut latest_first_response_byte_total = LatestTimedMetricValue::default();
    let mut latest_avg_total = LatestTimedMetricValue::default();
    for row in query_live_upstream_account_activity_aggregate_rows(
        &state.pool,
        source_scope,
        metric_range,
        true,
        DashboardActivityExcludedInvocationIdsFilter::None,
    )
    .await?
    {
        latest_first_response_byte_total.update(
            row.latest_first_response_byte_total_at,
            row.latest_first_response_byte_total_ms,
        );
        latest_avg_total.update(row.latest_avg_total_at, row.latest_avg_total_ms);
    }

    Ok((
        latest_first_response_byte_total.value,
        latest_avg_total.value,
    ))
}

pub(crate) async fn query_dashboard_activity_rate_usage_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityRateUsageRow>, ApiError> {
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT invoke_id, occurred_at, ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(
            " AS upstream_account_id, \
             COALESCE(total_tokens, 0) AS total_tokens, \
             COALESCE(cost, 0.0) AS total_cost \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(rate_window_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    Ok(query
        .build_query_as::<DashboardActivityRateUsageRow>()
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn load_dashboard_activity_rate_events_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>, ApiError> {
    let rows = query_dashboard_activity_rate_usage_rows(&state.pool, source_scope, range).await?;
    let db_account_by_key = rows
        .iter()
        .map(|row| {
            (
                (row.invoke_id.clone(), row.occurred_at.clone()),
                row.upstream_account_id,
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = state.proxy_runtime_invocations.snapshot();
    let terminal_keys =
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_snapshot, None).await?;
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let mut runtime_events_by_key =
        HashMap::<(String, String), (Option<i64>, UpstreamAccountRateUsageEvent)>::new();
    for record in runtime_snapshot {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_keys.contains(&key) {
            continue;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred_at < rate_window_start || occurred_at >= range.end {
            continue;
        }
        let upstream_account_id = record
            .upstream_account_id
            .or_else(|| db_account_by_key.get(&key).copied().flatten());
        runtime_events_by_key.insert(
            key,
            (
                upstream_account_id,
                UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: record.total_tokens.unwrap_or_default().max(0),
                    total_cost: record.cost.unwrap_or_default().max(0.0),
                },
            ),
        );
    }

    let mut events_by_account = HashMap::<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>::new();
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if runtime_events_by_key.contains_key(&(row.invoke_id.clone(), row.occurred_at.clone())) {
            continue;
        }
        events_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                total_tokens: row.total_tokens.max(0),
                total_cost: row.total_cost.max(0.0),
            });
    }

    for (_key, (upstream_account_id, event)) in runtime_events_by_key {
        events_by_account
            .entry(upstream_account_id)
            .or_default()
            .push(event);
    }

    Ok(events_by_account)
}

pub(crate) fn sum_dashboard_activity_rate_events(
    events_by_account: &HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>,
    range: ExactUtcRange,
) -> (Option<f64>, Option<f64>) {
    let mut saw_tokens_per_minute = false;
    let mut saw_spend_rate = false;
    let mut tokens_per_minute_total = 0.0;
    let mut spend_rate_total = 0.0;

    for events in events_by_account.values() {
        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(events, range.start, range.end);
        if let Some(value) = tokens_per_minute.filter(|value| value.is_finite()) {
            saw_tokens_per_minute = true;
            tokens_per_minute_total += value;
        }
        if let Some(value) = spend_rate.filter(|value| value.is_finite()) {
            saw_spend_rate = true;
            spend_rate_total += value;
        }
    }

    (
        saw_tokens_per_minute.then_some(tokens_per_minute_total),
        saw_spend_rate.then_some(spend_rate_total),
    )
}

pub(crate) fn dashboard_activity_last_complete_minute_window(
    range: ExactUtcRange,
) -> ExactUtcRange {
    let closed_window_end = range
        .end
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .expect("valid exact minute");
    let closed_window_start = (closed_window_end - ChronoDuration::minutes(1)).max(range.start);
    ExactUtcRange {
        start: closed_window_start,
        end: closed_window_end.max(closed_window_start),
    }
}

pub(crate) async fn load_dashboard_activity_summary_only_snapshot(
    state: &AppState,
    range_name: &str,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let mut totals =
        query_hourly_backed_summary_range(state, range.start, range.end, source_scope).await?;
    totals = totals.add(
        dashboard_activity_historical_live_gap_totals(state, source_scope, range, retention_cutoff)
            .await?,
    );
    let mut stats = totals.into_response();
    stats.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        None,
        Some((range.start, range.end)),
        SummaryLiveAugmentationPolicy {
            include_in_progress: range_name != "yesterday",
            // Hourly summary rollups do not retain non-success token totals; a raw full-range
            // scan here would defeat the summary-only fast path.
            include_non_success_tokens: false,
        },
        None,
    )
    .await?;
    apply_summary_live_augmentation(&mut stats, augmentation);
    let current_snapshot = if range_name == "yesterday" {
        let current_minute_by_account =
            load_dashboard_activity_current_minute_accumulators_by_account(
                state,
                source_scope,
                range,
            )
            .await?;
        sum_dashboard_activity_current_minute_accumulators(current_minute_by_account.into_values())
            .into_current_snapshot()
    } else {
        sum_dashboard_activity_current_snapshots(
            state
                .dashboard_network_speed_cache
                .snapshot_dashboard_activity_accounts(range.end)
                .into_values(),
        )
    };
    let snapshot_first_response_byte_total_avg_ms =
        current_snapshot.first_response_byte_total_avg_ms();
    let snapshot_avg_total_ms = current_snapshot.avg_total_ms();
    let (latest_first_response_byte_total_avg_ms, latest_avg_total_ms) =
        if snapshot_first_response_byte_total_avg_ms.is_none() || snapshot_avg_total_ms.is_none() {
            load_dashboard_activity_current_latency_fallback(state, source_scope, range).await?
        } else {
            (None, None)
        };
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            latest_first_response_byte_total_avg_ms,
            latest_avg_total_ms,
        );

    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        terminal_sequence: 0,
        accounts: Vec::new(),
        summary: DashboardActivitySummaryResponse {
            stats,
            tokens_per_minute: Some(current_snapshot.qualified_tokens.max(0) as f64),
            spend_rate: Some(current_snapshot.total_cost.max(0.0)),
            current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
            current_first_response_byte_total_avg_ms,
            current_avg_total_ms,
            current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
            model_performance: ModelPerformanceAccumulator::default()
                .into_response(range, model_performance_available),
        },
        summary_model_performance_accumulator: ModelPerformanceAccumulator::default(),
        account_model_performance_accumulators: HashMap::new(),
        account_latency_accumulators: HashMap::new(),
        model_performance_accumulator_ready: false,
        materialized_archive_fallback_totals: StatsTotals::default(),
        materialized_archive_details_limited: false,
        build_telemetry: DashboardActivityBuildTelemetry::default(),
    })
}

pub(crate) async fn load_dashboard_activity_snapshot(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
    load_dashboard_activity_snapshot_for_range(
        state,
        range_name,
        reporting_tz,
        range,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
    )
    .await
}

pub(crate) async fn load_dashboard_activity_snapshot_for_range(
    state: &AppState,
    range_name: &str,
    _reporting_tz: Tz,
    range: ExactUtcRange,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if !include_accounts {
        return load_dashboard_activity_summary_only_snapshot(
            state,
            range_name,
            source_scope,
            range,
        )
        .await;
    }
    let build = load_dashboard_activity_account_build_result(
        state,
        range_name,
        range,
        recent_limit,
        include_recent,
        in_progress_counts_override,
        DashboardActivityAccountBuilderKind::DashboardFull,
    )
    .await?;
    let mut summary = build_dashboard_activity_summary(
        &build.accounts,
        range_name != "yesterday",
        build.current_snapshot_summary,
        build.latest_first_response_byte_total_in_range,
        build.latest_avg_total_in_range,
        build.summary_model_performance,
    );
    dashboard_activity_apply_materialized_archive_fallback_to_stats(
        &mut summary.stats,
        build.materialized_archive_fallback_totals,
    );
    if build.materialized_archive_details_limited {
        dashboard_activity_clear_materialized_archive_detail_fields(&mut summary.stats);
    }
    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        terminal_sequence: 0,
        accounts: build.accounts,
        summary,
        summary_model_performance_accumulator: build.summary_model_performance_accumulator,
        account_model_performance_accumulators: build.account_model_performance_accumulators,
        account_latency_accumulators: build.account_latency_accumulators,
        model_performance_accumulator_ready: true,
        materialized_archive_fallback_totals: build.materialized_archive_fallback_totals,
        materialized_archive_details_limited: build.materialized_archive_details_limited,
        build_telemetry: build.build_telemetry,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardActivityAccountBuilderKind {
    DashboardFull,
    UpstreamAccount,
}

impl DashboardActivityAccountBuilderKind {
    pub(crate) fn preview_read_telemetry(self) -> UpstreamAccountActivityPreviewReadTelemetry {
        match self {
            Self::DashboardFull => UpstreamAccountActivityPreviewReadTelemetry {
                route: "dashboard",
                builder: "dashboard_full",
                purpose: "bounded_per_account_recent",
            },
            Self::UpstreamAccount => UpstreamAccountActivityPreviewReadTelemetry {
                route: "upstream_account",
                builder: "upstream_account",
                purpose: "bounded_per_account_recent",
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct DashboardActivityAccountBuildResult {
    pub(crate) accounts: Vec<DashboardActivityAccountResponse>,
    pub(crate) current_snapshot_summary: DashboardActivityCurrentSnapshot,
    pub(crate) latest_first_response_byte_total_in_range: Option<f64>,
    pub(crate) latest_avg_total_in_range: Option<f64>,
    pub(crate) summary_model_performance: ModelPerformanceResponse,
    pub(crate) summary_model_performance_accumulator: ModelPerformanceAccumulator,
    pub(crate) account_model_performance_accumulators:
        HashMap<Option<i64>, ModelPerformanceAccumulator>,
    pub(crate) account_latency_accumulators:
        HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    pub(crate) materialized_archive_fallback_totals: StatsTotals,
    pub(crate) materialized_archive_details_limited: bool,
    pub(crate) build_telemetry: DashboardActivityBuildTelemetry,
}

pub(crate) fn build_dashboard_activity_account_response(
    range_name: &str,
    range: ExactUtcRange,
    model_performance_available: bool,
    current_snapshot_by_account: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    account_meta: &HashMap<i64, UpstreamAccountActivityMetaRow>,
    effective_routing_rules: &HashMap<i64, crate::upstream_accounts::EffectiveRoutingRule>,
    in_progress_counts: &HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    upstream_account_id: Option<i64>,
    aggregate: UpstreamAccountActivityAccumulator,
) -> DashboardActivityAccountResponse {
    let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
    let status_fields =
        meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
    let model_performance = aggregate
        .model_performance
        .into_response(range, model_performance_available);
    let current_snapshot = current_snapshot_by_account
        .get(&upstream_account_id)
        .copied()
        .unwrap_or_default();
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            aggregate.latest_first_response_byte_total_ms,
            aggregate.latest_avg_total_ms,
        );
    let tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
    let spend_rate = Some(current_snapshot.total_cost.max(0.0));
    let (in_progress_invocation_count, in_progress_phase_counts, retry_invocation_count) =
        if range_name == "yesterday" {
            (None, None, None)
        } else {
            let summary = in_progress_counts
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            (
                Some(summary.in_progress_count),
                Some(summary.phase_counts),
                Some(summary.retry_count),
            )
        };
    let account_key = upstream_account_id
        .map(|id| format!("upstream:{id}"))
        .unwrap_or_else(|| "unassigned".to_string());

    DashboardActivityAccountResponse {
        account_key,
        upstream_account_id,
        display_name: upstream_account_id
            .map(|id| {
                resolve_upstream_account_activity_display_name(
                    id,
                    meta,
                    aggregate.display_name_hint.as_deref(),
                )
            })
            .unwrap_or_else(|| "未分配上游账号".to_string()),
        is_unassigned: upstream_account_id.is_none(),
        latest_conversation_created_at: aggregate.latest_conversation_created_at,
        last_invocation_at: aggregate.last_invocation_at,
        group_name: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.group_name.clone()),
        ),
        plan_type: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.plan_type.clone())
                .or(aggregate.plan_type_hint),
        ),
        enabled: status_fields.as_ref().map(|fields| fields.enabled),
        display_status: status_fields
            .as_ref()
            .map(|fields| fields.display_status.clone()),
        enable_status: status_fields
            .as_ref()
            .map(|fields| fields.enable_status.clone()),
        work_status: status_fields
            .as_ref()
            .map(|fields| fields.work_status.clone()),
        health_status: status_fields
            .as_ref()
            .map(|fields| fields.health_status.clone()),
        sync_state: status_fields
            .as_ref()
            .map(|fields| fields.sync_state.clone()),
        last_error: status_fields
            .as_ref()
            .and_then(|fields| fields.last_error.clone()),
        last_action_reason_message: status_fields
            .as_ref()
            .and_then(|fields| fields.last_action_reason_message.clone()),
        request_count: aggregate.request_count,
        success_count: aggregate.success_count,
        failure_count: aggregate.failure_count,
        non_success_count: aggregate.non_success_count,
        total_tokens: aggregate.total_tokens,
        success_tokens: aggregate.success_tokens,
        non_success_tokens: aggregate.non_success_tokens,
        failure_tokens: aggregate.failure_tokens,
        failure_cost: aggregate.failure_cost,
        non_success_cost: aggregate.non_success_cost,
        total_cost: aggregate.total_cost,
        usage_breakdown: aggregate.usage_breakdown.clone().into_response(),
        model_performance,
        cache_hit_rate: (aggregate.total_tokens > 0)
            .then_some(aggregate.cache_input_tokens as f64 / aggregate.total_tokens as f64),
        tokens_per_minute,
        spend_rate,
        first_byte_avg_ms: (aggregate.first_response_byte_total_sample_count > 0).then_some(
            aggregate.first_response_byte_total_sum_ms
                / aggregate.first_response_byte_total_sample_count as f64,
        ),
        first_response_byte_total_avg_ms: (aggregate.first_response_byte_total_sample_count > 0)
            .then_some(
                aggregate.first_response_byte_total_sum_ms
                    / aggregate.first_response_byte_total_sample_count as f64,
            ),
        first_token_avg_ms: (aggregate.first_token_sample_count > 0)
            .then_some(aggregate.first_token_sum_ms / aggregate.first_token_sample_count as f64),
        avg_total_ms: (aggregate.total_latency_sample_count > 0).then_some(
            aggregate.total_latency_sum_ms / aggregate.total_latency_sample_count as f64,
        ),
        current_first_response_byte_total_avg_ms,
        current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
        current_avg_total_ms,
        current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        upload_bytes_per_second: 0.0,
        download_bytes_per_second: 0.0,
        in_progress_wait_sum_ms: aggregate.in_progress_wait_sum_ms,
        in_progress_wait_sample_count: aggregate.in_progress_wait_sample_count,
        effective_routing_rule: upstream_account_id.map(|id| {
            effective_routing_rules
                .get(&id)
                .cloned()
                .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
        }),
        recent_invocations: aggregate.recent_invocations,
    }
}

pub(crate) async fn load_dashboard_activity_account_build_result(
    state: &AppState,
    range_name: &str,
    range: ExactUtcRange,
    recent_limit: usize,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
    builder_kind: DashboardActivityAccountBuilderKind,
) -> Result<DashboardActivityAccountBuildResult, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let mut account_activity = HashMap::<Option<i64>, UpstreamAccountActivityAccumulator>::new();
    let mut model_performance = ModelPerformanceAccumulator::default();
    let materialized_archive_fallback_totals = StatsTotals::default();
    let mut materialized_archive_details_limited = false;
    let current_snapshot_by_account = if range_name == "yesterday" {
        load_dashboard_activity_current_minute_accumulators_by_account(state, source_scope, range)
            .await?
            .into_iter()
            .map(|(upstream_account_id, accumulator)| {
                (upstream_account_id, accumulator.into_current_snapshot())
            })
            .collect::<HashMap<_, _>>()
    } else {
        state
            .dashboard_network_speed_cache
            .snapshot_dashboard_activity_accounts(range.end)
    };
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    let model_performance_duration_overrides = if builder_kind
        == DashboardActivityAccountBuilderKind::DashboardFull
        && model_performance_available
    {
        Some(
            query_live_model_performance_duration_overrides(&state.pool, source_scope, range, true)
                .await?,
        )
    } else {
        None
    };
    let account_range_build = if range_name == "yesterday" {
        AccountActivityRangeBuild {
            rows: query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
            telemetry: AccountActivityRangeBuildTelemetry {
                raw_fallback_range_count: 1,
                ..AccountActivityRangeBuildTelemetry::default()
            },
        }
    } else {
        load_upstream_account_activity_range_rows(
            state,
            source_scope,
            range,
            if builder_kind == DashboardActivityAccountBuilderKind::UpstreamAccount {
                "upstream_account"
            } else {
                "dashboard"
            },
        )
        .await?
    };
    let account_aggregation_telemetry = account_range_build.telemetry;
    for row in account_range_build.rows {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        merge_upstream_account_activity_aggregate_row(entry, &row);
    }
    let usage_breakdown_build = load_usage_breakdown_rows_for_range(
        state,
        source_scope,
        None,
        range,
        UsageBreakdownBuildTelemetry::from_range_route(
            if builder_kind == DashboardActivityAccountBuilderKind::UpstreamAccount {
                "upstream_account"
            } else {
                "dashboard"
            },
            range_name,
        ),
    )
    .await?
    .rows;
    for row in &usage_breakdown_build {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        entry.usage_breakdown.add_aggregate_row(row);
        entry.model_performance.add_aggregate_row(row);
        model_performance.add_aggregate_row(row);
    }
    if range.start < retention_cutoff {
        let archive_rows = query_completed_invocation_archive_activity_aggregate_rows(
            &state.pool,
            source_scope,
            range,
        )
        .await?;
        let QueryCompletedInvocationArchiveActivityAggregateRows {
            aggregates: archived_aggregates,
            skipped_materialized_ranges,
        } = archive_rows;
        for row in archived_aggregates {
            let entry = account_activity.entry(row.upstream_account_id).or_default();
            merge_latest_optional_timestamp(
                &mut entry.latest_conversation_created_at,
                row.latest_conversation_created_at,
            );
            merge_latest_optional_timestamp(&mut entry.last_invocation_at, row.last_invocation_at);
            entry.request_count += row.request_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.non_success_count += row.non_success_count;
            entry.total_tokens += row.total_tokens;
            entry.success_tokens += row.success_tokens;
            entry.non_success_tokens += row.non_success_tokens;
            entry.failure_tokens += row.failure_tokens;
            entry.failure_cost += row.failure_cost;
            entry.non_success_cost += row.non_success_cost;
            entry.cache_input_tokens += row.cache_input_tokens;
            entry.total_cost += row.total_cost;
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_token_sample_count += row.first_token_sample_count;
            entry.first_token_sum_ms += row.first_token_sum_ms;
            entry.total_latency_sample_count += row.total_latency_sample_count;
            entry.total_latency_sum_ms += row.total_latency_sum_ms;
            merge_latest_timed_metric(
                &mut entry.latest_first_response_byte_total_at,
                &mut entry.latest_first_response_byte_total_ms,
                row.latest_first_response_byte_total_at,
                row.latest_first_response_byte_total_ms,
            );
            merge_latest_timed_metric(
                &mut entry.latest_avg_total_at,
                &mut entry.latest_avg_total_ms,
                row.latest_avg_total_at,
                row.latest_avg_total_ms,
            );
        }
        if !skipped_materialized_ranges.is_empty() {
            let global_fallback_totals = dashboard_activity_materialized_archive_fallback_totals(
                state,
                source_scope,
                skipped_materialized_ranges.clone(),
            )
            .await?;
            let (global_first_token_count, global_first_token_sum_ms) =
                dashboard_activity_materialized_archive_first_token_fallback_totals(
                    state,
                    source_scope,
                    &skipped_materialized_ranges,
                )
                .await?;
            if dashboard_activity_stats_totals_has_values(global_fallback_totals)
                || global_first_token_count > 0
            {
                if builder_kind == DashboardActivityAccountBuilderKind::DashboardFull {
                    materialized_archive_details_limited = true;
                }
                let mut account_fallback_totals =
                    dashboard_activity_materialized_archive_account_fallback_totals(
                        state,
                        source_scope,
                        &skipped_materialized_ranges,
                    )
                    .await?;
                let account_fallback_sum = account_fallback_totals
                    .values()
                    .fold(StatsTotals::default(), |total, account_totals| {
                        total.add(account_totals.stats_totals())
                    });
                let residual_fallback_totals = dashboard_activity_stats_totals_subtract(
                    global_fallback_totals,
                    account_fallback_sum,
                );
                let (residual_first_token_count, residual_first_token_sum_ms) =
                    dashboard_activity_residual_first_token_totals(
                        global_first_token_count,
                        global_first_token_sum_ms,
                        &account_fallback_totals,
                    );
                if dashboard_activity_stats_totals_has_values(residual_fallback_totals)
                    || residual_first_token_count > 0
                {
                    let mut residual = DashboardActivityAccountFallbackTotals::from_stats_totals(
                        residual_fallback_totals,
                    );
                    residual.first_token_sample_count = residual_first_token_count;
                    residual.first_token_sum_ms = residual_first_token_sum_ms;
                    account_fallback_totals
                        .entry(None)
                        .or_default()
                        .add_assign(residual);
                }
                for (upstream_account_id, totals) in account_fallback_totals {
                    dashboard_activity_merge_account_fallback_totals(
                        account_activity.entry(upstream_account_id).or_default(),
                        totals,
                    );
                }
            }
        }
    }
    if let Some(model_performance_duration_overrides) = model_performance_duration_overrides {
        model_performance.wall_clock_usage_duration_ms =
            model_performance_duration_overrides.total_wall_clock_ms;
        for (group, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_group_wall_clock_ms
        {
            if let Some(entry) = model_performance.models.get_mut(&group) {
                entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
        for (model, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_model_wall_clock_ms
        {
            if let Some(entry) = model_performance.model_groups.get_mut(&model) {
                entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
        for (upstream_account_id, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_account_wall_clock_ms
        {
            if let Some(entry) = account_activity.get_mut(&upstream_account_id) {
                entry.model_performance.wall_clock_usage_duration_ms =
                    Some(wall_clock_usage_duration_ms);
            }
        }
        for (key, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_account_group_wall_clock_ms
        {
            if let Some(entry) = account_activity.get_mut(&key.upstream_account_id)
                && let Some(model_entry) = entry.model_performance.models.get_mut(&key.group)
            {
                model_entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
        for (key, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_account_model_wall_clock_ms
        {
            if let Some(entry) = account_activity.get_mut(&key.upstream_account_id)
                && let Some(model_entry) = entry.model_performance.model_groups.get_mut(&key.model)
            {
                model_entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
    }
    let mut build_telemetry = DashboardActivityBuildTelemetry {
        account_aggregation: account_aggregation_telemetry,
        ..DashboardActivityBuildTelemetry::default()
    };
    for row in
        query_live_upstream_account_activity_rate_rows(&state.pool, source_scope, range).await?
    {
        if let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) {
            account_activity
                .entry(row.upstream_account_id)
                .or_default()
                .rate_usage_events
                .push(UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: row.total_tokens.max(0),
                    total_cost: row.total_cost.max(0.0),
                });
        }
    }

    if include_recent {
        let hydrated_rows =
            query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
                &state.pool,
                source_scope,
                range,
                recent_limit,
                builder_kind.preview_read_telemetry(),
            )
            .await?;
        build_telemetry = DashboardActivityBuildTelemetry {
            preview_read_mode: "bounded_per_account",
            candidate_preview_id_count: hydrated_rows.candidate_preview_id_count,
            hydrated_preview_row_count: hydrated_rows.hydrated_preview_row_count,
            account_aggregation: account_aggregation_telemetry,
        };
        let mut recent_rows_by_account =
            HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
        let mut recent_rows = hydrated_rows.rows;
        overlay_runtime_upstream_account_activity_preview_rows(
            state,
            &mut recent_rows,
            source_scope,
            range,
        );
        overlay_runtime_terminal_upstream_account_activity_preview_rows(
            state,
            &mut recent_rows,
            source_scope,
            range,
        )
        .await?;
        let live_ids = recent_rows.iter().map(|row| row.id).collect::<HashSet<_>>();
        for row in recent_rows {
            merge_upstream_account_activity_recent_row_metadata(&mut account_activity, &row);
            recent_rows_by_account
                .entry(row.upstream_account_id)
                .or_default()
                .push(upstream_account_invocation_preview_from_row(row));
        }
        if range.start < retention_cutoff {
            let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
                &state.pool,
                source_scope,
                range,
                Some(&live_ids),
            )
            .await?;
            let archived_row_ids = archived_rows.iter().map(|row| row.id).collect::<Vec<_>>();
            let persisted_live_ids = query_live_upstream_account_activity_existing_invocation_ids(
                &state.pool,
                source_scope,
                &archived_row_ids,
            )
            .await?;
            archived_rows.retain(|row| !persisted_live_ids.contains(&row.id));
            for row in archived_rows {
                merge_upstream_account_activity_recent_row_metadata(&mut account_activity, &row);
                recent_rows_by_account
                    .entry(row.upstream_account_id)
                    .or_default()
                    .push(upstream_account_invocation_preview_from_row(row));
            }
        }
        for (upstream_account_id, mut recent_rows) in recent_rows_by_account {
            let entry = account_activity.entry(upstream_account_id).or_default();
            recent_rows.append(&mut entry.recent_invocations);
            let mut seen_keys = HashSet::with_capacity(recent_rows.len());
            recent_rows.retain(|invocation| {
                seen_keys.insert((invocation.invoke_id.clone(), invocation.occurred_at.clone()))
            });
            recent_rows.sort_by(|left, right| {
                right
                    .occurred_at
                    .cmp(&left.occurred_at)
                    .then_with(|| right.id.cmp(&left.id))
            });
            recent_rows.truncate(recent_limit);
            entry.recent_invocations = recent_rows;
        }
    }

    let in_progress_counts = if range_name == "yesterday" {
        HashMap::new()
    } else {
        match in_progress_counts_override {
            Some(counts) => counts,
            None => query_upstream_account_in_progress_counts(state, source_scope).await?,
        }
    };
    for upstream_account_id in in_progress_counts.keys() {
        account_activity.entry(*upstream_account_id).or_default();
    }

    let mut latest_first_response_byte_total_in_range = LatestTimedMetricValue::default();
    let mut latest_avg_total_in_range = LatestTimedMetricValue::default();
    for aggregate in account_activity.values() {
        latest_first_response_byte_total_in_range.update(
            aggregate.latest_first_response_byte_total_at.clone(),
            aggregate.latest_first_response_byte_total_ms,
        );
        latest_avg_total_in_range.update(
            aggregate.latest_avg_total_at.clone(),
            aggregate.latest_avg_total_ms,
        );
    }

    let account_ids = account_activity
        .keys()
        .filter_map(|id| *id)
        .collect::<Vec<_>>();
    let account_meta = query_upstream_account_activity_meta(&state.pool, &account_ids).await?;
    let effective_routing_rules =
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(
            &state.pool,
            &account_ids,
        )
        .await?;
    let account_model_performance_accumulators = account_activity
        .iter()
        .map(|(upstream_account_id, aggregate)| {
            (*upstream_account_id, aggregate.model_performance.clone())
        })
        .collect();
    let account_latency_accumulators = account_activity
        .iter()
        .map(|(upstream_account_id, aggregate)| {
            (
                *upstream_account_id,
                DashboardActivityAccountLatencyAccumulator::from_aggregate(aggregate),
            )
        })
        .collect();
    let summary_model_performance_accumulator = model_performance.clone();
    let mut accounts = account_activity
        .into_iter()
        .map(|(upstream_account_id, aggregate)| {
            build_dashboard_activity_account_response(
                range_name,
                range,
                model_performance_available,
                &current_snapshot_by_account,
                &account_meta,
                &effective_routing_rules,
                &in_progress_counts,
                upstream_account_id,
                aggregate,
            )
        })
        .collect::<Vec<_>>();
    sort_dashboard_activity_accounts(&mut accounts);
    Ok(DashboardActivityAccountBuildResult {
        accounts,
        current_snapshot_summary,
        latest_first_response_byte_total_in_range: latest_first_response_byte_total_in_range.value,
        latest_avg_total_in_range: latest_avg_total_in_range.value,
        summary_model_performance: model_performance
            .into_response(range, model_performance_available),
        summary_model_performance_accumulator,
        account_model_performance_accumulators,
        account_latency_accumulators,
        materialized_archive_fallback_totals,
        materialized_archive_details_limited,
        build_telemetry,
    })
}
