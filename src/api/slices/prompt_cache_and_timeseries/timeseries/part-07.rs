struct RuntimeTimeseriesOverlayContext<'a> {
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    db_runtime_records: &'a HashMap<(String, String), InvocationAggregateRecord>,
}

fn overlay_runtime_timeseries_in_flight(
    state: &AppState,
    context: RuntimeTimeseriesOverlayContext<'_>,
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
) -> Result<(), ApiError> {
    let mut runtime_overlay_row_count = 0_i64;
    let mut stale_db_runtime_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        if overlay_runtime_timeseries_record(
            aggregates,
            record,
            &context,
            &mut stale_db_runtime_row_count,
        )? {
            runtime_overlay_row_count += 1;
        }
    }
    if runtime_overlay_row_count > 0 || stale_db_runtime_row_count > 0 {
        debug!(
            endpoint = "/api/timeseries",
            runtime_overlay_row_count,
            stale_db_runtime_row_count,
            upstream_account_id = context.upstream_account_id,
            "overlayed memory runtime in-flight records into timeseries"
        );
    }
    Ok(())
}

fn overlay_runtime_timeseries_record(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    record: ApiInvocation,
    context: &RuntimeTimeseriesOverlayContext<'_>,
    stale_db_runtime_row_count: &mut i64,
) -> Result<bool, ApiError> {
    let key = (record.invoke_id.clone(), record.occurred_at.clone());
    let is_in_scope =
        context.source_scope != InvocationSourceScope::ProxyOnly || record.source == SOURCE_PROXY;
    let is_in_flight = prompt_shared::invocation_status_is_in_flight(record.status.as_deref());
    let is_account_match = context
        .upstream_account_id
        .is_none_or(|expected| record.upstream_account_id == Some(expected));
    let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
        return Ok(false);
    };
    let is_in_range = occurred_utc >= context.start_dt && occurred_utc < context.end_dt;
    if !is_in_scope || !is_in_flight || !is_account_match || !is_in_range {
        if let Some(db_record) = context.db_runtime_records.get(&key) {
            subtract_stale_db_runtime_record(
                aggregates,
                db_record,
                context.bucket_seconds,
                context.reporting_tz,
                stale_db_runtime_row_count,
            )?;
        }
        return Ok(false);
    }
    if let Some(db_record) = context.db_runtime_records.get(&key) {
        subtract_stale_db_runtime_record(
            aggregates,
            db_record,
            context.bucket_seconds,
            context.reporting_tz,
            stale_db_runtime_row_count,
        )?;
    }
    let bucket_epoch = align_reporting_bucket_epoch(
        occurred_utc.timestamp(),
        context.bucket_seconds,
        context.reporting_tz,
    )?;
    let entry = aggregates.entry(bucket_epoch).or_default();
    entry.total_count += 1;
    entry.in_flight_count += 1;
    entry
        .in_flight_phase_counts
        .increment_phase_name(runtime_record_live_phase(&record));
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
    Ok(true)
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

struct MaterializedTimeseriesResponseInput<'a> {
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    snapshot_id: i64,
    bucket_selection: &'a TimeseriesBucketSelection,
    aggregates: &'a BTreeMap<i64, BucketAggregate>,
    runtime_overlay: &'a BTreeMap<i64, BucketAggregate>,
    reporting_tz: Tz,
}

fn build_materialized_timeseries_response(
    input: MaterializedTimeseriesResponseInput<'_>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let MaterializedTimeseriesResponseInput {
        start_dt,
        end_dt,
        bucket_seconds,
        snapshot_id,
        bucket_selection,
        aggregates,
        runtime_overlay,
        reporting_tz,
    } = input;
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

pub(crate) struct TimeseriesResponseInput {
    pub(crate) start_dt: DateTime<Utc>,
    pub(crate) end_dt: DateTime<Utc>,
    pub(crate) bucket_seconds: i64,
    pub(crate) snapshot_id: i64,
    pub(crate) bucket_selection: TimeseriesBucketSelection,
    pub(crate) aggregates: BTreeMap<i64, BucketAggregate>,
    pub(crate) fill_start_epoch: i64,
    pub(crate) fill_end_epoch: i64,
    pub(crate) reporting_tz: Tz,
}

