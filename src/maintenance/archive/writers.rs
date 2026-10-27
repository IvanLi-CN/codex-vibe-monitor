use super::archive_publication::{
    ArchiveReplacement, replace_legacy_archive_file_with_cleanup_serialization,
};
use super::*;
use anyhow::bail;
use sqlx::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use std::io::ErrorKind;
use std::str::FromStr;
#[cfg(test)]
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum MonthlyWriterPhase {
    Preflight,
    PredecessorDigest,
    PredecessorIdentity,
    Inflate,
    CopyAndSourceIdentity,
    Finalize,
    SummaryPages,
    Deflate,
    OutputDigest,
    Publication,
    Cleanup,
}

impl MonthlyWriterPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::PredecessorDigest => "predecessor_digest",
            Self::PredecessorIdentity => "predecessor_identity",
            Self::Inflate => "inflate",
            Self::CopyAndSourceIdentity => "copy_and_source_identity",
            Self::Finalize => "finalize",
            Self::SummaryPages => "summary_pages",
            Self::Deflate => "deflate",
            Self::OutputDigest => "output_digest",
            Self::Publication => "publication",
            Self::Cleanup => "cleanup",
        }
    }
}

struct MonthlyWriterObservation {
    dataset: &'static str,
    active: Option<(MonthlyWriterPhase, std::time::Instant)>,
}

impl MonthlyWriterObservation {
    fn new(dataset: &'static str) -> Self {
        Self {
            dataset,
            active: Some((MonthlyWriterPhase::Preflight, std::time::Instant::now())),
        }
    }

    fn advance(&mut self, phase: MonthlyWriterPhase) {
        self.finish("completed");
        self.active = Some((phase, std::time::Instant::now()));
    }

    fn finish(&mut self, outcome: &'static str) {
        if let Some((phase, started)) = self.active.take() {
            // Fixed fields only: no archive path, month, source identity, or error content.
            // A stopped observation may be an error or cancellation; it does not prove native
            // worker completion or resource release.
            info!(
                dataset = self.dataset,
                writer_phase = phase.as_str(),
                phase_outcome = outcome,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "monthly archive writer phase observed"
            );
        }
    }
}

impl Drop for MonthlyWriterObservation {
    fn drop(&mut self) {
        self.finish("stopped");
    }
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_TASK_ARCHIVE_PAUSE: Duration;
    pub(crate) static RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_PAUSE: Duration;
    pub(crate) static RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_COMMITTED: Arc<Notify>;
    pub(crate) static RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_REUSED: Arc<Notify>;
    pub(crate) static RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_STORES:
        Arc<std::sync::atomic::AtomicUsize>;
}

#[derive(Debug, Clone)]
struct TaskArchiveSnapshotPageLoad {
    pages: Vec<TaskArchiveSnapshotPage>,
    fingerprints: Vec<super::super::retention::TaskArchiveSnapshotPageFingerprint>,
    valid: bool,
}

