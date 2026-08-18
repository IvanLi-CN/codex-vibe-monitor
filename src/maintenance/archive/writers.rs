use super::*;
use anyhow::bail;
use sqlx::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use std::io::ErrorKind;
use std::str::FromStr;

fn sync_published_archive_file(final_file_path: &Path) -> Result<()> {
    fs::File::open(final_file_path)
        .with_context(|| {
            format!(
                "failed to open archive file for sync {}",
                final_file_path.display()
            )
        })?
        .sync_all()
        .with_context(|| format!("failed to sync archive file {}", final_file_path.display()))?;
    if let Some(parent) = final_file_path.parent() {
        fs::File::open(parent)
            .with_context(|| {
                format!(
                    "failed to open archive directory for sync {}",
                    parent.display()
                )
            })?
            .sync_all()
            .with_context(|| format!("failed to sync archive directory {}", parent.display()))?;
    }
    Ok(())
}

fn publish_prepared_archive_file(temporary_file_path: &Path, final_file_path: &Path) -> Result<()> {
    match fs::hard_link(temporary_file_path, final_file_path) {
        Ok(()) => fs::remove_file(temporary_file_path).with_context(|| {
            format!(
                "failed to remove linked archive staging file {}",
                temporary_file_path.display()
            )
        })?,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let prepared_sha = sha256_hex_file(temporary_file_path)?;
            let existing_sha = sha256_hex_file(final_file_path)?;
            if prepared_sha != existing_sha {
                bail!(
                    "archive batch identity collision for {}",
                    final_file_path.display()
                );
            }
            fs::remove_file(temporary_file_path).with_context(|| {
                format!(
                    "failed to remove duplicate archive staging file {}",
                    temporary_file_path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to publish prepared archive file: {} -> {}",
                    temporary_file_path.display(),
                    final_file_path.display()
                )
            });
        }
    }
    sync_published_archive_file(final_file_path)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PoolUpstreamRequestAttemptArchiveRow {
    id: i64,
    attempt_public_id: Option<String>,
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    route_mode: String,
    sticky_key: Option<String>,
    routing_source: Option<String>,
    routing_selection_audit_json: Option<String>,
    upstream_base_url_host: Option<String>,
    group_name_snapshot: Option<String>,
    proxy_binding_key_snapshot: Option<String>,
    request_model: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_route_key: Option<String>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    requester_ip: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    status: String,
    phase: Option<String>,
    http_status: Option<i64>,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    error_message: Option<String>,
    downstream_error_message: Option<String>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<String>,
    upstream_request_compression_algorithm: Option<String>,
    upstream_request_compression_mode: Option<String>,
    upstream_request_logical_body_bytes: Option<i64>,
    upstream_request_transmitted_body_bytes: Option<i64>,
    upstream_request_header_bytes_approx: Option<i64>,
    upstream_response_body_bytes: Option<i64>,
    upstream_response_header_bytes_approx: Option<i64>,
    compact_support_status: Option<String>,
    compact_support_reason: Option<String>,
    request_summary_json: Option<String>,
    response_summary_json: Option<String>,
    response_raw_path: Option<String>,
    response_raw_codec: Option<String>,
    response_raw_size: Option<i64>,
    response_raw_truncated: Option<i64>,
    response_raw_truncated_reason: Option<String>,
    response_content_encoding: Option<String>,
    created_at: String,
}

pub(crate) async fn open_archive_sqlite_connection(path: &Path) -> Result<SqliteConnection> {
    ensure_attachable_archive_sqlite_path(path)?;
    let database_url = format!("sqlite://{}", path.to_string_lossy());
    let connect_opts = SqliteConnectOptions::from_str(&database_url)
        .context("invalid sqlite database url")?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete)
        .busy_timeout(Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS));
    SqliteConnection::connect_with(&connect_opts)
        .await
        .with_context(|| format!("failed to open archive sqlite file {}", path.display()))
}

pub(crate) fn ensure_attachable_archive_sqlite_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }
    if !path.exists() {
        fs::File::create(path)
            .with_context(|| format!("failed to create archive sqlite file {}", path.display()))?;
    }
    Ok(())
}

