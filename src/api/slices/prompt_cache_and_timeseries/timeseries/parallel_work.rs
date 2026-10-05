use super::*;
pub(crate) struct ParallelWorkProjectionBaseline {
    pub(crate) response: ParallelWorkStatsResponse,
    pub(crate) bucket_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) active_minute_stats: ParallelWorkActiveMinuteStats,
}

pub(crate) async fn load_parallel_work_stats_response(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkStatsResponse, ApiError> {
    load_parallel_work_stats_response_at(state, params, Utc::now()).await
}

pub(crate) async fn load_parallel_work_stats_response_at(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
    now: DateTime<Utc>,
) -> Result<ParallelWorkStatsResponse, ApiError> {
    load_parallel_work_projection_baseline_at(state, params, now)
        .await
        .map(|baseline| baseline.response)
}

pub(crate) async fn load_parallel_work_projection_baseline(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkProjectionBaseline, ApiError> {
    load_parallel_work_projection_baseline_at(state, params, Utc::now()).await
}

pub(crate) async fn load_parallel_work_projection_baseline_at(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
    now: DateTime<Utc>,
) -> Result<ParallelWorkProjectionBaseline, ApiError> {
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let upstream_account_id = params.upstream_account_id;
    let requested_range_window =
        resolve_range_window_at(&params.range, requested_reporting_tz, now)?;
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
        resolve_range_window_at(&params.range, reporting_tz, now)?
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

    let bucket_keys = if bucket_seconds >= 3_600 {
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
            let leading_exact_end = leading_full_bucket_start.min(range_window.end);
            let leading_bucket_keys = query_parallel_work_exact_key_sets(
                &state.pool,
                range_window.start,
                leading_exact_end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                upstream_account_id,
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
            upstream_account_id,
            Some(rollup_live_cursor),
            Some(snapshot_id),
        )
        .await?;
        for (bucket_epoch, keys) in tail_bucket_keys {
            bucket_keys.entry(bucket_epoch).or_default().extend(keys);
        }
        bucket_keys
    } else {
        query_parallel_work_exact_key_sets(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
            None,
            None,
        )
        .await?
    };
    let current_counts = bucket_keys
        .iter()
        .map(|(bucket_start_epoch, prompt_cache_keys)| {
            (*bucket_start_epoch, prompt_cache_keys.len() as i64)
        })
        .collect::<BTreeMap<_, _>>();
    let conversations = if range_window.duration <= ChronoDuration::hours(24) {
        query_parallel_work_conversation_spans(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
        )
        .await?
    } else {
        Vec::new()
    };
    let configured_full_detail_start_epoch =
        shanghai_retention_cutoff(state.config.invocation_success_full_days).timestamp();
    let active_minute_stats = query_parallel_work_active_minute_stats(
        &state.pool,
        range_window.start,
        range_window.end,
        source_scope,
        upstream_account_id,
        Some(
            load_parallel_work_full_detail_start_epoch(&state.pool)
                .await?
                .map(|persisted| persisted.max(configured_full_detail_start_epoch))
                .unwrap_or(configured_full_detail_start_epoch),
        ),
    )
    .await?;

    let current = build_parallel_work_window_response(
        fill_start,
        fill_end,
        bucket_seconds,
        reporting_tz,
        &current_counts,
        active_minute_stats,
        reporting_tz,
        time_zone_fallback,
        conversations,
    )?;

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
