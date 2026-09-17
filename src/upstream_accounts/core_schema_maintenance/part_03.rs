const POOL_UPSTREAM_ACCOUNTS_TABLE_SQL: &str = r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'codex',
            display_name TEXT NOT NULL,
            group_name TEXT,
            is_mother INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            status TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            email TEXT,
            verified_email TEXT,
            chatgpt_account_id TEXT,
            chatgpt_user_id TEXT,
            plan_type TEXT,
            plan_type_observed_at TEXT,
            masked_api_key TEXT,
            encrypted_credentials TEXT,
            has_refresh_token INTEGER NOT NULL DEFAULT 1,
            token_expires_at TEXT,
            last_refreshed_at TEXT,
            last_synced_at TEXT,
            last_successful_sync_at TEXT,
            last_error TEXT,
            last_error_at TEXT,
            last_action TEXT,
            last_action_source TEXT,
            last_action_reason_code TEXT,
            last_action_reason_message TEXT,
            last_action_http_status INTEGER,
            last_action_invoke_id TEXT,
            last_action_at TEXT,
            last_activity_at TEXT,
            last_selected_at TEXT,
            last_route_failure_at TEXT,
            last_route_failure_kind TEXT,
            cooldown_until TEXT,
            consecutive_route_failures INTEGER NOT NULL DEFAULT 0,
            temporary_route_failure_streak_started_at TEXT,
            compact_support_status TEXT,
            compact_support_observed_at TEXT,
            compact_support_reason TEXT,
            image_tool_capability TEXT NOT NULL DEFAULT 'unknown',
            response_endpoint_capability TEXT NOT NULL DEFAULT 'unknown',
            response_endpoint_capability_observed_at TEXT,
            response_endpoint_capability_reason TEXT,
            policy_response_endpoint_capability_override TEXT,
            chat_completions_capability TEXT NOT NULL DEFAULT 'unknown',
            chat_completions_capability_observed_at TEXT,
            chat_completions_capability_reason TEXT,
            policy_chat_completions_capability_override TEXT,
            image_endpoint_capability TEXT NOT NULL DEFAULT 'unknown',
            image_endpoint_capability_observed_at TEXT,
            image_endpoint_capability_reason TEXT,
            policy_image_endpoint_capability_override TEXT,
            response_image_tool_capability TEXT NOT NULL DEFAULT 'unknown',
            response_image_tool_capability_observed_at TEXT,
            response_image_tool_capability_reason TEXT,
            policy_response_image_tool_capability_override TEXT,
            codex_imagegen_capability TEXT NOT NULL DEFAULT 'unknown',
            codex_imagegen_capability_observed_at TEXT,
            codex_imagegen_capability_reason TEXT,
            policy_codex_imagegen_capability_override TEXT,
            standalone_search_capability TEXT NOT NULL DEFAULT 'unknown',
            standalone_search_capability_observed_at TEXT,
            standalone_search_capability_reason TEXT,
            policy_standalone_search_capability_override TEXT,
            local_primary_limit REAL,
            local_secondary_limit REAL,
            local_limit_unit TEXT,
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
            bound_proxy_keys_json TEXT,
            upstream_base_url TEXT,
            model_mappings_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
"#;

