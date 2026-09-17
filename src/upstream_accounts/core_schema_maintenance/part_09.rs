use super::*;

async fn ensure_pool_limit_samples(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_limit_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            captured_at TEXT NOT NULL,
            limit_id TEXT,
            limit_name TEXT,
            plan_type TEXT,
            primary_used_percent REAL,
            primary_window_minutes INTEGER,
            primary_resets_at TEXT,
            secondary_used_percent REAL,
            secondary_window_minutes INTEGER,
            secondary_resets_at TEXT,
            credits_has_credits INTEGER,
            credits_unlimited INTEGER,
            credits_balance TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_limit_samples table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_limit_samples_account_captured_at
        ON pool_upstream_account_limit_samples (account_id, captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_limit_samples_account_captured_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_limit_samples_account_captured_desc
        ON pool_upstream_account_limit_samples (account_id, captured_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_limit_samples_account_captured_desc")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_limit_samples_account_plan_type_desc
        ON pool_upstream_account_limit_samples (account_id, captured_at DESC, id DESC)
        WHERE plan_type IS NOT NULL AND TRIM(plan_type) <> ''
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_limit_samples_account_plan_type_desc")?;
    Ok(())
}
async fn ensure_pool_sticky_routes(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_sticky_routes (
            sticky_key TEXT PRIMARY KEY,
            account_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_sticky_routes table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_sticky_routes_account_updated
        ON pool_sticky_routes (account_id, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_sticky_routes_account_updated")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_sticky_routes_account_last_seen
        ON pool_sticky_routes (account_id, last_seen_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_sticky_routes_account_last_seen")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_sticky_model_routes (
            sticky_key TEXT NOT NULL,
            model_key TEXT NOT NULL,
            account_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            PRIMARY KEY (sticky_key, model_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_sticky_model_routes table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_sticky_model_routes_account_updated
        ON pool_sticky_model_routes (account_id, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_sticky_model_routes_account_updated")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_sticky_model_route_generations (
            sticky_key TEXT NOT NULL,
            model_key TEXT NOT NULL,
            generation INTEGER NOT NULL DEFAULT 0,
            last_clear_cause_attempt_public_id TEXT,
            last_clear_cause_http_status INTEGER,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (sticky_key, model_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_sticky_model_route_generations table existence")?;
    ensure_nullable_text_column(
        pool,
        "pool_sticky_model_route_generations",
        "last_clear_cause_attempt_public_id",
    )
    .await?;
    ensure_nullable_integer_column(
        pool,
        "pool_sticky_model_route_generations",
        "last_clear_cause_http_status",
    )
    .await?;
    Ok(())
}

async fn ensure_pool_sticky_route_generations(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_sticky_route_generations (
            sticky_key TEXT PRIMARY KEY,
            generation INTEGER NOT NULL DEFAULT 0,
            last_clear_cause_attempt_public_id TEXT,
            last_clear_cause_http_status INTEGER,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_sticky_route_generations table existence")?;
    ensure_nullable_text_column(
        pool,
        "pool_sticky_route_generations",
        "last_clear_cause_attempt_public_id",
    )
    .await?;
    ensure_nullable_integer_column(
        pool,
        "pool_sticky_route_generations",
        "last_clear_cause_http_status",
    )
    .await?;

    Ok(())
}
