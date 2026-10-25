use super::*;
#[tokio::test]
async fn ensure_schema_adds_codex_imagegen_column_without_losing_existing_binding_policies() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    sqlx::query(
        r#"
        CREATE TABLE prompt_cache_conversation_bindings (
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
                OR (binding_kind = 'group' AND group_name IS NOT NULL AND upstream_account_id IS NULL)
                OR (binding_kind = 'upstream_account' AND group_name IS NULL AND upstream_account_id IS NOT NULL)
            )
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create pre-Codex policy bindings table");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key, binding_kind, allow_switch_upstream,
            fast_mode_rewrite_mode, image_tool_rewrite_mode, available_models_json,
            forward_proxy_key, forward_proxy_keys_json
        )
        VALUES (?1, 'none', ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("pck-preserve-policy-columns")
    .bind(1_i64)
    .bind("force_remove")
    .bind("force_add")
    .bind(r#"["gpt-5.6"]"#)
    .bind("primary-egress")
    .bind(r#"["primary-egress","fallback-egress"]"#)
    .execute(&pool)
    .await
    .expect("insert pre-Codex policy binding");

    ensure_schema(&pool)
        .await
        .expect("schema migration should add only the Codex policy column");

    let row = sqlx::query_as::<_, PromptCacheBindingPolicyMigrationRow>(
        r#"
        SELECT
            allow_switch_upstream,
            fast_mode_rewrite_mode,
            image_tool_rewrite_mode,
            codex_imagegen_rewrite_mode,
            available_models_json,
            forward_proxy_key,
            forward_proxy_keys_json
        FROM prompt_cache_conversation_bindings
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind("pck-preserve-policy-columns")
    .fetch_one(&pool)
    .await
    .expect("load migrated policy binding");
    assert_eq!(row.allow_switch_upstream, Some(1));
    assert_eq!(row.fast_mode_rewrite_mode.as_deref(), Some("force_remove"));
    assert_eq!(row.image_tool_rewrite_mode.as_deref(), Some("force_add"));
    assert_eq!(row.codex_imagegen_rewrite_mode, None);
    assert_eq!(row.available_models_json.as_deref(), Some(r#"["gpt-5.6"]"#));
    assert_eq!(row.forward_proxy_key.as_deref(), Some("primary-egress"));
    assert_eq!(
        row.forward_proxy_keys_json.as_deref(),
        Some(r#"["primary-egress","fallback-egress"]"#)
    );
}

#[tokio::test]
async fn ensure_schema_preserves_prompt_cache_binding_timeouts_when_adding_policy_columns() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    sqlx::query(
        r#"
        CREATE TABLE prompt_cache_conversation_bindings (
            prompt_cache_key TEXT PRIMARY KEY,
            binding_kind TEXT NOT NULL,
            group_name TEXT,
            upstream_account_id INTEGER,
            responses_first_byte_timeout_secs INTEGER,
            compact_first_byte_timeout_secs INTEGER,
            responses_stream_timeout_secs INTEGER,
            compact_stream_timeout_secs INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy prompt cache conversation bindings table");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
        "#,
    )
    .bind("pck-timeout-policy-migration")
    .bind("none")
    .bind(181_i64)
    .bind(182_i64)
    .bind(183_i64)
    .bind(184_i64)
    .execute(&pool)
    .await
    .expect("insert legacy timeout override row");

    ensure_schema(&pool)
        .await
        .expect("schema migration should preserve timeout overrides");

    let row = sqlx::query_as::<_, PromptCacheBindingTimeoutMigrationRow>(
        r#"
        SELECT
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            allow_switch_upstream,
            codex_imagegen_rewrite_mode
        FROM prompt_cache_conversation_bindings
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind("pck-timeout-policy-migration")
    .fetch_one(&pool)
    .await
    .expect("load migrated prompt cache binding row");
    assert_eq!(row.responses_first_byte_timeout_secs, Some(181));
    assert_eq!(row.compact_first_byte_timeout_secs, Some(182));
    assert_eq!(row.responses_stream_timeout_secs, Some(183));
    assert_eq!(row.compact_stream_timeout_secs, Some(184));
    assert_eq!(row.allow_switch_upstream, None);
    assert_eq!(row.codex_imagegen_rewrite_mode, None);
}

#[tokio::test]
async fn ensure_schema_migrates_pre_timeout_prompt_cache_binding_table() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    sqlx::query(
        r#"
        CREATE TABLE prompt_cache_conversation_bindings (
            prompt_cache_key TEXT PRIMARY KEY,
            binding_kind TEXT NOT NULL,
            group_name TEXT,
            upstream_account_id INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create pre-timeout prompt cache conversation bindings table");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, NULL, datetime('now'), datetime('now'))
        "#,
    )
    .bind("pck-pre-timeout-policy-migration")
    .bind("group")
    .bind("team-a")
    .execute(&pool)
    .await
    .expect("insert pre-timeout binding row");

    ensure_schema(&pool)
        .await
        .expect("schema migration should support pre-timeout binding tables");

    let row = sqlx::query_as::<_, PromptCacheBindingTimeoutMigrationRow>(
        r#"
        SELECT
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            allow_switch_upstream,
            codex_imagegen_rewrite_mode
        FROM prompt_cache_conversation_bindings
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind("pck-pre-timeout-policy-migration")
    .fetch_one(&pool)
    .await
    .expect("load migrated pre-timeout prompt cache binding row");
    assert_eq!(row.responses_first_byte_timeout_secs, None);
    assert_eq!(row.compact_first_byte_timeout_secs, None);
    assert_eq!(row.responses_stream_timeout_secs, None);
    assert_eq!(row.compact_stream_timeout_secs, None);
    assert_eq!(row.allow_switch_upstream, None);
    assert_eq!(row.codex_imagegen_rewrite_mode, None);
}

#[tokio::test]
async fn ensure_schema_creates_prompt_cache_conversation_operation_events_table() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");

    ensure_schema(&pool)
        .await
        .expect("schema should create prompt cache operation events table");

    let columns = sqlx::query("PRAGMA table_info('prompt_cache_conversation_operation_events')")
        .fetch_all(&pool)
        .await
        .expect("inspect prompt cache operation events columns")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(columns.iter().any(|column| column == "prompt_cache_key"));
    assert!(columns.iter().any(|column| column == "info_types_json"));
    assert!(columns.iter().any(|column| column == "binding_before_json"));
    assert!(columns.iter().any(|column| column == "sticky_after_json"));
    assert!(
        columns
            .iter()
            .any(|column| column == "routing_context_json")
    );
}

#[tokio::test]
async fn ensure_schema_creates_sticky_affinity_generation_and_routing_source_storage() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");

    ensure_schema(&pool)
        .await
        .expect("schema should create sticky affinity generation and routing source storage");

    let generation_columns = sqlx::query("PRAGMA table_info('pool_sticky_route_generations')")
        .fetch_all(&pool)
        .await
        .expect("inspect sticky affinity generation columns")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(
        generation_columns
            .iter()
            .any(|column| column == "sticky_key")
    );
    assert!(
        generation_columns
            .iter()
            .any(|column| column == "generation")
    );
    assert!(
        generation_columns
            .iter()
            .any(|column| column == "updated_at")
    );

    let model_generation_columns =
        sqlx::query("PRAGMA table_info('pool_sticky_model_route_generations')")
            .fetch_all(&pool)
            .await
            .expect("inspect model sticky affinity generation columns")
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect::<Vec<_>>();
    assert!(
        model_generation_columns
            .iter()
            .any(|column| column == "last_clear_cause_attempt_public_id")
    );
    assert!(
        model_generation_columns
            .iter()
            .any(|column| column == "last_clear_cause_http_status")
    );

    let attempt_columns = sqlx::query("PRAGMA table_info('pool_upstream_request_attempts')")
        .fetch_all(&pool)
        .await
        .expect("inspect pool attempt columns")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(
        attempt_columns
            .iter()
            .any(|column| column == "routing_source")
    );
    assert!(
        attempt_columns
            .iter()
            .any(|column| column == "routing_selection_audit_json")
    );
}

#[tokio::test]
async fn ensure_schema_migrates_legacy_hourly_rollup_replay_identity_without_upgrading_unverified_markers()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("seed current schema before recreating the legacy replay table");

    sqlx::query("DROP TABLE hourly_rollup_archive_replay")
        .execute(&pool)
        .await
        .expect("drop current replay table");
    sqlx::query(
        r#"
        CREATE TABLE hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("recreate the pre-identity replay table");

    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path) \
         VALUES ('invocation_hourly', 'codex_invocations', 'legacy-replay.sqlite.gz')",
    )
    .execute(&pool)
    .await
    .expect("seed legacy replay marker without identity");

    ensure_schema(&pool)
        .await
        .expect("first startup migrates the legacy replay table without promoting it");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status) \
         VALUES ('codex_invocations', '2026-08', 'legacy-replay.sqlite.gz', 'legacy-replay-sha', 1, 'completed')",
    )
    .execute(&pool)
    .await
    .expect("seed completed manifest after the replay table migration");
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('hourly_rollup_archive_replay')")
            .fetch_all(&pool)
            .await
            .expect("inspect migrated replay columns");
    assert!(columns.iter().any(|column| column == "archive_sha256"));
    let first_start_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE target = 'invocation_hourly' AND dataset = 'codex_invocations' \
           AND file_path = 'legacy-replay.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load unverified replay identity after first startup");
    assert!(
        first_start_sha.is_none(),
        "a legacy replay marker remains unverified even when a completed manifest exists"
    );
    let repair_progress_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('hourly_rollup_archive_repair_progress')",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect archive repair progress columns");
    for column in [
        "cursor_id",
        "cursor_stale_rank",
        "cursor_month_key",
        "cursor_created_at",
    ] {
        assert!(
            repair_progress_columns.iter().any(|name| name == column),
            "missing repair cursor column {column}"
        );
    }
    let deferred_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('hourly_rollup_archive_repair_deferred')",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect deferred archive repair columns");
    for column in [
        "scope",
        "archive_id",
        "stale_rank",
        "month_key",
        "created_at",
        "file_path",
        "deferred_at",
    ] {
        assert!(
            deferred_columns.iter().any(|name| name == column),
            "missing deferred archive repair column {column}"
        );
    }
    let repair_plan = sqlx::query(
        r#"
        EXPLAIN QUERY PLAN
        SELECT id, file_path
        FROM archive_batches INDEXED BY idx_archive_batches_usage_breakdown_repair_candidates
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND historical_rollups_materialized_at IS NOT NULL
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
          AND (month_key, created_at, id) > ('2026-01', '2026-01-01 00:00:00', 0)
        ORDER BY month_key ASC, created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("explain usage breakdown repair candidate query")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>();
    assert!(
        repair_plan
            .iter()
            .any(|detail| detail.contains("idx_archive_batches_usage_breakdown_repair_candidates")),
        "usage breakdown repair should use the candidate partial index: {repair_plan:?}"
    );
    assert!(
        repair_plan.iter().any(|detail| detail.contains("SEARCH")
            && detail.contains("idx_archive_batches_usage_breakdown_repair_candidates")),
        "deep repair cursors should use an index range seek: {repair_plan:?}"
    );
    assert!(
        repair_plan
            .iter()
            .all(|detail| !detail.contains("USE TEMP B-TREE")),
        "usage breakdown repair should not require a temporary sort tree: {repair_plan:?}"
    );

    ensure_schema(&pool)
        .await
        .expect("second startup keeps the migrated replay table usable");
    let second_start_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE target = 'invocation_hourly' AND dataset = 'codex_invocations' \
           AND file_path = 'legacy-replay.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load unverified replay identity after second startup");
    assert!(
        second_start_sha.is_none(),
        "reapplying the schema must not upgrade an unverified legacy replay marker"
    );
}
