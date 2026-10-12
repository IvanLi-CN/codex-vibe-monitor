use super::*;

pub(crate) fn ensure_long_term_stats_schema_sql() -> &'static str {
    "long_term_usage_hourly, long_term_usage_daily, long_term_stats_state and long_term_stats_repair_queue"
}

pub(crate) async fn ensure_long_term_stats_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_hourly table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_daily (
            stats_date TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_daily table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            status TEXT NOT NULL DEFAULT 'preparing',
            statistics_start_date TEXT,
            integrity_source_start_date TEXT,
            integrity_source_pending_start_date TEXT,
            processed_rows INTEGER NOT NULL DEFAULT 0,
            total_rows INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_stats_state table")?;
    sqlx::query("INSERT OR IGNORE INTO long_term_stats_state (id, status) VALUES (?1, ?2)")
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .execute(pool)
        .await
        .context("failed to seed long term stats state")?;
    let state_columns = load_sqlite_table_columns(pool, "long_term_stats_state").await?;
    if !state_columns.contains("last_integrity_audit_at") {
        sqlx::query("ALTER TABLE long_term_stats_state ADD COLUMN last_integrity_audit_at TEXT")
            .execute(pool)
            .await
            .context("failed to add long term integrity audit timestamp")?;
    }
    if !state_columns.contains("integrity_source_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add long term integrity source boundary")?;
    }
    if !state_columns.contains("integrity_source_pending_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_pending_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add pending long term integrity source boundary")?;
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_repair_queue (
            stats_date TEXT PRIMARY KEY,
            expected_calls INTEGER NOT NULL,
            expected_token_total INTEGER NOT NULL,
            expected_cost_total REAL NOT NULL,
            observed_calls INTEGER NOT NULL,
            observed_token_total INTEGER NOT NULL,
            observed_cost_total REAL NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_retry_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_error TEXT NOT NULL,
            detected_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_stats_repair_queue_due ON long_term_stats_repair_queue (next_retry_at, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_daily_dimension_date ON long_term_usage_daily (dimension, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term daily index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_hourly_dimension_bucket ON long_term_usage_hourly (dimension, bucket_start_epoch)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term hourly index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            archive_sha256 TEXT,
            source_identity TEXT,
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term archive replay marker table")?;
    let replay_columns = load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add archive hash to replay markers")?;
    }
    if !replay_columns.contains("source_identity") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN source_identity TEXT")
            .execute(pool)
            .await
            .context("failed to add archive file identity to replay markers")?;
    }
    ensure_long_term_projection_schema(pool).await?;
    ensure_long_term_projection_source_indexes(pool).await?;
    ensure_long_term_projection_correction_trigger(pool).await?;
    ensure_long_term_projection_archive_trigger(pool).await?;
    Ok(())
}

pub(super) async fn ensure_long_term_projection_source_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    if !invocation_table_exists {
        return Ok(());
    }
    let columns = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if columns.contains("status") {
        // Terminal delta scans advance by id. Index the exact terminal predicate so a dense
        // running/pending prefix cannot turn a bounded incremental flush into a table scan.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_terminal_id
            ON codex_invocations (id)
            WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection terminal id index")?;
    }
    if !columns.contains("occurred_at") || !columns.contains("t_total_ms") {
        return Ok(());
    }
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_text_end
        ON codex_invocations (
            CASE
                WHEN instr(occurred_at, 'T') = 0
                  AND t_total_ms IS NOT NULL
                  AND t_total_ms > 0
                THEN julianday(occurred_at) + t_total_ms / 86400000.0
            END
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection text end index")?;
    if columns.contains("status") {
        // RFC3339 rows are exceptional, but preserving them requires an index-backed candidate
        // range before the exact (and deliberately compatibility-safe) epoch predicate runs.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_occurred_at
            ON codex_invocations (occurred_at)
            WHERE instr(occurred_at, 'T') > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 timestamp index")?;
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_duration
            ON codex_invocations (t_total_ms DESC)
            WHERE instr(occurred_at, 'T') > 0
              AND t_total_ms IS NOT NULL
              AND t_total_ms > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 duration index")?;
    }
    Ok(())
}

