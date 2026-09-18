pub(crate) async fn backfill_invocation_rollup_hourly_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<InvocationHourlyRollupReconciliation> {
    reconcile_invocation_rollup_hourly_from_sources(pool, || std::future::ready(())).await
}

#[cfg(test)]
pub(crate) async fn backfill_invocation_rollup_hourly_from_sources_with_snapshot_hook<F, Fut>(
    pool: &Pool<Sqlite>,
    before_live_source_scan: F,
) -> Result<InvocationHourlyRollupReconciliation>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    reconcile_invocation_rollup_hourly_from_sources(pool, before_live_source_scan).await
}

async fn reconcile_invocation_rollup_hourly_from_sources<F, Fut>(
    pool: &Pool<Sqlite>,
    before_live_source_scan: F,
) -> Result<InvocationHourlyRollupReconciliation>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    // Keep the manifest and live rows in one SQLite snapshot. Retention may move a row from the
    // live table into an archive between these scans; mixing snapshots would certify a gap as a
    // complete source and delete its canonical rollup.
    let mut tx = pool.begin().await?;
    let archive_files = match sqlx::query_as::<_, InvocationArchiveIntegrityFileRow>(
        r#"
        SELECT file_path, sha256
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(rows) => rows,
        Err(error) if error.to_string().contains("no such table") => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let integrity_source_start_date = load_long_term_integrity_source_start_date(&mut tx).await?;
    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut seen_ids = HashSet::new();
    let mut source_incomplete = false;
    let (archive_incomplete, unavailable_archive_file_paths) =
        collect_invocation_reconciliation_archive_rows(archive_files, &mut seen_ids, &mut overall)
            .await?;
    source_incomplete |= archive_incomplete;

    before_live_source_scan().await;

    let (live_incomplete, _cursor_id) =
        collect_live_invocation_reconciliation_rows(&mut tx, &mut seen_ids, &mut overall).await?;
    source_incomplete |= live_incomplete;

    if source_incomplete {
        // An incomplete source cannot certify a replacement rollup. Preserve the prior
        // canonical buckets so a legacy archive cannot turn missing Token components into zero.
        let invalidated = invalidate_incomplete_invocation_rollup_proofs(
            &mut tx,
            integrity_source_start_date.as_ref(),
        )
        .await?;
        warn!(
            invalidated,
            "left reconstructable canonical terminal integrity proofs unavailable because an invocation source is incomplete"
        );
        tx.commit().await?;
        return Ok(InvocationHourlyRollupReconciliation {
            applied_rollups: 0,
            invalidated_bucket_start_epochs: Vec::new(),
            unavailable_archive_file_paths,
            source_complete: false,
        });
    }

    let (applied_rollups, invalidated_bucket_start_epochs) =
        apply_invocation_hourly_reconciliation(
            tx.as_mut(),
            &overall,
            integrity_source_start_date.as_ref(),
        )
        .await?;
    tx.commit().await?;

    if !invalidated_bucket_start_epochs.is_empty() {
        warn!(
            stale_rollups = invalidated_bucket_start_epochs.len(),
            "removed canonical hourly buckets omitted by complete source reconciliation"
        );
    }
    Ok(InvocationHourlyRollupReconciliation {
        applied_rollups,
        invalidated_bucket_start_epochs: invalidated_bucket_start_epochs.into_iter().collect(),
        unavailable_archive_file_paths: Vec::new(),
        source_complete: true,
    })
}

