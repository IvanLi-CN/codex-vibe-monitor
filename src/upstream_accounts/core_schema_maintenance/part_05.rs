use super::*;

async fn ensure_pool_account_model_routes_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_model_routes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            model TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'available',
            priority TEXT NOT NULL DEFAULT 'normal',
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            streak_started_at TEXT,
            changed_at TEXT,
            last_seen_at TEXT NOT NULL,
            last_success_at TEXT,
            last_failure_at TEXT,
            last_failure_kind TEXT,
            last_failure_message TEXT,
            cooldown_until TEXT,
            cache_concurrency_limit INTEGER,
            cache_recovery_limit INTEGER,
            cache_low_hit_streak INTEGER NOT NULL DEFAULT 0,
            cache_cooldown_level INTEGER NOT NULL DEFAULT 0,
            cache_last_hit_rate_percent INTEGER,
            cache_usage_missing_since TEXT,
            cache_usage_missing_reason TEXT,
            UNIQUE(account_id, model),
            FOREIGN KEY(account_id) REFERENCES pool_upstream_accounts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_model_routes table existence")?;
    Ok(())
}
async fn ensure_pool_account_model_route_columns(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_upstream_account_model_routes", "reset_fence_at")
        .await
        .context("failed to ensure pool_upstream_account_model_routes.reset_fence_at")?;
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_account_model_routes",
        "cache_concurrency_limit",
    )
    .await
    .context("failed to ensure model route cache_concurrency_limit")?;
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_account_model_routes",
        "cache_recovery_limit",
    )
    .await
    .context("failed to ensure model route cache_recovery_limit")?;
    ensure_integer_column_with_default(
        pool,
        "pool_upstream_account_model_routes",
        "cache_low_hit_streak",
        "0",
    )
    .await
    .context("failed to ensure model route cache_low_hit_streak")?;
    ensure_integer_column_with_default(
        pool,
        "pool_upstream_account_model_routes",
        "cache_cooldown_level",
        "0",
    )
    .await
    .context("failed to ensure model route cache_cooldown_level")?;
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_account_model_routes",
        "cache_last_hit_rate_percent",
    )
    .await
    .context("failed to ensure model route cache_last_hit_rate_percent")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_model_routes",
        "cache_usage_missing_since",
    )
    .await
    .context("failed to ensure model route cache_usage_missing_since")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_model_routes",
        "cache_usage_missing_reason",
    )
    .await
    .context("failed to ensure model route cache_usage_missing_reason")?;
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_model_routes_account_seen
        ON pool_upstream_account_model_routes (account_id, last_seen_at DESC, model COLLATE NOCASE)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure model route account index")?;
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_model_routes_model_seen
        ON pool_upstream_account_model_routes (model COLLATE NOCASE, last_seen_at DESC, account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure model route model index")?;
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_model_routes_cooldown
        ON pool_upstream_account_model_routes (state, cooldown_until)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure model route cooldown index")?;
    Ok(())
}

async fn ensure_pool_account_model_route_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_proxy_time
        ON pool_upstream_account_events (forward_proxy_key, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_proxy_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_result_time
        ON pool_upstream_account_events (result, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_result_time")?;

    Ok(())
}
