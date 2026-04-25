use super::*;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use std::str::FromStr;

#[derive(Debug, Clone, FromRow)]
struct PoolUpstreamRequestAttemptArchiveRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    route_mode: String,
    sticky_key: Option<String>,
    group_name_snapshot: Option<String>,
    proxy_binding_key_snapshot: Option<String>,
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
    compact_support_status: Option<String>,
    compact_support_reason: Option<String>,
    created_at: String,
}

async fn open_archive_sqlite_connection(path: &Path) -> Result<SqliteConnection> {
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

fn ensure_attachable_archive_sqlite_path(path: &Path) -> Result<()> {
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

async fn finalize_archive_sqlite_file(path: &Path) -> Result<()> {
    let mut connection = open_archive_sqlite_connection(path).await?;
    sqlx::query("VACUUM")
        .execute(&mut connection)
        .await
        .with_context(|| format!("failed to finalize archive sqlite file {}", path.display()))?;
    connection.close().await?;
    Ok(())
}

async fn ensure_pool_upstream_request_attempts_archive_schema_direct(
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
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add pool_upstream_request_attempts archive column {column}"
                    )
                })?;
        }
    }
    Ok(())
}

async fn archive_pool_upstream_request_attempt_rows_into_month_batch(
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
                .push_bind(&row.invoke_id)
                .push_bind(&row.occurred_at)
                .push_bind(&row.endpoint)
                .push_bind(&row.route_mode)
                .push_bind(&row.sticky_key)
                .push_bind(&row.group_name_snapshot)
                .push_bind(&row.proxy_binding_key_snapshot)
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
                .push_bind(&row.compact_support_status)
                .push_bind(&row.compact_support_reason)
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
        archive_pool_upstream_request_attempt_rows_into_month_batch(
            pool, spec, ids, &work_path,
        )
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
    if let Err(err) = fs::rename(&temp_gzip_path, &final_path).with_context(|| {
        format!(
            "failed to move archive batch into place: {} -> {}",
            temp_gzip_path.display(),
            final_path.display()
        )
    }) {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }
    let _ = fs::remove_file(&work_path);

    let sha256 = sha256_hex_file(&final_path)?;
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
    let part_key = next_archive_segment_part_key(pool, spec.dataset, day_key).await?;
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
    fs::rename(&temp_gzip_path, &final_path).with_context(|| {
        format!(
            "failed to move archive segment into place: {} -> {}",
            temp_gzip_path.display(),
            final_path.display()
        )
    })?;

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