pub(super) fn sync_published_archive_file(final_file_path: &Path) -> Result<()> {
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
    let Some(_file_phase) = crate::maintenance::try_acquire_file_phase_permit() else {
        return Err(super::super::retention::retention_write_deferred(
            "archive_publish_file",
        ));
    };
    let _archive_lock = super::super::retention::retention_archive_file_try_lock(final_file_path)?;
    match fs::hard_link(temporary_file_path, final_file_path) {
        Ok(()) => fs::remove_file(temporary_file_path).with_context(|| {
            format!(
                "failed to remove linked archive staging file {}",
                temporary_file_path.display()
            )
        })?,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let prepared_sha = sha256_hex_file_with_deadline(
                temporary_file_path,
                super::super::retention::retention_archive_file_operation_deadline(),
            )?;
            let existing_sha = sha256_hex_file_with_deadline(
                final_file_path,
                super::super::retention::retention_archive_file_operation_deadline(),
            )?;
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
    upstream_request_model: Option<String>,
    model_mapping_pattern: Option<String>,
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

fn archive_disk_attach_uri(path: &Path) -> Result<String> {
    // ATTACH inherits SQLITE_OPEN_MEMORY from a memory-backed source connection unless
    // its URI explicitly selects disk mode. Plain paths silently create a second memory DB.
    let absolute_path = fs::canonicalize(path).context("resolve task archive SQLite path")?;
    let mut uri = url::Url::from_file_path(absolute_path)
        .map_err(|()| anyhow::anyhow!("invalid task archive SQLite path"))?;
    // The work file already exists; request only read/write access.
    uri.query_pairs_mut().append_pair("mode", "rw");
    Ok(uri.to_string())
}

pub(crate) async fn finalize_archive_sqlite_file(path: &Path) -> Result<()> {
    let file_phase = super::super::acquire_file_phase_permit("archive_sqlite_finalize").await?;
    let path = path.to_path_buf();
    super::super::spawn_file_phase_task(file_phase, async move {
        let mut connection = open_archive_sqlite_connection(&path).await?;
        sqlx::query("PRAGMA optimize")
            .execute(&mut connection)
            .await
            .with_context(|| {
                format!("failed to finalize archive sqlite file {}", path.display())
            })?;
        connection.close().await?;
        Ok(())
    })
    .await
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
        ("upstream_request_model", "TEXT"),
        ("model_mapping_pattern", "TEXT"),
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
    sha256: Option<String>,
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
    let mut committed_cursor_id = start_after_batch_id;
    let mut committed_scanned_batches = 0_u64;
    let mut committed_updated_rows = 0_u64;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    let result = async {
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
            SELECT id, file_path, sha256
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
              AND status = ?1
              AND cleanup_state = ?4
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
        .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        for batch in rows {
            let Some(expected_existing_sha256) =
                batch.sha256.as_deref().filter(|sha| !sha.trim().is_empty())
            else {
                last_seen_batch_id = batch.id;
                summary.scanned_batches += 1;
                committed_cursor_id = batch.id;
                committed_scanned_batches = summary.scanned_batches;
                hit_budget = true;
                push_backfill_sample(
                    &mut samples,
                    format!("batch_id={} sha256_missing", batch.id),
                );
                continue;
            };
            last_seen_batch_id = batch.id;
            summary.scanned_batches += 1;
            let archive_path = PathBuf::from(&batch.file_path);
            let predecessor_matches = {
                let file_phase = super::super::acquire_file_phase_permit(
                    "archive_public_id_predecessor_verify",
                )
                .await?;
                let archive_lock = super::super::retention::retention_archive_file_lock_async(
                    &archive_path,
                )
                .await?;
                let matches = archive_path.is_file()
                    && sha256_hex_file_with_deadline(
                        &archive_path,
                        super::super::retention::retention_archive_file_operation_deadline(),
                    )? == expected_existing_sha256;
                drop(archive_lock);
                drop(file_phase);
                matches
            };
            if !predecessor_matches {
                return Err(anyhow::anyhow!(
                    "pool archive identity verification failed before public-id backfill"
                ));
            }
            let suffix = retention_temp_suffix();
            let work_path = PathBuf::from(format!("{}.{}.sqlite", batch.file_path, suffix));
            let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", batch.file_path, suffix));
            let _work_cleanup = TempSqliteCleanup::new(work_path.clone());
            let _gzip_cleanup = TempSqliteCleanup::new(temp_gzip_path.clone());
            remove_temp_sqlite_artifacts(&work_path);
            remove_temp_sqlite_artifacts(&temp_gzip_path);

            if let Err(error) =
                inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &work_path).await
            {
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                return Err(error);
            }

            let file_phase = super::super::acquire_file_phase_permit(
                "archive_public_id_file_phase_sqlite",
            )
            .await?;
            let worker_work_path = work_path.clone();
            let Some(file_operation) =
                super::super::try_acquire_archive_file_operation_lease(&worker_work_path)
            else {
                return Err(super::super::retention::retention_write_deferred(
                    "archive_public_id_file_phase_sqlite",
                ));
            };
            let backfill_outcome = super::super::spawn_file_phase_task(file_phase, async move {
                let _file_operation = file_operation;
                let mut conn = open_archive_sqlite_connection(&worker_work_path).await?;
                ensure_pool_upstream_request_attempts_archive_schema_direct(&mut conn).await?;
                let outcome = backfill_pool_upstream_request_attempt_public_ids_on_connection(
                    &mut conn, 0, None, None,
                )
                .await?;
                conn.close().await?;
                Ok::<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>, anyhow::Error>(
                    outcome,
                )
            })
            .await;

            let outcome = match backfill_outcome {
                Ok(outcome) => outcome,
                Err(err) => {
                    remove_temp_sqlite_artifacts(&work_path);
                    remove_temp_sqlite_artifacts(&temp_gzip_path);
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
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                return Err(err);
            }
            if let Err(err) =
                deflate_sqlite_file_to_gzip_with_operation_budget(&work_path, &temp_gzip_path)
                    .await
            {
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                return Err(err);
            }
            // Hash the prepared artifact before entering the bounded publication transaction.
            let sha256 = sha256_hex_file_with_operation_budget(&temp_gzip_path).await?;
            let had_existing_file = archive_path.is_file();
            let backup_path = PathBuf::from(format!(
                "{}.{}.restore",
                archive_path.display(),
                retention_temp_suffix()
            ));
            let staged_path_string = backup_path.to_string_lossy().to_string();
            let write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                )
                .await;
            let staged = sqlx::query(
                "UPDATE archive_batches
                 SET replacement_staged_path = ?1
                 WHERE id = ?2 AND sha256 = ?3 AND replacement_staged_path IS NULL",
            )
            .bind(&staged_path_string)
            .bind(batch.id)
            .bind(expected_existing_sha256)
            .execute(pool)
            .await?;
            if staged.rows_affected() != 1 {
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                return Err(anyhow::anyhow!(
                    "archive public-id backfill replacement staging conflict; retry batch"
                ));
            }
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
            let current_state = sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT sha256, replacement_staged_path
                 FROM archive_batches
                 WHERE id = ?1 AND dataset = 'pool_upstream_request_attempts'
                   AND file_path = ?2 AND status = ?3 AND cleanup_state = ?4",
            )
            .bind(batch.id)
            .bind(&batch.file_path)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
            .fetch_optional(tx.as_mut())
            .await?;
            let Some((current_sha256, existing_staged_path)) = current_state else {
                tx.rollback().await?;
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                continue;
            };
            if current_sha256.as_deref() != Some(expected_existing_sha256)
                || existing_staged_path.as_deref() != Some(staged_path_string.as_str())
            {
                tx.rollback().await?;
                remove_temp_sqlite_artifacts(&work_path);
                remove_temp_sqlite_artifacts(&temp_gzip_path);
                continue;
            }
            let Some(file_phase) = super::super::try_acquire_file_phase_permit() else {
                tx.rollback().await?;
                return Err(super::super::retention::retention_write_deferred(
                    "archive_public_id_file_phase",
                ));
            };
            if had_existing_file {
                fs::rename(&archive_path, &backup_path).with_context(|| {
                    format!(
                        "failed to stage archive replacement rollback copy: {}",
                        backup_path.display()
                    )
                })?;
            } else {
                fs::copy(&temp_gzip_path, &backup_path).with_context(|| {
                    format!(
                        "failed to stage archive replacement recovery copy: {}",
                        backup_path.display()
                    )
                })?;
            }
            if let Err(error) = fs::rename(&temp_gzip_path, &archive_path).with_context(|| {
                format!(
                    "failed to move pool_upstream_request_attempts archive batch into place: {} -> {}",
                    temp_gzip_path.display(),
                    archive_path.display()
                )
            }) {
                tx.rollback().await?;
                if had_existing_file {
                    let _ = fs::rename(&backup_path, &archive_path);
                } else {
                    let _ = fs::remove_file(&backup_path);
                }
                return Err(error);
            }
            if let Err(error) = sync_published_archive_file(&archive_path) {
                tx.rollback().await?;
                let _ = fs::remove_file(&archive_path);
                if had_existing_file {
                    let _ = fs::rename(&backup_path, &archive_path);
                } else {
                    let _ = fs::remove_file(&backup_path);
                    let _ = sqlx::query(
                        "UPDATE archive_batches SET replacement_staged_path = NULL
                         WHERE id = ?1 AND replacement_staged_path = ?2",
                    )
                    .bind(batch.id)
                    .bind(&staged_path_string)
                    .execute(pool)
                    .await;
                }
                return Err(error);
            }
            drop(file_phase);
            remove_temp_sqlite_artifacts(&work_path);

            if let Err(error) = sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE id = ?2")
                .bind(&sha256)
                .bind(batch.id)
                .execute(tx.as_mut())
                .await
            {
                tx.rollback().await?;
                let Some(compensation_file_phase) =
                    super::super::try_acquire_file_phase_permit()
                else {
                    return Err(anyhow::Error::new(error).context(
                        "archive public-id backfill compensation deferred while the file phase is busy",
                    ));
                };
                let _ = fs::remove_file(&archive_path);
                if had_existing_file {
                    let _ = fs::rename(&backup_path, &archive_path);
                } else {
                    let _ = fs::remove_file(&backup_path);
                }
                drop(compensation_file_phase);
                return Err(error.into());
            }
            if let Err(error) = tx.commit().await {
                return Err(anyhow::anyhow!(
                    "archive public-id backfill commit outcome is unknown; recovery will reconcile: {error}"
                ));
            }
            drop(write_permit);
            committed_cursor_id = batch.id;
            committed_scanned_batches = summary.scanned_batches;
            committed_updated_rows = summary.updated_rows;
            let cleanup_file_phase =
                super::super::acquire_file_phase_permit("archive_public_id_cleanup").await?;
            let _ = fs::remove_file(&backup_path);
            drop(cleanup_file_phase);
            let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                )
                .await;
            let _ = sqlx::query(
                "UPDATE archive_batches SET replacement_staged_path = NULL
                 WHERE id = ?1 AND replacement_staged_path = ?2",
            )
            .bind(batch.id)
            .bind(backup_path.to_string_lossy().to_string())
            .execute(pool)
            .await;
        }
        }

        Ok(BackfillBatchOutcome {
            summary,
            next_cursor_id: committed_cursor_id,
            hit_budget,
            samples,
        })
    }
    .await;
    result.map_err(|error| {
        anyhow::Error::new(crate::BackfillPartialFailure {
            source: error,
            next_cursor_id: committed_cursor_id,
            scanned: committed_scanned_batches,
            updated: committed_updated_rows,
        })
    })
}