async fn ensure_pool_upstream_accounts_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(POOL_UPSTREAM_ACCOUNTS_TABLE_SQL)
        .execute(pool)
        .await
        .context("failed to ensure pool_upstream_accounts table existence")?;
    Ok(())
}
async fn ensure_pool_account_model_catalog(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_model_catalogs (
            account_id INTEGER PRIMARY KEY,
            models_json TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL DEFAULT 'never',
            last_attempted_at TEXT,
            last_successful_at TEXT,
            error_code TEXT,
            error_message TEXT,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(account_id) REFERENCES pool_upstream_accounts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_model_catalogs table existence")?;
    Ok(())
}

async fn ensure_pool_account_columns_core(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "group_name")
        .await
        .context("failed to ensure pool_upstream_accounts.group_name")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "deleted_at")
        .await
        .context("failed to ensure pool_upstream_accounts.deleted_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "verified_email")
        .await
        .context("failed to ensure pool_upstream_accounts.verified_email")?;
    ensure_integer_column_with_default(pool, "pool_upstream_accounts", "has_refresh_token", "1")
        .await
        .context("failed to ensure pool_upstream_accounts.has_refresh_token")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_selected_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_selected_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_route_failure_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_route_failure_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_route_failure_kind")
        .await
        .context("failed to ensure pool_upstream_accounts.last_route_failure_kind")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "cooldown_until")
        .await
        .context("failed to ensure pool_upstream_accounts.cooldown_until")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "compact_support_status")
        .await
        .context("failed to ensure pool_upstream_accounts.compact_support_status")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "compact_support_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.compact_support_observed_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "compact_support_reason")
        .await
        .context("failed to ensure pool_upstream_accounts.compact_support_reason")?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "model_mappings_json",
        "'[]'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.model_mappings_json")?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "image_tool_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.image_tool_capability")?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "response_endpoint_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.response_endpoint_capability")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "response_endpoint_capability_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.response_endpoint_capability_observed_at")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "response_endpoint_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.response_endpoint_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_response_endpoint_capability_override",
    )
    .await
    .context(
        "failed to ensure pool_upstream_accounts.policy_response_endpoint_capability_override",
    )?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "chat_completions_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.chat_completions_capability")?;
    Ok(())
}

async fn ensure_pool_account_columns_capabilities(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "chat_completions_capability_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.chat_completions_capability_observed_at")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "chat_completions_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.chat_completions_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_chat_completions_capability_override",
    )
    .await
    .context(
        "failed to ensure pool_upstream_accounts.policy_chat_completions_capability_override",
    )?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "image_endpoint_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.image_endpoint_capability")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "image_endpoint_capability_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.image_endpoint_capability_observed_at")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "image_endpoint_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.image_endpoint_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_image_endpoint_capability_override",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_image_endpoint_capability_override")?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "response_image_tool_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.response_image_tool_capability")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "response_image_tool_capability_observed_at",
    )
    .await
    .context(
        "failed to ensure pool_upstream_accounts.response_image_tool_capability_observed_at",
    )?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "response_image_tool_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.response_image_tool_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_response_image_tool_capability_override",
    )
    .await
    .context(
        "failed to ensure pool_upstream_accounts.policy_response_image_tool_capability_override",
    )?;
    Ok(())
}

async fn ensure_pool_account_columns_policies(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "codex_imagegen_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.codex_imagegen_capability")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "codex_imagegen_capability_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.codex_imagegen_capability_observed_at")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "codex_imagegen_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.codex_imagegen_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_codex_imagegen_capability_override",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_codex_imagegen_capability_override")?;
    ensure_text_column_with_default(
        pool,
        "pool_upstream_accounts",
        "standalone_search_capability",
        "'unknown'",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.standalone_search_capability")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "standalone_search_capability_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.standalone_search_capability_observed_at")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "standalone_search_capability_reason",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.standalone_search_capability_reason")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_standalone_search_capability_override",
    )
    .await
    .context(
        "failed to ensure pool_upstream_accounts.policy_standalone_search_capability_override",
    )?;
    ensure_integer_column_with_default(pool, "pool_upstream_accounts", "is_mother", "0")
        .await
        .context("failed to ensure pool_upstream_accounts.is_mother")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "upstream_base_url")
        .await
        .context("failed to ensure pool_upstream_accounts.upstream_base_url")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "bound_proxy_keys_json")
        .await
        .context("failed to ensure pool_upstream_accounts.bound_proxy_keys_json")?;
    ensure_nullable_integer_column(pool, "pool_upstream_accounts", "policy_allow_cut_out")
        .await
        .context("failed to ensure pool_upstream_accounts.policy_allow_cut_out")?;
    ensure_nullable_integer_column(pool, "pool_upstream_accounts", "policy_allow_cut_in")
        .await
        .context("failed to ensure pool_upstream_accounts.policy_allow_cut_in")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "policy_priority_tier")
        .await
        .context("failed to ensure pool_upstream_accounts.policy_priority_tier")?;
    migrate_legacy_no_new_policy(
        pool,
        "pool_upstream_accounts",
        "policy_priority_tier",
        "policy_block_new_conversations",
        "policy_allow_new_conversations",
    )
    .await
    .context("failed to migrate pool_upstream_accounts legacy new-conversation policy")?;
    Ok(())
}

