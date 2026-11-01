use super::minute_projection::timeseries_projection_scope;
use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;

fn timeseries_aggregate_overflow(field: &str) -> ApiError {
    ApiError::unavailable(anyhow!(
        "timeseries aggregate exceeded its numeric budget while adding {field}"
    ))
}

fn checked_add_i64(target: &mut i64, value: i64, field: &str) -> Result<(), ApiError> {
    *target = target
        .checked_add(value)
        .ok_or_else(|| timeseries_aggregate_overflow(field))?;
    Ok(())
}

fn checked_add_f64(target: &mut f64, value: f64, field: &str) -> Result<(), ApiError> {
    let next = *target + value;
    if !next.is_finite() {
        return Err(timeseries_aggregate_overflow(field));
    }
    *target = next;
    Ok(())
}

fn require_recorded_sample(recorded: bool, field: &str) -> Result<(), ApiError> {
    recorded
        .then_some(())
        .ok_or_else(|| timeseries_aggregate_overflow(field))
}

fn checked_increment_phase_counts(
    counts: &mut InvocationPhaseCountsResponse,
    phase: Option<&str>,
) -> Result<(), ApiError> {
    match normalized_runtime_text(phase).as_str() {
        "requesting" => checked_add_i64(&mut counts.requesting, 1, "requesting count"),
        "responding" => checked_add_i64(&mut counts.responding, 1, "responding count"),
        _ => checked_add_i64(&mut counts.queued, 1, "queued count"),
    }
}

fn add_optional_token_components_checked(
    entry: &mut BucketAggregate,
    total_tokens: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
) -> Result<(), ApiError> {
    checked_add_i64(
        &mut entry.total_tokens,
        total_tokens.unwrap_or_default(),
        "total tokens",
    )?;
    checked_add_i64(
        &mut entry.input_tokens,
        input_tokens.unwrap_or_default(),
        "input tokens",
    )?;
    checked_add_i64(
        &mut entry.output_tokens,
        output_tokens.unwrap_or_default(),
        "output tokens",
    )?;
    checked_add_i64(
        &mut entry.cache_input_tokens,
        cache_input_tokens.unwrap_or_default(),
        "cache input tokens",
    )?;
    checked_add_i64(
        &mut entry.reasoning_tokens,
        reasoning_tokens.unwrap_or_default(),
        "reasoning tokens",
    )?;
    if total_tokens.unwrap_or_default() > 0 {
        entry.token_components_observed = true;
        if input_tokens.is_none()
            || output_tokens.is_none()
            || cache_input_tokens.is_none()
            || reasoning_tokens.is_none()
        {
            checked_add_i64(
                &mut entry.token_component_incomplete_count,
                1,
                "incomplete token component count",
            )?;
        }
    }
    Ok(())
}

fn validate_timeseries_token_fields(
    total_tokens: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
) -> Result<(), ApiError> {
    for (field, value) in [
        ("total tokens", total_tokens),
        ("input tokens", input_tokens),
        ("output tokens", output_tokens),
        ("cache input tokens", cache_input_tokens),
        ("reasoning tokens", reasoning_tokens),
    ] {
        if value.is_some_and(|value| value < 0) {
            return Err(ApiError::unavailable(anyhow!(
                "timeseries projection contains invalid {field}"
            )));
        }
    }
    if let (Some(input_tokens), Some(cache_input_tokens)) = (input_tokens, cache_input_tokens)
        && cache_input_tokens > input_tokens
    {
        return Err(ApiError::unavailable(anyhow!(
            "timeseries projection contains cache input tokens exceeding input tokens"
        )));
    }
    Ok(())
}

