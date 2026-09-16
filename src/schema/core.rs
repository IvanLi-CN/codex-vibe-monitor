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
pub(crate) const PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME: &str =
    "prompt_cache_expression_indexes_v1";
pub(crate) const INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME: &str =
    "invocation_live_projection_v1";
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

pub(crate) async fn schema_sqlite_table_exists(
    pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<bool> {
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

pub(crate) fn invocation_in_progress_live_prompt_cache_key_expr(subject: &str) -> String {
    format!(
        "CASE WHEN json_valid({subject}.payload) THEN TRIM(CAST(json_extract({subject}.payload, '$.promptCacheKey') AS TEXT)) END"
    )
}

pub(crate) fn invocation_in_progress_live_upstream_account_id_expr(subject: &str) -> String {
    format!(
        "CASE WHEN json_valid({subject}.payload) THEN CAST(json_extract({subject}.payload, '$.upstreamAccountId') AS INTEGER) END"
    )
}

pub(crate) fn invocation_in_progress_live_refresh_set_clause() -> String {
    let display_status_sql = crate::api::invocation_display_status_sql();
    format!(
        r#"
        is_retry_after_failure_all = COALESCE((
            SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
            FROM (
                SELECT LOWER(TRIM({display_status_sql})) AS display_status
                FROM codex_invocations
                WHERE {prompt_cache_key_sql} = invocation_in_progress_live.prompt_cache_key
                  AND id < invocation_in_progress_live.invocation_id
                  AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                ORDER BY id DESC
                LIMIT 1
            ) AS previous_terminal
        ), 0),
        is_retry_after_failure_proxy_only = COALESCE((
            SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
            FROM (
                SELECT LOWER(TRIM({display_status_sql})) AS display_status
                FROM codex_invocations
                WHERE {prompt_cache_key_sql} = invocation_in_progress_live.prompt_cache_key
                  AND source = '{source_proxy}'
                  AND id < invocation_in_progress_live.invocation_id
                  AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                ORDER BY id DESC
                LIMIT 1
            ) AS previous_terminal
        ), 0),
        is_retry_after_failure_account_all = CASE
            WHEN invocation_in_progress_live.upstream_account_id IS NULL THEN 0
            ELSE COALESCE((
                SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
                FROM (
                    SELECT LOWER(TRIM({display_status_sql})) AS display_status
                    FROM codex_invocations
                    WHERE {prompt_cache_key_sql} = invocation_in_progress_live.prompt_cache_key
                      AND {upstream_account_id_sql} = invocation_in_progress_live.upstream_account_id
                      AND id < invocation_in_progress_live.invocation_id
                      AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                    ORDER BY id DESC
                    LIMIT 1
                ) AS previous_terminal
            ), 0)
        END,
        is_retry_after_failure_account_proxy_only = CASE
            WHEN invocation_in_progress_live.upstream_account_id IS NULL THEN 0
            ELSE COALESCE((
                SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
                FROM (
                    SELECT LOWER(TRIM({display_status_sql})) AS display_status
                    FROM codex_invocations
                    WHERE {prompt_cache_key_sql} = invocation_in_progress_live.prompt_cache_key
                      AND {upstream_account_id_sql} = invocation_in_progress_live.upstream_account_id
                      AND source = '{source_proxy}'
                      AND id < invocation_in_progress_live.invocation_id
                      AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                    ORDER BY id DESC
                    LIMIT 1
                ) AS previous_terminal
            ), 0)
        END,
        updated_at = datetime('now')
        "#,
        display_status_sql = display_status_sql,
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        upstream_account_id_sql = INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL,
        source_proxy = SOURCE_PROXY,
    )
}

pub(crate) fn invocation_in_progress_live_upsert_sql(subject: &str) -> String {
    let display_status_sql = crate::api::invocation_display_status_sql();
    let prompt_cache_key_expr = invocation_in_progress_live_prompt_cache_key_expr(subject);
    let upstream_account_id_expr = invocation_in_progress_live_upstream_account_id_expr(subject);
    format!(
        r#"
        INSERT INTO invocation_in_progress_live (
            invocation_id,
            source,
            upstream_account_id,
            prompt_cache_key,
            is_retry_after_failure_all,
            is_retry_after_failure_proxy_only,
            is_retry_after_failure_account_all,
            is_retry_after_failure_account_proxy_only,
            upstream_ttfb_ms,
            updated_at
        )
        SELECT
            id,
            source,
            {upstream_account_id_expr},
            {prompt_cache_key_expr},
            0,
            0,
            0,
            0,
            t_upstream_ttfb_ms,
            datetime('now')
        FROM codex_invocations
        WHERE id = {subject}.id
          AND LOWER(TRIM({display_status_sql})) IN ('running', 'pending')
        ON CONFLICT(invocation_id) DO UPDATE SET
            source = excluded.source,
            upstream_account_id = excluded.upstream_account_id,
            prompt_cache_key = excluded.prompt_cache_key,
            is_retry_after_failure_all = excluded.is_retry_after_failure_all,
            is_retry_after_failure_proxy_only = excluded.is_retry_after_failure_proxy_only,
            is_retry_after_failure_account_all = excluded.is_retry_after_failure_account_all,
            is_retry_after_failure_account_proxy_only = excluded.is_retry_after_failure_account_proxy_only,
            upstream_ttfb_ms = excluded.upstream_ttfb_ms,
            updated_at = excluded.updated_at
        "#,
        upstream_account_id_expr = upstream_account_id_expr,
        prompt_cache_key_expr = prompt_cache_key_expr,
        subject = subject,
        display_status_sql = display_status_sql,
    )
}

pub(crate) fn invocation_in_progress_live_refresh_sql_for_key(key_expr: &str) -> String {
    let refresh_set_clause = invocation_in_progress_live_refresh_set_clause();
    format!(
        r#"
        UPDATE invocation_in_progress_live
        SET {refresh_set_clause}
        WHERE prompt_cache_key = {key_expr}
          AND prompt_cache_key IS NOT NULL
          AND prompt_cache_key <> ''
        "#
    )
}

pub(crate) async fn rebuild_invocation_in_progress_live_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query("DELETE FROM invocation_in_progress_live")
        .execute(pool)
        .await
        .context("failed to clear invocation_in_progress_live before rebuild")?;

    let display_status_sql = crate::api::invocation_display_status_sql();
    let rebuild_insert_sql = format!(
        r#"
        INSERT INTO invocation_in_progress_live (
            invocation_id,
            source,
            upstream_account_id,
            prompt_cache_key,
            is_retry_after_failure_all,
            is_retry_after_failure_proxy_only,
            is_retry_after_failure_account_all,
            is_retry_after_failure_account_proxy_only,
            upstream_ttfb_ms,
            updated_at
        )
        SELECT
            id,
            source,
            {upstream_account_id_sql},
            {prompt_cache_key_sql},
            0,
            0,
            0,
            0,
            t_upstream_ttfb_ms,
            datetime('now')
        FROM codex_invocations
        WHERE LOWER(TRIM({display_status_sql})) IN ('running', 'pending')
        "#,
        upstream_account_id_sql = INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL,
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        display_status_sql = display_status_sql,
    );
    sqlx::query(&rebuild_insert_sql)
        .execute(pool)
        .await
        .context("failed to rebuild invocation_in_progress_live rows")?;

    let refresh_sql = format!(
        "UPDATE invocation_in_progress_live SET {}",
        invocation_in_progress_live_refresh_set_clause()
    );
    sqlx::query(&refresh_sql)
        .execute(pool)
        .await
        .context("failed to refresh invocation_in_progress_live retry flags during rebuild")?;

    Ok(())
}

pub(crate) async fn rebuild_invocation_in_progress_live_triggers(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin immediate invocation_in_progress_live trigger rebuild")?;

    // The update trigger writes this marker. Keep its table creation in the same transaction so
    // rebuilding the trigger cannot leave direct terminal writes pointing at a missing table.
    sqlx::query(TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL)
        .execute(tx.as_mut())
        .await
        .context("failed to ensure timeseries_minute_projection_v2 recovery table before trigger rebuild")?;

    for trigger_name in [
        "trg_codex_invocations_live_insert",
        "trg_codex_invocations_live_update",
        "trg_codex_invocations_live_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(tx.as_mut())
            .await
            .with_context(|| format!("failed to drop stale trigger {trigger_name}"))?;
    }

    create_invocation_insert_trigger(tx.as_mut()).await?;
    create_invocation_update_trigger(tx.as_mut()).await?;
    create_invocation_delete_trigger(tx.as_mut()).await?;
    tx.commit()
        .await
        .context("failed to commit invocation_in_progress_live trigger rebuild")?;

    Ok(())
}

async fn create_invocation_insert_trigger(tx: &mut sqlx::SqliteConnection) -> Result<()> {
    let insert_refresh_sql = invocation_in_progress_live_refresh_sql_for_key(
        &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
    );
    let insert_trigger_sql = format!(
        r#"
        CREATE TRIGGER trg_codex_invocations_live_insert
        AFTER INSERT ON codex_invocations
        BEGIN
            {upsert_sql};
            {refresh_sql};
        END
        "#,
        upsert_sql = invocation_in_progress_live_upsert_sql("NEW"),
        refresh_sql = insert_refresh_sql,
    );
    sqlx::query(&insert_trigger_sql)
        .execute(tx)
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_insert")?;
    Ok(())
}

async fn create_invocation_update_trigger(tx: &mut sqlx::SqliteConnection) -> Result<()> {
    let update_old_refresh_sql = invocation_in_progress_live_refresh_sql_for_key(
        &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
    );
    let update_new_refresh_sql = invocation_in_progress_live_refresh_sql_for_key(
        &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
    );
    // Proxy terminal writes are registered with the in-process projection hub before
    // persistence. A direct source correction can also turn a proxy in-flight row terminal, so
    // publish a constant-size durable recovery marker whenever either endpoint is non-proxy.
    let non_proxy_terminal_projection_invalidation_sql = r#"
        INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending, updated_at)
        SELECT 'timeseries_minute_v2', 1, 1, datetime('now')
        WHERE (COALESCE(OLD.source, '') <> 'proxy' OR COALESCE(NEW.source, '') <> 'proxy')
          AND LOWER(TRIM(COALESCE(OLD.status, ''))) IN ('running', 'pending')
          AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
        ON CONFLICT(consumer) DO UPDATE SET
            generation = timeseries_minute_projection_v2_recovery.generation + 1,
            invalidation_pending = 1,
            updated_at = excluded.updated_at
    "#;
    let update_trigger_sql = format!(
        r#"
        CREATE TRIGGER trg_codex_invocations_live_update
        AFTER UPDATE ON codex_invocations
        BEGIN
            DELETE FROM invocation_in_progress_live
            WHERE invocation_id = OLD.id;
            {upsert_sql};
            {refresh_old_sql};
            {refresh_new_sql};
            {non_proxy_terminal_projection_invalidation_sql};
        END
        "#,
        upsert_sql = invocation_in_progress_live_upsert_sql("NEW"),
        refresh_old_sql = update_old_refresh_sql,
        refresh_new_sql = update_new_refresh_sql,
        non_proxy_terminal_projection_invalidation_sql =
            non_proxy_terminal_projection_invalidation_sql,
    );
    sqlx::query(&update_trigger_sql)
        .execute(tx)
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_update")?;
    Ok(())
}

