use super::*;
pub(crate) struct HydratedUpstreamAccountPreviewRows {
    pub(crate) rows: Vec<UpstreamAccountInvocationPreviewRow>,
    pub(crate) candidate_preview_id_count: usize,
    pub(crate) hydrated_preview_row_count: usize,
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<HydratedUpstreamAccountPreviewRows, ApiError> {
    let ids = query_live_upstream_account_activity_preview_candidate_ids_per_account(
        pool,
        source_scope,
        range,
        limit_per_account,
        None,
    )
    .await?;
    let rows = query_live_upstream_account_activity_preview_rows_by_ids(
        pool,
        source_scope,
        &ids,
        telemetry,
    )
    .await?;
    Ok(HydratedUpstreamAccountPreviewRows {
        candidate_preview_id_count: ids.len(),
        hydrated_preview_row_count: rows.len(),
        rows,
    })
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows_per_account_limit(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    Ok(
        query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
            pool,
            source_scope,
            range,
            limit_per_account,
            telemetry,
        )
        .await?
        .rows,
    )
}

pub(crate) async fn query_live_upstream_account_activity_existing_invocation_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
) -> Result<HashSet<i64>, ApiError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }

    let mut existing_ids = HashSet::new();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM codex_invocations \
             WHERE id IN (",
        );
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        existing_ids.extend(query.build_query_scalar::<i64>().fetch_all(pool).await?);
    }
    Ok(existing_ids)
}

pub(crate) async fn query_live_dashboard_activity_persisted_terminal_ids_tx(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    keys: &[(String, String)],
) -> Result<HashMap<(String, String), i64>, ApiError> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    const KEY_CHUNK_SIZE: usize = 250;
    let mut persisted = HashMap::new();
    for chunk in keys.chunks(KEY_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, invoke_id, occurred_at FROM codex_invocations WHERE (",
        );
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
        persisted.extend(
            query
                .build_query_as::<(i64, String, String)>()
                .fetch_all(&mut *connection)
                .await?
                .into_iter()
                .map(|(id, invoke_id, occurred_at)| ((invoke_id, occurred_at), id)),
        );
    }
    Ok(persisted)
}

pub(crate) async fn query_live_dashboard_activity_snapshot_cursor_tx(
    connection: &mut SqliteConnection,
) -> Result<i64, ApiError> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&mut *connection)
            .await?,
    )
}

pub(crate) async fn query_live_dashboard_activity_reconcile_probe(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    keys: &[(String, String)],
) -> Result<(i64, HashMap<(String, String), i64>), ApiError> {
    let mut tx = pool.begin().await?;
    let cursor = query_live_dashboard_activity_snapshot_cursor_tx(tx.as_mut()).await?;
    let persisted =
        query_live_dashboard_activity_persisted_terminal_ids_tx(tx.as_mut(), source_scope, keys)
            .await?;
    tx.commit().await?;
    Ok((cursor, persisted))
}

pub(crate) async fn query_live_dashboard_activity_snapshot_cursor(
    pool: &Pool<Sqlite>,
) -> Result<i64, ApiError> {
    let mut connection = pool.acquire().await?;
    query_live_dashboard_activity_snapshot_cursor_tx(&mut connection).await
}

pub(crate) async fn try_begin_dashboard_activity_consistency_barrier(
    pool: &Pool<Sqlite>,
) -> Result<Option<sqlx::Transaction<'static, Sqlite>>, ApiError> {
    match tokio::time::timeout(
        Duration::from_millis(10),
        pool.begin_with("BEGIN IMMEDIATE"),
    )
    .await
    {
        Ok(Ok(transaction)) => Ok(Some(transaction)),
        Ok(Err(err)) => {
            let err = anyhow::Error::new(err);
            if crate::is_sqlite_lock_error(&err) {
                debug!(error = %err, "dashboard consistency barrier is currently unavailable");
                Ok(None)
            } else {
                Err(err.into())
            }
        }
        Err(_) => {
            debug!(
                "dashboard consistency barrier timed out before acquiring the write reservation"
            );
            Ok(None)
        }
    }
}

pub(crate) async fn begin_dashboard_activity_consistency_barrier(
    pool: &Pool<Sqlite>,
) -> Result<sqlx::Transaction<'static, Sqlite>, ApiError> {
    Ok(pool.begin_with("BEGIN IMMEDIATE").await?)
}

pub(crate) async fn finish_dashboard_activity_consistency_barrier(
    transaction: sqlx::Transaction<'static, Sqlite>,
    commit: bool,
) -> Result<(), ApiError> {
    if commit {
        transaction.commit().await?;
    } else {
        transaction.rollback().await?;
    }
    Ok(())
}

