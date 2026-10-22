use super::compatibility::{
    INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME, INVOCATION_RAW_CODEC_MIGRATION_NAME,
    INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    PROMPT_CACHE_WORKING_SET_TRIGGER_REFRESH_MIGRATION_NAME,
    TIMESERIES_MINUTE_PROJECTION_STARTUP_RECOVERY_BASELINE_MIGRATION_NAME,
    TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL, invocation_live_projection_objects_exist,
    record_schema_refresh_completion, record_schema_refresh_completion_in_transaction,
    schema_refresh_completed, schema_refresh_completed_in_transaction, sqlite_schema_object_exists,
    sqlite_table_exists,
};
use super::prompt_cache_working_set_triggers::rebuild_prompt_cache_working_set_live_triggers;
use super::runtime_system::legacy_raw_blob_link_seed_completed;
use super::*;

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

pub(crate) fn prompt_cache_working_set_live_refresh_sql_for_key(
    key_expr: &str,
    current_id_expr: &str,
) -> String {
    let display_status_sql = crate::api::invocation_display_status_sql();
    // Split the recent range from older in-flight rows. The old OR predicate evaluated
    // display status (including JSON) for every historical row of a large prompt key.
    // Existing live IDs cover older in-flight rows; the current source ID also covers
    // the mutation regardless of trigger order. Recheck source identity/time/display
    // semantics below so stale OLD live IDs cannot enter the refreshed result.
    let source_candidates_sql = format!(
        "SELECT * FROM codex_invocations INDEXED BY idx_codex_invocations_prompt_cache_key_occurred_at
         WHERE {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = {key_expr}
             AND occurred_at >= datetime('now', '+8 hours', '-{PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS} seconds')
         UNION ALL
         -- Recheck sparse older live candidates by their source row identity.
         SELECT * FROM codex_invocations WHERE id IN (
             SELECT invocation_id FROM invocation_in_progress_live
                 WHERE prompt_cache_key = {key_expr}
             UNION SELECT {current_id_expr}
         ) AND occurred_at < datetime('now', '+8 hours', '-{PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS} seconds')
             AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = {key_expr}"
    );
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
                FROM ({source_candidates_sql})
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
              FROM ({source_candidates_sql})
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
        source_candidates_sql = source_candidates_sql,
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

async fn prompt_cache_expression_indexes_exist(pool: &Pool<Sqlite>) -> Result<bool> {
    for index_name in [
        "idx_codex_invocations_prompt_cache_key_occurred_at",
        "idx_codex_invocations_prompt_cache_key_filter_occurred_at",
    ] {
        if !sqlite_schema_object_exists(pool, "index", index_name).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn ensure_prompt_cache_expression_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    let create_key_index_sql = r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END),
            occurred_at
        )
        "#;
    let create_key_filter_index_sql = r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.promptCacheKey') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#;

    if schema_refresh_completed(pool, PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME).await? {
        sqlx::query(create_key_index_sql)
            .execute(pool)
            .await
            .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;
        sqlx::query(create_key_filter_index_sql)
            .execute(pool)
            .await
            .context(
                "failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at",
            )?;
        return Ok(());
    }
    if prompt_cache_expression_indexes_exist(pool).await? {
        record_schema_refresh_completion(
            pool,
            PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
        )
        .await?;
        return Ok(());
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache expression index refresh")?;
    if schema_refresh_completed_in_transaction(
        &mut tx,
        PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    )
    .await?
    {
        tx.commit()
            .await
            .context("failed to commit prompt cache expression index refresh marker check")?;
        sqlx::query(create_key_index_sql)
            .execute(pool)
            .await
            .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;
        sqlx::query(create_key_filter_index_sql)
            .execute(pool)
            .await
            .context(
                "failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at",
            )?;
        return Ok(());
    }

    for index_name in [
        "idx_codex_invocations_prompt_cache_key_occurred_at",
        "idx_codex_invocations_prompt_cache_key_filter_occurred_at",
    ] {
        sqlx::query(&format!("DROP INDEX IF EXISTS {index_name}"))
            .execute(tx.as_mut())
            .await
            .with_context(|| format!("failed to drop stale {index_name}"))?;
    }
    sqlx::query(create_key_index_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;
    sqlx::query(create_key_filter_index_sql)
        .execute(tx.as_mut())
        .await
        .context(
            "failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at",
        )?;
    record_schema_refresh_completion_in_transaction(
        &mut tx,
        PROMPT_CACHE_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    )
    .await?;
    tx.commit()
        .await
        .context("failed to commit prompt cache expression index refresh")?;

    Ok(())
}

async fn ensure_invoke_id_filter_expression_index(pool: &Pool<Sqlite>) -> Result<()> {
    const INDEX_NAME: &str = "idx_codex_invocations_invoke_id_filter_id";
    const CREATE_INDEX_SQL: &str = r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_invoke_id_filter_id
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(invoke_id, '')))),
            id
        )
        "#;

    if schema_refresh_completed(
        pool,
        INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    )
    .await?
    {
        sqlx::query(CREATE_INDEX_SQL)
            .execute(pool)
            .await
            .context("failed to ensure invoke_id filter expression index")?;
        return Ok(());
    }
    if sqlite_schema_object_exists(pool, "index", INDEX_NAME).await? {
        record_schema_refresh_completion(
            pool,
            INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
        )
        .await?;
        return Ok(());
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin invoke_id filter expression index refresh")?;
    if schema_refresh_completed_in_transaction(
        &mut tx,
        INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    )
    .await?
    {
        tx.commit()
            .await
            .context("failed to commit invoke_id filter expression index marker check")?;
        sqlx::query(CREATE_INDEX_SQL)
            .execute(pool)
            .await
            .context("failed to ensure invoke_id filter expression index")?;
        return Ok(());
    }
    sqlx::query(CREATE_INDEX_SQL)
        .execute(tx.as_mut())
        .await
        .context("failed to create invoke_id filter expression index")?;
    record_schema_refresh_completion_in_transaction(
        &mut tx,
        INVOKE_ID_FILTER_EXPRESSION_INDEX_REFRESH_MIGRATION_NAME,
    )
    .await?;
    tx.commit()
        .await
        .context("failed to commit invoke_id filter expression index refresh")?;
    Ok(())
}