pub(crate) async fn archive_pool_upstream_request_attempt_rows_into_month_batch(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    ids: &[i64],
    work_path: &Path,
) -> Result<(i64, Vec<(i64, String)>, String)> {
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
    let file_phase = super::super::acquire_file_phase_permit("archive_month_batch_copy").await?;
    let work_path = work_path.to_path_buf();
    let ids = ids.to_vec();
    let Some(file_operation) = super::super::try_acquire_archive_file_operation_lease(&work_path)
    else {
        return Err(super::super::retention::retention_write_deferred(
            "archive_month_batch_copy",
        ));
    };
    super::super::spawn_file_phase_task(file_phase, async move {
        let _file_operation = file_operation;
        let mut conn = open_archive_sqlite_connection(&work_path).await?;
        sqlx::query(&create_sql)
            .execute(&mut conn)
            .await
            .context("failed to ensure direct pool_upstream_request_attempts archive schema")?;
        ensure_pool_upstream_request_attempts_archive_schema_direct(&mut conn).await?;

        let existing = sqlx::query_as::<_, (i64, String, Option<String>)>(
            "SELECT id, invoke_id, attempt_public_id FROM pool_upstream_request_attempts WHERE id IN (SELECT value FROM json_each(?1))",
        ).bind(serde_json::to_string(&ids)?).fetch_all(&mut conn).await?;
        for (id, invoke_id, public_id) in existing {
            let source = rows
                .iter()
                .find(|row| row.id == id)
                .context("archive source row disappeared")?;
            if source.invoke_id != invoke_id || source.attempt_public_id != public_id {
                bail!("monthly attempt archive row identity collision");
            }
        }

        for chunk in rows.chunks(16) {
            let mut insert = QueryBuilder::<Sqlite>::new(format!(
                "INSERT OR REPLACE INTO {} ({}) ",
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
                    .push_bind(&row.upstream_request_model)
                    .push_bind(&row.model_mapping_pattern)
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
        let source_identity = archive_table_source_identity_sha256(&mut conn, spec, "main", &ids)
            .await?;
        conn.close().await?;
        Ok((row_count, Vec::new(), source_identity))
    })
    .await
}

pub(crate) async fn archive_rows_into_month_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    let final_path = archive_batch_file_path(config, spec.dataset, month_key)?;
    archive_rows_into_month_batch_at_path(pool, spec, month_key, ids, final_path).await
}

pub(crate) async fn archive_rows_into_month_batch_at_path(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
    final_path: PathBuf,
) -> Result<ArchiveBatchOutcome> {
    archive_rows_into_month_batch_with_snapshots(pool, spec, month_key, ids, final_path, None, None)
        .await
}

pub(crate) async fn archive_rows_into_task_month_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
    snapshots: &mut Vec<TaskArchiveSnapshotPage>,
) -> Result<ArchiveBatchOutcome> {
    archive_rows_into_month_batch_with_snapshots(
        pool,
        spec,
        month_key,
        ids,
        archive_batch_file_path(config, spec.dataset, month_key)?,
        Some(snapshots),
        Some(shanghai_archive_expiry_from_reference_timestamp(
            &format_utc_iso(Utc::now()),
            config.invocation_archive_ttl_days,
        )?),
    )
    .await
}

async fn archive_rows_into_month_batch_with_snapshots(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
    final_path: PathBuf,
    snapshots: Option<&mut Vec<TaskArchiveSnapshotPage>>,
    archive_expires_at: Option<String>,
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive batch requires at least one row id");
    }
    let mut writer_observation = MonthlyWriterObservation::new(spec.dataset);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    // Interrupted work must release disk space even when preflight would otherwise refuse
    // to start a new archive batch. Contents are discarded, never used as a continuation.
    super::archive_task_work::discard_abandoned_task_work(&final_path)?;
    if !super::super::retention::archive_file_can_start(pool, spec, &final_path, ids).await? {
        return Err(super::super::retention::retention_write_deferred(
            "archive_batch_planning",
        ));
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!("{}.task-{}.sqlite", final_path.display(), suffix));
    let temp_gzip_path = PathBuf::from(format!("{}.task-{}.tmp", final_path.display(), suffix));
    let _work_owner_file =
        super::archive_task_work::create_task_work_file(&final_path, &work_path)?;
    // Cleanup is fenced while a blocking archive worker still owns the destination. Its error
    // path removes partial output before the next retention run can reuse the work directory.
    let _work_cleanup = TempSqliteCleanup::new(work_path.clone());
    let _gzip_cleanup = TempSqliteCleanup::new(temp_gzip_path.clone());
    writer_observation.advance(MonthlyWriterPhase::PredecessorDigest);
    let existing_final_sha256 = if final_path.exists() {
        Some(sha256_hex_file_with_deadline(
            &final_path,
            super::super::retention::retention_archive_file_operation_deadline(),
        )?)
    } else {
        None
    };
    writer_observation.advance(MonthlyWriterPhase::PredecessorIdentity);
    if spec.dataset == "codex_invocations"
        && let Some(existing_sha256) = existing_final_sha256.as_deref()
    {
        let known_manifest_sha256 = sqlx::query_scalar::<_, String>(
            "SELECT sha256 FROM archive_batches \
                 WHERE dataset = ?1 AND file_path = ?2 AND sha256 IS NOT NULL AND sha256 <> '' \
                 ORDER BY id DESC LIMIT 1",
        )
        .bind(spec.dataset)
        .bind(final_path.to_string_lossy().to_string())
        .fetch_optional(pool)
        .await?;
        let known_prepared_sha256 = sqlx::query_scalar::<_, String>(
            "SELECT artifact_sha256 FROM retention_prepared_archives \
                 WHERE dataset = ?1 AND file_path = ?2 AND artifact_sha256 IS NOT NULL \
                   AND state IN ('preparing', 'published') \
                 ORDER BY updated_at DESC, id DESC LIMIT 1",
        )
        .bind(spec.dataset)
        .bind(final_path.to_string_lossy().to_string())
        .fetch_optional(pool)
        .await?;
        for known_sha256 in [known_manifest_sha256, known_prepared_sha256]
            .into_iter()
            .flatten()
        {
            if known_sha256 != existing_sha256 {
                bail!(
                    "legacy archive existing artifact digest does not match its recorded identity"
                );
            }
        }
    }

    if temp_gzip_path.exists() {
        remove_temp_sqlite_artifacts(&temp_gzip_path);
    }

    let has_summary_pages = snapshots.is_some();
    writer_observation.advance(MonthlyWriterPhase::Inflate);
    if final_path.exists() {
        inflate_gzip_sqlite_file_with_operation_budget(&final_path, &work_path).await?;
    } else {
        ensure_attachable_archive_sqlite_path(&work_path)?;
    }
    writer_observation.advance(MonthlyWriterPhase::CopyAndSourceIdentity);
    let row_count = if spec.dataset == "pool_upstream_request_attempts" {
        async {
            let (count, activity, identity) =
                archive_pool_upstream_request_attempt_rows_into_month_batch(
                    pool, spec, ids, &work_path,
                )
                .await?;
            Ok::<_, anyhow::Error>((count, activity, Some(identity), false, None))
        }
        .await
    } else {
        let file_phase =
            super::super::acquire_file_phase_permit("archive_month_batch_copy").await?;
        let worker_connect_options = pool.connect_options().clone();
        let worker_spec = spec;
        let worker_ids = ids.to_vec();
        let worker_work_path = work_path.clone();
        let worker_has_summary_pages = has_summary_pages;
        let worker_has_existing_final_sha256 = existing_final_sha256.is_some();
        let test_archive_pause: Option<(Duration, Option<Duration>)> = {
            #[cfg(test)]
            {
                RETENTION_TEST_TASK_ARCHIVE_PAUSE
                    .try_with(|delay| {
                        let mut budget = super::super::retention::retention_run_remaining_budget();
                        #[cfg(test)]
                        if let Ok(quota_budget) = super::RETENTION_TEST_QUOTA_ARCHIVE_PREPARE_BUDGET
                            .try_with(|budget| *budget)
                        {
                            budget = Some(
                                budget
                                    .map(|current| current.min(quota_budget))
                                    .unwrap_or(quota_budget),
                            );
                        }
                        (*delay, budget)
                    })
                    .ok()
            }
            #[cfg(not(test))]
            {
                None
            }
        };
        let Some(file_operation) =
            super::super::try_acquire_archive_file_operation_lease(&worker_work_path)
        else {
            return Err(super::super::retention::retention_write_deferred(
                "archive_month_batch_copy",
            ));
        };
        super::super::spawn_file_phase_task(file_phase, async move {
            // An ATTACH must not survive cancellation on a pooled connection. Open a dedicated
            // connection with the pool options so pooled handles remain available; the native
            // worker owns the connection and the file permit until it has drained.
            let _file_operation = file_operation;
            let mut conn = SqliteConnection::connect_with(&worker_connect_options).await?;
            sqlx::query("ATTACH DATABASE ?1 AS archive_db")
                .bind(archive_disk_attach_uri(&worker_work_path)?)
                .execute(&mut conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to attach archive database {}",
                        worker_work_path.display()
                    )
                })?;
            sqlx::query("PRAGMA archive_db.journal_mode=DELETE")
                .execute(&mut conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to switch archive database {} to DELETE journal mode",
                        worker_work_path.display()
                    )
                })?;
            sqlx::query(worker_spec.create_sql)
                .execute(&mut conn)
                .await
                .with_context(|| {
                    format!("failed to ensure archive schema for {}", worker_spec.dataset)
                })?;
            if worker_spec.dataset == "codex_invocations" {
                ensure_codex_invocations_archive_schema(&mut conn).await?;
            } else if worker_spec.dataset == "pool_upstream_request_attempts" {
                ensure_pool_upstream_request_attempts_archive_schema(&mut conn).await?;
            }
            let predecessor_row_count = if worker_has_summary_pages
                && worker_has_existing_final_sha256
            {
                Some(
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM archive_db.codex_invocations",
                    )
                    .fetch_one(&mut conn)
                    .await?,
                )
            } else {
                None
            };
            if let Some((delay, remaining_budget)) = test_archive_pause {
                let pause = remaining_budget
                    .map(|remaining| {
                        delay.min(remaining.saturating_sub(Duration::from_millis(500)))
                    })
                    .unwrap_or(delay);
                if !pause.is_zero() {
                    tokio::time::sleep(pause).await;
                }
                if remaining_budget.is_some_and(|remaining| delay >= remaining) {
                    return Err(super::super::retention::retention_write_deferred(
                        "archive_month_batch_copy",
                    ));
                }
            }

            let upstream_last_activity = if worker_spec.dataset == "codex_invocations" {
                let mut rows = Vec::new();
                for chunk in worker_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
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
                            .fetch_all(&mut conn)
                            .await
                            .context("read task-local source account activity")?,
                    );
                }
                dedupe_archive_upstream_last_activity(
                    rows.into_iter()
                        .map(|row| (row.account_id, row.last_activity_at)),
                )
            } else {
                Vec::new()
            };

            // A previously published copy may still have a live source after a failed run.
            // Recopy that live row, but reject ID reuse across different immutable identities.
            let identity_columns = match worker_spec.dataset {
                "codex_invocations" => &["invoke_id"][..],
                "forward_proxy_attempts" => &["proxy_key", "occurred_at"][..],
                "codex_quota_snapshots" => &["captured_at"][..],
                _ => bail!("unsupported task-local monthly archive dataset"),
            };
            let identity_mismatch = identity_columns
                .iter()
                .map(|column| format!("a.{column} IS NOT s.{column}"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let ids_json = serde_json::to_string(&worker_ids)?;
            let collision: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM archive_db.{} a JOIN main.{} s ON a.id=s.id WHERE s.id IN (SELECT value FROM json_each(?1)) AND ({identity_mismatch}))",
                worker_spec.dataset, worker_spec.dataset,
            ))
            .bind(&ids_json)
            .fetch_one(&mut conn)
            .await?;
            if collision {
                bail!("monthly archive row identity collision");
            }
            let should_copy = if worker_spec.dataset == "codex_invocations" {
                let source_identity =
                    super::super::retention::invocation_archive_source_identity_sha256(
                        &mut conn,
                        super::super::retention::InvocationArchiveIdentityDatabase::Main,
                        &worker_ids,
                    )
                    .await?;
                let archive_row_count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM archive_db.codex_invocations
                     WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .bind(&ids_json)
                .fetch_one(&mut conn)
                .await?;
                if archive_row_count != worker_ids.len() as i64 {
                    true
                } else {
                    let archive_identity =
                        super::super::retention::invocation_archive_source_identity_sha256(
                            &mut conn,
                            super::super::retention::InvocationArchiveIdentityDatabase::Archive,
                            &worker_ids,
                        )
                        .await?;
                    archive_identity != source_identity
                }
            } else {
                true
            };
            if should_copy {
                let mut insert = QueryBuilder::<Sqlite>::new(format!(
                    "INSERT OR REPLACE INTO archive_db.{} ({}) SELECT {} FROM main.{} WHERE id IN (",
                    worker_spec.dataset,
                    worker_spec.columns,
                    worker_spec.columns,
                    worker_spec.dataset
                ));
                {
                    let mut separated = insert.separated(", ");
                    for id in &worker_ids {
                        separated.push_bind(id);
                    }
                }
                insert.push(")");
                insert.build().execute(&mut conn).await.with_context(|| {
                    format!(
                        "failed to copy rows into archive batch for {}",
                        worker_spec.dataset
                    )
                })?;
            }

            let count_query = format!("SELECT COUNT(*) FROM archive_db.{}", worker_spec.dataset);
            let row_count = sqlx::query_scalar::<_, i64>(&count_query)
                .fetch_one(&mut conn)
                .await
                .with_context(|| {
                    format!("failed to count archive rows for {}", worker_spec.dataset)
                })?;
            let source_identity_sha256 = if worker_spec.dataset == "codex_invocations" {
                Some(
                    super::super::retention::invocation_archive_source_identity_sha256(
                        &mut conn,
                        super::super::retention::InvocationArchiveIdentityDatabase::Archive,
                        &worker_ids,
                    )
                    .await?,
                )
            } else {
                Some(
                    archive_table_source_identity_sha256(
                        &mut conn,
                        worker_spec,
                        "archive_db",
                        &worker_ids,
                    )
                    .await?,
                )
            };
            sqlx::query("DETACH DATABASE archive_db")
                .execute(&mut conn)
                .await
                .context("failed to detach archive database")?;
            conn.close().await?;
            Ok::<
                (
                    i64,
                    Vec<(i64, String)>,
                    Option<String>,
                    bool,
                    Option<i64>,
                ),
                anyhow::Error,
            >((
                row_count,
                upstream_last_activity,
                source_identity_sha256,
                worker_has_summary_pages && !should_copy && worker_has_existing_final_sha256,
                predecessor_row_count,
            ))
        })
        .await
    };

    let (
        result,
        upstream_last_activity,
        source_identity_sha256,
        archive_content_unchanged,
        predecessor_row_count,
    ) = match row_count {
        Ok(values) => values,
        Err(err) => {
            remove_temp_sqlite_artifacts(&work_path);
            remove_temp_sqlite_artifacts(&temp_gzip_path);
            return Err(err);
        }
    };
    if has_summary_pages {
        writer_observation.advance(MonthlyWriterPhase::SummaryPages);
        prepare_task_month_snapshot_index(&work_path).await?;
    }
    writer_observation.advance(MonthlyWriterPhase::Finalize);
    if let Err(err) = finalize_archive_sqlite_file(&work_path).await {
        remove_temp_sqlite_artifacts(&work_path);
        remove_temp_sqlite_artifacts(&temp_gzip_path);
        return Err(err);
    }

    writer_observation.advance(MonthlyWriterPhase::Deflate);
    if archive_content_unchanged {
        let file_phase = super::super::acquire_file_phase_permit("archive_unchanged_stage").await?;
        if let Err(err) = fs::hard_link(&final_path, &temp_gzip_path).with_context(|| {
            format!(
                "failed to retain unchanged archive artifact {}",
                final_path.display()
            )
        }) {
            drop(file_phase);
            remove_temp_sqlite_artifacts(&work_path);
            remove_temp_sqlite_artifacts(&temp_gzip_path);
            return Err(err);
        }
        drop(file_phase);
    } else if let Err(err) =
        deflate_sqlite_file_to_gzip_with_operation_budget(&work_path, &temp_gzip_path).await
    {
        remove_temp_sqlite_artifacts(&work_path);
        remove_temp_sqlite_artifacts(&temp_gzip_path);
        return Err(err);
    }
    writer_observation.advance(MonthlyWriterPhase::OutputDigest);
    let sha256 = if archive_content_unchanged {
        existing_final_sha256
            .clone()
            .context("unchanged archive artifact is missing its predecessor digest")?
    } else {
        sha256_hex_file_with_operation_budget(&temp_gzip_path).await?
    };
    let mut outcome = ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key: month_key.to_string(),
        day_key: None,
        part_key: None,
        file_path: final_path.to_string_lossy().to_string(),
        sha256: sha256.clone(),
        source_identity_sha256,
        row_count: result,
        upstream_last_activity,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at,
        summary_source_kind: if has_summary_pages {
            SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
        } else {
            SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN
        },
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    };
    let snapshot_batch_id = if has_summary_pages {
        Some(
            super::super::retention::stage_task_summary_archive_manifest(
                pool,
                &outcome,
                existing_final_sha256.as_deref(),
                predecessor_row_count,
            )
            .await?
            .ok_or_else(|| {
                super::super::retention::retention_write_deferred("archive_snapshot_manifest")
            })?,
        )
    } else {
        None
    };
    writer_observation.advance(MonthlyWriterPhase::Publication);
    if let Err(err) = replace_legacy_archive_file_with_cleanup_serialization(
        pool,
        spec.dataset,
        month_key,
        &temp_gzip_path,
        &final_path,
        existing_final_sha256.as_deref(),
        ArchiveReplacement {
            row_count: Some(result),
            sha256: &sha256,
        },
    )
    .await
    {
        remove_temp_sqlite_artifacts(&work_path);
        remove_temp_sqlite_artifacts(&temp_gzip_path);
        return Err(err);
    }
    if let (Some(batch_id), Some(snapshots)) = (snapshot_batch_id, snapshots) {
        writer_observation.advance(MonthlyWriterPhase::SummaryPages);
        // The monthly target contains previous committed rows as well as this batch. Persist
        // each complete V2 page after the artifact is published, so cancellation leaves a
        // resumable prefix without granting cleanup authority before final proof.
        *snapshots =
            prepare_task_month_snapshot_pages(pool, &work_path, &outcome, batch_id).await?;
        if snapshots
            .iter()
            .map(|page| i64::from(page.row_count))
            .sum::<i64>()
            != result
        {
            bail!("monthly archive Summary snapshot row count mismatch");
        }
    }
    writer_observation.advance(MonthlyWriterPhase::Cleanup);
    let cleanup_complete = match super::super::try_acquire_file_phase_permit() {
        Some(_file_phase) => match fs::remove_file(&work_path) {
            Ok(()) => true,
            Err(error) => error.kind() == ErrorKind::NotFound,
        },
        None => false,
    };
    writer_observation.finish(if cleanup_complete {
        "completed"
    } else {
        "stopped"
    });

    outcome.sha256 = sha256;
    Ok(outcome)
}