fn validate_runtime_timeseries_numeric_fields(record: &ApiInvocation) -> Result<(), ApiError> {
    validate_timeseries_token_fields(
        record.total_tokens,
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
    )?;
    if record
        .cost
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(ApiError::unavailable(anyhow!(
            "timeseries runtime overlay contains invalid cost"
        )));
    }
    for (field, value) in [
        ("total latency", record.t_total_ms),
        ("request read latency", record.t_req_read_ms),
        ("request parse latency", record.t_req_parse_ms),
        ("upstream connect latency", record.t_upstream_connect_ms),
        ("upstream TTFB latency", record.t_upstream_ttfb_ms),
        ("upstream stream latency", record.t_upstream_stream_ms),
        ("first token latency", record.first_token_ms),
        ("response parse latency", record.t_resp_parse_ms),
        ("persist latency", record.t_persist_ms),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(ApiError::unavailable(anyhow!(
                "timeseries runtime overlay contains invalid {field}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn fold_minute_projection_aggregates(
    minute_aggregates: BTreeMap<i64, BucketAggregate>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
    let mut buckets = BTreeMap::new();
    for (minute_epoch, aggregate) in minute_aggregates {
        let bucket_epoch =
            align_reporting_bucket_epoch(minute_epoch, bucket_seconds, reporting_tz)?;
        merge_timeseries_bucket_aggregate(buckets.entry(bucket_epoch).or_default(), aggregate)?;
    }
    Ok(buckets)
}

pub(crate) fn add_timeseries_terminal_delta_to_aggregate(
    entry: &mut BucketAggregate,
    delta: &TimeseriesTerminalDelta,
) -> Result<(), ApiError> {
    let mut candidate = entry.clone();
    add_timeseries_terminal_delta_to_aggregate_inner(&mut candidate, delta)?;
    *entry = candidate;
    Ok(())
}

fn add_timeseries_terminal_delta_to_aggregate_inner(
    entry: &mut BucketAggregate,
    delta: &TimeseriesTerminalDelta,
) -> Result<(), ApiError> {
    validate_timeseries_token_fields(
        delta.total_tokens,
        delta.input_tokens,
        delta.output_tokens,
        delta.cache_input_tokens,
        delta.reasoning_tokens,
    )?;
    if delta
        .cost
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(ApiError::unavailable(anyhow!(
            "timeseries projection contains invalid cost"
        )));
    }
    checked_add_i64(&mut entry.total_count, 1, "total count")?;
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
        checked_add_i64(&mut entry.success_count, 1, "success count")?;
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        delta.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        checked_add_i64(&mut entry.failure_count, 1, "failure count")?;
    }
    let latency_status = is_success_like
        .then_some("success")
        .or(delta.status.as_deref());
    require_recorded_sample(
        entry.try_record_total_latency_sample(delta.t_total_ms),
        "total latency",
    )?;
    require_recorded_sample(
        entry.try_record_ttfb_sample(latency_status, delta.t_upstream_ttfb_ms),
        "first-byte latency",
    )?;
    require_recorded_sample(
        entry.try_record_first_response_byte_total_sample(
            delta.t_req_read_ms,
            delta.t_req_parse_ms,
            delta.t_upstream_connect_ms,
            delta.t_upstream_ttfb_ms,
        ),
        "first-response-byte latency",
    )?;
    require_recorded_sample(
        entry.try_record_first_token_sample(delta.first_token_ms),
        "first-token latency",
    )?;
    add_optional_token_components_checked(
        entry,
        delta.total_tokens,
        delta.input_tokens,
        delta.output_tokens,
        delta.cache_input_tokens,
        delta.reasoning_tokens,
    )?;
    let cost = delta.cost.unwrap_or_default();
    checked_add_f64(&mut entry.total_cost, cost, "total cost")?;
    if invocation_counts_toward_non_success_usage(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    ) {
        checked_add_f64(&mut entry.non_success_cost, cost, "non-success cost")?;
    }
    Ok(())
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
    let mut applied = 0_usize;
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
        )?;
        applied = applied
            .checked_add(1)
            .ok_or_else(|| timeseries_aggregate_overflow("pending delta count"))?;
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
        )?;
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
        validate_runtime_timeseries_numeric_fields(record)?;
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        checked_add_i64(&mut entry.total_count, 1, "total count")?;
        checked_add_i64(&mut entry.in_flight_count, 1, "in-flight count")?;
        checked_increment_phase_counts(
            &mut entry.in_flight_phase_counts,
            runtime_record_live_phase(record),
        )?;
        require_recorded_sample(
            entry.try_record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms),
            "first-byte latency",
        )?;
        require_recorded_sample(
            entry.try_record_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            ),
            "first-response-byte latency",
        )?;
        require_recorded_sample(
            entry.try_record_first_token_sample(runtime_record_first_token_ms(record)),
            "first-token latency",
        )?;
        add_optional_token_components_checked(
            entry,
            record.total_tokens,
            record.input_tokens,
            record.output_tokens,
            record.cache_input_tokens,
            record.reasoning_tokens,
        )?;
        checked_add_f64(
            &mut entry.total_cost,
            record.cost.unwrap_or_default(),
            "total cost",
        )?;
    }
    Ok(())
}

