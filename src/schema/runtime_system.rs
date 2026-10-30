use super::compatibility::{
    LEGACY_RAW_BLOB_LINK_SEED_MIGRATION_NAME, RETIRE_OPENAI_WEBSOCKET_PROXY_MIGRATION_NAME,
    ensure_column_with_definition, ensure_nullable_real_column,
    record_schema_refresh_completion_in_transaction,
};
use super::*;

pub(super) async fn ensure_runtime_system_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let proxy_model_settings_existing_columns =
        sqlx::query("PRAGMA table_info('proxy_model_settings')")
            .fetch_all(pool)
            .await
            .context("failed to inspect proxy_model_settings columns")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect::<Vec<_>>();
    let proxy_model_settings_had_owner_routing_column = proxy_model_settings_existing_columns
        .iter()
        .any(|column| column == "encrypted_session_owner_routing_enabled");
    let proxy_model_settings_had_owner_routing_init_column = proxy_model_settings_existing_columns
        .iter()
        .any(|column| column == "encrypted_session_owner_routing_initialized");
    let proxy_model_settings_had_singleton_row = if proxy_model_settings_existing_columns.is_empty()
    {
        false
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM proxy_model_settings
            WHERE id = ?1
            "#,
        )
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .fetch_one(pool)
        .await
        .context("failed to inspect proxy_model_settings singleton existence")?
            > 0
    };

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled',
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 3,
            openai_proxy_websocket_enabled INTEGER NOT NULL DEFAULT 0,
            openai_proxy_upstream_websocket_default_enabled INTEGER NOT NULL DEFAULT 0,
            request_body_logging_enabled INTEGER NOT NULL DEFAULT 1,
            response_body_logging_enabled INTEGER NOT NULL DEFAULT 1,
            encrypted_session_owner_routing_enabled INTEGER NOT NULL DEFAULT 0,
            encrypted_session_owner_routing_initialized INTEGER NOT NULL DEFAULT 0,
            websocket_settings_migrated INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_model_settings table existence")?;

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN enabled_preset_models_json TEXT
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure enabled_preset_models_json column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled'
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure fast_mode_rewrite_mode column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN preset_models_migrated INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure preset_models_migrated column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN upstream_429_max_retries INTEGER NOT NULL DEFAULT 3
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure upstream_429_max_retries column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN openai_proxy_websocket_enabled INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure openai_proxy_websocket_enabled column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN openai_proxy_upstream_websocket_default_enabled INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err)
            .context("failed to ensure openai_proxy_upstream_websocket_default_enabled column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN request_body_logging_enabled INTEGER NOT NULL DEFAULT 1
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure request_body_logging_enabled column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN response_body_logging_enabled INTEGER NOT NULL DEFAULT 1
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure response_body_logging_enabled column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN encrypted_session_owner_routing_enabled INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure encrypted_session_owner_routing_enabled column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN encrypted_session_owner_routing_initialized INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err)
            .context("failed to ensure encrypted_session_owner_routing_initialized column");
    }

    if proxy_model_settings_had_owner_routing_column
        && !proxy_model_settings_had_owner_routing_init_column
        && proxy_model_settings_had_singleton_row
    {
        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET encrypted_session_owner_routing_initialized = 1
            WHERE id = ?1
            "#,
        )
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(pool)
        .await
        .context("failed to preserve initialized encrypted owner routing settings")?;
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN websocket_settings_migrated INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure websocket_settings_migrated column");
    }

    let default_enabled_models_json = serde_json::to_string(&default_enabled_preset_models())
        .context("failed to serialize default enabled preset models")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries,
            openai_proxy_websocket_enabled,
            openai_proxy_upstream_websocket_default_enabled,
            request_body_logging_enabled,
            response_body_logging_enabled,
            encrypted_session_owner_routing_enabled,
            encrypted_session_owner_routing_initialized,
            websocket_settings_migrated,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
    .bind(i64::from(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES))
    .bind(0_i64)
    .bind(0_i64)
    .bind(1_i64)
    .bind(1_i64)
    .bind(DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED as i64)
    .bind(0_i64)
    .bind(0_i64)
    .bind(default_enabled_models_json)
    .execute(pool)
    .await
    .context("failed to ensure default proxy_model_settings row")?;

    ensure_proxy_enabled_models_contains_new_presets(pool)
        .await
        .context("failed to ensure proxy preset models list is up-to-date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            catalog_version TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_meta table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_models (
            model TEXT PRIMARY KEY,
            input_per_1m REAL NOT NULL,
            output_per_1m REAL NOT NULL,
            cache_input_per_1m REAL,
            cache_read_per_1m REAL,
            cache_write_per_1m REAL,
            reasoning_per_1m REAL,
            source TEXT NOT NULL DEFAULT 'custom',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_models table existence")?;

    ensure_nullable_real_column(pool, "pricing_settings_models", "cache_read_per_1m")
        .await
        .context("failed to ensure pricing_settings_models.cache_read_per_1m")?;
    ensure_nullable_real_column(pool, "pricing_settings_models", "cache_write_per_1m")
        .await
        .context("failed to ensure pricing_settings_models.cache_write_per_1m")?;
    ensure_pricing_settings_catalog_migrations(pool).await?;
    crate::models_dev_sync_memory::ensure_models_dev_sync_memory(pool).await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS oauth_bridge_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            installation_seed TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure oauth_bridge_settings table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            proxy_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_update_interval_secs INTEGER NOT NULL DEFAULT 3600,
            insert_direct INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_settings table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_runtime (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            weight REAL NOT NULL,
            success_ema REAL NOT NULL,
            latency_ema_ms REAL,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            is_penalized INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_runtime table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_metadata_history (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            egress_ip TEXT,
            egress_ip_provider TEXT,
            egress_ip_checked_at TEXT,
            egress_ip_error TEXT,
            egress_ip_error_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_metadata_history table existence")?;

    let forward_proxy_metadata_columns =
        load_sqlite_table_columns(pool, "forward_proxy_metadata_history").await?;
    for (column, ty) in [
        ("egress_ip", "TEXT"),
        ("egress_ip_provider", "TEXT"),
        ("egress_ip_checked_at", "TEXT"),
        ("egress_ip_error", "TEXT"),
        ("egress_ip_error_at", "TEXT"),
    ] {
        if !forward_proxy_metadata_columns.contains(column) {
            let statement =
                format!("ALTER TABLE forward_proxy_metadata_history ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add forward_proxy_metadata_history column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proxy_key TEXT NOT NULL,
            occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            failure_kind TEXT,
            is_probe INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempts table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_request_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            attempt_public_id TEXT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            invocation_id INTEGER,
            occurred_epoch_ms INTEGER GENERATED ALWAYS AS (
                CAST(ROUND((
                    julianday(
                        occurred_at,
                        CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
                    ) - 2440587.5
                ) * 86400000.0) AS INTEGER)
            ) VIRTUAL,
            endpoint TEXT NOT NULL,
            route_mode TEXT NOT NULL,
            sticky_key TEXT,
            routing_source TEXT,
            routing_selection_audit_json TEXT,
            upstream_base_url_host TEXT,
            group_name_snapshot TEXT,
            proxy_binding_key_snapshot TEXT,
            request_model TEXT,
            upstream_request_model TEXT,
            model_mapping_pattern TEXT,
            upstream_account_id INTEGER,
            upstream_route_key TEXT,
            attempt_index INTEGER NOT NULL,
            distinct_account_index INTEGER NOT NULL,
            same_account_retry_index INTEGER NOT NULL,
            requester_ip TEXT,
            started_at TEXT,
            finished_at TEXT,
            status TEXT NOT NULL,
            phase TEXT,
            http_status INTEGER,
            downstream_http_status INTEGER,
            failure_kind TEXT,
            error_message TEXT,
            downstream_error_message TEXT,
            connect_latency_ms REAL,
            first_byte_latency_ms REAL,
            stream_latency_ms REAL,
            upstream_request_id TEXT,
            upstream_request_compression_algorithm TEXT,
            upstream_request_compression_mode TEXT,
            upstream_request_logical_body_bytes INTEGER,
            upstream_request_transmitted_body_bytes INTEGER,
            upstream_request_header_bytes_approx INTEGER,
            upstream_response_body_bytes INTEGER,
            upstream_response_header_bytes_approx INTEGER,
            compact_support_status TEXT,
            compact_support_reason TEXT,
            request_summary_json TEXT,
            response_summary_json TEXT,
            response_raw_path TEXT,
            response_raw_codec TEXT NOT NULL DEFAULT 'identity',
            response_raw_size INTEGER,
            response_raw_truncated INTEGER NOT NULL DEFAULT 0,
            response_raw_truncated_reason TEXT,
            response_content_encoding TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_request_attempts table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_conversation_bindings (
            prompt_cache_key TEXT PRIMARY KEY,
            binding_kind TEXT NOT NULL CHECK(binding_kind IN ('none', 'group', 'upstream_account')),
            group_name TEXT,
            upstream_account_id INTEGER,
            responses_first_byte_timeout_secs INTEGER,
            compact_first_byte_timeout_secs INTEGER,
            image_first_byte_timeout_secs INTEGER,
            responses_stream_timeout_secs INTEGER,
            compact_stream_timeout_secs INTEGER,
            allow_switch_upstream INTEGER,
            fast_mode_rewrite_mode TEXT,
            image_tool_rewrite_mode TEXT,
            codex_imagegen_rewrite_mode TEXT,
            available_models_json TEXT,
            forward_proxy_key TEXT,
            forward_proxy_keys_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            CHECK (
                (binding_kind = 'none' AND group_name IS NULL AND upstream_account_id IS NULL)
                OR
                (binding_kind = 'group' AND group_name IS NOT NULL AND upstream_account_id IS NULL)
                OR
                (binding_kind = 'upstream_account' AND group_name IS NULL AND upstream_account_id IS NOT NULL)
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_conversation_bindings table existence")?;
    migrate_prompt_cache_conversation_bindings_contract(pool)
        .await
        .context("failed to migrate prompt_cache_conversation_bindings contract")?;
    let binding_columns =
        load_sqlite_table_columns(pool, "prompt_cache_conversation_bindings").await?;
    if !binding_columns.contains("available_models_mode") {
        sqlx::query(
            "ALTER TABLE prompt_cache_conversation_bindings ADD COLUMN available_models_mode TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add available_models_mode to prompt_cache_conversation_bindings")?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_conversation_bindings_group
        ON prompt_cache_conversation_bindings (group_name)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_conversation_bindings_group")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_conversation_bindings_account
        ON prompt_cache_conversation_bindings (upstream_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_conversation_bindings_account")?;

    sqlx::query(&prompt_cache_conversation_operation_events_create_sql(
        "prompt_cache_conversation_operation_events",
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_conversation_operation_events table existence")?;

    let existing_prompt_cache_operation_event_columns =
        load_sqlite_table_columns(pool, "prompt_cache_conversation_operation_events").await?;
    if !existing_prompt_cache_operation_event_columns.contains("routing_context_json") {
        sqlx::query(
            "ALTER TABLE prompt_cache_conversation_operation_events ADD COLUMN routing_context_json TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add prompt-cache operation routing context column")?;
    }
    if !existing_prompt_cache_operation_event_columns.contains("routing_scope_json") {
        sqlx::query(
            "ALTER TABLE prompt_cache_conversation_operation_events ADD COLUMN routing_scope_json TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add prompt-cache operation routing scope column")?;
    }
    if !existing_prompt_cache_operation_event_columns.contains("sticky_transitions_json") {
        sqlx::query(
            "ALTER TABLE prompt_cache_conversation_operation_events ADD COLUMN sticky_transitions_json TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add prompt-cache operation sticky transitions column")?;
    }
    sqlx::query(
        r#"
        UPDATE prompt_cache_conversation_operation_events
        SET routing_scope_json = '{"kind":"all"}'
        WHERE routing_scope_json IS NULL
          AND EXISTS (
              SELECT 1 FROM json_each(prompt_cache_conversation_operation_events.info_types_json)
              WHERE json_each.value = 'routing'
          )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to migrate legacy prompt-cache routing event scopes")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_conversation_operation_events_key_occurred
        ON prompt_cache_conversation_operation_events (prompt_cache_key, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_prompt_cache_conversation_operation_events_key_occurred",
    )?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_encrypted_session_owners (
            prompt_cache_key TEXT PRIMARY KEY,
            owner_upstream_account_id INTEGER NOT NULL,
            first_locked_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_confirmed_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_encrypted_session_owners table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_encrypted_session_owners_account
        ON prompt_cache_encrypted_session_owners (owner_upstream_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_encrypted_session_owners_account")?;

    let existing_pool_attempt_columns =
        load_sqlite_table_columns(pool, "pool_upstream_request_attempts").await?;
    for (column, ty) in [
        ("attempt_public_id", "TEXT"),
        ("invocation_id", "INTEGER"),
        ("routing_source", "TEXT"),
        ("routing_selection_audit_json", "TEXT"),
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("upstream_base_url_host", "TEXT"),
        ("upstream_request_compression_algorithm", "TEXT"),
        ("upstream_request_compression_mode", "TEXT"),
        ("upstream_request_logical_body_bytes", "INTEGER"),
        ("upstream_request_transmitted_body_bytes", "INTEGER"),
        ("upstream_request_header_bytes_approx", "INTEGER"),
        ("upstream_response_body_bytes", "INTEGER"),
        ("upstream_response_header_bytes_approx", "INTEGER"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
        ("request_model", "TEXT"),
        ("upstream_request_model", "TEXT"),
        ("model_mapping_pattern", "TEXT"),
        ("request_summary_json", "TEXT"),
        ("response_summary_json", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("response_content_encoding", "TEXT"),
    ] {
        if !existing_pool_attempt_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add pool_upstream_request_attempts column {column}")
                })?;
        }
    }

    let pool_attempt_columns: HashSet<String> =
        sqlx::query("PRAGMA table_xinfo('pool_upstream_request_attempts')")
            .fetch_all(pool)
            .await
            .context("failed to inspect pool_upstream_request_attempts schema")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !pool_attempt_columns.contains("occurred_epoch_ms") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_request_attempts
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
        .context("failed to add pool_upstream_request_attempts.occurred_epoch_ms")?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_weight_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            min_weight REAL NOT NULL,
            max_weight REAL NOT NULL,
            avg_weight REAL NOT NULL,
            last_weight REAL NOT NULL,
            last_sample_epoch_us INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_weight_hourly table existence")?;

    let existing_forward_proxy_weight_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('forward_proxy_weight_hourly')")
            .fetch_all(pool)
            .await
            .context("failed to inspect forward_proxy_weight_hourly schema")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !existing_forward_proxy_weight_columns.contains("last_sample_epoch_us") {
        sqlx::query(
            r#"
            ALTER TABLE forward_proxy_weight_hourly
            ADD COLUMN last_sample_epoch_us INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add last_sample_epoch_us to forward_proxy_weight_hourly")?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_proxy_time
        ON forward_proxy_attempts (proxy_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_proxy_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_time_proxy
        ON forward_proxy_attempts (occurred_at, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_time_proxy")?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_public_id
        ON pool_upstream_request_attempts (attempt_public_id)
        WHERE attempt_public_id IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_public_id")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_invoke_attempt
        ON pool_upstream_request_attempts (invoke_id, attempt_index)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_invoke_attempt")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_account_occurred_at
        ON pool_upstream_request_attempts (upstream_account_id, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_account_occurred_at")?;

    let success_attempt_index_started_at = Instant::now();
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pool_attempts_account_model_success ON pool_upstream_request_attempts (upstream_account_id, request_model COLLATE NOCASE, id DESC, started_at) WHERE status = 'success'")
        .execute(pool).await
        .context("failed to ensure index idx_pool_attempts_account_model_success")?;
    info!(
        index = "idx_pool_attempts_account_model_success",
        elapsed_ms = success_attempt_index_started_at.elapsed().as_millis() as u64,
        "ensured successful pool-attempt route lookup index"
    );

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_transport_decode_recent
        ON pool_upstream_request_attempts (
            upstream_account_id,
            route_mode,
            endpoint,
            occurred_at DESC,
            id DESC,
            phase
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_transport_decode_recent")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_sticky_occurred_at
        ON pool_upstream_request_attempts (sticky_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_sticky_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_occurred_at
        ON pool_upstream_request_attempts (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_timeline_epoch
        ON pool_upstream_request_attempts (occurred_epoch_ms DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_timeline_epoch")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_occurred_at_proxy_binding
        ON pool_upstream_request_attempts (occurred_at, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_pool_upstream_request_attempts_occurred_at_proxy_binding",
    )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_group_proxy_occurred_at
        ON pool_upstream_request_attempts (
            group_name_snapshot,
            occurred_at,
            proxy_binding_key_snapshot
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_group_proxy_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_pending_early_phase_started
        ON pool_upstream_request_attempts (status, started_at, endpoint, invoke_id, occurred_at)
        WHERE finished_at IS NULL
          AND COALESCE(first_byte_latency_ms, 0) <= 0
          AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_pool_upstream_request_attempts_pending_early_phase_started",
    )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_weight_hourly_time_proxy
        ON forward_proxy_weight_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_weight_hourly_time_proxy")?;

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
        ("enabled", "INTEGER NOT NULL DEFAULT 1"),
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
    for (column, definition) in [
        ("inventory_recheck_cursor", "TEXT NOT NULL DEFAULT ''"),
        ("inventory_recheck_active", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !raw_metrics_columns.contains(column) {
            sqlx::query(&format!(
                "ALTER TABLE system_raw_payload_metrics ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await
            .with_context(|| format!("failed to add system raw payload {column}"))?;
        }
    }
    for (column, definition) in [
        ("circuit_state", "TEXT NOT NULL DEFAULT 'unknown'"),
        ("circuit_reason", "TEXT"),
        ("circuit_available_bytes", "INTEGER"),
        ("circuit_expired_backlog_count", "INTEGER"),
        ("circuit_backlog_non_growing", "INTEGER"),
        ("circuit_updated_at", "TEXT"),
        ("circuit_recovery_pending", "INTEGER NOT NULL DEFAULT 0"),
        ("raw_overflow_spool_bytes", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !raw_metrics_columns.contains(column) {
            sqlx::query(&format!(
                "ALTER TABLE system_raw_payload_metrics ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await
            .with_context(|| format!("failed to add system raw payload {column}"))?;
        }
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
    retire_openai_websocket_proxy(pool).await?;
    ensure_long_term_projection_account_trigger(pool).await?;

    Ok(())
}

pub(crate) async fn retire_openai_websocket_proxy(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin WebSocket proxy retirement migration")?;

    let settings_rows = sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET openai_proxy_websocket_enabled = 0,
            openai_proxy_upstream_websocket_default_enabled = 0,
            websocket_settings_migrated = 1,
            updated_at = datetime('now')
        WHERE id = 1
          AND (
              openai_proxy_websocket_enabled <> 0
              OR openai_proxy_upstream_websocket_default_enabled <> 0
              OR websocket_settings_migrated <> 1
          )
        "#,
    )
    .execute(tx.as_mut())
    .await
    .context("failed to retire persisted WebSocket proxy settings")?
    .rows_affected();

    let oauth_session_links = sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET tag_ids_json = (
            SELECT CASE
                WHEN COUNT(*) = 0 THEN NULL
                ELSE json_group_array(
                    CASE
                        WHEN json_each.type IN ('object', 'array') THEN json(json_each.value)
                        WHEN json_each.type IN ('true', 'false') THEN json(json_each.type)
                        ELSE json_each.value
                    END
                )
            END
            FROM json_each(pool_oauth_login_sessions.tag_ids_json)
            WHERE json_each.type != 'integer'
               OR json_each.value NOT IN (
                SELECT id FROM pool_tags
                WHERE system_key = 'unsupported_transport:websocket'
            )
        )
        WHERE json_valid(tag_ids_json)
          AND json_type(tag_ids_json) = 'array'
          AND EXISTS (
              SELECT 1
              FROM json_each(pool_oauth_login_sessions.tag_ids_json)
              WHERE json_each.type = 'integer'
                AND json_each.value IN (
                  SELECT id FROM pool_tags
                  WHERE system_key = 'unsupported_transport:websocket'
              )
          )
        "#,
    )
    .execute(tx.as_mut())
    .await
    .context("failed to remove WebSocket capability tag from OAuth sessions")?
    .rows_affected();

    let tag_links = sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_tags
        WHERE tag_id IN (
            SELECT id FROM pool_tags
            WHERE system_key = 'unsupported_transport:websocket'
        )
        "#,
    )
    .execute(tx.as_mut())
    .await
    .context("failed to remove WebSocket capability tag associations")?
    .rows_affected();

    let tags =
        sqlx::query("DELETE FROM pool_tags WHERE system_key = 'unsupported_transport:websocket'")
            .execute(tx.as_mut())
            .await
            .context("failed to remove WebSocket capability tag")?
            .rows_affected();

    record_schema_refresh_completion_in_transaction(
        &mut tx,
        RETIRE_OPENAI_WEBSOCKET_PROXY_MIGRATION_NAME,
    )
    .await?;
    tx.commit()
        .await
        .context("failed to commit WebSocket proxy retirement migration")?;

    info!(
        migration = RETIRE_OPENAI_WEBSOCKET_PROXY_MIGRATION_NAME,
        settings_rows,
        oauth_session_links,
        tag_links,
        tags,
        "retired downstream WebSocket proxy state"
    );
    Ok(())
}

pub(super) async fn legacy_raw_blob_link_seed_completed(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<bool> {
    let migration_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind("proxy_raw_payload_blob_link_migrations")
    .fetch_one(tx.as_mut())
    .await?
        != 0;
    if !migration_table_exists {
        return Ok(false);
    }

    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM proxy_raw_payload_blob_link_migrations WHERE migration_name = ?1)",
    )
    .bind(LEGACY_RAW_BLOB_LINK_SEED_MIGRATION_NAME)
    .fetch_one(tx.as_mut())
    .await?
        != 0)
}

pub(crate) async fn proxy_raw_blob_link_seed_completed(pool: &Pool<Sqlite>) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let completed = legacy_raw_blob_link_seed_completed(&mut tx).await?;
    tx.rollback().await?;
    Ok(completed)
}

async fn ensure_proxy_raw_payload_blob_link_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blobs (
            raw_path TEXT PRIMARY KEY,
            storage_codec TEXT NOT NULL DEFAULT 'identity',
            logical_size_bytes INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blobs")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blob_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            raw_path TEXT NOT NULL,
            owner_kind TEXT NOT NULL CHECK(owner_kind IN ('invocation', 'attempt')),
            owner_id INTEGER NOT NULL,
            raw_role TEXT NOT NULL CHECK(raw_role IN ('request', 'response')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(raw_path, owner_kind, owner_id, raw_role),
            FOREIGN KEY(raw_path) REFERENCES proxy_raw_payload_blobs(raw_path) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob links")?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_proxy_raw_payload_blob_links_path ON proxy_raw_payload_blob_links (raw_path)",
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob link path index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blob_link_migrations (
            migration_name TEXT PRIMARY KEY,
            completed_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob link migrations")?;

    let mut raw_blob_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin proxy raw blob trigger refresh")?;
    for trigger in [
        "proxy_raw_blob_link_invocation_insert",
        "proxy_raw_blob_link_invocation_update",
        "proxy_raw_blob_link_invocation_delete",
        "proxy_raw_blob_link_attempt_insert",
        "proxy_raw_blob_link_attempt_update",
        "proxy_raw_blob_link_attempt_delete",
        "proxy_raw_blob_prune_unlinked",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(raw_blob_trigger_tx.as_mut())
            .await
            .with_context(|| format!("failed to replace proxy raw blob trigger {trigger}"))?;
    }

    let link_invocation = r#"
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.request_raw_path, COALESCE(NULLIF(TRIM(NEW.request_raw_codec), ''), 'identity'), COALESCE(NEW.request_raw_size, 0)
        WHERE NEW.request_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.request_raw_path, 'invocation', NEW.id, 'request'
        WHERE NEW.request_raw_path IS NOT NULL;
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.response_raw_path, COALESCE(NULLIF(TRIM(NEW.response_raw_codec), ''), 'identity'), COALESCE(NEW.response_raw_size, 0)
        WHERE NEW.response_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.response_raw_path, 'invocation', NEW.id, 'response'
        WHERE NEW.response_raw_path IS NOT NULL;
    "#;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_insert AFTER INSERT ON codex_invocations BEGIN {link_invocation} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob insert trigger")?;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_update AFTER UPDATE OF request_raw_path, request_raw_codec, request_raw_size, response_raw_path, response_raw_codec, response_raw_size ON codex_invocations BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'invocation' AND owner_id = NEW.id; {link_invocation} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob update trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_delete AFTER DELETE ON codex_invocations BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'invocation' AND owner_id = OLD.id; END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob delete trigger")?;

    let link_attempt = r#"
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.response_raw_path, COALESCE(NULLIF(TRIM(NEW.response_raw_codec), ''), 'identity'), COALESCE(NEW.response_raw_size, 0)
        WHERE NEW.response_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.response_raw_path, 'attempt', NEW.id, 'response'
        WHERE NEW.response_raw_path IS NOT NULL;
    "#;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_insert AFTER INSERT ON pool_upstream_request_attempts BEGIN {link_attempt} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob insert trigger")?;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_update AFTER UPDATE OF response_raw_path, response_raw_codec, response_raw_size ON pool_upstream_request_attempts BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'attempt' AND owner_id = NEW.id; {link_attempt} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob update trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_delete AFTER DELETE ON pool_upstream_request_attempts BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'attempt' AND owner_id = OLD.id; END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob delete trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_prune_unlinked AFTER DELETE ON proxy_raw_payload_blob_links BEGIN DELETE FROM proxy_raw_payload_blobs WHERE raw_path = OLD.raw_path AND NOT EXISTS (SELECT 1 FROM proxy_raw_payload_blob_links WHERE raw_path = OLD.raw_path); END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create proxy raw blob pruning trigger")?;
    raw_blob_trigger_tx
        .commit()
        .await
        .context("failed to commit proxy raw blob trigger refresh")?;

    seed_legacy_proxy_raw_payload_blob_links(pool).await?;

    Ok(())
}

async fn seed_legacy_proxy_raw_payload_blob_links(pool: &Pool<Sqlite>) -> Result<()> {
    const MIGRATION_NAME: &str = "seed_existing_raw_blob_links_v1";
    let already_seeded = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM proxy_raw_payload_blob_link_migrations WHERE migration_name = ?1)",
    )
    .bind(MIGRATION_NAME)
    .fetch_one(pool)
    .await?
        != 0;
    if already_seeded {
        return Ok(());
    }

    // Existing rows predate the link triggers. Seed both owner types atomically once so an
    // upgraded database retains paired response blobs and inventories attempt-only captures.
    let mut tx = pool.begin().await?;
    for (path_column, codec_column, size_column) in [
        ("request_raw_path", "request_raw_codec", "request_raw_size"),
        (
            "response_raw_path",
            "response_raw_codec",
            "response_raw_size",
        ),
    ] {
        let query = format!(
            "INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes) SELECT {path_column}, COALESCE(NULLIF(TRIM({codec_column}), ''), 'identity'), COALESCE({size_column}, 0) FROM codex_invocations WHERE {path_column} IS NOT NULL ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now')"
        );
        sqlx::query(&query).execute(tx.as_mut()).await?;
    }
    sqlx::query(
        "INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes) SELECT response_raw_path, COALESCE(NULLIF(TRIM(response_raw_codec), ''), 'identity'), COALESCE(response_raw_size, 0) FROM pool_upstream_request_attempts WHERE response_raw_path IS NOT NULL ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now')",
    )
    .execute(tx.as_mut())
    .await?;
    for (path_column, raw_role) in [
        ("request_raw_path", "request"),
        ("response_raw_path", "response"),
    ] {
        let query = format!(
            "INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role) SELECT {path_column}, 'invocation', id, '{raw_role}' FROM codex_invocations WHERE {path_column} IS NOT NULL"
        );
        sqlx::query(&query).execute(tx.as_mut()).await?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role) SELECT response_raw_path, 'attempt', id, 'response' FROM pool_upstream_request_attempts WHERE response_raw_path IS NOT NULL",
    )
    .execute(tx.as_mut())
    .await?;
    sqlx::query("INSERT INTO proxy_raw_payload_blob_link_migrations (migration_name) VALUES (?1)")
        .bind(MIGRATION_NAME)
        .execute(tx.as_mut())
        .await?;
    tx.commit().await?;
    Ok(())
}
