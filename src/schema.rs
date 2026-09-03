use super::*;

pub(crate) static ENSURE_SCHEMA_LOCKS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) const INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS: i64 = 300;
pub(crate) const SHANGHAI_NOW_SQL: &str = "datetime('now', '+8 hours')";
const INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET: &str =
    "invocation_rollup_hourly_token_components_v1";
const INVOCATION_RAW_CODEC_MIGRATION_NAME: &str = "backfill_raw_codecs_v1";
const LEGACY_RAW_BLOB_LINK_SEED_MIGRATION_NAME: &str = "seed_existing_raw_blob_links_v1";
const TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL: &str = r#"
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

async fn ensure_nullable_real_column(
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

async fn ensure_column_with_definition(
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
        .execute(tx.as_mut())
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_insert")?;

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
        .execute(tx.as_mut())
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_update")?;

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
        .execute(tx.as_mut())
        .await
        .context("failed to ensure trigger trg_codex_invocations_live_delete")?;

    tx.commit()
        .await
        .context("failed to commit invocation_in_progress_live trigger rebuild")?;

    Ok(())
}

pub(crate) fn prompt_cache_working_set_live_refresh_sql_for_key(key_expr: &str) -> String {
    let display_status_sql = crate::api::invocation_display_status_sql();
    format!(
        r#"
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
        "#,
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        display_status_sql = display_status_sql,
        key_expr = key_expr,
        source_proxy = SOURCE_PROXY,
        window_seconds = PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS,
    )
}

pub(crate) async fn rebuild_prompt_cache_working_set_live_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query("DELETE FROM prompt_cache_working_set_live")
        .execute(pool)
        .await
        .context("failed to clear prompt_cache_working_set_live before rebuild")?;

    let display_status_sql = crate::api::invocation_display_status_sql();
    let rebuild_sql = format!(
        r#"
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
            keyed.prompt_cache_key,
            1,
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
            COALESCE(SUM(CASE WHEN keyed.source = '{source_proxy}' THEN COALESCE(keyed.cost, 0.0) ELSE 0.0 END), 0.0) AS proxy_total_cost,
            {shanghai_now_sql}
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
            WHERE {prompt_cache_key_sql} IS NOT NULL
              AND {prompt_cache_key_sql} <> ''
              AND (
                    LOWER(TRIM({display_status_sql})) IN ('running', 'pending')
                    OR occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds')
              )
        ) AS keyed
        GROUP BY keyed.prompt_cache_key
        HAVING MAX(
            CASE
                WHEN keyed.is_in_flight = 1 THEN keyed.occurred_at
                WHEN keyed.occurred_at >= datetime('now', '+8 hours', '-{window_seconds} seconds') THEN keyed.occurred_at
                ELSE NULL
            END
        ) IS NOT NULL
        "#,
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        display_status_sql = display_status_sql,
        source_proxy = SOURCE_PROXY,
        window_seconds = PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS,
        shanghai_now_sql = SHANGHAI_NOW_SQL,
    );
    sqlx::query(&rebuild_sql)
        .execute(pool)
        .await
        .context("failed to rebuild prompt_cache_working_set_live rows")?;

    Ok(())
}

pub(crate) fn pool_upstream_node_health_hourly_archive_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
            archive_identity TEXT NOT NULL,
            archive_batch_id INTEGER,
            archive_file_path TEXT NOT NULL,
            proxy_binding_key_snapshot TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_identity, proxy_binding_key_snapshot, bucket_start_epoch)
        )
        "#
    )
}

pub(crate) fn prompt_cache_conversation_bindings_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
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
            available_models_mode TEXT,
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
        "#
    )
}

pub(crate) fn prompt_cache_conversation_operation_events_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prompt_cache_key TEXT NOT NULL,
            action TEXT NOT NULL,
            origin TEXT NOT NULL,
            info_types_json TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            headline TEXT NOT NULL,
            changed_fields_json TEXT,
            binding_before_json TEXT,
            binding_after_json TEXT,
            sticky_before_json TEXT,
            sticky_after_json TEXT,
            invoke_id TEXT,
            routing_context_json TEXT,
            routing_scope_json TEXT,
            sticky_transitions_json TEXT
        )
        "#
    )
}

pub(crate) async fn prompt_cache_conversation_bindings_existing_columns(
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashSet<String>> {
    let rows = sqlx::query("PRAGMA table_info('prompt_cache_conversation_bindings')")
        .fetch_all(pool)
        .await
        .context("failed to inspect prompt_cache_conversation_bindings columns")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect())
}

pub(crate) fn prompt_cache_binding_copy_expr(
    existing_columns: &std::collections::HashSet<String>,
    column: &str,
) -> &'static str {
    if existing_columns.contains(column) {
        match column {
            "responses_first_byte_timeout_secs" => "responses_first_byte_timeout_secs",
            "compact_first_byte_timeout_secs" => "compact_first_byte_timeout_secs",
            "image_first_byte_timeout_secs" => "image_first_byte_timeout_secs",
            "responses_stream_timeout_secs" => "responses_stream_timeout_secs",
            "compact_stream_timeout_secs" => "compact_stream_timeout_secs",
            "allow_switch_upstream" => "allow_switch_upstream",
            "fast_mode_rewrite_mode" => "fast_mode_rewrite_mode",
            "image_tool_rewrite_mode" => "image_tool_rewrite_mode",
            "codex_imagegen_rewrite_mode" => "codex_imagegen_rewrite_mode",
            "available_models_json" => "available_models_json",
            "available_models_mode" => "available_models_mode",
            "forward_proxy_key" => "forward_proxy_key",
            "forward_proxy_keys_json" => "forward_proxy_keys_json",
            _ => "NULL",
        }
    } else {
        "NULL"
    }
}

