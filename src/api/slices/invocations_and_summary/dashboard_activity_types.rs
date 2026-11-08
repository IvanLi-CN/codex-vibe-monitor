use super::*;
#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountActivityMetaRow {
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) display_name: Option<String>,
    pub(crate) group_name: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) status: String,
    pub(crate) enabled: i64,
    pub(crate) last_error: Option<String>,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_route_failure_at: Option<String>,
    pub(crate) last_route_failure_kind: Option<String>,
    pub(crate) last_action_reason_code: Option<String>,
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) temporary_route_failure_streak_started_at: Option<String>,
    pub(crate) last_selected_at: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct UpstreamAccountInProgressSummary {
    pub(crate) in_progress_count: i64,
    pub(crate) retry_count: i64,
    pub(crate) phase_counts: InvocationPhaseCountsResponse,
    pub(crate) wait_sum_ms: f64,
    pub(crate) wait_sample_count: i64,
}

impl UpstreamAccountInProgressSummary {
    pub(crate) fn add(&mut self, retry: bool, phase: Option<&str>, wait_ms: Option<f64>) {
        self.in_progress_count += 1;
        if retry {
            self.retry_count += 1;
        }
        self.phase_counts.increment_phase_name(phase);
        if let Some(wait_ms) = wait_ms.filter(|value| value.is_finite() && *value >= 0.0) {
            self.wait_sum_ms += wait_ms;
            self.wait_sample_count += 1;
        }
    }

    pub(crate) fn subtract(&mut self, retry: bool, phase: Option<&str>, wait_ms: Option<f64>) {
        self.in_progress_count = self.in_progress_count.saturating_sub(1);
        if retry {
            self.retry_count = self.retry_count.saturating_sub(1);
        }
        self.phase_counts.decrement_phase_name(phase);
        if let Some(wait_ms) = wait_ms.filter(|value| value.is_finite() && *value >= 0.0) {
            self.wait_sum_ms = (self.wait_sum_ms - wait_ms).max(0.0);
            self.wait_sample_count = self.wait_sample_count.saturating_sub(1);
        }
    }
}

pub(crate) fn normalized_wait_ms(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

#[path = "../invocations_and_summary_numeric.rs"]
mod summary_numeric;
pub(crate) use summary_numeric::{
    checked_summary_live_add, checked_summary_live_add_i64, checked_summary_live_subtract,
    merge_invocation_summary_totals, summary_live_numeric_overflow, summary_live_sum_from_average,
};

#[cfg(test)]
#[path = "../invocations_and_summary_numeric_tests.rs"]
mod summary_live_numeric_tests;

pub(crate) fn runtime_record_ttfb_differs(left: Option<f64>, right: Option<f64>) -> bool {
    normalized_wait_ms(left) != normalized_wait_ms(right)
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountActivityAccumulator {
    pub(crate) display_name_hint: Option<String>,
    pub(crate) plan_type_hint: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) non_success_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) success_tokens: i64,
    pub(crate) non_success_tokens: i64,
    pub(crate) failure_tokens: i64,
    pub(crate) failure_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) latest_first_response_byte_total_at: Option<String>,
    pub(crate) latest_first_response_byte_total_ms: Option<f64>,
    pub(crate) latest_avg_total_at: Option<String>,
    pub(crate) latest_avg_total_ms: Option<f64>,
    pub(crate) in_progress_wait_sample_count: i64,
    pub(crate) in_progress_wait_sum_ms: f64,
    pub(crate) last_occurred_at_epoch_ms: i64,
    pub(crate) latest_conversation_created_at: Option<String>,
    pub(crate) last_invocation_at: Option<String>,
    pub(crate) rate_usage_events: Vec<UpstreamAccountRateUsageEvent>,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) usage_breakdown: UsageBreakdownAccumulator,
    pub(crate) model_performance: ModelPerformanceAccumulator,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LatestTimedMetricValue {
    pub(crate) at: Option<String>,
    pub(crate) value: Option<f64>,
}