pub(crate) async fn finalize_archive_sqlite_file(path: &Path) -> Result<()> {
    let mut connection = open_archive_sqlite_connection(path).await?;
    sqlx::query("PRAGMA optimize")
        .execute(&mut connection)
        .await
        .with_context(|| format!("failed to finalize archive sqlite file {}", path.display()))?;
    connection.close().await?;
    Ok(())
}

pub(crate) async fn ensure_pool_upstream_request_attempts_archive_schema_direct(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns = sqlx::query("PRAGMA table_info('pool_upstream_request_attempts')")
        .fetch_all(&mut *conn)
        .await
        .context("failed to inspect pool_upstream_request_attempts archive schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    for (column, ty) in [
        ("attempt_public_id", "TEXT"),
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("upstream_base_url_host", "TEXT"),
        ("upstream_request_compression_algorithm", "TEXT"),
        ("upstream_request_compression_mode", "TEXT"),
        ("upstream_request_logical_body_bytes", "INTEGER"),
        ("upstream_request_transmitted_body_bytes", "INTEGER"),
        ("upstream_request_header_bytes_approx", "INTEGER"),
        ("upstream_response_body_bytes", "INTEGER"),
        ("upstream_response_header_bytes_approx", "INTEGER"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
        ("request_model", "TEXT"),
        ("routing_source", "TEXT"),
        ("routing_selection_audit_json", "TEXT"),
        ("request_summary_json", "TEXT"),
        ("response_summary_json", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("response_content_encoding", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!("failed to add pool_upstream_request_attempts archive column {column}")
                })?;
        }
    }
    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_public_id
        ON pool_upstream_request_attempts (attempt_public_id)
        WHERE attempt_public_id IS NOT NULL
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to ensure idx_pool_upstream_request_attempts_public_id")?;
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolAttemptPublicIdArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) updated_batches: u64,
    pub(crate) scanned_rows: u64,
    pub(crate) updated_rows: u64,
}

#[derive(Debug, FromRow)]
struct PoolAttemptPublicIdArchiveBatchRow {
    id: i64,
    file_path: String,
}

pub(crate) async fn backfill_pool_upstream_request_attempt_archive_public_ids_from_batch_cursor(
    pool: &Pool<Sqlite>,
    start_after_batch_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<PoolAttemptPublicIdArchiveBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = PoolAttemptPublicIdArchiveBackfillSummary::default();
    let mut last_seen_batch_id = start_after_batch_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(
            started_at,
            summary.scanned_batches,
            scan_limit,
            max_elapsed,
        ) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_as::<_, PoolAttemptPublicIdArchiveBatchRow>(
            r#"
            SELECT id, file_path
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
              AND status = ?1
              AND id > ?2
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(last_seen_batch_id)
        .bind(startup_backfill_query_limit(
            summary.scanned_batches,
            scan_limit,
        ))
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        for batch in rows {
            last_seen_batch_id = batch.id;
            summary.scanned_batches += 1;
            let archive_path = PathBuf::from(&batch.file_path);
            let suffix = retention_temp_suffix();
            let work_path = PathBuf::from(format!("{}.{}.sqlite", batch.file_path, suffix));
            let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", batch.file_path, suffix));
            let _ = fs::remove_file(&work_path);
            let _ = fs::remove_file(&temp_gzip_path);

            inflate_gzip_sqlite_file(&archive_path, &work_path)?;

            let backfill_outcome = async {
                let mut conn = open_archive_sqlite_connection(&work_path).await?;
                ensure_pool_upstream_request_attempts_archive_schema_direct(&mut conn).await?;
                let outcome = backfill_pool_upstream_request_attempt_public_ids_on_connection(
                    &mut conn, 0, None, None,
                )
                .await?;
                conn.close().await?;
                Ok::<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>, anyhow::Error>(
                    outcome,
                )
            }
            .await;

            let outcome = match backfill_outcome {
                Ok(outcome) => outcome,
                Err(err) => {
                    let _ = fs::remove_file(&work_path);
                    let _ = fs::remove_file(&temp_gzip_path);
                    return Err(err);
                }
            };

            summary.scanned_rows += outcome.summary.scanned;
            summary.updated_rows += outcome.summary.updated;
            if outcome.summary.updated > 0 {
                summary.updated_batches += 1;
                push_backfill_sample(
                    &mut samples,
                    format!("batch_id={} rows={}", batch.id, outcome.summary.updated),
                );
            }

            if let Err(err) = finalize_archive_sqlite_file(&work_path).await {
                let _ = fs::remove_file(&work_path);
                let _ = fs::remove_file(&temp_gzip_path);
                return Err(err);
            }
            if let Err(err) = deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path) {
                let _ = fs::remove_file(&work_path);
                let _ = fs::remove_file(&temp_gzip_path);
                return Err(err);
            }
            fs::rename(&temp_gzip_path, &archive_path).with_context(|| {
                format!(
                    "failed to move pool_upstream_request_attempts archive batch into place: {} -> {}",
                    temp_gzip_path.display(),
                    archive_path.display()
                )
            })?;
            sync_published_archive_file(&archive_path)?;
            let _ = fs::remove_file(&work_path);

            let sha256 = sha256_hex_file(&archive_path)?;
            sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE id = ?2")
                .bind(&sha256)
                .bind(batch.id)
                .execute(pool)
                .await?;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_batch_id,
        hit_budget,
        samples,
    })
}