pub(super) async fn ensure_long_term_projection_correction_trigger(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?;
    if invocation_table_exists == 0 {
        return Ok(());
    }

    // Corrections originate in several write-side workers. A normal terminal finalize moves an
    // in-flight record to a terminal state and is consumed through the projection cursor unless
    // a newer terminal row has already advanced that cursor past it. In that out-of-order case,
    // the cursor cannot revisit the row, so queue the affected date for an exact repair.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_invocation_correction")
        .execute(pool)
        .await?;
    let old_start_date = long_term_rfc3339_shanghai_date_sql("OLD.occurred_at", None);
    let old_end_date = long_term_rfc3339_shanghai_date_sql(
        "OLD.occurred_at",
        Some("MAX(COALESCE(OLD.t_total_ms, 0), 0)"),
    );
    let new_start_date = long_term_rfc3339_shanghai_date_sql("NEW.occurred_at", None);
    let new_end_date = long_term_rfc3339_shanghai_date_sql(
        "NEW.occurred_at",
        Some("MAX(COALESCE(NEW.t_total_ms, 0), 0)"),
    );
    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_invocation_correction
        AFTER UPDATE OF
          source, status, occurred_at, model, payload,
          input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens,
          cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
          t_upstream_ttfb_ms, t_upstream_stream_ms, error_message, failure_kind
        ON codex_invocations
        WHEN NOT (
          (
            LOWER(TRIM(COALESCE(OLD.status, ''))) IN ('running', 'pending')
            OR (
              LOWER(TRIM(COALESCE(OLD.status, ''))) = 'interrupted'
              AND LOWER(TRIM(COALESCE(OLD.failure_kind, ''))) = 'proxy_interrupted'
            )
          )
          AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
          AND NEW.id > COALESCE(
            (
              SELECT cursor_row_id
              FROM long_term_projection_state
              WHERE consumer = 'long_term_v1'
            ),
            0
          )
        )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
            SELECT
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_start_date}
                ELSE date(OLD.occurred_at) END,
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_end_date}
                ELSE date(julianday(OLD.occurred_at) + MAX(COALESCE(OLD.t_total_ms, 0), 0) / 86400000.0) END
            WHERE OLD.occurred_at IS NOT NULL AND TRIM(OLD.occurred_at) <> ''
            UNION ALL
            SELECT
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_start_date}
                ELSE date(NEW.occurred_at) END,
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_end_date}
                ELSE date(julianday(NEW.occurred_at) + MAX(COALESCE(NEW.t_total_ms, 0), 0) / 86400000.0) END
            WHERE NEW.occurred_at IS NOT NULL AND TRIM(NEW.occurred_at) <> ''
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM affected_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'invocation_correction'
          FROM affected_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection correction trigger")?;
    Ok(())
}

