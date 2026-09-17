async fn ensure_pool_group_notes_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_group_notes (
            group_name TEXT PRIMARY KEY,
            note TEXT NOT NULL,
            bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            node_shunt_enabled INTEGER NOT NULL DEFAULT 0,
            single_account_rotation_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 0,
            concurrency_limit INTEGER NOT NULL DEFAULT 0,
            policy_allow_cut_out INTEGER,
            policy_allow_cut_in INTEGER,
            policy_priority_tier TEXT,
            policy_fast_mode_rewrite_mode TEXT,
            policy_image_tool_rewrite_mode TEXT,
            policy_codex_imagegen_rewrite_mode TEXT,
            policy_request_compression_algorithm TEXT,
            policy_concurrency_limit INTEGER,
            policy_upstream_429_retry_enabled INTEGER,
            policy_upstream_429_max_retries INTEGER,
            policy_available_models_json TEXT,
            policy_available_models_mode TEXT,
            policy_status_change_upstream_http_401 INTEGER,
            policy_status_change_upstream_http_402 INTEGER,
            policy_status_change_upstream_http_403 INTEGER,
            policy_status_change_reauth_required INTEGER,
            policy_status_change_upstream_http_429_rate_limit INTEGER,
            policy_status_change_upstream_http_429_quota_exhausted INTEGER,
            policy_status_change_usage_snapshot_exhausted INTEGER,
            policy_status_change_quota_still_exhausted INTEGER,
            policy_status_change_transport_failure INTEGER,
            policy_status_change_upstream_server_overloaded INTEGER,
            policy_status_change_upstream_http_5xx INTEGER,
            policy_responses_first_byte_timeout_secs INTEGER,
            policy_compact_first_byte_timeout_secs INTEGER,
            policy_image_first_byte_timeout_secs INTEGER,
            policy_responses_stream_timeout_secs INTEGER,
            policy_compact_stream_timeout_secs INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_group_notes table existence")?;
    Ok(())
}
async fn ensure_pool_group_notes_legacy_columns(pool: &Pool<Sqlite>) -> Result<()> {
    let existing_group_note_columns =
        load_sqlite_table_columns(pool, "pool_upstream_account_group_notes").await?;
    if !existing_group_note_columns.contains("bound_proxy_keys_json") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.bound_proxy_keys_json")?;
    }
    if !existing_group_note_columns.contains("node_shunt_enabled") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN node_shunt_enabled INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.node_shunt_enabled")?;
    }
    if !existing_group_note_columns.contains("single_account_rotation_enabled") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN single_account_rotation_enabled INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context(
            "failed to add pool_upstream_account_group_notes.single_account_rotation_enabled",
        )?;
    }
    if !existing_group_note_columns.contains("upstream_429_retry_enabled") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.upstream_429_retry_enabled")?;
    }
    if !existing_group_note_columns.contains("upstream_429_max_retries") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN upstream_429_max_retries INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.upstream_429_max_retries")?;
    }
    ensure_integer_column_with_default(
        pool,
        "pool_upstream_account_group_notes",
        "concurrency_limit",
        "0",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.concurrency_limit")?;
    Ok(())
}

async fn ensure_pool_group_notes_policy_columns(pool: &Pool<Sqlite>) -> Result<()> {
    for column in [
        "policy_allow_cut_out",
        "policy_allow_cut_in",
        "policy_concurrency_limit",
        "policy_upstream_429_retry_enabled",
        "policy_upstream_429_max_retries",
    ] {
        ensure_nullable_integer_column(pool, "pool_upstream_account_group_notes", column)
            .await
            .with_context(|| {
                format!("failed to ensure pool_upstream_account_group_notes.{column}")
            })?;
    }
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_priority_tier",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.policy_priority_tier")?;
    migrate_legacy_no_new_policy(
        pool,
        "pool_upstream_account_group_notes",
        "policy_priority_tier",
        "policy_block_new_conversations",
        "policy_allow_new_conversations",
    )
    .await
    .context(
        "failed to migrate pool_upstream_account_group_notes legacy new-conversation policy",
    )?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_fast_mode_rewrite_mode",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.policy_fast_mode_rewrite_mode")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_image_tool_rewrite_mode",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.policy_image_tool_rewrite_mode")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_codex_imagegen_rewrite_mode",
    )
    .await
    .context(
        "failed to ensure pool_upstream_account_group_notes.policy_codex_imagegen_rewrite_mode",
    )?;
    Ok(())
}

async fn ensure_pool_group_notes_extended_policy(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_request_compression_algorithm",
    )
    .await
    .context(
        "failed to ensure pool_upstream_account_group_notes.policy_request_compression_algorithm",
    )?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_available_models_json",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.policy_available_models_json")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_account_group_notes",
        "policy_available_models_mode",
    )
    .await
    .context("failed to ensure pool_upstream_account_group_notes.policy_available_models_mode")?;
    for column in [
        "policy_status_change_upstream_http_401",
        "policy_status_change_upstream_http_402",
        "policy_status_change_upstream_http_403",
        "policy_status_change_reauth_required",
        "policy_status_change_upstream_http_429_rate_limit",
        "policy_status_change_upstream_http_429_quota_exhausted",
        "policy_status_change_usage_snapshot_exhausted",
        "policy_status_change_quota_still_exhausted",
        "policy_status_change_transport_failure",
        "policy_status_change_upstream_server_overloaded",
        "policy_status_change_upstream_http_5xx",
    ] {
        ensure_nullable_integer_column(pool, "pool_upstream_account_group_notes", column)
            .await
            .with_context(|| {
                format!("failed to ensure pool_upstream_account_group_notes.{column}")
            })?;
    }
    for column in [
        "policy_responses_first_byte_timeout_secs",
        "policy_compact_first_byte_timeout_secs",
        "policy_image_first_byte_timeout_secs",
        "policy_responses_stream_timeout_secs",
        "policy_compact_stream_timeout_secs",
    ] {
        ensure_nullable_integer_column(pool, "pool_upstream_account_group_notes", column)
            .await
            .with_context(|| {
                format!("failed to ensure pool_upstream_account_group_notes.{column}")
            })?;
    }

    Ok(())
}
