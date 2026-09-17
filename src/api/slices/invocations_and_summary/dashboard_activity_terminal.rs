fn dashboard_activity_terminal_delta(record: &ApiInvocation) -> DashboardActivityTerminalDelta {
    dashboard_activity_terminal_delta_with_first_token(
        record,
        runtime_record_first_token_ms(record),
    )
}

// Persisted dashboard rows come from build_invocation_select_query(), whose timing expression
// already establishes final-attempt ownership. Runtime records need the stricter retry guard.
pub(crate) fn persisted_dashboard_activity_terminal_delta(
    record: &ApiInvocation,
) -> DashboardActivityTerminalDelta {
    dashboard_activity_terminal_delta_with_first_token(
        record,
        finite_nonnegative_timing(record.first_token_ms),
    )
}

struct DashboardActivityTerminalDeltaMetadata {
    model: String,
    reasoning_effort: Option<String>,
    upstream_account_name: Option<String>,
    string_bytes: usize,
    recent_projection_string_bytes: usize,
}

fn dashboard_activity_terminal_delta_metadata(
    record: &ApiInvocation,
) -> DashboardActivityTerminalDeltaMetadata {
    let model = record
        .response_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let reasoning_effort = record
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let upstream_account_name = record
        .upstream_account_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let string_bytes = record.invoke_id.len()
        + record.occurred_at.len()
        + record.source.len()
        + model.len()
        + reasoning_effort.as_ref().map_or(0, String::len)
        + upstream_account_name.as_ref().map_or(0, String::len);
    let recent_projection_string_bytes = record.invoke_id.len()
        + record.prompt_cache_key.as_ref().map_or(0, String::len)
        + record.occurred_at.len()
        + record.status.as_ref().map_or(0, String::len)
        + record.live_phase.as_ref().map_or(0, String::len)
        + record.failure_class.as_ref().map_or(0, String::len)
        + record.route_mode.as_ref().map_or(0, String::len)
        + record.model.as_ref().map_or(0, String::len)
        + record.request_model.as_ref().map_or(0, String::len)
        + record.response_model.as_ref().map_or(0, String::len)
        + record.proxy_display_name.as_ref().map_or(0, String::len)
        + record.upstream_account_name.as_ref().map_or(0, String::len)
        + record.endpoint.as_ref().map_or(0, String::len)
        + record
            .compaction_request_kind
            .as_ref()
            .map_or(0, String::len)
        + record
            .compaction_response_kind
            .as_ref()
            .map_or(0, String::len)
        + record.image_intent.as_ref().map_or(0, String::len)
        + record.reasoning_effort.as_ref().map_or(0, String::len)
        + record.error_message.as_ref().map_or(0, String::len)
        + record
            .downstream_error_message
            .as_ref()
            .map_or(0, String::len)
        + record.failure_kind.as_ref().map_or(0, String::len)
        + record
            .response_content_encoding
            .as_ref()
            .map_or(0, String::len)
        + record
            .request_compression_algorithm
            .as_ref()
            .map_or(0, String::len)
        + record.transport.as_ref().map_or(0, String::len)
        + record
            .requested_service_tier
            .as_ref()
            .map_or(0, String::len)
        + record.service_tier.as_ref().map_or(0, String::len)
        + record.billing_service_tier.as_ref().map_or(0, String::len);
    DashboardActivityTerminalDeltaMetadata {
        model,
        reasoning_effort,
        upstream_account_name,
        string_bytes,
        recent_projection_string_bytes,
    }
}

