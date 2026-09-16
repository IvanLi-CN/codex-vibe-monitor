use super::*;

pub(super) async fn ensure_schema_archive(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_archive_batch_columns_and_classification(pool).await?;
    ensure_archive_coverage_and_triggers(pool).await?;
    ensure_archive_indexes(pool).await?;
    ensure_archive_activity_tables(pool).await?;
    ensure_archive_rollup_tables(pool).await?;
    Ok(())
}

async fn ensure_archive_batch_columns_and_classification(pool: &Pool<Sqlite>) -> Result<()> {
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

    // A detail-prune archive duplicates records still retained in the live table. Segment keys
    // encode the exact inclusive ID bounds in hexadecimal; only a contiguous live range proves
    // that every archived record remains live. Unknown legacy manifests stay fail-closed.
    classify_legacy_invocation_detail_archive_mirrors(pool)
        .await
        .context("failed to classify legacy invocation detail archive mirrors")?;
    Ok(())
}

async fn ensure_archive_coverage_and_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_archive_indexes(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_archive_activity_tables(pool: &Pool<Sqlite>) -> Result<()> {
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

async fn ensure_archive_rollup_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_daily (
            stats_date TEXT NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_daily table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_daily_source_date
        ON invocation_rollup_daily (source, stats_date)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_daily_source_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            terminal_count INTEGER NOT NULL DEFAULT 0,
            terminal_tokens INTEGER NOT NULL DEFAULT 0,
            terminal_cost REAL NOT NULL DEFAULT 0,
            terminal_proof_complete INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_hourly table existence")?;

    Ok(())
}