pub(crate) async fn query_archive_upstream_account_activity_overlapping_live_ids(
    live_pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashSet<i64>, ApiError> {
    let mut overlapping_ids = HashSet::new();
    let mut cursor_id = i64::MIN;
    loop {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM codex_invocations \
             WHERE id > ",
        );
        query
            .push_bind(cursor_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(range.end));
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query
            .push(" ORDER BY id LIMIT ")
            .push_bind(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE as i64);
        let ids = query
            .build_query_scalar::<i64>()
            .fetch_all(archive_pool)
            .await?;
        let Some(last_id) = ids.last().copied() else {
            break;
        };
        cursor_id = last_id;
        overlapping_ids.extend(
            query_live_upstream_account_activity_existing_invocation_ids(
                live_pool,
                source_scope,
                &ids,
            )
            .await?,
        );
        if ids.len() < DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE {
            break;
        }
    }
    Ok(overlapping_ids)
}

pub(crate) async fn query_runtime_recent_account_fallback_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    keys: &HashSet<(String, String)>,
) -> Result<Vec<RuntimeRecentAccountFallbackRow>, ApiError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let keys_json = serde_json::Value::Array(
        keys.iter()
            .map(|(invoke_id, occurred_at)| json!([invoke_id, occurred_at]))
            .collect(),
    )
    .to_string();
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH runtime_keys AS (\
           SELECT CAST(json_extract(value, '$[0]') AS TEXT) AS invoke_id, \
                  CAST(json_extract(value, '$[1]') AS TEXT) AS occurred_at \
             FROM json_each(",
    );
    query
        .push_bind(keys_json)
        .push(
            ")\
         ) \
         SELECT codex_invocations.invoke_id, codex_invocations.occurred_at, ",
        )
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL)
        .push(
            " AS upstream_account_plan_type \
             FROM codex_invocations \
             JOIN runtime_keys \
               ON runtime_keys.invoke_id = codex_invocations.invoke_id \
              AND runtime_keys.occurred_at = codex_invocations.occurred_at \
             WHERE 1 = 1",
        );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query
            .push(" AND codex_invocations.source = ")
            .push_bind(SOURCE_PROXY);
    }

    query
        .build_query_as::<RuntimeRecentAccountFallbackRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn runtime_upstream_account_activity_preview_row(
    record: ApiInvocation,
    source_scope: InvocationSourceScope,
) -> Option<UpstreamAccountInvocationPreviewRow> {
    runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, false)
}

pub(crate) fn runtime_upstream_account_activity_preview_row_with_terminal(
    record: ApiInvocation,
    source_scope: InvocationSourceScope,
    include_terminal: bool,
) -> Option<UpstreamAccountInvocationPreviewRow> {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return None;
    }
    if !include_terminal
        && !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        )
    {
        return None;
    }
    // The runtime snapshot stores TTFT once per invocation, while pool retries can replace the
    // final attempt. Until attempt-owned TTFT exists, suppress that invocation-level measurement
    // for retries so account previews cannot attribute an earlier attempt to the current one.
    let live_phase = runtime_record_live_phase(&record).map(str::to_string);
    let first_token_ms = runtime_record_first_token_ms(&record);
    Some(UpstreamAccountInvocationPreviewRow {
        upstream_account_id: record.upstream_account_id,
        id: record.id,
        invoke_id: record.invoke_id,
        prompt_cache_key: record.prompt_cache_key,
        occurred_at: record.occurred_at,
        conversation_created_at: None,
        status: record.status.unwrap_or_else(|| "running".to_string()),
        live_phase,
        failure_class: record.failure_class,
        route_mode: record.route_mode,
        model: record.model,
        request_model: record.request_model,
        response_model: record.response_model,
        total_tokens: record.total_tokens.unwrap_or_default(),
        cost: record.cost,
        cost_input: record.cost_input,
        cost_cache_write: record.cost_cache_write,
        cost_cache_read: record.cost_cache_read,
        cost_output: record.cost_output,
        cost_reasoning: record.cost_reasoning,
        source: Some(record.source),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        reasoning_effort: record.reasoning_effort,
        error_message: record.error_message,
        downstream_status_code: record.downstream_status_code,
        downstream_error_message: record.downstream_error_message,
        failure_kind: record.failure_kind,
        is_actionable: record.is_actionable.map(|value| if value { 1 } else { 0 }),
        proxy_display_name: record.proxy_display_name,
        upstream_account_name: record.upstream_account_name,
        upstream_account_plan_type: None,
        response_content_encoding: record.response_content_encoding,
        request_compression_algorithm: record.request_compression_algorithm,
        transport: record.transport,
        requested_service_tier: record.requested_service_tier,
        service_tier: record.service_tier,
        billing_service_tier: record.billing_service_tier,
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        first_token_ms,
        t_upstream_stream_ms: finite_positive_timing(record.t_upstream_stream_ms),
        t_resp_parse_ms: record.t_resp_parse_ms,
        t_persist_ms: record.t_persist_ms,
        t_total_ms: record.t_total_ms,
        endpoint: record.endpoint,
        compaction_request_kind: record.compaction_request_kind,
        compaction_response_kind: record.compaction_response_kind,
        image_intent: record.image_intent,
    })
}

