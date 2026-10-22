use super::compatibility::{
    INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET,
    TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL, ensure_column_with_definition,
};
use super::*;

pub(crate) async fn backfill_upstream_account_usage_hourly_status_counts(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
    let resolved_failure_sql = crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let upstream_account_id_sql = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let bucket_epoch_sql = "((CASE
                WHEN instr(occurred_at, 'T') > 0
                    THEN CAST(strftime('%s', occurred_at) AS INTEGER)
                ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
            END) / 3600) * 3600";
    let terminal_status_sql = "(LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending'))";
    let live_backfill_sql = format!(
        r#"
        UPDATE upstream_account_usage_hourly
        SET
            success_count = (
                SELECT COUNT(*)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND {success_like_sql}
                  AND {resolved_failure_sql} = 'none'
            ),
            cache_input_tokens = (
                SELECT COALESCE(SUM(cache_input_tokens), 0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
            ),
            reasoning_tokens = (
                SELECT COALESCE(SUM(reasoning_tokens), 0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
            ),
            failure_count = (
                SELECT COUNT(*)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND {terminal_status_sql}
                  AND {resolved_failure_sql} IN ('service_failure', 'client_failure', 'client_abort')
            ),
            non_success_cost = (
                SELECT COALESCE(SUM(COALESCE(cost, 0.0)), 0.0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND (
                    LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                    OR (
                        {terminal_status_sql}
                        AND {resolved_failure_sql} IN ('service_failure', 'client_failure', 'client_abort')
                    )
                  )
            )
        WHERE EXISTS (
            SELECT 1
            FROM codex_invocations
            WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
              AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
        )
        "#,
    );
    sqlx::query(&live_backfill_sql)
        .execute(pool)
        .await
        .context("failed to backfill live upstream account hourly status counts")?;

    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_hourly
        WHERE EXISTS (
            SELECT 1
            FROM archive_batches AS batches
            JOIN hourly_rollup_archive_replay AS replay
              ON replay.dataset = batches.dataset
             AND replay.file_path = batches.file_path
             AND replay.target = 'upstream_account_usage_hourly'
            WHERE batches.dataset = 'codex_invocations'
              AND batches.status = 'completed'
              AND batches.coverage_start_at IS NOT NULL
              AND batches.coverage_end_at IS NOT NULL
              AND upstream_account_usage_hourly.bucket_start_epoch BETWEEN
                    (((CASE
                        WHEN instr(batches.coverage_start_at, 'T') > 0
                            THEN CAST(strftime('%s', batches.coverage_start_at) AS INTEGER)
                        ELSE CAST(strftime('%s', batches.coverage_start_at || '+08:00') AS INTEGER)
                    END) / 3600) * 3600)
                AND (((CASE
                        WHEN instr(batches.coverage_end_at, 'T') > 0
                            THEN CAST(strftime('%s', batches.coverage_end_at) AS INTEGER)
                        ELSE CAST(strftime('%s', batches.coverage_end_at || '+08:00') AS INTEGER)
                    END) / 3600) * 3600)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale archived upstream account hourly rollups")?;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND EXISTS (
              SELECT 1
              FROM hourly_rollup_archive_replay AS replay
              WHERE replay.dataset = archive_batches.dataset
                AND replay.file_path = archive_batches.file_path
                AND replay.target = 'upstream_account_usage_hourly'
          )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to reopen upstream account hourly archive materialization")?;

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_progress
        WHERE dataset = 'codex_invocations'
          AND EXISTS (
              SELECT 1
              FROM hourly_rollup_archive_replay AS replay
              WHERE replay.dataset = hourly_rollup_archive_progress.dataset
                AND replay.file_path = hourly_rollup_archive_progress.file_path
                AND replay.target = 'upstream_account_usage_hourly'
          )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale upstream account hourly archive progress")?;

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND target = 'upstream_account_usage_hourly'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale upstream account hourly archive replay markers")?;

    Ok(())
}

pub(crate) async fn reopen_upstream_account_stats_rollup_archives(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    for target in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND EXISTS (
                  SELECT 1
                  FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.dataset = archive_batches.dataset
                    AND replay.file_path = archive_batches.file_path
                    AND replay.target = ?1
              )
            "#,
        )
        .bind(target)
        .execute(pool)
        .await
        .with_context(|| format!("failed to reopen archive materialization for {target}"))?;

        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE dataset = 'codex_invocations'
              AND target = ?1
            "#,
        )
        .bind(target)
        .execute(pool)
        .await
        .with_context(|| format!("failed to clear stale archive replay markers for {target}"))?;
    }

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_progress
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale invocation archive progress while reopening upstream account stats rollups")?;

    Ok(())
}