pub(crate) async fn migrate_prompt_cache_conversation_bindings_contract(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    const TEMP_TABLE: &str = "prompt_cache_conversation_bindings_v2";

    let current_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'prompt_cache_conversation_bindings' LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some(current_sql) = current_sql else {
        return Ok(());
    };
    let normalized_sql = current_sql.to_ascii_lowercase();
    let compatible_before_codex_imagegen = normalized_sql.contains("'none'")
        && normalized_sql.contains("responses_first_byte_timeout_secs")
        && normalized_sql.contains("compact_first_byte_timeout_secs")
        && normalized_sql.contains("image_first_byte_timeout_secs")
        && normalized_sql.contains("responses_stream_timeout_secs")
        && normalized_sql.contains("compact_stream_timeout_secs")
        && normalized_sql.contains("allow_switch_upstream")
        && normalized_sql.contains("fast_mode_rewrite_mode")
        && normalized_sql.contains("image_tool_rewrite_mode")
        && normalized_sql.contains("available_models_json")
        && normalized_sql.contains("forward_proxy_key")
        && normalized_sql.contains("forward_proxy_keys_json");
    let already_compatible =
        compatible_before_codex_imagegen && normalized_sql.contains("codex_imagegen_rewrite_mode");
    if already_compatible {
        return Ok(());
    }
    let existing_columns = prompt_cache_conversation_bindings_existing_columns(pool).await?;
    if compatible_before_codex_imagegen && !existing_columns.contains("codex_imagegen_rewrite_mode")
    {
        sqlx::query(
            "ALTER TABLE prompt_cache_conversation_bindings ADD COLUMN codex_imagegen_rewrite_mode TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add codex_imagegen_rewrite_mode to compatible conversation bindings")?;
        return Ok(());
    }
    let responses_first_byte_timeout_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "responses_first_byte_timeout_secs");
    let compact_first_byte_timeout_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "compact_first_byte_timeout_secs");
    let image_first_byte_timeout_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "image_first_byte_timeout_secs");
    let responses_stream_timeout_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "responses_stream_timeout_secs");
    let compact_stream_timeout_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "compact_stream_timeout_secs");
    let forward_proxy_key_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "forward_proxy_key");
    let allow_switch_upstream_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "allow_switch_upstream");
    let fast_mode_rewrite_mode_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "fast_mode_rewrite_mode");
    let image_tool_rewrite_mode_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "image_tool_rewrite_mode");
    let codex_imagegen_rewrite_mode_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "codex_imagegen_rewrite_mode");
    let available_models_json_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "available_models_json");
    let available_models_mode_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "available_models_mode");
    let forward_proxy_keys_json_copy =
        prompt_cache_binding_copy_expr(&existing_columns, "forward_proxy_keys_json");

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to clear stale prompt_cache_conversation_bindings migration temp table")?;

    let create_temp_sql = prompt_cache_conversation_bindings_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to create prompt_cache_conversation_bindings migration temp table")?;

    let copy_sql = format!(
        r#"
        INSERT INTO {TEMP_TABLE} (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            image_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            allow_switch_upstream,
            fast_mode_rewrite_mode,
            image_tool_rewrite_mode,
            codex_imagegen_rewrite_mode,
            available_models_json,
            available_models_mode,
            forward_proxy_key,
            forward_proxy_keys_json,
            created_at,
            updated_at
        )
        SELECT
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            {responses_first_byte_timeout_copy},
            {compact_first_byte_timeout_copy},
            {image_first_byte_timeout_copy},
            {responses_stream_timeout_copy},
            {compact_stream_timeout_copy},
            {allow_switch_upstream_copy},
            {fast_mode_rewrite_mode_copy},
            {image_tool_rewrite_mode_copy},
            {codex_imagegen_rewrite_mode_copy},
            {available_models_json_copy},
            {available_models_mode_copy},
            {forward_proxy_key_copy},
            {forward_proxy_keys_json_copy},
            created_at,
            updated_at
        FROM prompt_cache_conversation_bindings
        "#
    );
    sqlx::query(&copy_sql).execute(tx.as_mut()).await.context(
        "failed to copy prompt_cache_conversation_bindings rows into migration temp table",
    )?;

    sqlx::query("DROP TABLE prompt_cache_conversation_bindings")
        .execute(tx.as_mut())
        .await
        .context(
            "failed to drop legacy prompt_cache_conversation_bindings table during migration",
        )?;

    let rename_sql =
        format!("ALTER TABLE {TEMP_TABLE} RENAME TO prompt_cache_conversation_bindings");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to swap migrated prompt_cache_conversation_bindings table into place")?;

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn migrate_pool_upstream_node_health_hourly_archive_identity(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    const TEMP_TABLE: &str = "pool_upstream_node_health_hourly_archive_v2";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to clear stale pool_upstream_node_health_hourly_archive migration temp table",
        )?;

    let create_temp_sql = pool_upstream_node_health_hourly_archive_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to create pool_upstream_node_health_hourly_archive migration temp table",
        )?;

    let copy_sql = format!(
        r#"
        INSERT INTO {TEMP_TABLE} (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count,
            updated_at
        )
        SELECT
            CASE
                WHEN batches.id IS NOT NULL THEN 'batch:' || CAST(batches.id AS TEXT)
                ELSE 'legacy-file:' || legacy.archive_file_path
            END AS archive_identity,
            batches.id AS archive_batch_id,
            legacy.archive_file_path,
            legacy.proxy_binding_key_snapshot,
            legacy.bucket_start_epoch,
            legacy.success_count,
            legacy.failure_count,
            legacy.updated_at
        FROM pool_upstream_node_health_hourly_archive AS legacy
        LEFT JOIN archive_batches AS batches
          ON batches.dataset = 'pool_upstream_request_attempts'
         AND batches.file_path = legacy.archive_file_path
        "#
    );
    sqlx::query(&copy_sql).execute(tx.as_mut()).await.context(
        "failed to copy pool_upstream_node_health_hourly_archive rows into migration temp table",
    )?;

    sqlx::query("DROP TABLE pool_upstream_node_health_hourly_archive")
        .execute(tx.as_mut())
        .await
        .context(
            "failed to drop legacy pool_upstream_node_health_hourly_archive table during migration",
        )?;

    let rename_sql =
        format!("ALTER TABLE {TEMP_TABLE} RENAME TO pool_upstream_node_health_hourly_archive");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to swap migrated pool_upstream_node_health_hourly_archive table into place",
        )?;

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn backfill_upstream_account_usage_hourly_status_counts(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
    let resolved_failure_sql = crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let upstream_account_id_sql = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let bucket_epoch_sql = "((CASE
                WHEN instr(occurred_at, 'T') > 0
                    THEN CAST(strftime('%s', occurred_at) AS INTEGER)
                ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
            END) / 3600) * 3600";
    let terminal_status_sql = "(LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending'))";
    let live_backfill_sql = format!(
        r#"
        UPDATE upstream_account_usage_hourly
        SET
            success_count = (
                SELECT COUNT(*)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND {success_like_sql}
                  AND {resolved_failure_sql} = 'none'
            ),
            cache_input_tokens = (
                SELECT COALESCE(SUM(cache_input_tokens), 0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
            ),
            reasoning_tokens = (
                SELECT COALESCE(SUM(reasoning_tokens), 0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
            ),
            failure_count = (
                SELECT COUNT(*)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND {terminal_status_sql}
                  AND {resolved_failure_sql} IN ('service_failure', 'client_failure', 'client_abort')
            ),
            non_success_cost = (
                SELECT COALESCE(SUM(COALESCE(cost, 0.0)), 0.0)
                FROM codex_invocations
                WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
                  AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
                  AND (
                    LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                    OR (
                        {terminal_status_sql}
                        AND {resolved_failure_sql} IN ('service_failure', 'client_failure', 'client_abort')
                    )
                  )
            )
        WHERE EXISTS (
            SELECT 1
            FROM codex_invocations
            WHERE {bucket_epoch_sql} = upstream_account_usage_hourly.bucket_start_epoch
              AND {upstream_account_id_sql} = upstream_account_usage_hourly.upstream_account_id
        )
        "#,
    );
    sqlx::query(&live_backfill_sql)
        .execute(pool)
        .await
        .context("failed to backfill live upstream account hourly status counts")?;

    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_hourly
        WHERE EXISTS (
            SELECT 1
            FROM archive_batches AS batches
            JOIN hourly_rollup_archive_replay AS replay
              ON replay.dataset = batches.dataset
             AND replay.file_path = batches.file_path
             AND replay.target = 'upstream_account_usage_hourly'
            WHERE batches.dataset = 'codex_invocations'
              AND batches.status = 'completed'
              AND batches.coverage_start_at IS NOT NULL
              AND batches.coverage_end_at IS NOT NULL
              AND upstream_account_usage_hourly.bucket_start_epoch BETWEEN
                    (((CASE
                        WHEN instr(batches.coverage_start_at, 'T') > 0
                            THEN CAST(strftime('%s', batches.coverage_start_at) AS INTEGER)
                        ELSE CAST(strftime('%s', batches.coverage_start_at || '+08:00') AS INTEGER)
                    END) / 3600) * 3600)
                AND (((CASE
                        WHEN instr(batches.coverage_end_at, 'T') > 0
                            THEN CAST(strftime('%s', batches.coverage_end_at) AS INTEGER)
                        ELSE CAST(strftime('%s', batches.coverage_end_at || '+08:00') AS INTEGER)
                    END) / 3600) * 3600)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale archived upstream account hourly rollups")?;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND EXISTS (
              SELECT 1
              FROM hourly_rollup_archive_replay AS replay
              WHERE replay.dataset = archive_batches.dataset
                AND replay.file_path = archive_batches.file_path
                AND replay.target = 'upstream_account_usage_hourly'
          )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to reopen upstream account hourly archive materialization")?;

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_progress
        WHERE dataset = 'codex_invocations'
          AND EXISTS (
              SELECT 1
              FROM hourly_rollup_archive_replay AS replay
              WHERE replay.dataset = hourly_rollup_archive_progress.dataset
                AND replay.file_path = hourly_rollup_archive_progress.file_path
                AND replay.target = 'upstream_account_usage_hourly'
          )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale upstream account hourly archive progress")?;

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND target = 'upstream_account_usage_hourly'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale upstream account hourly archive replay markers")?;

    Ok(())
}

pub(crate) async fn reopen_upstream_account_stats_rollup_archives(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    for target in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND EXISTS (
                  SELECT 1
                  FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.dataset = archive_batches.dataset
                    AND replay.file_path = archive_batches.file_path
                    AND replay.target = ?1
              )
            "#,
        )
        .bind(target)
        .execute(pool)
        .await
        .with_context(|| format!("failed to reopen archive materialization for {target}"))?;

        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE dataset = 'codex_invocations'
              AND target = ?1
            "#,
        )
        .bind(target)
        .execute(pool)
        .await
        .with_context(|| format!("failed to clear stale archive replay markers for {target}"))?;
    }

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_progress
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to clear stale invocation archive progress while reopening upstream account stats rollups")?;

    Ok(())
}