pub(crate) fn overlay_runtime_upstream_account_activity_preview_rows(
    state: &AppState,
    rows: &mut Vec<UpstreamAccountInvocationPreviewRow>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) {
    let mut runtime_overlay_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(mut row) = runtime_upstream_account_activity_preview_row(record, source_scope)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at < range.start || occurred_at >= range.end {
            continue;
        }
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows
            .iter_mut()
            .find(|existing| (existing.invoke_id.clone(), existing.occurred_at.clone()) == key)
        {
            if matches!(
                normalized_runtime_text(Some(existing.status.as_str())).as_str(),
                "running" | "pending"
            ) {
                if row.upstream_account_id.is_none() && existing.upstream_account_id.is_some() {
                    row.upstream_account_id = existing.upstream_account_id;
                    row.upstream_account_name = existing.upstream_account_name.clone();
                    row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
                }
                *existing = row;
                runtime_overlay_row_count += 1;
            }
        } else {
            rows.push(row);
            runtime_overlay_row_count += 1;
        }
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count,
            "overlayed memory runtime invocation rows into upstream account activity"
        );
    }
}

pub(crate) async fn overlay_runtime_terminal_upstream_account_activity_preview_rows(
    state: &AppState,
    rows: &mut Vec<UpstreamAccountInvocationPreviewRow>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<(), ApiError> {
    let terminal_rows =
        load_dashboard_activity_runtime_terminal_preview_rows(state, source_scope, range).await?;
    if terminal_rows.is_empty() {
        return Ok(());
    }

    let mut rows_by_key = rows
        .drain(..)
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<HashMap<_, _>>();
    for mut row in terminal_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows_by_key.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = existing.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = existing.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
            }
        }
        rows_by_key.insert(key, row);
    }
    rows.extend(rows_by_key.into_values());
    Ok(())
}

pub(crate) async fn load_dashboard_activity_runtime_terminal_preview_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let mut terminal_rows = Vec::new();
    for record in state.proxy_runtime_invocations.snapshot() {
        if matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let Some(row) =
            runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, true)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at >= range.start && occurred_at < range.end {
            terminal_rows.push(row);
        }
    }
    if terminal_rows.is_empty() {
        return Ok(Vec::new());
    }

    let fallback_keys = terminal_rows
        .iter()
        .filter(|row| row.upstream_account_id.is_none())
        .map(|row| (row.invoke_id.clone(), row.occurred_at.clone()))
        .collect::<HashSet<_>>();
    let fallback_rows =
        query_runtime_recent_account_fallback_rows(&state.pool, source_scope, &fallback_keys)
            .await?
            .into_iter()
            .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
            .collect::<HashMap<_, _>>();

    for row in &mut terminal_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(fallback) = fallback_rows.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = fallback.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = fallback.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = fallback.upstream_account_plan_type.clone();
            }
        }
    }

    Ok(terminal_rows)
}

#[derive(Debug, Default)]
pub(crate) struct UsageBreakdownRowsBuildResult {
    pub(crate) rows: Vec<UpstreamAccountUsageBreakdownAggregateRow>,
    pub(crate) full_hour_bucket_count: usize,
    pub(crate) rollup_row_count: usize,
    pub(crate) partial_hour_row_count: usize,
    pub(crate) archive_batch_count: usize,
    pub(crate) fallback_reason: &'static str,
    pub(crate) legacy_pruned_payload_mode: &'static str,
}