impl LatestTimedMetricValue {
    pub(crate) fn update(&mut self, candidate_at: Option<String>, candidate_value: Option<f64>) {
        let Some(candidate_at) = candidate_at else {
            return;
        };
        let Some(candidate_value) = candidate_value.filter(|value| value.is_finite()) else {
            return;
        };
        let replace = match self.at.as_deref() {
            Some(current_at) => candidate_at.as_str() >= current_at,
            None => true,
        };
        if replace {
            self.at = Some(candidate_at);
            self.value = Some(candidate_value);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct UsageBreakdownGroupKey {
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct UsageBreakdownAccumulator {
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) costs: UsageCostBreakdownResponse,
    pub(crate) has_cost: bool,
    pub(crate) models: HashMap<UsageBreakdownGroupKey, UsageBreakdownAccumulator>,
}

impl UsageBreakdownAccumulator {
    pub(crate) fn add_cost_row(&mut self, total_cost: Option<f64>, costs: [Option<f64>; 5]) {
        let Some(total_cost) = total_cost else {
            return;
        };
        self.has_cost = true;
        if costs.iter().all(Option::is_some) {
            self.costs.input += costs[0].unwrap_or_default();
            self.costs.cache_write += costs[1].unwrap_or_default();
            self.costs.cache_read += costs[2].unwrap_or_default();
            self.costs.output += costs[3].unwrap_or_default();
            self.costs.reasoning += costs[4].unwrap_or_default();
        } else {
            self.costs.unknown += total_cost;
        }
    }

    pub(crate) fn merge_costs(&mut self, costs: &UsageCostBreakdownResponse) {
        self.has_cost = true;
        self.costs.input += costs.input;
        self.costs.cache_write += costs.cache_write;
        self.costs.cache_read += costs.cache_read;
        self.costs.output += costs.output;
        self.costs.reasoning += costs.reasoning;
        self.costs.unknown += costs.unknown;
    }

    pub(crate) fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
        let cache_read_tokens = row.cache_input_tokens.unwrap_or_default().max(0);
        let cache_write_tokens = row
            .input_tokens
            .unwrap_or_default()
            .max(0)
            .saturating_sub(cache_read_tokens);
        self.cache_write_tokens += cache_write_tokens;
        self.cache_read_tokens += cache_read_tokens;
        self.output_tokens += row.output_tokens.unwrap_or_default().max(0);

        let costs = [
            row.cost_input,
            row.cost_cache_write,
            row.cost_cache_read,
            row.cost_output,
            row.cost_reasoning,
        ];
        self.add_cost_row(row.cost, costs);

        let model = row
            .response_model
            .as_deref()
            .or(row.model.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let reasoning_effort = row
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let model_entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model,
                reasoning_effort,
            })
            .or_default();
        model_entry.cache_write_tokens += cache_write_tokens;
        model_entry.cache_read_tokens += cache_read_tokens;
        model_entry.output_tokens += row.output_tokens.unwrap_or_default().max(0);
        model_entry.add_cost_row(row.cost, costs);
    }

    pub(crate) fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.cache_write_tokens += row.cache_write_tokens;
        self.cache_read_tokens += row.cache_read_tokens;
        self.output_tokens += row.output_tokens;
        if row.has_cost > 0 {
            self.has_cost = true;
            self.costs.input += row.cost_input;
            self.costs.cache_write += row.cost_cache_write;
            self.costs.cache_read += row.cost_cache_read;
            self.costs.output += row.cost_output;
            self.costs.reasoning += row.cost_reasoning;
            self.costs.unknown += row.cost_unknown;
        }

        let model_entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
            })
            .or_default();
        model_entry.cache_write_tokens += row.cache_write_tokens;
        model_entry.cache_read_tokens += row.cache_read_tokens;
        model_entry.output_tokens += row.output_tokens;
        if row.has_cost > 0 {
            model_entry.has_cost = true;
            model_entry.costs.input += row.cost_input;
            model_entry.costs.cache_write += row.cost_cache_write;
            model_entry.costs.cache_read += row.cost_cache_read;
            model_entry.costs.output += row.cost_output;
            model_entry.costs.reasoning += row.cost_reasoning;
            model_entry.costs.unknown += row.cost_unknown;
        }
    }

    pub(crate) fn add_coarse_rollup_totals(
        &mut self,
        cache_write_tokens: i64,
        cache_read_tokens: i64,
        output_tokens: i64,
        total_cost: f64,
    ) {
        self.cache_write_tokens += cache_write_tokens.max(0);
        self.cache_read_tokens += cache_read_tokens.max(0);
        self.output_tokens += output_tokens.max(0);
        if total_cost > 0.0 {
            self.add_cost_row(Some(total_cost), [None, None, None, None, None]);
        }
    }

    pub(crate) fn merge_response(&mut self, response: &UsageBreakdownResponse) {
        self.cache_write_tokens += response.cache_write_tokens;
        self.cache_read_tokens += response.cache_read_tokens;
        self.output_tokens += response.output_tokens;
        if let Some(costs) = &response.costs {
            self.merge_costs(costs);
        }
        for model in &response.models {
            let entry = self
                .models
                .entry(UsageBreakdownGroupKey {
                    model: model.model.clone(),
                    reasoning_effort: model.reasoning_effort.clone(),
                })
                .or_default();
            entry.cache_write_tokens += model.cache_write_tokens;
            entry.cache_read_tokens += model.cache_read_tokens;
            entry.output_tokens += model.output_tokens;
            if let Some(costs) = &model.costs {
                entry.merge_costs(costs);
            }
        }
    }

    pub(crate) fn into_response(self) -> UsageBreakdownResponse {
        let mut models = self
            .models
            .into_iter()
            .filter_map(|(group, entry)| {
                let has_usage = entry.cache_write_tokens > 0
                    || entry.cache_read_tokens > 0
                    || entry.output_tokens > 0
                    || entry.costs.input != 0.0
                    || entry.costs.cache_write != 0.0
                    || entry.costs.cache_read != 0.0
                    || entry.costs.output != 0.0
                    || entry.costs.reasoning != 0.0
                    || entry.costs.unknown != 0.0
                    || entry.has_cost;
                has_usage.then_some(UsageBreakdownModelResponse {
                    model: group.model,
                    reasoning_effort: group.reasoning_effort,
                    cache_write_tokens: entry.cache_write_tokens,
                    cache_read_tokens: entry.cache_read_tokens,
                    output_tokens: entry.output_tokens,
                    costs: entry.has_cost.then_some(entry.costs),
                })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            left.model
                .cmp(&right.model)
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        UsageBreakdownResponse {
            cache_write_tokens: self.cache_write_tokens,
            cache_read_tokens: self.cache_read_tokens,
            output_tokens: self.output_tokens,
            costs: self.has_cost.then_some(self.costs),
            models,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountRateUsageEvent {
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardActivityCurrentMinuteAccumulator {
    pub(crate) qualified_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) response_duration_sample_count: i64,
    pub(crate) response_duration_sum_ms: f64,
}

impl DashboardActivityCurrentMinuteAccumulator {
    pub(crate) fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
        self.total_cost += row.cost.unwrap_or_default().max(0.0);

        let classification = resolve_failure_classification(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        let is_success =
            prompt_cache_and_timeseries_shared::prompt_invocation_status_is_success_like(
                Some(row.status.as_str()),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
        let is_qualified_tpm = is_success && row.cost.is_some();

        if is_qualified_tpm {
            self.qualified_tokens += row.total_tokens.max(0);
        }
        if let Some(first_token_ms) = row
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count += 1;
            self.first_token_sum_ms += first_token_ms;
        }
        if !is_success {
            return;
        }

        if let Some(first_response_byte_total_ms) =
            crate::stats::resolve_first_response_byte_total_ms(
                row.t_req_read_ms,
                row.t_req_parse_ms,
                row.t_upstream_connect_ms,
                row.t_upstream_ttfb_ms,
            )
        {
            self.first_response_byte_total_sample_count += 1;
            self.first_response_byte_total_sum_ms += first_response_byte_total_ms;
        }
        if let Some(total_ms) = row
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.total_latency_sample_count += 1;
            self.total_latency_sum_ms += total_ms;
        }
        if let Some(response_duration_ms) = row
            .t_upstream_stream_ms
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            self.response_duration_sample_count += 1;
            self.response_duration_sum_ms += response_duration_ms;
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.qualified_tokens += other.qualified_tokens;
        self.total_cost += other.total_cost;
        self.first_response_byte_total_sample_count += other.first_response_byte_total_sample_count;
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms;
        self.first_token_sample_count += other.first_token_sample_count;
        self.first_token_sum_ms += other.first_token_sum_ms;
        self.total_latency_sample_count += other.total_latency_sample_count;
        self.total_latency_sum_ms += other.total_latency_sum_ms;
        self.response_duration_sample_count += other.response_duration_sample_count;
        self.response_duration_sum_ms += other.response_duration_sum_ms;
    }

    pub(crate) fn first_response_byte_total_avg_ms(&self) -> Option<f64> {
        (self.first_response_byte_total_sample_count > 0).then_some(
            self.first_response_byte_total_sum_ms
                / self.first_response_byte_total_sample_count as f64,
        )
    }

    pub(crate) fn avg_total_ms(&self) -> Option<f64> {
        (self.total_latency_sample_count > 0)
            .then_some(self.total_latency_sum_ms / self.total_latency_sample_count as f64)
    }

    pub(crate) fn avg_response_duration_ms(&self) -> Option<f64> {
        (self.response_duration_sample_count > 0)
            .then_some(self.response_duration_sum_ms / self.response_duration_sample_count as f64)
    }

    pub(crate) fn first_token_avg_ms(&self) -> Option<f64> {
        (self.first_token_sample_count > 0)
            .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64)
    }

    pub(crate) fn into_current_snapshot(self) -> DashboardActivityCurrentSnapshot {
        DashboardActivityCurrentSnapshot {
            qualified_tokens: self.qualified_tokens,
            total_cost: self.total_cost,
            first_response_byte_total_sample_count: self.first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms: self.first_response_byte_total_sum_ms,
            first_token_sample_count: self.first_token_sample_count,
            first_token_sum_ms: self.first_token_sum_ms,
            total_latency_sample_count: self.total_latency_sample_count,
            total_latency_sum_ms: self.total_latency_sum_ms,
            response_duration_sample_count: self.response_duration_sample_count,
            response_duration_sum_ms: self.response_duration_sum_ms,
        }
    }
}

pub(crate) fn normalize_trimmed_optional_string_local(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn resolve_upstream_account_activity_display_name(
    account_id: i64,
    meta: Option<&UpstreamAccountActivityMetaRow>,
    hint: Option<&str>,
) -> String {
    if let Some(display_name) = meta
        .and_then(|row| row.display_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return display_name.to_string();
    }
    if let Some(display_name) = hint.map(str::trim).filter(|value| !value.is_empty()) {
        return display_name.to_string();
    }
    format!("账号 #{account_id}")
}

#[derive(Debug, Clone)]
pub(crate) struct UpstreamAccountActivityStatusFields {
    pub(crate) enabled: bool,
    pub(crate) display_status: String,
    pub(crate) enable_status: String,
    pub(crate) work_status: String,
    pub(crate) health_status: String,
    pub(crate) sync_state: String,
    pub(crate) last_error: Option<String>,
    pub(crate) last_action_reason_message: Option<String>,
}

pub(crate) fn build_upstream_account_activity_status_fields(
    meta: &UpstreamAccountActivityMetaRow,
    now: DateTime<Utc>,
) -> UpstreamAccountActivityStatusFields {
    let enabled = meta.enabled != 0;
    let enable_status = crate::upstream_accounts::derive_upstream_account_enable_status(enabled);
    let health_status = crate::upstream_accounts::derive_upstream_account_health_status(
        &meta.kind,
        enabled,
        &meta.status,
        meta.last_error.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
    );
    let sync_state =
        crate::upstream_accounts::derive_upstream_account_sync_state(enabled, &meta.status);
    let work_status = crate::upstream_accounts::derive_upstream_account_work_status(
        enabled,
        &meta.status,
        health_status,
        sync_state,
        false,
        meta.cooldown_until.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
        meta.temporary_route_failure_streak_started_at.as_deref(),
        meta.last_selected_at.as_deref(),
        now,
    );
    let display_status = crate::upstream_accounts::classify_upstream_account_display_status(
        &meta.kind,
        enabled,
        &meta.status,
        meta.last_error.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
    );

    UpstreamAccountActivityStatusFields {
        enabled,
        display_status: display_status.to_string(),
        enable_status: enable_status.to_string(),
        work_status: work_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        last_error: normalize_trimmed_optional_string_local(meta.last_error.clone()),
        last_action_reason_message: normalize_trimmed_optional_string_local(
            meta.last_action_reason_message.clone(),
        ),
    }
}

pub(crate) fn merge_latest_optional_timestamp(
    current: &mut Option<String>,
    candidate: Option<String>,
) {
    let Some(candidate) = candidate else {
        return;
    };
    *current = Some(match current.take() {
        Some(existing) => existing.max(candidate),
        None => candidate,
    });
}

pub(crate) fn merge_latest_timed_metric(
    current_at: &mut Option<String>,
    current_value: &mut Option<f64>,
    candidate_at: Option<String>,
    candidate_value: Option<f64>,
) {
    let mut latest = LatestTimedMetricValue {
        at: current_at.take(),
        value: current_value.take(),
    };
    latest.update(candidate_at, candidate_value);
    *current_at = latest.at;
    *current_value = latest.value;
}

pub(crate) fn invocation_upstream_account_id_with_attempt_fallback_sql(
    invocation_ref: &str,
) -> String {
    let payload_account_id_sql =
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.replace("payload", &format!("{invocation_ref}.payload"));
    format!(
        "COALESCE({payload_account_id_sql}, \
           {attempt_fallback}\
         )",
        attempt_fallback = invocation_upstream_account_id_from_attempt_fallback_sql(invocation_ref)
    )
}

pub(crate) fn invocation_upstream_account_id_from_attempt_fallback_sql(
    invocation_ref: &str,
) -> String {
    let normalized_epoch_ms = |reference: &str| {
        format!(
            "CAST(ROUND((julianday({reference}.occurred_at, CASE WHEN instr({reference}.occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END) - 2440587.5) * 86400000.0) AS INTEGER)"
        )
    };
    let attempt_epoch_ms = normalized_epoch_ms("attempt");
    let invocation_epoch_ms = normalized_epoch_ms(invocation_ref);
    format!(
        "(SELECT attempt.upstream_account_id \
           FROM pool_upstream_request_attempts attempt \
          WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
            AND {attempt_epoch_ms} = {invocation_epoch_ms} \
            AND attempt.upstream_account_id IS NOT NULL \
          ORDER BY attempt.attempt_index DESC, attempt.id DESC \
          LIMIT 1)"
    )
}

pub(crate) fn invocation_request_compression_algorithm_with_attempt_fallback_sql(
    invocation_ref: &str,
) -> String {
    format!(
        "CASE WHEN EXISTS(\
           SELECT 1 \
             FROM pool_upstream_request_attempts attempt \
            WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
              AND attempt.occurred_at = {invocation_ref}.occurred_at \
              AND LOWER(TRIM(COALESCE(attempt.status, ''))) <> 'budget_exhausted_final'\
         ) THEN (\
           SELECT NULLIF(TRIM(attempt.upstream_request_compression_algorithm), '') \
              FROM pool_upstream_request_attempts attempt \
             WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
               AND attempt.occurred_at = {invocation_ref}.occurred_at \
               AND LOWER(TRIM(COALESCE(attempt.status, ''))) <> 'budget_exhausted_final' \
             ORDER BY attempt.attempt_index DESC, attempt.id DESC \
             LIMIT 1\
         ) ELSE NULLIF(TRIM(CASE WHEN json_valid({invocation_ref}.payload) \
             THEN CAST(json_extract({invocation_ref}.payload, '$.requestCompressionAlgorithm') AS TEXT) \
           END), '') END"
    )
}

pub(crate) fn invocation_prompt_cache_key_sql(invocation_ref: &str) -> String {
    format!(
        "CASE WHEN json_valid({invocation_ref}.payload) \
           THEN TRIM(CAST(json_extract({invocation_ref}.payload, '$.promptCacheKey') AS TEXT)) \
         END"
    )
}

pub(crate) fn invocation_history_conversation_created_at_sql(
    prompt_cache_key_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let history_prompt_cache_key_sql = invocation_prompt_cache_key_sql("conversation_history");
    let source_filter = match source_scope {
        InvocationSourceScope::All => String::new(),
        InvocationSourceScope::ProxyOnly => {
            format!(" AND conversation_history.source = '{SOURCE_PROXY}'")
        }
    };
    format!(
        "(SELECT MIN(conversation_history.occurred_at) \
            FROM codex_invocations conversation_history \
           WHERE ({history_prompt_cache_key_sql}) = ({prompt_cache_key_sql}){source_filter})"
    )
}

pub(crate) fn prompt_cache_conversation_created_at_sql(
    prompt_cache_key_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let rollup_source_filter = match source_scope {
        InvocationSourceScope::All => String::new(),
        InvocationSourceScope::ProxyOnly => format!(" AND source = '{SOURCE_PROXY}'"),
    };
    let working_set_created_at_sql = match source_scope {
        InvocationSourceScope::All => "created_at",
        InvocationSourceScope::ProxyOnly => "COALESCE(proxy_created_at, created_at)",
    };
    let invocation_history_created_at_sql =
        invocation_history_conversation_created_at_sql(prompt_cache_key_sql, source_scope);
    format!(
        "COALESCE(\
            (SELECT MIN(first_seen_at) \
               FROM prompt_cache_rollup_hourly \
              WHERE prompt_cache_key = {prompt_cache_key_sql}{rollup_source_filter}), \
            (SELECT {working_set_created_at_sql} \
               FROM prompt_cache_working_set_live \
              WHERE prompt_cache_key = {prompt_cache_key_sql}), \
            {invocation_history_created_at_sql}\
         )"
    )
}

pub(crate) fn invocation_account_retry_after_failure_with_attempt_fallback_sql(
    current_upstream_account_id_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let previous_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let account_match_sql = format!(
        "({previous_upstream_account_id_sql} = {current_upstream_account_id_sql} \
         OR ({previous_upstream_account_id_sql} IS NULL AND {current_upstream_account_id_sql} IS NULL))"
    );
    let display_status_sql = invocation_display_status_sql();
    let source_filter = match source_scope {
        InvocationSourceScope::All => "",
        InvocationSourceScope::ProxyOnly => "AND source = 'proxy'",
    };
    format!(
        "COALESCE((
            SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
            FROM (
                SELECT LOWER(TRIM({display_status_sql})) AS display_status
                FROM codex_invocations
                WHERE {prompt_cache_key_sql} = live.prompt_cache_key
                  AND {account_match_sql}
                  AND id < live.invocation_id
                  {source_filter}
                  AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                ORDER BY id DESC
                LIMIT 1
            ) AS previous_terminal
        ), 0)",
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL,
        account_match_sql = account_match_sql,
    )
}