pub(crate) async fn archive_pool_upstream_request_attempt_rows_into_month_batch(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    ids: &[i64],
    work_path: &Path,
) -> Result<(i64, Vec<(i64, String)>)> {
    let create_sql = spec.create_sql.replace("archive_db.", "");
    let mut rows = Vec::new();
    for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {} FROM {} WHERE id IN (",
            spec.columns, spec.dataset
        ));
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(id);
            }
        }
        query.push(")");
        rows.extend(
            query
                .build_query_as::<PoolUpstreamRequestAttemptArchiveRow>()
                .fetch_all(pool)
                .await
                .with_context(|| {
                    format!(
                        "failed to load source rows for direct archive batch {}",
                        spec.dataset
                    )
                })?,
        );
    }
    let mut conn = open_archive_sqlite_connection(work_path).await?;
    sqlx::query(&create_sql)
        .execute(&mut conn)
        .await
        .context("failed to ensure direct pool_upstream_request_attempts archive schema")?;
    ensure_pool_upstream_request_attempts_archive_schema_direct(&mut conn).await?;

    for chunk in rows.chunks(16) {
        let mut insert = QueryBuilder::<Sqlite>::new(format!(
            "INSERT OR IGNORE INTO {} ({}) ",
            spec.dataset, spec.columns
        ));
        insert.push_values(chunk, |mut builder, row| {
            builder
                .push_bind(row.id)
                .push_bind(&row.attempt_public_id)
                .push_bind(&row.invoke_id)
                .push_bind(&row.occurred_at)
                .push_bind(&row.endpoint)
                .push_bind(&row.route_mode)
                .push_bind(&row.sticky_key)
                .push_bind(&row.routing_source)
                .push_bind(&row.routing_selection_audit_json)
                .push_bind(&row.upstream_base_url_host)
                .push_bind(&row.group_name_snapshot)
                .push_bind(&row.proxy_binding_key_snapshot)
                .push_bind(&row.request_model)
                .push_bind(row.upstream_account_id)
                .push_bind(&row.upstream_route_key)
                .push_bind(row.attempt_index)
                .push_bind(row.distinct_account_index)
                .push_bind(row.same_account_retry_index)
                .push_bind(&row.requester_ip)
                .push_bind(&row.started_at)
                .push_bind(&row.finished_at)
                .push_bind(&row.status)
                .push_bind(&row.phase)
                .push_bind(row.http_status)
                .push_bind(row.downstream_http_status)
                .push_bind(&row.failure_kind)
                .push_bind(&row.error_message)
                .push_bind(&row.downstream_error_message)
                .push_bind(row.connect_latency_ms)
                .push_bind(row.first_byte_latency_ms)
                .push_bind(row.stream_latency_ms)
                .push_bind(&row.upstream_request_id)
                .push_bind(&row.upstream_request_compression_algorithm)
                .push_bind(&row.upstream_request_compression_mode)
                .push_bind(row.upstream_request_logical_body_bytes)
                .push_bind(row.upstream_request_transmitted_body_bytes)
                .push_bind(row.upstream_request_header_bytes_approx)
                .push_bind(row.upstream_response_body_bytes)
                .push_bind(row.upstream_response_header_bytes_approx)
                .push_bind(&row.compact_support_status)
                .push_bind(&row.compact_support_reason)
                .push_bind(&row.request_summary_json)
                .push_bind(&row.response_summary_json)
                .push_bind(&row.response_raw_path)
                .push_bind(&row.response_raw_codec)
                .push_bind(row.response_raw_size)
                .push_bind(row.response_raw_truncated)
                .push_bind(&row.response_raw_truncated_reason)
                .push_bind(&row.response_content_encoding)
                .push_bind(&row.created_at);
        });
        insert.build().execute(&mut conn).await.with_context(|| {
            format!(
                "failed to copy rows into direct archive batch for {}",
                spec.dataset
            )
        })?;
    }

    let count_query = format!("SELECT COUNT(*) FROM {}", spec.dataset);
    let row_count = sqlx::query_scalar::<_, i64>(&count_query)
        .fetch_one(&mut conn)
        .await
        .with_context(|| format!("failed to count direct archive rows for {}", spec.dataset))?;
    conn.close().await?;
    Ok((row_count, Vec::new()))
}