async fn ensure_invocation_raw_codec_backfill(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_invocation_raw_codec_migrations (
            migration_name TEXT PRIMARY KEY,
            completed_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure raw codec migration marker table")?;

    let already_completed = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM codex_invocation_raw_codec_migrations WHERE migration_name = ?1)",
    )
    .bind(INVOCATION_RAW_CODEC_MIGRATION_NAME)
    .fetch_one(pool)
    .await?
        != 0;
    if already_completed {
        return Ok(());
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin raw codec backfill migration")?;
    let already_completed = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM codex_invocation_raw_codec_migrations WHERE migration_name = ?1)",
    )
    .bind(INVOCATION_RAW_CODEC_MIGRATION_NAME)
    .fetch_one(tx.as_mut())
    .await?
        != 0;
    if already_completed {
        tx.commit()
            .await
            .context("failed to commit raw codec migration marker check")?;
        return Ok(());
    }

    if !legacy_raw_blob_link_seed_completed(&mut tx).await? {
        sqlx::query(
            r#"
            UPDATE codex_invocations
            SET request_raw_codec = CASE
                    WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                    ELSE 'identity'
                END
            WHERE COALESCE(TRIM(request_raw_codec), '') = ''
               OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
            "#,
        )
        .execute(tx.as_mut())
        .await
        .context("failed to backfill codex_invocations request_raw_codec")?;

        sqlx::query(
            r#"
            UPDATE codex_invocations
            SET response_raw_codec = CASE
                    WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                    ELSE 'identity'
                END
            WHERE COALESCE(TRIM(response_raw_codec), '') = ''
               OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
            "#,
        )
        .execute(tx.as_mut())
        .await
        .context("failed to backfill codex_invocations response_raw_codec")?;
    }

    sqlx::query("INSERT INTO codex_invocation_raw_codec_migrations (migration_name) VALUES (?1)")
        .bind(INVOCATION_RAW_CODEC_MIGRATION_NAME)
        .execute(tx.as_mut())
        .await
        .context("failed to record raw codec backfill completion")?;
    tx.commit()
        .await
        .context("failed to commit raw codec backfill migration")?;

    Ok(())
}

