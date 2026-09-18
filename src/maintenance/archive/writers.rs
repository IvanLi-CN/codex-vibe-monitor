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
}

include!("writers/part-01.rs");
include!("writers/part-02.rs");
#[cfg(test)]
mod tests {
    include!("writers/tests/part-01.rs");
}
