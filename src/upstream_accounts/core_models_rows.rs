use super::*;

#[allow(dead_code)]
#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountRow {
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) provider: String,
    pub(crate) display_name: String,
    pub(crate) group_name: Option<String>,
    pub(crate) is_mother: i64,
    pub(crate) note: Option<String>,
    pub(crate) status: String,
    pub(crate) enabled: i64,
    pub(crate) email: Option<String>,
    #[sqlx(default)]
    pub(crate) verified_email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) plan_type_observed_at: Option<String>,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) encrypted_credentials: Option<String>,
    #[sqlx(default)]
    pub(crate) has_refresh_token: Option<i64>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) last_refreshed_at: Option<String>,
    pub(crate) last_synced_at: Option<String>,
    pub(crate) last_successful_sync_at: Option<String>,
    pub(crate) last_activity_at: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_action: Option<String>,
    pub(crate) last_action_source: Option<String>,
    pub(crate) last_action_reason_code: Option<String>,
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) last_action_http_status: Option<i64>,
    pub(crate) last_action_invoke_id: Option<String>,
    pub(crate) last_action_at: Option<String>,
    pub(crate) last_selected_at: Option<String>,
    pub(crate) last_route_failure_at: Option<String>,
    pub(crate) last_route_failure_kind: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) consecutive_route_failures: i64,
    pub(crate) temporary_route_failure_streak_started_at: Option<String>,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_observed_at: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) image_tool_capability: Option<String>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) local_limit_unit: Option<String>,
    pub(crate) policy_allow_cut_out: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_allow_cut_in: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_priority_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_fast_mode_rewrite_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_image_tool_rewrite_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_request_compression_algorithm: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_concurrency_limit: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_upstream_429_retry_enabled: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_upstream_429_max_retries: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_available_models_json: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_401: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_402: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_403: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_reauth_required: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_usage_snapshot_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_quota_still_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_transport_failure: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_server_overloaded: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_5xx: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_responses_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_compact_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_image_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_responses_stream_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_compact_stream_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) bound_proxy_keys_json: Option<String>,
    pub(crate) upstream_base_url: Option<String>,
    #[sqlx(default)]
    pub(crate) external_client_id: Option<String>,
    #[sqlx(default)]
    pub(crate) external_source_account_id: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl UpstreamAccountRow {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    pub(crate) fn normalized_group_name(&self) -> Option<&str> {
        self.group_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn bound_proxy_keys(&self) -> Vec<String> {
        decode_group_bound_proxy_keys_json(self.bound_proxy_keys_json.as_deref())
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct PoolRoutingSettingsRow {
    pub(crate) encrypted_api_key: Option<String>,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) primary_sync_interval_secs: Option<i64>,
    pub(crate) secondary_sync_interval_secs: Option<i64>,
    pub(crate) priority_available_account_cap: Option<i64>,
    pub(crate) responses_first_byte_timeout_secs: Option<i64>,
    pub(crate) compact_first_byte_timeout_secs: Option<i64>,
    pub(crate) image_first_byte_timeout_secs: Option<i64>,
    pub(crate) responses_stream_timeout_secs: Option<i64>,
    pub(crate) compact_stream_timeout_secs: Option<i64>,
    pub(crate) request_compression_algorithm: Option<String>,
    pub(crate) request_compression_level_preset: Option<String>,
    pub(crate) default_first_byte_timeout_secs: Option<i64>,
    pub(crate) upstream_handshake_timeout_secs: Option<i64>,
    pub(crate) request_read_timeout_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolRoutingMaintenanceSettings {
    pub(crate) primary_sync_interval_secs: u64,
    pub(crate) secondary_sync_interval_secs: u64,
    pub(crate) priority_available_account_cap: usize,
}

impl PoolRoutingMaintenanceSettings {
    pub(crate) fn into_response(self) -> PoolRoutingMaintenanceSettingsResponse {
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
    pub(crate) id: i64,
    pub(crate) status: String,
    pub(crate) last_synced_at: Option<String>,
    pub(crate) last_action_source: Option<String>,
    pub(crate) last_action_at: Option<String>,
    pub(crate) last_selected_at: Option<String>,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_route_failure_at: Option<String>,
    pub(crate) last_route_failure_kind: Option<String>,
    pub(crate) last_action_reason_code: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) temporary_route_failure_streak_started_at: Option<String>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) primary_resets_at: Option<String>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) secondary_resets_at: Option<String>,
    pub(crate) credits_has_credits: Option<i64>,
    pub(crate) credits_unlimited: Option<i64>,
    pub(crate) credits_balance: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaintenanceDispatchPlan {
    pub(crate) account_id: i64,
    pub(crate) tier: MaintenanceTier,
    pub(crate) sync_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum OptionalField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

pub(crate) fn deserialize_optional_field<'de, D, T>(
    deserializer: D,
) -> Result<OptionalField<T>, D::Error>
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
    pub(crate) sticky_key: String,
    pub(crate) account_id: i64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountRoutingCandidateRow {
    pub(crate) id: i64,
    pub(crate) plan_type: Option<String>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) secondary_window_minutes: Option<i64>,
    pub(crate) secondary_resets_at: Option<String>,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) primary_window_minutes: Option<i64>,
    pub(crate) primary_resets_at: Option<String>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) credits_has_credits: Option<i64>,
    pub(crate) credits_unlimited: Option<i64>,
    pub(crate) credits_balance: Option<String>,
    pub(crate) last_selected_at: Option<String>,
    pub(crate) active_sticky_conversations: i64,
    #[sqlx(default)]
    pub(crate) in_flight_reservations: i64,
}