fn validate_bucket_aggregate(aggregate: &BucketAggregate) -> Result<(), ApiError> {
    for (field, value) in [
        ("total count", aggregate.total_count),
        ("success count", aggregate.success_count),
        ("failure count", aggregate.failure_count),
        ("in-flight count", aggregate.in_flight_count),
        ("queued count", aggregate.in_flight_phase_counts.queued),
        (
            "requesting count",
            aggregate.in_flight_phase_counts.requesting,
        ),
        (
            "responding count",
            aggregate.in_flight_phase_counts.responding,
        ),
        ("total tokens", aggregate.total_tokens),
        ("input tokens", aggregate.input_tokens),
        ("output tokens", aggregate.output_tokens),
        ("cache input tokens", aggregate.cache_input_tokens),
        ("reasoning tokens", aggregate.reasoning_tokens),
        (
            "incomplete token component count",
            aggregate.token_component_incomplete_count,
        ),
        (
            "total latency sample count",
            aggregate.total_latency_sample_count,
        ),
        ("first-byte sample count", aggregate.first_byte_sample_count),
        (
            "first-response-byte sample count",
            aggregate.first_response_byte_total_sample_count,
        ),
        (
            "first-token sample count",
            aggregate.first_token_sample_count,
        ),
    ] {
        if value < 0 {
            return Err(timeseries_aggregate_overflow(field));
        }
    }
    for (field, value) in [
        ("total cost", aggregate.total_cost),
        ("non-success cost", aggregate.non_success_cost),
        ("total latency sum", aggregate.total_latency_sum_ms),
        ("first-byte latency sum", aggregate.first_byte_ttfb_sum_ms),
        (
            "first-response-byte latency sum",
            aggregate.first_response_byte_total_sum_ms,
        ),
        ("first-token latency sum", aggregate.first_token_sum_ms),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(timeseries_aggregate_overflow(field));
        }
    }
    for (field, histogram) in [
        ("first-byte histogram", &aggregate.first_byte_histogram),
        (
            "first-response-byte histogram",
            &aggregate.first_response_byte_total_histogram,
        ),
        ("first-token histogram", &aggregate.first_token_histogram),
    ] {
        if histogram.iter().any(|value| *value < 0) {
            return Err(timeseries_aggregate_overflow(field));
        }
    }
    Ok(())
}

fn merge_histogram_counts(
    target: &mut ApproxHistogramCounts,
    source: ApproxHistogramCounts,
    field: &str,
) -> Result<(), ApiError> {
    if target.is_empty() {
        *target = source;
        return Ok(());
    }
    if target.len() != source.len() {
        return Err(timeseries_aggregate_overflow(field));
    }
    for (target_value, source_value) in target.iter_mut().zip(source) {
        checked_add_i64(target_value, source_value, field)?;
    }
    Ok(())
}

pub(crate) fn merge_timeseries_bucket_aggregate(
    target: &mut BucketAggregate,
    source: BucketAggregate,
) -> Result<(), ApiError> {
    let mut candidate = target.clone();
    merge_timeseries_bucket_aggregate_inner(&mut candidate, source)?;
    *target = candidate;
    Ok(())
}

