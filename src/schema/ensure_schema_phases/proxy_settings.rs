use super::*;

pub(super) async fn ensure_schema_proxy_settings(pool: &Pool<Sqlite>) -> Result<()> {
    let (had_owner_routing, had_owner_routing_init, had_singleton) =
        inspect_proxy_model_settings(pool).await?;
    ensure_proxy_model_settings_base_columns(pool).await?;
    ensure_proxy_model_settings_owner_columns(
        pool,
        had_owner_routing,
        had_owner_routing_init,
        had_singleton,
    )
    .await?;
    ensure_proxy_model_settings_websocket_column(pool).await?;
    seed_proxy_model_settings(pool).await?;
    Ok(())
}

async fn inspect_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<(bool, bool, bool)> {
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
    Ok((
        proxy_model_settings_had_owner_routing_column,
        proxy_model_settings_had_owner_routing_init_column,
        proxy_model_settings_had_singleton_row,
    ))
}

async fn ensure_proxy_model_settings_base_columns(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_proxy_model_settings_owner_columns(
    pool: &Pool<Sqlite>,
    had_owner_routing: bool,
    had_owner_routing_init: bool,
    had_singleton: bool,
) -> Result<()> {
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
    if had_owner_routing && !had_owner_routing_init && had_singleton {
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
    Ok(())
}

async fn ensure_proxy_model_settings_websocket_column(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn seed_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<()> {
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
    .bind(DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED as i64)
    .bind(DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED as i64)
    .bind(1_i64)
    .bind(1_i64)
    .bind(DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED as i64)
    .bind(0_i64)
    .bind(0_i64)
    .bind(default_enabled_models_json)
    .execute(pool)
    .await
    .context("failed to ensure default proxy_model_settings row")?;
    Ok(())
}
