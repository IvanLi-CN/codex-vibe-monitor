use super::*;

async fn ensure_pool_oauth_session_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_oauth_login_sessions (
            login_id TEXT PRIMARY KEY,
            account_id INTEGER,
            display_name TEXT,
            email TEXT,
            group_name TEXT,
            group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            group_node_shunt_enabled INTEGER NOT NULL DEFAULT 0,
            group_node_shunt_enabled_requested INTEGER NOT NULL DEFAULT 0,
            group_single_account_rotation_enabled INTEGER NOT NULL DEFAULT 0,
            group_single_account_rotation_enabled_requested INTEGER NOT NULL DEFAULT 0,
            is_mother INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            tag_ids_json TEXT,
            group_note TEXT,
            group_concurrency_limit INTEGER NOT NULL DEFAULT 0,
            mailbox_session_id TEXT,
            generated_mailbox_address TEXT,
            state TEXT NOT NULL UNIQUE,
            pkce_verifier TEXT NOT NULL,
            redirect_uri TEXT NOT NULL,
            status TEXT NOT NULL,
            auth_url TEXT NOT NULL,
            error_message TEXT,
            pending_encrypted_credentials TEXT,
            pending_token_expires_at TEXT,
            pending_verified_email TEXT,
            pending_chatgpt_account_id TEXT,
            pending_chatgpt_user_id TEXT,
            pending_plan_type TEXT,
            pending_has_refresh_token INTEGER,
            expires_at TEXT NOT NULL,
            consumed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_oauth_login_sessions table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_egress_throttle (
            egress_key TEXT PRIMARY KEY,
            last_sent_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_egress_throttle table existence")?;
    Ok(())
}
async fn ensure_pool_oauth_session_columns(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_name")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_name")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "email")
        .await
        .context("failed to ensure pool_oauth_login_sessions.email")?;
    let existing_oauth_login_session_columns =
        load_sqlite_table_columns(pool, "pool_oauth_login_sessions").await?;
    if !existing_oauth_login_session_columns.contains("group_bound_proxy_keys_json") {
        sqlx::query(
            r#"
            ALTER TABLE pool_oauth_login_sessions
            ADD COLUMN group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_oauth_login_sessions.group_bound_proxy_keys_json")?;
    }
    if !existing_oauth_login_session_columns.contains("group_node_shunt_enabled") {
        sqlx::query(
            r#"
            ALTER TABLE pool_oauth_login_sessions
            ADD COLUMN group_node_shunt_enabled INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_oauth_login_sessions.group_node_shunt_enabled")?;
    }
    ensure_integer_column_with_default(
        pool,
        "pool_oauth_login_sessions",
        "group_node_shunt_enabled_requested",
        "0",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.group_node_shunt_enabled_requested")?;
    ensure_integer_column_with_default(
        pool,
        "pool_oauth_login_sessions",
        "group_single_account_rotation_enabled",
        "0",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.group_single_account_rotation_enabled")?;
    ensure_integer_column_with_default(
        pool,
        "pool_oauth_login_sessions",
        "group_single_account_rotation_enabled_requested",
        "0",
    )
    .await
    .context(
        "failed to ensure pool_oauth_login_sessions.group_single_account_rotation_enabled_requested",
    )?;
    Ok(())
}

async fn ensure_pool_oauth_session_pending_columns(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_note")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_note")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "mailbox_session_id")
        .await
        .context("failed to ensure pool_oauth_login_sessions.mailbox_session_id")?;
    ensure_nullable_text_column(
        pool,
        "pool_oauth_login_sessions",
        "generated_mailbox_address",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.generated_mailbox_address")?;
    ensure_integer_column_with_default(pool, "pool_oauth_login_sessions", "is_mother", "0")
        .await
        .context("failed to ensure pool_oauth_login_sessions.is_mother")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "tag_ids_json")
        .await
        .context("failed to ensure pool_oauth_login_sessions.tag_ids_json")?;
    ensure_integer_column_with_default(
        pool,
        "pool_oauth_login_sessions",
        "group_concurrency_limit",
        "0",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.group_concurrency_limit")?;
    ensure_nullable_text_column(
        pool,
        "pool_oauth_login_sessions",
        "pending_encrypted_credentials",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.pending_encrypted_credentials")?;
    ensure_nullable_text_column(
        pool,
        "pool_oauth_login_sessions",
        "pending_token_expires_at",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.pending_token_expires_at")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "pending_verified_email")
        .await
        .context("failed to ensure pool_oauth_login_sessions.pending_verified_email")?;
    ensure_nullable_text_column(
        pool,
        "pool_oauth_login_sessions",
        "pending_chatgpt_account_id",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.pending_chatgpt_account_id")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "pending_chatgpt_user_id")
        .await
        .context("failed to ensure pool_oauth_login_sessions.pending_chatgpt_user_id")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "pending_plan_type")
        .await
        .context("failed to ensure pool_oauth_login_sessions.pending_plan_type")?;
    ensure_nullable_integer_column(
        pool,
        "pool_oauth_login_sessions",
        "pending_has_refresh_token",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.pending_has_refresh_token")?;
    Ok(())
}

async fn ensure_pool_oauth_session_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_oauth_login_sessions_status_expires
        ON pool_oauth_login_sessions (status, expires_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_oauth_login_sessions_status_expires")?;

    Ok(())
}
