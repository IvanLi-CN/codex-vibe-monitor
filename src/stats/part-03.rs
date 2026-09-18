pub(crate) async fn open_invocation_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    open_archive_batch_pool(archive_row, "codex_invocations", read_surface).await
}

pub(crate) async fn open_pool_upstream_request_attempt_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    open_archive_batch_pool(archive_row, "pool_upstream_request_attempts", read_surface).await
}

async fn open_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    expected_table: &str,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    let archive_path = PathBuf::from(&archive_row.file_path);
    let is_materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
    if !archive_path.exists() {
        warn!(
            file_path = archive_row.file_path,
            read_surface,
            historical_rollups_materialized = is_materialized_archive,
            "skipping missing archive while serving read-only historical fallback"
        );
        return Ok(None);
    }

    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    if let Err(err) = inflate_gzip_sqlite_file(&archive_path, &temp_path) {
        drop(temp_cleanup);
        if is_unreadable_invocation_summary_archive_error(&err) {
            warn!(
                file_path = archive_row.file_path,
                read_surface,
                error = %err,
                historical_rollups_materialized = is_materialized_archive,
                "skipping unreadable archive while serving read-only historical fallback"
            );
            return Ok(None);
        }
        return Err(err);
    }
    let archive_pool = match SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))
    {
        Ok(pool) => pool,
        Err(err) => {
            drop(temp_cleanup);
            if is_unreadable_invocation_summary_archive_error(&err) {
                warn!(
                    file_path = archive_row.file_path,
                    read_surface,
                    error = %err,
                    historical_rollups_materialized = is_materialized_archive,
                    "skipping unreadable archive while serving read-only historical fallback"
                );
                return Ok(None);
            }
            return Err(err);
        }
    };
    let has_expected_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(expected_table)
    .fetch_one(&archive_pool)
    .await?
        != 0;
    if !has_expected_table {
        archive_pool.close().await;
        drop(temp_cleanup);
        return Ok(None);
    }
    if expected_table == "codex_invocations" {
        ensure_invocation_archive_first_token_compatibility(&archive_pool).await?;
    }

    Ok(Some((archive_pool, temp_cleanup)))
}

#[derive(Debug, FromRow)]
struct ArchivedPoolRequestCompressionRow {
    invoke_id: String,
    occurred_at: String,
    attempt_index: i64,
    id: i64,
    request_compression_algorithm: Option<String>,
}

#[derive(Debug)]
struct ArchivedPoolRequestCompression {
    attempt_index: i64,
    id: i64,
    algorithm: Option<String>,
}

type ArchivedPoolRequestCompressionMap = HashMap<(String, String), ArchivedPoolRequestCompression>;

fn merge_archived_pool_request_compression(
    compression_by_invocation: &mut ArchivedPoolRequestCompressionMap,
    key: (String, String),
    candidate: ArchivedPoolRequestCompression,
) {
    match compression_by_invocation.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
        std::collections::hash_map::Entry::Occupied(mut entry)
            if (candidate.attempt_index, candidate.id)
                > (entry.get().attempt_index, entry.get().id) =>
        {
            entry.insert(candidate);
        }
        std::collections::hash_map::Entry::Occupied(_) => {}
    }
}

async fn load_archived_pool_request_compressions_from_executor<'e, E>(
    executor: E,
    range: ExactUtcRange,
) -> Result<ArchivedPoolRequestCompressionMap>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let rows = sqlx::query_as::<_, ArchivedPoolRequestCompressionRow>(
        r#"
        SELECT
            invoke_id,
            occurred_at,
            attempt_index,
            id,
            NULLIF(TRIM(upstream_request_compression_algorithm), '') AS request_compression_algorithm
        FROM pool_upstream_request_attempts
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
          AND LOWER(TRIM(COALESCE(status, ''))) <> 'budget_exhausted_final'
        ORDER BY invoke_id ASC, occurred_at ASC, attempt_index DESC, id DESC
        "#,
    )
    .bind(db_occurred_at_lower_bound(range.start))
    .bind(db_occurred_at_upper_bound(range.end))
    .fetch_all(executor)
    .await?;

    let mut compression_by_invocation = HashMap::new();
    for row in rows {
        let key = (row.invoke_id, row.occurred_at);
        let candidate = ArchivedPoolRequestCompression {
            attempt_index: row.attempt_index,
            id: row.id,
            algorithm: row.request_compression_algorithm,
        };
        merge_archived_pool_request_compression(&mut compression_by_invocation, key, candidate);
    }
    Ok(compression_by_invocation)
}

