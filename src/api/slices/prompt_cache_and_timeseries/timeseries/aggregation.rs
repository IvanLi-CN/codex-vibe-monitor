use super::minute_projection::timeseries_projection_scope;
use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;
pub(crate) fn fold_minute_projection_aggregates(
    minute_aggregates: BTreeMap<i64, BucketAggregate>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
    let mut buckets = BTreeMap::new();
    for (minute_epoch, aggregate) in minute_aggregates {
        let bucket_epoch =
            align_reporting_bucket_epoch(minute_epoch, bucket_seconds, reporting_tz)?;
        merge_timeseries_bucket_aggregate(buckets.entry(bucket_epoch).or_default(), aggregate);
    }
    Ok(buckets)
}

pub(crate) fn add_timeseries_terminal_delta_to_aggregate(
    entry: &mut BucketAggregate,
    delta: &TimeseriesTerminalDelta,
) {
    entry.total_count += 1;
    let classification = resolve_failure_classification(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    );
    let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        delta.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    let latency_status = is_success_like
        .then_some("success")
        .or(delta.status.as_deref());
    entry.record_total_latency_sample(delta.t_total_ms);
    entry.record_exact_ttfb_sample(latency_status, delta.t_upstream_ttfb_ms);
    entry.record_exact_first_response_byte_total_sample(
        delta.t_req_read_ms,
        delta.t_req_parse_ms,
        delta.t_upstream_connect_ms,
        delta.t_upstream_ttfb_ms,
    );
    entry.record_first_token_sample(delta.first_token_ms);
    add_optional_token_components(
        entry,
        delta.total_tokens,
        delta.input_tokens,
        delta.output_tokens,
        delta.cache_input_tokens,
        delta.reasoning_tokens,
    );
    let cost = delta.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    ) {
        entry.non_success_cost += cost;
    }
}

pub(crate) fn add_optional_token_components(
    entry: &mut BucketAggregate,
    total_tokens: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
) {
    entry.total_tokens += total_tokens.unwrap_or_default();
    entry.input_tokens += input_tokens.unwrap_or_default();
    entry.output_tokens += output_tokens.unwrap_or_default();
    entry.cache_input_tokens += cache_input_tokens.unwrap_or_default();
    entry.reasoning_tokens += reasoning_tokens.unwrap_or_default();

    if total_tokens.unwrap_or_default() > 0 {
        entry.token_components_observed = true;
        if input_tokens.is_none()
            || output_tokens.is_none()
            || cache_input_tokens.is_none()
            || reasoning_tokens.is_none()
        {
            entry.token_component_incomplete_count += 1;
        }
    }
}

pub(crate) fn subtract_optional_token_components(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    entry.total_tokens = entry
        .total_tokens
        .saturating_sub(record.total_tokens.unwrap_or_default());
    entry.input_tokens = entry
        .input_tokens
        .saturating_sub(record.input_tokens.unwrap_or_default());
    entry.output_tokens = entry
        .output_tokens
        .saturating_sub(record.output_tokens.unwrap_or_default());
    entry.cache_input_tokens = entry
        .cache_input_tokens
        .saturating_sub(record.cache_input_tokens.unwrap_or_default());
    entry.reasoning_tokens = entry
        .reasoning_tokens
        .saturating_sub(record.reasoning_tokens.unwrap_or_default());
    if record.total_tokens.unwrap_or_default() > 0
        && (record.input_tokens.is_none()
            || record.output_tokens.is_none()
            || record.cache_input_tokens.is_none()
            || record.reasoning_tokens.is_none())
    {
        entry.token_component_incomplete_count =
            entry.token_component_incomplete_count.saturating_sub(1);
    }
}

