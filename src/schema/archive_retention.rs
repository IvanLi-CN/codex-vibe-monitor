use super::compatibility::ensure_column_with_definition;
use super::*;

pub(crate) fn pool_upstream_node_health_hourly_archive_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
            archive_identity TEXT NOT NULL,
            archive_batch_id INTEGER,
            archive_file_path TEXT NOT NULL,
            proxy_binding_key_snapshot TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_identity, proxy_binding_key_snapshot, bucket_start_epoch)
        )
        "#
    )
}

pub(crate) async fn migrate_pool_upstream_node_health_hourly_archive_identity(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    const TEMP_TABLE: &str = "pool_upstream_node_health_hourly_archive_v2";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to clear stale pool_upstream_node_health_hourly_archive migration temp table",
        )?;

    let create_temp_sql = pool_upstream_node_health_hourly_archive_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to create pool_upstream_node_health_hourly_archive migration temp table",
        )?;

    let copy_sql = format!(
        r#"
        INSERT INTO {TEMP_TABLE} (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count,
            updated_at
        )
        SELECT
            CASE
                WHEN batches.id IS NOT NULL THEN 'batch:' || CAST(batches.id AS TEXT)
                ELSE 'legacy-file:' || legacy.archive_file_path
            END AS archive_identity,
            batches.id AS archive_batch_id,
            legacy.archive_file_path,
            legacy.proxy_binding_key_snapshot,
            legacy.bucket_start_epoch,
            legacy.success_count,
            legacy.failure_count,
            legacy.updated_at
        FROM pool_upstream_node_health_hourly_archive AS legacy
        LEFT JOIN archive_batches AS batches
          ON batches.dataset = 'pool_upstream_request_attempts'
         AND batches.file_path = legacy.archive_file_path
        "#
    );
    sqlx::query(&copy_sql).execute(tx.as_mut()).await.context(
        "failed to copy pool_upstream_node_health_hourly_archive rows into migration temp table",
    )?;

    sqlx::query("DROP TABLE pool_upstream_node_health_hourly_archive")
        .execute(tx.as_mut())
        .await
        .context(
            "failed to drop legacy pool_upstream_node_health_hourly_archive table during migration",
        )?;

    let rename_sql =
        format!("ALTER TABLE {TEMP_TABLE} RENAME TO pool_upstream_node_health_hourly_archive");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to swap migrated pool_upstream_node_health_hourly_archive table into place",
        )?;

    tx.commit().await?;
    Ok(())
}

fn legacy_archive_segment_id_range(part_key: &str) -> Option<(i64, i64)> {
    let encoded = part_key.strip_prefix("part-")?;
    let (lower, remainder) = encoded.split_once('-')?;
    let (upper, _) = remainder.split_once('-')?;
    let lower = i64::from_str_radix(lower, 16).ok()?;
    let upper = i64::from_str_radix(upper, 16).ok()?;
    (lower > 0 && upper >= lower).then_some((lower, upper))
}

async fn classify_legacy_invocation_detail_archive_mirrors(pool: &Pool<Sqlite>) -> Result<()> {
    const CLASSIFICATION_CHUNK_SIZE: i64 = 512;
    let mut after_id = 0_i64;

    loop {
        let candidates = sqlx::query_as::<_, (i64, String)>(
            r#"
            SELECT id, part_key
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND summary_source_kind = 'unknown'
              AND layout = 'segment_v1'
              AND part_key IS NOT NULL
              AND id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(after_id)
        .bind(CLASSIFICATION_CHUNK_SIZE)
        .fetch_all(pool)
        .await?;
        let Some(last_id) = candidates.last().map(|(id, _)| *id) else {
            break;
        };
        after_id = last_id;

        let ranges = candidates
            .into_iter()
            .filter_map(|(id, part_key)| {
                legacy_archive_segment_id_range(&part_key)
                    .map(|(lower_id, upper_id)| (id, lower_id, upper_id))
            })
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            continue;
        }

        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "WITH candidates(id, lower_id, upper_id) AS (VALUES ",
        );
        for (index, (id, lower_id, upper_id)) in ranges.into_iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            query
                .push("(")
                .push_bind(id)
                .push(", ")
                .push_bind(lower_id)
                .push(", ")
                .push_bind(upper_id)
                .push(")");
        }
        query.push(
            ") \
             UPDATE archive_batches \
             SET summary_source_kind = 'live_mirror' \
             WHERE summary_source_kind = 'unknown' \
               AND id IN ( \
                    SELECT id FROM candidates \
                    WHERE ( \
                        SELECT COUNT(*) FROM codex_invocations AS live \
                        WHERE live.id BETWEEN lower_id AND upper_id \
                    ) = upper_id - lower_id + 1 \
               )",
        );
        query.build().execute(pool).await?;
    }

    Ok(())
}

