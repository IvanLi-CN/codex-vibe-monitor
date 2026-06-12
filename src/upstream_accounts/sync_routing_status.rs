fn build_effective_routing_rule(tags: &[AccountTagSummary]) -> EffectiveRoutingRule {
    let mut source_tag_ids = Vec::with_capacity(tags.len());
    let mut source_tag_names = Vec::with_capacity(tags.len());
    let mut block_new_conversations = false;
    let mut allow_cut_out = true;
    let mut allow_cut_in = true;
    let mut priority_tier = if tags.is_empty() {
        TagPriorityTier::Normal
    } else {
        TagPriorityTier::Primary
    };
    let mut fast_mode_rewrite_mode = TagFastModeRewriteMode::KeepOriginal;
    let mut concurrency_limit = 0;
    let mut upstream_429_retry_enabled = false;
    let mut upstream_429_max_retries = 0_u8;
    let mut available_models: Option<Vec<String>> = None;
    let mut tag_available_models_defined = false;
    let mut system_denied_models = BTreeSet::new();

    for tag in tags {
        source_tag_ids.push(tag.id);
        source_tag_names.push(tag.name.clone());
        block_new_conversations |= tag.routing_rule.block_new_conversations;
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
        if tag.routing_rule.upstream_429_retry_enabled {
            upstream_429_retry_enabled = true;
            upstream_429_max_retries =
                upstream_429_max_retries.max(tag.routing_rule.upstream_429_max_retries);
        }
        if !tag.routing_rule.available_models.is_empty() {
            tag_available_models_defined = true;
            available_models = Some(match available_models.take() {
                Some(current) => current
                    .into_iter()
                    .filter(|model| tag.routing_rule.available_models.contains(model))
                    .collect(),
                None => tag.routing_rule.available_models.clone(),
            });
        }
        if let Some(model) = tag
            .system_key
            .as_deref()
            .and_then(|value| value.strip_prefix("unsupported_model:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            system_denied_models.insert(model.to_string());
        }
    }

    let field_source = if tags.is_empty() { "root" } else { "tag" }.to_string();
    let available_models_source = if tag_available_models_defined {
        "tag"
    } else {
        "root"
    }
    .to_string();
    let system_denied_models_source = if system_denied_models.is_empty() {
        "root"
    } else {
        "system"
    }
    .to_string();
    EffectiveRoutingRule {
        block_new_conversations,
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        concurrency_limit,
        upstream_429_retry_enabled,
        upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            upstream_429_max_retries,
        ),
        available_models: available_models.unwrap_or_default(),
        available_models_defined: tag_available_models_defined,
        system_denied_models: system_denied_models.into_iter().collect(),
        source_tag_ids,
        source_tag_names,
        field_sources: EffectiveRoutingRuleFieldSources {
            block_new_conversations: field_source.clone(),
            allow_cut_out: field_source.clone(),
            allow_cut_in: field_source.clone(),
            priority_tier: field_source.clone(),
            fast_mode_rewrite_mode: field_source.clone(),
            concurrency_limit: field_source.clone(),
            upstream_429_retry: field_source.clone(),
            available_models: available_models_source,
            system_denied_models: system_denied_models_source,
        },
    }
}