pub(crate) async fn archive_rows_into_month_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive batch requires at least one row id");
    }

    let final_path = archive_batch_file_path(config, spec.dataset, month_key)?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!("{}.{}.sqlite", final_path.display(), suffix));
    let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", final_path.display(), suffix));

    if work_path.exists() {
        let _ = fs::remove_file(&work_path);
    }
    if temp_gzip_path.exists() {
        let _ = fs::remove_file(&temp_gzip_path);
    }

    if final_path.exists() {
        inflate_gzip_sqlite_file(&final_path, &work_path)?;
    } else {
        ensure_attachable_archive_sqlite_path(&work_path)?;
    }
    let row_count = if spec.dataset == "pool_upstream_request_attempts" {
        archive_pool_upstream_request_attempt_rows_into_month_batch(pool, spec, ids, &work_path)
            .await
    } else {
        async {
        let mut conn = pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ?1 AS archive_db")
            .bind(work_path.to_string_lossy().to_string())
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!("failed to attach archive database {}", work_path.display())
            })?;
        sqlx::query("PRAGMA archive_db.journal_mode=DELETE")
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!(
                    "failed to switch archive database {} to DELETE journal mode",
                    work_path.display()
                )
            })?;
        sqlx::query(spec.create_sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to ensure archive schema for {}", spec.dataset))?;
        if spec.dataset == "codex_invocations" {
            ensure_codex_invocations_archive_schema(&mut conn).await?;
        } else if spec.dataset == "pool_upstream_request_attempts" {
            ensure_pool_upstream_request_attempts_archive_schema(&mut conn).await?;
        }

        let upstream_last_activity = if spec.dataset == "codex_invocations" {
            let mut rows = Vec::new();
            for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS account_id, occurred_at FROM main.codex_invocations WHERE id IN (",
                );
                {
                    let mut separated = query.separated(", ");
                    for id in chunk {
                        separated.push_bind(id);
                    }
                }
                query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
                rows.extend(
                    query
                        .build_query_as::<ArchivedAccountLastActivityRow>()
                        .fetch_all(&mut *conn)
                        .await?,
                );
            }
            dedupe_archive_upstream_last_activity(
                rows.into_iter()
                    .map(|row| (row.account_id, row.last_activity_at)),
            )
        } else {
            Vec::new()
        };

        let mut insert = QueryBuilder::<Sqlite>::new(format!(
            "INSERT OR IGNORE INTO archive_db.{} ({}) SELECT {} FROM main.{} WHERE id IN (",
            spec.dataset, spec.columns, spec.columns, spec.dataset
        ));
        {
            let mut separated = insert.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
        }
        insert.push(")");
        insert.build().execute(&mut *conn).await.with_context(|| {
            format!(
                "failed to copy rows into archive batch for {}",
                spec.dataset
            )
        })?;

        let count_query = format!("SELECT COUNT(*) FROM archive_db.{}", spec.dataset);
        let row_count = sqlx::query_scalar::<_, i64>(&count_query)
            .fetch_one(&mut *conn)
            .await
            .with_context(|| format!("failed to count archive rows for {}", spec.dataset))?;
        sqlx::query("DETACH DATABASE archive_db")
            .execute(&mut *conn)
            .await
            .context("failed to detach archive database")?;
        Ok::<(i64, Vec<(i64, String)>), anyhow::Error>((row_count, upstream_last_activity))
    }
        .await
    };

    let (result, upstream_last_activity) = match row_count {
        Ok(values) => values,
        Err(err) => {
            let _ = fs::remove_file(&work_path);
            let _ = fs::remove_file(&temp_gzip_path);
            return Err(err);
        }
    };
    if let Err(err) = finalize_archive_sqlite_file(&work_path).await {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }

    if let Err(err) = deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path) {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }
    let sha256 = sha256_hex_file(&temp_gzip_path)?;
    if let Err(err) = replace_legacy_archive_file_with_cleanup_serialization(
        pool,
        spec.dataset,
        month_key,
        &temp_gzip_path,
        &final_path,
    )
    .await
    {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }
    let _ = fs::remove_file(&work_path);

    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key: month_key.to_string(),
        day_key: None,
        part_key: None,
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: result,
        upstream_last_activity,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    })
}

