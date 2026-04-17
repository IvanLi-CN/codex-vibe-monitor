pub(crate) async fn ensure_upstream_accounts_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
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
            chatgpt_account_id TEXT,
            chatgpt_user_id TEXT,
            plan_type TEXT,
            plan_type_observed_at TEXT,
            masked_api_key TEXT,
            encrypted_credentials TEXT,
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
            local_primary_limit REAL,
            local_secondary_limit REAL,
            local_limit_unit TEXT,
            upstream_base_url TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_accounts table existence")?;

    ensure_nullable_text_column(pool, "pool_upstream_accounts", "group_name")
        .await
        .context("failed to ensure pool_upstream_accounts.group_name")?;
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
    ensure_integer_column_with_default(pool, "pool_upstream_accounts", "is_mother", "0")
        .await
        .context("failed to ensure pool_upstream_accounts.is_mother")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "upstream_base_url")
        .await
        .context("failed to ensure pool_upstream_accounts.upstream_base_url")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "external_client_id")
        .await
        .context("failed to ensure pool_upstream_accounts.external_client_id")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "external_source_account_id",
    )
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

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            action TEXT NOT NULL,
            source TEXT NOT NULL,
            reason_code TEXT,
            reason_message TEXT,
            http_status INTEGER,
            failure_kind TEXT,
            invoke_id TEXT,
            sticky_key TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY(account_id) REFERENCES pool_upstream_accounts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_events table existence")?;
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
        CREATE TABLE IF NOT EXISTS pool_oauth_login_sessions (
            login_id TEXT PRIMARY KEY,
            account_id INTEGER,
            display_name TEXT,
            group_name TEXT,
            group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            group_node_shunt_enabled INTEGER NOT NULL DEFAULT 0,
            group_node_shunt_enabled_requested INTEGER NOT NULL DEFAULT 0,
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

    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_name")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_name")?;
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

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            guard_enabled INTEGER NOT NULL DEFAULT 0,
            lookback_hours INTEGER,
            max_conversations INTEGER,
            allow_cut_out INTEGER NOT NULL DEFAULT 1,
            allow_cut_in INTEGER NOT NULL DEFAULT 1,
            priority_tier TEXT NOT NULL DEFAULT 'normal',
            fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'keep_original',
            concurrency_limit INTEGER NOT NULL DEFAULT 0,
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

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_group_notes (
            group_name TEXT PRIMARY KEY,
            note TEXT NOT NULL,
            bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            node_shunt_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 0,
            concurrency_limit INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_group_notes table existence")?;
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
        CREATE TABLE IF NOT EXISTS pool_routing_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            encrypted_api_key TEXT,
            masked_api_key TEXT,
            primary_sync_interval_secs INTEGER,
            secondary_sync_interval_secs INTEGER,
            priority_available_account_cap INTEGER,
            responses_first_byte_timeout_secs INTEGER,
            compact_first_byte_timeout_secs INTEGER,
            responses_stream_timeout_secs INTEGER,
            compact_stream_timeout_secs INTEGER,
            default_first_byte_timeout_secs INTEGER,
            upstream_handshake_timeout_secs INTEGER,
            request_read_timeout_secs INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_routing_settings table existence")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "primary_sync_interval_secs")
        .await
        .context("failed to ensure pool_routing_settings.primary_sync_interval_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "secondary_sync_interval_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.secondary_sync_interval_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "priority_available_account_cap",
    )
    .await
    .context("failed to ensure pool_routing_settings.priority_available_account_cap")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "responses_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.responses_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "compact_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.compact_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "responses_stream_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.responses_stream_timeout_secs")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "compact_stream_timeout_secs")
        .await
        .context("failed to ensure pool_routing_settings.compact_stream_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "default_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.default_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "upstream_handshake_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.upstream_handshake_timeout_secs")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "request_read_timeout_secs")
        .await
        .context("failed to ensure pool_routing_settings.request_read_timeout_secs")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_routing_settings (
            id,
            encrypted_api_key,
            masked_api_key,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
        ) VALUES (?1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to ensure default pool_routing_settings row")?;

    Ok(())
}

pub(crate) fn spawn_upstream_account_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("upstream account maintenance stopped");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_upstream_account_maintenance_once(state.clone()).await {
                        warn!(error = %err, "failed to run upstream account maintenance");
                    }
                }
            }
        }
    })
}

async fn ensure_nullable_text_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

async fn ensure_nullable_integer_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} INTEGER");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

async fn ensure_text_column_with_default(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
    default_value: &str,
) -> Result<()> {
    let pragma_statement = format!("PRAGMA table_info({table_name})");
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&pragma_statement).fetch_all(pool).await?;
    if columns
        .iter()
        .any(|(_, name, _, _, _, _)| name == column_name)
    {
        return Ok(());
    }

    let statement = format!(
        "ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT NOT NULL DEFAULT {default_value}"
    );
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

async fn sqlite_table_exists(pool: &Pool<Sqlite>, table_name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await?
        > 0)
}