pub(crate) fn add_pending_timeseries_deltas(
    state: &AppState,
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    max_row_id: Option<i64>,
    snapshot_id: i64,
) -> Result<usize, ApiError> {
    let mut applied = 0;
    let selection = TimeseriesProjectionSelection {
        source_scope: timeseries_projection_scope(source_scope),
        upstream_account_id,
    };
    for (_, row_id, delta) in state
        .terminal_projection_hub
        .pending_timeseries_deltas_for_selection(selection, 10_000)
    {
        if max_row_id.is_some_and(|cursor| row_id > cursor) {
            // Rows above the projection cursor are already represented by the SQL tail.
            continue;
        }
        if row_id > snapshot_id {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && delta.source != SOURCE_PROXY {
            continue;
        }
        if upstream_account_id
            .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
        {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
            continue;
        };
        if occurred < start || occurred >= end {
            continue;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_timeseries_terminal_delta_to_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            &delta,
        );
        applied += 1;
    }
    Ok(applied)
}

pub(crate) fn add_terminal_timeseries_records(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    records: Vec<InvocationAggregateRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_exact_record_to_timeseries_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            &record,
        );
    }
    Ok(())
}

pub(crate) fn fill_timeseries_buckets(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(i64, i64), ApiError> {
    let fill_start_epoch =
        align_reporting_bucket_epoch(start.timestamp(), bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }
    Ok((fill_start_epoch, fill_end_epoch))
}

pub(crate) fn overlay_runtime_timeseries_snapshot(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    runtime_records: &[ApiInvocation],
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in runtime_records {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref())
            || upstream_account_id
                .is_some_and(|account_id| record.upstream_account_id != Some(account_id))
        {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred < start || occurred >= end {
            continue;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        entry.in_flight_count += 1;
        entry
            .in_flight_phase_counts
            .increment_phase_name(runtime_record_live_phase(record));
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.record_first_token_sample(runtime_record_first_token_ms(record));
        add_optional_token_components(
            entry,
            record.total_tokens,
            record.input_tokens,
            record.output_tokens,
            record.cache_input_tokens,
            record.reasoning_tokens,
        );
        entry.total_cost += record.cost.unwrap_or_default();
    }
    Ok(())
}

pub(crate) fn merge_timeseries_bucket_aggregate(
    target: &mut BucketAggregate,
    source: BucketAggregate,
) {
    target.total_count += source.total_count;
    target.success_count += source.success_count;
    target.failure_count += source.failure_count;
    target.in_flight_count += source.in_flight_count;
    target.in_flight_phase_counts.queued += source.in_flight_phase_counts.queued;
    target.in_flight_phase_counts.requesting += source.in_flight_phase_counts.requesting;
    target.in_flight_phase_counts.responding += source.in_flight_phase_counts.responding;
    target.total_tokens += source.total_tokens;
    target.input_tokens += source.input_tokens;
    target.output_tokens += source.output_tokens;
    target.cache_input_tokens += source.cache_input_tokens;
    target.reasoning_tokens += source.reasoning_tokens;
    target.token_components_observed |= source.token_components_observed;
    target.token_component_incomplete_count += source.token_component_incomplete_count;
    target.total_cost += source.total_cost;
    target.non_success_cost += source.non_success_cost;
    target.total_latency_sum_ms += source.total_latency_sum_ms;
    target.total_latency_sample_count += source.total_latency_sample_count;
    target
        .total_latency_values
        .extend(source.total_latency_values);
    target.first_byte_ttfb_sum_ms += source.first_byte_ttfb_sum_ms;
    target.first_byte_sample_count += source.first_byte_sample_count;
    target.first_response_byte_total_sum_ms += source.first_response_byte_total_sum_ms;
    target.first_response_byte_total_sample_count += source.first_response_byte_total_sample_count;
    target.first_token_sum_ms += source.first_token_sum_ms;
    target.first_token_sample_count += source.first_token_sample_count;
    target
        .first_byte_ttfb_values
        .extend(source.first_byte_ttfb_values);
    target
        .first_response_byte_total_values
        .extend(source.first_response_byte_total_values);
    target.first_token_values.extend(source.first_token_values);
    let first_byte_histogram = source.first_byte_histogram;
    if target.first_byte_histogram.is_empty() {
        target.first_byte_histogram = first_byte_histogram;
    } else {
        for (target_value, source_value) in target
            .first_byte_histogram
            .iter_mut()
            .zip(first_byte_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_response_byte_total_histogram = source.first_response_byte_total_histogram;
    if target.first_response_byte_total_histogram.is_empty() {
        target.first_response_byte_total_histogram = first_response_byte_total_histogram;
    } else {
        for (target_value, source_value) in target
            .first_response_byte_total_histogram
            .iter_mut()
            .zip(first_response_byte_total_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_token_histogram = source.first_token_histogram;
    if target.first_token_histogram.is_empty() {
        target.first_token_histogram = first_token_histogram;
    } else {
        for (target_value, source_value) in target
            .first_token_histogram
            .iter_mut()
            .zip(first_token_histogram)
        {
            *target_value += source_value;
        }
    }
}

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
            entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
                decode_approx_histogram(&row.first_byte_histogram)
            } else {
                let mut merged = entry.first_byte_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_byte_histogram),
                )?;
                merged
            };
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_response_byte_total_histogram =
                if entry.first_response_byte_total_histogram.is_empty() {
                    decode_approx_histogram(&row.first_response_byte_total_histogram)
                } else {
                    let mut merged = entry.first_response_byte_total_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_response_byte_total_histogram),
                    )?;
                    merged
                };
            entry.first_token_sample_count += row.first_token_sample_count;
            entry.first_token_sum_ms += row.first_token_sum_ms;
            entry.first_token_histogram = if entry.first_token_histogram.is_empty() {
                decode_approx_histogram(&row.first_token_histogram)
            } else {
                let mut merged = entry.first_token_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_token_histogram),
                )?;
                merged
            };
        }
    }
    Ok(())
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