#[derive(Debug, Clone, FromRow)]
struct RoutingPolicyOverrideRow {
    id: i64,
    policy_block_new_conversations: Option<i64>,
    policy_allow_cut_out: Option<i64>,
    policy_allow_cut_in: Option<i64>,
    policy_priority_tier: Option<String>,
    policy_fast_mode_rewrite_mode: Option<String>,
    policy_concurrency_limit: Option<i64>,
    policy_upstream_429_retry_enabled: Option<i64>,
    policy_upstream_429_max_retries: Option<i64>,
    policy_available_models_json: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct GroupRoutingPolicyOverrideRow {
    group_name: String,
    concurrency_limit: i64,
    upstream_429_retry_enabled: i64,
    upstream_429_max_retries: i64,
    policy_block_new_conversations: Option<i64>,
    policy_allow_cut_out: Option<i64>,
    policy_allow_cut_in: Option<i64>,
    policy_priority_tier: Option<String>,
    policy_fast_mode_rewrite_mode: Option<String>,
    policy_concurrency_limit: Option<i64>,
    policy_upstream_429_retry_enabled: Option<i64>,
    policy_upstream_429_max_retries: Option<i64>,
    policy_available_models_json: Option<String>,
}

fn apply_routing_policy_override(
    rule: &mut EffectiveRoutingRule,
    source: &str,
    block_new_conversations: Option<i64>,
    allow_cut_out: Option<i64>,
    allow_cut_in: Option<i64>,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
    upstream_429_retry_enabled: Option<i64>,
    upstream_429_max_retries: Option<i64>,
    available_models_json: Option<&str>,
) {
    if let Some(block_new_conversations) = block_new_conversations {
        if block_new_conversations != 0 {
            rule.field_sources.block_new_conversations = source.to_string();
            rule.block_new_conversations = true;
        }
    }
    if let Some(allow_cut_out) = allow_cut_out {
        rule.field_sources.allow_cut_out = source.to_string();
        rule.allow_cut_out = allow_cut_out != 0;
    }
    if let Some(allow_cut_in) = allow_cut_in {
        rule.field_sources.allow_cut_in = source.to_string();
        rule.allow_cut_in = allow_cut_in != 0;
    }
    if priority_tier.is_some()
        && let Ok(priority_tier) = normalize_tag_priority_tier(priority_tier)
    {
        rule.field_sources.priority_tier = source.to_string();
        rule.priority_tier = priority_tier;
    }
    if fast_mode_rewrite_mode.is_some()
        && let Ok(fast_mode_rewrite_mode) =
        normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)
    {
        rule.field_sources.fast_mode_rewrite_mode = source.to_string();
        rule.fast_mode_rewrite_mode = fast_mode_rewrite_mode;
    }
    if let Some(concurrency_limit) = concurrency_limit {
        if let Ok(concurrency_limit) =
            normalize_concurrency_limit(Some(concurrency_limit), "concurrencyLimit")
        {
            rule.field_sources.concurrency_limit = source.to_string();
            rule.concurrency_limit = concurrency_limit;
        }
    }
    if let Some(upstream_429_retry_enabled) = upstream_429_retry_enabled {
        rule.field_sources.upstream_429_retry = source.to_string();
        rule.upstream_429_retry_enabled = upstream_429_retry_enabled != 0;
        rule.upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
            rule.upstream_429_retry_enabled,
            upstream_429_max_retries
                .map(decode_group_upstream_429_max_retries)
                .unwrap_or_default(),
        );
    }
    if let Some(available_models_json) = available_models_json {
        let available_models = parse_string_array_json(Some(available_models_json));
        if !available_models.is_empty() {
            rule.field_sources.available_models = source.to_string();
            rule.available_models = available_models;
            rule.available_models_defined = true;
        }
    }
}

async fn load_group_routing_policy_override_map(
    pool: &Pool<Sqlite>,
    group_names: &[String],
) -> Result<HashMap<String, GroupRoutingPolicyOverrideRow>> {
    if group_names.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            group_name,
            concurrency_limit,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            policy_block_new_conversations,
            policy_allow_cut_out,
            policy_allow_cut_in,
            policy_priority_tier,
            policy_fast_mode_rewrite_mode,
            policy_concurrency_limit,
            policy_upstream_429_retry_enabled,
            policy_upstream_429_max_retries,
            policy_available_models_json
        FROM pool_upstream_account_group_notes
        WHERE group_name IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for group_name in group_names {
            separated.push_bind(group_name);
        }
    }
    let rows = query
        .push(")")
        .build_query_as::<GroupRoutingPolicyOverrideRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.group_name.clone(), row))
        .collect())
}

async fn load_account_routing_policy_override_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, RoutingPolicyOverrideRow>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            policy_block_new_conversations,
            policy_allow_cut_out,
            policy_allow_cut_in,
            policy_priority_tier,
            policy_fast_mode_rewrite_mode,
            policy_concurrency_limit,
            policy_upstream_429_retry_enabled,
            policy_upstream_429_max_retries,
            policy_available_models_json
        FROM pool_upstream_accounts
        WHERE id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(")")
        .build_query_as::<RoutingPolicyOverrideRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