fn dashboard_activity_terminal_delta_with_first_token(
    record: &ApiInvocation,
    first_token_ms: Option<f64>,
) -> DashboardActivityTerminalDelta {
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable.map(i64::from),
    );
    let success = runtime_record_is_success_for_summary(record)
        && classification.failure_class == FailureClass::None;
    let failure = matches!(
        classification.failure_class,
        FailureClass::ServiceFailure | FailureClass::ClientFailure | FailureClass::ClientAbort
    );
    let cache_read_tokens = record.cache_input_tokens.unwrap_or_default().max(0);
    let cache_write_tokens = record
        .cache_write_tokens
        .unwrap_or_else(|| {
            record
                .input_tokens
                .unwrap_or_default()
                .saturating_sub(cache_read_tokens)
        })
        .max(0);
    let metadata = dashboard_activity_terminal_delta_metadata(record);
    let breakdown_complete = record.cost.is_some()
        && record.cost_input.is_some()
        && record.cost_cache_write.is_some()
        && record.cost_cache_read.is_some()
        && record.cost_output.is_some()
        && record.cost_reasoning.is_some();
    let total_cost = record.cost.unwrap_or_default().max(0.0);
    let mut timeseries = TimeseriesTerminalDelta::from(record);
    timeseries.first_token_ms = first_token_ms;
    DashboardActivityTerminalDelta {
        terminal_sequence: 0,
        timeseries,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: record.source.clone(),
        model: metadata.model,
        reasoning_effort: metadata.reasoning_effort,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: metadata.upstream_account_name,
        success,
        failure,
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        cache_write_tokens,
        cache_read_tokens,
        output_tokens: record.output_tokens.unwrap_or_default().max(0),
        has_cost: record.cost.is_some(),
        total_cost,
        cost_input: record
            .cost_input
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_cache_write: record
            .cost_cache_write
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_cache_read: record
            .cost_cache_read
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_output: record
            .cost_output
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_reasoning: record
            .cost_reasoning
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_unknown: if breakdown_complete { 0.0 } else { total_cost },
        t_total_ms: record.t_total_ms,
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        first_token_ms,
        t_upstream_stream_ms: record.t_upstream_stream_ms,
        recent_invocation: invocation_preview_from_runtime_record(record),
        persisted_row_id: (record.id > 0).then_some(record.id),
        estimated_bytes: std::mem::size_of::<DashboardActivityTerminalDelta>()
            + metadata.string_bytes
            + metadata.recent_projection_string_bytes,
    }
}

fn dashboard_activity_terminal_delta_matches_source_scope(
    delta: &DashboardActivityTerminalDelta,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope != InvocationSourceScope::ProxyOnly || delta.source == SOURCE_PROXY
}

fn dashboard_activity_terminal_delta_is_within_range(
    delta: &DashboardActivityTerminalDelta,
    range: ExactUtcRange,
) -> bool {
    parse_to_utc_datetime(&delta.occurred_at)
        .is_some_and(|occurred_at| occurred_at >= range.start && occurred_at < range.end)
}

fn add_dashboard_activity_terminal_usage(
    usage: &mut UsageBreakdownResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let costs = delta.has_cost.then_some(UsageCostBreakdownResponse {
        input: delta.cost_input,
        cache_write: delta.cost_cache_write,
        cache_read: delta.cost_cache_read,
        output: delta.cost_output,
        reasoning: delta.cost_reasoning,
        unknown: delta.cost_unknown,
    });

    usage.cache_write_tokens += delta.cache_write_tokens;
    usage.cache_read_tokens += delta.cache_read_tokens;
    usage.output_tokens += delta.output_tokens;
    if let Some(costs) = &costs {
        let target = usage
            .costs
            .get_or_insert_with(UsageCostBreakdownResponse::default);
        target.input += costs.input;
        target.cache_write += costs.cache_write;
        target.cache_read += costs.cache_read;
        target.output += costs.output;
        target.reasoning += costs.reasoning;
        target.unknown += costs.unknown;
    }

    let model_usage = usage.models.iter_mut().find(|entry| {
        entry.model == delta.model
            && entry.reasoning_effort.as_deref() == delta.reasoning_effort.as_deref()
    });
    let entry = if let Some(entry) = model_usage {
        entry
    } else {
        usage.models.push(UsageBreakdownModelResponse {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
        });
        usage.models.last_mut().expect("just inserted usage model")
    };
    entry.cache_write_tokens += delta.cache_write_tokens;
    entry.cache_read_tokens += delta.cache_read_tokens;
    entry.output_tokens += delta.output_tokens;
    if let Some(costs) = costs {
        let target = entry
            .costs
            .get_or_insert_with(UsageCostBreakdownResponse::default);
        target.input += costs.input;
        target.cache_write += costs.cache_write;
        target.cache_read += costs.cache_read;
        target.output += costs.output;
        target.reasoning += costs.reasoning;
        target.unknown += costs.unknown;
    }
    usage.models.sort_by(|left, right| {
        left.model
            .cmp(&right.model)
            .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
    });
}