async fn ensure_timeseries_minute_projection_startup_recovery_baseline(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    if schema_refresh_completed(
        pool,
        TIMESERIES_MINUTE_PROJECTION_STARTUP_RECOVERY_BASELINE_MIGRATION_NAME,
    )
    .await?
    {
        return Ok(());
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin timeseries startup recovery baseline")?;
    if schema_refresh_completed_in_transaction(
        &mut tx,
        TIMESERIES_MINUTE_PROJECTION_STARTUP_RECOVERY_BASELINE_MIGRATION_NAME,
    )
    .await?
    {
        tx.commit()
            .await
            .context("failed to commit timeseries startup recovery baseline marker check")?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending, updated_at) VALUES (?1, 1, 1, datetime('now')) ON CONFLICT(consumer) DO UPDATE SET generation = MAX(timeseries_minute_projection_v2_recovery.generation, 1), invalidation_pending = 1, updated_at = excluded.updated_at",
    )
    .bind("timeseries_minute_v2")
    .execute(tx.as_mut())
    .await
    .context("failed to publish timeseries startup recovery baseline")?;
    record_schema_refresh_completion_in_transaction(
        &mut tx,
        TIMESERIES_MINUTE_PROJECTION_STARTUP_RECOVERY_BASELINE_MIGRATION_NAME,
    )
    .await?;
    tx.commit()
        .await
        .context("failed to commit timeseries startup recovery baseline")?;
    Ok(())
}