pub(crate) fn usage_breakdown_request_count(
    rows: &[UpstreamAccountUsageBreakdownAggregateRow],
) -> usize {
    rows.iter()
        .map(|row| row.request_count.max(0) as usize)
        .sum::<usize>()
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UsageBreakdownArchiveFallbackRange {
    pub(crate) range: ExactUtcRange,
    pub(crate) skip_replayed_prefix: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct HourlyRollupArchiveProgressRow {
    pub(crate) file_path: String,
    pub(crate) cursor_id: i64,
}

pub(crate) async fn load_hourly_rollup_archive_progress_by_file_path(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_paths: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    if file_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let mut progress = HashMap::new();
    for file_paths in file_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT file_path, cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ",
        );
        query.push_bind(dataset).push(" AND file_path IN (");
        {
            let mut separated = query.separated(", ");
            for file_path in file_paths {
                separated.push_bind(file_path);
            }
        }
        query.push(")");

        progress.extend(
            query
                .build_query_as::<HourlyRollupArchiveProgressRow>()
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| (row.file_path, row.cursor_id.max(0))),
        );
    }
    Ok(progress)
}

pub(crate) async fn load_usage_breakdown_archive_progress_by_file_path(
    pool: &Pool<Sqlite>,
    file_paths: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    load_hourly_rollup_archive_progress_by_file_path(
        pool,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        file_paths,
    )
    .await
}

pub(crate) async fn query_unmaterialized_archive_usage_breakdown_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ranges: &[UsageBreakdownArchiveFallbackRange],
    upstream_account_id: Option<i64>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<
    (
        Vec<UpstreamAccountUsageBreakdownAggregateRow>,
        usize,
        &'static str,
        &'static str,
    ),
    ApiError,
> {
    if ranges.is_empty() {
        return Ok((Vec::new(), 0, "none", "none"));
    }

    let archive_lookup_range = ranges
        .iter()
        .map(|fallback_range| fallback_range.range)
        .reduce(|combined, range| ExactUtcRange {
            start: combined.start.min(range.start),
            end: combined.end.max(range.end),
        });
    let archive_rows = crate::stats::load_invocation_archives_missing_effective_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        archive_lookup_range.map(|range| (range.start, range.end)),
    )
    .await?;
    if archive_rows.is_empty() {
        return Ok((Vec::new(), 0, "none", "none"));
    }

    let archive_progress_by_file_path = load_usage_breakdown_archive_progress_by_file_path(
        pool,
        &archive_rows
            .iter()
            .map(|archive_row| archive_row.file_path().to_string())
            .collect::<Vec<_>>(),
    )
    .await?;
    let mut fallback_reason = "archive_rollup_marker_missing";
    let mut legacy_pruned_payload_mode = "none";
    let mut saw_partial_progress = false;
    let mut rows_by_key =
        HashMap::<UsageBreakdownAggregateMergeKey, UpstreamAccountUsageBreakdownAggregateRow>::new(
        );
    for archive_row in &archive_rows {
        let start_after_id = archive_progress_by_file_path
            .get(archive_row.file_path())
            .copied()
            .unwrap_or(0)
            .max(0);
        saw_partial_progress |= start_after_id > 0;
        let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
            archive_row,
            "usage-breakdown-fallback",
        )
        .await?
        else {
            fallback_reason = "archive_batch_unavailable";
            continue;
        };
        if crate::maintenance::invocation_archive_has_pruned_success_details_in_db(&archive_pool)
            .await?
        {
            legacy_pruned_payload_mode = "structured_rollup_unknown_reasoning";
        }
        let has_cost_breakdown_columns =
            crate::stats::sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input")
                .await?;
        let exclude_filter = prepare_dashboard_activity_excluded_invocation_ids_filter(
            &archive_pool,
            exclude_invocation_ids,
        )
        .await?;
        for fallback_range in ranges {
            let rows = query_upstream_account_usage_breakdown_rows_from_executor(
                &archive_pool,
                source_scope,
                fallback_range.range,
                has_cost_breakdown_columns,
                false,
                exclude_filter,
                (fallback_range.skip_replayed_prefix && start_after_id > 0)
                    .then_some(start_after_id),
                None,
                upstream_account_id,
            )
            .await?;
            for row in &rows {
                merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    if fallback_reason == "archive_rollup_marker_missing" && saw_partial_progress {
        fallback_reason = "archive_rollup_partial_progress";
    }

    Ok((
        rows_by_key.into_values().collect(),
        archive_rows.len(),
        fallback_reason,
        legacy_pruned_payload_mode,
    ))
}

pub(crate) fn build_usage_breakdown_archive_fallback_ranges(
    range: ExactUtcRange,
    range_plan: &HourlyRollupExactRangePlan,
    retention_cutoff: DateTime<Utc>,
) -> Result<Vec<UsageBreakdownArchiveFallbackRange>, ApiError> {
    let mut ranges = Vec::new();
    if let Some(full_hour_range) =
        dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
    {
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            full_hour_range.start,
            full_hour_range.end,
            true,
        )?;
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            range.start,
            full_hour_range.start.min(range.end).min(retention_cutoff),
            false,
        )?;
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            full_hour_range.end.max(range.start),
            range.end.min(retention_cutoff),
            false,
        )?;
    } else {
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            range.start,
            range.end.min(retention_cutoff),
            false,
        )?;
    }
    Ok(ranges)
}

