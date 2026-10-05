use super::aggregation::{
    add_exact_record_to_timeseries_aggregate, add_exact_records_to_timeseries_aggregates,
    add_optional_token_components, add_pending_timeseries_deltas,
    add_rollup_rows_to_timeseries_aggregates, add_terminal_timeseries_records,
    build_timeseries_response, collect_in_flight_aggregate_records, fill_timeseries_buckets,
    fold_minute_projection_aggregates, overlay_runtime_timeseries_in_flight,
    timeseries_point_from_aggregate,
};
use super::minute_projection::{
    TimeseriesMinuteProjectionV2Load, TimeseriesMinuteProjectionWarmOutcome,
    complete_minute_bounds, evaluate_timeseries_minute_projection_eligibility,
    load_timeseries_minute_projection_v2,
    store_timeseries_minute_projection_v2_warm_with_eligibility_retry,
    timeseries_minute_projection_v2_snapshot_is_current,
};
use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;
pub(crate) fn timeseries_topic_uses_hourly_rollup_baseline(
    _params: &TimeseriesQuery,
    reporting_tz: Tz,
    range_window: &RangeWindow,
    bucket_seconds: i64,
    invocation_max_days: u64,
) -> Result<bool, ApiError> {
    if bucket_seconds < 3_600 {
        return Ok(false);
    }
    let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, range_window);
    let needs_historical_rollups =
        range_window.start < shanghai_retention_cutoff(invocation_max_days);
    if !tz_is_hour_aligned && needs_historical_rollups {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
        )));
    }
    Ok(tz_is_hour_aligned)
}

