use super::*;
use super::prompt_cache_and_timeseries_shared as prompt_shared;

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let bucket_selection = resolve_timeseries_bucket_selection(
        &params,
        &range_window,
        state.config.invocation_max_days,
    )?;
    if let Some(upstream_account_id) = params.upstream_account_id {
        return fetch_timeseries_for_account(
            state,
            reporting_tz,
            source_scope,
            range_window,
            bucket_selection,
            upstream_account_id,
        )
        .await;
    }
    let bucket_seconds = bucket_selection.bucket_seconds;

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned {
            if needs_historical_rollups {
                return Err(ApiError::bad_request(anyhow!(
                    "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
                )));
            }
        } else {
            return fetch_timeseries_from_hourly_rollups(
                state,
                params,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
            )
            .await;
        }
    }

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);

    let records = query_invocation_aggregate_records_from_live_range(
        &state.pool,
        ExactUtcRange {
            start: start_dt,
            end: end_dt,
        },
        source_scope,
        None,
        Some(snapshot_id),
    )
    .await?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    let start_epoch = start_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        let bucket_epoch = align_reporting_bucket_epoch(epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        let classification = resolve_failure_classification(
            record.status.as_deref(),
            record.error_message.as_deref(),
            record.failure_kind.as_deref(),
            record.failure_class.as_deref(),
            record.is_actionable,
        );
        let is_success_like = prompt_shared::invocation_status_is_success_like(
            record.status.as_deref(),
            record.error_message.as_deref(),
        ) && classification.failure_class == FailureClass::None;
        if is_success_like {
            entry.success_count += 1;
        } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            entry.in_flight_count += 1;
        } else if prompt_shared::invocation_status_counts_toward_terminal_totals(record.status.as_deref())
            && classification.failure_class != FailureClass::None
        {
            entry.failure_count += 1;
        }
        let latency_status = if is_success_like {
            Some("success")
        } else {
            record.status.as_deref()
        };
        entry.record_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.cache_input_tokens += record.cache_input_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        query_crs_deltas(
            &state.pool,
            relay,
            start_epoch,
            crate::stats::exclusive_epoch_upper_bound(end_dt),
        )
        .await?
    } else {
        Vec::new()
    };

    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += delta.total_count;
        entry.success_count += delta.success_count;
        entry.failure_count += delta.failure_count;
        entry.total_tokens += delta.total_tokens;
        entry.total_cost += delta.total_cost;
    }

    // Fill every bucket that intersects the requested range using reporting-timezone
    // boundaries rather than fixed UTC-duration strides. This keeps DST transition
    // days aligned to local clock buckets.
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
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

    let response = TimeseriesResponse {
        range_start: start_str_iso,
        range_end: format_utc_iso(end_dt),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    };

    Ok(Json(response))
}