fn subtract_dashboard_activity_terminal_usage(
    usage: &mut UsageBreakdownResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    usage.cache_write_tokens = usage
        .cache_write_tokens
        .saturating_sub(delta.cache_write_tokens);
    usage.cache_read_tokens = usage
        .cache_read_tokens
        .saturating_sub(delta.cache_read_tokens);
    usage.output_tokens = usage.output_tokens.saturating_sub(delta.output_tokens);
    if let Some(costs) = usage.costs.as_mut() {
        costs.input = (costs.input - delta.cost_input).max(0.0);
        costs.cache_write = (costs.cache_write - delta.cost_cache_write).max(0.0);
        costs.cache_read = (costs.cache_read - delta.cost_cache_read).max(0.0);
        costs.output = (costs.output - delta.cost_output).max(0.0);
        costs.reasoning = (costs.reasoning - delta.cost_reasoning).max(0.0);
        costs.unknown = (costs.unknown - delta.cost_unknown).max(0.0);
    }
    if let Some(entry) = usage.models.iter_mut().find(|entry| {
        entry.model == delta.model
            && entry.reasoning_effort.as_deref() == delta.reasoning_effort.as_deref()
    }) {
        entry.cache_write_tokens = entry
            .cache_write_tokens
            .saturating_sub(delta.cache_write_tokens);
        entry.cache_read_tokens = entry
            .cache_read_tokens
            .saturating_sub(delta.cache_read_tokens);
        entry.output_tokens = entry.output_tokens.saturating_sub(delta.output_tokens);
        if let Some(costs) = entry.costs.as_mut() {
            costs.input = (costs.input - delta.cost_input).max(0.0);
            costs.cache_write = (costs.cache_write - delta.cost_cache_write).max(0.0);
            costs.cache_read = (costs.cache_read - delta.cost_cache_read).max(0.0);
            costs.output = (costs.output - delta.cost_output).max(0.0);
            costs.reasoning = (costs.reasoning - delta.cost_reasoning).max(0.0);
            costs.unknown = (costs.unknown - delta.cost_unknown).max(0.0);
        }
    }
    usage.models.retain(|entry| {
        entry.cache_write_tokens != 0
            || entry.cache_read_tokens != 0
            || entry.output_tokens != 0
            || entry.costs.as_ref().is_some_and(|costs| {
                costs.input != 0.0
                    || costs.cache_write != 0.0
                    || costs.cache_read != 0.0
                    || costs.output != 0.0
                    || costs.reasoning != 0.0
                    || costs.unknown != 0.0
            })
    });
}

fn dashboard_activity_terminal_account(
    snapshot: &DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) -> DashboardActivityAccountResponse {
    dashboard_activity_terminal_account_for_range(
        ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        },
        delta,
    )
}

