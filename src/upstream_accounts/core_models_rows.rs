#[allow(dead_code)]
#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountRow {
    id: i64,
    kind: String,
    provider: String,
    display_name: String,
    group_name: Option<String>,
    is_mother: i64,
    note: Option<String>,
    status: String,
    enabled: i64,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    plan_type: Option<String>,
    plan_type_observed_at: Option<String>,
    masked_api_key: Option<String>,
    encrypted_credentials: Option<String>,
    token_expires_at: Option<String>,
    last_refreshed_at: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_activity_at: Option<String>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    last_action: Option<String>,
    last_action_source: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_reason_message: Option<String>,
    last_action_http_status: Option<i64>,
    last_action_invoke_id: Option<String>,
    last_action_at: Option<String>,
    last_selected_at: Option<String>,
    last_route_failure_at: Option<String>,
    last_route_failure_kind: Option<String>,
    cooldown_until: Option<String>,
    consecutive_route_failures: i64,
    temporary_route_failure_streak_started_at: Option<String>,
    compact_support_status: Option<String>,
    compact_support_observed_at: Option<String>,
    compact_support_reason: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
    upstream_base_url: Option<String>,
    #[sqlx(default)]
    external_client_id: Option<String>,
    #[sqlx(default)]
    external_source_account_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PoolRoutingSettingsRow {
    encrypted_api_key: Option<String>,
    masked_api_key: Option<String>,
    primary_sync_interval_secs: Option<i64>,
    secondary_sync_interval_secs: Option<i64>,
    priority_available_account_cap: Option<i64>,
    responses_first_byte_timeout_secs: Option<i64>,
    compact_first_byte_timeout_secs: Option<i64>,
    responses_stream_timeout_secs: Option<i64>,
    compact_stream_timeout_secs: Option<i64>,
    default_first_byte_timeout_secs: Option<i64>,
    upstream_handshake_timeout_secs: Option<i64>,
    request_read_timeout_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolRoutingMaintenanceSettings {
    primary_sync_interval_secs: u64,
    secondary_sync_interval_secs: u64,
    priority_available_account_cap: usize,
}

impl PoolRoutingMaintenanceSettings {
    fn into_response(self) -> PoolRoutingMaintenanceSettingsResponse {
        PoolRoutingMaintenanceSettingsResponse {
            primary_sync_interval_secs: self.primary_sync_interval_secs,
            secondary_sync_interval_secs: self.secondary_sync_interval_secs,
            priority_available_account_cap: self.priority_available_account_cap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaintenanceTier {
    HighFrequency,
    Priority,
    Secondary,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct MaintenanceCandidateRow {
    id: i64,
    status: String,
    last_synced_at: Option<String>,
    last_action_source: Option<String>,
    last_action_at: Option<String>,
    last_selected_at: Option<String>,
    last_error_at: Option<String>,
    last_error: Option<String>,
    last_route_failure_at: Option<String>,
    last_route_failure_kind: Option<String>,
    last_action_reason_code: Option<String>,
    cooldown_until: Option<String>,
    temporary_route_failure_streak_started_at: Option<String>,
    token_expires_at: Option<String>,
    primary_used_percent: Option<f64>,
    primary_resets_at: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_resets_at: Option<String>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaintenanceDispatchPlan {
    account_id: i64,
    tier: MaintenanceTier,
    sync_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum OptionalField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<OptionalField<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    if raw.is_null() {
        return Ok(OptionalField::Null);
    }

    serde_json::from_value(raw)
        .map(OptionalField::Value)
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
pub(crate) struct PoolStickyRouteRow {
    sticky_key: String,
    account_id: i64,
    created_at: String,
    updated_at: String,
    last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountRoutingCandidateRow {
    id: i64,
    plan_type: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
    last_selected_at: Option<String>,
    active_sticky_conversations: i64,
    #[sqlx(default)]
    in_flight_reservations: i64,
}

impl AccountRoutingCandidateRow {
    fn effective_load(&self) -> i64 {
        self.active_sticky_conversations
            .saturating_add(self.in_flight_reservations.max(0))
    }

    fn capacity_profile(&self) -> RoutingCapacityProfile {
        let signals = self.window_signals();
        if signals.short_signal {
            RoutingCapacityProfile {
                soft_limit: 2,
                hard_cap: 3,
            }
        } else if signals.long_signal {
            RoutingCapacityProfile {
                soft_limit: 1,
                hard_cap: 2,
            }
        } else {
            RoutingCapacityProfile {
                soft_limit: 2,
                hard_cap: 3,
            }
        }
    }

    fn normalized_window_pressure(&self, now: DateTime<Utc>) -> NormalizedRoutingPressure {
        let mut short_pressure = None;
        let mut long_pressure = None;
        for window in [
            routing_window_state(
                self.primary_used_percent,
                self.primary_window_minutes,
                self.primary_resets_at.as_deref(),
                now,
                RoutingWindowBucket::Short,
            ),
            routing_window_state(
                self.secondary_used_percent,
                self.secondary_window_minutes,
                self.secondary_resets_at.as_deref(),
                now,
                RoutingWindowBucket::Long,
            ),
        ]
        .into_iter()
        .flatten()
        {
            match window.bucket {
                RoutingWindowBucket::Short => {
                    short_pressure = Some(short_pressure.unwrap_or(0.0_f64).max(window.pressure));
                }
                RoutingWindowBucket::Long => {
                    long_pressure = Some(long_pressure.unwrap_or(0.0_f64).max(window.pressure));
                }
            }
        }
        NormalizedRoutingPressure {
            short_pressure,
            long_pressure,
        }
    }

    fn window_signals(&self) -> RoutingWindowSignals {
        let mut short_signal = false;
        let mut long_signal = false;
        for window_minutes in [self.primary_window_minutes, self.secondary_window_minutes]
            .into_iter()
            .flatten()
        {
            if window_minutes <= 360 {
                short_signal = true;
            } else {
                long_signal = true;
            }
        }
        if self.primary_window_minutes.is_none()
            && (self.primary_used_percent.is_some() || self.local_primary_limit.is_some())
        {
            short_signal = true;
        }
        if self.secondary_window_minutes.is_none()
            && (self.secondary_used_percent.is_some() || self.local_secondary_limit.is_some())
        {
            long_signal = true;
        }
        RoutingWindowSignals {
            short_signal,
            long_signal,
        }
    }

    fn scarcity_score(&self, now: DateTime<Utc>) -> f64 {
        let pressure = self.normalized_window_pressure(now);
        match (pressure.short_pressure, pressure.long_pressure) {
            (Some(short), Some(long)) => (0.65 * short) + (0.35 * long),
            (Some(short), None) => short,
            (None, Some(long)) => long,
            (None, None) => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RoutingCapacityProfile {
    soft_limit: i64,
    hard_cap: i64,
}

#[derive(Debug, Clone, Copy)]
struct NormalizedRoutingPressure {
    short_pressure: Option<f64>,
    long_pressure: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct RoutingWindowSignals {
    short_signal: bool,
    long_signal: bool,
}

#[derive(Debug, Clone, Copy)]
struct RoutingWindowState {
    bucket: RoutingWindowBucket,
    pressure: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingWindowBucket {
    Short,
    Long,
}

fn routing_window_state(
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    resets_at: Option<&str>,
    now: DateTime<Utc>,
    default_bucket: RoutingWindowBucket,
) -> Option<RoutingWindowState> {
    let used_ratio = normalize_unit_ratio(used_percent? / 100.0);
    let (bucket, remaining_ratio) = if let Some(window_minutes) = window_minutes {
        let window_minutes = window_minutes.max(1);
        let bucket = if window_minutes <= 360 {
            RoutingWindowBucket::Short
        } else {
            RoutingWindowBucket::Long
        };
        let window_duration_secs = (window_minutes as f64) * 60.0;
        let remaining_ratio = resets_at
            .and_then(parse_rfc3339_utc)
            .map(|reset_at| {
                normalize_unit_ratio(
                    (reset_at - now).num_seconds().max(0) as f64 / window_duration_secs,
                )
            })
            .unwrap_or(1.0);
        (bucket, remaining_ratio)
    } else {
        (default_bucket, 1.0)
    };
    Some(RoutingWindowState {
        bucket,
        pressure: used_ratio * remaining_ratio,
    })
}

fn normalize_unit_ratio(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

#[derive(Debug, FromRow)]
struct AccountActiveConversationCountRow {
    account_id: i64,
    active_conversation_count: i64,
}

#[derive(Debug, Clone, FromRow)]
struct TagRow {
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
    priority_tier: String,
    fast_mode_rewrite_mode: String,
    concurrency_limit: i64,
}

#[derive(Debug, Clone, FromRow)]
struct AccountTagRow {
    account_id: i64,
    tag_id: i64,
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
    priority_tier: String,
    fast_mode_rewrite_mode: String,
    concurrency_limit: i64,
}

#[derive(Debug, Clone, FromRow)]
struct TagListRow {
    id: i64,
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
    priority_tier: String,
    fast_mode_rewrite_mode: String,
    concurrency_limit: i64,
    updated_at: String,
    account_count: i64,
    group_count: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct StickyKeyAggregateRow {
    sticky_key: String,
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    created_at: String,
    last_activity_at: String,
}

#[derive(Debug, FromRow)]
struct AccountLastActivityRow {
    account_id: i64,
    last_activity_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct AccountWindowUsageRow {
    occurred_at: String,
    upstream_account_id: i64,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountWindowUsageAccumulator {
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
}

impl AccountWindowUsageAccumulator {
    fn add_row(&mut self, row: &AccountWindowUsageRow) {
        self.request_count += 1;
        self.total_tokens += row.total_tokens.unwrap_or_default();
        self.total_cost += row.cost.unwrap_or_default();
        self.input_tokens += row.input_tokens.unwrap_or_default();
        self.output_tokens += row.output_tokens.unwrap_or_default();
        self.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
    }

    fn into_snapshot(self) -> RateWindowActualUsage {
        RateWindowActualUsage {
            request_count: self.request_count,
            total_tokens: self.total_tokens,
            total_cost: self.total_cost,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_input_tokens: self.cache_input_tokens,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct AccountWindowUsagePlan {
    primary: Option<AccountWindowUsageRange>,
    secondary: Option<AccountWindowUsageRange>,
}

#[derive(Debug, Clone)]
struct AccountWindowUsageRange {
    start_at: String,
    end_at: String,
}

#[derive(Debug, Clone, Copy)]
struct AccountWindowUsageRangeBounds {
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

impl AccountWindowUsageRangeBounds {
    fn into_range(self) -> AccountWindowUsageRange {
        AccountWindowUsageRange {
            start_at: format_naive(self.start_at.with_timezone(&Shanghai).naive_local()),
            end_at: format_naive(self.end_at.with_timezone(&Shanghai).naive_local()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountWindowUsageSummary {
    primary: AccountWindowUsageAccumulator,
    secondary: AccountWindowUsageAccumulator,
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountActionEventRow {
    id: i64,
    occurred_at: String,
    action: String,
    source: String,
    reason_code: Option<String>,
    reason_message: Option<String>,
    http_status: Option<i64>,
    failure_kind: Option<String>,
    invoke_id: Option<String>,
    sticky_key: Option<String>,
    created_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct StickyKeyEventRow {
    occurred_at: String,
    status: String,
    request_tokens: i64,
    sticky_key: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct AccountStickyKeyInvocationPreviewRow {
    sticky_key: String,
    id: i64,
    invoke_id: String,
    occurred_at: String,
    status: String,
    failure_class: Option<String>,
    route_mode: Option<String>,
    model: Option<String>,
    total_tokens: i64,
    cost: Option<f64>,
    source: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    reasoning_effort: Option<String>,
    error_message: Option<String>,
    downstream_status_code: Option<i64>,
    downstream_error_message: Option<String>,
    failure_kind: Option<String>,
    is_actionable: Option<i64>,
    proxy_display_name: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    response_content_encoding: Option<String>,
    requested_service_tier: Option<String>,
    service_tier: Option<String>,
    billing_service_tier: Option<String>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
    t_total_ms: Option<f64>,
    endpoint: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct UpstreamAccountSampleRow {
    captured_at: String,
    limit_id: Option<String>,
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub(crate) struct OauthLoginSessionRow {
    login_id: String,
    account_id: Option<i64>,
    display_name: Option<String>,
    group_name: Option<String>,
    group_bound_proxy_keys_json: Option<String>,
    group_node_shunt_enabled: i64,
    group_node_shunt_enabled_requested: i64,
    is_mother: i64,
    note: Option<String>,
    tag_ids_json: Option<String>,
    group_note: Option<String>,
    group_concurrency_limit: i64,
    mailbox_session_id: Option<String>,
    mailbox_address: Option<String>,
    state: String,
    pkce_verifier: String,
    redirect_uri: String,
    status: String,
    auth_url: String,
    error_message: Option<String>,
    expires_at: String,
    consumed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct OauthMailboxSessionRow {
    session_id: String,
    remote_email_id: String,
    email_address: String,
    email_domain: String,
    mailbox_source: Option<String>,
    latest_code_value: Option<String>,
    latest_code_source: Option<String>,
    latest_code_updated_at: Option<String>,
    invite_subject: Option<String>,
    invite_copy_value: Option<String>,
    invite_copy_label: Option<String>,
    invite_updated_at: Option<String>,
    invited: i64,
    last_message_id: Option<String>,
    created_at: String,
    updated_at: String,
    expires_at: String,
}