async fn legacy_raw_blob_link_seed_completed(
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

fn legacy_archive_segment_id_range(part_key: &str) -> Option<(i64, i64)> {
    let encoded = part_key.strip_prefix("part-")?;
    let (lower, remainder) = encoded.split_once('-')?;
    let (upper, _) = remainder.split_once('-')?;
    let lower = i64::from_str_radix(lower, 16).ok()?;
    let upper = i64::from_str_radix(upper, 16).ok()?;
    (lower > 0 && upper >= lower).then_some((lower, upper))
}

async fn classify_legacy_invocation_detail_archive_mirrors(pool: &Pool<Sqlite>) -> Result<()> {
    const CLASSIFICATION_CHUNK_SIZE: i64 = 512;
    let mut after_id = 0_i64;

    loop {
        let candidates = sqlx::query_as::<_, (i64, String)>(
            r#"
            SELECT id, part_key
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND summary_source_kind = 'unknown'
              AND layout = 'segment_v1'
              AND part_key IS NOT NULL
              AND id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(after_id)
        .bind(CLASSIFICATION_CHUNK_SIZE)
        .fetch_all(pool)
        .await?;
        let Some(last_id) = candidates.last().map(|(id, _)| *id) else {
            break;
        };
        after_id = last_id;

        let ranges = candidates
            .into_iter()
            .filter_map(|(id, part_key)| {
                legacy_archive_segment_id_range(&part_key)
                    .map(|(lower_id, upper_id)| (id, lower_id, upper_id))
            })
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            continue;
        }

        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "WITH candidates(id, lower_id, upper_id) AS (VALUES ",
        );
        for (index, (id, lower_id, upper_id)) in ranges.into_iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            query
                .push("(")
                .push_bind(id)
                .push(", ")
                .push_bind(lower_id)
                .push(", ")
                .push_bind(upper_id)
                .push(")");
        }
        query.push(
            ") \
             UPDATE archive_batches \
             SET summary_source_kind = 'live_mirror' \
             WHERE summary_source_kind = 'unknown' \
               AND id IN ( \
                    SELECT id FROM candidates \
                    WHERE ( \
                        SELECT COUNT(*) FROM codex_invocations AS live \
                        WHERE live.id BETWEEN lower_id AND upper_id \
                    ) = upper_id - lower_id + 1 \
               )",
        );
        query.build().execute(pool).await?;
    }

    Ok(())
}

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let schema_lock = ensure_schema_lock(pool);
    let _schema_guard = schema_lock.lock_owned().await;

    // Existing live-update triggers may already reference this table after an interrupted older
    // startup. Restore it before any schema work can update invocations.
    sqlx::query(TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL)
        .execute(pool)
        .await
        .context("failed to ensure timeseries_minute_projection_v2 recovery table existence")?;

    let create_sql = codex_invocations_create_sql("codex_invocations");
    sqlx::query(&create_sql)
        .execute(pool)
        .await
        .context("failed to ensure codex_invocations table existence")?;

    let mut existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if existing.contains("raw_expires_at") {
        migrate_codex_invocations_drop_raw_expires_at(pool, &existing).await?;
        existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    }

    for (column, ty) in [
        ("source", "TEXT NOT NULL DEFAULT 'xy'"),
        ("model", "TEXT"),
        ("input_tokens", "INTEGER"),
        ("output_tokens", "INTEGER"),
        ("cache_input_tokens", "INTEGER"),
        ("reasoning_tokens", "INTEGER"),
        ("total_tokens", "INTEGER"),
        ("cost", "REAL"),
        ("cost_input", "REAL"),
        ("cost_cache_write", "REAL"),
        ("cost_cache_read", "REAL"),
        ("cost_output", "REAL"),
        ("cost_reasoning", "REAL"),
        ("status", "TEXT"),
        ("error_message", "TEXT"),
        ("failure_kind", "TEXT"),
        ("failure_class", "TEXT"),
        ("is_actionable", "INTEGER NOT NULL DEFAULT 0"),
        ("payload", "TEXT"),
        ("cost_estimated", "INTEGER NOT NULL DEFAULT 0"),
        ("price_version", "TEXT"),
        ("request_raw_path", "TEXT"),
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("request_raw_size", "INTEGER"),
        ("request_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("request_raw_truncated_reason", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("timeline_json", "TEXT"),
        ("detail_level", "TEXT NOT NULL DEFAULT 'full'"),
        ("detail_pruned_at", "TEXT"),
        ("detail_prune_reason", "TEXT"),
        ("t_total_ms", "REAL"),
        ("t_req_read_ms", "REAL"),
        ("t_req_parse_ms", "REAL"),
        ("t_upstream_connect_ms", "REAL"),
        ("t_upstream_ttfb_ms", "REAL"),
        ("first_token_ms", "REAL"),
        ("t_upstream_stream_ms", "REAL"),
        ("t_resp_parse_ms", "REAL"),
        ("t_persist_ms", "REAL"),
    ] {
        if !existing.contains(column) {
            let statement = format!("ALTER TABLE codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add column {column}"))?;
        }
    }

    ensure_invocation_raw_codec_backfill(pool).await?;

    // Speed up time-range scans and ordering on the stats endpoints
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at
        ON codex_invocations (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at")?;

    // Benefit queries that filter by time and status (e.g., error distribution)
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at_status
        ON codex_invocations (occurred_at, status)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at_status")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_source_occurred_at
        ON codex_invocations (source, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_source_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_class_occurred_at
        ON codex_invocations (failure_class, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_class_occurred_at")?;

    let mut prompt_cache_key_index_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache key index refresh")?;
    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_occurred_at")
        .execute(prompt_cache_key_index_tx.as_mut())
        .await
        .context("failed to drop stale idx_codex_invocations_prompt_cache_key_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(prompt_cache_key_index_tx.as_mut())
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;
    prompt_cache_key_index_tx
        .commit()
        .await
        .context("failed to commit prompt cache key index refresh")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_recent_lookup
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END),
            source,
            occurred_at DESC,
            id DESC
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_recent_lookup")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_occurred_at
        ON codex_invocations (model, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_occurred_at
        ON codex_invocations (failure_kind, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_invoke_id_occurred_at
        ON codex_invocations (invoke_id, occurred_at, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_invoke_id_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.endpoint') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.requesterIp') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_upstream_account_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_upstream_account_occurred_at")?;

    // The account Sticky-key preview ranks recent invocations within each key. Keep
    // the JSON compatibility expressions identical to that query so SQLite can use
    // the index for both predicates and the window ordering.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_account_sticky_key_recent
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END),
            (CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END),
            occurred_at DESC,
            id DESC
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_account_sticky_key_recent")?;

    // The records analytics page compares trimmed lowercase text for exact-match filters.
    // Mirror those expressions in dedicated indexes so high-volume searches avoid full index scans.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(model, '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END,
                failure_kind
            ), '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_filter_occurred_at")?;

    let mut prompt_cache_key_filter_index_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache key filter index refresh")?;
    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at")
        .execute(prompt_cache_key_filter_index_tx.as_mut())
        .await
        .context(
            "failed to drop stale idx_codex_invocations_prompt_cache_key_filter_occurred_at",
        )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.promptCacheKey') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(prompt_cache_key_filter_index_tx.as_mut())
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at")?;
    prompt_cache_key_filter_index_tx
        .commit()
        .await
        .context("failed to commit prompt cache key filter index refresh")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_proxy_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                COALESCE(
                    NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), ''),
                    CASE WHEN TRIM(source) != 'proxy' THEN TRIM(source) END
                ),
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_proxy_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_request_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE request_raw_path IS NOT NULL
          AND request_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_request_raw_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_response_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE response_raw_path IS NOT NULL
          AND response_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_response_raw_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_proxy_usage_backfill_pending
        ON codex_invocations (source, status, id)
        WHERE total_tokens IS NULL
          AND response_raw_path IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_proxy_usage_backfill_pending")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_in_progress_live (
            invocation_id INTEGER PRIMARY KEY,
            source TEXT NOT NULL,
            upstream_account_id INTEGER,
            prompt_cache_key TEXT,
            is_retry_after_failure_all INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_proxy_only INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_account_all INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_account_proxy_only INTEGER NOT NULL DEFAULT 0,
            upstream_ttfb_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_in_progress_live table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_in_progress_live_source_account
        ON invocation_in_progress_live (source, upstream_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_in_progress_live_source_account")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_in_progress_live_prompt_cache_key
        ON invocation_in_progress_live (prompt_cache_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_in_progress_live_prompt_cache_key")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_working_set_live (
            prompt_cache_key TEXT PRIMARY KEY,
            source_scope_all INTEGER NOT NULL DEFAULT 1,
            source_scope_proxy_only INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            last_activity_at TEXT NOT NULL,
            last_terminal_at TEXT,
            last_in_flight_at TEXT,
            sort_anchor_at TEXT NOT NULL,
            request_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0.0,
            proxy_created_at TEXT,
            proxy_last_activity_at TEXT,
            proxy_last_terminal_at TEXT,
            proxy_last_in_flight_at TEXT,
            proxy_sort_anchor_at TEXT,
            proxy_request_count INTEGER NOT NULL DEFAULT 0,
            proxy_total_tokens INTEGER NOT NULL DEFAULT 0,
            proxy_total_cost REAL NOT NULL DEFAULT 0.0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_working_set_live table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_working_set_live_sort_anchor
        ON prompt_cache_working_set_live (sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_prompt_cache_working_set_live_sort_anchor")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_working_set_live_proxy_sort_anchor
        ON prompt_cache_working_set_live (source_scope_proxy_only, sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_prompt_cache_working_set_live_proxy_sort_anchor")?;

    rebuild_invocation_in_progress_live_triggers(pool)
        .await
        .context("failed to rebuild invocation_in_progress_live triggers at startup")?;

    let prompt_cache_insert_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_insert
        AFTER INSERT ON codex_invocations
        BEGIN
            {refresh_sql};
        END
        "#,
        refresh_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW")
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
            &invocation_in_progress_live_prompt_cache_key_expr("OLD")
        ),
        refresh_new_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW")
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
            &invocation_in_progress_live_prompt_cache_key_expr("OLD")
        ),
    );
    let mut prompt_cache_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache working set trigger refresh")?;
    for trigger_name in [
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(prompt_cache_trigger_tx.as_mut())
            .await
            .with_context(|| format!("failed to drop stale trigger {trigger_name}"))?;
    }
    sqlx::query(&prompt_cache_insert_trigger_sql)
        .execute(prompt_cache_trigger_tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_insert",
        )?;
    sqlx::query(&prompt_cache_update_trigger_sql)
        .execute(prompt_cache_trigger_tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_update",
        )?;
    sqlx::query(&prompt_cache_delete_trigger_sql)
        .execute(prompt_cache_trigger_tx.as_mut())
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_delete",
        )?;
    prompt_cache_trigger_tx
        .commit()
        .await
        .context("failed to commit prompt cache working set trigger refresh")?;

    rebuild_invocation_in_progress_live_table(pool)
        .await
        .context("failed to rebuild invocation_in_progress_live table at startup")?;
    rebuild_prompt_cache_working_set_live_table(pool)
        .await
        .context("failed to rebuild prompt_cache_working_set_live table at startup")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_quota_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at TEXT NOT NULL DEFAULT (datetime('now')),
            amount_limit REAL,
            used_amount REAL,
            remaining_amount REAL,
            period TEXT,
            period_reset_time TEXT,
            expire_time TEXT,
            is_active INTEGER,
            total_cost REAL,
            total_requests INTEGER,
            total_tokens INTEGER,
            last_request_time TEXT,
            billing_type TEXT,
            remaining_count INTEGER,
            used_count INTEGER,
            sub_type_name TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure codex_quota_snapshots table existence")?;

    // Speed up latest snapshot lookup
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_quota_snapshots_captured_at
        ON codex_quota_snapshots (captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_quota_snapshots_captured_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL DEFAULT 'legacy_month',
            codec TEXT NOT NULL DEFAULT 'gzip',
            writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1',
            cleanup_state TEXT NOT NULL DEFAULT 'active',
            cleanup_source_safe_start_date TEXT,
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_start_epoch INTEGER,
            coverage_end_epoch INTEGER,
            archive_expires_at TEXT,
            summary_source_kind TEXT NOT NULL DEFAULT 'unknown',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;

    let archive_batch_columns = load_sqlite_table_columns(pool, "archive_batches").await?;
    for (column, ty) in [
        // A legacy archive without a recorded hash cannot prove replay identity and remains
        // pending until it is rebuilt; use a nullable upgrade column for that state.
        ("sha256", "TEXT"),
        ("day_key", "TEXT"),
        ("part_key", "TEXT"),
        ("layout", "TEXT NOT NULL DEFAULT 'legacy_month'"),
        ("codec", "TEXT NOT NULL DEFAULT 'gzip'"),
        ("writer_version", "TEXT NOT NULL DEFAULT 'legacy_month_v1'"),
        ("cleanup_state", "TEXT NOT NULL DEFAULT 'active'"),
        ("cleanup_source_safe_start_date", "TEXT"),
        ("superseded_by", "INTEGER"),
        ("coverage_start_at", "TEXT"),
        ("coverage_end_at", "TEXT"),
        ("coverage_start_epoch", "INTEGER"),
        ("coverage_end_epoch", "INTEGER"),
        ("archive_expires_at", "TEXT"),
        ("upstream_activity_manifest_refreshed_at", "TEXT"),
        ("historical_rollups_materialized_at", "TEXT"),
        ("summary_source_kind", "TEXT NOT NULL DEFAULT 'unknown'"),
    ] {
        if !archive_batch_columns.contains(column) {
            let statement = format!("ALTER TABLE archive_batches ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add archive_batches column {column}"))?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_summary_source_classification
        ON archive_batches (dataset, status, summary_source_kind, layout, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive source classification index")?;

    // A detail-prune archive duplicates records still retained in the live table. Segment keys
    // encode the exact inclusive ID bounds in hexadecimal; only a contiguous live range proves
    // that every archived record remains live. Unknown legacy manifests stay fail-closed.
    classify_legacy_invocation_detail_archive_mirrors(pool)
        .await
        .context("failed to classify legacy invocation detail archive mirrors")?;

    // Archive writers retain the established text bounds for compatibility, while read-side
    // coverage planners use these normalized epochs for indexed range lookups.
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET
            coverage_start_epoch = CASE
                WHEN coverage_start_at IS NULL THEN NULL
                WHEN instr(coverage_start_at, 'T') > 0
                    THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
            END,
            coverage_end_epoch = CASE
                WHEN coverage_end_at IS NULL THEN NULL
                WHEN instr(coverage_end_at, 'T') > 0
                    THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
            END
        WHERE (coverage_start_at IS NULL AND coverage_start_epoch IS NOT NULL)
           OR (coverage_start_at IS NOT NULL AND coverage_start_epoch IS NULL)
           OR (coverage_end_at IS NULL AND coverage_end_epoch IS NOT NULL)
           OR (coverage_end_at IS NOT NULL AND coverage_end_epoch IS NULL)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill normalized archive coverage epochs")?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_archive_batches_coverage_epoch_insert
        AFTER INSERT ON archive_batches
        BEGIN
            UPDATE archive_batches
            SET
                coverage_start_epoch = CASE
                    WHEN coverage_start_at IS NULL THEN NULL
                    WHEN instr(coverage_start_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
                END,
                coverage_end_epoch = CASE
                    WHEN coverage_end_at IS NULL THEN NULL
                    WHEN instr(coverage_end_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
                END
            WHERE id = NEW.id;
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive coverage epoch insert trigger")?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_archive_batches_coverage_epoch_update
        AFTER UPDATE OF coverage_start_at, coverage_end_at ON archive_batches
        BEGIN
            UPDATE archive_batches
            SET
                coverage_start_epoch = CASE
                    WHEN coverage_start_at IS NULL THEN NULL
                    WHEN instr(coverage_start_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_start_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_start_at || '+08:00') AS INTEGER)
                END,
                coverage_end_epoch = CASE
                    WHEN coverage_end_at IS NULL THEN NULL
                    WHEN instr(coverage_end_at, 'T') > 0
                        THEN CAST(strftime('%s', coverage_end_at) AS INTEGER)
                    ELSE CAST(strftime('%s', coverage_end_at || '+08:00') AS INTEGER)
                END
            WHERE id = NEW.id;
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive coverage epoch update trigger")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_month
        ON archive_batches (dataset, month_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_month")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_file_path
        ON archive_batches (dataset, file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_file_path")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_layout_day_part
        ON archive_batches (dataset, layout, day_key, part_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_layout_day_part")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_manifest_pending
        ON archive_batches (dataset, status, upstream_activity_manifest_refreshed_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_invocation_manifest_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_rollup_materialization
        ON archive_batches (dataset, status, historical_rollups_materialized_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_rollup_materialization")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_summary_source_coverage
        ON archive_batches (
            dataset,
            status,
            summary_source_kind,
            coverage_end_epoch,
            coverage_start_epoch,
            id
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive source coverage index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_coverage_epoch
        ON archive_batches (coverage_end_epoch, coverage_start_epoch)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND coverage_start_epoch IS NOT NULL
          AND coverage_end_epoch IS NOT NULL
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation archive coverage epoch index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_legacy_coverage_month
        ON archive_batches (month_key)
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND (coverage_start_at IS NULL OR coverage_end_at IS NULL)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation archive legacy coverage month index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batch_upstream_activity (
            archive_batch_id INTEGER NOT NULL,
            account_id INTEGER NOT NULL,
            last_activity_at TEXT NOT NULL,
            PRIMARY KEY (archive_batch_id, account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batch_upstream_activity table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_account_last_activity
        ON archive_batch_upstream_activity (account_id, last_activity_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_account_last_activity")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_batch
        ON archive_batch_upstream_activity (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_materialized_buckets (
            target TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            materialized_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_materialized_buckets table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_hourly_rollup_materialized_buckets_target_bucket
        ON hourly_rollup_materialized_buckets (target, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_hourly_rollup_materialized_buckets_target_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch INTEGER PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure account_activity_v2_bucket_repair_watermarks table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_daily (
            stats_date TEXT NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_daily table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_daily_source_date
        ON invocation_rollup_daily (source, stats_date)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_daily_source_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            terminal_count INTEGER NOT NULL DEFAULT 0,
            terminal_tokens INTEGER NOT NULL DEFAULT 0,
            terminal_cost REAL NOT NULL DEFAULT 0,
            terminal_proof_complete INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_hourly table existence")?;

    let invocation_rollup_hourly_columns =
        load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    let has_existing_invocation_rollup_hourly_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM invocation_rollup_hourly)")
            .fetch_one(pool)
            .await?
            != 0;
    let mut added_invocation_rollup_hourly_columns = false;
    for (column, ty) in [
        ("terminal_count", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_cost", "REAL NOT NULL DEFAULT 0"),
        ("terminal_proof_complete", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_response_byte_total_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_max_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_invocation_rollup_hourly_columns = true;
            let statement =
                format!("ALTER TABLE invocation_rollup_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add invocation_rollup_hourly column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_hourly_source_bucket
        ON invocation_rollup_hourly (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_hourly_source_bucket")?;
    if has_existing_invocation_rollup_hourly_rows {
        // The state boundary is the durable completion marker. It must not depend on whether
        // this invocation added columns: a process can stop after the final ALTER TABLE and
        // before this bootstrap runs.
        ensure_long_term_stats_schema(pool).await?;
        let integrity_source_start_date = sqlx::query_scalar::<_, Option<String>>(
            "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_optional(pool)
        .await?
        .flatten();
        if integrity_source_start_date.is_none() {
            crate::long_term_stats::bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(pool)
                .await?;
        }
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_failure_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            failure_class TEXT NOT NULL,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            error_category TEXT NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, failure_class, is_actionable, error_category)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_failure_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_failure_rollup_hourly_bucket
        ON invocation_failure_rollup_hourly (bucket_start_epoch, source, failure_class)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_failure_rollup_hourly_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_perf_stage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            stage TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            sum_ms REAL NOT NULL,
            max_ms REAL NOT NULL,
            histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, stage)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_perf_stage_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_proxy_perf_stage_hourly_stage_bucket
        ON proxy_perf_stage_hourly (stage, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_proxy_perf_stage_hourly_stage_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_rollup_hourly_key_bucket
        ON prompt_cache_rollup_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_rollup_hourly_key_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_upstream_account_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            upstream_account_name TEXT,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_upstream_account_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_upstream_account_hourly_key_bucket
        ON prompt_cache_upstream_account_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_upstream_account_hourly_key_bucket")?;

    // These tables intentionally keep only the key identity needed to calculate
    // parallel-work averages. They are not a second copy of invocation history.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_minute_key_rollup_source_minute
        ON parallel_work_minute_key_rollup (source, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work minute source range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, upstream_account_id, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_minute_key_rollup_account_range
        ON parallel_work_upstream_account_minute_key_rollup (upstream_account_id, minute_start_epoch, source)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account minute range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_hourly_rollup_scope_range
        ON parallel_work_hourly_rollup (source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work hourly scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_id INTEGER NOT NULL,
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_hourly_rollup_account_range
        ON parallel_work_upstream_account_hourly_rollup (upstream_account_id, source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account hourly range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_coverage (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            minute_keys_complete INTEGER NOT NULL DEFAULT 0,
            hourly_scalar_complete INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_coverage table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_rollup_coverage_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            full_detail_start_epoch INTEGER NOT NULL,
            latest_unrecoverable_detail_epoch INTEGER
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_rollup_coverage_state table existence")?;

    ensure_column_with_definition(
        pool,
        "parallel_work_rollup_coverage_state",
        "latest_unrecoverable_detail_epoch",
        "INTEGER",
    )
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_records (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL,
            upstream_account_key INTEGER NOT NULL,
            records_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_records table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2 (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_key INTEGER NOT NULL,
            aggregate_json TEXT NOT NULL,
            total_latency_samples_json TEXT NOT NULL,
            first_byte_samples_json TEXT NOT NULL,
            first_response_byte_total_samples_json TEXT NOT NULL,
            first_token_samples_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            coverage_state TEXT NOT NULL DEFAULT 'warming',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_timeseries_minute_projection_v2_scope_range
        ON timeseries_minute_projection_v2 (source_scope, upstream_account_key, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 state table existence")?;

    sqlx::query(TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL)
        .execute(pool)
        .await
        .context("failed to ensure timeseries_minute_projection_v2 recovery table existence")?;

    // This durable fence must exist before runtime accepts HTTP reads. The projection supervisor
    // is intentionally P2 and can start later, while direct non-proxy terminal corrections must
    // synchronously publish only a constant-size recovery marker instead of mutating every
    // projection row in a terminal write transaction.
    let mut replacement_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin non-proxy terminal replacement trigger refresh")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_timeseries_minute_projection_non_proxy_terminal_replacement",
    )
    .execute(replacement_trigger_tx.as_mut())
    .await
    .context("failed to refresh non-proxy terminal replacement projection trigger")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_timeseries_minute_projection_non_proxy_terminal_replacement
        AFTER UPDATE ON codex_invocations
        WHEN (
            COALESCE(OLD.source, '') <> 'proxy'
            OR COALESCE(NEW.source, '') <> 'proxy'
        )
        AND LOWER(TRIM(COALESCE(OLD.status, ''))) NOT IN ('running', 'pending')
        AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
        AND (
            OLD.occurred_at IS NOT NEW.occurred_at
            OR OLD.source IS NOT NEW.source
            OR OLD.model IS NOT NEW.model
            OR OLD.input_tokens IS NOT NEW.input_tokens
            OR OLD.output_tokens IS NOT NEW.output_tokens
            OR OLD.cache_input_tokens IS NOT NEW.cache_input_tokens
            OR OLD.reasoning_tokens IS NOT NEW.reasoning_tokens
            OR OLD.total_tokens IS NOT NEW.total_tokens
            OR OLD.cost IS NOT NEW.cost
            OR OLD.status IS NOT NEW.status
            OR OLD.error_message IS NOT NEW.error_message
            OR OLD.failure_kind IS NOT NEW.failure_kind
            OR OLD.failure_class IS NOT NEW.failure_class
            OR OLD.is_actionable IS NOT NEW.is_actionable
            OR OLD.payload IS NOT NEW.payload
            OR OLD.t_total_ms IS NOT NEW.t_total_ms
            OR OLD.t_req_read_ms IS NOT NEW.t_req_read_ms
            OR OLD.t_req_parse_ms IS NOT NEW.t_req_parse_ms
            OR OLD.t_upstream_connect_ms IS NOT NEW.t_upstream_connect_ms
            OR OLD.t_upstream_ttfb_ms IS NOT NEW.t_upstream_ttfb_ms
            OR OLD.t_upstream_stream_ms IS NOT NEW.t_upstream_stream_ms
            OR OLD.first_token_ms IS NOT NEW.first_token_ms
        )
        BEGIN
            INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending, updated_at)
            VALUES ('timeseries_minute_v2', 1, 1, datetime('now'))
            ON CONFLICT(consumer) DO UPDATE SET
                generation = timeseries_minute_projection_v2_recovery.generation + 1,
                invalidation_pending = 1,
                updated_at = excluded.updated_at;
        END
        "#,
    )
    .execute(replacement_trigger_tx.as_mut())
    .await
    .context("failed to ensure non-proxy terminal replacement projection trigger")?;
    replacement_trigger_tx
        .commit()
        .await
        .context("failed to commit non-proxy terminal replacement trigger refresh")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_rollup_maintenance_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            next_hour_epoch INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_rollup_maintenance_state table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            non_success_cost REAL NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cache_input_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_usage_hourly table existence")?;

    let upstream_account_usage_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_usage_hourly").await?;
    let mut upstream_account_usage_hourly_needs_status_backfill = false;
    for (column, ty) in [
        ("success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("failure_count", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
    ] {
        if !upstream_account_usage_hourly_columns.contains(column) {
            upstream_account_usage_hourly_needs_status_backfill = true;
            let sql = format!("ALTER TABLE upstream_account_usage_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&sql).execute(pool).await.with_context(|| {
                format!("failed to add upstream_account_usage_hourly column {column}")
            })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_hourly_account_bucket
        ON upstream_account_usage_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_usage_breakdown_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            normalized_model TEXT NOT NULL,
            normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_input REAL NOT NULL DEFAULT 0,
            cost_cache_write REAL NOT NULL DEFAULT 0,
            cost_cache_read REAL NOT NULL DEFAULT 0,
            cost_output REAL NOT NULL DEFAULT 0,
            cost_reasoning REAL NOT NULL DEFAULT 0,
            cost_unknown REAL NOT NULL DEFAULT 0,
            has_cost INTEGER NOT NULL DEFAULT 0,
            performance_total_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
            performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_response_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_account_key,
                normalized_model,
                normalized_reasoning_effort
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_usage_breakdown_hourly table existence")?;

    for (column, definition) in [
        (
            "performance_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("performance_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(
            pool,
            "upstream_account_usage_breakdown_hourly",
            column,
            definition,
        )
        .await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_breakdown_hourly_account_bucket
        ON upstream_account_usage_breakdown_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_breakdown_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_request_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_cost REAL NOT NULL DEFAULT 0,
            activity_v2_non_success_cost REAL NOT NULL DEFAULT 0,
            activity_v2_cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_cost REAL NOT NULL DEFAULT 0,
            activity_v2_first_response_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_response_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_max_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_last_invocation_at TEXT,
            activity_v2_latest_unkeyed_conversation_at TEXT,
            activity_v2_latest_first_response_at TEXT,
            activity_v2_latest_first_response_ms REAL,
            activity_v2_latest_total_latency_at TEXT,
            activity_v2_latest_total_latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_hourly table existence")?;
    let upstream_account_stats_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_hourly").await?;
    let mut added_upstream_account_stats_columns = false;
    for (column, ty) in [
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !upstream_account_stats_hourly_columns.contains(column) {
            added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    for (column, ty) in [
        ("activity_v2_request_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_count", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_tokens", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_failure_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_cost", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_non_success_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_cache_input_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_response_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_response_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        (
            "activity_v2_total_latency_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_total_latency_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        ("activity_v2_last_invocation_at", "TEXT"),
        ("activity_v2_latest_unkeyed_conversation_at", "TEXT"),
        ("activity_v2_latest_first_response_at", "TEXT"),
        ("activity_v2_latest_first_response_ms", "REAL"),
        ("activity_v2_latest_total_latency_at", "TEXT"),
        ("activity_v2_latest_total_latency_ms", "REAL"),
    ] {
        if !upstream_account_stats_hourly_columns.contains(column) {
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    let upstream_account_stats_hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_hourly rows")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_account_bucket
        ON upstream_account_stats_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_source_account_bucket
        ON upstream_account_stats_hourly (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_source_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_minute table existence")?;
    let upstream_account_stats_minute_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_minute").await?;
    for (column, ty) in [
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !upstream_account_stats_minute_columns.contains(column) {
            added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_minute ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_minute column {column}")
                })?;
        }
    }
    let upstream_account_stats_minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_minute rows")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_account_bucket
        ON upstream_account_stats_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_source_account_bucket
        ON upstream_account_stats_minute (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_source_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_host_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_host_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_host_bucket
        ON upstream_host_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_host_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_source_host_bucket
        ON upstream_host_network_minute (source, upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_source_host_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_socket_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upstream_account_id INTEGER,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_socket_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_account_bucket
        ON upstream_socket_network_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_source_bucket
        ON upstream_socket_network_minute (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_source_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_host_bucket
        ON upstream_socket_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_host_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_sticky_key_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            sticky_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id, sticky_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_sticky_key_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_sticky_key_hourly_account_bucket
        ON upstream_sticky_key_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_sticky_key_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempt_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            latency_sample_count INTEGER NOT NULL DEFAULT 0,
            latency_sum_ms REAL NOT NULL DEFAULT 0,
            latency_max_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempt_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempt_hourly_bucket_proxy
        ON forward_proxy_attempt_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempt_hourly_bucket_proxy")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_node_health_archive (
            archive_file_path TEXT NOT NULL,
            archived_row_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            proxy_binding_key_snapshot TEXT NOT NULL,
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_file_path, archived_row_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_node_health_archive table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_occurred_at_binding
        ON pool_upstream_node_health_archive (occurred_at, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_occurred_at_binding")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_file
        ON pool_upstream_node_health_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_file")?;

    let hourly_archive_sql = pool_upstream_node_health_hourly_archive_create_sql(
        "pool_upstream_node_health_hourly_archive",
    );
    sqlx::query(&hourly_archive_sql)
        .execute(pool)
        .await
        .context("failed to ensure pool_upstream_node_health_hourly_archive table existence")?;

    let hourly_archive_columns =
        load_sqlite_table_columns(pool, "pool_upstream_node_health_hourly_archive").await?;
    if !hourly_archive_columns.contains("archive_identity")
        || !hourly_archive_columns.contains("archive_batch_id")
    {
        migrate_pool_upstream_node_health_hourly_archive_identity(pool).await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_bucket_binding
        ON pool_upstream_node_health_hourly_archive (bucket_start_epoch, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_pool_upstream_node_health_hourly_archive_bucket_binding",
    )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_file
        ON pool_upstream_node_health_hourly_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_file")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_batch
        ON pool_upstream_node_health_hourly_archive (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            archive_sha256 TEXT,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

    // SQLite leaves an existing table untouched for CREATE TABLE IF NOT EXISTS. Upgrade the
    // pre-identity replay table before any startup backfill reads or writes archive_sha256.
    let hourly_rollup_archive_replay_columns =
        load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !hourly_rollup_archive_replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add hourly rollup archive replay identity column")?;
    }

    // An authoritative invocation archive becomes Summary-visible only after its bounded source
    // coverage and the three Summary rollup proofs commit in the same transaction. Live-detail
    // mirrors never enter Summary source coverage and therefore do not require these proofs.
    let mut archive_guard_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin authoritative invocation archive guard refresh")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_insert_authoritative_invocation_archive_requires_summary_proof",
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to replace authoritative invocation archive insert guard")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_update_authoritative_invocation_archive_requires_summary_proof",
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to replace authoritative invocation archive update guard")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_insert_authoritative_invocation_archive_requires_summary_proof
        BEFORE INSERT ON archive_batches
        WHEN NEW.dataset = 'codex_invocations'
          AND NEW.status = 'completed'
          AND NEW.summary_source_kind = 'authoritative'
          AND (
              NEW.coverage_start_at IS NULL
              OR NEW.coverage_end_at IS NULL
              OR NEW.historical_rollups_materialized_at IS NULL
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'invocation_rollup_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_stats_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_usage_breakdown_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
          )
        BEGIN
            SELECT RAISE(ABORT, 'completed codex_invocations archive requires Summary publication proof');
        END
        "#,
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to ensure authoritative invocation archive insert guard")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_update_authoritative_invocation_archive_requires_summary_proof
        BEFORE UPDATE OF status, summary_source_kind ON archive_batches
        WHEN NEW.dataset = 'codex_invocations'
          AND NEW.status = 'completed'
          AND NEW.summary_source_kind = 'authoritative'
          AND (OLD.status <> 'completed' OR OLD.summary_source_kind <> 'authoritative')
          AND (
              NEW.coverage_start_at IS NULL
              OR NEW.coverage_end_at IS NULL
              OR NEW.historical_rollups_materialized_at IS NULL
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'invocation_rollup_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_stats_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_usage_breakdown_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
          )
        BEGIN
            SELECT RAISE(ABORT, 'completed codex_invocations archive requires Summary publication proof');
        END
        "#,
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to ensure authoritative invocation archive update guard")?;
    archive_guard_trigger_tx
        .commit()
        .await
        .context("failed to commit authoritative invocation archive guard refresh")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_progress (
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_progress table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_live_progress (
            dataset TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_live_progress table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_coverage_checkpoint (
            scope TEXT PRIMARY KEY,
            manifest_high_watermark_id INTEGER NOT NULL,
            next_manifest_id INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_coverage_checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_journal (
            cursor INTEGER PRIMARY KEY AUTOINCREMENT,
            descriptor_version INTEGER NOT NULL,
            source_kind TEXT NOT NULL,
            source_revision INTEGER NOT NULL,
            first_row_id INTEGER NOT NULL,
            last_row_id INTEGER NOT NULL,
            occurred_start TEXT NOT NULL,
            occurred_end TEXT NOT NULL,
            descriptor_json TEXT NOT NULL,
            descriptor_bytes INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_journal table existence")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_summary_source_change_journal_revision \
         ON summary_source_change_journal (source_revision, cursor)",
    )
    .execute(pool)
    .await
    .context("failed to ensure summary source change journal revision index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_compaction_proof (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            first_cursor INTEGER NOT NULL,
            last_cursor INTEGER NOT NULL,
            proof_kind TEXT NOT NULL,
            proof_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_compaction_proof table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_cursor (
            scope TEXT PRIMARY KEY,
            cursor INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_cursor table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            page_index INTEGER NOT NULL,
            coverage_start TEXT NOT NULL,
            coverage_end TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            payload BLOB NOT NULL,
            payload_bytes INTEGER NOT NULL,
            snapshot_sha256 TEXT NOT NULL,
            format_version INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_batch_id, manifest_sha256, page_index)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot table existence")?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot",
        "format_version",
        "INTEGER NOT NULL DEFAULT 1",
    )
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_summary_archive_snapshot_manifest \
         ON summary_archive_snapshot (manifest_sha256, archive_batch_id, page_index)",
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive snapshot manifest index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot_backfill_checkpoint (
            scope TEXT PRIMARY KEY,
            next_archive_batch_id INTEGER NOT NULL DEFAULT 0,
            manifest_high_watermark_id INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot_backfill_checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot_backfill_outcome (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            disposition TEXT NOT NULL,
            failure_kind TEXT NOT NULL DEFAULT '',
            next_probe_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_batch_id, manifest_sha256)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot_backfill_outcome table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_checkpoint (
            scope TEXT PRIMARY KEY,
            live_high_watermark_id INTEGER NOT NULL,
            rollup_live_cursor INTEGER NOT NULL,
            account_rollup_live_cursor INTEGER,
            manifest_high_watermark_id INTEGER,
            durable_terminal_sequence_watermark INTEGER NOT NULL,
            global_manifest_next_id INTEGER NOT NULL DEFAULT 0,
            account_manifest_next_id INTEGER NOT NULL DEFAULT 0,
            global_manifest_complete INTEGER NOT NULL DEFAULT 0,
            account_manifest_complete INTEGER NOT NULL DEFAULT 0,
            global_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            account_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            usage_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            global_rollup_complete INTEGER NOT NULL DEFAULT 0,
            account_rollup_complete INTEGER NOT NULL DEFAULT 0,
            usage_rollup_complete INTEGER NOT NULL DEFAULT 0,
            account_unavailable INTEGER NOT NULL DEFAULT 0,
            global_usage_unavailable INTEGER NOT NULL DEFAULT 0,
            account_usage_unavailable INTEGER NOT NULL DEFAULT 0,
            global_total_count INTEGER NOT NULL DEFAULT 0,
            global_success_count INTEGER NOT NULL DEFAULT 0,
            global_failure_count INTEGER NOT NULL DEFAULT 0,
            global_total_tokens INTEGER NOT NULL DEFAULT 0,
            global_non_success_tokens INTEGER NOT NULL DEFAULT 0,
            global_total_cost REAL NOT NULL DEFAULT 0,
            global_non_success_cost REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_account_checkpoint (
            scope TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            non_success_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (scope, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_account_checkpoint table existence")?;

    for (column, definition) in [
        ("usage_rollup_next_rowid", "INTEGER NOT NULL DEFAULT 0"),
        ("usage_rollup_complete", "INTEGER NOT NULL DEFAULT 0"),
        ("global_usage_unavailable", "INTEGER NOT NULL DEFAULT 0"),
        ("account_usage_unavailable", "INTEGER NOT NULL DEFAULT 0"),
        ("global_non_success_tokens", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(
            pool,
            "summary_all_time_projection_checkpoint",
            column,
            definition,
        )
        .await?;
    }
    ensure_column_with_definition(
        pool,
        "summary_all_time_projection_account_checkpoint",
        "non_success_tokens",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_usage_checkpoint (
            scope TEXT NOT NULL,
            aggregate_scope TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL DEFAULT 0,
            normalized_model TEXT NOT NULL,
            normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_input REAL NOT NULL DEFAULT 0,
            cost_cache_write REAL NOT NULL DEFAULT 0,
            cost_cache_read REAL NOT NULL DEFAULT 0,
            cost_output REAL NOT NULL DEFAULT 0,
            cost_reasoning REAL NOT NULL DEFAULT 0,
            cost_unknown REAL NOT NULL DEFAULT 0,
            has_cost INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (
                scope,
                aggregate_scope,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_usage_checkpoint table existence")?;

    if has_existing_invocation_rollup_hourly_rows {
        let reconciliation_complete = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE((SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1), 0)",
        )
        .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
        .fetch_one(pool)
        .await?
            != 0;
        if !reconciliation_complete || added_invocation_rollup_hourly_columns {
            let reconciliation = backfill_invocation_rollup_hourly_from_sources(pool).await?;
            if reconciliation.source_complete {
                sqlx::query(
                    r#"
                    INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
                    VALUES (?1, 1)
                    ON CONFLICT(dataset) DO UPDATE SET
                        cursor_id = excluded.cursor_id,
                        updated_at = datetime('now')
                    "#,
                )
                .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
                .execute(pool)
                .await?;
            }
            info!(
                rebuilt_rows = reconciliation.applied_rollups,
                source_complete = reconciliation.source_complete,
                "backfilled invocation hourly rollups after adding aggregate columns"
            );
        }
    }

    if upstream_account_usage_hourly_needs_status_backfill {
        backfill_upstream_account_usage_hourly_status_counts(pool).await?;
    }
    if upstream_account_stats_hourly_count == 0
        || upstream_account_stats_minute_count == 0
        || added_upstream_account_stats_columns
    {
        rebuild_upstream_account_stats_rollups_from_sources(pool)
            .await
            .context("failed to rebuild upstream account stats rollups from sources")?;
    }

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
    ensure_long_term_projection_account_trigger(pool).await?;

    Ok(())
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
