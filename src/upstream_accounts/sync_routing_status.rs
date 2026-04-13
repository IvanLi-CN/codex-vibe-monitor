fn build_effective_routing_rule(tags: &[AccountTagSummary]) -> EffectiveRoutingRule {
    let mut source_tag_ids = Vec::with_capacity(tags.len());
    let mut source_tag_names = Vec::with_capacity(tags.len());
    let mut guard_rules = Vec::new();
    let mut allow_cut_out = true;
    let mut allow_cut_in = true;
    let mut priority_tier = if tags.is_empty() {
        TagPriorityTier::Normal
    } else {
        TagPriorityTier::Primary
    };
    let mut fast_mode_rewrite_mode = TagFastModeRewriteMode::KeepOriginal;
    let mut concurrency_limit = 0;
    let mut representative_guard: Option<(i64, i64)> = None;

    for tag in tags {
        source_tag_ids.push(tag.id);
        source_tag_names.push(tag.name.clone());
        allow_cut_out &= tag.routing_rule.allow_cut_out;
        allow_cut_in &= tag.routing_rule.allow_cut_in;
        priority_tier = priority_tier.min(tag.routing_rule.priority_tier);
        if tag.routing_rule.fast_mode_rewrite_mode.merge_rank()
            < fast_mode_rewrite_mode.merge_rank()
        {
            fast_mode_rewrite_mode = tag.routing_rule.fast_mode_rewrite_mode;
        }
        concurrency_limit =
            merge_concurrency_limits(concurrency_limit, tag.routing_rule.concurrency_limit);
        if tag.routing_rule.guard_enabled
            && let (Some(lookback_hours), Some(max_conversations)) = (
                tag.routing_rule.lookback_hours,
                tag.routing_rule.max_conversations,
            )
        {
            guard_rules.push(EffectiveConversationGuard {
                tag_id: tag.id,
                tag_name: tag.name.clone(),
                lookback_hours,
                max_conversations,
            });
            representative_guard = match representative_guard {
                Some((current_hours, current_max))
                    if current_max < max_conversations
                        || (current_max == max_conversations
                            && current_hours >= lookback_hours) =>
                {
                    Some((current_hours, current_max))
                }
                _ => Some((lookback_hours, max_conversations)),
            };
        }
    }

    EffectiveRoutingRule {
        guard_enabled: !guard_rules.is_empty(),
        lookback_hours: representative_guard.map(|(hours, _)| hours),
        max_conversations: representative_guard.map(|(_, max)| max),
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        concurrency_limit,
        source_tag_ids,
        source_tag_names,
        guard_rules,
    }
}

fn merge_concurrency_limits(current: i64, next: i64) -> i64 {
    match (current, next) {
        (0, 0) => 0,
        (0, next) if next > 0 => next,
        (current, 0) if current > 0 => current,
        (current, next) => current.min(next),
    }
}

fn effective_account_status(row: &UpstreamAccountRow) -> String {
    if row.enabled == 0 {
        UPSTREAM_ACCOUNT_STATUS_DISABLED.to_string()
    } else {
        row.status.clone()
    }
}

fn derive_upstream_account_enable_status(enabled: bool) -> &'static str {
    if enabled {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED
    } else {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED
    }
}

fn derive_upstream_account_sync_state(enabled: bool, raw_status: &str) -> &'static str {
    if !enabled {
        return UPSTREAM_ACCOUNT_SYNC_STATE_IDLE;
    }
    if raw_status
        .trim()
        .eq_ignore_ascii_case(UPSTREAM_ACCOUNT_STATUS_SYNCING)
    {
        UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING
    } else {
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE
    }
}