pub(super) async fn ensure_invocation_live_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_timeseries_minute_projection_startup_recovery_baseline(pool).await?;

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

    ensure_reported_cache_write_tokens_column(pool).await?;

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
    ensure_timeline_snapshot_table(pool).await?;
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

    // Retention walks the same eligibility window in occurred_at/id order. The composite key
    // keeps each bounded page on the index instead of sorting the remaining history repeatedly.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at_id
        ON codex_invocations (occurred_at, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at_id")?;

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

    // Detail retention filters on the full-detail tier and then walks the same time/id order.
    // Keep that candidate scan bounded by the relevant slice instead of scanning every row in
    // the retention window before applying the success-like status predicate.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_detail_level_occurred_at
        ON codex_invocations (detail_level, occurred_at, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_detail_level_occurred_at")?;

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

    ensure_prompt_cache_expression_indexes(pool).await?;
    ensure_invoke_id_filter_expression_index(pool).await?;

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

    ensure_prompt_cache_conversations_schema(pool).await?;

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

    let invocation_in_progress_live_existed =
        sqlite_table_exists(pool, "invocation_in_progress_live").await?;
    let prompt_cache_working_set_live_existed =
        sqlite_table_exists(pool, "prompt_cache_working_set_live").await?;

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

    let live_projection_objects_existed = invocation_live_projection_objects_exist(
        pool,
        invocation_in_progress_live_existed,
        prompt_cache_working_set_live_existed,
    )
    .await?;
    let mut live_projection_refresh_completed =
        schema_refresh_completed(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME).await?;
    if !live_projection_refresh_completed && live_projection_objects_existed {
        record_schema_refresh_completion(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME)
            .await?;
        live_projection_refresh_completed = true;
    }
    let refresh_live_projection =
        !live_projection_refresh_completed || !live_projection_objects_existed;
    if refresh_live_projection {
        rebuild_invocation_in_progress_live_triggers(pool)
            .await
            .context("failed to rebuild invocation_in_progress_live triggers")?;
        rebuild_prompt_cache_working_set_live_triggers(pool)
            .await
            .context("failed to rebuild prompt cache working set triggers")?;
        rebuild_invocation_in_progress_live_table(pool)
            .await
            .context("failed to rebuild invocation_in_progress_live table")?;
        rebuild_prompt_cache_working_set_live_table(pool)
            .await
            .context("failed to rebuild prompt_cache_working_set_live table")?;
        record_schema_refresh_completion(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME)
            .await?;
    }
    // Existing live tables and their original completion marker do not need a row rebuild.
    // Upgrade just the trigger definitions, preserving projection rows and business cursors.
    let indexed_working_set_triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'
         AND name IN ('trg_codex_invocations_prompt_cache_working_set_insert',
             'trg_codex_invocations_prompt_cache_working_set_update',
             'trg_codex_invocations_prompt_cache_working_set_delete')
         AND instr(sql, 'SELECT invocation_id FROM invocation_in_progress_live') > 0
         AND instr(sql, 'INDEXED BY idx_codex_invocations_prompt_cache_key_occurred_at') > 0
         AND instr(sql, 'NOT INDEXED') = 0",
    )
    .fetch_one(pool)
    .await?;
    if indexed_working_set_triggers != 3
        || !schema_refresh_completed(
            pool,
            PROMPT_CACHE_WORKING_SET_TRIGGER_REFRESH_MIGRATION_NAME,
        )
        .await?
    {
        rebuild_prompt_cache_working_set_live_triggers(pool)
            .await
            .context("failed to upgrade prompt cache working set trigger dependencies")?;
    }

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

    Ok(())
}

pub(crate) async fn ensure_reported_cache_write_tokens_column(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin reported cache-write schema migration")?;
    let columns =
        load_sqlite_table_columns_from_connection(tx.as_mut(), None, "codex_invocations").await?;
    if !columns.contains("reported_cache_write_tokens") {
        sqlx::query("ALTER TABLE codex_invocations ADD COLUMN reported_cache_write_tokens INTEGER")
            .execute(tx.as_mut())
            .await
            .context("failed to add column reported_cache_write_tokens")?;
    }
    tx.commit()
        .await
        .context("failed to commit reported cache-write schema migration")?;
    Ok(())
}