async fn fetch_timeseries_for_account(
    state: Arc<AppState>,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
    upstream_account_id: i64,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_dt = range_window.start;
    let end_dt = range_window.end;
    let start_epoch = start_dt.timestamp();
    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned && needs_historical_rollups {
            return Err(ApiError::bad_request(anyhow!(
                "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
            )));
        }
    }

    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let range_plan = if bucket_seconds >= 3_600 {
        build_hourly_rollup_exact_range_plan(
            start_dt,
            end_dt,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?
    } else {
        let rollup_bucket_seconds = 60;
        let range_start_epoch = if start_dt.timestamp().rem_euclid(rollup_bucket_seconds) == 0 {
            start_dt.timestamp()
        } else {
            align_bucket_epoch(
                start_dt
                    .timestamp()
                    .saturating_add(rollup_bucket_seconds.saturating_sub(1)),
                rollup_bucket_seconds,
                0,
            )
        };
        let range_end_epoch = align_bucket_epoch(end_dt.timestamp(), rollup_bucket_seconds, 0);
        let mut live_exact_ranges = Vec::new();
        let first_full_bucket_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid first full bucket start epoch"))?;
        let last_full_bucket_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid last full bucket end epoch"))?;
        push_exact_range(&mut live_exact_ranges, start_dt, end_dt.min(first_full_bucket_start))?;
        push_exact_range(&mut live_exact_ranges, start_dt.max(last_full_bucket_end), end_dt)?;
        HourlyRollupExactRangePlan {
            full_hour_range: (range_start_epoch < range_end_epoch)
                .then_some((range_start_epoch, range_end_epoch)),
            live_exact_ranges,
        }
    };
    let mut tx = state.pool.begin().await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let table_name = if bucket_seconds >= 3_600 {
            "upstream_account_stats_hourly"
        } else {
            "upstream_account_stats_minute"
        };
        let rows = query_upstream_account_stats_rollup_range_tx(
            tx.as_mut(),
            table_name,
            range_start_epoch,
            range_end_epoch,
            source_scope,
            upstream_account_id,
        )
        .await?;
        add_rollup_rows_to_timeseries_aggregates(
            &mut aggregates,
            rows,
            bucket_seconds,
            reporting_tz,
        )?;
    }

    let boundary_snapshot_id = rollup_live_cursor.min(snapshot_id);
    if !range_plan.live_exact_ranges.is_empty() && boundary_snapshot_id > 0 {
        let exact_records = query_invocation_exact_records_for_account_tx(
            tx.as_mut(),
            &range_plan,
            source_scope,
            boundary_snapshot_id,
            upstream_account_id,
        )
        .await?;
        add_exact_records_to_timeseries_aggregates(
            &mut aggregates,
            exact_records,
            bucket_seconds,
            reporting_tz,
        )?;
    }

    let mut archive_overlap_ids = HashSet::new();
    if rollup_live_cursor < snapshot_id {
        let tail_range_plan = HourlyRollupExactRangePlan {
            full_hour_range: None,
            live_exact_ranges: exact_utc_range(start_dt, end_dt)?.into_iter().collect(),
        };
        let tail_records = query_invocation_exact_records_tx_for_account(
            tx.as_mut(),
            &tail_range_plan,
            source_scope,
            snapshot_id,
            upstream_account_id,
            rollup_live_cursor,
        )
        .await?;
        archive_overlap_ids.extend(tail_records.iter().map(|record| record.id));
        add_exact_records_to_timeseries_aggregates(
            &mut aggregates,
            tail_records,
            bucket_seconds,
            reporting_tz,
        )?;
    }
    if bucket_seconds >= 3_600
        && let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range
    {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid account archived timeseries start epoch")))?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid account archived timeseries end epoch")))?;
        let archived_rows = crate::stats::query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
            &state.pool,
            source_scope,
            Some((archived_start, archived_end)),
            Some(&archive_overlap_ids),
            upstream_account_id,
        )
        .await?;
        for row in archived_rows {
            let bucket_epoch =
                align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
            if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
                entry.total_count += row.request_count;
                entry.success_count += row.success_count;
                entry.failure_count += row.failure_count;
                entry.total_tokens += row.total_tokens;
                entry.cache_input_tokens += row.cache_input_tokens;
                entry.total_cost += row.total_cost;
            }
        }
    }
    drop(tx);
    build_timeseries_response(
        start_dt,
        end_dt,
        bucket_seconds,
        snapshot_id,
        bucket_selection,
        aggregates,
        fill_start_epoch,
        fill_end_epoch,
        reporting_tz,
    )
}

fn add_rollup_rows_to_timeseries_aggregates(
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
            entry.cache_input_tokens += row.cache_input_tokens;
            entry.total_cost += row.total_cost;
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
            entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
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
        }
    }
    Ok(())
}

fn add_exact_records_to_timeseries_aggregates(
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
            entry.total_count += 1;
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            let is_success_like = prompt_shared::invocation_status_is_success_like(
                record.status.as_deref(),
                record.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
            if is_success_like {
                entry.success_count += 1;
            } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
                entry.in_flight_count += 1;
            } else if prompt_shared::invocation_status_counts_toward_terminal_totals(record.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                entry.failure_count += 1;
            }
            let latency_status = if is_success_like {
                Some("success")
            } else {
                record.status.as_deref()
            };
            entry.record_exact_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
            entry.record_exact_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
            entry.total_tokens += record.total_tokens.unwrap_or_default();
            entry.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
            entry.total_cost += record.cost.unwrap_or_default();
        }
    }
    Ok(())
}