async fn prepare_task_month_snapshot_index(work_path: &Path) -> Result<()> {
    let file_phase = super::super::acquire_file_phase_permit("archive_snapshot_index").await?;
    let work_path = work_path.to_path_buf();
    super::super::spawn_file_phase_task(file_phase, async move {
        let options = SqliteConnectOptions::from_str(&sqlite_url_for_path(&work_path))?;
        let archive_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let result = sqlx::query("CREATE INDEX IF NOT EXISTS idx_retention_snapshot_time_id ON codex_invocations(julianday(occurred_at), id)")
            .execute(&archive_pool)
            .await
            .map(|_| ());
        archive_pool.close().await;
        result.map_err(Into::into)
    })
    .await
}

async fn run_archive_sqlite_file_phase<T, F>(operation: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let file_phase = super::super::acquire_file_phase_permit(operation).await?;
    super::super::spawn_file_phase_task(file_phase, work).await
}

async fn load_task_month_snapshot_pages(
    pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    first_page_query: &str,
    next_page_query: &str,
    batch_id: i64,
    manifest_sha256: &str,
) -> Result<TaskArchiveSnapshotPageLoad> {
    let rows = sqlx::query_as::<_, (i64, String, String, i64, Vec<u8>, i64, String, i64)>(
        "SELECT page_index, coverage_start, coverage_end, row_count, payload, format_version,
                snapshot_sha256, payload_bytes
         FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2
         ORDER BY page_index ASC",
    )
    .bind(batch_id)
    .bind(manifest_sha256)
    .fetch_all(pool)
    .await?;
    let fingerprints = rows
        .iter()
        .map(
            |(
                page_index,
                coverage_start,
                coverage_end,
                row_count,
                payload,
                format_version,
                _snapshot_sha256,
                payload_bytes,
            )| super::super::retention::TaskArchiveSnapshotPageFingerprint {
                page_index: *page_index,
                coverage_start: coverage_start.clone(),
                coverage_end: coverage_end.clone(),
                row_count: *row_count,
                payload_bytes: *payload_bytes,
                payload_sha256:
                    crate::summary_source_change::summary_archive_snapshot_payload_sha256(payload),
                format_version: *format_version,
            },
        )
        .collect::<Vec<_>>();
    let mut pages = Vec::with_capacity(rows.len());
    let mut previous_cursor: Option<(String, i64)> = None;
    for (
        page_index,
        coverage_start,
        coverage_end,
        row_count,
        payload,
        format_version,
        _snapshot_sha256,
        _payload_bytes,
    ) in rows
    {
        if page_index != pages.len() as i64
            || format_version != SUMMARY_ARCHIVE_SNAPSHOT_V2
            || row_count <= 0
        {
            return Ok(TaskArchiveSnapshotPageLoad {
                pages: Vec::new(),
                fingerprints,
                valid: false,
            });
        }
        let records = match crate::summary_source_change::decode_summary_archive_snapshot_v2_payload(
            &payload,
        ) {
            Ok(records) => records,
            Err(_) => {
                return Ok(TaskArchiveSnapshotPageLoad {
                    pages: Vec::new(),
                    fingerprints,
                    valid: false,
                });
            }
        };
        let expected_row_count = match usize::try_from(row_count) {
            Ok(row_count) => row_count,
            Err(_) => {
                return Ok(TaskArchiveSnapshotPageLoad {
                    pages: Vec::new(),
                    fingerprints,
                    valid: false,
                });
            }
        };
        if records.len() != expected_row_count {
            return Ok(TaskArchiveSnapshotPageLoad {
                pages: Vec::new(),
                fingerprints,
                valid: false,
            });
        }
        let archive_rows = if let Some((occurred_at, row_id)) = previous_cursor.as_ref() {
            let query = next_page_query.to_owned();
            let archive_pool = archive_pool.clone();
            let occurred_at = occurred_at.clone();
            let row_id = *row_id;
            run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
                Ok::<_, anyhow::Error>(
                    sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
                        .bind(occurred_at)
                        .bind(row_id)
                        .bind(row_count)
                        .fetch_all(&archive_pool)
                        .await?,
                )
            })
            .await?
        } else {
            let query = first_page_query.to_owned();
            let archive_pool = archive_pool.clone();
            run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
                Ok::<_, anyhow::Error>(
                    sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
                        .bind(row_count)
                        .fetch_all(&archive_pool)
                        .await?,
                )
            })
            .await?
        };
        let record_ids = records.iter().map(|record| record.id).collect::<Vec<_>>();
        let archive_ids = archive_rows.iter().map(|row| row.id).collect::<Vec<_>>();
        if archive_ids != record_ids
            || archive_rows
                .first()
                .is_none_or(|row| row.occurred_at != coverage_start)
            || archive_rows
                .last()
                .is_none_or(|row| row.occurred_at != coverage_end)
        {
            return Ok(TaskArchiveSnapshotPageLoad {
                pages: Vec::new(),
                fingerprints,
                valid: false,
            });
        }
        let ids = archive_ids;
        let ids_json = serde_json::to_string(&ids)?;
        let archive_pool = archive_pool.clone();
        let invoke_ids = run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
            Ok::<_, anyhow::Error>(
                sqlx::query_as::<_, (i64, String)>(
                    "SELECT id, invoke_id FROM codex_invocations WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .bind(ids_json)
                .fetch_all(&archive_pool)
                .await?
                .into_iter()
                .collect::<HashMap<_, _>>(),
            )
        })
        .await?;
        if invoke_ids.len() != archive_rows.len()
            || super::super::retention::encode_summary_archive_snapshot_v2_payload(
                &archive_rows,
                &invoke_ids,
            )? != payload
        {
            return Ok(TaskArchiveSnapshotPageLoad {
                pages: Vec::new(),
                fingerprints,
                valid: false,
            });
        }
        previous_cursor = archive_rows
            .last()
            .map(|row| (row.occurred_at.clone(), row.id));
        pages.push(TaskArchiveSnapshotPage {
            coverage_start,
            coverage_end,
            row_count: u32::try_from(row_count)?,
            payload,
        });
    }
    Ok(TaskArchiveSnapshotPageLoad {
        pages,
        fingerprints,
        valid: true,
    })
}

