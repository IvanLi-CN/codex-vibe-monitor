async fn query_live_upstream_account_activity_preview_candidate_ids_per_account(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    max_candidate_ids: Option<usize>,
) -> Result<Vec<i64>, ApiError> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH ranked AS (\
           SELECT id \
             FROM (\
               SELECT id, ROW_NUMBER() OVER (PARTITION BY ",
    );
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(
            " ORDER BY occurred_at DESC, id DESC) AS account_rank \
               FROM codex_invocations \
               WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(") WHERE account_rank <= ")
        .push_bind(limit_per_account as i64)
        .push(") SELECT id FROM ranked");
    if let Some(max_candidate_ids) = max_candidate_ids {
        query
            .push(" LIMIT ")
            .push_bind(max_candidate_ids.saturating_add(1) as i64);
    }
    Ok(query.build_query_scalar::<i64>().fetch_all(pool).await?)
}

async fn query_live_upstream_account_activity_preview_rows_by_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_live_upstream_account_activity_preview_rows_by_ids_connection(
        &mut connection,
        source_scope,
        ids,
        telemetry,
    )
    .await
}

async fn query_live_upstream_account_activity_preview_rows_by_ids_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let mut rows = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        build_upstream_account_activity_preview_select(&mut query, source_scope);
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        query.push(" ORDER BY occurred_at DESC, id DESC");

        rows.extend(
            query
                .build_query_as::<UpstreamAccountInvocationPreviewRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            candidate_preview_id_count = ids.len(),
            hydrated_preview_row_count = rows.len(),
            ?source_scope,
            selected_preview_row_count = ids.len(),
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity preview hydration"
        );
    }
    Ok(rows)
}

struct HydratedUpstreamAccountPreviewRows {
    rows: Vec<UpstreamAccountInvocationPreviewRow>,
    candidate_preview_id_count: usize,
    hydrated_preview_row_count: usize,
}

async fn query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
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

async fn query_live_upstream_account_activity_preview_rows_per_account_limit(
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

async fn query_live_upstream_account_activity_existing_invocation_ids(
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

async fn query_live_dashboard_activity_persisted_terminal_ids_tx(
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

async fn query_live_dashboard_activity_snapshot_cursor_tx(
    connection: &mut SqliteConnection,
) -> Result<i64, ApiError> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&mut *connection)
            .await?,
    )
}

async fn query_live_dashboard_activity_reconcile_probe(
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

async fn query_live_dashboard_activity_snapshot_cursor(
    pool: &Pool<Sqlite>,
) -> Result<i64, ApiError> {
    let mut connection = pool.acquire().await?;
    query_live_dashboard_activity_snapshot_cursor_tx(&mut connection).await
}

async fn try_begin_dashboard_activity_consistency_barrier(
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

async fn begin_dashboard_activity_consistency_barrier(
    pool: &Pool<Sqlite>,
) -> Result<sqlx::Transaction<'static, Sqlite>, ApiError> {
    Ok(pool.begin_with("BEGIN IMMEDIATE").await?)
}

async fn finish_dashboard_activity_consistency_barrier(
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

async fn query_archive_upstream_account_activity_overlapping_live_ids(
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

async fn query_runtime_recent_account_fallback_rows(
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

fn runtime_upstream_account_activity_preview_row_with_terminal(
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

async fn overlay_runtime_terminal_upstream_account_activity_preview_rows(
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

async fn load_dashboard_activity_runtime_terminal_preview_rows(
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
struct UsageBreakdownRowsBuildResult {
    rows: Vec<UpstreamAccountUsageBreakdownAggregateRow>,
    full_hour_bucket_count: usize,
    rollup_row_count: usize,
    partial_hour_row_count: usize,
    archive_batch_count: usize,
    fallback_reason: &'static str,
    legacy_pruned_payload_mode: &'static str,
}

fn usage_breakdown_request_count(rows: &[UpstreamAccountUsageBreakdownAggregateRow]) -> usize {
    rows.iter()
        .map(|row| row.request_count.max(0) as usize)
        .sum::<usize>()
}

#[derive(Debug, Clone, Copy)]
struct UsageBreakdownArchiveFallbackRange {
    range: ExactUtcRange,
    skip_replayed_prefix: bool,
}

#[derive(Debug, FromRow)]
struct HourlyRollupArchiveProgressRow {
    file_path: String,
    cursor_id: i64,
}

async fn load_hourly_rollup_archive_progress_by_file_path(
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

async fn load_usage_breakdown_archive_progress_by_file_path(
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