pub(crate) async fn fetch_timeseries_query(
    state: Arc<AppState>,
    params: TimeseriesQuery,
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

    if timeseries_topic_uses_hourly_rollup_baseline(
        &params,
        reporting_tz,
        &range_window,
        bucket_seconds,
        state.config.invocation_max_days,
    )? {
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

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);
    let use_minute_projection = range_window.duration <= ChronoDuration::days(1);
    let mut projection_eligibility = evaluate_timeseries_minute_projection_eligibility(
        &state.pool,
        state.terminal_projection_hub.as_ref(),
        use_minute_projection,
        source_scope,
        None,
        start_dt,
        end_dt,
    )
    .await?;
    if projection_eligibility.can_load()
        && let Some(TimeseriesMinuteProjectionV2Load {
            aggregates: minute_aggregates,
            cursor: projection_cursor,
            snapshot_fence,
        }) =
            load_timeseries_minute_projection_v2(&state.pool, start_dt, end_dt, source_scope, None)
                .await?
    {
        let mut aggregates =
            fold_minute_projection_aggregates(minute_aggregates, bucket_seconds, reporting_tz)?;
        let (full_minute_start_epoch, full_minute_end_epoch) =
            complete_minute_bounds(start_dt, end_dt);
        let full_minute_start = Utc
            .timestamp_opt(full_minute_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid timeseries full-minute start"))?;
        let full_minute_end = Utc
            .timestamp_opt(full_minute_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid timeseries full-minute end"))?;
        let mut exact_records = query_invocation_aggregate_records_from_live_range(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(projection_cursor),
            Some(snapshot_id),
        )
        .await?;
        for (boundary_start, boundary_end) in [
            (start_dt, end_dt.min(full_minute_start)),
            (start_dt.max(full_minute_end), end_dt),
        ] {
            if let Some(range) = exact_utc_range(boundary_start, boundary_end)? {
                exact_records.extend(
                    query_invocation_aggregate_records_from_live_range(
                        &state.pool,
                        range,
                        source_scope,
                        None,
                        Some(snapshot_id),
                    )
                    .await?,
                );
            }
        }
        let in_flight_records = query_in_flight_invocation_aggregate_records_from_live_range(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(snapshot_id),
        )
        .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&in_flight_records);
        let mut seen_ids = exact_records
            .iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        extend_unique_invocation_records(&mut exact_records, &mut seen_ids, in_flight_records);
        for record in exact_records {
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
        let memory_overlay_count = add_pending_timeseries_deltas(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            None,
            full_minute_start,
            full_minute_end,
            bucket_seconds,
            reporting_tz,
            Some(projection_cursor),
            snapshot_id,
        )?;
        let fill_start_epoch =
            align_reporting_bucket_epoch(start_dt.timestamp(), bucket_seconds, reporting_tz)?;
        let fill_end_epoch =
            resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
        let mut bucket_cursor = fill_start_epoch;
        while bucket_cursor < fill_end_epoch {
            aggregates.entry(bucket_cursor).or_default();
            bucket_cursor =
                next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
        }
        overlay_runtime_timeseries_in_flight(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            None,
            start_dt,
            end_dt,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
        if timeseries_minute_projection_v2_snapshot_is_current(
            &state.pool,
            state.terminal_projection_hub.as_ref(),
            start_dt,
            end_dt,
            source_scope,
            None,
            &snapshot_fence,
        )
        .await?
        {
            debug!(
                route = "timeseries_http_or_topic",
                builder = "minute_projection_v2",
                response_source = "minute_projection",
                minute_rollup_count = aggregates.len(),
                memory_overlay_count,
                exact_fallback_minute_count = 2_u8,
                raw_row_count = db_runtime_records.len(),
                coverage_state = "covered",
                projection_cursor,
                "built open-window timeseries from minute projection"
            );
            return build_timeseries_response(
                start_dt,
                end_dt,
                bucket_seconds,
                snapshot_id,
                bucket_selection,
                aggregates,
                fill_start_epoch,
                fill_end_epoch,
                reporting_tz,
            );
        }
        projection_eligibility.mark_coverage_invalidated();
        debug!(
            route = "timeseries_http_or_topic",
            projection_cursor,
            "minute projection coverage changed while loading the live tail; falling back to exact records"
        );
    }

    let (records, response_source) = if use_minute_projection {
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
        if projection_eligibility.can_warm() {
            // Projection persistence is P2 work. Never make a read request wait for a SQLite
            // writer when a first-time exact fallback already produced a valid response.
            let projection_pool = state.pool.clone();
            let terminal_projection_hub = state.terminal_projection_hub.clone();
            tokio::spawn(async move {
                match store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
                    &projection_pool,
                    start_dt,
                    end_dt,
                    source_scope,
                    None,
                    terminal_projection_hub.as_ref(),
                    "http_exact_fallback",
                )
                .await
                {
                    Ok(TimeseriesMinuteProjectionWarmOutcome::Stored) => {}
                    Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(_)) => {
                        debug!(
                            route = "timeseries_projection",
                            projection_store_outcome = "deferred",
                            "v2 minute projection warm write yielded to higher-priority work"
                        );
                    }
                    Err(error) => {
                        debug!(
                            route = "timeseries_projection",
                            projection_store_outcome = "failed",
                            ?error,
                            "v2 minute projection warm write failed"
                        );
                    }
                }
            });
        }
        (
            records,
            if projection_eligibility.coverage_invalidation_pending() {
                "exact_fallback_projection_invalidated"
            } else {
                "exact_fallback"
            },
        )
    } else {
        (
            query_invocation_aggregate_records_from_live_range(
                &state.pool,
                ExactUtcRange {
                    start: start_dt,
                    end: end_dt,
                },
                source_scope,
                None,
                Some(snapshot_id),
            )
            .await?,
            "exact_range",
        )
    };
    debug!(
        route = "timeseries_http_or_topic",
        builder = "minute_projection_v2",
        response_source,
        raw_row_count = records.len(),
        coverage_state = if response_source == "minute_projection" {
            "covered"
        } else {
            "warming"
        },
        "built open-window timeseries"
    );
    let db_runtime_records = collect_in_flight_aggregate_records(&records);

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
        entry.record_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
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
        let cost = record.cost.unwrap_or(0.0);
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
    overlay_runtime_timeseries_in_flight(
        state.as_ref(),
        &mut aggregates,
        source_scope,
        None,
        start_dt,
        end_dt,
        bucket_seconds,
        reporting_tz,
        &db_runtime_records,
    )?;

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

pub(crate) async fn fetch_timeseries_for_account(
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

    let minute_projection_candidate =
        bucket_seconds < 3_600 && range_window.duration <= ChronoDuration::days(1);
    let mut projection_eligibility = evaluate_timeseries_minute_projection_eligibility(
        &state.pool,
        state.terminal_projection_hub.as_ref(),
        minute_projection_candidate,
        source_scope,
        Some(upstream_account_id),
        start_dt,
        end_dt,
    )
    .await?;
    if projection_eligibility.can_load()
        && let Some(TimeseriesMinuteProjectionV2Load {
            aggregates: minute_aggregates,
            cursor: projection_cursor,
            snapshot_fence,
        }) = load_timeseries_minute_projection_v2(
            &state.pool,
            start_dt,
            end_dt,
            source_scope,
            Some(upstream_account_id),
        )
        .await?
    {
        let mut aggregates =
            fold_minute_projection_aggregates(minute_aggregates, bucket_seconds, reporting_tz)?;
        let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
        let (full_minute_start_epoch, full_minute_end_epoch) =
            complete_minute_bounds(start_dt, end_dt);
        let full_minute_start = Utc
            .timestamp_opt(full_minute_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account timeseries full-minute start"))?;
        let full_minute_end = Utc
            .timestamp_opt(full_minute_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account timeseries full-minute end"))?;
        let mut exact_records = query_invocation_aggregate_records_from_live_range_for_account(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(projection_cursor),
            Some(snapshot_id),
            upstream_account_id,
        )
        .await?;
        for (boundary_start, boundary_end) in [
            (start_dt, end_dt.min(full_minute_start)),
            (start_dt.max(full_minute_end), end_dt),
        ] {
            if let Some(range) = exact_utc_range(boundary_start, boundary_end)? {
                exact_records.extend(
                    query_invocation_aggregate_records_from_live_range_for_account(
                        &state.pool,
                        range,
                        source_scope,
                        None,
                        Some(snapshot_id),
                        upstream_account_id,
                    )
                    .await?,
                );
            }
        }
        let in_flight_records =
            query_in_flight_invocation_aggregate_records_from_live_range_for_account(
                &state.pool,
                ExactUtcRange {
                    start: full_minute_start,
                    end: full_minute_end,
                },
                source_scope,
                Some(snapshot_id),
                upstream_account_id,
            )
            .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&in_flight_records);
        let mut seen_ids = exact_records
            .iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        extend_unique_invocation_records(&mut exact_records, &mut seen_ids, in_flight_records);
        for record in exact_records {
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
        let memory_overlay_count = add_pending_timeseries_deltas(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            Some(upstream_account_id),
            full_minute_start,
            full_minute_end,
            bucket_seconds,
            reporting_tz,
            Some(projection_cursor),
            snapshot_id,
        )?;
        let fill_start_epoch =
            align_reporting_bucket_epoch(start_dt.timestamp(), bucket_seconds, reporting_tz)?;
        let fill_end_epoch =
            resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
        let mut bucket_cursor = fill_start_epoch;
        while bucket_cursor < fill_end_epoch {
            aggregates.entry(bucket_cursor).or_default();
            bucket_cursor =
                next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
        }
        overlay_runtime_timeseries_in_flight(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            Some(upstream_account_id),
            start_dt,
            end_dt,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
        if timeseries_minute_projection_v2_snapshot_is_current(
            &state.pool,
            state.terminal_projection_hub.as_ref(),
            start_dt,
            end_dt,
            source_scope,
            Some(upstream_account_id),
            &snapshot_fence,
        )
        .await?
        {
            debug!(
                route = "timeseries_http_or_topic",
                builder = "minute_projection_v2",
                response_source = "minute_projection",
                upstream_account_id,
                minute_rollup_count = aggregates.len(),
                memory_overlay_count,
                exact_fallback_minute_count = 2_u8,
                raw_row_count = db_runtime_records.len(),
                coverage_state = "covered",
                projection_cursor,
                "built account open-window timeseries from minute projection"
            );
            return build_timeseries_response(
                start_dt,
                end_dt,
                bucket_seconds,
                snapshot_id,
                bucket_selection,
                aggregates,
                fill_start_epoch,
                fill_end_epoch,
                reporting_tz,
            );
        }
        projection_eligibility.mark_coverage_invalidated();
        debug!(
            route = "timeseries_http_or_topic",
            upstream_account_id,
            projection_cursor,
            "account minute projection coverage changed while loading the live tail; falling back to exact records"
        );
    }

    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    if projection_eligibility.coverage_invalidation_pending() {
        let records = query_invocation_aggregate_records_from_live_range_for_account(
            &state.pool,
            ExactUtcRange {
                start: start_dt,
                end: end_dt,
            },
            source_scope,
            None,
            Some(snapshot_id),
            upstream_account_id,
        )
        .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&records);
        add_exact_records_to_timeseries_aggregates(
            &mut aggregates,
            records,
            bucket_seconds,
            reporting_tz,
        )?;
        overlay_runtime_timeseries_in_flight(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            Some(upstream_account_id),
            start_dt,
            end_dt,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
        if projection_eligibility.can_warm() {
            let projection_pool = state.pool.clone();
            let terminal_projection_hub = state.terminal_projection_hub.clone();
            tokio::spawn(async move {
                match store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
                    &projection_pool,
                    start_dt,
                    end_dt,
                    source_scope,
                    Some(upstream_account_id),
                    terminal_projection_hub.as_ref(),
                    "account_exact_fallback",
                )
                .await
                {
                    Ok(TimeseriesMinuteProjectionWarmOutcome::Stored) => {}
                    Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(_)) => {
                        debug!(
                            route = "timeseries_projection",
                            upstream_account_id,
                            projection_store_outcome = "deferred",
                            "account v2 minute projection warm write yielded to higher-priority work"
                        );
                    }
                    Err(error) => {
                        debug!(
                            route = "timeseries_projection",
                            upstream_account_id,
                            projection_store_outcome = "deferred_failed",
                            ?error,
                            "account v2 minute projection warm write failed"
                        );
                    }
                }
            });
        }
        debug!(
            route = "timeseries_http_or_topic",
            builder = "minute_projection_v2",
            response_source = "exact_fallback_projection_invalidated",
            upstream_account_id,
            raw_row_count = db_runtime_records.len(),
            coverage_state = "warming",
            "built account open-window timeseries from exact fallback"
        );
        return build_timeseries_response(
            start_dt,
            end_dt,
            bucket_seconds,
            snapshot_id,
            bucket_selection,
            aggregates,
            fill_start_epoch,
            fill_end_epoch,
            reporting_tz,
        );
    }
    if projection_eligibility.is_candidate() && projection_eligibility.can_warm() {
        // Account projections include empty minutes, so a first exact fallback must warm
        // the complete selection instead of relying on future terminal events to fill gaps.
        let projection_pool = state.pool.clone();
        let terminal_projection_hub = state.terminal_projection_hub.clone();
        tokio::spawn(async move {
            match store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
                &projection_pool,
                start_dt,
                end_dt,
                source_scope,
                Some(upstream_account_id),
                terminal_projection_hub.as_ref(),
                "account_exact_fallback",
            )
            .await
            {
                Ok(TimeseriesMinuteProjectionWarmOutcome::Stored) => {}
                Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(_)) => {
                    debug!(
                        route = "timeseries_projection",
                        upstream_account_id,
                        projection_store_outcome = "deferred",
                        "account v2 minute projection warm write yielded to higher-priority work"
                    );
                }
                Err(error) => {
                    debug!(
                        route = "timeseries_projection",
                        upstream_account_id,
                        projection_store_outcome = "deferred_failed",
                        ?error,
                        "account v2 minute projection warm write failed"
                    );
                }
            }
        });
    }
    let mut db_runtime_records = HashMap::new();
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
        push_exact_range(
            &mut live_exact_ranges,
            start_dt,
            end_dt.min(first_full_bucket_start),
        )?;
        push_exact_range(
            &mut live_exact_ranges,
            start_dt.max(last_full_bucket_end),
            end_dt,
        )?;
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
        db_runtime_records.extend(collect_in_flight_aggregate_records(&exact_records));
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
        db_runtime_records.extend(collect_in_flight_aggregate_records(&tail_records));
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
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries start epoch"))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries end epoch"))
            })?;
        let archived_rows =
            crate::stats::query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
                &state.pool,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
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
    }
    overlay_runtime_timeseries_in_flight(
        state.as_ref(),
        &mut aggregates,
        source_scope,
        Some(upstream_account_id),
        start_dt,
        end_dt,
        bucket_seconds,
        reporting_tz,
        &db_runtime_records,
    )?;
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