async fn prepare_task_month_snapshot_pages(
    pool: &Pool<Sqlite>,
    work_path: &Path,
    outcome: &ArchiveBatchOutcome,
    batch_id: i64,
) -> Result<Vec<TaskArchiveSnapshotPage>> {
    let work_path = work_path.to_path_buf();
    let archive_pool = run_archive_sqlite_file_phase("archive_snapshot_open", async move {
        let options = SqliteConnectOptions::from_str(&sqlite_url_for_path(&work_path))?;
        Ok::<_, anyhow::Error>(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await?,
        )
    })
    .await?;
    let result = async {
        let columns = {
            let archive_pool = archive_pool.clone();
            run_archive_sqlite_file_phase("archive_snapshot_schema_read", async move {
                load_archive_table_columns(&archive_pool, "codex_invocations").await
            })
            .await?
        };
        // Tuple keyset pagination must seek rather than sort/scan the month once per 400-row
        // page. The index was installed before the artifact was compressed.
        let base = build_invocation_archive_rows_chunk_query(&columns);
        let first_query = base.replace(
            "WHERE id > ?1\n        ORDER BY id ASC\n        LIMIT ?2",
            "WHERE julianday(occurred_at) IS NOT NULL\n        ORDER BY julianday(occurred_at), id\n        LIMIT ?1",
        );
        let next_query = base.replace(
            "WHERE id > ?1\n        ORDER BY id ASC\n        LIMIT ?2",
            "WHERE (julianday(occurred_at), id) > (julianday(?1), ?2)\n        ORDER BY julianday(occurred_at), id\n        LIMIT ?3",
        );
        let loaded_pages = load_task_month_snapshot_pages(
            pool,
            &archive_pool,
            &first_query,
            &next_query,
            batch_id,
            &outcome.sha256,
        )
        .await?;
        let mut pages = if loaded_pages.valid {
            loaded_pages.pages
        } else {
            if super::super::retention::clear_task_summary_snapshot_pages(
                pool,
                batch_id,
                &outcome.sha256,
                &loaded_pages.fingerprints,
            )
            .await?
            .is_none()
            {
                return Err(super::super::retention::retention_write_deferred(
                    "archive_snapshot_page_reset",
                ));
            }
            Vec::new()
        };
        #[cfg(test)]
        if !pages.is_empty()
            && let Ok(notify) = RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_REUSED.try_with(Arc::clone)
        {
            notify.notify_one();
        }
        let (mut occurred_at, mut row_id) = if let Some(page) = pages.last() {
            let records = crate::summary_source_change::decode_summary_archive_snapshot_v2_payload(
                &page.payload,
            )?;
            let last = records.last().expect("nonempty persisted snapshot page");
            (Some(last.occurred_at.clone()), last.id)
        } else {
            (None, 0)
        };
        loop {
            let rows = if let Some(cursor) = occurred_at.as_deref() {
                let query = next_query.clone();
                let archive_pool = archive_pool.clone();
                let cursor = cursor.to_owned();
                run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
                    Ok::<_, anyhow::Error>(
                        sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
                            .bind(cursor)
                            .bind(row_id)
                            .bind(SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS as i64)
                            .fetch_all(&archive_pool)
                            .await?,
                    )
                })
                .await?
            } else {
                let query = first_query.clone();
                let archive_pool = archive_pool.clone();
                run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
                    Ok::<_, anyhow::Error>(
                        sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
                            .bind(SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS as i64)
                            .fetch_all(&archive_pool)
                            .await?,
                    )
                })
                .await?
            };
            if rows.is_empty() { break; }
            let mut identities = QueryBuilder::<Sqlite>::new(
                "SELECT id, invoke_id FROM codex_invocations WHERE id IN (",
            );
            let mut ids = identities.separated(", ");
            for row in &rows { ids.push_bind(row.id); }
            ids.push_unseparated(")");
            let archive_pool = archive_pool.clone();
            let invoke_ids = run_archive_sqlite_file_phase("archive_snapshot_page_read", async move {
                Ok::<_, anyhow::Error>(
                    identities
                        .build_query_as::<(i64, String)>()
                        .fetch_all(&archive_pool)
                        .await?
                        .into_iter()
                        .collect::<HashMap<_, _>>(),
                )
            })
            .await?;
            let first = rows.first().expect("nonempty snapshot page");
            let last = rows.last().expect("nonempty snapshot page");
            let page = TaskArchiveSnapshotPage {
                coverage_start: first.occurred_at.clone(),
                coverage_end: last.occurred_at.clone(),
                row_count: u32::try_from(rows.len())?,
                payload: super::super::retention::encode_summary_archive_snapshot_v2_payload(
                    &rows, &invoke_ids,
                )?,
            };
            let page_index = u32::try_from(pages.len())?;
            let stored = super::super::retention::store_task_summary_snapshot_page(
                pool,
                outcome,
                batch_id,
                page_index,
                page.clone(),
            )
            .await?;
            if stored.is_none() {
                return Err(super::super::retention::retention_write_deferred(
                    "archive_snapshot_page",
                ));
            }
            #[cfg(test)]
            if let Ok(counter) = RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_STORES
                .try_with(Arc::clone)
            {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            pages.push(page);
            #[cfg(test)]
            if pages.len() == 1 {
                if let Ok(notify) = RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_COMMITTED
                    .try_with(Arc::clone)
                {
                    notify.notify_one();
                }
                if let Ok(delay) = RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_PAUSE
                    .try_with(|delay| *delay)
                {
                    tokio::time::sleep(delay).await;
                }
            }
            occurred_at = Some(last.occurred_at.clone());
            row_id = last.id;
            if super::super::retention::retention_run_remaining_budget()
                .is_some_and(|remaining| remaining.is_zero()) {
                return Err(super::super::retention::retention_write_deferred(
                    "archive_snapshot_page",
                ));
            }
            if super::super::retention::retention_run_shutdown_token()
                .is_some_and(|shutdown| shutdown.is_cancelled())
            {
                return Err(super::super::retention::retention_write_deferred(
                    "archive_snapshot_page",
                ));
            }
        }
        Ok::<_, anyhow::Error>(pages)
    }.await;
    archive_pool.close().await;
    result
}