async fn ensure_pool_account_columns_timeout(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_fast_mode_rewrite_mode",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_fast_mode_rewrite_mode")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_image_tool_rewrite_mode",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_image_tool_rewrite_mode")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_codex_imagegen_rewrite_mode",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_codex_imagegen_rewrite_mode")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_request_compression_algorithm",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_request_compression_algorithm")?;
    ensure_nullable_integer_column(pool, "pool_upstream_accounts", "policy_concurrency_limit")
        .await
        .context("failed to ensure pool_upstream_accounts.policy_concurrency_limit")?;
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_accounts",
        "policy_upstream_429_retry_enabled",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_upstream_429_retry_enabled")?;
    ensure_nullable_integer_column(
        pool,
        "pool_upstream_accounts",
        "policy_upstream_429_max_retries",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_upstream_429_max_retries")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_available_models_json",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_available_models_json")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "policy_available_models_mode",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.policy_available_models_mode")?;
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
        ensure_nullable_integer_column(pool, "pool_upstream_accounts", column)
            .await
            .with_context(|| format!("failed to ensure pool_upstream_accounts.{column}"))?;
    }
    for column in [
        "policy_responses_first_byte_timeout_secs",
        "policy_compact_first_byte_timeout_secs",
        "policy_image_first_byte_timeout_secs",
        "policy_responses_stream_timeout_secs",
        "policy_compact_stream_timeout_secs",
    ] {
        ensure_nullable_integer_column(pool, "pool_upstream_accounts", column)
            .await
            .with_context(|| format!("failed to ensure pool_upstream_accounts.{column}"))?;
    }
    Ok(())
}

async fn ensure_pool_account_columns_metadata(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "external_client_id")
        .await
        .context("failed to ensure pool_upstream_accounts.external_client_id")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "external_source_account_id")
        .await
        .context("failed to ensure pool_upstream_accounts.external_source_account_id")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "plan_type_observed_at")
        .await
        .context("failed to ensure pool_upstream_accounts.plan_type_observed_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_activity_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_activity_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_source")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_source")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_reason_code")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_reason_code")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_reason_message")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_reason_message")?;
    ensure_nullable_integer_column(pool, "pool_upstream_accounts", "last_action_http_status")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_http_status")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_invoke_id")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_invoke_id")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_at")?;
    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN last_activity_live_backfill_completed INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context(
            "failed to ensure pool_upstream_accounts.last_activity_live_backfill_completed",
        );
    }
    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN last_activity_archive_backfill_completed INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context(
            "failed to ensure pool_upstream_accounts.last_activity_archive_backfill_completed",
        );
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN consecutive_route_failures INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err)
            .context("failed to ensure pool_upstream_accounts.consecutive_route_failures");
    }
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "temporary_route_failure_streak_started_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.temporary_route_failure_streak_started_at")?;
    Ok(())
}

async fn ensure_pool_account_indexes_and_backfills(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_kind_enabled
        ON pool_upstream_accounts (kind, enabled)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_kind_enabled")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_maintenance_due
        ON pool_upstream_accounts (
            kind,
            enabled,
            status,
            cooldown_until,
            last_synced_at,
            last_successful_sync_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_maintenance_due")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_chatgpt_account_id
        ON pool_upstream_accounts (chatgpt_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_chatgpt_account_id")?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_upstream_accounts_external_source
        ON pool_upstream_accounts (external_client_id, external_source_account_id)
        WHERE external_client_id IS NOT NULL
          AND external_source_account_id IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_external_source")?;

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET verified_email = email
        WHERE kind = 'oauth_codex'
          AND verified_email IS NULL
          AND NULLIF(TRIM(email), '') IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill pool_upstream_accounts.verified_email from email")?;

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET group_name = ?1
        WHERE kind = ?2
          AND NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL
        "#,
    )
    .bind(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .execute(pool)
    .await
    .context("failed to normalize blank OAuth pool_upstream_accounts.group_name")?;

    Ok(())
}