fn derive_upstream_account_health_status(
    account_kind: &str,
    enabled: bool,
    raw_status: &str,
    last_error: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> &'static str {
    if !enabled {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    let status = raw_status.trim().to_ascii_lowercase();
    let error_message = last_error.unwrap_or_default();
    if matches!(
        status.as_str(),
        UPSTREAM_ACCOUNT_STATUS_ACTIVE | UPSTREAM_ACCOUNT_STATUS_SYNCING
    ) && is_transient_route_failure_error(
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    if upstream_account_rate_limit_state_is_current(
        status.as_str(),
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    if status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
        || last_action_reason_code == Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        || (account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
            && status == UPSTREAM_ACCOUNT_STATUS_ERROR
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message))
    {
        return UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH;
    }
    if is_upstream_unavailable_error_message(error_message) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE;
    }
    if upstream_account_upstream_rejected_state_is_current(
        status.as_str(),
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED;
    }
    if is_upstream_rejected_error_message(error_message) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED;
    }
    if status == UPSTREAM_ACCOUNT_STATUS_ERROR
        || is_bridge_error_message(error_message)
        || !error_message.trim().is_empty()
    {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER;
    }
    UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
}

fn is_transient_route_failure_error(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    if last_error_at.is_none() || last_error_at != last_route_failure_at {
        return false;
    }
    let failure_kind = last_route_failure_kind
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(
        last_action_reason_code,
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
    ) {
        return matches!(failure_kind, Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED));
    }
    route_failure_kind_is_temporary(failure_kind)
        || route_failure_kind_is_rate_limited(failure_kind)
}

fn derive_upstream_account_work_status(
    enabled: bool,
    raw_status: &str,
    health_status: &str,
    sync_state: &str,
    snapshot_exhausted: bool,
    cooldown_until: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
    temporary_route_failure_streak_started_at: Option<&str>,
    last_selected_at: Option<&str>,
    now: DateTime<Utc>,
) -> &'static str {
    if !enabled || sync_state == UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING {
        return UPSTREAM_ACCOUNT_WORK_STATUS_IDLE;
    }
    if snapshot_exhausted
        || upstream_account_quota_exhausted_state_is_current(
            raw_status,
            last_error_at,
            last_route_failure_at,
            last_route_failure_kind,
            last_action_reason_code,
        )
    {
        return UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED;
    }
    if health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE;
    }
    if upstream_account_degraded_state_is_current(
        raw_status,
        cooldown_until,
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
        temporary_route_failure_streak_started_at,
        now,
    ) {
        return UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED;
    }
    let active_cutoff = now - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES);
    if last_selected_at
        .and_then(parse_rfc3339_utc)
        .is_some_and(|selected_at| selected_at >= active_cutoff)
    {
        return UPSTREAM_ACCOUNT_WORK_STATUS_WORKING;
    }
    UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
}

fn classify_upstream_account_display_status(
    account_kind: &str,
    enabled: bool,
    raw_status: &str,
    last_error: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> &'static str {
    let enable_status = derive_upstream_account_enable_status(enabled);
    if enable_status == UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED {
        return UPSTREAM_ACCOUNT_STATUS_DISABLED;
    }
    let sync_state = derive_upstream_account_sync_state(enabled, raw_status);
    if sync_state == UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING {
        return UPSTREAM_ACCOUNT_STATUS_SYNCING;
    }
    let health_status = derive_upstream_account_health_status(
        account_kind,
        enabled,
        raw_status,
        last_error,
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    );
    if health_status == UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return UPSTREAM_ACCOUNT_STATUS_ACTIVE;
    }
    health_status
}

fn matches_upstream_account_filters(
    item: &UpstreamAccountSummary,
    work_status_filters: &[&str],
    enable_status_filters: &[&str],
    health_status_filters: &[&str],
    sync_state_filter: Option<&str>,
) -> bool {
    (work_status_filters.is_empty() || work_status_filters.contains(&item.work_status.as_str()))
        && (enable_status_filters.is_empty()
            || enable_status_filters.contains(&item.enable_status.as_str()))
        && (health_status_filters.is_empty()
            || health_status_filters.contains(&item.health_status.as_str()))
        && sync_state_filter.is_none_or(|value| item.sync_state == value)
}

struct NormalizedUpstreamAccountListFilters {
    work_status_filters: Vec<&'static str>,
    enable_status_filters: Vec<&'static str>,
    health_status_filters: Vec<&'static str>,
    sync_state_filter: Option<&'static str>,
}