pub(crate) fn dashboard_activity_terminal_account_for_range(
    range: ExactUtcRange,
    delta: &DashboardActivityTerminalDelta,
) -> DashboardActivityAccountResponse {
    let upstream_account_id = delta.upstream_account_id;
    DashboardActivityAccountResponse {
        account_key: upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string()),
        upstream_account_id,
        display_name: upstream_account_id
            .and_then(|_| delta.upstream_account_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                upstream_account_id
                    .map(|id| format!("Upstream {id}"))
                    .unwrap_or_else(|| "Unassigned upstream account".to_string())
            }),
        is_unassigned: upstream_account_id.is_none(),
        latest_conversation_created_at: None,
        last_invocation_at: None,
        group_name: None,
        plan_type: None,
        enabled: None,
        display_status: None,
        enable_status: None,
        work_status: None,
        health_status: None,
        sync_state: None,
        last_error: None,
        last_action_reason_message: None,
        request_count: 0,
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
        usage_breakdown: UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        },
        model_performance: ModelPerformanceAccumulator::default().into_response(range, false),
        cache_hit_rate: None,
        tokens_per_minute: None,
        spend_rate: None,
        first_byte_avg_ms: None,
        first_response_byte_total_avg_ms: None,
        first_token_avg_ms: None,
        avg_total_ms: None,
        current_first_response_byte_total_avg_ms: None,
        current_first_token_avg_ms: None,
        current_avg_total_ms: None,
        current_avg_response_ms: None,
        in_progress_invocation_count: Some(0),
        in_progress_phase_counts: Some(InvocationPhaseCountsResponse::default()),
        retry_invocation_count: Some(0),
        upload_bytes_per_second: 0.0,
        download_bytes_per_second: 0.0,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        effective_routing_rule: None,
        recent_invocations: Vec::new(),
    }
}

fn apply_dashboard_activity_compact_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) {
    snapshot.terminal_sequence = snapshot.terminal_sequence.max(delta.terminal_sequence);
    apply_dashboard_activity_terminal_delta_to_stats(&mut snapshot.summary.stats, delta);

    let model_performance_available = snapshot.summary.model_performance.available;
    let snapshot_range = ExactUtcRange {
        start: snapshot.range_start,
        end: snapshot.range_end,
    };
    if snapshot.model_performance_accumulator_ready {
        snapshot
            .summary_model_performance_accumulator
            .add_terminal_delta(delta);
        snapshot.summary.model_performance = snapshot
            .summary_model_performance_accumulator
            .clone()
            .into_response(snapshot_range, model_performance_available);
        snapshot
            .account_model_performance_accumulators
            .entry(delta.upstream_account_id)
            .or_default()
            .add_terminal_delta(delta);
    }
    let account_model_performance = snapshot
        .account_model_performance_accumulators
        .get(&delta.upstream_account_id)
        .cloned()
        .unwrap_or_default()
        .into_response(snapshot_range, model_performance_available);

    if !snapshot
        .accounts
        .iter()
        .any(|account| account.upstream_account_id == delta.upstream_account_id)
    {
        snapshot
            .accounts
            .push(dashboard_activity_terminal_account(snapshot, delta));
    }
    let account_latency = snapshot
        .account_latency_accumulators
        .entry(delta.upstream_account_id)
        .or_default();
    account_latency.add_terminal_delta(delta);
    let account_latency = *account_latency;
    let account = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == delta.upstream_account_id)
        .expect("terminal account inserted when absent");
    apply_dashboard_activity_terminal_delta_to_account(account, delta);
    account_latency.apply_to_account(account);
    account.model_performance = account_model_performance;
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
}

pub(crate) fn apply_dashboard_activity_terminal_delta_to_stats(
    stats: &mut StatsResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    stats.total_count += 1;
    stats.success_count += i64::from(delta.success);
    stats.failure_count += i64::from(delta.failure);
    stats.total_tokens += delta.total_tokens;
    stats.total_cost += delta.total_cost;
    if non_success {
        if let Some(non_success_cost) = stats.non_success_cost.as_mut() {
            *non_success_cost += delta.total_cost;
        }
        if let Some(non_success_tokens) = stats.non_success_tokens.as_mut() {
            *non_success_tokens += delta.total_tokens;
        }
    }
    if let Some(usage) = stats.usage_breakdown.as_mut() {
        add_dashboard_activity_terminal_usage(usage, delta);
    }
}