pub(super) async fn ensure_long_term_projection_archive_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    let archive_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archive_batches')",
    )
    .fetch_one(pool)
    .await?;
    if archive_table_exists == 0 {
        return Ok(());
    }

    // Archive writes and rewrites are source changes for durable long-term rollups. Invocation
    // archives provide the terminal facts; attempt archives provide a later account-attribution
    // fallback. Both must invalidate the same target dates. Recreate these triggers so upgrades
    // do not retain a prior definition that only observed invocation archives.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_insert")
        .execute(pool)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_update")
        .execute(pool)
        .await?;

    let new_coverage_start_date =
        long_term_rfc3339_shanghai_date_sql("NEW.coverage_start_at", None);
    let new_coverage_end_date = long_term_rfc3339_shanghai_date_sql("NEW.coverage_end_at", None);
    let old_coverage_start_date =
        long_term_rfc3339_shanghai_date_sql("OLD.coverage_start_at", None);
    let old_coverage_end_date = long_term_rfc3339_shanghai_date_sql("OLD.coverage_end_at", None);

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_insert
        AFTER INSERT ON archive_batches
        WHEN NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
          AND NEW.status = 'completed'
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE covered_dates(bucket_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive insert trigger")?;

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_update
        AFTER UPDATE OF dataset, status, file_path, sha256, coverage_start_at, coverage_end_at, historical_rollups_materialized_at ON archive_batches
        WHEN (
              NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            )
            OR (
              OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
            )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE coverage_ranges(start_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
            WHERE NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            UNION ALL
            SELECT
              COALESCE(CASE WHEN instr(OLD.coverage_start_at, 'T') > 0 THEN {old_coverage_start_date} ELSE date(OLD.coverage_start_at) END, date(OLD.month_key || '-01')),
              COALESCE(CASE WHEN instr(OLD.coverage_end_at, 'T') > 0 THEN {old_coverage_end_date} ELSE date(OLD.coverage_end_at) END, date(OLD.month_key || '-01', '+1 month', '-1 day'))
            WHERE OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
          ), covered_dates(bucket_date, end_date) AS (
            SELECT start_date, end_date FROM coverage_ranges
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive update trigger")?;
    Ok(())
}

pub(crate) async fn ensure_long_term_projection_account_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    for trigger in [
        "long_term_projection_account_kind_update",
        "long_term_projection_account_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(pool)
            .await?;
    }
    for (trigger, event, account_id) in [
        (
            "long_term_projection_account_kind_update",
            "AFTER UPDATE OF kind ON pool_upstream_accounts WHEN OLD.kind IS NOT NEW.kind",
            "NEW.id",
        ),
        (
            "long_term_projection_account_delete",
            "AFTER DELETE ON pool_upstream_accounts",
            "OLD.id",
        ),
    ] {
        let occurred_at_start_date = long_term_rfc3339_shanghai_date_sql("inv.occurred_at", None);
        let occurred_at_end_date = long_term_rfc3339_shanghai_date_sql(
            "inv.occurred_at",
            Some("MAX(COALESCE(inv.t_total_ms, 0), 0)"),
        );
        let archive_coverage_start_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_start_at", None);
        let archive_coverage_end_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_end_at", None);
        let statement = format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS {trigger}
            {event}
            BEGIN
              INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
              WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
                SELECT
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_start_date}
                    ELSE date(inv.occurred_at) END,
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_end_date}
                    ELSE date(julianday(inv.occurred_at) + MAX(COALESCE(inv.t_total_ms, 0), 0) / 86400000.0) END
                FROM codex_invocations inv
                WHERE (CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END) = {account_id}
                   OR EXISTS (
                        SELECT 1 FROM pool_upstream_request_attempts attempt
                        WHERE attempt.invoke_id = inv.invoke_id
                          AND attempt.occurred_at = inv.occurred_at
                          AND attempt.upstream_account_id = {account_id}
                      )
                UNION ALL
                SELECT
                  COALESCE(CASE WHEN instr(archive.coverage_start_at, 'T') > 0
                    THEN {archive_coverage_start_date}
                    ELSE date(archive.coverage_start_at) END, date(archive.month_key || '-01')),
                  COALESCE(CASE WHEN instr(archive.coverage_end_at, 'T') > 0
                    THEN {archive_coverage_end_date}
                    ELSE date(archive.coverage_end_at) END, date(archive.month_key || '-01', '+1 month', '-1 day'))
                FROM archive_batches archive
                WHERE archive.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
                  AND archive.status = 'completed'
                UNION ALL
                SELECT date(bucket_date, '+1 day'), end_date
                FROM affected_dates
                WHERE bucket_date < end_date
              )
              SELECT DISTINCT bucket_date, 'account_classification_changed'
              FROM affected_dates
              WHERE bucket_date IS NOT NULL
              ON CONFLICT(bucket_date) DO UPDATE SET
                repair_reason = excluded.repair_reason,
                generation = long_term_projection_dirty_buckets.generation + 1,
                next_attempt_at = NULL,
                updated_at = datetime('now');
            END
            "#,
        );
        sqlx::query(&statement).execute(pool).await?;
    }
    Ok(())
}

