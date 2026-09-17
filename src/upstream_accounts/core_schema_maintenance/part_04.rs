use super::*;

async fn ensure_external_api_keys(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS external_api_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            client_id TEXT NOT NULL,
            name TEXT NOT NULL,
            secret_hash TEXT NOT NULL,
            secret_prefix TEXT NOT NULL,
            status TEXT NOT NULL,
            last_used_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            rotated_from_key_id INTEGER,
            FOREIGN KEY(rotated_from_key_id) REFERENCES external_api_keys(id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure external_api_keys table existence")?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_external_api_keys_secret_hash
        ON external_api_keys (secret_hash)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_external_api_keys_secret_hash")?;

    sqlx::query("DROP INDEX IF EXISTS idx_external_api_keys_client_id")
        .execute(pool)
        .await
        .context("failed to drop legacy idx_external_api_keys_client_id")?;

    sqlx::query("DROP INDEX IF EXISTS idx_external_api_keys_active_client_id")
        .execute(pool)
        .await
        .context("failed to drop stale idx_external_api_keys_active_client_id")?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_external_api_keys_active_client_id
        ON external_api_keys (client_id)
        WHERE status = 'active'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_external_api_keys_active_client_id")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_external_api_keys_client_status
        ON external_api_keys (client_id, status)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_external_api_keys_client_status")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_external_api_keys_rotated_from
        ON external_api_keys (rotated_from_key_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_external_api_keys_rotated_from")?;
    Ok(())
}
async fn ensure_pool_account_events_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            occurred_epoch_ms INTEGER GENERATED ALWAYS AS (
                CAST(ROUND((
                    julianday(
                        occurred_at,
                        CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
                    ) - 2440587.5
                ) * 86400000.0) AS INTEGER)
            ) VIRTUAL,
            action TEXT NOT NULL,
            source TEXT NOT NULL,
            account_display_name TEXT,
            account_group_name TEXT,
            forward_proxy_key TEXT,
            forward_proxy_display_name TEXT,
            forward_proxy_egress_ip TEXT,
            result TEXT,
            result_description TEXT,
            reason_code TEXT,
            reason_message TEXT,
            http_status INTEGER,
            failure_kind TEXT,
            invoke_id TEXT,
            attempt_id INTEGER,
            sticky_key TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY(account_id) REFERENCES pool_upstream_accounts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_events table existence")?;
    let upstream_account_event_columns: std::collections::HashSet<String> =
        sqlx::query("PRAGMA table_xinfo('pool_upstream_account_events')")
            .fetch_all(pool)
            .await
            .context("failed to inspect pool_upstream_account_events schema")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !upstream_account_event_columns.contains("occurred_epoch_ms") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_events
            ADD COLUMN occurred_epoch_ms INTEGER GENERATED ALWAYS AS (
                CAST(ROUND((
                    julianday(
                        occurred_at,
                        CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
                    ) - 2440587.5
                ) * 86400000.0) AS INTEGER)
            ) VIRTUAL
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_events.occurred_epoch_ms")?;
    }
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_account_time
        ON pool_upstream_account_events (account_id, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_account_time")?;
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_time
        ON pool_upstream_account_events (occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_time")?;
    Ok(())
}

async fn ensure_pool_account_events_columns_and_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_upstream_account_events", "account_display_name")
        .await
        .context("failed to ensure pool_upstream_account_events.account_display_name")?;
    ensure_nullable_text_column(pool, "pool_upstream_account_events", "account_group_name")
        .await
        .context("failed to ensure pool_upstream_account_events.account_group_name")?;
    ensure_nullable_text_column(pool, "pool_upstream_account_events", "forward_proxy_key")
        .await
        .context("failed to ensure pool_upstream_account_events.forward_proxy_key")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_events",
        "forward_proxy_display_name",
    )
    .await
    .context("failed to ensure pool_upstream_account_events.forward_proxy_display_name")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_events",
        "forward_proxy_egress_ip",
    )
    .await
    .context("failed to ensure pool_upstream_account_events.forward_proxy_egress_ip")?;
    ensure_nullable_text_column(pool, "pool_upstream_account_events", "result")
        .await
        .context("failed to ensure pool_upstream_account_events.result")?;
    ensure_nullable_text_column(pool, "pool_upstream_account_events", "result_description")
        .await
        .context("failed to ensure pool_upstream_account_events.result_description")?;
    ensure_nullable_integer_column(pool, "pool_upstream_account_events", "attempt_id")
        .await
        .context("failed to ensure pool_upstream_account_events.attempt_id")?;
    for column in [
        "model",
        "model_route_state_before",
        "model_route_state_after",
        "model_route_priority_before",
        "model_route_priority_after",
        "model_route_cooldown_until",
    ] {
        ensure_nullable_text_column(pool, "pool_upstream_account_events", column)
            .await
            .with_context(|| format!("failed to ensure pool_upstream_account_events.{column}"))?;
    }
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_account_events",
        "model_route_failure_count",
    )
    .await
    .context("failed to ensure pool_upstream_account_events.model_route_failure_count")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_account_model_time
        ON pool_upstream_account_events (account_id, model, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_account_model_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_attempt_latest
        ON pool_upstream_account_events (attempt_id, occurred_epoch_ms DESC, id DESC)
        WHERE attempt_id IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_attempt_latest")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_timeline_unlinked_epoch
        ON pool_upstream_account_events (occurred_epoch_ms DESC, id DESC)
        WHERE attempt_id IS NULL AND model IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_timeline_unlinked_epoch")?;

    Ok(())
}