async fn create_invocation_delete_trigger(tx: &mut sqlx::SqliteConnection) -> Result<()> {
    let delete_refresh_sql = invocation_in_progress_live_refresh_sql_for_key(
        &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
    );
    let delete_trigger_sql = format!(
        r#"
        CREATE TRIGGER trg_codex_invocations_live_delete
        AFTER DELETE ON codex_invocations
        BEGIN
            DELETE FROM invocation_in_progress_live
            WHERE invocation_id = OLD.id;
            {refresh_sql};
        END
        "#,
        refresh_sql = delete_refresh_sql,
    );
    sqlx::query(&delete_trigger_sql)
        .execute(tx)
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_delete")?;
    Ok(())
}

pub(crate) async fn rebuild_prompt_cache_working_set_live_triggers(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let prompt_cache_insert_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_insert
        AFTER INSERT ON codex_invocations
        BEGIN
            {refresh_sql};
        END
        "#,
        refresh_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
        ),
    );
    let prompt_cache_update_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_update
        AFTER UPDATE ON codex_invocations
        BEGIN
            {refresh_old_sql};
            {refresh_new_sql};
        END
        "#,
        refresh_old_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
        ),
        refresh_new_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
        ),
    );
    let prompt_cache_delete_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_delete
        AFTER DELETE ON codex_invocations
        BEGIN
            {refresh_sql};
        END
        "#,
        refresh_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
        ),
    );
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache working set trigger refresh")?;
    for trigger_name in [
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(tx.as_mut())
            .await
            .with_context(|| format!("failed to drop stale trigger {trigger_name}"))?;
    }
    sqlx::query(&prompt_cache_insert_trigger_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_insert",
        )?;
    sqlx::query(&prompt_cache_update_trigger_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_update",
        )?;
    sqlx::query(&prompt_cache_delete_trigger_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_delete",
        )?;
    tx.commit()
        .await
        .context("failed to commit prompt cache working set trigger refresh")?;

    Ok(())
}