pub(crate) fn apply_dashboard_activity_terminal_delta_to_account(
    account: &mut DashboardActivityAccountResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    account.request_count += 1;
    account.success_count += i64::from(delta.success);
    account.failure_count += i64::from(delta.failure);
    account.non_success_count += i64::from(non_success);
    account.total_tokens += delta.total_tokens;
    account.total_cost += delta.total_cost;
    if delta.success {
        account.success_tokens += delta.total_tokens;
    } else {
        account.non_success_tokens += delta.total_tokens;
    }
    if delta.failure {
        account.failure_tokens += delta.total_tokens;
        account.failure_cost += delta.total_cost;
    }
    if non_success {
        account.non_success_cost += delta.total_cost;
    }
    if account
        .last_invocation_at
        .as_deref()
        .is_none_or(|current| delta.occurred_at.as_str() > current)
    {
        account.last_invocation_at = Some(delta.occurred_at.clone());
    }
    add_dashboard_activity_terminal_usage(&mut account.usage_breakdown, delta);
    account.cache_hit_rate = (account.total_tokens > 0)
        .then_some(account.usage_breakdown.cache_read_tokens as f64 / account.total_tokens as f64);
}

fn apply_dashboard_activity_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    record: &ApiInvocation,
) {
    apply_dashboard_activity_compact_terminal_delta(
        snapshot,
        &dashboard_activity_terminal_delta(record),
    );
}

fn subtract_dashboard_activity_compact_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    subtract_dashboard_activity_terminal_stats(&mut snapshot.summary.stats, delta, non_success);
    let snapshot_range = ExactUtcRange {
        start: snapshot.range_start,
        end: snapshot.range_end,
    };
    let available = snapshot.summary.model_performance.available;
    if snapshot.model_performance_accumulator_ready {
        snapshot
            .summary_model_performance_accumulator
            .subtract_terminal_delta(delta);
        snapshot.summary.model_performance = snapshot
            .summary_model_performance_accumulator
            .clone()
            .into_response(snapshot_range, available);
        if let Some(accumulator) = snapshot
            .account_model_performance_accumulators
            .get_mut(&delta.upstream_account_id)
        {
            accumulator.subtract_terminal_delta(delta);
        }
    }
    if let Some(latency) = snapshot
        .account_latency_accumulators
        .get_mut(&delta.upstream_account_id)
    {
        latency.subtract_terminal_delta(delta);
    }
    let account_performance = snapshot
        .account_model_performance_accumulators
        .get(&delta.upstream_account_id)
        .cloned()
        .unwrap_or_default()
        .into_response(snapshot_range, available);
    if let Some(account) = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == delta.upstream_account_id)
    {
        account.request_count = account.request_count.saturating_sub(1);
        account.success_count = account
            .success_count
            .saturating_sub(i64::from(delta.success));
        account.failure_count = account
            .failure_count
            .saturating_sub(i64::from(delta.failure));
        account.non_success_count = account
            .non_success_count
            .saturating_sub(i64::from(non_success));
        account.total_tokens = account.total_tokens.saturating_sub(delta.total_tokens);
        account.total_cost = (account.total_cost - delta.total_cost).max(0.0);
        if delta.success {
            account.success_tokens = account.success_tokens.saturating_sub(delta.total_tokens);
        } else {
            account.non_success_tokens = account
                .non_success_tokens
                .saturating_sub(delta.total_tokens);
        }
        if delta.failure {
            account.failure_tokens = account.failure_tokens.saturating_sub(delta.total_tokens);
            account.failure_cost = (account.failure_cost - delta.total_cost).max(0.0);
        }
        if non_success {
            account.non_success_cost = (account.non_success_cost - delta.total_cost).max(0.0);
        }
        if account.last_invocation_at.as_deref() == Some(delta.occurred_at.as_str()) {
            account.last_invocation_at = None;
        }
        subtract_dashboard_activity_terminal_usage(&mut account.usage_breakdown, delta);
        if let Some(latency) = snapshot
            .account_latency_accumulators
            .get(&delta.upstream_account_id)
            .copied()
        {
            latency.apply_to_account(account);
        }
        account.cache_hit_rate = (account.total_tokens > 0).then_some(
            account.usage_breakdown.cache_read_tokens as f64 / account.total_tokens as f64,
        );
        account.model_performance = account_performance;
    }
    snapshot
        .accounts
        .retain(|account| account.request_count > 0);
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
}

