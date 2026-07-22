use anyhow::{Result, anyhow};
use std::collections::BTreeMap;

use crate::api::{
    normalized_usage_breakdown_account_key, normalized_usage_breakdown_model,
    normalized_usage_breakdown_reasoning_effort, runtime_text_equals,
};
use crate::maintenance::invocation_status_is_success_like as archive_invocation_status_is_success_like;
use crate::{
    ApproxHistogramCounts, DETAIL_LEVEL_FULL, FailureClass, InvocationHourlySourceRecord,
    add_approx_histogram_sample, align_bucket_epoch, empty_approx_histogram,
    invocation_counts_toward_non_success_usage, invocation_status_counts_toward_terminal_totals,
    normalize_non_negative_timing_value, parse_to_utc_datetime, parse_utc_naive,
    resolve_failure_classification, resolve_first_response_byte_total_ms,
};

#[derive(Debug, Default)]
pub(crate) struct InvocationHourlyRollupDelta {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_sum_ms: f64,
    pub(crate) first_byte_max_ms: f64,
    pub(crate) first_byte_histogram: ApproxHistogramCounts,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_max_ms: f64,
    pub(crate) first_response_byte_total_histogram: ApproxHistogramCounts,
}

#[derive(Debug, Default)]
pub(crate) struct ProxyPerfStageHourlyDelta {
    pub(crate) sample_count: i64,
    pub(crate) sum_ms: f64,
    pub(crate) max_ms: f64,
    pub(crate) histogram: ApproxHistogramCounts,
}

#[derive(Debug, Default)]
pub(crate) struct KeyedConversationHourlyDelta {
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_seen_at: String,
    pub(crate) last_seen_at: String,
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountUsageHourlyDelta {
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) first_seen_at: String,
    pub(crate) last_seen_at: String,
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountUsageBreakdownHourlyDelta {
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
    pub(crate) performance_usage_duration_sample_count: i64,
    pub(crate) performance_usage_duration_sum_ms: f64,
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountStatsDelta {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) in_flight_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_sum_ms: f64,
    pub(crate) first_byte_max_ms: f64,
    pub(crate) first_byte_histogram: ApproxHistogramCounts,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_max_ms: f64,
    pub(crate) first_response_byte_total_histogram: ApproxHistogramCounts,
}

#[derive(Debug, Default)]
pub(crate) struct ForwardProxyAttemptHourlyDelta {
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) latency_sample_count: i64,
    pub(crate) latency_sum_ms: f64,
    pub(crate) latency_max_ms: f64,
}

pub(crate) fn invocation_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    let occurred_at_utc = parse_to_utc_datetime(occurred_at)
        .ok_or_else(|| anyhow!("failed to parse invocation occurred_at: {occurred_at}"))?;
    Ok(align_bucket_epoch(occurred_at_utc.timestamp(), 3600, 0))
}

pub(crate) fn invocation_bucket_start_epoch_for_seconds(
    occurred_at: &str,
    bucket_seconds: i64,
) -> Result<i64> {
    let occurred_at_utc = parse_to_utc_datetime(occurred_at)
        .ok_or_else(|| anyhow!("failed to parse invocation occurred_at: {occurred_at}"))?;
    Ok(align_bucket_epoch(
        occurred_at_utc.timestamp(),
        bucket_seconds,
        0,
    ))
}

pub(crate) fn forward_proxy_attempt_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    Ok(align_bucket_epoch(
        parse_utc_naive(occurred_at)?.and_utc().timestamp(),
        3600,
        0,
    ))
}

pub(crate) fn keyed_conversation_delta<'a>(
    map: &'a mut BTreeMap<(i64, String, String), KeyedConversationHourlyDelta>,
    bucket_start_epoch: i64,
    source: &str,
    key: &str,
    occurred_at: &str,
) -> &'a mut KeyedConversationHourlyDelta {
    let entry = map
        .entry((bucket_start_epoch, source.to_string(), key.to_string()))
        .or_insert_with(|| KeyedConversationHourlyDelta {
            first_seen_at: occurred_at.to_string(),
            last_seen_at: occurred_at.to_string(),
            ..KeyedConversationHourlyDelta::default()
        });
    if entry.first_seen_at.is_empty() || occurred_at < entry.first_seen_at.as_str() {
        entry.first_seen_at = occurred_at.to_string();
    }
    if entry.last_seen_at.is_empty() || occurred_at > entry.last_seen_at.as_str() {
        entry.last_seen_at = occurred_at.to_string();
    }
    entry
}

pub(crate) fn record_proxy_perf_stage_sample(
    map: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    bucket_start_epoch: i64,
    stage: &str,
    value_ms: Option<f64>,
) {
    let Some(value_ms) = value_ms else {
        return;
    };
    if !value_ms.is_finite() || value_ms < 0.0 {
        return;
    }
    let entry = map
        .entry((bucket_start_epoch, stage.to_string()))
        .or_insert_with(|| ProxyPerfStageHourlyDelta {
            histogram: empty_approx_histogram(),
            ..ProxyPerfStageHourlyDelta::default()
        });
    entry.sample_count += 1;
    entry.sum_ms += value_ms;
    entry.max_ms = entry.max_ms.max(value_ms);
    add_approx_histogram_sample(&mut entry.histogram, value_ms);
}