fn apply_group_routing_policy_override(
    rule: &mut EffectiveRoutingRule,
    row: &GroupRoutingPolicyOverrideRow,
) {
    apply_routing_policy_override(
        rule,
        "group",
        row.policy_block_new_conversations,
        row.policy_allow_cut_out,
        row.policy_allow_cut_in,
        row.policy_priority_tier.as_deref(),
        row.policy_fast_mode_rewrite_mode.as_deref(),
        row.policy_concurrency_limit,
        row.policy_upstream_429_retry_enabled,
        row.policy_upstream_429_max_retries,
        row.policy_available_models_json.as_deref(),
    );
    if row.policy_concurrency_limit.is_none() && row.concurrency_limit > 0 {
        rule.field_sources.concurrency_limit = "group".to_string();
        rule.concurrency_limit = row.concurrency_limit;
    }
    if row.policy_upstream_429_retry_enabled.is_none()
        && decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled)
    {
        rule.field_sources.upstream_429_retry = "group".to_string();
        rule.upstream_429_retry_enabled = true;
        rule.upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
            true,
            decode_group_upstream_429_max_retries(row.upstream_429_max_retries),
        );
    }
}

fn apply_tag_layer_routing_policy(rule: &mut EffectiveRoutingRule, tag_rule: &EffectiveRoutingRule) {
    let inherited_block_new_conversations = rule.block_new_conversations;
    let inherited_block_source = rule.field_sources.block_new_conversations.clone();
    let inherited_available_models = rule.available_models.clone();
    let inherited_available_models_defined = rule.available_models_defined;
    let inherited_available_models_source = rule.field_sources.available_models.clone();
    rule.block_new_conversations |= tag_rule.block_new_conversations;
    rule.allow_cut_out = tag_rule.allow_cut_out;
    rule.allow_cut_in = tag_rule.allow_cut_in;
    rule.priority_tier = tag_rule.priority_tier;
    rule.fast_mode_rewrite_mode = tag_rule.fast_mode_rewrite_mode;
    rule.concurrency_limit = tag_rule.concurrency_limit;
    rule.upstream_429_retry_enabled = tag_rule.upstream_429_retry_enabled;
    rule.upstream_429_max_retries = if tag_rule.upstream_429_retry_enabled {
        tag_rule.upstream_429_max_retries
    } else {
        0
    };
    rule.available_models = tag_rule.available_models.clone();
    rule.available_models_defined = tag_rule.available_models_defined;
    rule.system_denied_models = tag_rule.system_denied_models.clone();
    rule.source_tag_ids = tag_rule.source_tag_ids.clone();
    rule.source_tag_names = tag_rule.source_tag_names.clone();
    rule.field_sources = tag_rule.field_sources.clone();
    if inherited_block_new_conversations {
        rule.field_sources.block_new_conversations = inherited_block_source;
    }
    if tag_rule.field_sources.available_models != "tag" {
        rule.available_models = inherited_available_models;
        rule.available_models_defined = inherited_available_models_defined;
        rule.field_sources.available_models = inherited_available_models_source;
    }
}

fn apply_account_routing_policy_override(
    rule: &mut EffectiveRoutingRule,
    row: &RoutingPolicyOverrideRow,
) {
    apply_routing_policy_override(
        rule,
        "account",
        row.policy_block_new_conversations,
        row.policy_allow_cut_out,
        row.policy_allow_cut_in,
        row.policy_priority_tier.as_deref(),
        row.policy_fast_mode_rewrite_mode.as_deref(),
        row.policy_concurrency_limit,
        row.policy_upstream_429_retry_enabled,
        row.policy_upstream_429_max_retries,
        row.policy_available_models_json.as_deref(),
    );
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

fn maintenance_upstream_rejected_failed_at(
    last_action_source: Option<&str>,
    last_action_reason_code: Option<&str>,
    last_action_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_error_at: Option<&str>,
    last_error: Option<&str>,
) -> Option<DateTime<Utc>> {
    if !matches!(
        last_action_source,
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
    ) {
        return None;
    }
    let has_maintenance_upstream_rejected_marker =
        account_reason_is_maintenance_upstream_rejected(last_action_reason_code)
            || route_failure_kind_is_maintenance_upstream_rejected(last_route_failure_kind)
            || last_error.is_some_and(maintenance_upstream_rejected_error_message);
    if !has_maintenance_upstream_rejected_marker {
        return None;
    }

    last_action_at
        .and_then(parse_rfc3339_utc)
        .or_else(|| last_route_failure_at.and_then(parse_rfc3339_utc))
        .or_else(|| last_error_at.and_then(parse_rfc3339_utc))
}

fn maintenance_sync_rejected_cooldown_until(
    source: &str,
    reason_code: &str,
    reason_message: &str,
    occurred_at: &str,
) -> Option<String> {
    if source != UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE {
        return None;
    }
    if !account_reason_is_maintenance_upstream_rejected(Some(reason_code))
        && !maintenance_upstream_rejected_error_message(reason_message)
    {
        return None;
    }
    let occurred_at = parse_rfc3339_utc(occurred_at)?;
    Some(format_utc_iso(
        occurred_at
            + ChronoDuration::seconds(UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS),
    ))
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

fn derive_upstream_account_action_result(
    action: &str,
    reason_code: Option<&str>,
    reason_message: Option<&str>,
) -> &'static str {
    if action == UPSTREAM_ACCOUNT_ACTION_SYNC_DEFERRED
        || reason_code == Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED)
        || reason_message.is_some_and(|message| message.contains("remaining"))
    {
        return "deferred";
    }
    if matches!(
        action,
        UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED
            | UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED
            | UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED
    ) {
        return "success";
    }
    "failed"
}

