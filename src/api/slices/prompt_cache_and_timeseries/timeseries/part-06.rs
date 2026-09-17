pub(crate) fn add_rollup_rows_to_timeseries_aggregates(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    rows: Vec<UpstreamAccountStatsRollupRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for row in rows {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += row.total_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.in_flight_count += row.in_flight_count;
            entry.total_tokens += row.total_tokens;
            entry.input_tokens += row.input_tokens;
            entry.output_tokens += row.output_tokens;
            entry.cache_input_tokens += row.cache_input_tokens;
            entry.reasoning_tokens += row.reasoning_tokens;
            if row.total_tokens > 0 {
                entry.token_components_observed = true;
            }
            entry.total_cost += row.total_cost;
            entry.non_success_cost += row.non_success_cost;
            entry.total_latency_sample_count += row.total_latency_sample_count;
            entry.total_latency_sum_ms += row.total_latency_sum_ms;
            entry.first_byte_sample_count += row.first_byte_sample_count;
            entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
            entry.first_byte_histogram =
                merge_rollup_histogram(&entry.first_byte_histogram, &row.first_byte_histogram)?;
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_response_byte_total_histogram = merge_rollup_histogram(
                &entry.first_response_byte_total_histogram,
                &row.first_response_byte_total_histogram,
            )?;
            entry.first_token_sample_count += row.first_token_sample_count;
            entry.first_token_sum_ms += row.first_token_sum_ms;
            entry.first_token_histogram =
                merge_rollup_histogram(&entry.first_token_histogram, &row.first_token_histogram)?;
        }
    }
    Ok(())
}

fn merge_rollup_histogram(current: &[i64], incoming_json: &str) -> Result<Vec<i64>, ApiError> {
    let incoming = decode_approx_histogram(incoming_json);
    if current.is_empty() {
        return Ok(incoming);
    }
    let mut merged = current.to_vec();
    merge_approx_histogram_into(&mut merged, &incoming)?;
    Ok(merged)
}

pub(crate) fn add_exact_records_to_timeseries_aggregates(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    records: Vec<InvocationAggregateRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            add_exact_record_to_timeseries_aggregate(entry, &record);
        }
    }
    Ok(())
}

pub(crate) fn add_exact_record_to_timeseries_aggregate(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    entry.total_count += 1;
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    );
    let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
        record.status.as_deref(),
        record.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        entry.in_flight_count += 1;
        entry
            .in_flight_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        record.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    let latency_status = if is_success_like {
        Some("success")
    } else {
        record.status.as_deref()
    };
    if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        entry.record_total_latency_sample(record.t_total_ms);
    }
    entry.record_exact_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
    entry.record_exact_first_response_byte_total_sample(
        record.t_req_read_ms,
        record.t_req_parse_ms,
        record.t_upstream_connect_ms,
        record.t_upstream_ttfb_ms,
    );
    entry.record_first_token_sample(record.first_token_ms);
    add_optional_token_components(
        entry,
        record.total_tokens,
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
    );
    let cost = record.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    ) {
        entry.non_success_cost += cost;
    }
}

pub(crate) fn subtract_stale_in_flight_record_from_timeseries_aggregate(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        return;
    }
    entry.total_count = entry.total_count.saturating_sub(1);
    entry.in_flight_count = entry.in_flight_count.saturating_sub(1);
    entry
        .in_flight_phase_counts
        .decrement_phase_name(record.live_phase.as_deref());
    subtract_optional_token_components(entry, record);
    entry.total_cost = (entry.total_cost - record.cost.unwrap_or_default()).max(0.0);
    entry.remove_exact_first_response_byte_total_sample(
        record.t_req_read_ms,
        record.t_req_parse_ms,
        record.t_upstream_connect_ms,
        record.t_upstream_ttfb_ms,
    );
    entry.remove_exact_first_token_sample(record.first_token_ms);
}