fn subtract_dashboard_activity_terminal_stats(
    stats: &mut StatsResponse,
    delta: &DashboardActivityTerminalDelta,
    non_success: bool,
) {
    stats.total_count = stats.total_count.saturating_sub(1);
    stats.success_count = stats.success_count.saturating_sub(i64::from(delta.success));
    stats.failure_count = stats.failure_count.saturating_sub(i64::from(delta.failure));
    stats.total_tokens = stats.total_tokens.saturating_sub(delta.total_tokens);
    stats.total_cost = (stats.total_cost - delta.total_cost).max(0.0);
    if non_success {
        if let Some(value) = stats.non_success_cost.as_mut() {
            *value = (*value - delta.total_cost).max(0.0);
        }
        if let Some(value) = stats.non_success_tokens.as_mut() {
            *value = value.saturating_sub(delta.total_tokens);
        }
    }
    if let Some(usage) = stats.usage_breakdown.as_mut() {
        subtract_dashboard_activity_terminal_usage(usage, delta);
    }
}

fn restore_dashboard_activity_last_invocation_after_expiry(
    snapshot: &mut DashboardActivitySnapshot,
    expired: &DashboardActivityTerminalDelta,
    remaining_expiry_deltas: &VecDeque<DashboardActivityTerminalDelta>,
) {
    let Some(account) = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == expired.upstream_account_id)
    else {
        return;
    };
    if account.last_invocation_at.is_some() || account.request_count <= 0 {
        return;
    }
    account.last_invocation_at = remaining_expiry_deltas
        .iter()
        .filter(|delta| delta.upstream_account_id == expired.upstream_account_id)
        .max_by_key(|delta| parse_to_utc_datetime(&delta.occurred_at))
        .map(|delta| delta.occurred_at.clone());
}