impl AccountRoutingCandidateRow {
    pub(crate) fn effective_load(&self) -> i64 {
        self.active_sticky_conversations
            .saturating_add(self.in_flight_reservations.max(0))
    }

    pub(crate) fn capacity_profile(&self) -> RoutingCapacityProfile {
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

    pub(crate) fn scarcity_score(&self, now: DateTime<Utc>) -> f64 {
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
pub(crate) struct RoutingCapacityProfile {
    pub(crate) soft_limit: i64,
    pub(crate) hard_cap: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NormalizedRoutingPressure {
    pub(crate) short_pressure: Option<f64>,
    pub(crate) long_pressure: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RoutingWindowSignals {
    pub(crate) short_signal: bool,
    pub(crate) long_signal: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RoutingWindowState {
    pub(crate) bucket: RoutingWindowBucket,
    pub(crate) pressure: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutingWindowBucket {
    Short,
    Long,
}

pub(crate) fn routing_window_state(
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

pub(crate) fn normalize_unit_ratio(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

#[derive(Debug, FromRow)]
pub(crate) struct AccountActiveConversationCountRow {
    pub(crate) account_id: i64,
    pub(crate) active_conversation_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TagRow {
    pub(crate) name: String,
    #[sqlx(default)]
    pub(crate) system_key: Option<String>,
    #[sqlx(default)]
    pub(crate) protected: i64,
    pub(crate) allow_cut_out: i64,
    pub(crate) allow_cut_in: i64,
    pub(crate) priority_tier: String,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) concurrency_limit: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_retry_enabled: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_max_retries: i64,
    #[sqlx(default)]
    pub(crate) available_models_json: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountTagRow {
    pub(crate) account_id: i64,
    pub(crate) tag_id: i64,
    pub(crate) name: String,
    #[sqlx(default)]
    pub(crate) system_key: Option<String>,
    #[sqlx(default)]
    pub(crate) protected: i64,
    pub(crate) allow_cut_out: i64,
    pub(crate) allow_cut_in: i64,
    pub(crate) priority_tier: String,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) concurrency_limit: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_retry_enabled: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_max_retries: i64,
    #[sqlx(default)]
    pub(crate) available_models_json: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TagListRow {
    pub(crate) id: i64,
    pub(crate) name: String,
    #[sqlx(default)]
    pub(crate) system_key: Option<String>,
    #[sqlx(default)]
    pub(crate) protected: i64,
    pub(crate) allow_cut_out: i64,
    pub(crate) allow_cut_in: i64,
    pub(crate) priority_tier: String,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) concurrency_limit: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_retry_enabled: i64,
    #[sqlx(default)]
    pub(crate) upstream_429_max_retries: i64,
    #[sqlx(default)]
    pub(crate) available_models_json: Option<String>,
    pub(crate) updated_at: String,
    pub(crate) account_count: i64,
    pub(crate) group_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountGroupListRow {
    pub(crate) group_name: String,
    pub(crate) account_count: i64,
    pub(crate) note: Option<String>,
    pub(crate) bound_proxy_keys_json: Option<String>,
    pub(crate) node_shunt_enabled: Option<i64>,
    pub(crate) single_account_rotation_enabled: Option<i64>,
    pub(crate) upstream_429_retry_enabled: Option<i64>,
    pub(crate) upstream_429_max_retries: Option<i64>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) policy_allow_cut_out: Option<i64>,
    pub(crate) policy_allow_cut_in: Option<i64>,
    pub(crate) policy_priority_tier: Option<String>,
    pub(crate) policy_fast_mode_rewrite_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_image_tool_rewrite_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_request_compression_algorithm: Option<String>,
    pub(crate) policy_concurrency_limit: Option<i64>,
    pub(crate) policy_upstream_429_retry_enabled: Option<i64>,
    pub(crate) policy_upstream_429_max_retries: Option<i64>,
    pub(crate) policy_available_models_json: Option<String>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_401: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_402: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_403: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_reauth_required: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_usage_snapshot_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_quota_still_exhausted: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_transport_failure: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_server_overloaded: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_status_change_upstream_http_5xx: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_responses_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_compact_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_image_first_byte_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_responses_stream_timeout_secs: Option<i64>,
    #[sqlx(default)]
    pub(crate) policy_compact_stream_timeout_secs: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct StickyKeyAggregateRow {
    pub(crate) sticky_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) created_at: String,
    pub(crate) last_activity_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct AccountLastActivityRow {
    pub(crate) account_id: i64,
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountWindowUsageRow {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: i64,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountWindowUsageHourlyRow {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) upstream_account_id: i64,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AccountWindowUsageMinuteRow {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) upstream_account_id: i64,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AccountWindowUsageAccumulator {
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
}

impl AccountWindowUsageAccumulator {
    pub(crate) fn merge(&mut self, other: Self) {
        self.request_count += other.request_count;
        self.total_tokens += other.total_tokens;
        self.total_cost += other.total_cost;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_input_tokens += other.cache_input_tokens;
    }

    pub(crate) fn add_row(&mut self, row: &AccountWindowUsageRow) {
        self.request_count += 1;
        self.total_tokens += row.total_tokens.unwrap_or_default();
        self.total_cost += row.cost.unwrap_or_default();
        self.input_tokens += row.input_tokens.unwrap_or_default();
        self.output_tokens += row.output_tokens.unwrap_or_default();
        self.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
    }

    pub(crate) fn add_hourly_row(&mut self, row: &AccountWindowUsageHourlyRow) {
        self.request_count += row.request_count.max(0);
        self.total_tokens += row.total_tokens.max(0);
        self.total_cost += row.total_cost.max(0.0);
        self.input_tokens += row.input_tokens.max(0);
        self.output_tokens += row.output_tokens.max(0);
        self.cache_input_tokens += row.cache_input_tokens.max(0);
    }

    pub(crate) fn add_minute_row(&mut self, row: &AccountWindowUsageMinuteRow) {
        self.request_count += row.request_count.max(0);
        self.total_tokens += row.total_tokens.max(0);
        self.total_cost += row.total_cost.max(0.0);
        self.input_tokens += row.input_tokens.max(0);
        self.output_tokens += row.output_tokens.max(0);
        self.cache_input_tokens += row.cache_input_tokens.max(0);
    }

    pub(crate) fn into_snapshot(self) -> RateWindowActualUsage {
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
pub(crate) struct AccountWindowUsagePlan {
    pub(crate) primary: Option<AccountWindowUsageRange>,
    pub(crate) secondary: Option<AccountWindowUsageRange>,
}

#[derive(Debug, Clone)]
pub(crate) struct AccountWindowUsageRange {
    pub(crate) start_at: String,
    pub(crate) end_at: String,
    pub(crate) start_at_epoch: i64,
    pub(crate) end_at_epoch: i64,
    pub(crate) full_minute_start_epoch: Option<i64>,
    pub(crate) full_minute_end_epoch: Option<i64>,
    pub(crate) full_hour_start_epoch: Option<i64>,
    pub(crate) full_hour_end_epoch: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AccountWindowUsageRangeBounds {
    pub(crate) start_at: DateTime<Utc>,
    pub(crate) end_at: DateTime<Utc>,
}

impl AccountWindowUsageRangeBounds {
    pub(crate) fn into_range(self) -> AccountWindowUsageRange {
        let start_epoch = self.start_at.timestamp();
        let end_epoch = self.end_at.timestamp();
        let full_minute_start_epoch = if start_epoch.rem_euclid(60) == 0 {
            start_epoch
        } else {
            align_bucket_epoch(start_epoch.saturating_add(59), 60, 0)
        };
        let full_minute_end_epoch = align_bucket_epoch(end_epoch, 60, 0);
        let full_hour_start_epoch = if start_epoch.rem_euclid(3_600) == 0 {
            start_epoch
        } else {
            align_bucket_epoch(start_epoch.saturating_add(3_599), 3_600, 0)
        };
        let full_hour_end_epoch = align_bucket_epoch(end_epoch, 3_600, 0);
        AccountWindowUsageRange {
            start_at: format_naive(self.start_at.with_timezone(&Shanghai).naive_local()),
            end_at: format_naive(self.end_at.with_timezone(&Shanghai).naive_local()),
            start_at_epoch: start_epoch,
            end_at_epoch: end_epoch,
            full_minute_start_epoch: (full_minute_start_epoch < full_minute_end_epoch)
                .then_some(full_minute_start_epoch),
            full_minute_end_epoch: (full_minute_start_epoch < full_minute_end_epoch)
                .then_some(full_minute_end_epoch),
            full_hour_start_epoch: (full_hour_start_epoch < full_hour_end_epoch)
                .then_some(full_hour_start_epoch),
            full_hour_end_epoch: (full_hour_start_epoch < full_hour_end_epoch)
                .then_some(full_hour_end_epoch),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AccountWindowUsageSummary {
    pub(crate) primary: AccountWindowUsageAccumulator,
    pub(crate) secondary: AccountWindowUsageAccumulator,
}

impl AccountWindowUsageSummary {
    pub(crate) fn merge(&mut self, other: Self) {
        self.primary.merge(other.primary);
        self.secondary.merge(other.secondary);
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountActionEventRow {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) action: String,
    pub(crate) source: String,
    pub(crate) account_display_name: Option<String>,
    pub(crate) account_group_name: Option<String>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_display_name: Option<String>,
    pub(crate) forward_proxy_egress_ip: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) result_description: Option<String>,
    pub(crate) reason_code: Option<String>,
    pub(crate) reason_message: Option<String>,
    pub(crate) http_status: Option<i64>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) attempt_public_id: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) created_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct StickyKeyEventRow {
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) request_tokens: i64,
    pub(crate) sticky_key: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct AccountStickyKeyInvocationPreviewRow {
    pub(crate) sticky_key: String,
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_status_code: Option<i64>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) endpoint: Option<String>,
    pub(crate) compaction_request_kind: Option<String>,
    pub(crate) compaction_response_kind: Option<String>,
    pub(crate) image_intent: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountSampleRow {
    pub(crate) captured_at: String,
    pub(crate) limit_id: Option<String>,
    pub(crate) limit_name: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) primary_window_minutes: Option<i64>,
    pub(crate) primary_resets_at: Option<String>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) secondary_window_minutes: Option<i64>,
    pub(crate) secondary_resets_at: Option<String>,
    pub(crate) credits_has_credits: Option<i64>,
    pub(crate) credits_unlimited: Option<i64>,
    pub(crate) credits_balance: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub(crate) struct OauthLoginSessionRow {
    pub(crate) login_id: String,
    pub(crate) account_id: Option<i64>,
    pub(crate) display_name: Option<String>,
    #[sqlx(default)]
    pub(crate) email: Option<String>,
    pub(crate) group_name: Option<String>,
    pub(crate) group_bound_proxy_keys_json: Option<String>,
    pub(crate) group_node_shunt_enabled: i64,
    pub(crate) group_node_shunt_enabled_requested: i64,
    pub(crate) group_single_account_rotation_enabled: i64,
    pub(crate) group_single_account_rotation_enabled_requested: i64,
    pub(crate) is_mother: i64,
    pub(crate) note: Option<String>,
    pub(crate) tag_ids_json: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) group_concurrency_limit: i64,
    pub(crate) mailbox_session_id: Option<String>,
    pub(crate) mailbox_address: Option<String>,
    pub(crate) state: String,
    pub(crate) pkce_verifier: String,
    pub(crate) redirect_uri: String,
    pub(crate) status: String,
    pub(crate) auth_url: String,
    pub(crate) error_message: Option<String>,
    pub(crate) pending_encrypted_credentials: Option<String>,
    pub(crate) pending_token_expires_at: Option<String>,
    pub(crate) pending_verified_email: Option<String>,
    pub(crate) pending_chatgpt_account_id: Option<String>,
    pub(crate) pending_chatgpt_user_id: Option<String>,
    pub(crate) pending_plan_type: Option<String>,
    pub(crate) pending_has_refresh_token: Option<i64>,
    pub(crate) expires_at: String,
    pub(crate) consumed_at: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub(crate) struct OauthMailboxSessionRow {
    pub(crate) session_id: String,
    pub(crate) remote_email_id: String,
    pub(crate) email_address: String,
    pub(crate) email_domain: String,
    pub(crate) mailbox_source: Option<String>,
    pub(crate) latest_code_value: Option<String>,
    pub(crate) latest_code_source: Option<String>,
    pub(crate) latest_code_updated_at: Option<String>,
    pub(crate) invite_subject: Option<String>,
    pub(crate) invite_copy_value: Option<String>,
    pub(crate) invite_copy_label: Option<String>,
    pub(crate) invite_updated_at: Option<String>,
    pub(crate) invited: i64,
    pub(crate) last_message_id: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) expires_at: String,
}