async fn collect_invocation_reconciliation_archive_rows(
    archive_files: Vec<InvocationArchiveIntegrityFileRow>,
    seen_ids: &mut HashSet<i64>,
    overall: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
) -> Result<(bool, Vec<String>)> {
    let mut source_incomplete = false;
    let mut unavailable_archive_file_paths = Vec::new();
    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during invocation hourly rollup backfill"
            );
            source_incomplete = true;
            unavailable_archive_file_paths.push(archive_file.file_path.clone());
            continue;
        }
        let actual_sha256 = match sha256_hex_file(&archive_path) {
            Ok(value) => value,
            Err(error) => {
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    error = %error,
                    "could not verify archive batch identity during invocation hourly rollup proof reconciliation"
                );
                source_incomplete = true;
                unavailable_archive_file_paths.push(archive_file.file_path.clone());
                continue;
            }
        };
        if actual_sha256 != archive_file.sha256 {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = %archive_path.display(),
                expected_sha256 = archive_file.sha256,
                actual_sha256,
                "archive batch identity does not match its manifest during invocation hourly rollup proof reconciliation"
            );
            source_incomplete = true;
            unavailable_archive_file_paths.push(archive_file.file_path.clone());
            continue;
        }
        let (rows, incomplete) = match load_invocation_reconciliation_archive_file_rows(
            &archive_path,
            seen_ids,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                source_incomplete = true;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    error = %error,
                    "could not read archive batch during invocation hourly rollup proof reconciliation"
                );
                unavailable_archive_file_paths.push(archive_file.file_path.clone());
                continue;
            }
        };
        source_incomplete |= incomplete;
        accumulate_invocation_hourly_overall_rollups(overall, &rows)?;
    }
    Ok((source_incomplete, unavailable_archive_file_paths))
}

async fn invalidate_incomplete_invocation_rollup_proofs(
    tx: &mut SqliteConnection,
    integrity_source_start_date: Option<&NaiveDate>,
) -> Result<u64> {
    if let Some(source_start) = integrity_source_start_date {
        let boundary_start_epoch = long_term_integrity_source_boundary_start_epoch(source_start)?;
        return Ok(sqlx::query(
            "UPDATE invocation_rollup_hourly SET terminal_proof_complete = 0 WHERE terminal_proof_complete <> 0 AND bucket_start_epoch >= ?1",
        )
        .bind(boundary_start_epoch)
        .execute(&mut *tx)
        .await?
        .rows_affected());
    }
    Ok(sqlx::query(
        "UPDATE invocation_rollup_hourly SET terminal_proof_complete = 0 WHERE terminal_proof_complete <> 0",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected())
}

const INVOCATION_HOURLY_RECONCILIATION_UPSERT_SQL: &str = r#"
INSERT INTO invocation_rollup_hourly (
    bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count,
    terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, cache_input_tokens,
    total_cost, non_success_cost, total_latency_sample_count, total_latency_sum_ms,
    first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram,
    first_response_byte_total_sample_count, first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms, first_response_byte_total_histogram,
    first_token_sample_count, first_token_sum_ms, first_token_max_ms, first_token_histogram,
    input_tokens, output_tokens, reasoning_tokens, updated_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, datetime('now'))
ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
    total_count = excluded.total_count,
    success_count = excluded.success_count,
    failure_count = excluded.failure_count,
    terminal_count = excluded.terminal_count,
    terminal_tokens = excluded.terminal_tokens,
    terminal_cost = excluded.terminal_cost,
    terminal_proof_complete = 1,
    total_tokens = excluded.total_tokens,
    cache_input_tokens = excluded.cache_input_tokens,
    input_tokens = excluded.input_tokens,
    output_tokens = excluded.output_tokens,
    reasoning_tokens = excluded.reasoning_tokens,
    total_cost = excluded.total_cost,
    non_success_cost = excluded.non_success_cost,
    total_latency_sample_count = excluded.total_latency_sample_count,
    total_latency_sum_ms = excluded.total_latency_sum_ms,
    first_byte_sample_count = excluded.first_byte_sample_count,
    first_byte_sum_ms = excluded.first_byte_sum_ms,
    first_byte_max_ms = excluded.first_byte_max_ms,
    first_byte_histogram = excluded.first_byte_histogram,
    first_response_byte_total_sample_count = excluded.first_response_byte_total_sample_count,
    first_response_byte_total_sum_ms = excluded.first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms = excluded.first_response_byte_total_max_ms,
    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
    first_token_sample_count = excluded.first_token_sample_count,
    first_token_sum_ms = excluded.first_token_sum_ms,
    first_token_max_ms = excluded.first_token_max_ms,
    first_token_histogram = excluded.first_token_histogram,
    updated_at = datetime('now')
"#;

async fn apply_invocation_hourly_reconciliation(
    tx: &mut SqliteConnection,
    overall: &BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    integrity_source_start_date: Option<&NaiveDate>,
) -> Result<(usize, BTreeSet<i64>)> {
    let canonical_rollup_keys = sqlx::query_as::<_, (i64, String)>(
        "SELECT bucket_start_epoch, source FROM invocation_rollup_hourly",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut invalidated_bucket_start_epochs = BTreeSet::new();
    for (bucket_start_epoch, source) in canonical_rollup_keys {
        if overall.contains_key(&(bucket_start_epoch, source.clone()))
            || long_term_integrity_source_boundary_excludes_bucket(
                bucket_start_epoch,
                integrity_source_start_date,
            )
        {
            continue;
        }
        sqlx::query(
            "DELETE FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        invalidated_bucket_start_epochs.insert(bucket_start_epoch);
    }
    let mut applied_rollups = 0;
    for ((bucket_start_epoch, source), delta) in overall {
        sqlx::query(INVOCATION_HOURLY_RECONCILIATION_UPSERT_SQL)
            .bind(*bucket_start_epoch)
            .bind(source)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.terminal_count)
            .bind(delta.terminal_tokens)
            .bind(delta.terminal_cost)
            .bind(delta.total_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&delta.first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &delta.first_response_byte_total_histogram,
            )?)
            .bind(delta.first_token_sample_count)
            .bind(delta.first_token_sum_ms)
            .bind(delta.first_token_max_ms)
            .bind(encode_approx_histogram(&delta.first_token_histogram)?)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.reasoning_tokens)
            .execute(&mut *tx)
            .await?;
        applied_rollups += 1;
    }
    Ok((applied_rollups, invalidated_bucket_start_epochs))
}