pub(crate) struct TimeseriesHourlyRollupBaseline {
    pub(crate) snapshot_id: i64,
    pub(crate) aggregates: BTreeMap<i64, BucketAggregate>,
}

pub(crate) async fn build_timeseries_hourly_rollup_baseline(
    state: &AppState,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
    include_runtime: bool,
) -> Result<TimeseriesHourlyRollupBaseline, ApiError> {
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
        if let Some((hourly_cursor, hourly_end_epoch)) = range_plan.full_hour_range {
            let mut tx = state.pool.begin().await?;
            let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
            let rollup_live_cursor =
                load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
            let hourly_rows = query_invocation_hourly_rollup_range_tx(
                tx.as_mut(),
                hourly_cursor,
                hourly_end_epoch,
                source_scope,
            )
            .await?;
            let mut exact_records = query_invocation_exact_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                snapshot_id,
            )
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
    let db_runtime_records = if include_runtime {
        collect_in_flight_aggregate_records(&exact_records)
    } else {
        HashMap::new()
    };
    for record in exact_records {
        if !include_runtime
            && prompt_shared::invocation_status_is_in_flight(record.status.as_deref())
        {
            continue;
        }
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            add_exact_record_to_timeseries_aggregate(entry, &record);
        }
    }

    if include_runtime {
        overlay_runtime_timeseries_in_flight(
            state,
            &mut aggregates,
            source_scope,
            None,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
    }

    Ok(TimeseriesHourlyRollupBaseline {
        snapshot_id,
        aggregates,
    })
}