pub(crate) fn compute_upstream_account_activity_rates(
    rate_usage_events: &[UpstreamAccountRateUsageEvent],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> (Option<f64>, Option<f64>) {
    (
        compute_upstream_account_activity_tail_rate(
            rate_usage_events,
            range_start,
            range_end,
            |usage| usage.total_tokens.max(0) as f64,
        ),
        compute_upstream_account_activity_tail_rate(
            rate_usage_events,
            range_start,
            range_end,
            |usage| usage.total_cost.max(0.0),
        ),
    )
}

pub(crate) fn compute_upstream_account_activity_tail_rate(
    rate_usage_events: &[UpstreamAccountRateUsageEvent],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    value_of: impl Fn(&UpstreamAccountRateUsageEvent) -> f64,
) -> Option<f64> {
    const RATE_WINDOW_MILLIS: i64 = 5 * 60 * 1_000;
    const MINUTE_MILLIS: i64 = 60 * 1_000;

    let anchor_epoch_ms = range_end.timestamp_millis();
    let range_start_epoch_ms = range_start.timestamp_millis();
    if anchor_epoch_ms <= range_start_epoch_ms {
        return Some(0.0);
    }
    let window_start_epoch_ms = range_start_epoch_ms.max(anchor_epoch_ms - RATE_WINDOW_MILLIS);

    let first_active_epoch_ms = rate_usage_events
        .iter()
        .filter_map(|usage| {
            if usage.occurred_at_epoch_ms < window_start_epoch_ms
                || usage.occurred_at_epoch_ms >= anchor_epoch_ms
            {
                return None;
            }
            let value = value_of(usage);
            (value.is_finite() && value > 0.0).then_some(usage.occurred_at_epoch_ms)
        })
        .min();

    let Some(first_active_epoch_ms) = first_active_epoch_ms else {
        return Some(0.0);
    };
    let active_bucket_start_epoch_ms =
        first_active_epoch_ms.div_euclid(MINUTE_MILLIS) * MINUTE_MILLIS;
    let active_start_epoch_ms = window_start_epoch_ms.max(active_bucket_start_epoch_ms);
    let active_millis = (anchor_epoch_ms - active_start_epoch_ms).max(0);
    if active_millis == 0 {
        return Some(0.0);
    }
    let total_value = rate_usage_events
        .iter()
        .filter_map(|usage| {
            if usage.occurred_at_epoch_ms < active_start_epoch_ms
                || usage.occurred_at_epoch_ms >= anchor_epoch_ms
            {
                return None;
            }
            let value = value_of(usage);
            (value.is_finite() && value > 0.0).then_some(value)
        })
        .sum::<f64>();

    Some(total_value / (active_millis as f64 / MINUTE_MILLIS as f64))
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountActivityAggregateRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) latest_conversation_created_at: Option<String>,
    pub(crate) last_invocation_at: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) non_success_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) success_tokens: i64,
    pub(crate) non_success_tokens: i64,
    pub(crate) failure_tokens: i64,
    pub(crate) failure_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) latest_first_response_byte_total_at: Option<String>,
    pub(crate) latest_first_response_byte_total_ms: Option<f64>,
    pub(crate) latest_avg_total_at: Option<String>,
    pub(crate) latest_avg_total_ms: Option<f64>,
}