pub(crate) fn push_usage_breakdown_archive_fallback_range(
    ranges: &mut Vec<UsageBreakdownArchiveFallbackRange>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    skip_replayed_prefix: bool,
) -> Result<(), ApiError> {
    if start >= end {
        return Ok(());
    }
    ranges.push(UsageBreakdownArchiveFallbackRange {
        range: ExactUtcRange { start, end },
        skip_replayed_prefix,
    });
    Ok(())
}

pub(crate) async fn load_usage_breakdown_rows_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: UsageBreakdownBuildTelemetry,
) -> Result<UsageBreakdownRowsBuildResult, ApiError> {
    let started_at = Instant::now();
    let has_cost_breakdown_columns =
        crate::stats::sqlite_table_has_column(&state.pool, "codex_invocations", "cost_input")
            .await?;
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let range_plan =
        build_hourly_rollup_exact_range_plan(range.start, range.end, retention_cutoff)?;
    let full_hour_bucket_count = range_plan
        .full_hour_range
        .map(|(start_epoch, end_epoch)| ((end_epoch - start_epoch).max(0) / 3_600) as usize)
        .unwrap_or(0);
    let mut rows_by_key =
        HashMap::<UsageBreakdownAggregateMergeKey, UpstreamAccountUsageBreakdownAggregateRow>::new(
        );
    let mut partial_hour_row_count = 0usize;
    let mut rollup_row_count = 0usize;
    let mut archive_overlap_ids = HashSet::new();

    {
        let mut tx = state.pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;

        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rollup_rows = query_upstream_account_usage_breakdown_hourly_rollup_range_tx(
                tx.as_mut(),
                range_start_epoch,
                range_end_epoch,
                source_scope,
                upstream_account_id,
            )
            .await?;
            rollup_row_count = rollup_rows.len();
            for row in &rollup_rows {
                merge_usage_breakdown_rollup_record_map(&mut rows_by_key, row);
            }
        }

        if snapshot_id > 0 {
            for exact_range in &range_plan.live_exact_ranges {
                let exact_rows = query_live_upstream_account_usage_breakdown_rows_tx(
                    tx.as_mut(),
                    source_scope,
                    *exact_range,
                    has_cost_breakdown_columns,
                    true,
                    DashboardActivityExcludedInvocationIdsFilter::None,
                    None,
                    Some(snapshot_id),
                    upstream_account_id,
                )
                .await?;
                partial_hour_row_count += usage_breakdown_request_count(&exact_rows);
                for row in &exact_rows {
                    merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
                }
            }

            if let Some(full_hour_range) =
                dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
            {
                let tail_rows = query_live_upstream_account_usage_breakdown_rows_tx(
                    tx.as_mut(),
                    source_scope,
                    full_hour_range,
                    has_cost_breakdown_columns,
                    true,
                    DashboardActivityExcludedInvocationIdsFilter::None,
                    Some(rollup_live_cursor),
                    Some(snapshot_id),
                    upstream_account_id,
                )
                .await?;
                partial_hour_row_count += usage_breakdown_request_count(&tail_rows);
                for row in &tail_rows {
                    merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
                }

                archive_overlap_ids = if let Some(upstream_account_id) = upstream_account_id {
                    query_invocation_full_hour_tail_records_tx_for_account(
                        tx.as_mut(),
                        &range_plan,
                        source_scope,
                        rollup_live_cursor,
                        snapshot_id,
                        upstream_account_id,
                    )
                    .await?
                    .into_iter()
                    .map(|record| record.id)
                    .collect()
                } else {
                    query_invocation_full_hour_tail_records_tx(
                        tx.as_mut(),
                        &range_plan,
                        source_scope,
                        rollup_live_cursor,
                        snapshot_id,
                    )
                    .await?
                    .into_iter()
                    .map(|record| record.id)
                    .collect()
                };
            }
        }
    }

    let archive_fallback_ranges =
        build_usage_breakdown_archive_fallback_ranges(range, &range_plan, retention_cutoff)?;
    let (archive_rows, archive_batch_count, fallback_reason, legacy_pruned_payload_mode) =
        if archive_fallback_ranges.is_empty() {
            (Vec::new(), 0, "none", "none")
        } else {
            query_unmaterialized_archive_usage_breakdown_rows(
                &state.pool,
                source_scope,
                &archive_fallback_ranges,
                upstream_account_id,
                (!archive_overlap_ids.is_empty()).then_some(&archive_overlap_ids),
            )
            .await?
        };
    for row in &archive_rows {
        merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
    }

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_usage_breakdown_builder_telemetry(
        telemetry,
        source_scope,
        full_hour_bucket_count,
        rollup_row_count,
        partial_hour_row_count,
        archive_batch_count,
        fallback_reason,
        legacy_pruned_payload_mode,
        elapsed_ms,
    );

    Ok(UsageBreakdownRowsBuildResult {
        rows: rows_by_key.into_values().collect(),
        full_hour_bucket_count,
        rollup_row_count,
        partial_hour_row_count,
        archive_batch_count,
        fallback_reason,
        legacy_pruned_payload_mode,
    })
}