pub(super) async fn ensure_archive_retention_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL DEFAULT 'legacy_month',
            codec TEXT NOT NULL DEFAULT 'gzip',
            writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1',
            cleanup_state TEXT NOT NULL DEFAULT 'active',
            cleanup_source_safe_start_date TEXT,
            replacement_staged_path TEXT,
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_start_epoch INTEGER,
            coverage_end_epoch INTEGER,
            archive_expires_at TEXT,
            summary_source_kind TEXT NOT NULL DEFAULT 'unknown',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;

    let archive_batch_columns = load_sqlite_table_columns(pool, "archive_batches").await?;
    for (column, ty) in [
        // A legacy archive without a recorded hash cannot prove replay identity and remains
        // pending until it is rebuilt; use a nullable upgrade column for that state.
        ("sha256", "TEXT"),
        ("day_key", "TEXT"),
        ("part_key", "TEXT"),
        ("layout", "TEXT NOT NULL DEFAULT 'legacy_month'"),
        ("codec", "TEXT NOT NULL DEFAULT 'gzip'"),
        ("writer_version", "TEXT NOT NULL DEFAULT 'legacy_month_v1'"),
        ("cleanup_state", "TEXT NOT NULL DEFAULT 'active'"),
        ("cleanup_source_safe_start_date", "TEXT"),
        ("replacement_staged_path", "TEXT"),
        ("superseded_by", "INTEGER"),
        ("coverage_start_at", "TEXT"),
        ("coverage_end_at", "TEXT"),
        ("coverage_start_epoch", "INTEGER"),
        ("coverage_end_epoch", "INTEGER"),
        ("archive_expires_at", "TEXT"),
        ("upstream_activity_manifest_refreshed_at", "TEXT"),
        ("historical_rollups_materialized_at", "TEXT"),
        ("summary_source_kind", "TEXT NOT NULL DEFAULT 'unknown'"),
    ] {
        if !archive_batch_columns.contains(column) {
            let statement = format!("ALTER TABLE archive_batches ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add archive_batches column {column}"))?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_summary_source_classification
        ON archive_batches (dataset, status, summary_source_kind, layout, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive source classification index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_pending_summary_rollup_order
        ON archive_batches (dataset, status, month_key, created_at, id)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pending summary rollup order index")?;

    // Force-repair pages seek by the archive primary key and retain that order across commits.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_summary_rollup_repair_seek
        ON archive_batches (dataset, status, id)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary rollup repair seek index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS retention_prepared_archives (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prepared_key TEXT NOT NULL UNIQUE,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            source_ids_json TEXT NOT NULL,
            source_identity_sha256 TEXT NOT NULL,
            publication_kind TEXT,
            state TEXT NOT NULL,
            artifact_sha256 TEXT,
            artifact_bytes INTEGER,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_retry_at TEXT,
            last_failure_stage TEXT,
            last_failure_fingerprint TEXT,
            staged_file_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            quarantined_at TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention prepared archive table existence")?;

    let prepared_archive_columns =
        load_sqlite_table_columns(pool, "retention_prepared_archives").await?;
    if !prepared_archive_columns.contains("publication_kind") {
        sqlx::query("ALTER TABLE retention_prepared_archives ADD COLUMN publication_kind TEXT")
            .execute(pool)
            .await
            .context("failed to add retention_prepared_archives.publication_kind")?;
    }
    if !prepared_archive_columns.contains("staged_file_path") {
        sqlx::query("ALTER TABLE retention_prepared_archives ADD COLUMN staged_file_path TEXT")
            .execute(pool)
            .await
            .context("failed to add retention_prepared_archives.staged_file_path")?;
    }

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_retention_prepared_archives_dataset_path
        ON retention_prepared_archives (dataset, file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention prepared archive path index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_retention_prepared_archives_due
        ON retention_prepared_archives (state, next_retry_at, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention prepared archive due index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS retention_recovery_cursors (
            scope TEXT PRIMARY KEY,
            cursor TEXT NOT NULL DEFAULT '',
            next_retry_at TEXT,
            consecutive_failure_count INTEGER NOT NULL DEFAULT 0,
            last_failure_fingerprint TEXT,
            defer_reason TEXT,
            last_progress_at TEXT,
            last_admission_stage TEXT,
            last_admission_cause TEXT,
            last_settled_pass_at TEXT,
            last_settled_pass_complete INTEGER,
            last_settled_pass_inspected_entries INTEGER,
            last_settled_pass_referenced_skipped INTEGER,
            last_settled_pass_quarantined INTEGER,
            last_settled_pass_removed INTEGER,
            last_settled_pass_removed_bytes INTEGER,
            last_nonzero_removal_at TEXT,
            last_nonzero_removal INTEGER,
            last_nonzero_removal_bytes INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention recovery cursor table existence")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO retention_recovery_cursors (scope, cursor)
        VALUES ('legacy_archive_segments', ''), ('raw_payload_files', ''), ('prepared_archives', '')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to seed retention recovery cursor")?;

    for (column, definition) in [
        ("next_retry_at", "TEXT"),
        ("consecutive_failure_count", "INTEGER NOT NULL DEFAULT 0"),
        ("last_failure_fingerprint", "TEXT"),
        ("defer_reason", "TEXT"),
        ("last_progress_at", "TEXT"),
        ("last_admission_stage", "TEXT"),
        ("last_admission_cause", "TEXT"),
        ("last_settled_pass_at", "TEXT"),
        ("last_settled_pass_complete", "INTEGER"),
        ("last_settled_pass_inspected_entries", "INTEGER"),
        ("last_settled_pass_referenced_skipped", "INTEGER"),
        ("last_settled_pass_quarantined", "INTEGER"),
        ("last_settled_pass_removed", "INTEGER"),
        ("last_settled_pass_removed_bytes", "INTEGER"),
        ("last_nonzero_removal_at", "TEXT"),
        ("last_nonzero_removal", "INTEGER"),
        ("last_nonzero_removal_bytes", "INTEGER"),
    ] {
        ensure_column_with_definition(pool, "retention_recovery_cursors", column, definition)
            .await
            .with_context(|| format!("failed to ensure retention_recovery_cursors.{column}"))?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS retention_raw_reconciliation (
            raw_path TEXT PRIMARY KEY,
            file_identity TEXT NOT NULL,
            byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
            quarantined_at TEXT NOT NULL,
            release_pending INTEGER NOT NULL DEFAULT 0,
            release_pending_at TEXT,
            removal_evidence_persisted INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention raw reconciliation table existence")?;

    ensure_column_with_definition(
        pool,
        "retention_raw_reconciliation",
        "release_pending",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await
    .context("failed to ensure retention_raw_reconciliation.release_pending")?;
    ensure_column_with_definition(
        pool,
        "retention_raw_reconciliation",
        "release_pending_at",
        "TEXT",
    )
    .await
    .context("failed to ensure retention_raw_reconciliation.release_pending_at")?;
    ensure_column_with_definition(
        pool,
        "retention_raw_reconciliation",
        "removal_evidence_persisted",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await
    .context("failed to ensure retention_raw_reconciliation.removal_evidence_persisted")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_retention_raw_reconciliation_quarantine
        ON retention_raw_reconciliation (quarantined_at, raw_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention raw reconciliation quarantine index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_retention_raw_reconciliation_pending
        ON retention_raw_reconciliation (updated_at, raw_path)
        WHERE release_pending != 0
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention raw reconciliation pending index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_retention_raw_reconciliation_active
        ON retention_raw_reconciliation (raw_path)
        WHERE release_pending = 0
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure retention raw reconciliation active index")?;

    // A detail-prune archive duplicates records still retained in the live table. Segment keys
    // encode the exact inclusive ID bounds in hexadecimal; only a contiguous live range proves
    // that every archived record remains live. Unknown legacy manifests stay fail-closed.
    classify_legacy_invocation_detail_archive_mirrors(pool)
        .await
        .context("failed to classify legacy invocation detail archive mirrors")?;

    // Archive writers retain the established text bounds for compatibility, while read-side
    // coverage planners use these normalized epochs for indexed range lookups.
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET
            coverage_start_epoch = CASE
                WHEN coverage_start_at IS NULL THEN NULL
                WHEN instr(coverage_start_at, 'T') > 0
                    THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
            END,
            coverage_end_epoch = CASE
                WHEN coverage_end_at IS NULL THEN NULL
                WHEN instr(coverage_end_at, 'T') > 0
                    THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
            END
        WHERE (coverage_start_at IS NULL AND coverage_start_epoch IS NOT NULL)
           OR (coverage_start_at IS NOT NULL AND coverage_start_epoch IS NULL)
           OR (coverage_end_at IS NULL AND coverage_end_epoch IS NOT NULL)
           OR (coverage_end_at IS NOT NULL AND coverage_end_epoch IS NULL)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill normalized archive coverage epochs")?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_archive_batches_coverage_epoch_insert
        AFTER INSERT ON archive_batches
        BEGIN
            UPDATE archive_batches
            SET
                coverage_start_epoch = CASE
                    WHEN coverage_start_at IS NULL THEN NULL
                    WHEN instr(coverage_start_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
                END,
                coverage_end_epoch = CASE
                    WHEN coverage_end_at IS NULL THEN NULL
                    WHEN instr(coverage_end_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
                END
            WHERE id = NEW.id;
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive coverage epoch insert trigger")?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_archive_batches_coverage_epoch_update
        AFTER UPDATE OF coverage_start_at, coverage_end_at ON archive_batches
        BEGIN
            UPDATE archive_batches
            SET
                coverage_start_epoch = CASE
                    WHEN coverage_start_at IS NULL THEN NULL
                    WHEN instr(coverage_start_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
                END,
                coverage_end_epoch = CASE
                    WHEN coverage_end_at IS NULL THEN NULL
                    WHEN instr(coverage_end_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
                END
            WHERE id = NEW.id;
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive coverage epoch update trigger")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_month
        ON archive_batches (dataset, month_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_month")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_file_path
        ON archive_batches (dataset, file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_file_path")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_layout_day_part
        ON archive_batches (dataset, layout, day_key, part_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_layout_day_part")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_manifest_pending
        ON archive_batches (dataset, status, upstream_activity_manifest_refreshed_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_invocation_manifest_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_rollup_materialization
        ON archive_batches (dataset, status, historical_rollups_materialized_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_rollup_materialization")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_usage_breakdown_repair_order
        ON archive_batches (
            dataset,
            status,
            historical_rollups_materialized_at,
            month_key,
            created_at,
            id
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_usage_breakdown_repair_order")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_usage_breakdown_repair_candidates
        ON archive_batches (month_key, created_at, id)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND historical_rollups_materialized_at IS NOT NULL
          AND TRIM(sha256) <> ''
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_usage_breakdown_repair_candidates")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_summary_source_coverage
        ON archive_batches (
            dataset,
            status,
            summary_source_kind,
            coverage_end_epoch,
            coverage_start_epoch,
            id
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive source coverage index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_coverage_epoch
        ON archive_batches (coverage_end_epoch, coverage_start_epoch)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND coverage_start_epoch IS NOT NULL
          AND coverage_end_epoch IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation archive coverage epoch index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_legacy_coverage_month
        ON archive_batches (month_key)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND (coverage_start_at IS NULL OR coverage_end_at IS NULL)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation archive legacy coverage month index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batch_upstream_activity (
            archive_batch_id INTEGER NOT NULL,
            account_id INTEGER NOT NULL,
            last_activity_at TEXT NOT NULL,
            PRIMARY KEY (archive_batch_id, account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batch_upstream_activity table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_account_last_activity
        ON archive_batch_upstream_activity (account_id, last_activity_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_account_last_activity")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_batch
        ON archive_batch_upstream_activity (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_materialized_buckets (
            target TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            materialized_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_materialized_buckets table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_hourly_rollup_materialized_buckets_target_bucket
        ON hourly_rollup_materialized_buckets (target, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_hourly_rollup_materialized_buckets_target_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch INTEGER PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure account_activity_v2_bucket_repair_watermarks table existence")?;

    Ok(())
}

pub(super) async fn ensure_hourly_archive_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let hourly_archive_sql = pool_upstream_node_health_hourly_archive_create_sql(
        "pool_upstream_node_health_hourly_archive",
    );
    sqlx::query(&hourly_archive_sql)
        .execute(pool)
        .await
        .context("failed to ensure pool_upstream_node_health_hourly_archive table existence")?;

    let hourly_archive_columns =
        load_sqlite_table_columns(pool, "pool_upstream_node_health_hourly_archive").await?;
    if !hourly_archive_columns.contains("archive_identity")
        || !hourly_archive_columns.contains("archive_batch_id")
    {
        migrate_pool_upstream_node_health_hourly_archive_identity(pool).await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_bucket_binding
        ON pool_upstream_node_health_hourly_archive (bucket_start_epoch, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_pool_upstream_node_health_hourly_archive_bucket_binding",
    )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_file
        ON pool_upstream_node_health_hourly_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_file")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_batch
        ON pool_upstream_node_health_hourly_archive (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            archive_sha256 TEXT,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

    // SQLite leaves an existing table untouched for CREATE TABLE IF NOT EXISTS. Upgrade the
    // pre-identity replay table before any startup backfill reads or writes archive_sha256.
    let hourly_rollup_archive_replay_columns =
        load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !hourly_rollup_archive_replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add hourly rollup archive replay identity column")?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_hourly_rollup_archive_replay_dataset_file_sha
        ON hourly_rollup_archive_replay (dataset, file_path, archive_sha256)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly rollup archive replay identity lookup index")?;

    Ok(())
}
