use super::*;

async fn ensure_pool_tags(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            allow_cut_out INTEGER NOT NULL DEFAULT 1,
            allow_cut_in INTEGER NOT NULL DEFAULT 1,
            priority_tier TEXT NOT NULL DEFAULT 'normal',
            fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'keep_original',
            concurrency_limit INTEGER NOT NULL DEFAULT 0,
            upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 0,
            available_models_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_tags table existence")?;
    ensure_text_column_with_default(pool, "pool_tags", "priority_tier", "'normal'")
        .await
        .context("failed to ensure pool_tags.priority_tier")?;
    ensure_text_column_with_default(
        pool,
        "pool_tags",
        "fast_mode_rewrite_mode",
        "'keep_original'",
    )
    .await
    .context("failed to ensure pool_tags.fast_mode_rewrite_mode")?;
    ensure_integer_column_with_default(pool, "pool_tags", "concurrency_limit", "0")
        .await
        .context("failed to ensure pool_tags.concurrency_limit")?;
    ensure_integer_column_with_default(pool, "pool_tags", "upstream_429_retry_enabled", "0")
        .await
        .context("failed to ensure pool_tags.upstream_429_retry_enabled")?;
    ensure_integer_column_with_default(pool, "pool_tags", "upstream_429_max_retries", "0")
        .await
        .context("failed to ensure pool_tags.upstream_429_max_retries")?;
    ensure_text_column_with_default(pool, "pool_tags", "available_models_json", "'[]'")
        .await
        .context("failed to ensure pool_tags.available_models_json")?;
    ensure_nullable_text_column(pool, "pool_tags", "system_key")
        .await
        .context("failed to ensure pool_tags.system_key")?;
    ensure_integer_column_with_default(pool, "pool_tags", "protected", "0")
        .await
        .context("failed to ensure pool_tags.protected")?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_tags_system_key
        ON pool_tags (system_key)
        WHERE system_key IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_tags_system_key")?;

    ensure_gpt55_unsupported_system_tag(pool)
        .await
        .context("failed to ensure gpt-5.5 unsupported system tag")?;
    ensure_websocket_unsupported_system_tag(pool)
        .await
        .context("failed to ensure websocket unsupported system tag")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_tags (
            account_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (account_id, tag_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_tags table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_tags_tag_id
        ON pool_upstream_account_tags (tag_id, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_tags_tag_id")?;

    cleanup_non_system_tags(pool)
        .await
        .context("failed to delete legacy non-system tags")?;
    Ok(())
}
async fn ensure_pool_oauth_mailbox(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_oauth_mailbox_sessions (
            session_id TEXT PRIMARY KEY,
            remote_email_id TEXT NOT NULL,
            email_address TEXT NOT NULL,
            email_domain TEXT NOT NULL,
            mailbox_source TEXT,
            latest_code_value TEXT,
            latest_code_source TEXT,
            latest_code_updated_at TEXT,
            invite_subject TEXT,
            invite_copy_value TEXT,
            invite_copy_label TEXT,
            invite_updated_at TEXT,
            invited INTEGER NOT NULL DEFAULT 0,
            last_message_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_oauth_mailbox_sessions table existence")?;
    ensure_nullable_text_column(pool, "pool_oauth_mailbox_sessions", "mailbox_source")
        .await
        .context("failed to ensure pool_oauth_mailbox_sessions.mailbox_source")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_oauth_mailbox_sessions_expires_at
        ON pool_oauth_mailbox_sessions (expires_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_oauth_mailbox_sessions_expires_at")?;

    Ok(())
}
