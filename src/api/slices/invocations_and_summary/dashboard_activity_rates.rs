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
    latest_conversation_created_at: Option<String>,
    last_invocation_at: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    failure_count: i64,
    non_success_count: i64,
    pub(crate) total_tokens: i64,
    success_tokens: i64,
    non_success_tokens: i64,
    failure_tokens: i64,
    failure_cost: f64,
    non_success_cost: f64,
    cache_input_tokens: i64,
    pub(crate) total_cost: f64,
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
}

fn merge_upstream_account_activity_aggregate_row(
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
struct UpstreamAccountPromptCacheCreatedAtRow {
    upstream_account_id: Option<i64>,
    prompt_cache_key: String,
    first_occurred_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountUsageBreakdownAggregateRow {
    upstream_account_id: Option<i64>,
    model: String,
    reasoning_effort: Option<String>,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    cost_input: f64,
    cost_cache_write: f64,
    cost_cache_read: f64,
    cost_output: f64,
    cost_reasoning: f64,
    cost_unknown: f64,
    has_cost: i64,
    performance_total_tokens: i64,
    performance_stream_output_tokens: i64,
    performance_stream_duration_ms: f64,
    performance_response_sample_count: i64,
    performance_response_sum_ms: f64,
    performance_first_byte_sample_count: i64,
    performance_first_byte_sum_ms: f64,
    performance_first_token_sample_count: i64,
    performance_first_token_sum_ms: f64,
    performance_usage_duration_sample_count: i64,
    performance_usage_duration_sum_ms: f64,
}

type UsageBreakdownAggregateMergeKey = (Option<i64>, String, Option<String>);

fn merge_usage_breakdown_aggregate_row_map(
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

fn merge_usage_breakdown_rollup_record_map(
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