async fn record_upstream_account_action(
    pool: &Pool<Sqlite>,
    account_id: i64,
    payload: UpstreamAccountActionPayload<'_>,
) -> Result<()> {
    record_upstream_account_action_with_proxy_snapshot(pool, account_id, payload, None).await
}

async fn record_upstream_account_action_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    payload: UpstreamAccountActionPayload<'_>,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    let reason_message = payload
        .reason_message
        .and_then(sanitize_account_action_message);
    let created_at = payload.occurred_at;
    let account_snapshot = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        r#"
        SELECT display_name, group_name
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let (account_display_name, account_group_name) = account_snapshot.unwrap_or_default();
    let result = derive_upstream_account_action_result(
        payload.action,
        payload.reason_code,
        reason_message.as_deref(),
    );
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, account_display_name, account_group_name,
            forward_proxy_key, forward_proxy_display_name, forward_proxy_egress_ip, result,
            result_description, reason_code, reason_message, http_status, failure_kind, invoke_id,
            sticky_key, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(account_id)
    .bind(payload.occurred_at)
    .bind(payload.action)
    .bind(payload.source)
    .bind(account_display_name)
    .bind(account_group_name)
    .bind(proxy_snapshot.as_ref().map(|snapshot| snapshot.proxy_key.as_str()))
    .bind(
        proxy_snapshot
            .as_ref()
            .map(|snapshot| snapshot.proxy_display_name.as_str()),
    )
    .bind(
        proxy_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.proxy_egress_ip.as_deref()),
    )
    .bind(result)
    .bind(reason_message.as_deref())
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

pub(crate) async fn record_account_maintenance_deferred(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &'static str,
    reason_message: &str,
    occurred_at: &str,
    forward_proxy_key: Option<&str>,
    forward_proxy_display_name: Option<&str>,
    forward_proxy_egress_ip: Option<&str>,
) -> Result<()> {
    let proxy_snapshot = forward_proxy_key.map(|proxy_key| AccountMaintenanceProxySnapshot {
        proxy_key: proxy_key.to_string(),
        proxy_display_name: forward_proxy_display_name
            .unwrap_or(proxy_key)
            .to_string(),
        proxy_egress_ip: forward_proxy_egress_ip.map(ToOwned::to_owned),
    });
    record_upstream_account_action_with_proxy_snapshot(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_DEFERRED,
            source,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at,
        },
        proxy_snapshot.as_ref(),
    )
    .await
}

