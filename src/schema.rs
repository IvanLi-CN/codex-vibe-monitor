use super::*;

pub(crate) static ENSURE_SCHEMA_LOCKS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) const INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS: i64 = 300;
pub(crate) const SHANGHAI_NOW_SQL: &str = "datetime('now', '+8 hours')";

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
        END
        "#,
        upsert_sql = invocation_in_progress_live_upsert_sql("NEW"),
        refresh_old_sql = update_old_refresh_sql,
        refresh_new_sql = update_new_refresh_sql,
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
            "forward_proxy_key" => "forward_proxy_key",
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
    let already_compatible = normalized_sql.contains("'none'")
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
    if already_compatible {
        return Ok(());
    }
    let existing_columns = prompt_cache_conversation_bindings_existing_columns(pool).await?;
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
            available_models_json,
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
            NULL,
            NULL,
            NULL,
            NULL,
            {forward_proxy_key_copy},
            NULL,
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

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let schema_lock = ensure_schema_lock(pool);
    let _schema_guard = schema_lock.lock_owned().await;

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
    .execute(pool)
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
    .execute(pool)
    .await
    .context("failed to backfill codex_invocations response_raw_codec")?;

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

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_occurred_at")
        .execute(pool)
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
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;

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

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at")
        .execute(pool)
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
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at")?;

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

    for trigger_name in [
        "trg_codex_invocations_live_insert",
        "trg_codex_invocations_live_update",
        "trg_codex_invocations_live_delete",
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(pool)
            .await
            .with_context(|| format!("failed to drop stale trigger {trigger_name}"))?;
    }

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
    sqlx::query(&prompt_cache_insert_trigger_sql)
        .execute(pool)
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_insert",
        )?;

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
    sqlx::query(&prompt_cache_update_trigger_sql)
        .execute(pool)
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_update",
        )?;

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
    sqlx::query(&prompt_cache_delete_trigger_sql)
        .execute(pool)
        .await
        .context(
            "failed to ensure trigger trg_codex_invocations_prompt_cache_working_set_delete",
        )?;

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
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            archive_expires_at TEXT,
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
        ("day_key", "TEXT"),
        ("part_key", "TEXT"),
        ("layout", "TEXT NOT NULL DEFAULT 'legacy_month'"),
        ("codec", "TEXT NOT NULL DEFAULT 'gzip'"),
        ("writer_version", "TEXT NOT NULL DEFAULT 'legacy_month_v1'"),
        ("cleanup_state", "TEXT NOT NULL DEFAULT 'active'"),
        ("superseded_by", "INTEGER"),
        ("coverage_start_at", "TEXT"),
        ("coverage_end_at", "TEXT"),
        ("archive_expires_at", "TEXT"),
        ("upstream_activity_manifest_refreshed_at", "TEXT"),
        ("historical_rollups_materialized_at", "TEXT"),
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
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_month
        ON archive_batches (dataset, month_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_month")?;

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
            total_tokens INTEGER NOT NULL,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
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
    let mut added_invocation_rollup_rebuild_columns = false;
    for (column, ty) in [
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
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
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_invocation_rollup_rebuild_columns = true;
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
    if added_invocation_rollup_rebuild_columns {
        let rebuilt_rows = backfill_invocation_rollup_hourly_from_sources(pool).await?;
        info!(
            rebuilt_rows,
            "backfilled invocation hourly rollups after adding aggregate columns"
        );
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
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
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
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
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
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

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

    if upstream_account_usage_hourly_needs_status_backfill {
        backfill_upstream_account_usage_hourly_status_counts(pool).await?;
    }
    if upstream_account_stats_hourly_count == 0
        || upstream_account_stats_minute_count == 0
        || added_upstream_account_stats_columns
    {
        reopen_upstream_account_stats_rollup_archives(pool).await?;
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
            endpoint TEXT NOT NULL,
            route_mode TEXT NOT NULL,
            sticky_key TEXT,
            upstream_base_url_host TEXT,
            group_name_snapshot TEXT,
            proxy_binding_key_snapshot TEXT,
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
        ("request_summary_json", "TEXT"),
        ("response_summary_json", "TEXT"),
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

    seed_default_pricing_catalog(pool).await?;
    ensure_upstream_accounts_schema(pool).await?;

    Ok(())
}
