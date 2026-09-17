async fn ensure_pool_routing_settings_table(pool: &Pool<Sqlite>) -> Result<()> {
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
            image_first_byte_timeout_secs INTEGER,
            responses_stream_timeout_secs INTEGER,
            compact_stream_timeout_secs INTEGER,
            request_compression_algorithm TEXT,
            request_compression_level_preset TEXT,
            codex_imagegen_rewrite_mode TEXT,
            available_models_json TEXT NOT NULL DEFAULT '[]',
            available_models_mode TEXT NOT NULL DEFAULT 'denylist',
            default_first_byte_timeout_secs INTEGER,
            upstream_handshake_timeout_secs INTEGER,
            request_read_timeout_secs INTEGER,
            cache_hit_protection_enabled INTEGER NOT NULL DEFAULT 0,
            cache_hit_low_rate_threshold_percent INTEGER NOT NULL DEFAULT 10,
            cache_hit_overflow_mode TEXT NOT NULL DEFAULT 'queue',
            priority_handoff_admission_enabled INTEGER NOT NULL DEFAULT 1,
            capability_axis_split_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_routing_settings table existence")?;
    Ok(())
}

async fn ensure_pool_routing_settings_columns(pool: &Pool<Sqlite>) -> Result<bool> {
    let existing_routing_settings_columns =
        load_sqlite_table_columns(pool, "pool_routing_settings").await?;
    let routing_models_mode_was_missing =
        !existing_routing_settings_columns.contains("available_models_mode");
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
        "image_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.image_first_byte_timeout_secs")?;
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
    ensure_nullable_text_column(
        pool,
        "pool_routing_settings",
        "request_compression_algorithm",
    )
    .await
    .context("failed to ensure pool_routing_settings.request_compression_algorithm")?;
    ensure_nullable_text_column(
        pool,
        "pool_routing_settings",
        "request_compression_level_preset",
    )
    .await
    .context("failed to ensure pool_routing_settings.request_compression_level_preset")?;
    ensure_nullable_text_column(pool, "pool_routing_settings", "codex_imagegen_rewrite_mode")
        .await
        .context("failed to ensure pool_routing_settings.codex_imagegen_rewrite_mode")?;
    ensure_text_column_with_default(
        pool,
        "pool_routing_settings",
        "available_models_json",
        "'[]'",
    )
    .await
    .context("failed to ensure pool_routing_settings.available_models_json")?;
    ensure_text_column_with_default(
        pool,
        "pool_routing_settings",
        "available_models_mode",
        "'denylist'",
    )
    .await
    .context("failed to ensure pool_routing_settings.available_models_mode")?;
    Ok(routing_models_mode_was_missing)
}

async fn finish_pool_routing_settings(
    pool: &Pool<Sqlite>,
    routing_models_mode_was_missing: bool,
) -> Result<()> {
    if routing_models_mode_was_missing {
        sqlx::query(
            r#"
            UPDATE pool_routing_settings
            SET available_models_mode = CASE
                WHEN trim(coalesce(available_models_json, '')) IN ('', '[]') THEN 'denylist'
                ELSE 'allowlist'
            END
            "#,
        )
        .execute(pool)
        .await
        .context("failed to backfill legacy pool_routing_settings.available_models_mode")?;
    }
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
    ensure_integer_column_with_default(
        pool,
        "pool_routing_settings",
        "cache_hit_protection_enabled",
        "0",
    )
    .await
    .context("failed to ensure pool_routing_settings.cache_hit_protection_enabled")?;
    ensure_integer_column_with_default(
        pool,
        "pool_routing_settings",
        "cache_hit_low_rate_threshold_percent",
        "10",
    )
    .await
    .context("failed to ensure pool_routing_settings.cache_hit_low_rate_threshold_percent")?;
    ensure_text_column_with_default(
        pool,
        "pool_routing_settings",
        "cache_hit_overflow_mode",
        "'queue'",
    )
    .await
    .context("failed to ensure pool_routing_settings.cache_hit_overflow_mode")?;
    ensure_integer_column_with_default(
        pool,
        "pool_routing_settings",
        "priority_handoff_admission_enabled",
        "1",
    )
    .await
    .context("failed to ensure pool_routing_settings.priority_handoff_admission_enabled")?;
    ensure_integer_column_with_default(
        pool,
        "pool_routing_settings",
        "capability_axis_split_migrated",
        "0",
    )
    .await
    .context("failed to ensure pool_routing_settings.capability_axis_split_migrated")?;

    ensure_pool_routing_settings_default_row_and_migrations(pool).await?;

    Ok(())
}

async fn ensure_pool_routing_settings_default_row_and_migrations(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_routing_settings (
            id,
            encrypted_api_key,
            masked_api_key,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            image_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
        ) VALUES (?1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to ensure default pool_routing_settings row")?;
    ensure_upstream_account_capability_axis_split_migrated(pool).await?;
    repair_responses_lite_image_tool_capability_observations(pool).await?;
    ensure_api_key_transit_proxy_bindings_migrated(pool).await?;
    Ok(())
}
