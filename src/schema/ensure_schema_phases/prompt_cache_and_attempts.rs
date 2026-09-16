async fn ensure_schema_prompt_cache_and_attempts(pool: &Pool<Sqlite>) -> Result<()> {
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
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET cache_read_per_1m = cache_input_per_1m
        WHERE cache_read_per_1m IS NULL
          AND cache_input_per_1m IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill pricing_settings_models.cache_read_per_1m")?;

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

    Ok(())
}