pub(crate) async fn rebuild_upstream_account_stats_rollups_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<(usize, usize)> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut seen_ids = HashSet::new();
    let mut source_rows = Vec::<InvocationHourlySourceRecord>::new();
    let mut source_incomplete = false;

    for archive_file in archive_files {
        let (rows, incomplete) =
            load_upstream_account_stats_rebuild_archive_file(&archive_file, &mut seen_ids).await?;
        source_incomplete |= incomplete;
        source_rows.extend(rows);
    }

    let (live_rows, live_incomplete, cursor_id) =
        load_live_invocation_source_rows_for_stats_rebuild(pool, &mut seen_ids).await?;
    source_incomplete |= live_incomplete;
    source_rows.extend(live_rows);

    if source_incomplete {
        let hourly_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
                .fetch_one(pool)
                .await?;
        let minute_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
                .fetch_one(pool)
                .await?;
        return Ok((hourly_count.max(0) as usize, minute_count.max(0) as usize));
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM upstream_account_stats_hourly")
        .execute(tx.as_mut())
        .await?;
    sqlx::query("DELETE FROM upstream_account_stats_minute")
        .execute(tx.as_mut())
        .await?;
    if !source_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &source_rows,
            &[
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            ],
        )
        .await?;
    }
    if cursor_id > 0 {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            cursor_id,
        )
        .await?;
    }
    tx.commit().await?;

    let hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    Ok((hourly_count.max(0) as usize, minute_count.max(0) as usize))
}

async fn load_upstream_account_stats_rebuild_archive_file(
    archive_file: &ArchiveBatchFileRow,
    seen_ids: &mut HashSet<i64>,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let archive_path = PathBuf::from(&archive_file.file_path);
    if !archive_path.exists() {
        warn!(
            dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path = archive_file.file_path,
            "skipping missing archive batch during upstream account stats rollup rebuild"
        );
        return Ok((Vec::new(), true));
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
    inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
    let archive_columns = load_archive_table_columns(&archive_pool, "codex_invocations").await?;
    let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
    let mut rows = Vec::new();
    let mut source_incomplete = false;
    let mut archive_cursor_id = 0_i64;
    loop {
        let mut batch = sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
            .bind(archive_cursor_id)
            .bind(BACKFILL_BATCH_SIZE)
            .fetch_all(&archive_pool)
            .await?;
        if batch.is_empty() {
            break;
        }
        archive_cursor_id = batch.last().map(|row| row.id).unwrap_or(archive_cursor_id);
        batch.retain(|row| seen_ids.insert(row.id));
        if batch.iter().any(|row| !row.has_complete_token_components()) {
            source_incomplete = true;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = %archive_path.display(),
                "skipping upstream account stats rollup rebuild because archive token components are incomplete"
            );
            break;
        }
        rows.extend(batch);
    }
    archive_pool.close().await;
    drop(temp_cleanup);
    Ok((rows, source_incomplete))
}