async fn replace_legacy_archive_file_with_cleanup_serialization(
    pool: &Pool<Sqlite>,
    dataset: &str,
    month_key: &str,
    temporary_file_path: &Path,
    final_file_path: &Path,
) -> Result<()> {
    // Cleanup finalization holds the same SQLite writer lock while it verifies and removes a
    // pending file. Keep reactivation and rename inside that lock so the two file operations
    // cannot interleave across processes.
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("legacy_archive_file_publish")
            .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "legacy_archive_file_publish",
        ));
    };
    let prepared_bytes = temporary_file_path
        .metadata()
        .map(|metadata| metadata.len() as usize)
        .unwrap_or_default();
    let execute_started = Instant::now();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET cleanup_state = ?1,
            cleanup_source_safe_start_date = NULL
        WHERE dataset = ?2
          AND month_key = ?3
          AND file_path = ?4
          AND cleanup_state = ?5
        "#,
    )
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind(dataset)
    .bind(month_key)
    .bind(final_file_path.to_string_lossy().to_string())
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .execute(tx.as_mut())
    .await?;
    if let Err(error) = fs::rename(temporary_file_path, final_file_path).with_context(|| {
        format!(
            "failed to move archive batch into place: {} -> {}",
            temporary_file_path.display(),
            final_file_path.display()
        )
    }) {
        tx.rollback().await?;
        return Err(error);
    }
    if let Err(error) = sync_published_archive_file(final_file_path) {
        tx.rollback().await?;
        return Err(error);
    }
    let commit_started = Instant::now();
    tx.commit().await?;
    super::super::retention::retention_record_commit!(
        "legacy_archive_file_publish",
        admission.admission_mode(),
        1,
        prepared_bytes,
        Duration::ZERO,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count(),
        0,
    );
    Ok(())
}