fn timeseries_point_from_aggregate(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    agg: &BucketAggregate,
) -> TimeseriesPoint {
    let has_calls = agg
        .total_count
        .max(agg.success_count + agg.failure_count + agg.in_flight_count.max(0))
        > 0;
    TimeseriesPoint {
        bucket_start: format_utc_iso(start),
        bucket_end: format_utc_iso(end),
        total_count: agg.total_count,
        success_count: agg.success_count,
        failure_count: agg.failure_count,
        in_flight_count: agg.in_flight_count,
        total_tokens: agg.total_tokens,
        cache_input_tokens: agg.cache_input_tokens,
        total_cost: agg.total_cost,
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
    }
}

fn build_timeseries_response(
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

#[cfg(test)]
pub(crate) async fn fetch_parallel_work_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    load_parallel_work_stats_response(&state, params).await.map(Json)
}

pub(crate) async fn fetch_parallel_work_stats_cached(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Response, ApiError> {
    let response = load_parallel_work_stats_response(&state, params).await?;
    let body = serde_json::to_vec(&response)
        .map_err(|err| ApiError::from(anyhow!("failed to serialize parallel-work stats: {err}")))?;
    let etag = parallel_work_stats_etag(&body);
    let mut response = if request_etag_matches(&headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response()
    };
    let etag_value = HeaderValue::from_str(&etag)
        .map_err(|err| ApiError::from(anyhow!("invalid parallel-work etag: {err}")))?;
    response.headers_mut().insert(axum::http::header::ETAG, etag_value);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    Ok(response)
}

fn parallel_work_stats_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!("\"parallel-work-{digest:x}\"")
}

fn request_etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        })
        .unwrap_or(false)
}