pub(crate) async fn build_timeseries_account_hourly_rollup_baseline(
    state: &AppState,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
    upstream_account_id: i64,
) -> Result<TimeseriesHourlyRollupBaseline, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    debug_assert!(bucket_seconds >= 3_600);
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;
    let mut aggregates = BTreeMap::new();
    fill_timeseries_buckets(
        &mut aggregates,
        range_window.start,
        range_window.end,
        bucket_seconds,
        reporting_tz,
    )?;

    let (snapshot_id, hourly_rows, exact_records, archive_overlap_ids) = {
        let mut tx = state.pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
        let hourly_rows =
            if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
                query_upstream_account_stats_rollup_range_tx(
                    tx.as_mut(),
                    "upstream_account_stats_hourly",
                    range_start_epoch,
                    range_end_epoch,
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                Vec::new()
            };
        let mut exact_records = Vec::new();
        let boundary_snapshot_id = rollup_live_cursor.min(snapshot_id);
        if !range_plan.live_exact_ranges.is_empty() && boundary_snapshot_id > 0 {
            exact_records.extend(
                query_invocation_exact_records_for_account_tx(
                    tx.as_mut(),
                    &range_plan,
                    source_scope,
                    boundary_snapshot_id,
                    upstream_account_id,
                )
                .await?,
            );
        }
        let mut archive_overlap_ids = HashSet::new();
        if rollup_live_cursor < snapshot_id {
            let tail_range_plan = HourlyRollupExactRangePlan {
                full_hour_range: None,
                live_exact_ranges: exact_utc_range(range_window.start, range_window.end)?
                    .into_iter()
                    .collect(),
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
            exact_records.extend(tail_records);
        }
        (snapshot_id, hourly_rows, exact_records, archive_overlap_ids)
    };

    add_rollup_rows_to_timeseries_aggregates(
        &mut aggregates,
        hourly_rows,
        bucket_seconds,
        reporting_tz,
    )?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries start epoch"))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries end epoch"))
            })?;
        let archived_rows =
            crate::stats::query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
                &state.pool,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                source_scope,
                Some((archived_start, archived_end)),
                Some(&archive_overlap_ids),
                upstream_account_id,
            )
            .await?;
        add_rollup_rows_to_timeseries_aggregates(
            &mut aggregates,
            archived_rows,
            bucket_seconds,
            reporting_tz,
        )?;
    }
    add_terminal_timeseries_records(&mut aggregates, exact_records, bucket_seconds, reporting_tz)?;

    Ok(TimeseriesHourlyRollupBaseline {
        snapshot_id,
        aggregates,
    })
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let baseline = build_timeseries_hourly_rollup_baseline(
        state.as_ref(),
        reporting_tz,
        source_scope,
        &range_window,
        &bucket_selection,
        true,
    )
    .await?;
    let fill_start_epoch = align_reporting_bucket_epoch(
        range_window.start.timestamp(),
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(
        range_window.end,
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    build_timeseries_response(
        range_window.start,
        range_window.display_end,
        bucket_selection.bucket_seconds,
        baseline.snapshot_id,
        bucket_selection,
        baseline.aggregates,
        fill_start_epoch,
        fill_end_epoch,
        reporting_tz,
    )
}