pub(crate) fn accumulate_invocation_hourly_overall_rollups(
    overall: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        let overall_entry = overall
            .entry((bucket_start_epoch, row.source.clone()))
            .or_insert_with(|| InvocationHourlyRollupDelta {
                first_byte_histogram: empty_approx_histogram(),
                first_response_byte_total_histogram: empty_approx_histogram(),
                ..InvocationHourlyRollupDelta::default()
            });
        overall_entry.total_count += 1;
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        let has_terminal_status =
            invocation_status_counts_toward_terminal_totals(row.status.as_deref());
        let is_success_like = archive_invocation_status_is_success_like(
            row.status.as_deref(),
            row.error_message.as_deref(),
        ) && classification.failure_class == FailureClass::None;
        if is_success_like {
            overall_entry.success_count += 1;
        } else if has_terminal_status && classification.failure_class != FailureClass::None {
            overall_entry.failure_count += 1;
        }
        overall_entry.total_tokens += row.total_tokens.unwrap_or_default();
        overall_entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
        let cost = row.cost.unwrap_or_default();
        overall_entry.total_cost += cost;
        if invocation_counts_toward_non_success_usage(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        ) {
            overall_entry.non_success_cost += cost;
        }
        if has_terminal_status
            && let Some(total_ms) = normalize_non_negative_timing_value(row.t_total_ms)
        {
            overall_entry.total_latency_sample_count += 1;
            overall_entry.total_latency_sum_ms += total_ms;
        }
        if is_success_like
            && let Some(ttfb_ms) = row.t_upstream_ttfb_ms
            && ttfb_ms.is_finite()
            && ttfb_ms > 0.0
        {
            overall_entry.first_byte_sample_count += 1;
            overall_entry.first_byte_sum_ms += ttfb_ms;
            overall_entry.first_byte_max_ms = overall_entry.first_byte_max_ms.max(ttfb_ms);
            add_approx_histogram_sample(&mut overall_entry.first_byte_histogram, ttfb_ms);
        }
        if let Some(first_response_byte_total_ms) = resolve_first_response_byte_total_ms(
            row.t_req_read_ms,
            row.t_req_parse_ms,
            row.t_upstream_connect_ms,
            row.t_upstream_ttfb_ms,
        ) {
            overall_entry.first_response_byte_total_sample_count += 1;
            overall_entry.first_response_byte_total_sum_ms += first_response_byte_total_ms;
            overall_entry.first_response_byte_total_max_ms = overall_entry
                .first_response_byte_total_max_ms
                .max(first_response_byte_total_ms);
            add_approx_histogram_sample(
                &mut overall_entry.first_response_byte_total_histogram,
                first_response_byte_total_ms,
            );
        }
    }

    Ok(())
}