pub(crate) async fn load_usage_breakdown_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<Option<UsageBreakdownResponse>, ApiError> {
    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    let build = load_usage_breakdown_rows_for_range(
        state,
        source_scope,
        upstream_account_id,
        range,
        UsageBreakdownBuildTelemetry::from_summary(telemetry),
    )
    .await?;
    for row in &build.rows {
        usage_breakdown.add_aggregate_row(row);
    }
    Ok(Some(usage_breakdown.into_response()))
}

pub(crate) async fn query_upstream_account_activity_meta(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, UpstreamAccountActivityMetaRow>, ApiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, kind, display_name, group_name, plan_type, status, enabled, \
            last_error, last_error_at, last_route_failure_at, last_route_failure_kind, \
            last_action_reason_code, last_action_reason_message, cooldown_until, \
            temporary_route_failure_streak_started_at, last_selected_at \
         FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(*account_id);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<UpstreamAccountActivityMetaRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect())
}

pub(crate) async fn query_upstream_account_in_progress_counts(
    state: &AppState,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    query_upstream_account_in_progress_counts_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        source_scope,
    )
    .await
}

pub(crate) async fn query_upstream_account_in_progress_counts_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    let baseline = query_dashboard_runtime_projection_baseline(pool, source_scope).await?;
    query_upstream_account_in_progress_counts_with_baseline(
        pool,
        proxy_runtime_invocations,
        source_scope,
        &baseline,
    )
    .await
}