fn normalize_upstream_account_list_filters(
    params: &ListUpstreamAccountsQuery,
) -> NormalizedUpstreamAccountListFilters {
    let legacy_status_filter =
        normalize_legacy_upstream_account_status_filter(params.status.as_deref());
    NormalizedUpstreamAccountListFilters {
        work_status_filters: collect_normalized_upstream_account_filters(
            &params.work_status,
            legacy_status_filter.work_status,
            normalize_upstream_account_work_status_filter,
        ),
        enable_status_filters: collect_normalized_upstream_account_filters(
            &params.enable_status,
            legacy_status_filter.enable_status,
            normalize_upstream_account_enable_status_filter,
        ),
        health_status_filters: collect_normalized_upstream_account_filters(
            &params.health_status,
            legacy_status_filter.health_status,
            normalize_upstream_account_health_status_filter,
        ),
        sync_state_filter: legacy_status_filter.sync_state,
    }
}

fn filter_upstream_account_summaries(
    items: Vec<UpstreamAccountSummary>,
    filters: &NormalizedUpstreamAccountListFilters,
) -> Vec<UpstreamAccountSummary> {
    items
        .into_iter()
        .filter(|item| {
            matches_upstream_account_filters(
                item,
                &filters.work_status_filters,
                &filters.enable_status_filters,
                &filters.health_status_filters,
                filters.sync_state_filter,
            )
        })
        .collect()
}

fn build_upstream_account_list_metrics(
    items: &[UpstreamAccountSummary],
) -> UpstreamAccountListMetrics {
    UpstreamAccountListMetrics {
        total: items.len(),
        oauth: items
            .iter()
            .filter(|item| item.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
            .count(),
        api_key: items
            .iter()
            .filter(|item| item.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
            .count(),
        attention: items
            .iter()
            .filter(|item| {
                item.health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
                    || matches!(
                        item.work_status.as_str(),
                        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
                            | UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
                    )
            })
            .count(),
    }
}

fn build_api_key_window(
    limit: Option<f64>,
    unit: Option<&str>,
    window_duration_mins: i64,
) -> Option<RateWindowSnapshot> {
    let limit_text = match limit {
        Some(value) => format!(
            "{} {}",
            format_compact_decimal(value),
            unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)
        ),
        None => "—".to_string(),
    };
    Some(RateWindowSnapshot {
        used_percent: 0.0,
        used_text: format!("0 {}", unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)),
        limit_text,
        resets_at: None,
        window_duration_mins,
        actual_usage: None,
    })
}

fn build_window_snapshot(
    used_percent: Option<f64>,
    window_duration_mins: Option<i64>,
    resets_at: Option<&str>,
) -> Option<RateWindowSnapshot> {
    let used_percent = used_percent?;
    let window_duration_mins = window_duration_mins?;
    Some(RateWindowSnapshot {
        used_percent,
        used_text: format!("{}%", format_percent(used_percent)),
        limit_text: format_window_label(window_duration_mins),
        resets_at: resets_at.map(ToOwned::to_owned),
        window_duration_mins,
        actual_usage: None,
    })
}

struct UpstreamAccountActionPayload<'a> {
    action: &'a str,
    source: &'a str,
    reason_code: Option<&'a str>,
    reason_message: Option<&'a str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&'a str>,
    invoke_id: Option<&'a str>,
    sticky_key: Option<&'a str>,
    occurred_at: &'a str,
}

fn sync_cause_action_source(cause: SyncCause) -> &'static str {
    match cause {
        SyncCause::Manual => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncCause::Maintenance => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncCause::PostCreate => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE,
    }
}

fn sanitize_account_action_message(message: &str) -> Option<String> {
    let collapsed = message
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\n' && ch != '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(240).collect())
}

fn upstream_account_history_retention_days() -> u64 {
    parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
    )
    .unwrap_or(DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS)
}