fn merge_timeseries_bucket_aggregate_inner(
    target: &mut BucketAggregate,
    source: BucketAggregate,
) -> Result<(), ApiError> {
    validate_bucket_aggregate(target)?;
    validate_bucket_aggregate(&source)?;
    macro_rules! add_i64 {
        ($field:ident, $label:literal) => {
            checked_add_i64(&mut target.$field, source.$field, $label)?;
        };
    }
    macro_rules! add_f64 {
        ($field:ident, $label:literal) => {
            checked_add_f64(&mut target.$field, source.$field, $label)?;
        };
    }
    add_i64!(total_count, "total count");
    add_i64!(success_count, "success count");
    add_i64!(failure_count, "failure count");
    add_i64!(in_flight_count, "in-flight count");
    checked_add_i64(
        &mut target.in_flight_phase_counts.queued,
        source.in_flight_phase_counts.queued,
        "queued count",
    )?;
    checked_add_i64(
        &mut target.in_flight_phase_counts.requesting,
        source.in_flight_phase_counts.requesting,
        "requesting count",
    )?;
    checked_add_i64(
        &mut target.in_flight_phase_counts.responding,
        source.in_flight_phase_counts.responding,
        "responding count",
    )?;
    add_i64!(total_tokens, "total tokens");
    add_i64!(input_tokens, "input tokens");
    add_i64!(output_tokens, "output tokens");
    add_i64!(cache_input_tokens, "cache input tokens");
    add_i64!(reasoning_tokens, "reasoning tokens");
    target.token_components_observed |= source.token_components_observed;
    add_i64!(
        token_component_incomplete_count,
        "incomplete token component count"
    );
    add_f64!(total_cost, "total cost");
    add_f64!(non_success_cost, "non-success cost");
    add_f64!(total_latency_sum_ms, "total latency sum");
    add_i64!(total_latency_sample_count, "total latency sample count");
    target
        .total_latency_values
        .extend(source.total_latency_values);
    add_f64!(first_byte_ttfb_sum_ms, "first-byte latency sum");
    add_i64!(first_byte_sample_count, "first-byte sample count");
    add_f64!(
        first_response_byte_total_sum_ms,
        "first-response-byte latency sum"
    );
    add_i64!(
        first_response_byte_total_sample_count,
        "first-response-byte sample count"
    );
    add_f64!(first_token_sum_ms, "first-token latency sum");
    add_i64!(first_token_sample_count, "first-token sample count");
    target
        .first_byte_ttfb_values
        .extend(source.first_byte_ttfb_values);
    target
        .first_response_byte_total_values
        .extend(source.first_response_byte_total_values);
    target.first_token_values.extend(source.first_token_values);
    merge_histogram_counts(
        &mut target.first_byte_histogram,
        source.first_byte_histogram,
        "first-byte histogram",
    )?;
    merge_histogram_counts(
        &mut target.first_response_byte_total_histogram,
        source.first_response_byte_total_histogram,
        "first-response-byte histogram",
    )?;
    merge_histogram_counts(
        &mut target.first_token_histogram,
        source.first_token_histogram,
        "first-token histogram",
    )?;
    Ok(())
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
            merge_timeseries_bucket_aggregate(
                entry,
                BucketAggregate {
                    total_count: row.total_count,
                    success_count: row.success_count,
                    failure_count: row.failure_count,
                    in_flight_count: row.in_flight_count,
                    total_tokens: row.total_tokens,
                    input_tokens: row.input_tokens,
                    output_tokens: row.output_tokens,
                    cache_input_tokens: row.cache_input_tokens,
                    reasoning_tokens: row.reasoning_tokens,
                    token_components_observed: row.total_tokens > 0,
                    total_cost: row.total_cost,
                    non_success_cost: row.non_success_cost,
                    total_latency_sample_count: row.total_latency_sample_count,
                    total_latency_sum_ms: row.total_latency_sum_ms,
                    first_byte_sample_count: row.first_byte_sample_count,
                    first_byte_ttfb_sum_ms: row.first_byte_sum_ms,
                    first_byte_histogram: decode_approx_histogram(&row.first_byte_histogram),
                    first_response_byte_total_sample_count: row
                        .first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms: row.first_response_byte_total_sum_ms,
                    first_response_byte_total_histogram: decode_approx_histogram(
                        &row.first_response_byte_total_histogram,
                    ),
                    first_token_sample_count: row.first_token_sample_count,
                    first_token_sum_ms: row.first_token_sum_ms,
                    first_token_histogram: decode_approx_histogram(&row.first_token_histogram),
                    ..BucketAggregate::default()
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn add_invocation_rollup_rows_to_timeseries_aggregates(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    rows: Vec<InvocationHourlyRollupRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for row in rows {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            merge_timeseries_bucket_aggregate(
                entry,
                BucketAggregate {
                    total_count: row.total_count,
                    success_count: row.success_count,
                    failure_count: row.failure_count,
                    total_tokens: row.total_tokens,
                    input_tokens: row.input_tokens,
                    output_tokens: row.output_tokens,
                    cache_input_tokens: row.cache_input_tokens,
                    reasoning_tokens: row.reasoning_tokens,
                    token_components_observed: row.total_tokens > 0,
                    total_cost: row.total_cost,
                    non_success_cost: row.non_success_cost,
                    total_latency_sample_count: row.total_latency_sample_count,
                    total_latency_sum_ms: row.total_latency_sum_ms,
                    first_byte_sample_count: row.first_byte_sample_count,
                    first_byte_ttfb_sum_ms: row.first_byte_sum_ms,
                    first_byte_histogram: decode_approx_histogram(&row.first_byte_histogram),
                    first_response_byte_total_sample_count: row
                        .first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms: row.first_response_byte_total_sum_ms,
                    first_response_byte_total_histogram: decode_approx_histogram(
                        &row.first_response_byte_total_histogram,
                    ),
                    first_token_sample_count: row.first_token_sample_count,
                    first_token_sum_ms: row.first_token_sum_ms,
                    first_token_histogram: decode_approx_histogram(&row.first_token_histogram),
                    ..BucketAggregate::default()
                },
            )?;
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
            add_exact_record_to_timeseries_aggregate(entry, &record)?;
        }
    }
    Ok(())
}

pub(crate) fn add_exact_record_to_timeseries_aggregate(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) -> Result<(), ApiError> {
    let mut candidate = entry.clone();
    add_exact_record_to_timeseries_aggregate_inner(&mut candidate, record)?;
    *entry = candidate;
    Ok(())
}

fn add_exact_record_to_timeseries_aggregate_inner(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) -> Result<(), ApiError> {
    validate_timeseries_token_fields(
        record.total_tokens,
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
    )?;
    if record
        .cost
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(ApiError::unavailable(anyhow!(
            "timeseries projection contains invalid cost"
        )));
    }
    checked_add_i64(&mut entry.total_count, 1, "total count")?;
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
        checked_add_i64(&mut entry.success_count, 1, "success count")?;
    } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        checked_add_i64(&mut entry.in_flight_count, 1, "in-flight count")?;
        checked_increment_phase_counts(
            &mut entry.in_flight_phase_counts,
            record.live_phase.as_deref(),
        )?;
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        record.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        checked_add_i64(&mut entry.failure_count, 1, "failure count")?;
    }
    let latency_status = if is_success_like {
        Some("success")
    } else {
        record.status.as_deref()
    };
    if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        require_recorded_sample(
            entry.try_record_total_latency_sample(record.t_total_ms),
            "total latency",
        )?;
    }
    require_recorded_sample(
        entry.try_record_ttfb_sample(latency_status, record.t_upstream_ttfb_ms),
        "first-byte latency",
    )?;
    require_recorded_sample(
        entry.try_record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        ),
        "first-response-byte latency",
    )?;
    require_recorded_sample(
        entry.try_record_first_token_sample(record.first_token_ms),
        "first-token latency",
    )?;
    add_optional_token_components_checked(
        entry,
        record.total_tokens,
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
    )?;
    let cost = record.cost.unwrap_or_default();
    checked_add_f64(&mut entry.total_cost, cost, "total cost")?;
    if invocation_counts_toward_non_success_usage(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    ) {
        checked_add_f64(&mut entry.non_success_cost, cost, "non-success cost")?;
    }
    Ok(())
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
        validate_runtime_timeseries_numeric_fields(&record)?;
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
        checked_add_i64(&mut entry.total_count, 1, "total count")?;
        checked_add_i64(&mut entry.in_flight_count, 1, "in-flight count")?;
        let runtime_phase = runtime_record_live_phase(&record);
        checked_increment_phase_counts(&mut entry.in_flight_phase_counts, runtime_phase)?;
        require_recorded_sample(
            entry.try_record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms),
            "first-byte latency",
        )?;
        require_recorded_sample(
            entry.try_record_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            ),
            "first-response-byte latency",
        )?;
        require_recorded_sample(
            entry.try_record_first_token_sample(runtime_record_first_token_ms(&record)),
            "first-token latency",
        )?;
        add_optional_token_components_checked(
            entry,
            record.total_tokens,
            record.input_tokens,
            record.output_tokens,
            record.cache_input_tokens,
            record.reasoning_tokens,
        )?;
        checked_add_f64(
            &mut entry.total_cost,
            record.cost.unwrap_or_default(),
            "total cost",
        )?;
        checked_add_i64(
            &mut runtime_overlay_row_count,
            1,
            "runtime overlay row count",
        )?;
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
        *stale_db_runtime_row_count = stale_db_runtime_row_count
            .checked_add(1)
            .ok_or_else(|| timeseries_aggregate_overflow("stale runtime row count"))?;
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
                merge_timeseries_bucket_aggregate(&mut combined, overlay.clone())?;
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
