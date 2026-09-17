#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountActivityMetaRow {
    id: i64,
    kind: String,
    display_name: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
    status: String,
    enabled: i64,
    last_error: Option<String>,
    last_error_at: Option<String>,
    last_route_failure_at: Option<String>,
    last_route_failure_kind: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_reason_message: Option<String>,
    cooldown_until: Option<String>,
    temporary_route_failure_streak_started_at: Option<String>,
    last_selected_at: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct UpstreamAccountInProgressSummary {
    in_progress_count: i64,
    retry_count: i64,
    phase_counts: InvocationPhaseCountsResponse,
    wait_sum_ms: f64,
    wait_sample_count: i64,
}

impl UpstreamAccountInProgressSummary {
    fn add(&mut self, retry: bool, phase: Option<&str>, wait_ms: Option<f64>) {
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

    fn subtract(&mut self, retry: bool, phase: Option<&str>, wait_ms: Option<f64>) {
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

fn runtime_record_ttfb_differs(left: Option<f64>, right: Option<f64>) -> bool {
    normalized_wait_ms(left) != normalized_wait_ms(right)
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountActivityAccumulator {
    display_name_hint: Option<String>,
    plan_type_hint: Option<String>,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    non_success_count: i64,
    total_tokens: i64,
    success_tokens: i64,
    non_success_tokens: i64,
    failure_tokens: i64,
    failure_cost: f64,
    non_success_cost: f64,
    cache_input_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
    latest_first_response_byte_total_at: Option<String>,
    latest_first_response_byte_total_ms: Option<f64>,
    latest_avg_total_at: Option<String>,
    latest_avg_total_ms: Option<f64>,
    in_progress_wait_sample_count: i64,
    in_progress_wait_sum_ms: f64,
    last_occurred_at_epoch_ms: i64,
    latest_conversation_created_at: Option<String>,
    last_invocation_at: Option<String>,
    rate_usage_events: Vec<UpstreamAccountRateUsageEvent>,
    recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    usage_breakdown: UsageBreakdownAccumulator,
    model_performance: ModelPerformanceAccumulator,
}

#[derive(Debug, Clone, Default)]
struct LatestTimedMetricValue {
    at: Option<String>,
    value: Option<f64>,
}

impl LatestTimedMetricValue {
    fn update(&mut self, candidate_at: Option<String>, candidate_value: Option<f64>) {
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
    model: String,
    reasoning_effort: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct UsageBreakdownAccumulator {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    costs: UsageCostBreakdownResponse,
    has_cost: bool,
    models: HashMap<UsageBreakdownGroupKey, UsageBreakdownAccumulator>,
}

impl UsageBreakdownAccumulator {
    fn add_cost_row(&mut self, total_cost: Option<f64>, costs: [Option<f64>; 5]) {
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

    fn merge_costs(&mut self, costs: &UsageCostBreakdownResponse) {
        self.has_cost = true;
        self.costs.input += costs.input;
        self.costs.cache_write += costs.cache_write;
        self.costs.cache_read += costs.cache_read;
        self.costs.output += costs.output;
        self.costs.reasoning += costs.reasoning;
        self.costs.unknown += costs.unknown;
    }

    fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
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

    fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
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

    fn add_coarse_rollup_totals(
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

    fn merge_response(&mut self, response: &UsageBreakdownResponse) {
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

    fn into_response(self) -> UsageBreakdownResponse {
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
    occurred_at_epoch_ms: i64,
    total_tokens: i64,
    total_cost: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardActivityCurrentMinuteAccumulator {
    qualified_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
    response_duration_sample_count: i64,
    response_duration_sum_ms: f64,
}

impl DashboardActivityCurrentMinuteAccumulator {
    fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
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

    fn merge(&mut self, other: Self) {
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

    fn first_response_byte_total_avg_ms(&self) -> Option<f64> {
        (self.first_response_byte_total_sample_count > 0).then_some(
            self.first_response_byte_total_sum_ms
                / self.first_response_byte_total_sample_count as f64,
        )
    }

    fn avg_total_ms(&self) -> Option<f64> {
        (self.total_latency_sample_count > 0)
            .then_some(self.total_latency_sum_ms / self.total_latency_sample_count as f64)
    }

    fn avg_response_duration_ms(&self) -> Option<f64> {
        (self.response_duration_sample_count > 0)
            .then_some(self.response_duration_sum_ms / self.response_duration_sample_count as f64)
    }

    fn first_token_avg_ms(&self) -> Option<f64> {
        (self.first_token_sample_count > 0)
            .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64)
    }

    fn into_current_snapshot(self) -> DashboardActivityCurrentSnapshot {
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
    enabled: bool,
    display_status: String,
    enable_status: String,
    work_status: String,
    health_status: String,
    sync_state: String,
    last_error: Option<String>,
    last_action_reason_message: Option<String>,
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

fn merge_latest_optional_timestamp(current: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    *current = Some(match current.take() {
        Some(existing) => existing.max(candidate),
        None => candidate,
    });
}

fn merge_latest_timed_metric(
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
    format!(
        "COALESCE(\
           CASE WHEN json_valid({invocation_ref}.payload) \
             THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) \
           END, \
           (SELECT attempt.upstream_account_id \
              FROM pool_upstream_request_attempts attempt \
             WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
               AND attempt.upstream_account_id IS NOT NULL \
             ORDER BY attempt.attempt_index DESC, attempt.id DESC \
             LIMIT 1)\
         )"
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