/// Applies an accepted terminal record to every warm open-range baseline. This is deliberately
/// independent from the subscription listener so a lagged broadcast cannot drop cumulative UI
/// totals before the next reconciliation build.
pub(crate) async fn apply_dashboard_activity_terminal_record(
    state: &AppState,
    record: &ApiInvocation,
) -> DashboardActivityTerminalDeltaOutcome {
    let mut delta = dashboard_activity_terminal_delta(record);
    let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
        return DashboardActivityTerminalDeltaOutcome::default();
    };
    let key = delta.key();
    let mut outcome = DashboardActivityTerminalDeltaOutcome::default();
    let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
    if cache.read_model.applied_terminal_keys.contains_key(&key) {
        cache.read_model.duplicate_delta_count += 1;
        outcome.duplicate = true;
        return outcome;
    }
    cache.read_model.next_terminal_sequence =
        cache.read_model.next_terminal_sequence.saturating_add(1);
    delta.terminal_sequence = cache.read_model.next_terminal_sequence;
    outcome.terminal_sequence = Some(delta.terminal_sequence);
    insert_dashboard_activity_applied_terminal_key(&mut cache.read_model, key);
    cache.read_model.terminal_delta_count += 1;
    prune_dashboard_activity_applied_terminal_keys(&mut cache.read_model);
    // Registration happens before the SQLite enqueue.  Even when the caller already carries a
    // positive row id, that id is not a durability proof until the writer ACKs the committed
    // transaction below; advancing the contiguous watermark here would let a failed enqueue be
    // published as covered source progress.
    let hard_limit_reason = dashboard_activity_terminal_delta_needs_persistence_ack(&delta)
        .then(|| dashboard_activity_terminal_delta_hard_limit_reason(&cache.read_model, &delta))
        .flatten();
    if let Some(reason) = hard_limit_reason {
        cache.read_model.hard_limit_reason = Some(reason);
        cache.read_model.hard_limit_sequence = Some(delta.terminal_sequence);
        cache.read_model.pending_terminal_overflow_count += 1;
        outcome.hard_limit_reason = Some(reason);
        tracing::warn!(
            hard_limit_reason = reason,
            pending_delta_count = cache.read_model.pending_terminal_deltas.len(),
            pending_delta_estimated_bytes = cache.read_model.pending_delta_estimated_bytes,
            persisted_ack_pending_count = cache.read_model.persisted_ack_pending_count,
            "dashboard activity read model entered dirty last-good protection"
        );
        return outcome;
    }
    let mut expiry_hard_limit = None;
    for (selection, entry) in &mut cache.entries {
        if !dashboard_activity_selection_includes_compact_terminal(selection, &delta, occurred_at) {
            outcome.skipped_out_of_range_count += 1;
            continue;
        }
        if dashboard_activity_entry_includes_terminal(entry, &delta) {
            continue;
        }
        if let Err(reason) = insert_dashboard_activity_expiry_delta(
            &mut entry.expiry_terminal_deltas,
            &mut entry.expiry_delta_estimated_bytes,
            entry.expiry_covered_until,
            &delta,
            occurred_at,
        ) {
            entry.expiry_covered_until = Some(DateTime::<Utc>::MIN_UTC);
            expiry_hard_limit = Some(reason);
            continue;
        }
        apply_dashboard_activity_compact_terminal_delta(&mut entry.response, &delta);
        outcome.applied_selection_count += 1;
    }
    let evicted_entry_count = prune_dashboard_activity_snapshot_entries(&mut cache, None);
    if evicted_entry_count > 0 {
        prune_dashboard_activity_terminal_deltas(&mut cache);
        tracing::debug!(
            evicted_entry_count,
            cache_entry_count = cache.entries.len(),
            "evicted dashboard activity snapshots to enforce the global cache budget"
        );
    }
    if let Some(reason) = expiry_hard_limit {
        cache.read_model.hard_limit_reason = Some(reason);
        cache.read_model.pending_terminal_overflow_count += 1;
        outcome.hard_limit_reason = Some(reason);
        return outcome;
    }
    outcome.terminal_delta = Some(delta.clone());
    if !dashboard_activity_terminal_delta_needs_persistence_ack(&delta) {
        return outcome;
    }
    cache.read_model.pending_delta_estimated_bytes = cache
        .read_model
        .pending_delta_estimated_bytes
        .saturating_add(delta.estimated_bytes);
    cache.read_model.pending_terminal_deltas.push_back(delta);
    outcome
}

fn prune_dashboard_activity_applied_terminal_keys(read_model: &mut DashboardActivityReadModel) {
    let max_age = Duration::from_secs(7 * 24 * 60 * 60);
    while let Some((key, applied_at)) = read_model.applied_terminal_key_order.front() {
        let is_live_entry = read_model.applied_terminal_keys.get(key) == Some(applied_at);
        if is_live_entry
            && read_model.applied_terminal_keys.len()
                <= DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
            && applied_at.elapsed() <= max_age
        {
            break;
        }
        let key = key.clone();
        let applied_at = *applied_at;
        read_model.applied_terminal_key_order.pop_front();
        if read_model.applied_terminal_keys.get(&key) == Some(&applied_at) {
            read_model.applied_terminal_keys.remove(&key);
        }
    }
}
