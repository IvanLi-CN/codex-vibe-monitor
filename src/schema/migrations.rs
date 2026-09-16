use super::*;

pub(crate) async fn rebuild_prompt_cache_working_set_live_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query("DELETE FROM prompt_cache_working_set_live")
        .execute(pool)
        .await
        .context("failed to clear prompt_cache_working_set_live before rebuild")?;
    let rebuild_sql = build_prompt_cache_working_set_live_rebuild_sql();
    sqlx::query(&rebuild_sql)
        .execute(pool)
        .await
        .context("failed to rebuild prompt_cache_working_set_live rows")?;
    Ok(())
}

fn build_prompt_cache_working_set_live_rebuild_sql() -> String {
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
    rebuild_sql
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
    let copies = prompt_cache_binding_copy_expressions(&existing_columns);
    migrate_prompt_cache_conversation_bindings_rows(pool, &copies).await
}

struct PromptCacheBindingCopyExpressions {
    responses_first_byte_timeout_copy: &'static str,
    compact_first_byte_timeout_copy: &'static str,
    image_first_byte_timeout_copy: &'static str,
    responses_stream_timeout_copy: &'static str,
    compact_stream_timeout_copy: &'static str,
    forward_proxy_key_copy: &'static str,
    allow_switch_upstream_copy: &'static str,
    fast_mode_rewrite_mode_copy: &'static str,
    image_tool_rewrite_mode_copy: &'static str,
    codex_imagegen_rewrite_mode_copy: &'static str,
    available_models_json_copy: &'static str,
    available_models_mode_copy: &'static str,
    forward_proxy_keys_json_copy: &'static str,
}

fn prompt_cache_binding_copy_expressions(
    existing_columns: &std::collections::HashSet<String>,
) -> PromptCacheBindingCopyExpressions {
    PromptCacheBindingCopyExpressions {
        responses_first_byte_timeout_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "responses_first_byte_timeout_secs",
        ),
        compact_first_byte_timeout_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "compact_first_byte_timeout_secs",
        ),
        image_first_byte_timeout_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "image_first_byte_timeout_secs",
        ),
        responses_stream_timeout_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "responses_stream_timeout_secs",
        ),
        compact_stream_timeout_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "compact_stream_timeout_secs",
        ),
        forward_proxy_key_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "forward_proxy_key",
        ),
        allow_switch_upstream_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "allow_switch_upstream",
        ),
        fast_mode_rewrite_mode_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "fast_mode_rewrite_mode",
        ),
        image_tool_rewrite_mode_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "image_tool_rewrite_mode",
        ),
        codex_imagegen_rewrite_mode_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "codex_imagegen_rewrite_mode",
        ),
        available_models_json_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "available_models_json",
        ),
        available_models_mode_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "available_models_mode",
        ),
        forward_proxy_keys_json_copy: prompt_cache_binding_copy_expr(
            existing_columns,
            "forward_proxy_keys_json",
        ),
    }
}

async fn migrate_prompt_cache_conversation_bindings_rows(
    pool: &Pool<Sqlite>,
    copies: &PromptCacheBindingCopyExpressions,
) -> Result<()> {
    const TEMP_TABLE: &str = "prompt_cache_conversation_bindings_v2";
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
        "#,
        responses_first_byte_timeout_copy = copies.responses_first_byte_timeout_copy,
        compact_first_byte_timeout_copy = copies.compact_first_byte_timeout_copy,
        image_first_byte_timeout_copy = copies.image_first_byte_timeout_copy,
        responses_stream_timeout_copy = copies.responses_stream_timeout_copy,
        compact_stream_timeout_copy = copies.compact_stream_timeout_copy,
        allow_switch_upstream_copy = copies.allow_switch_upstream_copy,
        fast_mode_rewrite_mode_copy = copies.fast_mode_rewrite_mode_copy,
        image_tool_rewrite_mode_copy = copies.image_tool_rewrite_mode_copy,
        codex_imagegen_rewrite_mode_copy = copies.codex_imagegen_rewrite_mode_copy,
        available_models_json_copy = copies.available_models_json_copy,
        available_models_mode_copy = copies.available_models_mode_copy,
        forward_proxy_key_copy = copies.forward_proxy_key_copy,
        forward_proxy_keys_json_copy = copies.forward_proxy_keys_json_copy,
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

    reopen_upstream_account_usage_hourly_archives(pool).await?;
    Ok(())
}

async fn reopen_upstream_account_usage_hourly_archives(pool: &Pool<Sqlite>) -> Result<()> {
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

pub(crate) async fn ensure_invocation_raw_codec_backfill(pool: &Pool<Sqlite>) -> Result<()> {
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

pub(crate) async fn prompt_cache_expression_indexes_exist(pool: &Pool<Sqlite>) -> Result<bool> {
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

pub(crate) async fn ensure_prompt_cache_expression_indexes(pool: &Pool<Sqlite>) -> Result<()> {
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