async fn load_parallel_work_stats_response(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkStatsResponse, ApiError> {
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let requested_range_window = resolve_range_window(&params.range, requested_reporting_tz)?;
    let bucket_params = TimeseriesQuery {
        range: params.range.clone(),
        bucket: params.bucket.clone(),
        settlement_hour: None,
        time_zone: params.time_zone.clone(),
        upstream_account_id: None,
    };
    let bucket_selection = resolve_timeseries_bucket_selection(
        &bucket_params,
        &requested_range_window,
        state.config.invocation_max_days,
    )?;
    let bucket_seconds = bucket_selection.bucket_seconds;
    let (reporting_tz, time_zone_fallback) = if bucket_seconds >= 3_600 {
        resolve_parallel_work_rollup_reporting_tz(
            requested_reporting_tz,
            &requested_range_window,
        )
    } else {
        (requested_reporting_tz, false)
    };
    let range_window = if time_zone_fallback {
        resolve_range_window(&params.range, reporting_tz)?
    } else {
        requested_range_window
    };
    let fill_start_epoch =
        align_reporting_bucket_epoch(range_window.start.timestamp(), bucket_seconds, reporting_tz)?;
    let fill_end_epoch =
        resolve_timeseries_fill_end_epoch(range_window.end, bucket_seconds, reporting_tz)?;
    let fill_start = Utc
        .timestamp_opt(fill_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work fill start epoch")))?;
    let fill_end = Utc
        .timestamp_opt(fill_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work fill end epoch")))?;

    let current_counts = if bucket_seconds >= 3_600 {
        let leading_full_bucket_epoch = if fill_start < range_window.start {
            next_reporting_bucket_epoch(fill_start_epoch, bucket_seconds, reporting_tz)?
        } else {
            fill_start_epoch
        };
        let leading_full_bucket_start = Utc
            .timestamp_opt(leading_full_bucket_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work rollup start epoch")))?;
        let mut bucket_keys = query_parallel_work_bucket_key_sets_from_hourly_rollups(
            &state.pool,
            leading_full_bucket_start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
        )
        .await?;
        let mut tx = state.pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
        drop(tx);
        if fill_start < range_window.start && range_window.start < leading_full_bucket_start {
            let leading_exact_end = leading_full_bucket_start.min(range_window.end);
            let leading_bucket_keys = query_parallel_work_exact_key_sets(
                &state.pool,
                range_window.start,
                leading_exact_end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                None,
                Some(snapshot_id),
            )
            .await?;
            for (bucket_epoch, keys) in leading_bucket_keys {
                bucket_keys.entry(bucket_epoch).or_default().extend(keys);
            }
        }
        let tail_bucket_keys = query_parallel_work_exact_key_sets(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            Some(rollup_live_cursor),
            Some(snapshot_id),
        )
        .await?;
        for (bucket_epoch, keys) in tail_bucket_keys {
            bucket_keys.entry(bucket_epoch).or_default().extend(keys);
        }
        parallel_work_counts_from_key_sets(bucket_keys)
    } else {
        parallel_work_counts_from_key_sets(
            query_parallel_work_exact_key_sets(
                &state.pool,
                range_window.start,
                range_window.end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                None,
                None,
            )
            .await?,
        )
    };
    let conversations = if range_window.duration <= ChronoDuration::hours(24) {
        query_parallel_work_conversation_spans(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
        )
        .await?
    } else {
        Vec::new()
    };

    let current = build_parallel_work_window_response(
        fill_start,
        fill_end,
        bucket_seconds,
        reporting_tz,
        &current_counts,
        reporting_tz,
        time_zone_fallback,
        conversations,
    )?;

    Ok(ParallelWorkStatsResponse {
        current: current.clone(),
        minute7d: current.clone(),
        hour30d: current.clone(),
        day_all: current,
    })
}

async fn query_parallel_work_conversation_spans(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
) -> Result<Vec<ParallelWorkConversation>> {
    let mut query = QueryBuilder::new("SELECT ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS conversation_id, MIN(occurred_at) AS first_occurred_at, MAX(occurred_at) AS last_occurred_at, COUNT(*) AS request_count FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(range_end))
        .push(" AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" IS NOT NULL AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" != ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" GROUP BY ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" ORDER BY last_occurred_at DESC, request_count DESC LIMIT 80");

    let rows = query
        .build_query_as::<ParallelWorkConversationSpanRow>()
        .fetch_all(pool)
        .await?;
    let mut conversations = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(first_occurred_at) = parse_to_utc_datetime(&row.first_occurred_at) else {
            continue;
        };
        let Some(last_occurred_at) = parse_to_utc_datetime(&row.last_occurred_at) else {
            continue;
        };
        let start_epoch =
            align_reporting_bucket_epoch(first_occurred_at.timestamp(), bucket_seconds, reporting_tz)?;
        let end_bucket_epoch =
            align_reporting_bucket_epoch(last_occurred_at.timestamp(), bucket_seconds, reporting_tz)?;
        let end_epoch = next_reporting_bucket_epoch(end_bucket_epoch, bucket_seconds, reporting_tz)?;
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid parallel-work conversation start epoch"))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid parallel-work conversation end epoch"))?;
        conversations.push(ParallelWorkConversation {
            conversation_id: row.conversation_id,
            start: format_utc_iso(start),
            end: format_utc_iso(end),
            request_count: row.request_count,
        });
    }

    Ok(conversations)
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_epoch = range_window.start.timestamp();
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch =
        resolve_timeseries_fill_end_epoch(range_window.end, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let (snapshot_id, hourly_rows, exact_records, archive_overlap_ids) =
        if range_plan.full_hour_range.is_some() {
        let mut tx = state.pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
        let (hourly_cursor, hourly_end_epoch) = range_plan
            .full_hour_range
            .expect("full_hour_range is present when hourly rollups are enabled");
        let hourly_rows = query_invocation_hourly_rollup_range_tx(
            tx.as_mut(),
            hourly_cursor,
            hourly_end_epoch,
            source_scope,
        )
        .await?;
        let mut exact_records =
            query_invocation_exact_records_tx(tx.as_mut(), &range_plan, source_scope, snapshot_id)
                .await?;
        let tail_records = query_invocation_full_hour_tail_records_tx(
            tx.as_mut(),
            &range_plan,
            source_scope,
            rollup_live_cursor,
            snapshot_id,
        )
        .await?;
        let archive_overlap_ids = tail_records
            .iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        exact_records.extend(tail_records);
        (snapshot_id, hourly_rows, exact_records, archive_overlap_ids)
    } else {
        let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
        let exact_records =
            query_invocation_exact_records(&state.pool, &range_plan, source_scope, snapshot_id)
                .await?;
        (snapshot_id, Vec::new(), exact_records, HashSet::new())
    };
    let archived_hourly_rows = if let Some((range_start_epoch, range_end_epoch)) =
        range_plan.full_hour_range
    {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries start epoch")))?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries end epoch")))?;
        crate::stats::query_unmaterialized_invocation_archive_hourly_rollup_deltas(
            &state.pool,
            source_scope,
            Some((archived_start, archived_end)),
            Some(&archive_overlap_ids),
        )
        .await?
    } else {
        Vec::new()
    };

    for row in hourly_rows.into_iter().chain(archived_hourly_rows) {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += row.total_count;
        entry.success_count += row.success_count;
        entry.failure_count += row.failure_count;
        entry.total_tokens += row.total_tokens;
        entry.cache_input_tokens += row.cache_input_tokens;
        entry.total_cost += row.total_cost;
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
        entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
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
    }
    for record in exact_records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += 1;
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            let is_success_like = prompt_shared::invocation_status_is_success_like(
                record.status.as_deref(),
                record.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
            if is_success_like {
                entry.success_count += 1;
            } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
                entry.in_flight_count += 1;
            } else if prompt_shared::invocation_status_counts_toward_terminal_totals(record.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                entry.failure_count += 1;
            }
            let latency_status = if is_success_like {
                Some("success")
            } else {
                record.status.as_deref()
            };
            entry.record_exact_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
            entry.record_exact_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
            entry.total_tokens += record.total_tokens.unwrap_or_default();
            entry.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
            entry.total_cost += record.cost.unwrap_or_default();
        }
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
        && let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)?
    {
        query_crs_deltas(
            &state.pool,
            relay,
            effective_range.start.timestamp(),
            crate::stats::exclusive_epoch_upper_bound(effective_range.end),
        )
        .await?
    } else {
        Vec::new()
    };
    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += delta.total_count;
            entry.success_count += delta.success_count;
            entry.failure_count += delta.failure_count;
            entry.total_tokens += delta.total_tokens;
            entry.total_cost += delta.total_cost;
        }
    }

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
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeseries_point_clears_latency_when_bucket_has_no_calls() {
        let mut aggregate = BucketAggregate::default();
        aggregate.first_byte_sample_count = 1;
        aggregate.first_byte_ttfb_sum_ms = 750.0;
        aggregate.first_byte_ttfb_values.push(750.0);
        aggregate.first_response_byte_total_sample_count = 1;
        aggregate.first_response_byte_total_sum_ms = 18_225.02;
        aggregate
            .first_response_byte_total_values
            .push(18_225.02);

        let point = timeseries_point_from_aggregate(
            Utc.timestamp_opt(1_775_608_200, 0)
                .single()
                .expect("valid start timestamp"),
            Utc.timestamp_opt(1_775_608_260, 0)
                .single()
                .expect("valid end timestamp"),
            &aggregate,
        );

        assert_eq!(point.total_count, 0);
        assert_eq!(point.first_byte_sample_count, 0);
        assert!(point.first_byte_avg_ms.is_none());
        assert!(point.first_byte_p95_ms.is_none());
        assert_eq!(point.first_response_byte_total_sample_count, 0);
        assert!(point.first_response_byte_total_avg_ms.is_none());
        assert!(point.first_response_byte_total_p95_ms.is_none());
    }
}
