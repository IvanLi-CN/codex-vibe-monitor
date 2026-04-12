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
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
        let first_response_byte_total_avg_ms = agg.first_response_byte_total_avg_ms();
        let first_response_byte_total_p95_ms = agg.first_response_byte_total_p95_ms();
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            in_flight_count: agg.in_flight_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms,
            first_byte_p95_ms,
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms,
            first_response_byte_total_p95_ms,
        });
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

pub(crate) async fn fetch_parallel_work_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let now = Utc::now();

    let minute7d_window = resolve_complete_parallel_work_window(
        now,
        ChronoDuration::days(7),
        60,
        requested_reporting_tz,
    )?;
    let requested_hour30d_window = resolve_complete_parallel_work_window(
        now,
        ChronoDuration::days(30),
        3_600,
        requested_reporting_tz,
    )?;
    let (hour30d_reporting_tz, hour30d_time_zone_fallback) =
        resolve_parallel_work_rollup_reporting_tz(
            requested_reporting_tz,
            &requested_hour30d_window,
        );
    let hour30d_window = if hour30d_time_zone_fallback {
        resolve_complete_parallel_work_window(
            now,
            ChronoDuration::days(30),
            3_600,
            hour30d_reporting_tz,
        )?
    } else {
        requested_hour30d_window
    };

    let minute7d_counts = query_parallel_work_minute_counts(
        &state.pool,
        minute7d_window.start,
        minute7d_window.end,
        requested_reporting_tz,
        source_scope,
    )
    .await?;
    let hour30d_counts = query_parallel_work_hourly_counts(
        &state.pool,
        hour30d_window.start,
        hour30d_window.end,
        source_scope,
    )
    .await?;

    let (day_all_window, day_all_reporting_tz, day_all_time_zone_fallback) =
        resolve_parallel_work_day_all_window_with_fallback(
            &state.pool,
            requested_reporting_tz,
            source_scope,
        )
        .await?;
    let day_all_counts = if let Some(window) = day_all_window.as_ref() {
        query_parallel_work_day_counts_from_hourly_rollups(
            &state.pool,
            window.start,
            window.end,
            day_all_reporting_tz,
            source_scope,
        )
        .await?
    } else {
        BTreeMap::new()
    };

    let latest_complete_day_end = local_midnight_utc(
        now.with_timezone(&day_all_reporting_tz).date_naive(),
        day_all_reporting_tz,
    );

    Ok(Json(ParallelWorkStatsResponse {
        minute7d: build_parallel_work_window_response(
            minute7d_window.start,
            minute7d_window.end,
            60,
            requested_reporting_tz,
            &minute7d_counts,
            requested_reporting_tz,
            false,
        )?,
        hour30d: build_parallel_work_window_response(
            hour30d_window.start,
            hour30d_window.end,
            3_600,
            hour30d_reporting_tz,
            &hour30d_counts,
            hour30d_reporting_tz,
            hour30d_time_zone_fallback,
        )?,
        day_all: if let Some(window) = day_all_window {
            build_parallel_work_window_response(
                window.start,
                window.end,
                86_400,
                day_all_reporting_tz,
                &day_all_counts,
                day_all_reporting_tz,
                day_all_time_zone_fallback,
            )?
        } else {
            empty_parallel_work_window_response(
                latest_complete_day_end,
                86_400,
                day_all_reporting_tz,
                day_all_time_zone_fallback,
            )
        },
    }))
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

    let (snapshot_id, hourly_rows, exact_records) = if range_plan.full_hour_range.is_some() {
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
        exact_records.extend(
            query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?,
        );
        (snapshot_id, hourly_rows, exact_records)
    } else {
        let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
        let exact_records =
            query_invocation_exact_records(&state.pool, &range_plan, source_scope, snapshot_id)
                .await?;
        (snapshot_id, Vec::new(), exact_records)
    };

    for row in hourly_rows {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += row.total_count;
        entry.success_count += row.success_count;
        entry.failure_count += row.failure_count;
        entry.total_tokens += row.total_tokens;
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
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            in_flight_count: agg.in_flight_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms: agg.first_byte_avg_ms(),
            first_byte_p95_ms: agg.first_byte_p95_ms(),
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms: agg.first_response_byte_total_avg_ms(),
            first_response_byte_total_p95_ms: agg.first_response_byte_total_p95_ms(),
        });
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