pub(crate) fn overlay_runtime_timeseries_in_flight(
    state: &AppState,
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    db_runtime_records: &HashMap<(String, String), InvocationAggregateRecord>,
) -> Result<(), ApiError> {
    let mut runtime_overlay_row_count = 0_i64;
    let mut stale_db_runtime_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if let Some(expected_upstream_account_id) = upstream_account_id
            && record.upstream_account_id != Some(expected_upstream_account_id)
        {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred_utc < start_dt || occurred_utc >= end_dt {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if let Some(db_record) = db_runtime_records.get(&key) {
            subtract_stale_db_runtime_record(
                aggregates,
                db_record,
                bucket_seconds,
                reporting_tz,
                &mut stale_db_runtime_row_count,
            )?;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        entry.in_flight_count += 1;
        let runtime_phase = runtime_record_live_phase(&record);
        entry
            .in_flight_phase_counts
            .increment_phase_name(runtime_phase);
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.record_first_token_sample(runtime_record_first_token_ms(&record));
        add_optional_token_components(
            entry,
            record.total_tokens,
            record.input_tokens,
            record.output_tokens,
            record.cache_input_tokens,
            record.reasoning_tokens,
        );
        entry.total_cost += record.cost.unwrap_or_default();
        runtime_overlay_row_count += 1;
    }
    if runtime_overlay_row_count > 0 || stale_db_runtime_row_count > 0 {
        debug!(
            endpoint = "/api/timeseries",
            runtime_overlay_row_count,
            stale_db_runtime_row_count,
            upstream_account_id,
            "overlayed memory runtime in-flight records into timeseries"
        );
    }
    Ok(())
}

pub(crate) fn subtract_stale_db_runtime_record(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    record: &InvocationAggregateRecord,
    bucket_seconds: i64,
    reporting_tz: Tz,
    stale_db_runtime_row_count: &mut i64,
) -> Result<(), ApiError> {
    let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
        return Ok(());
    };
    let bucket_epoch =
        align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
    if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
        subtract_stale_in_flight_record_from_timeseries_aggregate(entry, record);
        *stale_db_runtime_row_count += 1;
    }
    Ok(())
}

pub(crate) fn collect_in_flight_aggregate_records(
    records: &[InvocationAggregateRecord],
) -> HashMap<(String, String), InvocationAggregateRecord> {
    records
        .iter()
        .filter(|record| prompt_shared::invocation_status_is_in_flight(record.status.as_deref()))
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record.clone(),
            )
        })
        .collect()
}