pub(crate) fn build_timeseries_response(
    input: TimeseriesResponseInput,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let TimeseriesResponseInput {
        start_dt,
        end_dt,
        bucket_seconds,
        snapshot_id,
        bucket_selection,
        aggregates,
        fill_start_epoch,
        fill_end_epoch,
        reporting_tz,
    } = input;
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
    load_parallel_work_stats_response(&state, params)
        .await
        .map(Json)
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
    response
        .headers_mut()
        .insert(axum::http::header::ETAG, etag_value);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    Ok(response)
}

pub(crate) fn parallel_work_stats_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!("\"parallel-work-{digest:x}\"")
}

pub(crate) fn request_etag_matches(headers: &HeaderMap, etag: &str) -> bool {
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

pub(crate) struct ParallelWorkProjectionBaseline {
    pub(crate) response: ParallelWorkStatsResponse,
    pub(crate) bucket_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) active_minute_stats: ParallelWorkActiveMinuteStats,
}

pub(crate) async fn load_parallel_work_stats_response(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkStatsResponse, ApiError> {
    load_parallel_work_projection_baseline(state, params)
        .await
        .map(|baseline| baseline.response)
}

pub(crate) async fn load_parallel_work_projection_baseline(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkProjectionBaseline, ApiError> {
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let upstream_account_id = params.upstream_account_id;
    let requested_range_window = resolve_range_window(&params.range, requested_reporting_tz)?;
    let bucket_params = TimeseriesQuery {
        range: params.range.clone(),
        bucket: params.bucket.clone(),
        settlement_hour: None,
        time_zone: params.time_zone.clone(),
        upstream_account_id,
    };
    let bucket_selection = resolve_timeseries_bucket_selection(
        &bucket_params,
        &requested_range_window,
        state.config.invocation_max_days,
    )?;
    let bucket_seconds = bucket_selection.bucket_seconds;
    let (reporting_tz, time_zone_fallback) = if bucket_seconds >= 3_600 {
        resolve_parallel_work_rollup_reporting_tz(requested_reporting_tz, &requested_range_window)
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

    let bucket_keys = load_parallel_work_bucket_keys(
        state,
        ParallelWorkBucketKeyContext {
            bucket_seconds,
            fill_start,
            fill_start_epoch,
            range_window: &range_window,
            reporting_tz,
            source_scope,
            upstream_account_id,
        },
    )
    .await?;
    let current_counts = bucket_keys
        .iter()
        .map(|(bucket_start_epoch, prompt_cache_keys)| {
            (*bucket_start_epoch, prompt_cache_keys.len() as i64)
        })
        .collect::<BTreeMap<_, _>>();
    let (conversations, active_minute_stats) =
        load_parallel_work_baseline_live_sources(ParallelWorkLiveSourcesContext {
            state,
            range_window: &range_window,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
        })
        .await?;

    let current = build_parallel_work_window_response(ParallelWorkWindowResponseInput {
        range_start: fill_start,
        range_end: fill_end,
        bucket_seconds,
        reporting_tz,
        counts_by_bucket: current_counts,
        active_minute_stats,
        effective_time_zone: reporting_tz,
        time_zone_fallback,
        conversations,
    })?;

    Ok(ParallelWorkProjectionBaseline {
        response: ParallelWorkStatsResponse {
            current: current.clone(),
            minute7d: current.clone(),
            hour30d: current.clone(),
            day_all: current,
        },
        bucket_keys,
        active_minute_stats,
    })
}

struct ParallelWorkLiveSourcesContext<'a> {
    state: &'a Arc<AppState>,
    range_window: &'a RangeWindow,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
}

async fn load_parallel_work_baseline_live_sources(
    context: ParallelWorkLiveSourcesContext<'_>,
) -> Result<(Vec<ParallelWorkConversation>, ParallelWorkActiveMinuteStats), ApiError> {
    let conversations = if context.range_window.duration <= ChronoDuration::hours(24) {
        query_parallel_work_conversation_spans(
            &context.state.pool,
            context.range_window.start,
            context.range_window.end,
            context.bucket_seconds,
            context.reporting_tz,
            context.source_scope,
            context.upstream_account_id,
        )
        .await?
    } else {
        Vec::new()
    };
    let active_minute_stats = load_parallel_work_active_minute_stats(
        context.state,
        context.range_window,
        context.source_scope,
        context.upstream_account_id,
    )
    .await?;
    Ok((conversations, active_minute_stats))
}

async fn load_parallel_work_active_minute_stats(
    state: &Arc<AppState>,
    range_window: &RangeWindow,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats, ApiError> {
    let configured_start =
        shanghai_retention_cutoff(state.config.invocation_success_full_days).timestamp();
    let persisted_start = load_parallel_work_full_detail_start_epoch(&state.pool).await?;
    let full_detail_start = persisted_start
        .map(|value| value.max(configured_start))
        .unwrap_or(configured_start);
    Ok(query_parallel_work_active_minute_stats(
        &state.pool,
        range_window.start,
        range_window.end,
        source_scope,
        upstream_account_id,
        Some(full_detail_start),
    )
    .await?)
}

struct ParallelWorkBucketKeyContext<'a> {
    bucket_seconds: i64,
    fill_start: DateTime<Utc>,
    fill_start_epoch: i64,
    range_window: &'a RangeWindow,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
}

async fn load_parallel_work_bucket_keys(
    state: &Arc<AppState>,
    context: ParallelWorkBucketKeyContext<'_>,
) -> Result<BTreeMap<i64, HashSet<String>>, ApiError> {
    let ParallelWorkBucketKeyContext {
        bucket_seconds,
        fill_start,
        fill_start_epoch,
        range_window,
        reporting_tz,
        source_scope,
        upstream_account_id,
    } = context;
    if bucket_seconds < 3_600 {
        return Ok(query_parallel_work_exact_key_sets(
            &state.pool,
            ParallelWorkExactKeySetsQuery {
                range_start: range_window.start,
                range_end: range_window.end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                upstream_account_id,
                start_after_id: None,
                snapshot_id: None,
            },
        )
        .await?);
    }
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
        upstream_account_id,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    drop(tx);
    if fill_start < range_window.start && range_window.start < leading_full_bucket_start {
        let leading_bucket_keys = query_parallel_work_exact_key_sets(
            &state.pool,
            ParallelWorkExactKeySetsQuery {
                range_start: range_window.start,
                range_end: leading_full_bucket_start.min(range_window.end),
                bucket_seconds,
                reporting_tz,
                source_scope,
                upstream_account_id,
                start_after_id: None,
                snapshot_id: Some(snapshot_id),
            },
        )
        .await?;
        merge_parallel_work_bucket_keys(&mut bucket_keys, leading_bucket_keys);
    }
    let tail_bucket_keys = query_parallel_work_exact_key_sets(
        &state.pool,
        ParallelWorkExactKeySetsQuery {
            range_start: range_window.start,
            range_end: range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id: None,
            start_after_id: Some(rollup_live_cursor),
            snapshot_id: Some(snapshot_id),
        },
    )
    .await?;
    merge_parallel_work_bucket_keys(&mut bucket_keys, tail_bucket_keys);
    Ok(bucket_keys)
}

fn merge_parallel_work_bucket_keys(
    target: &mut BTreeMap<i64, HashSet<String>>,
    additions: BTreeMap<i64, HashSet<String>>,
) {
    for (bucket_epoch, keys) in additions {
        target.entry(bucket_epoch).or_default().extend(keys);
    }
}

pub(crate) async fn query_parallel_work_conversation_spans(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
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
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
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
        let start_epoch = align_reporting_bucket_epoch(
            first_occurred_at.timestamp(),
            bucket_seconds,
            reporting_tz,
        )?;
        let end_bucket_epoch = align_reporting_bucket_epoch(
            last_occurred_at.timestamp(),
            bucket_seconds,
            reporting_tz,
        )?;
        let end_epoch =
            next_reporting_bucket_epoch(end_bucket_epoch, bucket_seconds, reporting_tz)?;
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

struct TimeseriesHourlyRollupBaseline {
    snapshot_id: i64,
    aggregates: BTreeMap<i64, BucketAggregate>,
}

fn add_invocation_hourly_rollup_to_timeseries_aggregate(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    row: InvocationHourlyRollupRecord,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
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
    entry.token_components_observed |= row.total_tokens > 0;
    entry.total_cost += row.total_cost;
    entry.non_success_cost += row.non_success_cost;
    entry.total_latency_sample_count += row.total_latency_sample_count;
    entry.total_latency_sum_ms += row.total_latency_sum_ms;
    entry.first_byte_sample_count += row.first_byte_sample_count;
    entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
    entry.first_byte_histogram =
        merge_timeseries_histogram(&entry.first_byte_histogram, &row.first_byte_histogram)?;
    entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
    entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
    entry.first_response_byte_total_histogram = merge_timeseries_histogram(
        &entry.first_response_byte_total_histogram,
        &row.first_response_byte_total_histogram,
    )?;
    entry.first_token_sample_count += row.first_token_sample_count;
    entry.first_token_sum_ms += row.first_token_sum_ms;
    entry.first_token_histogram =
        merge_timeseries_histogram(&entry.first_token_histogram, &row.first_token_histogram)?;
    Ok(())
}

fn merge_timeseries_histogram(current: &[i64], incoming_json: &str) -> Result<Vec<i64>, ApiError> {
    let incoming = decode_approx_histogram(incoming_json);
    if current.is_empty() {
        return Ok(incoming);
    }
    let mut merged = current.to_vec();
    merge_approx_histogram_into(&mut merged, &incoming)?;
    Ok(merged)
}

async fn build_timeseries_hourly_rollup_baseline(
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

    let mut aggregates = seed_timeseries_hourly_rollup_buckets(
        start_epoch,
        range_window.end,
        bucket_seconds,
        reporting_tz,
    )?;

    let (snapshot_id, hourly_rows, exact_records, archived_hourly_rows) =
        load_timeseries_hourly_rollup_sources(state, &range_plan, source_scope).await?;

    for row in hourly_rows.into_iter().chain(archived_hourly_rows) {
        add_invocation_hourly_rollup_to_timeseries_aggregate(
            &mut aggregates,
            row,
            bucket_seconds,
            reporting_tz,
        )?;
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
            RuntimeTimeseriesOverlayContext {
                source_scope,
                upstream_account_id: None,
                start_dt: range_window.start,
                end_dt: range_window.end,
                bucket_seconds,
                reporting_tz,
                db_runtime_records: &db_runtime_records,
            },
            &mut aggregates,
        )?;
    }

    Ok(TimeseriesHourlyRollupBaseline {
        snapshot_id,
        aggregates,
    })
}

fn seed_timeseries_hourly_rollup_buckets(
    start_epoch: i64,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end, bucket_seconds, reporting_tz)?;
    let mut aggregates = BTreeMap::new();
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }
    Ok(aggregates)
}