async fn record_upstream_account_action(
    pool: &Pool<Sqlite>,
    account_id: i64,
    payload: UpstreamAccountActionPayload<'_>,
) -> Result<()> {
    let reason_message = payload
        .reason_message
        .and_then(sanitize_account_action_message);
    let created_at = payload.occurred_at;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, reason_code, reason_message,
            http_status, failure_kind, invoke_id, sticky_key, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(account_id)
    .bind(payload.occurred_at)
    .bind(payload.action)
    .bind(payload.source)
    .bind(payload.reason_code)
    .bind(&reason_message)
    .bind(payload.http_status.map(|value| i64::from(value.as_u16())))
    .bind(payload.failure_kind)
    .bind(payload.invoke_id)
    .bind(payload.sticky_key)
    .bind(created_at)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_action = ?2,
            last_action_source = ?3,
            last_action_reason_code = ?4,
            last_action_reason_message = ?5,
            last_action_http_status = ?6,
            last_action_invoke_id = ?7,
            last_action_at = ?8
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(payload.action)
    .bind(payload.source)
    .bind(payload.reason_code)
    .bind(&reason_message)
    .bind(payload.http_status.map(|value| i64::from(value.as_u16())))
    .bind(payload.invoke_id)
    .bind(payload.occurred_at)
    .execute(pool)
    .await?;

    let retention_cutoff = format_utc_iso(
        Utc::now() - ChronoDuration::days(upstream_account_history_retention_days() as i64),
    );
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_events
        WHERE account_id = ?1 AND occurred_at < ?2
        "#,
    )
    .bind(account_id)
    .bind(retention_cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

fn message_mentions_http_status(message: &str, status: StatusCode) -> bool {
    let code = status.as_u16();
    let code_text = code.to_string();
    [
        format!("returned {code_text}"),
        format!("responded with {code_text}"),
        format!("{code_text}:"),
        format!("status {code_text}"),
        format!("status code {code_text}"),
        format!(" {code_text} "),
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn extract_status_code_from_error_message(message: &str) -> Option<StatusCode> {
    [
        StatusCode::UNAUTHORIZED,
        StatusCode::PAYMENT_REQUIRED,
        StatusCode::FORBIDDEN,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ]
    .into_iter()
    .find(|status| message_mentions_http_status(message, *status))
}

fn classify_sync_failure(
    account_kind: &str,
    error_message: &str,
) -> (
    UpstreamAccountFailureDisposition,
    &'static str,
    Option<&'static str>,
    Option<StatusCode>,
    &'static str,
) {
    if account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
        && is_explicit_reauth_error_message(error_message)
        && !is_scope_permission_error_message(error_message)
        && !is_bridge_error_message(error_message)
    {
        return (
            UpstreamAccountFailureDisposition::HardUnavailable,
            UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
            Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            extract_status_code_from_error_message(error_message),
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
        );
    }

    if let Some(status) = extract_status_code_from_error_message(error_message) {
        let classification =
            classify_pool_account_http_failure(account_kind, status, error_message);
        return (
            classification.disposition,
            classification.reason_code,
            classification.next_account_status,
            Some(status),
            classification.failure_kind,
        );
    }

    let normalized = error_message.to_ascii_lowercase();
    if normalized.contains("failed to request")
        || normalized.contains("timed out")
        || normalized.contains("connection")
        || normalized.contains("transport")
    {
        return (
            UpstreamAccountFailureDisposition::Retryable,
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            None,
            None,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        );
    }

    (
        UpstreamAccountFailureDisposition::Retryable,
        UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
        None,
        None,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
    )
}

fn account_reason_is_rate_limited(reason_code: Option<&str>) -> bool {
    account_reason_is_quota_exhausted(reason_code)
}

fn account_reason_is_temporary_failure(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT
                | UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE
                | UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED
        )
    )
}

fn account_reason_is_upstream_rejected(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some("upstream_http_401" | "upstream_http_402" | "upstream_http_403")
    )
}

fn account_reason_is_maintenance_upstream_rejected(reason_code: Option<&str>) -> bool {
    matches!(reason_code, Some("upstream_http_402" | "upstream_rejected"))
}

fn account_reason_is_quota_exhausted(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED
                | UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED
        )
    )
}

fn status_preserves_current_route_failure(raw_status: &str) -> bool {
    matches!(
        raw_status.trim().to_ascii_lowercase().as_str(),
        UPSTREAM_ACCOUNT_STATUS_ACTIVE | UPSTREAM_ACCOUNT_STATUS_SYNCING
    )
}