pub(crate) async fn query_dashboard_runtime_projection_baseline(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<DashboardRuntimeProjectionBaseline, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: Option<i64>,
        retry_count: i64,
        upstream_ttfb_ms: Option<f64>,
        live_phase: Option<String>,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = invocation_account_retry_after_failure_with_attempt_fallback_sql(
        resolved_upstream_account_id_sql.as_str(),
        source_scope,
    );
    let final_first_token_timing_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
    let final_live_phase_sql =
        invocation_live_phase_sql_with_timing_sql("inv", final_first_token_timing_sql.as_str());
    let mut db_key_query = QueryBuilder::<Sqlite>::new(
        "SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ",
    );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(" AS retry_count, live.upstream_ttfb_ms AS upstream_ttfb_ms, ");
    db_key_query.push(final_live_phase_sql.as_str()).push(
        " AS live_phase \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    let db_rows = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(pool)
        .await?;
    Ok(DashboardRuntimeProjectionBaseline {
        records: db_rows
            .into_iter()
            .map(|row| DashboardRuntimeBaselineRecord {
                key: RuntimeInvocationKey::new(row.invoke_id, row.occurred_at),
                upstream_account_id: row.upstream_account_id,
                upstream_account_name: None,
                is_retry: row.retry_count > 0,
                live_phase: row.live_phase,
                wait_ms: row.upstream_ttfb_ms,
            })
            .collect(),
        source_scope,
        network_open_buckets: HashMap::new(),
    })
}

pub(crate) async fn query_upstream_account_in_progress_counts_with_baseline(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    source_scope: InvocationSourceScope,
    baseline: &DashboardRuntimeProjectionBaseline,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    let mut counts = HashMap::<Option<i64>, UpstreamAccountInProgressSummary>::new();
    for row in &baseline.records {
        counts.entry(row.upstream_account_id).or_default().add(
            row.is_retry,
            row.live_phase.as_deref(),
            row.wait_ms,
        );
    }
    let db_runtime_keys = baseline
        .records
        .iter()
        .map(|row| {
            (
                (row.key.invoke_id.clone(), row.key.occurred_at.clone()),
                (
                    row.upstream_account_id,
                    row.is_retry,
                    row.live_phase.clone(),
                    row.wait_ms,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = proxy_runtime_invocations.snapshot();
    let db_terminal_keys =
        query_terminal_db_keys_for_runtime_records(pool, &runtime_snapshot, None).await?;
    let mut runtime_overlay_row_count = 0_i64;
    for record in runtime_snapshot {
        if db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        let runtime_phase = runtime_record_live_phase(&record);
        if let Some((db_upstream_account_id, db_is_retry, db_phase, db_wait_ms)) =
            db_runtime_keys.get(&key)
        {
            let runtime_is_retry = runtime_record_is_retry(&record);
            let upstream_account_id = record.upstream_account_id.or(*db_upstream_account_id);
            if *db_upstream_account_id != upstream_account_id {
                if let Some(entry) = counts.get_mut(db_upstream_account_id) {
                    entry.subtract(*db_is_retry, db_phase.as_deref(), *db_wait_ms);
                }
                counts.entry(upstream_account_id).or_default().add(
                    runtime_is_retry,
                    runtime_phase,
                    record.t_upstream_ttfb_ms,
                );
                runtime_overlay_row_count += 1;
            } else {
                let entry = counts.entry(upstream_account_id).or_default();
                if runtime_is_retry && !*db_is_retry {
                    entry.retry_count += 1;
                }
                if runtime_phase != db_phase.as_deref() {
                    entry.phase_counts.decrement_phase_name(db_phase.as_deref());
                    entry.phase_counts.increment_phase_name(runtime_phase);
                    runtime_overlay_row_count += 1;
                }
                if runtime_record_ttfb_differs(record.t_upstream_ttfb_ms, *db_wait_ms) {
                    entry.wait_sum_ms = (entry.wait_sum_ms
                        - normalized_wait_ms(*db_wait_ms).unwrap_or_default())
                    .max(0.0)
                        + normalized_wait_ms(record.t_upstream_ttfb_ms).unwrap_or_default();
                    entry.wait_sample_count = entry
                        .wait_sample_count
                        .saturating_sub(i64::from(normalized_wait_ms(*db_wait_ms).is_some()))
                        + i64::from(normalized_wait_ms(record.t_upstream_ttfb_ms).is_some());
                    runtime_overlay_row_count += 1;
                }
            }
            continue;
        }
        counts.entry(record.upstream_account_id).or_default().add(
            runtime_record_is_retry(&record),
            runtime_phase,
            record.t_upstream_ttfb_ms,
        );
        runtime_overlay_row_count += 1;
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count, "overlayed memory runtime account in-progress counts"
        );
    }
    Ok(counts)
}

pub(crate) async fn query_dashboard_activity_live_snapshot(
    state: &AppState,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    query_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        revision,
    )
    .await
}

pub(crate) async fn query_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
        pool,
        proxy_runtime_invocations,
        dashboard_network_speed_cache,
        revision,
    )
    .await
    .map(|(snapshot, _)| snapshot)
}

pub(crate) async fn query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    revision: u64,
) -> Result<
    (
        DashboardActivityLiveSnapshot,
        DashboardRuntimeProjectionBaseline,
    ),
    ApiError,