pub(crate) fn merge_upstream_account_activity_aggregate_row(
    entry: &mut UpstreamAccountActivityAccumulator,
    row: &UpstreamAccountActivityAggregateRow,
) {
    merge_latest_optional_timestamp(
        &mut entry.latest_conversation_created_at,
        row.latest_conversation_created_at.clone(),
    );
    merge_latest_optional_timestamp(
        &mut entry.last_invocation_at,
        row.last_invocation_at.clone(),
    );
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
    entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
    entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
    entry.first_token_sample_count += row.first_token_sample_count;
    entry.first_token_sum_ms += row.first_token_sum_ms;
    entry.total_latency_sample_count += row.total_latency_sample_count;
    entry.total_latency_sum_ms += row.total_latency_sum_ms;
    merge_latest_timed_metric(
        &mut entry.latest_first_response_byte_total_at,
        &mut entry.latest_first_response_byte_total_ms,
        row.latest_first_response_byte_total_at.clone(),
        row.latest_first_response_byte_total_ms,
    );
    merge_latest_timed_metric(
        &mut entry.latest_avg_total_at,
        &mut entry.latest_avg_total_ms,
        row.latest_avg_total_at.clone(),
        row.latest_avg_total_ms,
    );
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountPromptCacheCreatedAtRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) prompt_cache_key: String,
    pub(crate) first_occurred_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountUsageBreakdownAggregateRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cost_input: f64,
    pub(crate) cost_cache_write: f64,
    pub(crate) cost_cache_read: f64,
    pub(crate) cost_output: f64,
    pub(crate) cost_reasoning: f64,
    pub(crate) cost_unknown: f64,
    pub(crate) has_cost: i64,
    pub(crate) performance_total_tokens: i64,
    pub(crate) performance_stream_output_tokens: i64,
    pub(crate) performance_stream_duration_ms: f64,
    pub(crate) performance_response_sample_count: i64,
    pub(crate) performance_response_sum_ms: f64,
    pub(crate) performance_first_byte_sample_count: i64,
    pub(crate) performance_first_byte_sum_ms: f64,
    pub(crate) performance_first_token_sample_count: i64,
    pub(crate) performance_first_token_sum_ms: f64,
    pub(crate) performance_usage_duration_sample_count: i64,
    pub(crate) performance_usage_duration_sum_ms: f64,
}