fn account_reason_overrides_current_route_failure(
    raw_status: &str,
    reason_code: Option<&str>,
) -> bool {
    account_reason_is_upstream_rejected(reason_code)
        || matches!(
            reason_code,
            Some(
                UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED
                    | UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED
            )
        )
        || (matches!(
            reason_code,
            Some(
                UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR
                    | UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE
            )
        ) && !status_preserves_current_route_failure(raw_status))
}

fn route_failure_kind_is_rate_limited(failure_kind: Option<&str>) -> bool {
    route_failure_kind_is_quota_exhausted(failure_kind)
}

fn route_failure_kind_is_temporary(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                | FORWARD_PROXY_FAILURE_SEND_ERROR
                | FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT
                | FORWARD_PROXY_FAILURE_STREAM_ERROR
                | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                | PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
        )
    )
}

fn route_failure_kind_is_quota_exhausted(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED
        )
    )
}

fn route_failure_kind_is_upstream_rejected(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH | PROXY_FAILURE_UPSTREAM_HTTP_402)
    )
}

fn route_failure_kind_is_maintenance_upstream_rejected(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    )
}

fn upstream_rejected_maintenance_cooldown_until(now: DateTime<Utc>) -> String {
    format_utc_iso(
        now + ChronoDuration::seconds(
            UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
        ),
    )
}

fn sync_failure_requires_upstream_rejected_maintenance_cooldown(
    reason_code: &str,
    failure_kind: &str,
    error_message: &str,
) -> bool {
    account_reason_is_maintenance_upstream_rejected(Some(reason_code))
        || route_failure_kind_is_maintenance_upstream_rejected(Some(failure_kind))
        || maintenance_upstream_rejected_error_message(error_message)
}

fn route_failure_is_current(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
) -> bool {
    last_error_at.is_some() && last_error_at == last_route_failure_at
}

fn current_route_failure_is_rate_limited(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_rate_limited(last_route_failure_kind)
}

fn current_route_failure_is_temporary(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_temporary(last_route_failure_kind)
}

fn current_route_failure_is_quota_exhausted(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_quota_exhausted(last_route_failure_kind)
}

fn current_route_failure_is_upstream_rejected(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_upstream_rejected(last_route_failure_kind)
}