pub(crate) async fn archive_rows_into_segment_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    day_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    let part_key = archive_segment_part_key_for_ids(ids)?;
    let final_path = archive_segment_file_path(
        config,
        spec.dataset,
        day_key,
        &part_key,
        config.invocation_archive_codec,
    )?;
    archive_rows_into_segment_batch_at_path(pool, config, spec, day_key, ids, final_path).await
}

pub(crate) async fn archive_rows_into_segment_batch_at_path(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    day_key: &str,
    ids: &[i64],
    final_path: PathBuf,
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive segment requires at least one row id");
    }
    if spec.dataset != "codex_invocations" {
        bail!("archive segment writer only supports codex_invocations");
    }
    let month_key = archive_month_key_from_day_key(day_key)?;
    let part_key = archive_segment_part_key_for_ids(ids)?;
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
    let _temp_cleanup = TempSqliteCleanup::new(work_path.clone());
    let _gzip_cleanup = TempSqliteCleanup::new(temp_gzip_path.clone());
    ensure_attachable_archive_sqlite_path(&work_path)?;
    let file_phase = super::super::acquire_file_phase_permit("archive_segment_copy").await?;
    let worker_pool = pool.clone();
    let worker_ids = ids.to_vec();
    let worker_work_path = work_path.clone();
    let Some(file_operation) =
        super::super::try_acquire_archive_file_operation_lease(&worker_work_path)
    else {
        return Err(super::super::retention::retention_write_deferred(
            "archive_segment_copy",
        ));
    };
    let row_count = super::super::spawn_file_phase_task(file_phase, async move {
        let _file_operation = file_operation;
        let mut conn = worker_pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ?1 AS archive_db")
            .bind(archive_disk_attach_uri(&worker_work_path)?)
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!(
                    "failed to attach archive database {}",
                    worker_work_path.display()
                )
            })?;
        sqlx::query("PRAGMA archive_db.journal_mode=DELETE")
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!(
                    "failed to switch archive database {} to DELETE journal mode",
                    worker_work_path.display()
                )
            })?;
        sqlx::query(spec.create_sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to ensure archive schema for {}", spec.dataset))?;
        ensure_codex_invocations_archive_schema(&mut conn).await?;

        let mut upstream_last_activity = Vec::new();
        for chunk in worker_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
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
            for id in &worker_ids {
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
        let source_identity_sha256 = super::super::retention::invocation_archive_source_identity_sha256(
            &mut conn,
            super::super::retention::InvocationArchiveIdentityDatabase::Archive,
            &worker_ids,
        )
        .await?;
        sqlx::query("DETACH DATABASE archive_db")
            .execute(&mut *conn)
            .await
            .context("failed to detach archive database")?;
        Ok::<(i64, Vec<(i64, String)>, String), anyhow::Error>((
            row_count,
            upstream_last_activity,
            source_identity_sha256,
        ))
    })
    .await?;

    finalize_archive_sqlite_file(&work_path).await?;

    deflate_sqlite_file_to_gzip_with_operation_budget(&work_path, &temp_gzip_path).await?;
    publish_prepared_archive_file(&temp_gzip_path, &final_path)?;

    let sha256 = sha256_hex_file_with_operation_budget(&final_path).await?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key,
        day_key: Some(day_key.to_string()),
        part_key: Some(part_key),
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        source_identity_sha256: Some(row_count.2),
        row_count: row_count.0,
        upstream_last_activity: row_count.1,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
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
    let archive_batch_id =
        upsert_archive_batch_manifest_with_status(tx, batch, ARCHIVE_STATUS_COMPLETED, false)
            .await?;
    append_archive_batch_summary_descriptor(tx, batch, archive_batch_id).await
}