pub(crate) type UsageBreakdownAggregateMergeKey = (Option<i64>, String, Option<String>);

pub(crate) fn merge_usage_breakdown_aggregate_row_map(
    rows_by_key: &mut HashMap<
        UsageBreakdownAggregateMergeKey,
        UpstreamAccountUsageBreakdownAggregateRow,
    >,
    row: &UpstreamAccountUsageBreakdownAggregateRow,
) {
    let key = (
        row.upstream_account_id,
        row.model.clone(),
        row.reasoning_effort.clone(),
    );
    let entry =
        rows_by_key
            .entry(key)
            .or_insert_with(|| UpstreamAccountUsageBreakdownAggregateRow {
                upstream_account_id: row.upstream_account_id,
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
                request_count: 0,
                success_count: 0,
                failure_count: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                output_tokens: 0,
                cost_input: 0.0,
                cost_cache_write: 0.0,
                cost_cache_read: 0.0,
                cost_output: 0.0,
                cost_reasoning: 0.0,
                cost_unknown: 0.0,
                has_cost: 0,
                performance_total_tokens: 0,
                performance_stream_output_tokens: 0,
                performance_stream_duration_ms: 0.0,
                performance_response_sample_count: 0,
                performance_response_sum_ms: 0.0,
                performance_first_byte_sample_count: 0,
                performance_first_byte_sum_ms: 0.0,
                performance_first_token_sample_count: 0,
                performance_first_token_sum_ms: 0.0,
                performance_usage_duration_sample_count: 0,
                performance_usage_duration_sum_ms: 0.0,
            });
    entry.request_count += row.request_count;
    entry.success_count += row.success_count;
    entry.failure_count += row.failure_count;
    entry.cache_write_tokens += row.cache_write_tokens;
    entry.cache_read_tokens += row.cache_read_tokens;
    entry.output_tokens += row.output_tokens;
    entry.cost_input += row.cost_input;
    entry.cost_cache_write += row.cost_cache_write;
    entry.cost_cache_read += row.cost_cache_read;
    entry.cost_output += row.cost_output;
    entry.cost_reasoning += row.cost_reasoning;
    entry.cost_unknown += row.cost_unknown;
    entry.has_cost += row.has_cost;
    entry.performance_total_tokens += row.performance_total_tokens;
    entry.performance_stream_output_tokens += row.performance_stream_output_tokens;
    entry.performance_stream_duration_ms += row.performance_stream_duration_ms;
    entry.performance_response_sample_count += row.performance_response_sample_count;
    entry.performance_response_sum_ms += row.performance_response_sum_ms;
    entry.performance_first_byte_sample_count += row.performance_first_byte_sample_count;
    entry.performance_first_byte_sum_ms += row.performance_first_byte_sum_ms;
    entry.performance_first_token_sample_count += row.performance_first_token_sample_count;
    entry.performance_first_token_sum_ms += row.performance_first_token_sum_ms;
    entry.performance_usage_duration_sample_count += row.performance_usage_duration_sample_count;
    entry.performance_usage_duration_sum_ms += row.performance_usage_duration_sum_ms;
}

