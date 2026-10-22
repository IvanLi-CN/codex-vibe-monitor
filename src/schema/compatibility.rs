use super::*;

pub(crate) static ENSURE_SCHEMA_LOCKS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) const INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS: i64 = 300;
pub(crate) const SHANGHAI_NOW_SQL: &str = "datetime('now', '+8 hours')";
pub(crate) const INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET: &str =
    "invocation_rollup_hourly_token_components_v1";
pub(crate) const INVOCATION_RAW_CODEC_MIGRATION_NAME: &str = "backfill_raw_codecs_v1";
pub(crate) const LEGACY_RAW_BLOB_LINK_SEED_MIGRATION_NAME: &str = "seed_existing_raw_blob_links_v1";
pub(crate) const SCHEMA_REFRESH_MIGRATIONS_TABLE: &str = "schema_refresh_migrations";
pub(crate) const RETIRE_OPENAI_WEBSOCKET_PROXY_MIGRATION_NAME: &str =
    "retire_openai_websocket_proxy_v1";
pub(crate) const PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME: &str =
    "prompt_cache_expression_indexes_v1";
pub(crate) const INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME: &str =
    "invoke_id_filter_expression_index_v1";
pub(crate) const TIMESERIES_MINUTE_PROJECTION_STARTUP_RECOVERY_BASELINE_MIGRATION_NAME: &str =
    "timeseries_minute_projection_startup_recovery_baseline_v1";
pub(crate) const INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME: &str =
    "invocation_live_projection_v1";
pub(crate) const PROMPT_CACHE_WORKING_SET_TRIGGER_REFRESH_MIGRATION_NAME: &str =
    "prompt_cache_working_set_relevant_updates_v1";
pub(crate) const TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2_recovery (
        consumer TEXT PRIMARY KEY,
        generation INTEGER NOT NULL DEFAULT 0,
        invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
"#;

pub(crate) fn ensure_schema_lock_key(pool: &Pool<Sqlite>) -> String {
    let connect_options = pool.connect_options();
    let filename = connect_options.get_filename();

    if filename == std::path::Path::new(":memory:") {
        format!(
            "sqlite:memory:{:p}",
            std::sync::Arc::as_ptr(&connect_options)
        )
    } else {
        format!("sqlite:{}", filename.to_string_lossy())
    }
}

pub(crate) fn ensure_schema_lock(pool: &Pool<Sqlite>) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let key = ensure_schema_lock_key(pool);
    let mut registry = ENSURE_SCHEMA_LOCKS
        .lock()
        .expect("schema lock registry should remain available");

    if let Some(lock) = registry.get(&key).and_then(std::sync::Weak::upgrade) {
        return lock;
    }

    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    registry.insert(key, std::sync::Arc::downgrade(&lock));
    lock
}

pub(crate) async fn ensure_schema_refresh_migrations_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {SCHEMA_REFRESH_MIGRATIONS_TABLE} (\
            migration_name TEXT PRIMARY KEY,\
            completed_at TEXT NOT NULL DEFAULT (datetime('now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure schema refresh migration marker table")?;
    Ok(())
}

pub(crate) async fn schema_refresh_completed(
    pool: &Pool<Sqlite>,
    migration_name: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM {SCHEMA_REFRESH_MIGRATIONS_TABLE} WHERE migration_name = ?1)"
    ))
    .bind(migration_name)
    .fetch_one(pool)
    .await?
        != 0)
}

pub(crate) async fn schema_refresh_completed_in_transaction(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    migration_name: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM {SCHEMA_REFRESH_MIGRATIONS_TABLE} WHERE migration_name = ?1)"
    ))
    .bind(migration_name)
    .fetch_one(tx.as_mut())
    .await?
        != 0)
}

pub(crate) async fn record_schema_refresh_completion(
    pool: &Pool<Sqlite>,
    migration_name: &str,
) -> Result<()> {
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO {SCHEMA_REFRESH_MIGRATIONS_TABLE} (migration_name) VALUES (?1)"
    ))
    .bind(migration_name)
    .execute(pool)
    .await
    .with_context(|| format!("failed to record schema refresh completion for {migration_name}"))?;
    Ok(())
}

pub(crate) async fn record_schema_refresh_completion_in_transaction(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    migration_name: &str,
) -> Result<()> {
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO {SCHEMA_REFRESH_MIGRATIONS_TABLE} (migration_name) VALUES (?1)"
    ))
    .bind(migration_name)
    .execute(tx.as_mut())
    .await
    .with_context(|| format!("failed to record schema refresh completion for {migration_name}"))?;
    Ok(())
}
pub(crate) async fn sqlite_table_exists(pool: &Pool<Sqlite>, table_name: &str) -> Result<bool> {
    sqlite_schema_object_exists(pool, "table", table_name).await
}

pub(crate) async fn sqlite_schema_object_exists(
    pool: &Pool<Sqlite>,
    object_type: &str,
    object_name: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
    )
    .bind(object_type)
    .bind(object_name)
    .fetch_one(pool)
    .await?
        != 0)
}

pub(crate) async fn invocation_live_projection_objects_exist(
    pool: &Pool<Sqlite>,
    invocation_in_progress_live_exists: bool,
    prompt_cache_working_set_live_exists: bool,
) -> Result<bool> {
    if !invocation_in_progress_live_exists || !prompt_cache_working_set_live_exists {
        return Ok(false);
    }

    for trigger_name in [
        "trg_codex_invocations_live_insert",
        "trg_codex_invocations_live_update",
        "trg_codex_invocations_live_delete",
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        if !sqlite_schema_object_exists(pool, "trigger", trigger_name).await? {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(crate) async fn ensure_nullable_real_column(
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

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} REAL");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_column_with_definition(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
    definition: &str,
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
    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}