pub(crate) async fn archive_rows_into_segment_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    day_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive segment requires at least one row id");
    }
    if spec.dataset != "codex_invocations" {
        bail!("archive segment writer only supports codex_invocations");
    }
    let month_key = archive_month_key_from_day_key(day_key)?;
    let part_key = archive_segment_part_key_for_ids(ids)?;
    let final_path = archive_segment_file_path(
        config,
        spec.dataset,
        day_key,
        &part_key,
        config.invocation_archive_codec,
    )?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!(
        "{}.{}.partial.sqlite",
        final_path.display(),
        suffix
    ));
    let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", final_path.display(), suffix));
    let _temp_cleanup = TempSqliteCleanup(work_path.clone());
    let _gzip_cleanup = TempSqliteCleanup(temp_gzip_path.clone());
    ensure_attachable_archive_sqlite_path(&work_path)?;
    let row_count = async {
        let mut conn = pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ?1 AS archive_db")
            .bind(work_path.to_string_lossy().to_string())
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!("failed to attach archive database {}", work_path.display())
            })?;
        sqlx::query("PRAGMA archive_db.journal_mode=DELETE")
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!(
                    "failed to switch archive database {} to DELETE journal mode",
                    work_path.display()
                )
            })?;
        sqlx::query(spec.create_sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to ensure archive schema for {}", spec.dataset))?;
        ensure_codex_invocations_archive_schema(&mut conn).await?;

        let mut upstream_last_activity = Vec::new();
        for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS account_id, occurred_at FROM main.codex_invocations WHERE id IN (",
            );
            {
                let mut separated = query.separated(", ");
                for id in chunk {
                    separated.push_bind(id);
                }
            }
            query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
            upstream_last_activity.extend(
                query
                    .build_query_as::<ArchivedAccountLastActivityRow>()
                    .fetch_all(&mut *conn)
                    .await?,
            );
        }
        let upstream_last_activity = dedupe_archive_upstream_last_activity(
            upstream_last_activity
                .into_iter()
                .map(|row| (row.account_id, row.last_activity_at)),
        );

        let mut insert = QueryBuilder::<Sqlite>::new(format!(
            "INSERT OR IGNORE INTO archive_db.{} ({}) SELECT {} FROM main.{} WHERE id IN (",
            spec.dataset, spec.columns, spec.columns, spec.dataset
        ));
        {
            let mut separated = insert.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
        }
        insert.push(")");
        insert.build().execute(&mut *conn).await.with_context(|| {
            format!(
                "failed to copy rows into archive segment for {}",
                spec.dataset
            )
        })?;

        let count_query = format!("SELECT COUNT(*) FROM archive_db.{}", spec.dataset);
        let row_count = sqlx::query_scalar::<_, i64>(&count_query)
            .fetch_one(&mut *conn)
            .await
            .with_context(|| format!("failed to count archive rows for {}", spec.dataset))?;
        sqlx::query("DETACH DATABASE archive_db")
            .execute(&mut *conn)
            .await
            .context("failed to detach archive database")?;
        Ok::<(i64, Vec<(i64, String)>), anyhow::Error>((row_count, upstream_last_activity))
    }
    .await?;

    finalize_archive_sqlite_file(&work_path).await?;

    deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path)?;
    publish_prepared_archive_file(&temp_gzip_path, &final_path)?;

    let sha256 = sha256_hex_file(&final_path)?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key,
        day_key: Some(day_key.to_string()),
        part_key: Some(part_key),
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: row_count.0,
        upstream_last_activity: row_count.1,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        layout: ARCHIVE_LAYOUT_SEGMENT_V1,
        codec: config.invocation_archive_codec.as_str(),
        writer_version: ARCHIVE_WRITER_VERSION_SEGMENT_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    })
}