pub(crate) fn merge_usage_breakdown_rollup_record_map(
    rows_by_key: &mut HashMap<
        UsageBreakdownAggregateMergeKey,
        UpstreamAccountUsageBreakdownAggregateRow,
    >,
    row: &UpstreamAccountUsageBreakdownHourlyRollupRecord,
) {
    merge_usage_breakdown_aggregate_row_map(
        rows_by_key,
        &UpstreamAccountUsageBreakdownAggregateRow {
            upstream_account_id: row.upstream_account_id,
            model: row.model.clone(),
            reasoning_effort: row.reasoning_effort.clone(),
            request_count: row.request_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            cache_write_tokens: row.cache_write_tokens,
            cache_read_tokens: row.cache_read_tokens,
            output_tokens: row.output_tokens,
            cost_input: row.cost_input,
            cost_cache_write: row.cost_cache_write,
            cost_cache_read: row.cost_cache_read,
            cost_output: row.cost_output,
            cost_reasoning: row.cost_reasoning,
            cost_unknown: row.cost_unknown,
            has_cost: row.has_cost,
            performance_total_tokens: row.performance_total_tokens,
            performance_stream_output_tokens: row.performance_stream_output_tokens,
            performance_stream_duration_ms: row.performance_stream_duration_ms,
            performance_response_sample_count: row.performance_response_sample_count,
            performance_response_sum_ms: row.performance_response_sum_ms,
            performance_first_byte_sample_count: row.performance_first_byte_sample_count,
            performance_first_byte_sum_ms: row.performance_first_byte_sum_ms,
            performance_first_token_sample_count: row.performance_first_token_sample_count,
            performance_first_token_sum_ms: row.performance_first_token_sum_ms,
            performance_usage_duration_sample_count: row.performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms: row.performance_usage_duration_sum_ms,
        },
    );
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct SuccessfulBilledUsageDurationIntervalRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) start_epoch_ms: f64,
    pub(crate) end_epoch_ms: f64,
}