async fn load_live_invocation_source_rows_for_stats_rebuild(
    pool: &Pool<Sqlite>,
    seen_ids: &mut HashSet<i64>,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool, i64)> {
    let mut cursor_id = 0_i64;
    let mut live_conn = pool.acquire().await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(&mut live_conn).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(&mut live_conn).await?;
    let mut rows = Vec::new();
    let mut source_incomplete = false;
    loop {
        let mut batch = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
            SELECT id, occurred_at, source, status, detail_level, model, input_tokens,
                output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost,
                {} AS upstream_account_id, cost_input, cost_cache_write, cost_cache_read,
                cost_output, cost_reasoning, error_message, failure_kind, failure_class,
                is_actionable, payload, t_total_ms, t_req_read_ms, t_req_parse_ms,
                t_upstream_connect_ms, t_upstream_ttfb_ms, {} AS first_token_ms,
                t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
            upstream_account_id_sql, first_token_ms_sql,
        ))
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *live_conn)
        .await?;
        if batch.is_empty() {
            break;
        }
        cursor_id = batch.last().map(|row| row.id).unwrap_or(cursor_id);
        batch.retain(|row| seen_ids.insert(row.id));
        if batch.iter().any(|row| !row.has_complete_token_components()) {
            source_incomplete = true;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping upstream account stats rollup rebuild because live token components are incomplete"
            );
            break;
        }
        rows.extend(batch);
    }
    drop(live_conn);
    Ok((rows, source_incomplete, cursor_id))
}

async fn load_invocation_reconciliation_archive_file_rows(
    archive_path: &Path,
    seen_ids: &mut HashSet<i64>,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
    let result: Result<(Vec<InvocationHourlySourceRecord>, bool)> = async {
        let archive_columns = load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
        let mut rows = Vec::new();
        let mut source_incomplete = false;
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut batch = sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                .bind(archive_cursor_id)
                .bind(BACKFILL_BATCH_SIZE)
                .fetch_all(&archive_pool)
                .await?;
            if batch.is_empty() {
                break;
            }
            archive_cursor_id = batch.last().map(|row| row.id).unwrap_or(archive_cursor_id);
            batch.retain(|row| seen_ids.insert(row.id));
            if batch.is_empty() {
                continue;
            }
            if batch.iter().any(|row| !row.has_complete_token_components()) {
                source_incomplete = true;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    "skipping invocation hourly rollup reconciliation because archive token components are incomplete"
                );
                break;
            }
            rows.extend(batch);
        }
        Ok((rows, source_incomplete))
    }
    .await;
    archive_pool.close().await;
    drop(temp_cleanup);
    result
}

async fn collect_live_invocation_reconciliation_rows(
    tx: &mut SqliteConnection,
    seen_ids: &mut HashSet<i64>,
    overall: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
) -> Result<(bool, i64)> {
    let mut cursor_id = 0_i64;
    let mut source_incomplete = false;
    loop {
        let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT id, occurred_at, source, status, detail_level, model, input_tokens,
                output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, cost_input,
                cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                error_message, failure_kind, failure_class, is_actionable, payload,
                t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
                t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms,
                t_resp_parse_ms, t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        if rows.iter().any(|row| !row.has_complete_token_components()) {
            source_incomplete = true;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping invocation hourly rollup reconciliation because live token components are incomplete"
            );
            break;
        }
        accumulate_invocation_hourly_overall_rollups(overall, &rows)?;
    }
    Ok((source_incomplete, cursor_id))
}
