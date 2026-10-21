use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvocationHourlyRollupReconciliation {
    pub(crate) applied_rollups: usize,
    /// A complete source scan omitted a canonical bucket inside the reconstructable source
    /// window. The stale bucket is removed and callers use these dates to repair dependent
    /// long-term materializations, including an empty replacement when appropriate.
    pub(crate) invalidated_bucket_start_epochs: Vec<i64>,
    /// Completed invocation archives that could not participate in the source scan. Consumers
    /// must invalidate their own replay markers so a restored file is read again.
    pub(crate) unavailable_archive_file_paths: Vec<String>,
    pub(crate) source_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct InvocationArchiveIntegrityFileRow {
    file_path: String,
    sha256: Option<String>,
}

async fn load_long_term_integrity_source_start_date(
    connection: &mut SqliteConnection,
) -> Result<Option<NaiveDate>> {
    let source_start = match sqlx::query_scalar::<_, Option<String>>(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_optional(connection)
    .await
    {
        Ok(Some(Some(value))) => value,
        Ok(Some(None) | None) => return Ok(None),
        // Invocation hourly rollups can be reconciled during schema recovery before the
        // long-term state table exists. Without a durable boundary, preserve the existing
        // conservative behavior and require every trusted bucket to be present in sources.
        Err(error)
            if error.to_string().contains("no such table")
                || error.to_string().contains("no such column") =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    NaiveDate::parse_from_str(&source_start, "%Y-%m-%d")
        .map(Some)
        .map_err(|error| {
            anyhow!("long-term integrity source boundary is invalid ({source_start}): {error}")
        })
}

fn long_term_integrity_source_boundary_excludes_bucket(
    bucket_start_epoch: i64,
    source_start: Option<&NaiveDate>,
) -> bool {
    let Some(source_start) = source_start else {
        return false;
    };
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .is_some_and(|bucket_start| bucket_start.date_naive() < *source_start)
}

fn long_term_integrity_source_boundary_start_epoch(source_start: &NaiveDate) -> Result<i64> {
    let midnight = source_start
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("long-term integrity source boundary has no local midnight"))?;
    Shanghai
        .from_local_datetime(&midnight)
        .single()
        .map(|value| value.timestamp())
        .ok_or_else(|| anyhow!("long-term integrity source boundary midnight is ambiguous"))
}

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
    let mut before_live_source_scan = Some(before_live_source_scan);
    reconcile_invocation_rollup_hourly_from_sources_with_retry(
        pool,
        &mut before_live_source_scan,
        INVOCATION_ROLLUP_SOURCE_SNAPSHOT_RETRIES,
    )
    .await
}

async fn reconcile_invocation_rollup_hourly_from_sources_with_retry<F, Fut>(
    pool: &Pool<Sqlite>,
    before_live_source_scan: &mut Option<F>,
    retries_remaining: u8,
) -> Result<InvocationHourlyRollupReconciliation>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    // Keep the database transaction out of archive hashing/decompression. A short read
    // transaction is enough to capture the initial manifest and integrity boundary; the live
    // source scan below establishes its own snapshot and rechecks this manifest before writes.
    let (archive_files, integrity_source_start_date) = {
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
        let integrity_source_start_date =
            load_long_term_integrity_source_start_date(&mut tx).await?;
        tx.commit().await?;
        (archive_files, integrity_source_start_date)
    };
    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut seen_ids = HashSet::new();
    let mut source_incomplete = false;
    let mut unavailable_archive_file_paths = Vec::new();

    for archive_file in &archive_files {
        let Some(expected_sha256) = archive_file
            .sha256
            .as_deref()
            .filter(|sha| !sha.trim().is_empty())
        else {
            source_incomplete = true;
            unavailable_archive_file_paths.push(archive_file.file_path.clone());
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping archive batch with missing identity during invocation hourly rollup proof reconciliation"
            );
            continue;
        };
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
        let _archive_lock = match retention_archive_file_lock(&archive_path) {
            Ok(lock) => lock,
            Err(error) => {
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    error = %error,
                    "could not acquire archive lock during invocation hourly rollup proof reconciliation"
                );
                source_incomplete = true;
                unavailable_archive_file_paths.push(archive_file.file_path.clone());
                continue;
            }
        };
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
        if expected_sha256 != actual_sha256 {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                expected_sha256_present = true,
                actual_sha256,
                "archive batch identity does not match its manifest during invocation hourly rollup proof reconciliation"
            );
            source_incomplete = true;
            unavailable_archive_file_paths.push(archive_file.file_path.clone());
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
        let archive_result = async {
            inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await?;
            let archive_pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&sqlite_url_for_path(&temp_path))
                .await
                .with_context(|| {
                    format!("failed to open archive batch {}", archive_path.display())
                })?;
            let result: Result<()> = async {
                let archive_columns =
                    load_archive_table_columns(&archive_pool, "codex_invocations").await?;
                let archive_query_sql =
                    build_legacy_compatible_invocation_archive_query(&archive_columns);
                let mut archive_cursor_id = 0_i64;
                loop {
                    let mut rows =
                        sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                            .bind(archive_cursor_id)
                            .bind(BACKFILL_BATCH_SIZE)
                            .fetch_all(&archive_pool)
                            .await?;
                    if rows.is_empty() {
                        break;
                    }
                    archive_cursor_id = rows.last().map(|row| row.id).unwrap_or(archive_cursor_id);
                    rows.retain(|row| seen_ids.insert(row.id));
                    if rows.is_empty() {
                        continue;
                    }
                    if rows
                        .iter()
                        .any(|row| !row.has_complete_token_components())
                    {
                        source_incomplete = true;
                        warn!(
                            dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                            file_path = %archive_path.display(),
                            "skipping invocation hourly rollup reconciliation because archive token components are incomplete"
                        );
                        break;
                    }
                    accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
                }
                Ok(())
            }
            .await;
            archive_pool.close().await;
            result
        }
        .await;
        drop(temp_cleanup);
        if let Err(error) = archive_result {
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
    }

    // Establish a live-table snapshot before the test/retention hook runs. The transaction now
    // covers only bounded SQLite reads and the final rollup writes, never archive file I/O.
    let mut tx = pool.begin().await?;
    let _: Option<i64> =
        sqlx::query_scalar("SELECT id FROM codex_invocations ORDER BY id ASC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(before_live_source_scan) = before_live_source_scan.take() {
        before_live_source_scan().await;
    }

    let mut cursor_id = 0_i64;
    loop {
        let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
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
        accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
    }

    let archive_files_after = match sqlx::query_as::<_, InvocationArchiveIntegrityFileRow>(
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
    if archive_files_after != archive_files {
        warn!(
            dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
            retries_remaining,
            "archive manifest changed during invocation hourly rollup snapshot; retrying"
        );
        drop(tx);
        if retries_remaining > 0 {
            return Box::pin(reconcile_invocation_rollup_hourly_from_sources_with_retry(
                pool,
                before_live_source_scan,
                retries_remaining - 1,
            ))
            .await;
        }
        bail!(
            "archive manifest changed during invocation hourly rollup snapshot after retries; retryable snapshot-write conflict"
        );
    }

    if source_incomplete {
        // An incomplete source cannot certify a replacement rollup. Preserve the prior
        // canonical buckets so a legacy archive cannot turn missing Token components into zero.
        let invalidated = if let Some(source_start) = integrity_source_start_date.as_ref() {
            let boundary_start_epoch =
                long_term_integrity_source_boundary_start_epoch(source_start)?;
            sqlx::query(
                "UPDATE invocation_rollup_hourly SET terminal_proof_complete = 0 WHERE terminal_proof_complete <> 0 AND bucket_start_epoch >= ?1",
            )
            .bind(boundary_start_epoch)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE invocation_rollup_hourly SET terminal_proof_complete = 0 WHERE terminal_proof_complete <> 0",
            )
            .execute(&mut *tx)
            .await?
            .rows_affected()
        };
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

    let mut applied_rollups = 0usize;
    let mut invalidated_bucket_start_epochs = BTreeSet::new();
    let canonical_rollup_keys = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT bucket_start_epoch, source
        FROM invocation_rollup_hourly
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;
    for (bucket_start_epoch, source) in canonical_rollup_keys {
        if overall.contains_key(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        // Archives removed after a verified two-phase cleanup are intentionally outside the
        // reconstructable source window. Their canonical proofs remain the durable integrity
        // evidence; only buckets whose sources should still exist may be revoked as missing.
        if long_term_integrity_source_boundary_excludes_bucket(
            bucket_start_epoch,
            integrity_source_start_date.as_ref(),
        ) {
            continue;
        }
        // A complete source reconciliation is authoritative within the active source window.
        // Leaving a stale row untrusted would hide this date from the audit forever; remove it
        // so the queued long-term repair can prove and publish an empty replacement if needed.
        sqlx::query(
            "DELETE FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        invalidated_bucket_start_epochs.insert(bucket_start_epoch);
    }
    for ((bucket_start_epoch, source), delta) in &overall {
        // This scan consumed every readable archive and every live source row. It can therefore
        // replace a contradictory canonical value atomically instead of leaving the bucket
        // permanently untrusted and blocking the long-term repair queue.
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_hourly (
                bucket_start_epoch,
                source,
                total_count,
                success_count,
                failure_count,
                terminal_count,
                terminal_tokens,
                terminal_cost,
                terminal_proof_complete,
                total_tokens,
                cache_input_tokens,
                total_cost,
                non_success_cost,
                total_latency_sample_count,
                total_latency_sum_ms,
                first_byte_sample_count,
                first_byte_sum_ms,
                first_byte_max_ms,
                first_byte_histogram,
                first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms,
                first_response_byte_total_histogram,
                first_token_sample_count,
                first_token_sum_ms,
                first_token_max_ms,
                first_token_histogram,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                updated_at
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
            "#,
        )
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
        .execute(tx.as_mut())
        .await?;
        applied_rollups += 1;
    }
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

pub(crate) async fn rebuild_upstream_account_stats_rollups_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<(usize, usize)> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, sha256, coverage_start_at, coverage_end_at
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
        let archive_path = PathBuf::from(&archive_file.file_path);
        if archive_path.parent().is_none_or(|parent| !parent.exists()) || !archive_path.is_file() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping missing archive batch during upstream account stats rollup rebuild"
            );
            source_incomplete = true;
            continue;
        }
        let _archive_lock = retention_archive_file_lock(&archive_path)?;
        let Some(expected_sha256) = archive_file.sha256.as_deref() else {
            source_incomplete = true;
            continue;
        };
        if sha256_hex_file(&archive_path).ok().as_deref() != Some(expected_sha256) {
            source_incomplete = true;
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
        inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut archive_rows =
                sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                    .bind(archive_cursor_id)
                    .bind(BACKFILL_BATCH_SIZE)
                    .fetch_all(&archive_pool)
                    .await?;
            if archive_rows.is_empty() {
                break;
            }
            archive_cursor_id = archive_rows
                .last()
                .map(|row| row.id)
                .unwrap_or(archive_cursor_id);
            archive_rows.retain(|row| seen_ids.insert(row.id));
            if archive_rows
                .iter()
                .any(|row| !row.has_complete_token_components())
            {
                source_incomplete = true;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    "skipping upstream account stats rollup rebuild because archive token components are incomplete"
                );
                break;
            }
            source_rows.extend(archive_rows);
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    let mut live_conn = pool.acquire().await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(&mut live_conn).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(&mut live_conn).await?;
    loop {
        let mut live_rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                {} AS upstream_account_id,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                {} AS first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
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
        if live_rows.is_empty() {
            break;
        }
        cursor_id = live_rows.last().map(|row| row.id).unwrap_or(cursor_id);
        live_rows.retain(|row| seen_ids.insert(row.id));
        if live_rows
            .iter()
            .any(|row| !row.has_complete_token_components())
        {
            source_incomplete = true;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping upstream account stats rollup rebuild because live token components are incomplete"
            );
            break;
        }
        source_rows.extend(live_rows);
    }
    drop(live_conn);

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
