pub(crate) async fn load_invocation_archives_missing_effective_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let archive_rows =
        load_invocation_archives_missing_rollup_target(executor, target, range).await?;
    if !account_archive_target_treats_materialized_batch_as_replayed(target) {
        return Ok(archive_rows);
    }
    Ok(archive_rows
        .into_iter()
        .filter(|archive_row| archive_row.historical_rollups_materialized_at.is_none())
        .collect())
}

pub(crate) async fn load_invocation_archives_missing_summary_rollup_markers(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            batches.file_path,
            batches.month_key,
            batches.coverage_start_at,
            batches.coverage_end_at,
            batches.historical_rollups_materialized_at,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_overall,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND (
            NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
            OR NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?3
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

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
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, invoke_id, ");
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
    query
        .push(crate::api::INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key, occurred_at, ")
        .push(conversation_created_at_sql.as_str())
        .push(" AS conversation_created_at, ")
        .push(crate::api::invocation_display_status_sql())
        .push(" AS status, ")
        .push("NULL AS live_phase, ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(crate::api::INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(crate::api::INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(crate::api::INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, ")
        .push(cost_breakdown_columns)
        .push(", source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(crate::api::INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(crate::api::INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(crate::api::INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push("NULL AS upstream_account_name, NULL AS upstream_account_plan_type, ")
        .push(crate::api::INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(" AS response_content_encoding, ")
        .push(crate::api::INVOCATION_REQUEST_COMPRESSION_ALGORITHM_SQL)
        .push(" AS request_compression_algorithm, ")
        .push(crate::api::INVOCATION_TRANSPORT_SQL)
        .push(" AS transport, ")
        .push(crate::api::INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(" AS compaction_request_kind, ")
        .push(crate::api::INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(" AS compaction_response_kind, ")
        .push(crate::api::INVOCATION_IMAGE_INTENT_SQL)
        .push(
            " AS image_intent, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
                 THEN json_extract(payload, '$.requestedServiceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
                 THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
                 THEN json_extract(payload, '$.serviceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
                 THEN json_extract(payload, '$.service_tier') END AS service_tier, \
             ",
        )
        .push(crate::api::INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(" AS billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, ")
        .push(first_token_ms)
        .push(", t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(crate::api::INVOCATION_ENDPOINT_SQL)
        .push(
            " AS endpoint \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct MaterializedBucketRow {
    bucket_start_epoch: i64,
    source: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ReplayedInvocationArchiveRow {
    file_path: String,
    month_key: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) async fn load_materialized_rollup_bucket_sources(
    pool: &Pool<Sqlite>,
    target: &str,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_sources.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum materialized bucket epoch"))?;
    let max_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum materialized bucket epoch"))?;

    let rows = sqlx::query_as::<_, MaterializedBucketRow>(
        r#"
        SELECT bucket_start_epoch, source
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1
          AND bucket_start_epoch >= ?2
          AND bucket_start_epoch <= ?3
        "#,
    )
    .bind(target)
    .bind(min_bucket_epoch)
    .bind(max_bucket_epoch)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.bucket_start_epoch, row.source))
        .filter(|key| bucket_sources.contains(key))
        .collect())
}

pub(crate) fn shanghai_month_keys_for_bucket_starts(
    bucket_start_epochs: impl IntoIterator<Item = i64>,
) -> HashSet<String> {
    bucket_start_epochs
        .into_iter()
        .filter_map(|bucket_start_epoch| {
            Utc.timestamp_opt(bucket_start_epoch, 0)
                .single()
                .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
        })
        .collect()
}

pub(crate) fn shanghai_month_key_for_bucket_start(bucket_start_epoch: i64) -> Option<String> {
    Utc.timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

pub(crate) fn shanghai_month_bucket_start_epochs(month_key: &str) -> Result<HashSet<i64>> {
    let month_start = NaiveDate::parse_from_str(&format!("{month_key}-01"), "%Y-%m-%d")
        .with_context(|| format!("invalid archive month key: {month_key}"))?;
    let next_month_start = if month_start.month() == 12 {
        NaiveDate::from_ymd_opt(month_start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(month_start.year(), month_start.month() + 1, 1)
    }
    .ok_or_else(|| anyhow!("failed to resolve next month start for archive month {month_key}"))?;
    let month_start_local = month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve local month start for archive month {month_key}")
    })?;
    let next_month_start_local = next_month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve next local month start for archive month {month_key}")
    })?;
    let start_epoch = Shanghai
        .from_local_datetime(&month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();
    let end_epoch_exclusive = Shanghai
        .from_local_datetime(&next_month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize next archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();

    let mut bucket_start_epochs = HashSet::new();
    let mut current_epoch = align_bucket_epoch(start_epoch, 3_600, 0);
    while current_epoch < end_epoch_exclusive {
        bucket_start_epochs.insert(current_epoch);
        current_epoch += 3_600;
    }
    Ok(bucket_start_epochs)
}

pub(crate) fn archive_bucket_start_epochs_from_bounds(
    month_key: Option<&str>,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<HashSet<i64>> {
    if let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at) {
        let coverage_start_epoch = summary_rollup_bucket_start_epoch(coverage_start_at)?;
        let coverage_end_epoch = summary_rollup_bucket_start_epoch(coverage_end_at)?;
        if coverage_end_epoch < coverage_start_epoch {
            return Ok(HashSet::new());
        }

        let mut bucket_start_epochs = HashSet::new();
        let mut current_epoch = coverage_start_epoch;
        while current_epoch <= coverage_end_epoch {
            bucket_start_epochs.insert(current_epoch);
            current_epoch += 3_600;
        }
        return Ok(bucket_start_epochs);
    }

    month_key
        .map(shanghai_month_bucket_start_epochs)
        .transpose()
        .map(|maybe_buckets| maybe_buckets.unwrap_or_default())
}

pub(crate) fn archive_bucket_start_epochs_for_row(
    archive_row: &ArchiveBatchPathRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        archive_row.month_key.as_deref(),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) fn replayed_archive_bucket_start_epochs(
    archive_row: &ReplayedInvocationArchiveRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        Some(archive_row.month_key.as_str()),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) async fn load_replayed_invocation_archives_for_month_keys(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    month_keys: &HashSet<String>,
) -> Result<Vec<ReplayedInvocationArchiveRow>> {
    if month_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut month_keys = month_keys.iter().cloned().collect::<Vec<_>>();
    month_keys.sort();

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT batches.file_path, batches.month_key
             , batches.coverage_start_at, batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
         AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
         AND EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target =
        "#,
    );
    query.push_bind(target);
    query.push(
        r#"
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
         )
         AND batches.month_key IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for month_key in month_keys {
            separated.push_bind(month_key);
        }
    }
    query.push(") ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");

    query
        .build_query_as::<ReplayedInvocationArchiveRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) fn materialized_archive_path_row(
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
) -> ArchiveBatchPathRow {
    ArchiveBatchPathRow {
        file_path,
        month_key: None,
        coverage_start_at,
        coverage_end_at,
        historical_rollups_materialized_at: Some("materialized".to_string()),
        needs_overall: None,
        needs_failures: None,
    }
}

#[derive(Debug, Default)]
pub(crate) struct PendingInvocationArchiveOverallState {
    unmaterialized: BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    materialized: BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
    unreadable_unmaterialized_paths: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct PendingProxyPerfArchiveState {
    unmaterialized: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    materialized: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct PendingInvocationArchiveFailureState {
    unmaterialized_rows: Vec<ArchivedInvocationFailureRow>,
    materialized_row_counts: HashMap<(i64, String, String, i64, String), usize>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
}