#[derive(Debug, Default)]
pub(crate) struct ModelPerformanceDurationOverrides {
    pub(crate) total_wall_clock_ms: Option<f64>,
    pub(crate) by_account_wall_clock_ms: HashMap<Option<i64>, f64>,
    pub(crate) by_group_wall_clock_ms: HashMap<UsageBreakdownGroupKey, f64>,
    pub(crate) by_model_wall_clock_ms: HashMap<String, f64>,
    pub(crate) by_account_group_wall_clock_ms: HashMap<AccountModelGroupKey, f64>,
    pub(crate) by_account_model_wall_clock_ms: HashMap<AccountModelKey, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AccountModelGroupKey {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) group: UsageBreakdownGroupKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AccountModelKey {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) model: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct UsageDurationUnionAccumulator {
    pub(crate) saw_interval: bool,
    pub(crate) total_ms: f64,
    pub(crate) current_end_epoch_ms: Option<f64>,
}

impl UsageDurationUnionAccumulator {
    pub(crate) fn push_interval(&mut self, start_epoch_ms: f64, end_epoch_ms: f64) {
        if !start_epoch_ms.is_finite() || !end_epoch_ms.is_finite() || end_epoch_ms < start_epoch_ms
        {
            return;
        }
        self.saw_interval = true;
        match self.current_end_epoch_ms {
            None => {
                self.total_ms += end_epoch_ms - start_epoch_ms;
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
            Some(current_end_epoch_ms) if end_epoch_ms <= current_end_epoch_ms => {}
            Some(current_end_epoch_ms) => {
                self.total_ms += end_epoch_ms - start_epoch_ms.max(current_end_epoch_ms);
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
        }
    }

    pub(crate) fn total_ms(self) -> Option<f64> {
        self.saw_interval.then_some(self.total_ms)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ModelPerformanceWallClockUnionState {
    pub(crate) total: UsageDurationUnionAccumulator,
    pub(crate) by_account: HashMap<Option<i64>, UsageDurationUnionAccumulator>,
    pub(crate) by_group: HashMap<UsageBreakdownGroupKey, UsageDurationUnionAccumulator>,
    pub(crate) by_model: HashMap<String, UsageDurationUnionAccumulator>,
    pub(crate) by_account_group: HashMap<AccountModelGroupKey, UsageDurationUnionAccumulator>,
    pub(crate) by_account_model: HashMap<AccountModelKey, UsageDurationUnionAccumulator>,
}

impl ModelPerformanceWallClockUnionState {
    pub(crate) fn push_row(&mut self, row: &SuccessfulBilledUsageDurationIntervalRow) {
        let group = UsageBreakdownGroupKey {
            model: row.model.clone(),
            reasoning_effort: row.reasoning_effort.clone(),
        };
        self.total
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_group
            .entry(group.clone())
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_model
            .entry(row.model.clone())
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account_group
            .entry(AccountModelGroupKey {
                upstream_account_id: row.upstream_account_id,
                group,
            })
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account_model
            .entry(AccountModelKey {
                upstream_account_id: row.upstream_account_id,
                model: row.model.clone(),
            })
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
    }

    pub(crate) fn into_overrides(self) -> ModelPerformanceDurationOverrides {
        ModelPerformanceDurationOverrides {
            total_wall_clock_ms: self.total.total_ms(),
            by_account_wall_clock_ms: self
                .by_account
                .into_iter()
                .filter_map(|(upstream_account_id, accumulator)| {
                    accumulator
                        .total_ms()
                        .map(|total_ms| (upstream_account_id, total_ms))
                })
                .collect(),
            by_group_wall_clock_ms: self
                .by_group
                .into_iter()
                .filter_map(|(group, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (group, total_ms))
                })
                .collect(),
            by_model_wall_clock_ms: self
                .by_model
                .into_iter()
                .filter_map(|(model, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (model, total_ms))
                })
                .collect(),
            by_account_group_wall_clock_ms: self
                .by_account_group
                .into_iter()
                .filter_map(|(key, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (key, total_ms))
                })
                .collect(),
            by_account_model_wall_clock_ms: self
                .by_account_model
                .into_iter()
                .filter_map(|(key, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (key, total_ms))
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ModelPerformanceAccumulator {
    pub(crate) total_tokens: i64,
    pub(crate) stream_output_tokens: i64,
    pub(crate) stream_duration_ms: f64,
    pub(crate) response_sample_count: i64,
    pub(crate) response_sum_ms: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) cumulative_usage_duration_sample_count: i64,
    pub(crate) cumulative_usage_duration_sum_ms: f64,
    pub(crate) wall_clock_usage_duration_ms: Option<f64>,
    pub(crate) models: HashMap<UsageBreakdownGroupKey, ModelPerformanceAccumulator>,
    pub(crate) model_groups: HashMap<String, ModelPerformanceAccumulator>,
}

impl ModelPerformanceAccumulator {
    pub(crate) fn add_aggregate_values(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.total_tokens += row.performance_total_tokens.max(0);
        self.stream_output_tokens += row.performance_stream_output_tokens.max(0);
        self.stream_duration_ms += row.performance_stream_duration_ms.max(0.0);
        self.response_sample_count += row.performance_response_sample_count.max(0);
        self.response_sum_ms += row.performance_response_sum_ms.max(0.0);
        self.first_byte_sample_count += row.performance_first_byte_sample_count.max(0);
        self.first_byte_sum_ms += row.performance_first_byte_sum_ms.max(0.0);
        self.first_token_sample_count += row.performance_first_token_sample_count.max(0);
        self.first_token_sum_ms += row.performance_first_token_sum_ms.max(0.0);
        self.cumulative_usage_duration_sample_count +=
            row.performance_usage_duration_sample_count.max(0);
        self.cumulative_usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);
    }

    pub(crate) fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.add_aggregate_values(row);

        let entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
            })
            .or_default();
        entry.add_aggregate_values(row);
        self.model_groups
            .entry(row.model.clone())
            .or_default()
            .add_aggregate_values(row);
    }

    pub(crate) fn add_terminal_record(&mut self, record: &ApiInvocation) {
        self.add_terminal_delta(&dashboard_activity_terminal_delta(record));
    }

    pub(crate) fn add_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        let group = UsageBreakdownGroupKey {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
        };
        self.add_terminal_delta_values(delta);
        self.models
            .entry(group)
            .or_default()
            .add_terminal_delta_values(delta);
        self.model_groups
            .entry(delta.model.clone())
            .or_default()
            .add_terminal_delta_values(delta);
    }