fn upstream_account_rate_limit_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_rate_limited(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_rate_limited(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn account_has_active_cooldown(cooldown_until: Option<&str>, now: DateTime<Utc>) -> bool {
    cooldown_until
        .and_then(parse_rfc3339_utc)
        .is_some_and(|until| until > now)
}

fn upstream_account_degraded_state_is_current(
    raw_status: &str,
    cooldown_until: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
    temporary_route_failure_streak_started_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    if account_reason_overrides_current_route_failure(raw_status, last_action_reason_code) {
        return false;
    }
    let degraded_anchor_at = last_route_failure_at
        .and_then(parse_rfc3339_utc)
        .or_else(|| temporary_route_failure_streak_started_at.and_then(parse_rfc3339_utc));
    if account_has_active_cooldown(cooldown_until, now)
        && route_failure_kind_is_temporary(last_route_failure_kind)
    {
        return true;
    }
    if current_route_failure_is_temporary(
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
    ) || account_reason_is_temporary_failure(last_action_reason_code)
        && route_failure_kind_is_temporary(last_route_failure_kind)
    {
        return degraded_anchor_at.is_some_and(|failed_at| {
            failed_at + ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS)
                > now
        });
    }
    false
}

fn upstream_account_quota_exhausted_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_quota_exhausted(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_quota_exhausted(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn upstream_account_upstream_rejected_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_upstream_rejected(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_upstream_rejected(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn route_failure_kind_requires_manual_api_key_recovery(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH
                | PROXY_FAILURE_UPSTREAM_HTTP_402
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        )
    )
}

fn should_clear_route_failure_state_after_sync_success(row: &UpstreamAccountRow) -> bool {
    row.status != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        || route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        )
}

fn account_update_requests_manual_recovery(payload: &UpdateUpstreamAccountRequest) -> bool {
    payload.enabled == Some(true)
        || payload
            .api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

async fn set_account_status(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error
                ELSE ?3
            END,
            last_error_at = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error_at
                WHEN ?3 IS NULL THEN last_error_at
                ELSE ?4
            END,
            last_route_failure_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_at
            END,
            last_route_failure_kind = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_kind
            END,
            cooldown_until = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE cooldown_until
            END,
            consecutive_route_failures = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN 0
                ELSE consecutive_route_failures
            END,
            temporary_route_failure_streak_started_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE temporary_route_failure_streak_started_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(last_error)
    .bind(&now_iso)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(UPSTREAM_ACCOUNT_STATUS_SYNCING)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_account_sync_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    route_state: SyncSuccessRouteState,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    match route_state {
        SyncSuccessRouteState::PreserveFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .execute(pool)
            .await?;
        }
        SyncSuccessRouteState::ClearFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    last_route_failure_at = NULL,
                    last_route_failure_kind = NULL,
                    cooldown_until = NULL,
                    consecutive_route_failures = 0,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .execute(pool)
            .await?;
        }
    }
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED,
            source,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK),
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_recovery_blocked(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    status: &str,
    reason_code: &'static str,
    reason_message: &str,
    preserved_error: Option<&str>,
    failure_kind: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = COALESCE(?4, last_error),
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(preserved_error)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_hard_unavailable(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    reason_code: &'static str,
    reason_message: &str,
    failure_kind: &'static str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let cooldown_until = sync_failure_requires_upstream_rejected_maintenance_cooldown(
        reason_code,
        failure_kind,
        reason_message,
    )
    .then(|| upstream_rejected_maintenance_cooldown_until(Utc::now()));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = ?3,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
    .bind(&now_iso)
    .bind(reason_message)
    .bind(failure_kind)
    .bind(cooldown_until.as_deref())
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    status: &str,
    error_message: &str,
    reason_code: &'static str,
    http_status: Option<StatusCode>,
    failure_kind: &'static str,
    preserved_route_failure_kind: Option<&str>,
    clear_transient_route_failure_state: bool,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let cooldown_until = sync_failure_requires_upstream_rejected_maintenance_cooldown(
        reason_code,
        failure_kind,
        error_message,
    )
    .then(|| upstream_rejected_maintenance_cooldown_until(Utc::now()));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = CASE
                WHEN ?6 = 1 AND ?5 IS NULL THEN NULL
                WHEN ?5 IS NULL THEN last_route_failure_at
                ELSE ?3
            END,
            last_route_failure_kind = CASE
                WHEN ?6 = 1 AND ?5 IS NULL THEN NULL
                ELSE COALESCE(?5, last_route_failure_kind)
            END,
            cooldown_until = CASE
                WHEN ?7 IS NOT NULL THEN ?7
                WHEN ?6 = 1 THEN NULL
                ELSE cooldown_until
            END,
            temporary_route_failure_streak_started_at = CASE
                WHEN ?6 = 1 THEN NULL
                ELSE temporary_route_failure_streak_started_at
            END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(error_message)
    .bind(preserved_route_failure_kind)
    .bind(clear_transient_route_failure_state)
    .bind(cooldown_until.as_deref())
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_classified_account_sync_failure(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    source: &str,
    error_message: &str,
) -> Result<()> {
    let (disposition, reason_code, next_status, http_status, failure_kind) =
        classify_sync_failure(&row.kind, error_message);
    let next_status = match disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => UPSTREAM_ACCOUNT_STATUS_ACTIVE,
    };
    let (preserved_route_failure_kind, clear_transient_route_failure_state) = match disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => (Some(failure_kind), true),
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => (
            row.last_route_failure_kind.as_deref().filter(|_| {
                status_preserves_current_route_failure(&row.status)
                    && (upstream_account_quota_exhausted_state_is_current(
                        &row.status,
                        row.last_error_at.as_deref(),
                        row.last_route_failure_at.as_deref(),
                        row.last_route_failure_kind.as_deref(),
                        row.last_action_reason_code.as_deref(),
                    ) || route_failure_kind_is_temporary(
                        row.last_route_failure_kind.as_deref(),
                    ))
            }),
            false,
        ),
    };
    record_account_sync_failure(
        pool,
        row.id,
        source,
        next_status,
        error_message,
        reason_code,
        http_status,
        failure_kind,
        preserved_route_failure_kind,
        clear_transient_route_failure_state,
    )
    .await
}