pub(super) struct RollupBackfillState {
    has_existing_invocation_rollup_hourly_rows: bool,
    added_invocation_rollup_hourly_columns: bool,
    upstream_account_usage_hourly_needs_status_backfill: bool,
    upstream_account_stats_hourly_count: i64,
    upstream_account_stats_minute_count: i64,
    added_upstream_account_stats_columns: bool,
}

pub(super) async fn ensure_rollup_statistics_schema(
    pool: &Pool<Sqlite>,
) -> Result<RollupBackfillState> {
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

    let invocation_rollup_hourly_columns =
        load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    let has_existing_invocation_rollup_hourly_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM invocation_rollup_hourly)")
            .fetch_one(pool)
            .await?
            != 0;
    let mut added_invocation_rollup_hourly_columns = false;
    for (column, ty) in [
        ("terminal_count", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_cost", "REAL NOT NULL DEFAULT 0"),
        ("terminal_proof_complete", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_response_byte_total_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_max_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_invocation_rollup_hourly_columns = true;
            let statement =
                format!("ALTER TABLE invocation_rollup_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add invocation_rollup_hourly column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_hourly_source_bucket
        ON invocation_rollup_hourly (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_hourly_source_bucket")?;
    if has_existing_invocation_rollup_hourly_rows {
        // The state boundary is the durable completion marker. It must not depend on whether
        // this invocation added columns: a process can stop after the final ALTER TABLE and
        // before this bootstrap runs.
        ensure_long_term_stats_schema(pool).await?;
        let integrity_source_start_date = sqlx::query_scalar::<_, Option<String>>(
            "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_optional(pool)
        .await?
        .flatten();
        if integrity_source_start_date.is_none() {
            crate::long_term_stats::bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(pool)
                .await?;
        }
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_failure_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            failure_class TEXT NOT NULL,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            error_category TEXT NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, failure_class, is_actionable, error_category)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_failure_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_failure_rollup_hourly_bucket
        ON invocation_failure_rollup_hourly (bucket_start_epoch, source, failure_class)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_failure_rollup_hourly_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_perf_stage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            stage TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            sum_ms REAL NOT NULL,
            max_ms REAL NOT NULL,
            histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, stage)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_perf_stage_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_proxy_perf_stage_hourly_stage_bucket
        ON proxy_perf_stage_hourly (stage, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_proxy_perf_stage_hourly_stage_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_rollup_hourly_key_bucket
        ON prompt_cache_rollup_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_rollup_hourly_key_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_upstream_account_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            upstream_account_name TEXT,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_upstream_account_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_upstream_account_hourly_key_bucket
        ON prompt_cache_upstream_account_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_upstream_account_hourly_key_bucket")?;

    // These tables intentionally keep only the key identity needed to calculate
    // parallel-work averages. They are not a second copy of invocation history.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_minute_key_rollup_source_minute
        ON parallel_work_minute_key_rollup (source, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work minute source range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, upstream_account_id, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_minute_key_rollup_account_range
        ON parallel_work_upstream_account_minute_key_rollup (upstream_account_id, minute_start_epoch, source)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account minute range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_hourly_rollup_scope_range
        ON parallel_work_hourly_rollup (source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work hourly scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_id INTEGER NOT NULL,
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_hourly_rollup_account_range
        ON parallel_work_upstream_account_hourly_rollup (upstream_account_id, source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account hourly range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_coverage (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            minute_keys_complete INTEGER NOT NULL DEFAULT 0,
            hourly_scalar_complete INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_coverage table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_rollup_coverage_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            full_detail_start_epoch INTEGER NOT NULL,
            latest_unrecoverable_detail_epoch INTEGER
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_rollup_coverage_state table existence")?;

    ensure_column_with_definition(
        pool,
        "parallel_work_rollup_coverage_state",
        "latest_unrecoverable_detail_epoch",
        "INTEGER",
    )
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_records (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL,
            upstream_account_key INTEGER NOT NULL,
            records_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_records table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2 (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_key INTEGER NOT NULL,
            aggregate_json TEXT NOT NULL,
            total_latency_samples_json TEXT NOT NULL,
            first_byte_samples_json TEXT NOT NULL,
            first_response_byte_total_samples_json TEXT NOT NULL,
            first_token_samples_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            coverage_state TEXT NOT NULL DEFAULT 'warming',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_timeseries_minute_projection_v2_scope_range
        ON timeseries_minute_projection_v2 (source_scope, upstream_account_key, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 state table existence")?;

    sqlx::query(TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL)
        .execute(pool)
        .await
        .context("failed to ensure timeseries_minute_projection_v2 recovery table existence")?;

    // This durable fence must exist before runtime accepts HTTP reads. The projection supervisor
    // is intentionally P2 and can start later, while direct non-proxy terminal corrections must
    // synchronously publish only a constant-size recovery marker instead of mutating every
    // projection row in a terminal write transaction.
    let mut replacement_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin non-proxy terminal replacement trigger refresh")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_timeseries_minute_projection_non_proxy_terminal_replacement",
    )
    .execute(replacement_trigger_tx.as_mut())
    .await
    .context("failed to refresh non-proxy terminal replacement projection trigger")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_timeseries_minute_projection_non_proxy_terminal_replacement
        AFTER UPDATE ON codex_invocations
        WHEN (
            COALESCE(OLD.source, '') <> 'proxy'
            OR COALESCE(NEW.source, '') <> 'proxy'
        )
        AND LOWER(TRIM(COALESCE(OLD.status, ''))) NOT IN ('running', 'pending')
        AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
        AND (
            OLD.occurred_at IS NOT NEW.occurred_at
            OR OLD.source IS NOT NEW.source
            OR OLD.model IS NOT NEW.model
            OR OLD.input_tokens IS NOT NEW.input_tokens
            OR OLD.output_tokens IS NOT NEW.output_tokens
            OR OLD.cache_input_tokens IS NOT NEW.cache_input_tokens
            OR OLD.reasoning_tokens IS NOT NEW.reasoning_tokens
            OR OLD.total_tokens IS NOT NEW.total_tokens
            OR OLD.cost IS NOT NEW.cost
            OR OLD.status IS NOT NEW.status
            OR OLD.error_message IS NOT NEW.error_message
            OR OLD.failure_kind IS NOT NEW.failure_kind
            OR OLD.failure_class IS NOT NEW.failure_class
            OR OLD.is_actionable IS NOT NEW.is_actionable
            OR OLD.payload IS NOT NEW.payload
            OR OLD.t_total_ms IS NOT NEW.t_total_ms
            OR OLD.t_req_read_ms IS NOT NEW.t_req_read_ms
            OR OLD.t_req_parse_ms IS NOT NEW.t_req_parse_ms
            OR OLD.t_upstream_connect_ms IS NOT NEW.t_upstream_connect_ms
            OR OLD.t_upstream_ttfb_ms IS NOT NEW.t_upstream_ttfb_ms
            OR OLD.t_upstream_stream_ms IS NOT NEW.t_upstream_stream_ms
            OR OLD.first_token_ms IS NOT NEW.first_token_ms
        )
        BEGIN
            INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending, updated_at)
            VALUES ('timeseries_minute_v2', 1, 1, datetime('now'))
            ON CONFLICT(consumer) DO UPDATE SET
                generation = timeseries_minute_projection_v2_recovery.generation + 1,
                invalidation_pending = 1,
                updated_at = excluded.updated_at;
        END
        "#,
    )
    .execute(replacement_trigger_tx.as_mut())
    .await
    .context("failed to ensure non-proxy terminal replacement projection trigger")?;
    replacement_trigger_tx
        .commit()
        .await
        .context("failed to commit non-proxy terminal replacement trigger refresh")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_rollup_maintenance_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            next_hour_epoch INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_rollup_maintenance_state table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            non_success_cost REAL NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cache_input_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_usage_hourly table existence")?;

    let upstream_account_usage_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_usage_hourly").await?;
    let mut upstream_account_usage_hourly_needs_status_backfill = false;
    for (column, ty) in [
        ("success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("failure_count", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
    ] {
        if !upstream_account_usage_hourly_columns.contains(column) {
            upstream_account_usage_hourly_needs_status_backfill = true;
            let sql = format!("ALTER TABLE upstream_account_usage_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&sql).execute(pool).await.with_context(|| {
                format!("failed to add upstream_account_usage_hourly column {column}")
            })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_hourly_account_bucket
        ON upstream_account_usage_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_usage_breakdown_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            normalized_model TEXT NOT NULL,
            normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_input REAL NOT NULL DEFAULT 0,
            cost_cache_write REAL NOT NULL DEFAULT 0,
            cost_cache_read REAL NOT NULL DEFAULT 0,
            cost_output REAL NOT NULL DEFAULT 0,
            cost_reasoning REAL NOT NULL DEFAULT 0,
            cost_unknown REAL NOT NULL DEFAULT 0,
            has_cost INTEGER NOT NULL DEFAULT 0,
            performance_total_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
            performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_response_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_account_key,
                normalized_model,
                normalized_reasoning_effort
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_usage_breakdown_hourly table existence")?;

    for (column, definition) in [
        (
            "performance_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("performance_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(
            pool,
            "upstream_account_usage_breakdown_hourly",
            column,
            definition,
        )
        .await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_breakdown_hourly_account_bucket
        ON upstream_account_usage_breakdown_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_breakdown_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
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
            activity_v2_request_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_cost REAL NOT NULL DEFAULT 0,
            activity_v2_non_success_cost REAL NOT NULL DEFAULT 0,
            activity_v2_cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_cost REAL NOT NULL DEFAULT 0,
            activity_v2_first_response_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_response_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_max_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_last_invocation_at TEXT,
            activity_v2_latest_unkeyed_conversation_at TEXT,
            activity_v2_latest_first_response_at TEXT,
            activity_v2_latest_first_response_ms REAL,
            activity_v2_latest_total_latency_at TEXT,
            activity_v2_latest_total_latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_hourly table existence")?;
    let upstream_account_stats_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_hourly").await?;
    let mut added_upstream_account_stats_columns = false;
    for (column, ty) in [
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !upstream_account_stats_hourly_columns.contains(column) {
            added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    for (column, ty) in [
        ("activity_v2_request_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_count", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_tokens", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_failure_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_cost", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_non_success_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_cache_input_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_response_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_response_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        (
            "activity_v2_total_latency_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_total_latency_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        ("activity_v2_last_invocation_at", "TEXT"),
        ("activity_v2_latest_unkeyed_conversation_at", "TEXT"),
        ("activity_v2_latest_first_response_at", "TEXT"),
        ("activity_v2_latest_first_response_ms", "REAL"),
        ("activity_v2_latest_total_latency_at", "TEXT"),
        ("activity_v2_latest_total_latency_ms", "REAL"),
    ] {
        if !upstream_account_stats_hourly_columns.contains(column) {
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    let upstream_account_stats_hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_hourly rows")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_account_bucket
        ON upstream_account_stats_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_source_account_bucket
        ON upstream_account_stats_hourly (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_source_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
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
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_minute table existence")?;
    let upstream_account_stats_minute_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_minute").await?;
    for (column, ty) in [
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !upstream_account_stats_minute_columns.contains(column) {
            added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_minute ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_minute column {column}")
                })?;
        }
    }
    let upstream_account_stats_minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_minute rows")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_account_bucket
        ON upstream_account_stats_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_source_account_bucket
        ON upstream_account_stats_minute (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_source_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_host_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_host_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_host_bucket
        ON upstream_host_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_host_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_source_host_bucket
        ON upstream_host_network_minute (source, upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_source_host_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_socket_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upstream_account_id INTEGER,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_socket_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_account_bucket
        ON upstream_socket_network_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_source_bucket
        ON upstream_socket_network_minute (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_source_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_host_bucket
        ON upstream_socket_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_host_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_sticky_key_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            sticky_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id, sticky_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_sticky_key_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_sticky_key_hourly_account_bucket
        ON upstream_sticky_key_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_sticky_key_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempt_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            latency_sample_count INTEGER NOT NULL DEFAULT 0,
            latency_sum_ms REAL NOT NULL DEFAULT 0,
            latency_max_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempt_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempt_hourly_bucket_proxy
        ON forward_proxy_attempt_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempt_hourly_bucket_proxy")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_node_health_archive (
            archive_file_path TEXT NOT NULL,
            archived_row_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            proxy_binding_key_snapshot TEXT NOT NULL,
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_file_path, archived_row_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_node_health_archive table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_occurred_at_binding
        ON pool_upstream_node_health_archive (occurred_at, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_occurred_at_binding")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_file
        ON pool_upstream_node_health_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_file")?;

    Ok(RollupBackfillState {
        has_existing_invocation_rollup_hourly_rows,
        added_invocation_rollup_hourly_columns,
        upstream_account_usage_hourly_needs_status_backfill,
        upstream_account_stats_hourly_count,
        upstream_account_stats_minute_count,
        added_upstream_account_stats_columns,
    })
}

pub(super) async fn finish_rollup_backfills(
    pool: &Pool<Sqlite>,
    state: RollupBackfillState,
) -> Result<()> {
    let RollupBackfillState {
        has_existing_invocation_rollup_hourly_rows,
        added_invocation_rollup_hourly_columns,
        upstream_account_usage_hourly_needs_status_backfill,
        upstream_account_stats_hourly_count,
        upstream_account_stats_minute_count,
        added_upstream_account_stats_columns,
    } = state;
    if has_existing_invocation_rollup_hourly_rows {
        let reconciliation_complete = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE((SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1), 0)",
        )
        .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
        .fetch_one(pool)
        .await?
            != 0;
        if !reconciliation_complete || added_invocation_rollup_hourly_columns {
            let reconciliation = backfill_invocation_rollup_hourly_from_sources(pool).await?;
            if reconciliation.source_complete {
                sqlx::query(
                    r#"
                    INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
                    VALUES (?1, 1)
                    ON CONFLICT(dataset) DO UPDATE SET
                        cursor_id = excluded.cursor_id,
                        updated_at = datetime('now')
                    "#,
                )
                .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
                .execute(pool)
                .await?;
            }
            info!(
                rebuilt_rows = reconciliation.applied_rollups,
                source_complete = reconciliation.source_complete,
                "backfilled invocation hourly rollups after adding aggregate columns"
            );
        }
    }

    if upstream_account_usage_hourly_needs_status_backfill {
        backfill_upstream_account_usage_hourly_status_counts(pool).await?;
    }
    if upstream_account_stats_hourly_count == 0
        || upstream_account_stats_minute_count == 0
        || added_upstream_account_stats_columns
    {
        rebuild_upstream_account_stats_rollups_from_sources(pool)
            .await
            .context("failed to rebuild upstream account stats rollups from sources")?;
    }

    Ok(())
}