async fn load_archived_pool_request_compressions(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
) -> Result<ArchivedPoolRequestCompressionMap> {
    let archive_rows = load_completed_archive_paths_for_dataset_in_range(
        pool,
        "pool_upstream_request_attempts",
        Some((range.start, range.end)),
    )
    .await?;
    let mut compression_by_invocation = HashMap::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) = open_archive_batch_pool(
            &archive_row,
            "pool_upstream_request_attempts",
            "upstream-account-activity-pool-attempts",
        )
        .await?
        else {
            continue;
        };
        if sqlite_table_has_column(
            &archive_pool,
            "pool_upstream_request_attempts",
            "upstream_request_compression_algorithm",
        )
        .await?
        {
            for (key, compression) in
                load_archived_pool_request_compressions_from_executor(&archive_pool, range).await?
            {
                merge_archived_pool_request_compression(
                    &mut compression_by_invocation,
                    key,
                    compression,
                );
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(compression_by_invocation)
}

async fn ensure_invocation_archive_first_token_compatibility(pool: &Pool<Sqlite>) -> Result<()> {
    if !sqlite_table_has_column(pool, "codex_invocations", "first_token_ms").await? {
        sqlx::query("ALTER TABLE codex_invocations ADD COLUMN first_token_ms REAL")
            .execute(pool)
            .await
            .context("failed to add nullable TTFT compatibility column to archive copy")?;
    }
    Ok(())
}

#[cfg(test)]
mod ttft_archive_compatibility_tests {
    use super::*;

    #[tokio::test]
    async fn legacy_archive_rows_expose_null_ttft_after_compatibility_upgrade() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open sqlite");
        sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create legacy invocation table");
        sqlx::query("INSERT INTO codex_invocations (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("insert legacy row");

        ensure_invocation_archive_first_token_compatibility(&pool)
            .await
            .expect("upgrade temporary archive copy");

        let first_token_ms =
            sqlx::query_scalar::<_, Option<f64>>("SELECT first_token_ms FROM codex_invocations")
                .fetch_one(&pool)
                .await
                .expect("read nullable TTFT");
        assert_eq!(first_token_ms, None);
    }

    #[tokio::test]
    async fn archived_pool_request_compression_uses_the_final_attempt() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open sqlite");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, status TEXT, upstream_request_compression_algorithm TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create archived attempts table");

        let range = ExactUtcRange {
            start: Utc::now() - ChronoDuration::minutes(1),
            end: Utc::now() + ChronoDuration::minutes(1),
        };
        let occurred_at = db_occurred_at_lower_bound(Utc::now());
        let prior_occurred_at =
            db_occurred_at_lower_bound(Utc::now() - ChronoDuration::seconds(30));
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, status, upstream_request_compression_algorithm) VALUES (1, 'retry', ?1, 1, 'success', 'br'), (2, 'retry', ?1, 2, 'success', 'zstd'), (3, 'retry', ?2, 1, 'success', 'gzip'), (4, 'final-unknown', ?1, 1, 'success', 'gzip'), (5, 'final-unknown', ?1, 2, 'http_failure', NULL), (6, 'retry', ?1, 3, 'budget_exhausted_final', NULL)",
        )
        .bind(&occurred_at)
        .bind(&prior_occurred_at)
        .execute(&pool)
        .await
        .expect("insert archived attempts");

        let mut compression_by_invocation =
            load_archived_pool_request_compressions_from_executor(&pool, range)
                .await
                .expect("read archived request compression");

        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), occurred_at.clone()))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("zstd"))
        );
        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), prior_occurred_at))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("gzip"))
        );
        assert_eq!(
            compression_by_invocation
                .get(&("final-unknown".to_string(), occurred_at.clone()))
                .map(|compression| compression.algorithm.as_deref()),
            Some(None)
        );

        merge_archived_pool_request_compression(
            &mut compression_by_invocation,
            ("retry".to_string(), occurred_at.clone()),
            ArchivedPoolRequestCompression {
                attempt_index: 3,
                id: 6,
                algorithm: Some("deflate".to_string()),
            },
        );
        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), occurred_at))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("deflate"))
        );
    }
}

