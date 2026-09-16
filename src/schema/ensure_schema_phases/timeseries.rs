async fn ensure_schema_timeseries(pool: &Pool<Sqlite>) -> Result<()> {
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

    Ok(())
}
