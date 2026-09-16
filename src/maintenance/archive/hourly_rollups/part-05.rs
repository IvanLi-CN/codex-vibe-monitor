fn upstream_host_network_direct_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_direct_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_pool_attempt_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_pool_attempt_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_payload_host_sql(alias: &str) -> String {
    format!(
        "COALESCE( \
            NULLIF(LOWER(TRIM(CAST(CASE \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstreamBaseUrlHost') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstreamBaseUrlHost') \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstream_base_url_host') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstream_base_url_host') \
            END AS TEXT))), ''), \
            {unknown_host} \
        )",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn upstream_host_network_attempt_host_sql(alias: &str) -> String {
    format!(
        "COALESCE(NULLIF(LOWER(TRIM({alias}.upstream_base_url_host)), ''), {unknown_host})",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    max_id_sql: &str,
) -> Result<(i64, bool)> {
    if crate::stats::hourly_rollup_progress_exists(&mut *tx, dataset).await? {
        return Ok((
            load_hourly_rollup_live_progress_tx(tx, dataset).await?,
            false,
        ));
    }
    let cursor_id = sqlx::query_scalar::<_, Option<i64>>(max_id_sql)
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(0)
        .max(0);
    save_hourly_rollup_live_progress_tx(tx, dataset, cursor_id).await?;
    Ok((cursor_id, true))
}

async fn upsert_upstream_host_network_minute_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[UpstreamHostNetworkMinuteSourceRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut aggregates = BTreeMap::<
        (i64, String, String),
        crate::dashboard_network_speed::DashboardNetworkByteTotals,
    >::new();
    for row in rows {
        let entry = aggregates
            .entry((
                row.bucket_start_epoch,
                row.source.clone(),
                row.upstream_base_url_host.clone(),
            ))
            .or_default();
        entry.upload_bytes = entry.upload_bytes.saturating_add(row.upload_bytes.max(0));
        entry.download_bytes = entry
            .download_bytes
            .saturating_add(row.download_bytes.max(0));
    }

    for ((bucket_start_epoch, source, upstream_base_url_host), totals) in aggregates {
        sqlx::query(
            r#"
            INSERT INTO upstream_host_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upload_bytes,
                download_bytes,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source, upstream_base_url_host) DO UPDATE SET
                upload_bytes = upstream_host_network_minute.upload_bytes + excluded.upload_bytes,
                download_bytes = upstream_host_network_minute.download_bytes + excluded.download_bytes,
                updated_at = datetime('now')
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_base_url_host)
        .bind(totals.upload_bytes.max(0))
        .bind(totals.download_bytes.max(0))
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        "SELECT MAX(id) FROM codex_invocations",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_direct_upload_bytes_sql("inv");
    let download_bytes_sql = upstream_host_network_direct_download_bytes_sql("inv");
    let host_sql = upstream_host_network_payload_host_sql("inv");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                inv.id,
                ((unixepoch(inv.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM codex_invocations AS inv
            WHERE inv.id > ?1
              AND (
                    COALESCE(
                        CASE
                            WHEN json_valid(inv.payload)
                                THEN TRIM(CAST(json_extract(inv.payload, '$.routeMode') AS TEXT))
                        END,
                        ''
                    ) <> ?2
                    OR NOT EXISTS (
                        SELECT 1
                        FROM pool_upstream_request_attempts AS attempts
                        WHERE attempts.invoke_id = inv.invoke_id
                          AND attempts.occurred_at = inv.occurred_at
                    )
              )
            ORDER BY inv.id ASC
            LIMIT ?3
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_pool_attempts(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        "SELECT MAX(id) FROM pool_upstream_request_attempts",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_pool_attempt_upload_bytes_sql("attempts");
    let download_bytes_sql = upstream_host_network_pool_attempt_download_bytes_sql("attempts");
    let host_sql = upstream_host_network_attempt_host_sql("attempts");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                attempts.id,
                ((unixepoch(attempts.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM pool_upstream_request_attempts AS attempts
            INNER JOIN codex_invocations AS inv
                ON inv.invoke_id = attempts.invoke_id
               AND inv.occurred_at = attempts.occurred_at
            WHERE attempts.id > ?1
            ORDER BY attempts.id ASC
            LIMIT ?2
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

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

#[derive(Debug, Clone, FromRow)]
struct InvocationArchiveIntegrityFileRow {
    file_path: String,
    sha256: String,
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

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        let archive_result = async {
            inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
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

    before_live_source_scan().await;

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