const PROMPT_CACHE_WORKING_SET_LIVE_REFRESH_SQL_TEMPLATE: &str = r#"
        INSERT INTO prompt_cache_working_set_live (
            prompt_cache_key,
            source_scope_all,
            source_scope_proxy_only,
            created_at,
            last_activity_at,
            last_terminal_at,
            last_in_flight_at,
            sort_anchor_at,
            request_count,
            total_tokens,
            total_cost,
            proxy_created_at,
            proxy_last_activity_at,
            proxy_last_terminal_at,
            proxy_last_in_flight_at,
            proxy_sort_anchor_at,
            proxy_request_count,
            proxy_total_tokens,
            proxy_total_cost,
            updated_at
        )
        SELECT
            candidate.prompt_cache_key,
            1,
            candidate.source_scope_proxy_only,
            candidate.created_at,
            candidate.last_activity_at,
            candidate.last_terminal_at,
            candidate.last_in_flight_at,
            candidate.sort_anchor_at,
            candidate.request_count,
            candidate.total_tokens,
            candidate.total_cost,
            candidate.proxy_created_at,
            candidate.proxy_last_activity_at,
            candidate.proxy_last_terminal_at,
            candidate.proxy_last_in_flight_at,
            candidate.proxy_sort_anchor_at,
            candidate.proxy_request_count,
            candidate.proxy_total_tokens,
            candidate.proxy_total_cost,
            datetime('now')
        FROM (
            SELECT
                keyed.prompt_cache_key AS prompt_cache_key,
                CASE WHEN MAX(CASE WHEN keyed.source = '{source_proxy}' THEN 1 ELSE 0 END) = 1 THEN 1 ELSE 0 END AS source_scope_proxy_only,
                MIN(keyed.occurred_at) AS created_at,
                MAX(keyed.occurred_at) AS last_activity_at,
                MAX(CASE WHEN keyed.is_in_flight = 0 THEN keyed.occurred_at END) AS last_terminal_at,
                MAX(CASE WHEN keyed.is_in_flight = 1 THEN keyed.occurred_at END) AS last_in_flight_at,
                MAX(
                    CASE
                        WHEN keyed.is_in_flight = 1 THEN keyed.occurred_at
                        WHEN keyed.occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds') THEN keyed.occurred_at
                        ELSE NULL
                    END
                ) AS sort_anchor_at,
                COUNT(*) AS request_count,
                COALESCE(SUM(COALESCE(keyed.total_tokens, 0)), 0) AS total_tokens,
                COALESCE(SUM(COALESCE(keyed.cost, 0.0)), 0.0) AS total_cost,
                MIN(CASE WHEN keyed.source = '{source_proxy}' THEN keyed.occurred_at END) AS proxy_created_at,
                MAX(CASE WHEN keyed.source = '{source_proxy}' THEN keyed.occurred_at END) AS proxy_last_activity_at,
                MAX(CASE WHEN keyed.source = '{source_proxy}' AND keyed.is_in_flight = 0 THEN keyed.occurred_at END) AS proxy_last_terminal_at,
                MAX(CASE WHEN keyed.source = '{source_proxy}' AND keyed.is_in_flight = 1 THEN keyed.occurred_at END) AS proxy_last_in_flight_at,
                MAX(
                    CASE
                        WHEN keyed.source = '{source_proxy}' AND keyed.is_in_flight = 1 THEN keyed.occurred_at
                        WHEN keyed.source = '{source_proxy}' AND keyed.occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds') THEN keyed.occurred_at
                        ELSE NULL
                    END
                ) AS proxy_sort_anchor_at,
                COALESCE(SUM(CASE WHEN keyed.source = '{source_proxy}' THEN 1 ELSE 0 END), 0) AS proxy_request_count,
                COALESCE(SUM(CASE WHEN keyed.source = '{source_proxy}' THEN COALESCE(keyed.total_tokens, 0) ELSE 0 END), 0) AS proxy_total_tokens,
                COALESCE(SUM(CASE WHEN keyed.source = '{source_proxy}' THEN COALESCE(keyed.cost, 0.0) ELSE 0.0 END), 0.0) AS proxy_total_cost
            FROM (
                SELECT
                    {prompt_cache_key_sql} AS prompt_cache_key,
                    source,
                    occurred_at,
                    total_tokens,
                    cost,
                    CASE
                        WHEN LOWER(TRIM({display_status_sql})) IN ('running', 'pending') THEN 1
                        ELSE 0
                    END AS is_in_flight
                FROM codex_invocations
                WHERE {prompt_cache_key_sql} = {key_expr}
                  AND {prompt_cache_key_sql} IS NOT NULL
                  AND {prompt_cache_key_sql} <> ''
                  AND (
                        LOWER(TRIM({display_status_sql})) IN ('running', 'pending')
                        OR occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds')
                  )
            ) AS keyed
            GROUP BY keyed.prompt_cache_key
        ) AS candidate
        WHERE candidate.sort_anchor_at IS NOT NULL
        ON CONFLICT(prompt_cache_key) DO UPDATE SET
            source_scope_all = excluded.source_scope_all,
            source_scope_proxy_only = excluded.source_scope_proxy_only,
            created_at = excluded.created_at,
            last_activity_at = excluded.last_activity_at,
            last_terminal_at = excluded.last_terminal_at,
            last_in_flight_at = excluded.last_in_flight_at,
            sort_anchor_at = excluded.sort_anchor_at,
            request_count = excluded.request_count,
            total_tokens = excluded.total_tokens,
            total_cost = excluded.total_cost,
            proxy_created_at = excluded.proxy_created_at,
            proxy_last_activity_at = excluded.proxy_last_activity_at,
            proxy_last_terminal_at = excluded.proxy_last_terminal_at,
            proxy_last_in_flight_at = excluded.proxy_last_in_flight_at,
            proxy_sort_anchor_at = excluded.proxy_sort_anchor_at,
            proxy_request_count = excluded.proxy_request_count,
            proxy_total_tokens = excluded.proxy_total_tokens,
            proxy_total_cost = excluded.proxy_total_cost,
            updated_at = excluded.updated_at;
        DELETE FROM prompt_cache_working_set_live
        WHERE prompt_cache_key = {key_expr}
          AND prompt_cache_key IS NOT NULL
          AND prompt_cache_key <> ''
          AND NOT EXISTS (
              SELECT 1
              FROM codex_invocations
              WHERE {prompt_cache_key_sql} = {key_expr}
                AND {prompt_cache_key_sql} IS NOT NULL
                AND {prompt_cache_key_sql} <> ''
                AND (
                    LOWER(TRIM({display_status_sql})) IN ('running', 'pending')
                    OR occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds')
                )
          )
        "#;

pub(crate) fn prompt_cache_working_set_live_refresh_sql_for_key(key_expr: &str) -> String {
    let display_status_sql = crate::api::invocation_display_status_sql();
    PROMPT_CACHE_WORKING_SET_LIVE_REFRESH_SQL_TEMPLATE
        .replace(
            "{prompt_cache_key_sql}",
            INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        )
        .replace("{display_status_sql}", &display_status_sql)
        .replace("{key_expr}", key_expr)
        .replace("{source_proxy}", SOURCE_PROXY)
        .replace(
            "{window_seconds}",
            &PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS.to_string(),
        )
        .replace("{shanghai_now_sql}", SHANGHAI_NOW_SQL)
}