pub(crate) async fn stage_invocation_archive_batch_manifest(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<()> {
    if batch.dataset != HOURLY_ROLLUP_DATASET_INVOCATIONS
        || batch.summary_source_kind != SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
    {
        bail!(
            "only an authoritative codex_invocations archive can enter Summary publication staging"
        );
    }
    let archive_batch_id =
        upsert_archive_batch_manifest_with_status(tx, batch, ARCHIVE_STATUS_MATERIALIZING, false)
            .await?;
    append_archive_batch_summary_descriptor(tx, batch, archive_batch_id).await
}

pub(crate) async fn stage_invocation_archive_batch_manifest_for_summary_pages(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<i64> {
    if batch.dataset != HOURLY_ROLLUP_DATASET_INVOCATIONS
        || batch.summary_source_kind != SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
    {
        bail!(
            "only an authoritative codex_invocations archive can enter Summary publication staging"
        );
    }
    // Keep the existing file identity until the publication transaction renames and commits the
    // replacement. If cancellation occurs before that point, the next run must still accept the
    // old final file as the predecessor rather than treating it as a manifest collision.
    upsert_archive_batch_manifest_with_status(tx, batch, ARCHIVE_STATUS_MATERIALIZING, true).await
}

pub(crate) async fn finalize_invocation_archive_batch_publication_tx(
    tx: &mut sqlx::SqliteConnection,
    file_path: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE archive_batches
        SET status = ?1
        WHERE dataset = ?2
          AND file_path = ?3
          AND status IN (?4, ?5)
          AND summary_source_kind = ?6
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .bind(ARCHIVE_STATUS_MATERIALIZING)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        bail!("staged invocation archive was not available for publication: {file_path}");
    }
    Ok(())
}

async fn upsert_archive_batch_manifest_with_status(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
    status: &str,
    preserve_existing_artifact_identity: bool,
) -> Result<i64> {
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
            summary_source_kind,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'))
        ON CONFLICT(dataset, month_key, file_path) DO UPDATE SET
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            sha256 = CASE
                WHEN ?18 THEN archive_batches.sha256
                ELSE excluded.sha256
            END,
            row_count = CASE
                WHEN ?18 THEN archive_batches.row_count
                ELSE excluded.row_count
            END,
            -- A completed artifact is an authority fence: a stale retry may refresh its
            -- metadata, but it must never reopen the manifest as materializing.
            status = CASE
                WHEN archive_batches.status = 'completed'
                     AND excluded.status = 'materializing'
                    THEN archive_batches.status
                ELSE excluded.status
            END,
            layout = excluded.layout,
            codec = excluded.codec,
            writer_version = excluded.writer_version,
            cleanup_state = excluded.cleanup_state,
            cleanup_source_safe_start_date = NULL,
            superseded_by = excluded.superseded_by,
            coverage_start_at = CASE
                WHEN archive_batches.coverage_start_at IS NULL THEN excluded.coverage_start_at
                WHEN excluded.coverage_start_at IS NULL THEN archive_batches.coverage_start_at
                ELSE MIN(archive_batches.coverage_start_at, excluded.coverage_start_at)
            END,
            coverage_end_at = CASE
                WHEN archive_batches.coverage_end_at IS NULL THEN excluded.coverage_end_at
                WHEN excluded.coverage_end_at IS NULL THEN archive_batches.coverage_end_at
                ELSE MAX(archive_batches.coverage_end_at, excluded.coverage_end_at)
            END,
            archive_expires_at = CASE
                WHEN archive_batches.archive_expires_at IS NULL OR excluded.archive_expires_at IS NULL
                    THEN excluded.archive_expires_at
                ELSE MAX(archive_batches.archive_expires_at, excluded.archive_expires_at)
            END,
            summary_source_kind = excluded.summary_source_kind,
            created_at = datetime('now')
        WHERE NOT (
            archive_batches.summary_source_kind = 'authoritative'
            AND excluded.summary_source_kind = 'live_mirror'
        )
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(batch.day_key.as_deref())
    .bind(batch.part_key.as_deref())
    .bind(&batch.file_path)
    .bind(&batch.sha256)
    .bind(batch.row_count)
    .bind(status)
    .bind(batch.layout)
    .bind(batch.codec)
    .bind(batch.writer_version)
    .bind(batch.cleanup_state)
    .bind(batch.superseded_by)
    .bind(batch.coverage_start_at.as_deref())
    .bind(batch.coverage_end_at.as_deref())
    .bind(batch.archive_expires_at.as_deref())
    .bind(batch.summary_source_kind)
    .bind(preserve_existing_artifact_identity)
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
    Ok(archive_batch_id)
}

async fn append_archive_batch_summary_descriptor(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
    archive_batch_id: i64,
) -> Result<()> {
    if batch.dataset != "codex_invocations" {
        return Ok(());
    }
    let descriptor = SummarySourceChangeDescriptor::coverage_change(
        u64::try_from(archive_batch_id).context("archive batch source revision overflow")?,
        vec![SummarySourceChangeEntry {
            row_id: archive_batch_id,
            invoke_id: format!("archive:{}", batch.sha256),
            occurred_at: batch
                .coverage_start_at
                .clone()
                .unwrap_or_else(|| batch.month_key.clone()),
            upstream_account_id: None,
            current_rank: None,
        }],
    )?;
    append_summary_source_change_descriptor_tx(tx, &descriptor).await?;
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

    async fn with_test_file_phase<F, T>(future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let permit = crate::maintenance::acquire_file_phase_permit("archive_writer_test")
            .await
            .expect("acquire archive writer test file phase permit");
        crate::maintenance::with_file_phase_permit(permit, future).await
    }

    #[test]
    fn segment_part_key_is_stable_for_one_prepared_identity() {
        let forward = archive_segment_part_key_for_ids(&[7, 3, 5]).expect("part key");
        let reordered = archive_segment_part_key_for_ids(&[5, 7, 3]).expect("part key");
        let distinct = archive_segment_part_key_for_ids(&[3, 5, 8]).expect("part key");

        assert_eq!(forward, reordered);
        assert_ne!(forward, distinct);
    }

    #[tokio::test]
    async fn prepared_archive_publish_rejects_conflicting_existing_identity() {
        with_test_file_phase(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn prepared_archive_publish_reuses_a_matching_identity() {
        with_test_file_phase(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn live_mirror_manifest_does_not_downgrade_authoritative_manifest() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let authoritative = ArchiveBatchOutcome {
            dataset: HOURLY_ROLLUP_DATASET_INVOCATIONS,
            month_key: "2026-01".to_string(),
            day_key: None,
            part_key: None,
            file_path: "/tmp/codex-invocations-2026-01.sqlite.gz".to_string(),
            sha256: "authoritative-sha".to_string(),
            source_identity_sha256: None,
            row_count: 10,
            upstream_last_activity: Vec::new(),
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
            summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE,
            layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
            codec: ARCHIVE_FILE_CODEC_GZIP,
            writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
            cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
            superseded_by: None,
        };
        let live_mirror = ArchiveBatchOutcome {
            dataset: HOURLY_ROLLUP_DATASET_INVOCATIONS,
            month_key: "2026-01".to_string(),
            day_key: None,
            part_key: None,
            file_path: authoritative.file_path.clone(),
            sha256: "live-mirror-sha".to_string(),
            source_identity_sha256: None,
            row_count: 11,
            upstream_last_activity: Vec::new(),
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
            summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR,
            layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
            codec: ARCHIVE_FILE_CODEC_GZIP,
            writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
            cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
            superseded_by: None,
        };
        let mut tx = pool.begin().await.expect("begin manifest transaction");
        upsert_archive_batch_manifest_with_status(
            tx.as_mut(),
            &authoritative,
            ARCHIVE_STATUS_MATERIALIZING,
            false,
        )
        .await
        .expect("insert authoritative manifest");
        upsert_archive_batch_manifest_with_status(
            tx.as_mut(),
            &live_mirror,
            ARCHIVE_STATUS_MATERIALIZING,
            false,
        )
        .await
        .expect("update manifest from live mirror");
        tx.commit().await.expect("commit manifest transaction");

        let manifest: (String, String, i64) = sqlx::query_as(
            "SELECT summary_source_kind, sha256, row_count FROM archive_batches WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind("2026-01")
        .bind(&authoritative.file_path)
        .fetch_one(&pool)
        .await
        .expect("load merged manifest");
        assert_eq!(manifest.0, SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE);
        assert_eq!(manifest.1, "authoritative-sha");
        assert_eq!(manifest.2, 10);
        pool.close().await;
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
            None,
            ArchiveReplacement {
                row_count: None,
                sha256: "replacement-sha",
            },
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

    #[tokio::test]
    async fn legacy_archive_replacement_rejects_a_stale_preparation_baseline() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-legacy-archive-baseline-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        fs::create_dir_all(&root).expect("create archive test directory");
        let final_path = root.join("archive.sqlite.gz");
        let temporary_path = root.join("archive.next.tmp");
        fs::write(&final_path, b"newer archive").expect("write newer archive");
        fs::write(&temporary_path, b"stale prepared archive").expect("write stale archive");

        let error = replace_legacy_archive_file_with_cleanup_serialization(
            &pool,
            "codex_invocations",
            "2026-01",
            &temporary_path,
            &final_path,
            Some(&format!("{:x}", Sha256::digest(b"older archive"))),
            ArchiveReplacement {
                row_count: None,
                sha256: "replacement-sha",
            },
        )
        .await
        .expect_err("stale baseline must not overwrite a newer archive");
        assert!(error.to_string().contains("retry required"));
        assert_eq!(
            fs::read(&final_path).expect("read newer archive"),
            b"newer archive"
        );
        assert!(
            temporary_path.exists(),
            "caller retains stale staging for retry cleanup"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