pub(super) async fn ensure_long_term_projection_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_daily_verify_at TEXT,
            daily_verify_pending INTEGER NOT NULL DEFAULT 0,
            daily_verify_bucket_date TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection state table")?;
    let daily_verify_migration =
        sqlx::query("ALTER TABLE long_term_projection_state ADD COLUMN last_daily_verify_at TEXT")
            .execute(pool)
            .await;
    if let Err(error) = daily_verify_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let daily_verify_pending_migration = sqlx::query(
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_pending INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_verify_pending_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let daily_verify_bucket_date_migration = sqlx::query(
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_bucket_date TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_verify_bucket_date_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_dirty_buckets (
            bucket_date TEXT PRIMARY KEY,
            repair_reason TEXT NOT NULL,
            generation INTEGER NOT NULL DEFAULT 1,
            queued_at TEXT NOT NULL DEFAULT (datetime('now')),
            next_attempt_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection dirty bucket table")?;
    let next_attempt_migration = sqlx::query(
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN next_attempt_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = next_attempt_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let generation_migration = sqlx::query(
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN generation INTEGER NOT NULL DEFAULT 1",
    )
    .execute(pool)
    .await;
    if let Err(error) = generation_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_bucket_state (
            bucket_date TEXT PRIMARY KEY,
            interval_baseline_ready INTEGER NOT NULL DEFAULT 0,
            active_daily_backup_token TEXT,
            publication_token TEXT,
            publication_generation INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection bucket state table")?;
    let daily_backup_token_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN active_daily_backup_token TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_backup_token_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let publication_token_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_token TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = publication_token_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let publication_generation_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_generation INTEGER",
    )
    .execute(pool)
    .await;
    if let Err(error) = publication_generation_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backups (
            rebuild_token TEXT NOT NULL,
            stats_date TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (rebuild_token, stats_date, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_date_publications (
            publication_token TEXT PRIMARY KEY,
            published INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection date publication table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_archive_compatibility (
            file_path TEXT PRIMARY KEY,
            archive_sha256 TEXT NOT NULL,
            file_fingerprint TEXT,
            has_legacy_crossing INTEGER NOT NULL,
            legacy_max_duration_ms REAL,
            legacy_min_occurred_at TEXT,
            has_rfc3339 INTEGER NOT NULL,
            rfc3339_max_duration_ms REAL,
            rfc3339_min_occurred_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive compatibility table")?;
    let archive_legacy_duration_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_max_duration_ms REAL",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_legacy_duration_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_legacy_start_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_min_occurred_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_legacy_start_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_fingerprint_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN file_fingerprint TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_fingerprint_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_rfc3339_duration_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_max_duration_ms REAL",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_rfc3339_duration_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_rfc3339_start_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_min_occurred_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_rfc3339_start_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_daily_backups_token_date ON long_term_projection_daily_backups (rebuild_token, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backup_claims (
            bucket_date TEXT PRIMARY KEY,
            rebuild_token TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup claim table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_intervals (
            bucket_kind TEXT NOT NULL,
            bucket_date TEXT NOT NULL,
            bucket_key TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            invocation_row_id INTEGER NOT NULL,
            interval_start_ms INTEGER NOT NULL,
            interval_end_ms INTEGER NOT NULL,
            PRIMARY KEY (
                bucket_kind,
                bucket_key,
                dimension,
                series_key,
                invocation_row_id,
                interval_start_ms,
                interval_end_ms
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval table")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_date ON long_term_projection_intervals (bucket_date, bucket_kind)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval date index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_invocation ON long_term_projection_intervals (invocation_row_id)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval invocation index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_interval_state (
            invocation_row_id INTEGER PRIMARY KEY,
            model_series_key TEXT NOT NULL,
            upstream_series_key TEXT NOT NULL,
            interval_start_ms INTEGER NOT NULL,
            interval_end_ms INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection canonical interval state table")?;
    sqlx::query("DROP INDEX IF EXISTS idx_long_term_projection_interval_state_range")
        .execute(pool)
        .await
        .context("failed to replace long-term projection interval range index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_state_end_start ON long_term_projection_interval_state (interval_end_ms, interval_start_ms)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection canonical interval range index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_interval_suppressions (
            invocation_row_id INTEGER NOT NULL,
            bucket_date TEXT NOT NULL,
            PRIMARY KEY (invocation_row_id, bucket_date)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval suppression table")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_suppressions_date ON long_term_projection_interval_suppressions (bucket_date, invocation_row_id)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval suppression date index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_rebuild_members (
            rebuild_token TEXT NOT NULL,
            invocation_row_id INTEGER NOT NULL,
            PRIMARY KEY (rebuild_token, invocation_row_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection rebuild membership table")?;
    Ok(())
}