pub(crate) fn timeseries_point_from_aggregate(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    agg: &BucketAggregate,
) -> TimeseriesPoint {
    let has_calls = agg
        .total_count
        .max(agg.success_count + agg.failure_count + agg.in_flight_count.max(0))
        > 0;
    let token_components_complete = agg.total_tokens <= 0
        || (agg.token_components_observed
            && agg.token_component_incomplete_count == 0
            && agg.input_tokens.checked_add(agg.output_tokens) == Some(agg.total_tokens));
    TimeseriesPoint {
        bucket_start: format_utc_iso(start),
        bucket_end: format_utc_iso(end),
        total_count: agg.total_count,
        success_count: agg.success_count,
        failure_count: agg.failure_count,
        in_flight_count: agg.in_flight_count,
        in_flight_phase_counts: agg.in_flight_phase_counts,
        total_tokens: agg.total_tokens,
        input_tokens: token_components_complete.then_some(agg.input_tokens),
        output_tokens: token_components_complete.then_some(agg.output_tokens),
        cache_input_tokens: token_components_complete.then_some(agg.cache_input_tokens),
        reasoning_tokens: token_components_complete.then_some(agg.reasoning_tokens),
        total_cost: agg.total_cost,
        non_success_cost: agg.non_success_cost,
        avg_total_ms: has_calls.then(|| agg.total_latency_avg_ms()).flatten(),
        total_latency_sample_count: if has_calls {
            agg.total_latency_sample_count
        } else {
            0
        },
        first_byte_sample_count: if has_calls {
            agg.first_byte_sample_count
        } else {
            0
        },
        first_byte_avg_ms: has_calls.then(|| agg.first_byte_avg_ms()).flatten(),
        first_byte_p95_ms: has_calls.then(|| agg.first_byte_p95_ms()).flatten(),
        first_response_byte_total_sample_count: if has_calls {
            agg.first_response_byte_total_sample_count
        } else {
            0
        },
        first_response_byte_total_avg_ms: has_calls
            .then(|| agg.first_response_byte_total_avg_ms())
            .flatten(),
        first_response_byte_total_p95_ms: has_calls
            .then(|| agg.first_response_byte_total_p95_ms())
            .flatten(),
        first_token_sample_count: if has_calls {
            agg.first_token_sample_count
        } else {
            0
        },
        first_token_avg_ms: has_calls.then(|| agg.first_token_avg_ms()).flatten(),
        first_token_p95_ms: has_calls.then(|| agg.first_token_p95_ms()).flatten(),
    }
}

pub(crate) fn build_materialized_timeseries_response(
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    snapshot_id: i64,
    bucket_selection: &TimeseriesBucketSelection,
    aggregates: &BTreeMap<i64, BucketAggregate>,
    runtime_overlay: &BTreeMap<i64, BucketAggregate>,
    reporting_tz: Tz,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let fill_start_epoch =
        align_reporting_bucket_epoch(start_dt.timestamp(), bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut points = Vec::new();
    let mut bucket_epoch = fill_start_epoch;
    while bucket_epoch < fill_end_epoch {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let point = match (
            aggregates.get(&bucket_epoch),
            runtime_overlay.get(&bucket_epoch),
        ) {
            (Some(base), Some(overlay)) => {
                let mut combined = base.clone();
                merge_timeseries_bucket_aggregate(&mut combined, overlay.clone());
                timeseries_point_from_aggregate(start, end, &combined)
            }
            (Some(base), None) => timeseries_point_from_aggregate(start, end, base),
            (None, Some(overlay)) => timeseries_point_from_aggregate(start, end, overlay),
            (None, None) => {
                timeseries_point_from_aggregate(start, end, &BucketAggregate::default())
            }
        };
        points.push(point);
        bucket_epoch = bucket_end_epoch;
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(end_dt),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket.clone(),
        available_buckets: bucket_selection.available_buckets.clone(),
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}

pub(crate) fn build_timeseries_response(
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    snapshot_id: i64,
    bucket_selection: TimeseriesBucketSelection,
    aggregates: BTreeMap<i64, BucketAggregate>,
    fill_start_epoch: i64,
    fill_end_epoch: i64,
    reporting_tz: Tz,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(timeseries_point_from_aggregate(start, end, &agg));
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(end_dt),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}
