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

fn dashboard_activity_snapshot_selection_anchor(
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

fn dashboard_activity_snapshot_cache_ttl(_range_name: &str) -> Duration {
    Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS)
}

fn dashboard_activity_snapshot_cache_can_track_expiry(
    range_name: &str,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> bool {
    parse_duration_spec(range_name).is_err() || range.start >= retention_cutoff
}

fn dashboard_activity_entry_expiry_covers(
    entry: &DashboardActivitySnapshotCacheEntry,
    range: ExactUtcRange,
) -> bool {
    entry
        .expiry_covered_until
        .is_none_or(|covered_until| range.start <= covered_until)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardActivitySnapshotReuseMode {
    Current,
    LastGoodReconcileBackoff,
}

fn dashboard_activity_snapshot_reuse_mode(
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

fn mark_dashboard_activity_reconcile_failed(entry: &mut DashboardActivitySnapshotCacheEntry) {
    entry.last_reconcile_attempted_at = Instant::now();
    entry.last_reconcile_failed = true;
}

fn dashboard_activity_pressure_reconcile_deferred(
    entry: &DashboardActivitySnapshotCacheEntry,
) -> bool {
    entry.cached_at.elapsed() <= DASHBOARD_ACTIVITY_PRESSURE_RECONCILE_MAX_AGE
}

fn insert_dashboard_activity_expiry_delta(
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

async fn load_dashboard_activity_expiry_deltas(
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

fn resolve_dashboard_activity_exact_range(
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

fn dashboard_activity_full_hour_exact_range(
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
    let (status_fields, plan_type_hint, display_name) =
        dashboard_activity_live_account_display_data(live_account, meta, &recent_invocations);

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

fn dashboard_activity_live_account_display_data(
    live_account: &DashboardActivityLiveAccount,
    meta: Option<&UpstreamAccountActivityMetaRow>,
    recent_invocations: &[PromptCacheConversationInvocationPreviewResponse],
) -> (
    Option<UpstreamAccountActivityStatusFields>,
    Option<String>,
    String,
) {
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
    (status_fields, plan_type_hint, display_name)
}

fn merge_dashboard_activity_recent_invocations(
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

async fn load_dashboard_activity_live_recent_invocations_by_account(
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

fn apply_dashboard_activity_live_summary(
    summary: &mut DashboardActivitySummaryResponse,
    live: &DashboardActivityLiveSnapshot,
    current_snapshot: DashboardActivityCurrentSnapshot,
) {
    summary.stats.in_progress_conversation_count = Some(live.in_progress_invocation_count);
    summary.stats.in_progress_retry_conversation_count = Some(live.retry_invocation_count);
    summary.stats.in_progress_phase_counts = Some(live.in_progress_phase_counts);
    summary.tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
    summary.spend_rate = Some(current_snapshot.total_cost.max(0.0));
    summary.current_first_response_byte_total_avg_ms = current_snapshot
        .first_response_byte_total_avg_ms()
        .or(summary.current_first_response_byte_total_avg_ms);
    summary.current_first_token_avg_ms = current_snapshot.first_token_avg_ms();
    summary.current_avg_total_ms = current_snapshot
        .avg_total_ms()
        .or(summary.current_avg_total_ms);
    summary.current_avg_response_ms = current_snapshot
        .avg_response_duration_ms()
        .or(summary.current_avg_response_ms);
}

async fn refresh_dashboard_activity_existing_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    live_accounts: &HashMap<String, DashboardActivityLiveAccount>,
    current_snapshots: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    refreshed_recent: &mut Option<
        HashMap<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>,
    >,
    recent_limit: usize,
) -> Result<(), ApiError> {
    let refresh_ids = snapshot
        .accounts
        .iter()
        .filter(|account| account.enabled.is_none() && account.upstream_account_id.is_some())
        .filter_map(|account| account.upstream_account_id)
        .collect::<Vec<_>>();
    let (metadata, routing_rules) = if refresh_ids.is_empty() {
        (HashMap::new(), HashMap::new())
    } else {
        (
            query_upstream_account_activity_meta(&state.pool, &refresh_ids).await?,
            crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                &state.pool,
                &refresh_ids,
            )
            .await?,
        )
    };
    for account in &mut snapshot.accounts {
        if let Some(account_id) = account.upstream_account_id
            && let Some(meta) = metadata.get(&account_id)
        {
            let status = build_upstream_account_activity_status_fields(meta, Utc::now());
            account.display_name = resolve_upstream_account_activity_display_name(
                account_id,
                Some(meta),
                Some(account.display_name.as_str()),
            );
            account.group_name = normalize_trimmed_optional_string_local(meta.group_name.clone());
            account.plan_type = normalize_trimmed_optional_string_local(meta.plan_type.clone());
            account.enabled = Some(status.enabled);
            account.display_status = Some(status.display_status);
            account.enable_status = Some(status.enable_status);
            account.work_status = Some(status.work_status);
            account.health_status = Some(status.health_status);
            account.sync_state = Some(status.sync_state);
            account.last_error = status.last_error;
            account.last_action_reason_message = status.last_action_reason_message;
            account.effective_routing_rule = Some(
                routing_rules
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
        let current = current_snapshots
            .get(&account.upstream_account_id)
            .copied()
            .unwrap_or_default();
        account.tokens_per_minute = Some(current.qualified_tokens.max(0) as f64);
        account.spend_rate = Some(current.total_cost.max(0.0));
        account.current_first_response_byte_total_avg_ms = current
            .first_response_byte_total_avg_ms()
            .or(account.current_first_response_byte_total_avg_ms);
        account.current_first_token_avg_ms = current.first_token_avg_ms();
        account.current_avg_total_ms = current.avg_total_ms().or(account.current_avg_total_ms);
        account.current_avg_response_ms = current
            .avg_response_duration_ms()
            .or(account.current_avg_response_ms);
        if let Some(recent) = refreshed_recent.as_mut()
            && let Some(recent_invocations) = recent.remove(&account.upstream_account_id)
        {
            account.recent_invocations = merge_dashboard_activity_recent_invocations(
                recent_invocations,
                std::mem::take(&mut account.recent_invocations),
                recent_limit,
            );
        }
    }
    Ok(())
}

async fn append_dashboard_activity_missing_live_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    missing_accounts: Vec<&DashboardActivityLiveAccount>,
    current_snapshots: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    refreshed_recent: &mut Option<
        HashMap<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>,
    >,
    request_range: ExactUtcRange,
) -> Result<(), ApiError> {
    if missing_accounts.is_empty() {
        return Ok(());
    }
    let account_ids = missing_accounts
        .iter()
        .filter_map(|account| account.upstream_account_id)
        .collect::<Vec<_>>();
    let metadata = query_upstream_account_activity_meta(&state.pool, &account_ids).await?;
    let routing_rules = crate::upstream_accounts::load_effective_routing_rules_for_accounts(
        &state.pool,
        &account_ids,
    )
    .await?;
    for live_account in missing_accounts {
        let account_id = live_account.upstream_account_id;
        let recent = refreshed_recent
            .as_mut()
            .and_then(|recent| recent.remove(&account_id))
            .unwrap_or_default();
        let rule = account_id.map(|id| {
            routing_rules
                .get(&id)
                .cloned()
                .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
        });
        snapshot.accounts.push(dashboard_activity_account_from_live(
            live_account,
            account_id.and_then(|id| metadata.get(&id)),
            request_range,
            current_snapshots
                .get(&account_id)
                .copied()
                .unwrap_or_default(),
            snapshot.summary.model_performance.available,
            rule,
            recent,
        ));
    }
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
    Ok(())
}

async fn append_dashboard_activity_recent_only_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    account_ids: Vec<Option<i64>>,
    refreshed_recent: &mut HashMap<
        Option<i64>,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    >,
    current_snapshots: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    request_range: ExactUtcRange,
) -> Result<(), ApiError> {
    if account_ids.is_empty() {
        return Ok(());
    }
    let metadata_ids = account_ids.iter().filter_map(|id| *id).collect::<Vec<_>>();
    let metadata = if metadata_ids.is_empty() {
        HashMap::new()
    } else {
        query_upstream_account_activity_meta(&state.pool, &metadata_ids).await?
    };
    let routing_rules = if metadata_ids.is_empty() {
        HashMap::new()
    } else {
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(
            &state.pool,
            &metadata_ids,
        )
        .await?
    };
    for account_id in account_ids {
        let recent = refreshed_recent.remove(&account_id).unwrap_or_default();
        if recent.is_empty() {
            continue;
        }
        let live_account = DashboardActivityLiveAccount {
            account_key: account_id
                .map(|id| format!("upstream:{id}"))
                .unwrap_or_else(|| "unassigned".to_string()),
            upstream_account_id: account_id,
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
        let rule = account_id.map(|id| {
            routing_rules
                .get(&id)
                .cloned()
                .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
        });
        snapshot.accounts.push(dashboard_activity_account_from_live(
            &live_account,
            account_id.and_then(|id| metadata.get(&id)),
            request_range,
            current_snapshots
                .get(&account_id)
                .copied()
                .unwrap_or_default(),
            snapshot.summary.model_performance.available,
            rule,
            recent,
        ));
    }
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
    Ok(())
}

async fn overlay_dashboard_activity_live_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    live: DashboardActivityLiveSnapshot,
    request_range: ExactUtcRange,
    include_accounts: bool,
    include_recent: bool,
    recent_limit: usize,
) -> Result<(), ApiError> {
    let current_snapshot_by_account = state
        .dashboard_network_speed_cache
        .snapshot_dashboard_activity_accounts(Utc::now());
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    apply_dashboard_activity_live_summary(&mut snapshot.summary, &live, current_snapshot_summary);

    if !include_accounts {
        return Ok(());
    }

    let mut refreshed_recent_invocations_by_account =
        load_dashboard_activity_live_recent_if_requested(
            state,
            request_range,
            include_recent,
            recent_limit,
        )
        .await?;
    let live_accounts = live
        .accounts
        .into_iter()
        .map(|account| (account.account_key.clone(), account))
        .collect::<HashMap<_, _>>();
    refresh_dashboard_activity_existing_accounts(
        state,
        snapshot,
        &live_accounts,
        &current_snapshot_by_account,
        &mut refreshed_recent_invocations_by_account,
        recent_limit,
    )
    .await?;

    let existing_account_keys = snapshot
        .accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    let missing_live_accounts = live_accounts
        .values()
        .filter(|account| !existing_account_keys.contains(&account.account_key))
        .collect::<Vec<_>>();

    append_dashboard_activity_missing_live_accounts(
        state,
        snapshot,
        missing_live_accounts,
        &current_snapshot_by_account,
        &mut refreshed_recent_invocations_by_account,
        request_range,
    )
    .await?;

    if let Some(mut refreshed_recent_invocations_by_account) =
        refreshed_recent_invocations_by_account
    {
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

        append_dashboard_activity_recent_only_accounts(
            state,
            snapshot,
            missing_terminal_account_ids,
            &mut refreshed_recent_invocations_by_account,
            &current_snapshot_by_account,
            request_range,
        )
        .await?;
    }

    finalize_dashboard_activity_live_overlay(snapshot, current_snapshot_summary);

    Ok(())
}

async fn load_dashboard_activity_live_recent_if_requested(
    state: &AppState,
    request_range: ExactUtcRange,
    include_recent: bool,
    recent_limit: usize,
) -> Result<
    Option<HashMap<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>>,
    ApiError,
> {
    if !include_recent {
        return Ok(None);
    }
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    load_dashboard_activity_live_recent_invocations_by_account(
        state,
        source_scope,
        request_range,
        recent_limit,
    )
    .await
    .map(Some)
}

fn finalize_dashboard_activity_live_overlay(
    snapshot: &mut DashboardActivitySnapshot,
    current_snapshot_summary: DashboardActivityCurrentSnapshot,
) {
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
}