pub(crate) async fn upsert_archived_upstream_last_activity(
    tx: &mut sqlx::SqliteConnection,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    for (account_id, occurred_at) in &deduped_values {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                    WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                    ELSE last_activity_at
                END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_archive_batch_manifest(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            day_key,
            part_key,
            file_path,
            sha256,
            row_count,
            status,
            layout,
            codec,
            writer_version,
            cleanup_state,
            superseded_by,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now'))
        ON CONFLICT(dataset, month_key, file_path) DO UPDATE SET
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            sha256 = excluded.sha256,
            row_count = excluded.row_count,
            status = excluded.status,
            layout = excluded.layout,
            codec = excluded.codec,
            writer_version = excluded.writer_version,
            cleanup_state = excluded.cleanup_state,
            cleanup_source_safe_start_date = NULL,
            superseded_by = excluded.superseded_by,
            coverage_start_at = excluded.coverage_start_at,
            coverage_end_at = excluded.coverage_end_at,
            archive_expires_at = excluded.archive_expires_at,
            created_at = datetime('now')
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(batch.day_key.as_deref())
    .bind(batch.part_key.as_deref())
    .bind(&batch.file_path)
    .bind(&batch.sha256)
    .bind(batch.row_count)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(batch.layout)
    .bind(batch.codec)
    .bind(batch.writer_version)
    .bind(batch.cleanup_state)
    .bind(batch.superseded_by)
    .bind(batch.coverage_start_at.as_deref())
    .bind(batch.coverage_end_at.as_deref())
    .bind(batch.archive_expires_at.as_deref())
    .execute(&mut *tx)
    .await?;
    let deduped_upstream_last_activity =
        dedupe_archive_upstream_last_activity(batch.upstream_last_activity.iter().cloned());
    let archive_batch_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = ?1
          AND month_key = ?2
          AND file_path = ?3
        LIMIT 1
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(&batch.file_path)
    .fetch_one(&mut *tx)
    .await?;
    if batch.dataset == "codex_invocations" {
        write_archive_batch_upstream_activity(
            tx,
            archive_batch_id,
            &deduped_upstream_last_activity,
        )
        .await?;
    }
    if batch.dataset == "codex_invocations" && !deduped_upstream_last_activity.is_empty() {
        upsert_archived_upstream_last_activity(tx, &deduped_upstream_last_activity).await?;
    }
    Ok(())
}

pub(crate) async fn load_archive_batch_id_for_file_tx(
    tx: &mut sqlx::SqliteConnection,
    dataset: &str,
    month_key: &str,
    file_path: &str,
) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = ?1
          AND month_key = ?2
          AND file_path = ?3
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(month_key)
    .bind(file_path)
    .fetch_one(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to load archive batch id for dataset={dataset} month_key={month_key} file_path={file_path}"
        )
    })
}