    pub(crate) fn add_terminal_delta_values(&mut self, delta: &DashboardActivityTerminalDelta) {
        let success_like = delta.success;
        let success_billed = success_like && delta.has_cost;
        if success_billed {
            self.total_tokens += delta.total_tokens;
        }
        if let Some(stream_duration_ms) = success_like
            .then_some(delta.t_upstream_stream_ms)
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            if success_billed {
                self.stream_output_tokens += delta.output_tokens;
                self.stream_duration_ms += stream_duration_ms;
            }
            self.response_sample_count += 1;
            self.response_sum_ms += stream_duration_ms;
        }
        if success_billed
            && delta
                .t_upstream_ttfb_ms
                .is_some_and(|value| value.is_finite() && value > 0.0)
            && let Some(first_response_byte_total_ms) =
                crate::stats::resolve_first_response_byte_total_ms(
                    delta.t_req_read_ms,
                    delta.t_req_parse_ms,
                    delta.t_upstream_connect_ms,
                    delta.t_upstream_ttfb_ms,
                )
        {
            self.first_byte_sample_count += 1;
            self.first_byte_sum_ms += first_response_byte_total_ms;
        }
        if let Some(first_token_ms) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count += 1;
            self.first_token_sum_ms += first_token_ms;
        }
        if success_billed
            && let Some(total_ms) = delta
                .t_total_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.cumulative_usage_duration_sample_count += 1;
            self.cumulative_usage_duration_sum_ms += total_ms;
            // The persisted baseline stores only the prior interval union, so a single new
            // interval cannot be merged exactly until reconciliation rebuilds that union.
            self.wall_clock_usage_duration_ms = None;
        }
    }

    pub(crate) fn subtract_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        let group = UsageBreakdownGroupKey {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
        };
        self.subtract_terminal_delta_values(delta);
        if let Some(model) = self.models.get_mut(&group) {
            model.subtract_terminal_delta_values(delta);
            if model.is_empty() {
                self.models.remove(&group);
            }
        }
        if let Some(model) = self.model_groups.get_mut(&delta.model) {
            model.subtract_terminal_delta_values(delta);
            if model.is_empty() {
                self.model_groups.remove(&delta.model);
            }
        }
    }

    pub(crate) fn subtract_terminal_delta_values(
        &mut self,
        delta: &DashboardActivityTerminalDelta,
    ) {
        let success_billed = delta.success && delta.has_cost;
        if success_billed {
            self.total_tokens = self.total_tokens.saturating_sub(delta.total_tokens);
        }
        if let Some(stream_duration_ms) = delta
            .success
            .then_some(delta.t_upstream_stream_ms)
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            if success_billed {
                self.stream_output_tokens = self
                    .stream_output_tokens
                    .saturating_sub(delta.output_tokens);
                self.stream_duration_ms = (self.stream_duration_ms - stream_duration_ms).max(0.0);
            }
            self.response_sample_count = self.response_sample_count.saturating_sub(1);
            self.response_sum_ms = (self.response_sum_ms - stream_duration_ms).max(0.0);
        }
        if success_billed
            && delta
                .t_upstream_ttfb_ms
                .is_some_and(|value| value.is_finite() && value > 0.0)
            && let Some(value) = crate::stats::resolve_first_response_byte_total_ms(
                delta.t_req_read_ms,
                delta.t_req_parse_ms,
                delta.t_upstream_connect_ms,
                delta.t_upstream_ttfb_ms,
            )
        {
            self.first_byte_sample_count = self.first_byte_sample_count.saturating_sub(1);
            self.first_byte_sum_ms = (self.first_byte_sum_ms - value).max(0.0);
        }
        if let Some(value) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count = self.first_token_sample_count.saturating_sub(1);
            self.first_token_sum_ms = (self.first_token_sum_ms - value).max(0.0);
        }
        if success_billed
            && let Some(value) = delta
                .t_total_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.cumulative_usage_duration_sample_count = self
                .cumulative_usage_duration_sample_count
                .saturating_sub(1);
            self.cumulative_usage_duration_sum_ms =
                (self.cumulative_usage_duration_sum_ms - value).max(0.0);
            self.wall_clock_usage_duration_ms = None;
        }
    }

    pub(crate) fn metrics(&self, range: ExactUtcRange) -> ModelPerformanceMetricsResponse {
        let range_minutes = (range.end - range.start).num_milliseconds() as f64 / 60_000.0;
        let cumulative_usage_duration_ms = (self.cumulative_usage_duration_sample_count > 0)
            .then_some(self.cumulative_usage_duration_sum_ms);
        ModelPerformanceMetricsResponse {
            tokens_per_minute: if range_minutes > 0.0 {
                self.total_tokens as f64 / range_minutes
            } else {
                0.0
            },
            streaming_response_rate: (self.stream_duration_ms > 0.0)
                .then_some(self.stream_output_tokens as f64 / (self.stream_duration_ms / 1_000.0)),
            avg_response_ms: (self.response_sample_count > 0)
                .then_some(self.response_sum_ms / self.response_sample_count as f64),
            avg_first_response_byte_total_ms: (self.first_byte_sample_count > 0)
                .then_some(self.first_byte_sum_ms / self.first_byte_sample_count as f64),
            avg_first_token_ms: (self.first_token_sample_count > 0)
                .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64),
            wall_clock_usage_duration_ms: self.wall_clock_usage_duration_ms,
            cumulative_usage_duration_ms,
            parallelism: match (
                self.wall_clock_usage_duration_ms,
                cumulative_usage_duration_ms,
            ) {
                (Some(wall_clock_ms), Some(cumulative_ms)) if wall_clock_ms > 0.0 => {
                    Some(cumulative_ms / wall_clock_ms)
                }
                _ => None,
            },
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.total_tokens == 0
            && self.stream_duration_ms == 0.0
            && self.response_sample_count == 0
            && self.first_byte_sample_count == 0
            && self.first_token_sample_count == 0
            && self.cumulative_usage_duration_sample_count == 0
            && self.wall_clock_usage_duration_ms.is_none()
    }

    pub(crate) fn into_model_response(
        model: String,
        reasoning_effort: Option<String>,
        entry: ModelPerformanceAccumulator,
        range: ExactUtcRange,
    ) -> Option<ModelPerformanceModelResponse> {
        (!entry.is_empty()).then_some(ModelPerformanceModelResponse {
            model,
            reasoning_effort,
            metrics: entry.metrics(range),
        })
    }

    pub(crate) fn into_response(
        self,
        range: ExactUtcRange,
        available: bool,
    ) -> ModelPerformanceResponse {
        let total = self.metrics(range);
        let mut models = self
            .models
            .into_iter()
            .filter_map(|(group, entry)| {
                Self::into_model_response(group.model, group.reasoning_effort, entry, range)
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            right
                .metrics
                .cumulative_usage_duration_ms
                .unwrap_or_default()
                .total_cmp(
                    &left
                        .metrics
                        .cumulative_usage_duration_ms
                        .unwrap_or_default(),
                )
                .then_with(|| left.model.cmp(&right.model))
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        let mut model_groups = self
            .model_groups
            .into_iter()
            .filter_map(|(model, entry)| Self::into_model_response(model, None, entry, range))
            .collect::<Vec<_>>();
        model_groups.sort_by(|left, right| {
            right
                .metrics
                .cumulative_usage_duration_ms
                .unwrap_or_default()
                .total_cmp(
                    &left
                        .metrics
                        .cumulative_usage_duration_ms
                        .unwrap_or_default(),
                )
                .then_with(|| left.model.cmp(&right.model))
        });
        ModelPerformanceResponse {
            available,
            total,
            models,
            model_groups,
        }
    }

    pub(crate) fn unavailable(range: ExactUtcRange) -> ModelPerformanceResponse {
        Self::default().into_response(range, false)
    }
}

pub(crate) fn compute_model_performance_duration_overrides(
    rows: &[SuccessfulBilledUsageDurationIntervalRow],
) -> ModelPerformanceDurationOverrides {
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    for row in rows {
        union_state.push_row(row);
    }
    union_state.into_overrides()
}