> {
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;
    flush_dashboard_network_socket_minute_rollups(pool, dashboard_network_speed_cache, now).await?;
    let mut baseline = query_dashboard_runtime_projection_baseline(pool, source_scope).await?;
    let counts = query_upstream_account_in_progress_counts_with_baseline(
        pool,
        proxy_runtime_invocations,
        source_scope,
        &baseline,
    )
    .await?;
    let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
    let global_realtime_rate = build_dashboard_network_realtime_rate_response(
        dashboard_network_speed_cache
            .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
    );
    let mut live_bucket_account_ids = counts.keys().copied().collect::<Vec<_>>();
    for upstream_account_id in account_rates.keys() {
        if !live_bucket_account_ids.contains(upstream_account_id) {
            live_bucket_account_ids.push(*upstream_account_id);
        }
    }
    let mut live_buckets = HashMap::new();
    for upstream_account_id in live_bucket_account_ids {
        live_buckets.insert(
            upstream_account_id,
            load_dashboard_network_live_bucket_point(
                pool,
                dashboard_network_speed_cache,
                source_scope,
                now,
                match upstream_account_id {
                    Some(id) => DashboardNetworkScopeKey::Account(id),
                    None => DashboardNetworkScopeKey::Unassigned,
                },
            )
            .await?,
        );
    }
    let mut accounts = counts
        .into_iter()
        .map(|(upstream_account_id, summary)| {
            let rate = account_rates
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            DashboardActivityLiveAccount {
                account_key: upstream_account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                upstream_account_id,
                upstream_account_name: None,
                in_progress_invocation_count: summary.in_progress_count,
                in_progress_phase_counts: summary.phase_counts,
                retry_invocation_count: summary.retry_count,
                in_progress_wait_sum_ms: summary.wait_sum_ms,
                in_progress_wait_sample_count: summary.wait_sample_count,
                upload_bytes_per_second: rate.upload_bytes_per_second,
                download_bytes_per_second: rate.download_bytes_per_second,
                network_live_bucket: live_buckets.get(&upstream_account_id).cloned(),
            }
        })
        .collect::<Vec<_>>();
    let existing_account_keys = accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    for (upstream_account_id, rate) in account_rates {
        let account_key = upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string());
        if existing_account_keys.contains(&account_key) {
            continue;
        }
        accounts.push(DashboardActivityLiveAccount {
            account_key,
            upstream_account_id,
            upstream_account_name: None,
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            upload_bytes_per_second: rate.upload_bytes_per_second,
            download_bytes_per_second: rate.download_bytes_per_second,
            network_live_bucket: live_buckets.get(&upstream_account_id).cloned(),
        });
    }
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));

    let global_live_bucket = load_dashboard_network_live_bucket_point(
        pool,
        dashboard_network_speed_cache,
        source_scope,
        now,
        DashboardNetworkScopeKey::Global,
    )
    .await?;
    let mut network_open_buckets = HashMap::new();
    for (upstream_account_id, live_bucket) in &live_buckets {
        let scope = DashboardNetworkScopeKey::account_scope(*upstream_account_id);
        let memory = dashboard_network_speed_cache.snapshot_open_bucket(scope, now);
        network_open_buckets.insert(
            scope,
            DashboardRuntimeNetworkOpenBucketBaseline {
                bucket_start: memory.bucket_start,
                bucket_end: memory.bucket_end,
                baseline_totals: DashboardNetworkByteTotals {
                    upload_bytes: live_bucket.upload_bytes,
                    download_bytes: live_bucket.download_bytes,
                },
                memory_totals_at_install: memory.totals,
            },
        );
    }
    let global_memory =
        dashboard_network_speed_cache.snapshot_open_bucket(DashboardNetworkScopeKey::Global, now);
    network_open_buckets.insert(
        DashboardNetworkScopeKey::Global,
        DashboardRuntimeNetworkOpenBucketBaseline {
            bucket_start: global_memory.bucket_start,
            bucket_end: global_memory.bucket_end,
            baseline_totals: DashboardNetworkByteTotals {
                upload_bytes: global_live_bucket.upload_bytes,
                download_bytes: global_live_bucket.download_bytes,
            },
            memory_totals_at_install: global_memory.totals,
        },
    );
    baseline.network_open_buckets = network_open_buckets;

    let mut in_progress_phase_counts = InvocationPhaseCountsResponse::default();
    let mut in_progress_invocation_count = 0;
    let mut retry_invocation_count = 0;
    let mut in_progress_wait_sum_ms = 0.0;
    let mut in_progress_wait_sample_count = 0;
    for account in &accounts {
        in_progress_invocation_count += account.in_progress_invocation_count;
        retry_invocation_count += account.retry_invocation_count;
        in_progress_wait_sum_ms += account.in_progress_wait_sum_ms;
        in_progress_wait_sample_count += account.in_progress_wait_sample_count;
        in_progress_phase_counts.queued += account.in_progress_phase_counts.queued;
        in_progress_phase_counts.requesting += account.in_progress_phase_counts.requesting;
        in_progress_phase_counts.responding += account.in_progress_phase_counts.responding;
    }

    Ok((
        DashboardActivityLiveSnapshot {
            revision,
            generated_at: format_utc_iso(now),
            in_progress_invocation_count,
            in_progress_phase_counts,
            retry_invocation_count,
            in_progress_wait_sum_ms,
            in_progress_wait_sample_count,
            network_live_bucket: Some(global_live_bucket),
            network_realtime_rate: Some(global_realtime_rate),
            accounts,
        },
        baseline,
    ))
}

pub(crate) fn dashboard_live_snapshot_in_progress_counts(
    snapshot: &DashboardActivityLiveSnapshot,
) -> HashMap<Option<i64>, UpstreamAccountInProgressSummary> {
    snapshot
        .accounts
        .iter()
        .map(|account| {
            (
                account.upstream_account_id,
                UpstreamAccountInProgressSummary {
                    in_progress_count: account.in_progress_invocation_count,
                    retry_count: account.retry_invocation_count,
                    phase_counts: account.in_progress_phase_counts,
                    wait_sum_ms: account.in_progress_wait_sum_ms,
                    wait_sample_count: account.in_progress_wait_sample_count,
                },
            )
        })
        .collect()
}