pub(crate) async fn write_archive_batch_upstream_activity(
    tx: &mut sqlx::SqliteConnection,
    archive_batch_id: i64,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
        .bind(archive_batch_id)
        .execute(&mut *tx)
        .await?;
    for chunk in deduped_values.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut insert = QueryBuilder::<Sqlite>::new(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) ",
        );
        insert.push_values(chunk, |mut row, (account_id, last_activity_at)| {
            row.push_bind(archive_batch_id)
                .push_bind(account_id)
                .push_bind(last_activity_at);
        });
        insert.push(
            " ON CONFLICT(archive_batch_id, account_id) DO UPDATE SET last_activity_at = CASE \
             WHEN excluded.last_activity_at > last_activity_at THEN excluded.last_activity_at \
             ELSE last_activity_at END",
        );
        insert.build().execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') WHERE id = ?1",
    )
    .bind(archive_batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempArchiveTestDirectory(std::path::PathBuf);

    impl Drop for TempArchiveTestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn segment_part_key_is_stable_for_one_prepared_identity() {
        let forward = archive_segment_part_key_for_ids(&[7, 3, 5]).expect("part key");
        let reordered = archive_segment_part_key_for_ids(&[5, 7, 3]).expect("part key");
        let distinct = archive_segment_part_key_for_ids(&[3, 5, 8]).expect("part key");

        assert_eq!(forward, reordered);
        assert_ne!(forward, distinct);
    }

    #[test]
    fn prepared_archive_publish_rejects_conflicting_existing_identity() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-publish-{}",
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create archive root");
        let temporary = root.join("prepared.sqlite.gz");
        let final_path = root.join("part.sqlite.gz");
        fs::write(&temporary, b"prepared").expect("write prepared archive");
        fs::write(&final_path, b"different").expect("write conflicting archive");

        let error = publish_prepared_archive_file(&temporary, &final_path)
            .expect_err("conflicting archive identity must fail");
        assert!(error.to_string().contains("identity collision"));
        assert!(temporary.exists());
        assert_eq!(
            fs::read(&final_path).expect("read final archive"),
            b"different"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepared_archive_publish_reuses_a_matching_identity() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-publish-{}",
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create archive root");
        let first_temporary = root.join("first.sqlite.gz");
        let retry_temporary = root.join("retry.sqlite.gz");
        let final_path = root.join("part.sqlite.gz");
        fs::write(&first_temporary, b"prepared").expect("write prepared archive");
        publish_prepared_archive_file(&first_temporary, &final_path)
            .expect("publish first prepared archive");
        fs::write(&retry_temporary, b"prepared").expect("write retry archive");
        publish_prepared_archive_file(&retry_temporary, &final_path)
            .expect("reuse matching prepared archive");

        assert!(!first_temporary.exists());
        assert!(!retry_temporary.exists());
        assert_eq!(
            fs::read(&final_path).expect("read final archive"),
            b"prepared"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn archive_finalization_skips_full_vacuum() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-finalization-{}",
            retention_temp_suffix()
        ));
        let _root_cleanup = TempArchiveTestDirectory(root.clone());
        fs::create_dir_all(&root).expect("create archive root");
        let path = root.join("periodic-maintenance.sqlite");

        let mut connection = open_archive_sqlite_connection(&path)
            .await
            .expect("open archive sqlite file");
        sqlx::query("CREATE TABLE archive_payloads (payload BLOB NOT NULL)")
            .execute(&mut connection)
            .await
            .expect("create archive payload table");
        sqlx::query("INSERT INTO archive_payloads (payload) VALUES (?1)")
            .bind(vec![0_u8; 512 * 1024])
            .execute(&mut connection)
            .await
            .expect("insert archive payload");
        sqlx::query("DELETE FROM archive_payloads")
            .execute(&mut connection)
            .await
            .expect("delete archive payload");
        let free_pages_before: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut connection)
            .await
            .expect("read archive freelist before finalization");
        assert!(
            free_pages_before > 0,
            "fixture must contain reclaimable pages before archive finalization"
        );
        connection.close().await.expect("close archive sqlite file");

        finalize_archive_sqlite_file(&path)
            .await
            .expect("finalize archive sqlite file");

        let mut connection = open_archive_sqlite_connection(&path)
            .await
            .expect("reopen archive sqlite file");
        let free_pages_after: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut connection)
            .await
            .expect("read archive freelist after finalization");
        connection
            .close()
            .await
            .expect("close inspected archive sqlite file");
        let preserved_reclaimable_pages = free_pages_after > 0;

        assert!(
            preserved_reclaimable_pages,
            "periodic archive finalization must not run a full VACUUM"
        );
    }

    #[tokio::test]
    async fn legacy_archive_replacement_keeps_pending_cleanup_when_rename_fails() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let final_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-legacy-archive-replace-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_temp_path = final_path.with_extension("missing.tmp");
        fs::write(&final_path, b"old archive content").expect("write old archive file");
        let old_sha256 = sha256_hex_file(&final_path).expect("hash old archive file");
        let final_path_string = final_path.to_string_lossy().to_string();
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                cleanup_state,
                cleanup_source_safe_start_date,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', 'delete_pending', '2025-01-04', datetime('now'))
            "#,
        )
        .bind(&final_path_string)
        .bind(&old_sha256)
        .execute(&pool)
        .await
        .expect("insert pending archive manifest");

        replace_legacy_archive_file_with_cleanup_serialization(
            &pool,
            "codex_quota_snapshots",
            "2025-01",
            &missing_temp_path,
            &final_path,
        )
        .await
        .expect_err("missing replacement file must roll back pending reactivation");

        assert_eq!(
            fs::read(&final_path).expect("read original archive"),
            b"old archive content"
        );
        let manifest: (String, Option<String>) = sqlx::query_as(
            "SELECT cleanup_state, cleanup_source_safe_start_date FROM archive_batches WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load retained pending manifest");
        assert_eq!(manifest.0, ARCHIVE_CLEANUP_STATE_DELETE_PENDING);
        assert_eq!(manifest.1.as_deref(), Some("2025-01-04"));

        let _ = fs::remove_file(&final_path);
    }
}