pub(crate) fn accumulate_upstream_account_usage_breakdown_rollup(
    breakdowns: &mut BTreeMap<
        (i64, String, String, Option<i64>, String, String),
        UpstreamAccountUsageBreakdownHourlyDelta,
    >,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    if runtime_text_equals(row.status.as_deref(), "running")
        || runtime_text_equals(row.status.as_deref(), "pending")
    {
        return Ok(());
    }

    let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
    let (upstream_account_key, upstream_account_id) = normalized_usage_breakdown_account_key(
        row.payload.as_deref(),
        row.resolved_upstream_account_id(),
    );
    let model = normalized_usage_breakdown_model(row.model.as_deref(), row.payload.as_deref());
    let reasoning_effort =
        normalized_usage_breakdown_reasoning_effort(row.payload.as_deref()).unwrap_or_default();
    let entry = breakdowns
        .entry((
            bucket_start_epoch,
            row.source.clone(),
            upstream_account_key,
            upstream_account_id,
            model,
            reasoning_effort,
        ))
        .or_default();
    entry.request_count += 1;

    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let has_terminal_status =
        invocation_status_counts_toward_terminal_totals(row.status.as_deref());
    let is_success_like = archive_invocation_status_is_success_like(
        row.status.as_deref(),
        row.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if has_terminal_status && classification.failure_class != FailureClass::None {
        entry.failure_count += 1;
    }

    let input_tokens = row.input_tokens.unwrap_or_default().max(0);
    let cache_read_tokens = row.cache_input_tokens.unwrap_or_default().max(0);
    entry.cache_write_tokens += input_tokens.saturating_sub(cache_read_tokens);
    entry.cache_read_tokens += cache_read_tokens;
    entry.output_tokens += row.output_tokens.unwrap_or_default().max(0);

    if let Some(total_cost) = row.cost {
        entry.has_cost += 1;
        if let (
            Some(cost_input),
            Some(cost_cache_write),
            Some(cost_cache_read),
            Some(cost_output),
            Some(cost_reasoning),
        ) = (
            row.cost_input,
            row.cost_cache_write,
            row.cost_cache_read,
            row.cost_output,
            row.cost_reasoning,
        ) {
            entry.cost_input += cost_input;
            entry.cost_cache_write += cost_cache_write;
            entry.cost_cache_read += cost_cache_read;
            entry.cost_output += cost_output;
            entry.cost_reasoning += cost_reasoning;
        } else {
            entry.cost_unknown += total_cost;
        }
    }

    let success_billed = is_success_like && row.cost.is_some();
    if success_billed {
        entry.performance_total_tokens += row.total_tokens.unwrap_or_default().max(0);
        if let Some(stream_duration_ms) =
            normalize_non_negative_timing_value(row.t_upstream_stream_ms)
        {
            entry.performance_stream_output_tokens += row.output_tokens.unwrap_or_default().max(0);
            entry.performance_stream_duration_ms += stream_duration_ms;
            entry.performance_response_sample_count += 1;
            entry.performance_response_sum_ms += stream_duration_ms;
        }
        if let Some(ttfb_ms) = row
            .t_upstream_ttfb_ms
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            entry.performance_first_byte_sample_count += 1;
            entry.performance_first_byte_sum_ms +=
                normalize_non_negative_timing_value(row.t_req_read_ms).unwrap_or_default()
                    + normalize_non_negative_timing_value(row.t_req_parse_ms).unwrap_or_default()
                    + normalize_non_negative_timing_value(row.t_upstream_connect_ms)
                        .unwrap_or_default()
                    + ttfb_ms;
        }
        if let Some(total_duration_ms) = normalize_non_negative_timing_value(row.t_total_ms) {
            entry.performance_usage_duration_sample_count += 1;
            entry.performance_usage_duration_sum_ms += total_duration_ms;
        }
    }

    Ok(())
}

pub(crate) fn invocation_archive_has_pruned_success_details(
    rows: &[InvocationHourlySourceRecord],
) -> bool {
    rows.iter().any(|row| {
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        row.detail_level != DETAIL_LEVEL_FULL
            && archive_invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            )
            && classification.failure_class == FailureClass::None
    })
}

pub(crate) fn accumulate_upstream_account_stats_delta(
    entry: &mut UpstreamAccountStatsDelta,
    row: &InvocationHourlySourceRecord,
) {
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let has_terminal_status =
        invocation_status_counts_toward_terminal_totals(row.status.as_deref());
    let is_success_like = archive_invocation_status_is_success_like(
        row.status.as_deref(),
        row.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;

    entry.total_count += 1;
    if is_success_like {
        entry.success_count += 1;
    } else if has_terminal_status && classification.failure_class != FailureClass::None {
        entry.failure_count += 1;
    } else {
        entry.in_flight_count += 1;
    }
    entry.total_tokens += row.total_tokens.unwrap_or_default();
    entry.input_tokens += row.input_tokens.unwrap_or_default();
    entry.output_tokens += row.output_tokens.unwrap_or_default();
    entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
    let cost = row.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    ) {
        entry.non_success_cost += cost;
    }
    if has_terminal_status
        && let Some(total_ms) = normalize_non_negative_timing_value(row.t_total_ms)
    {
        entry.total_latency_sample_count += 1;
        entry.total_latency_sum_ms += total_ms;
    }

    if is_success_like
        && let Some(ttfb_ms) = row.t_upstream_ttfb_ms
        && ttfb_ms.is_finite()
        && ttfb_ms > 0.0
    {
        if entry.first_byte_histogram.is_empty() {
            entry.first_byte_histogram = empty_approx_histogram();
        }
        entry.first_byte_sample_count += 1;
        entry.first_byte_sum_ms += ttfb_ms;
        entry.first_byte_max_ms = entry.first_byte_max_ms.max(ttfb_ms);
        add_approx_histogram_sample(&mut entry.first_byte_histogram, ttfb_ms);
    }
    if let Some(first_response_byte_total_ms) = resolve_first_response_byte_total_ms(
        row.t_req_read_ms,
        row.t_req_parse_ms,
        row.t_upstream_connect_ms,
        row.t_upstream_ttfb_ms,
    ) {
        if entry.first_response_byte_total_histogram.is_empty() {
            entry.first_response_byte_total_histogram = empty_approx_histogram();
        }
        entry.first_response_byte_total_sample_count += 1;
        entry.first_response_byte_total_sum_ms += first_response_byte_total_ms;
        entry.first_response_byte_total_max_ms = entry
            .first_response_byte_total_max_ms
            .max(first_response_byte_total_ms);
        add_approx_histogram_sample(
            &mut entry.first_response_byte_total_histogram,
            first_response_byte_total_ms,
        );
    }
}
