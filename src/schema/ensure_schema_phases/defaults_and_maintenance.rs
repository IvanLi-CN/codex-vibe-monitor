async fn ensure_schema_defaults_and_maintenance(pool: &Pool<Sqlite>) -> Result<()> {
    let default_proxy_urls_json =
        serde_json::to_string(&Vec::<String>::new()).context("serialize default proxy urls")?;
    let default_subscription_urls_json = serde_json::to_string(&Vec::<String>::new())
        .context("serialize default proxy subscription urls")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO forward_proxy_settings (
            id,
            proxy_urls_json,
            subscription_urls_json,
            subscription_update_interval_secs,
            insert_direct
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .bind(default_proxy_urls_json)
    .bind(default_subscription_urls_json)
    .bind(DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS as i64)
    .bind(DEFAULT_FORWARD_PROXY_INSERT_DIRECT as i64)
    .execute(pool)
    .await
    .context("failed to ensure default forward_proxy_settings row")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS startup_backfill_progress (
            task_name TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            next_run_after TEXT,
            zero_update_streak INTEGER NOT NULL DEFAULT 0,
            last_started_at TEXT,
            last_finished_at TEXT,
            last_scanned INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER NOT NULL DEFAULT 0,
            last_status TEXT NOT NULL DEFAULT 'idle'
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure startup_backfill_progress table existence")?;

    for (column, definition) in [
        ("suspension_reason", "TEXT"),
        ("next_probe_at", "TEXT"),
        ("wake_generation", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(pool, "startup_backfill_progress", column, definition)
            .await
            .with_context(|| format!("failed to ensure startup_backfill_progress.{column}"))?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_task_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_kind TEXT NOT NULL,
            trigger_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT,
            detail TEXT,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            duration_ms INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure system_task_runs table existence")?;

    sqlx::query(
        r#"
        UPDATE system_task_runs
        SET
            started_at = CASE
                -- Pre-ISO task rows used the application's Shanghai-local timestamp convention.
                WHEN date(substr(started_at, 1, 10)) = substr(started_at, 1, 10)
                    AND time(substr(started_at, 12, 8)) = substr(started_at, 12, 8)
                    AND started_at GLOB '????-??-?? ??:??:??*'
                    AND (started_at GLOB '????-??-?? ??:??:??*Z'
                        OR started_at GLOB '????-??-?? ??:??:??*[-+]??:??')
                    THEN COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', started_at), started_at)
                WHEN date(substr(started_at, 1, 10)) = substr(started_at, 1, 10)
                    AND time(substr(started_at, 12, 8)) = substr(started_at, 12, 8)
                    AND started_at GLOB '????-??-?? ??:??:??*'
                    THEN COALESCE(
                        strftime('%Y-%m-%dT%H:%M:%fZ', started_at, '-8 hours'),
                        started_at
                    )
                WHEN date(substr(started_at, 1, 10)) = substr(started_at, 1, 10)
                    AND time(substr(started_at, 12, 8)) = substr(started_at, 12, 8)
                    THEN COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', started_at), started_at)
                ELSE started_at
            END,
            finished_at = CASE
                WHEN finished_at IS NULL THEN NULL
                WHEN date(substr(finished_at, 1, 10)) = substr(finished_at, 1, 10)
                    AND time(substr(finished_at, 12, 8)) = substr(finished_at, 12, 8)
                    AND finished_at GLOB '????-??-?? ??:??:??*'
                    AND (finished_at GLOB '????-??-?? ??:??:??*Z'
                        OR finished_at GLOB '????-??-?? ??:??:??*[-+]??:??')
                    THEN COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', finished_at), finished_at)
                WHEN date(substr(finished_at, 1, 10)) = substr(finished_at, 1, 10)
                    AND time(substr(finished_at, 12, 8)) = substr(finished_at, 12, 8)
                    AND finished_at GLOB '????-??-?? ??:??:??*'
                    THEN COALESCE(
                        strftime('%Y-%m-%dT%H:%M:%fZ', finished_at, '-8 hours'),
                        finished_at
                    )
                WHEN date(substr(finished_at, 1, 10)) = substr(finished_at, 1, 10)
                    AND time(substr(finished_at, 12, 8)) = substr(finished_at, 12, 8)
                    THEN COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', finished_at), finished_at)
                ELSE finished_at
            END
        WHERE
            (started_at NOT GLOB '????-??-??T??:??:??.???Z'
                AND date(substr(started_at, 1, 10)) = substr(started_at, 1, 10)
                AND time(substr(started_at, 12, 8)) = substr(started_at, 12, 8)
                AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) IS NOT NULL)
            OR (finished_at IS NOT NULL
                AND finished_at NOT GLOB '????-??-??T??:??:??.???Z'
                AND date(substr(finished_at, 1, 10)) = substr(finished_at, 1, 10)
                AND time(substr(finished_at, 12, 8)) = substr(finished_at, 12, 8)
                AND strftime('%Y-%m-%dT%H:%M:%fZ', finished_at) IS NOT NULL)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to normalize system_task_runs timestamps to UTC ISO")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_system_task_runs_task_time
        ON system_task_runs (task_kind, started_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_system_task_runs_task_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_system_task_runs_status_time
        ON system_task_runs (status, started_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_system_task_runs_status_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_system_task_runs_started_at_id
        ON system_task_runs (started_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_system_task_runs_started_at_id")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_system_task_runs_task_status_time
        ON system_task_runs (task_kind, status, started_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_system_task_runs_task_status_time")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_raw_payload_metrics (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            inventory_state TEXT NOT NULL DEFAULT 'preparing',
            inventory_cursor INTEGER NOT NULL DEFAULT 0,
            raw_count INTEGER NOT NULL DEFAULT 0,
            raw_bytes INTEGER NOT NULL DEFAULT 0,
            request_raw_count INTEGER NOT NULL DEFAULT 0,
            request_raw_bytes INTEGER NOT NULL DEFAULT 0,
            response_raw_count INTEGER NOT NULL DEFAULT 0,
            response_raw_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure system_raw_payload_metrics table existence")?;

    let raw_metrics_columns = load_sqlite_table_columns(pool, "system_raw_payload_metrics").await?;
    if !raw_metrics_columns.contains("link_inventory_cursor") {
        sqlx::query(
            "ALTER TABLE system_raw_payload_metrics ADD COLUMN link_inventory_cursor INTEGER NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await
        .context("failed to add system raw payload link inventory cursor")?;
    }

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO system_raw_payload_metrics (singleton)
        VALUES (1)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to seed system raw payload metrics")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_raw_payload_inventory_paths (
            raw_path TEXT PRIMARY KEY,
            byte_size INTEGER NOT NULL,
            request_seen INTEGER NOT NULL DEFAULT 0,
            response_seen INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure system raw payload inventory path table existence")?;

    ensure_proxy_raw_payload_blob_link_schema(pool).await?;

    seed_default_pricing_catalog(pool).await?;
    ensure_long_term_stats_schema(pool).await?;
    ensure_upstream_accounts_schema(pool).await?;
    ensure_long_term_projection_account_trigger(pool).await?;
    ensure_summary_coverage_revision_schema(pool).await?;

    Ok(())
}