fn upstream_account_invocation_preview_select_sql(
    source_scope: InvocationSourceScope,
    has_cost_breakdown_columns: bool,
    has_first_token_column: bool,
) -> String {
    let conversation_created_at_sql = crate::api::invocation_history_conversation_created_at_sql(
        crate::api::INVOCATION_PROMPT_CACHE_KEY_SQL,
        source_scope,
    );
    let cost_breakdown_columns = if has_cost_breakdown_columns {
        "cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning"
    } else {
        "NULL AS cost_input, NULL AS cost_cache_write, NULL AS cost_cache_read, NULL AS cost_output, NULL AS cost_reasoning"
    };
    let first_token_ms = if has_first_token_column {
        "first_token_ms"
    } else {
        "NULL AS first_token_ms"
    };
    format!(
        "id, invoke_id, {} AS prompt_cache_key, occurred_at, {} AS conversation_created_at, {} AS status, NULL AS live_phase, {} AS failure_class, {} AS route_mode, model, {} AS request_model, {} AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, {}, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, {} AS reasoning_effort, error_message, {} AS failure_kind, CASE WHEN {} = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, {} AS proxy_display_name, {} AS upstream_account_id, NULL AS upstream_account_name, NULL AS upstream_account_plan_type, {} AS response_content_encoding, {} AS request_compression_algorithm, {} AS transport, {} AS compaction_request_kind, {} AS compaction_response_kind, {} AS image_intent, CASE WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' THEN json_extract(payload, '$.requestedServiceTier') WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, CASE WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' THEN json_extract(payload, '$.serviceTier') WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' THEN json_extract(payload, '$.service_tier') END AS service_tier, {} AS billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, {}, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, {} AS downstream_status_code, {} AS downstream_error_message, {} AS endpoint",
        crate::api::INVOCATION_PROMPT_CACHE_KEY_SQL,
        conversation_created_at_sql,
        crate::api::invocation_display_status_sql(),
        crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        crate::api::INVOCATION_ROUTE_MODE_SQL,
        crate::api::INVOCATION_REQUEST_MODEL_SQL,
        crate::api::INVOCATION_RESPONSE_MODEL_SQL,
        cost_breakdown_columns,
        crate::api::INVOCATION_REASONING_EFFORT_SQL,
        crate::api::INVOCATION_FAILURE_KIND_SQL,
        crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        crate::api::INVOCATION_PROXY_DISPLAY_SQL,
        crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL,
        crate::api::INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        crate::api::INVOCATION_REQUEST_COMPRESSION_ALGORITHM_SQL,
        crate::api::INVOCATION_TRANSPORT_SQL,
        crate::api::INVOCATION_COMPACTION_REQUEST_KIND_SQL,
        crate::api::INVOCATION_COMPACTION_RESPONSE_KIND_SQL,
        crate::api::INVOCATION_IMAGE_INTENT_SQL,
        crate::api::INVOCATION_BILLING_SERVICE_TIER_SQL,
        first_token_ms,
        crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL,
        crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL,
        crate::api::INVOCATION_ENDPOINT_SQL,
    )
}

pub(crate) async fn query_upstream_account_invocation_preview_rows_from_executor<'e, E>(
    executor: E,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    has_cost_breakdown_columns: bool,
    has_first_token_column: bool,
) -> Result<Vec<crate::api::UpstreamAccountInvocationPreviewRow>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_invocation_preview_select_sql(
            source_scope,
            has_cost_breakdown_columns,
            has_first_token_column,
        ))
        .push(" FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at DESC, id DESC");

    query
        .build_query_as::<crate::api::UpstreamAccountInvocationPreviewRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_completed_invocation_archive_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<Vec<crate::api::UpstreamAccountInvocationPreviewRow>> {
    let archive_rows =
        load_completed_invocation_archive_paths_in_range(pool, Some((range.start, range.end)))
            .await?;
    let archived_pool_request_compressions =
        load_archived_pool_request_compressions(pool, range).await?;
    let mut previews = Vec::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "upstream-account-activity").await?
        else {
            continue;
        };
        let has_cost_breakdown_columns =
            sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input").await?;
        let has_first_token_column =
            sqlite_table_has_column(&archive_pool, "codex_invocations", "first_token_ms").await?;
        let mut rows = query_upstream_account_invocation_preview_rows_from_executor(
            &archive_pool,
            range,
            source_scope,
            has_cost_breakdown_columns,
            has_first_token_column,
        )
        .await?;
        for row in &mut rows {
            if let Some(compression) = archived_pool_request_compressions
                .get(&(row.invoke_id.clone(), row.occurred_at.clone()))
            {
                row.request_compression_algorithm = compression.algorithm.clone();
            }
        }
        let rows = rows
            .into_iter()
            .filter(|row| {
                !exclude_invocation_ids.is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
            })
            .collect::<Vec<_>>();
        previews.extend(rows);
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(previews)
}