async fn load_timeseries_hourly_rollup_sources(
    state: &AppState,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
) -> Result<
    (
        i64,
        Vec<InvocationHourlyRollupRecord>,
        Vec<InvocationAggregateRecord>,
        Vec<InvocationHourlyRollupRecord>,
    ),
    ApiError,
> {
    let (snapshot_id, hourly_rows, exact_records, overlap_ids) =
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
                range_plan,
                source_scope,
                snapshot_id,
            )
            .await?;
            let tail_records = query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?;
            let overlap_ids = tail_records
                .iter()
                .map(|record| record.id)
                .collect::<HashSet<_>>();
            exact_records.extend(tail_records);
            (snapshot_id, hourly_rows, exact_records, overlap_ids)
        } else {
            let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
            let exact_records =
                query_invocation_exact_records(&state.pool, range_plan, source_scope, snapshot_id)
                    .await?;
            (snapshot_id, Vec::new(), exact_records, HashSet::new())
        };
    let archived = if let Some((start_epoch, end_epoch)) = range_plan.full_hour_range {
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries start epoch")))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries end epoch")))?;
        crate::stats::query_unmaterialized_invocation_archive_hourly_rollup_deltas(
            &state.pool,
            source_scope,
            Some((start, end)),
            Some(&overlap_ids),
        )
        .await?
    } else {
        Vec::new()
    };
    Ok((snapshot_id, hourly_rows, exact_records, archived))
}