#[cfg(test)]
mod account_action_event_tests {
    use super::*;
    use sqlx::Row;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure upstream account schema");
        pool
    }

    async fn insert_test_account(pool: &Pool<Sqlite>) -> i64 {
        let now = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, status, enabled, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind("Fixture Account")
        .bind("production")
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(&now)
        .execute(pool)
        .await
        .expect("insert test account")
        .last_insert_rowid()
    }

    #[tokio::test]
    async fn record_action_persists_snapshot_result_and_proxy_fields() {
        let pool = test_pool().await;
        let account_id = insert_test_account(&pool).await;
        let occurred_at = format_utc_iso(Utc::now());

        let proxy_snapshot = AccountMaintenanceProxySnapshot {
            proxy_key: "jp-edge-01".to_string(),
            proxy_display_name: "JP Edge 01".to_string(),
            proxy_egress_ip: Some("203.0.113.10".to_string()),
        };
        record_upstream_account_action_with_proxy_snapshot(
            &pool,
            account_id,
            UpstreamAccountActionPayload {
                action: UPSTREAM_ACCOUNT_ACTION_SYNC_DEFERRED,
                source: UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
                reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED),
                reason_message: Some(
                    "maintenance egress via JP Edge 01 is throttled for another 520 seconds",
                ),
                http_status: None,
                failure_kind: None,
                invoke_id: None,
                sticky_key: None,
                occurred_at: &occurred_at,
            },
            Some(&proxy_snapshot),
        )
        .await
        .expect("record action");

        let row = sqlx::query(
            r#"
            SELECT account_display_name, account_group_name, forward_proxy_key,
                   forward_proxy_display_name, forward_proxy_egress_ip, result,
                   result_description
            FROM pool_upstream_account_events
            WHERE account_id = ?1
            "#,
        )
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .expect("load event");

        assert_eq!(
            row.try_get::<String, _>("account_display_name").unwrap(),
            "Fixture Account"
        );
        assert_eq!(
            row.try_get::<String, _>("account_group_name").unwrap(),
            "production"
        );
        assert_eq!(
            row.try_get::<String, _>("forward_proxy_key").unwrap(),
            "jp-edge-01"
        );
        assert_eq!(
            row.try_get::<String, _>("forward_proxy_display_name")
                .unwrap(),
            "JP Edge 01"
        );
        assert_eq!(
            row.try_get::<String, _>("forward_proxy_egress_ip")
                .unwrap(),
            "203.0.113.10"
        );
        assert_eq!(row.try_get::<String, _>("result").unwrap(), "deferred");
        assert_eq!(
            row.try_get::<String, _>("result_description").unwrap(),
            "maintenance egress via JP Edge 01 is throttled for another 520 seconds"
        );
    }
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

pub(crate) async fn set_account_status(
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
    mark_account_sync_success_with_proxy_snapshot(pool, account_id, source, route_state, None).await
}

async fn mark_account_sync_success_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    route_state: SyncSuccessRouteState,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
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
                    cooldown_until = CASE
                        WHEN last_action_source = ?4
                             AND last_action_reason_code IN (?5, ?6) THEN NULL
                        ELSE cooldown_until
                    END,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
            .bind("upstream_http_402")
            .bind("upstream_rejected")
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
    record_upstream_account_action_with_proxy_snapshot(
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
        proxy_snapshot,
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
            cooldown_until = CASE
                WHEN last_action_source = ?5
                     AND last_action_reason_code IN (?6, ?7) THEN NULL
                ELSE cooldown_until
            END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(preserved_error)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
    .bind("upstream_http_402")
    .bind("upstream_rejected")
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
    let cooldown_until =
        maintenance_sync_rejected_cooldown_until(source, reason_code, reason_message, &now_iso);
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
    .bind(cooldown_until)
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
    record_account_sync_failure_with_proxy_snapshot(
        pool,
        account_id,
        source,
        status,
        error_message,
        reason_code,
        http_status,
        failure_kind,
        preserved_route_failure_kind,
        clear_transient_route_failure_state,
        None,
    )
    .await
}

async fn record_account_sync_failure_with_proxy_snapshot(
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
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let cooldown_until =
        maintenance_sync_rejected_cooldown_until(source, reason_code, error_message, &now_iso);
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
                WHEN last_action_source = ?8
                     AND last_action_reason_code IN (?9, ?10) THEN NULL
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
    .bind(cooldown_until)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
    .bind("upstream_http_402")
    .bind("upstream_rejected")
    .execute(pool)
    .await?;
    record_upstream_account_action_with_proxy_snapshot(
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
        proxy_snapshot,
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
    record_classified_account_sync_failure_with_proxy_snapshot(
        pool,
        row,
        source,
        error_message,
        None,
    )
    .await
}

async fn record_classified_account_sync_failure_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    source: &str,
    error_message: &str,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
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
    record_account_sync_failure_with_proxy_snapshot(
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
        proxy_snapshot,
    )
    .await
}
