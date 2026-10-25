use super::*;

const RETIRED_WEBSOCKET_TAG: &str = "unsupported_transport:websocket";

async fn seed_websocket_retirement_state(pool: &SqlitePool) -> (i64, i64) {
    ensure_schema(pool).await.expect("ensure current schema");
    sqlx::query(
        "DELETE FROM schema_refresh_migrations WHERE migration_name = 'retire_openai_websocket_proxy_v1'",
    )
    .execute(pool)
    .await
    .expect("reset WebSocket retirement migration marker");
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET openai_proxy_websocket_enabled = 1,
            openai_proxy_upstream_websocket_default_enabled = 1,
            websocket_settings_migrated = 0
        WHERE id = 1
        "#,
    )
    .execute(pool)
    .await
    .expect("seed legacy WebSocket settings");

    let websocket_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, created_at, updated_at
        )
        VALUES (?1, ?2, 1, datetime('now'), datetime('now'))
        RETURNING id
        "#,
    )
    .bind("Legacy WebSocket capability")
    .bind(RETIRED_WEBSOCKET_TAG)
    .fetch_one(pool)
    .await
    .expect("seed retired WebSocket capability tag");
    let unrelated_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, created_at, updated_at
        )
        VALUES (?1, ?2, 1, datetime('now'), datetime('now'))
        RETURNING id
        "#,
    )
    .bind("Unrelated capability")
    .bind("unsupported_model:legacy-test")
    .fetch_one(pool)
    .await
    .expect("seed unrelated system tag");
    for tag_id in [websocket_tag_id, unrelated_tag_id] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_tags (
                account_id, tag_id, created_at, updated_at
            )
            VALUES (4242, ?1, datetime('now'), datetime('now'))
            "#,
        )
        .bind(tag_id)
        .execute(pool)
        .await
        .expect("seed system tag association");
    }
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, is_mother, note, tag_ids_json,
            state, pkce_verifier, redirect_uri, status, auth_url, expires_at,
            created_at, updated_at
        ) VALUES (
            ?1, 4242, 'Legacy WebSocket session', 0, NULL, ?2,
            ?3, 'pkce-retire-websocket', 'https://example.com/callback', 'pending',
            'https://example.com/auth', ?4, ?4, ?4
        )
        "#,
    )
    .bind("retire-websocket-session")
    .bind(format!("[{websocket_tag_id},{unrelated_tag_id}]"))
    .bind("state-retire-websocket")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed OAuth session tag selection");
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, is_mother, note, tag_ids_json,
            state, pkce_verifier, redirect_uri, status, auth_url, expires_at,
            created_at, updated_at
        ) VALUES (
            ?1, 4242, 'Legacy malformed tag values', 0, NULL, ?2,
            ?3, 'pkce-retire-websocket-malformed', 'https://example.com/callback', 'pending',
            'https://example.com/auth', ?4, ?4, ?4
        )
        "#,
    )
    .bind("retire-websocket-malformed-session")
    .bind(format!(
        "[\"{websocket_tag_id}x\",{websocket_tag_id}.9,\"{websocket_tag_id}\"]"
    ))
    .bind("state-retire-websocket-malformed")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed malformed OAuth session tag values");
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, is_mother, note, tag_ids_json,
            state, pkce_verifier, redirect_uri, status, auth_url, expires_at,
            created_at, updated_at
        ) VALUES (
            ?1, 4242, 'Legacy JSON tag values', 0, NULL, ?2,
            ?3, 'pkce-retire-websocket-json-values', 'https://example.com/callback', 'pending',
            'https://example.com/auth', ?4, ?4, ?4
        )
        "#,
    )
    .bind("retire-websocket-json-values-session")
    .bind(format!(
        "[{websocket_tag_id},{{\"kind\":\"object\"}},[\"nested\",{{\"ok\":true}}],true,false,null,\"keep\"]"
    ))
    .bind("state-retire-websocket-json-values")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed non-scalar JSON tag values");
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, is_mother, note, tag_ids_json,
            state, pkce_verifier, redirect_uri, status, auth_url, expires_at,
            created_at, updated_at
        ) VALUES (
            ?1, 4242, 'Legacy scalar tag root', 0, NULL, ?2,
            ?3, 'pkce-retire-websocket-scalar', 'https://example.com/callback', 'pending',
            'https://example.com/auth', ?4, ?4, ?4
        )
        "#,
    )
    .bind("retire-websocket-scalar-session")
    .bind(websocket_tag_id.to_string())
    .bind("state-retire-websocket-scalar")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed scalar JSON tag root");
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, is_mother, note, tag_ids_json,
            state, pkce_verifier, redirect_uri, status, auth_url, expires_at,
            created_at, updated_at
        ) VALUES (
            ?1, 4242, 'Legacy object tag root', 0, NULL, ?2,
            ?3, 'pkce-retire-websocket-object', 'https://example.com/callback', 'pending',
            'https://example.com/auth', ?4, ?4, ?4
        )
        "#,
    )
    .bind("retire-websocket-object-session")
    .bind(format!(r#"{{"retired":{websocket_tag_id}}}"#))
    .bind("state-retire-websocket-object")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed object JSON tag root");
    (websocket_tag_id, unrelated_tag_id)
}

#[tokio::test]
async fn retire_websocket_proxy_migration_is_idempotent_and_preserves_unrelated_state() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    let (websocket_tag_id, unrelated_tag_id) = seed_websocket_retirement_state(&pool).await;

    ensure_schema(&pool)
        .await
        .expect("rerun schema with WebSocket retirement migration");

    let settings: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT openai_proxy_websocket_enabled,
               openai_proxy_upstream_websocket_default_enabled,
               websocket_settings_migrated
        FROM proxy_model_settings
        WHERE id = 1
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load retired WebSocket settings");
    assert_eq!(settings, (0, 0, 1));

    let websocket_tag_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
            .bind(websocket_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count retired WebSocket tag");
    assert_eq!(websocket_tag_count, 0);
    let websocket_link_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1")
            .bind(websocket_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count retired WebSocket tag links");
    assert_eq!(websocket_link_count, 0);
    let unrelated_tag_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
            .bind(unrelated_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count unrelated tag");
    assert_eq!(unrelated_tag_count, 1);
    let unrelated_link_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1")
            .bind(unrelated_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count unrelated tag links");
    assert_eq!(unrelated_link_count, 1);
    let remaining_session_tags: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load OAuth session tags after retirement");
    assert_eq!(remaining_session_tags, format!("[{unrelated_tag_id}]"));
    let malformed_session_tags: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-malformed-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load malformed OAuth session tags after retirement");
    assert_eq!(
        malformed_session_tags,
        format!("[\"{websocket_tag_id}x\",{websocket_tag_id}.9,\"{websocket_tag_id}\"]")
    );
    let json_value_session_tags: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-json-values-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load JSON tag values after retirement");
    assert_eq!(
        json_value_session_tags,
        r#"[{"kind":"object"},["nested",{"ok":true}],true,false,null,"keep"]"#
    );
    let scalar_session_tags: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-scalar-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load scalar JSON tag root after retirement");
    assert_eq!(scalar_session_tags, websocket_tag_id.to_string());
    let object_session_tags: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-object-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load object JSON tag root after retirement");
    assert_eq!(
        object_session_tags,
        format!(r#"{{"retired":{websocket_tag_id}}}"#)
    );
    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name = 'retire_openai_websocket_proxy_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("count WebSocket retirement markers");
    assert_eq!(marker_count, 1);

    let reintroduced_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (name, system_key, protected, created_at, updated_at)
        VALUES ('Reintroduced WebSocket capability', 'unsupported_transport:websocket', 1, datetime('now'), datetime('now'))
        RETURNING id
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("reintroduce retired WebSocket capability tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
        VALUES (4242, ?1, datetime('now'), datetime('now'))
        "#,
    )
    .bind(reintroduced_tag_id)
    .execute(&pool)
    .await
    .expect("reintroduce retired WebSocket capability link");

    crate::schema::retire_openai_websocket_proxy(&pool)
        .await
        .expect("rerun WebSocket proxy retirement migration");
    let rerun_settings: (i64, i64, i64) = sqlx::query_as(
        "SELECT openai_proxy_websocket_enabled, openai_proxy_upstream_websocket_default_enabled, websocket_settings_migrated FROM proxy_model_settings WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load settings after idempotent rerun");
    assert_eq!(rerun_settings, settings);
    let reintroduced_tag_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
            .bind(reintroduced_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count reintroduced WebSocket tag after rerun");
    assert_eq!(reintroduced_tag_count, 0);
}

#[tokio::test]
async fn retire_websocket_proxy_migration_rolls_back_and_reenters_after_failure() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    let (websocket_tag_id, unrelated_tag_id) = seed_websocket_retirement_state(&pool).await;
    sqlx::query(
        r#"
        CREATE TRIGGER fail_websocket_retirement_settings_update
        BEFORE UPDATE OF openai_proxy_websocket_enabled ON proxy_model_settings
        BEGIN
            SELECT RAISE(ABORT, 'forced WebSocket retirement failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("create migration failure trigger");

    let error = crate::schema::retire_openai_websocket_proxy(&pool)
        .await
        .expect_err("migration should fail through trigger");
    assert!(
        error
            .to_string()
            .contains("retire persisted WebSocket proxy settings")
    );

    sqlx::query("DROP TRIGGER fail_websocket_retirement_settings_update")
        .execute(&pool)
        .await
        .expect("drop migration failure trigger");
    let settings: (i64, i64, i64) = sqlx::query_as(
        "SELECT openai_proxy_websocket_enabled, openai_proxy_upstream_websocket_default_enabled, websocket_settings_migrated FROM proxy_model_settings WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load settings after rollback");
    assert_eq!(settings, (1, 1, 0));
    let tag_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
        .bind(websocket_tag_id)
        .fetch_one(&pool)
        .await
        .expect("count WebSocket tag after rollback");
    assert_eq!(tag_count, 1);
    let link_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1")
            .bind(websocket_tag_id)
            .fetch_one(&pool)
            .await
            .expect("count WebSocket tag link after rollback");
    assert_eq!(link_count, 1);
    let session_tags_after_rollback: String = sqlx::query_scalar(
        "SELECT tag_ids_json FROM pool_oauth_login_sessions WHERE login_id = 'retire-websocket-session'",
    )
    .fetch_one(&pool)
    .await
    .expect("load OAuth session tags after rollback");
    assert_eq!(
        session_tags_after_rollback,
        format!("[{websocket_tag_id},{unrelated_tag_id}]")
    );
    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name = 'retire_openai_websocket_proxy_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("count marker after rollback");
    assert_eq!(marker_count, 0);

    crate::schema::retire_openai_websocket_proxy(&pool)
        .await
        .expect("forward-repair migration after rollback");
    let repaired_settings: (i64, i64, i64) = sqlx::query_as(
        "SELECT openai_proxy_websocket_enabled, openai_proxy_upstream_websocket_default_enabled, websocket_settings_migrated FROM proxy_model_settings WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load settings after forward repair");
    assert_eq!(repaired_settings, (0, 0, 1));
    let repaired_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name = 'retire_openai_websocket_proxy_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("count marker after forward repair");
    assert_eq!(repaired_marker_count, 1);
}

async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    query: Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;
    crate::fetch_prompt_cache_conversations(State(state), query).await
}

#[derive(Clone, Copy)]
enum HistoricalRetentionSchema {
    V271,
    V2802,
}

fn historical_codex_invocations_schema_sql(version: HistoricalRetentionSchema) -> &'static str {
    match version {
        HistoricalRetentionSchema::V271 => {
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'xy',
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                status TEXT,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER NOT NULL DEFAULT 0,
                payload TEXT,
                raw_response TEXT NOT NULL,
                cost_estimated INTEGER NOT NULL DEFAULT 0,
                price_version TEXT,
                request_raw_path TEXT,
                request_raw_codec TEXT NOT NULL DEFAULT 'identity',
                request_raw_size INTEGER,
                request_raw_truncated INTEGER NOT NULL DEFAULT 0,
                request_raw_truncated_reason TEXT,
                response_raw_path TEXT,
                response_raw_codec TEXT NOT NULL DEFAULT 'identity',
                response_raw_size INTEGER,
                response_raw_truncated INTEGER NOT NULL DEFAULT 0,
                response_raw_truncated_reason TEXT,
                timeline_json TEXT,
                detail_level TEXT NOT NULL DEFAULT 'full',
                detail_pruned_at TEXT,
                detail_prune_reason TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                first_token_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL,
                created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
                UNIQUE(invoke_id, occurred_at)
            )
            "#
        }
        HistoricalRetentionSchema::V2802 => {
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'xy',
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                reported_cache_write_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                status TEXT,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER NOT NULL DEFAULT 0,
                payload TEXT,
                raw_response TEXT NOT NULL,
                cost_estimated INTEGER NOT NULL DEFAULT 0,
                price_version TEXT,
                request_raw_path TEXT,
                request_raw_codec TEXT NOT NULL DEFAULT 'identity',
                request_raw_size INTEGER,
                request_raw_truncated INTEGER NOT NULL DEFAULT 0,
                request_raw_truncated_reason TEXT,
                response_raw_path TEXT,
                response_raw_codec TEXT NOT NULL DEFAULT 'identity',
                response_raw_size INTEGER,
                response_raw_truncated INTEGER NOT NULL DEFAULT 0,
                response_raw_truncated_reason TEXT,
                timeline_json TEXT,
                detail_level TEXT NOT NULL DEFAULT 'full',
                detail_pruned_at TEXT,
                detail_prune_reason TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                first_token_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL,
                created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
                UNIQUE(invoke_id, occurred_at)
            )
            "#
        }
    }
}

async fn create_historical_retention_fixture(version: HistoricalRetentionSchema) -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("historical fixture sqlite");
    sqlx::query(historical_codex_invocations_schema_sql(version))
        .execute(&pool)
        .await
        .expect("create historical invocation schema");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response) VALUES ('historical-retention-row','2026-09-01 00:00:00',?1,'success',?2,'{}')",
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "historical-retention-key"}).to_string())
    .execute(&pool)
    .await
    .expect("seed historical invocation");

    if matches!(version, HistoricalRetentionSchema::V2802) {
        sqlx::query(
            "CREATE TABLE prompt_cache_conversation_stats_refresh_queue (prompt_cache_key TEXT PRIMARY KEY, enqueued_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')))"
        )
        .execute(&pool)
        .await
        .expect("create v2.80.2 refresh queue");
        sqlx::query("INSERT INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key) VALUES ('stopped-release-key')")
            .execute(&pool)
            .await
            .expect("seed stopped-release queue");
    }
    pool
}

#[tokio::test]
async fn ensure_schema_repairs_v271_and_v2802_retention_fixtures_without_historical_backfill() {
    for version in [
        HistoricalRetentionSchema::V271,
        HistoricalRetentionSchema::V2802,
    ] {
        let pool = create_historical_retention_fixture(version).await;
        let source_count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("count historical invocations before repair");

        ensure_schema(&pool)
            .await
            .expect("repair historical retention schema");
        ensure_schema(&pool)
            .await
            .expect("repeat historical retention schema repair");

        let source_count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("count historical invocations after repair");
        assert_eq!(source_count_after, source_count_before);

        let materialization_progress: (String, Option<i64>, i64) = sqlx::query_as(
            "SELECT phase,total_keys,completed_keys FROM prompt_cache_conversation_migration_progress WHERE migration_name='prompt_cache_conversations_materialization_v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load untouched materialization progress");
        assert_eq!(materialization_progress.0, "identity_backfill");
        assert_eq!(materialization_progress.1, None);
        assert_eq!(materialization_progress.2, 0);

        let conversation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
                .fetch_one(&pool)
                .await
                .expect("count conversations after repair");
        assert_eq!(conversation_count, 0);

        for table in [
            "prompt_cache_conversation_stats_generation_clock",
            "prompt_cache_conversation_stats_refresh_staging",
            "prompt_cache_conversation_orphan_cleanup_state",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("check repaired retention table");
            assert_eq!(exists, 1, "missing repaired table {table}");
        }

        sqlx::query(
            "INSERT OR REPLACE INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key,generation) VALUES ('stopped-release-key',7)",
        )
        .execute(&pool)
        .await
        .expect("seed resumable queue generation");
        sqlx::query(
            "INSERT OR REPLACE INTO prompt_cache_conversation_stats_refresh_staging (prompt_cache_key,generation,source_max_invocation_id,cursor_occurred_at,cursor_id,accumulator_json,page_size) VALUES ('stopped-release-key',7,1,'2026-09-01 00:00:00',1,'{}',64)",
        )
        .execute(&pool)
        .await
        .expect("seed resumable aggregate staging");

        ensure_schema(&pool)
            .await
            .expect("repair after interrupted retention continuation");
        let continuation: (i64, i64) = sqlx::query_as(
            "SELECT queue.generation, staging.generation FROM prompt_cache_conversation_stats_refresh_queue queue JOIN prompt_cache_conversation_stats_refresh_staging staging USING(prompt_cache_key) WHERE queue.prompt_cache_key='stopped-release-key'",
        )
        .fetch_one(&pool)
        .await
        .expect("load preserved continuation state");
        assert_eq!(continuation, (7, 7));
    }
}

#[tokio::test]
async fn ensure_schema_adds_nullable_reported_cache_write_tokens_to_legacy_invocations() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    let legacy_create_sql = codex_invocations_create_sql("codex_invocations")
        .replace("            reported_cache_write_tokens INTEGER,\n", "");
    sqlx::query(&legacy_create_sql)
        .execute(&pool)
        .await
        .expect("create pre-migration invocation schema");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, raw_response) VALUES ('legacy-usage', '2026-09-01 00:00:00', '{}')",
    )
    .execute(&pool)
    .await
    .expect("insert legacy invocation");

    ensure_schema(&pool)
        .await
        .expect("migrate legacy invocation schema");

    let reported_column = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(&pool)
        .await
        .expect("inspect invocation schema")
        .into_iter()
        .find(|row| row.get::<String, _>("name") == "reported_cache_write_tokens")
        .expect("reported cache-write column should be added");
    assert_eq!(reported_column.get::<i64, _>("notnull"), 0);
    let legacy_value: Option<i64> = sqlx::query_scalar(
        "SELECT reported_cache_write_tokens FROM codex_invocations WHERE invoke_id = 'legacy-usage'",
    )
    .fetch_one(&pool)
    .await
    .expect("load legacy reported cache-write value");
    assert_eq!(legacy_value, None);
}

#[tokio::test]
async fn ensure_schema_enables_legacy_startup_backfill_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    sqlx::query(
        r#"
        CREATE TABLE startup_backfill_progress (
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
    .execute(&pool)
    .await
    .expect("create legacy startup backfill schema");
    sqlx::query(
        "INSERT INTO startup_backfill_progress (task_name, cursor_id) \
         VALUES ('prompt_cache_conversations_materialization_v1', 12)",
    )
    .execute(&pool)
    .await
    .expect("insert legacy startup backfill progress");

    ensure_schema(&pool)
        .await
        .expect("migrate legacy startup backfill schema");

    let (enabled, cursor_id): (i64, i64) = sqlx::query_as(
        "SELECT enabled, cursor_id FROM startup_backfill_progress \
         WHERE task_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load migrated startup backfill progress");
    assert_eq!(enabled, 1);
    assert_eq!(cursor_id, 12);
}

#[tokio::test]
async fn ensure_schema_defers_prompt_cache_conversation_materialization_to_startup_backfill() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    for (invoke_id, status, tokens, cost) in [
        ("LEGACY0001", "success", 12_i64, 0.25_f64),
        ("LEGACY0002", "failed", 8_i64, 0.15_f64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-09-01 00:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(tokens)
        .bind(cost)
        .bind(json!({"promptCacheKey": "conversation-backfill-key"}).to_string())
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert legacy invocation");
    }

    ensure_schema(&pool)
        .await
        .expect("install prompt-cache conversation structure");

    let rows_after_schema: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
            .fetch_one(&pool)
            .await
            .expect("count prompt-cache conversations after schema installation");
    assert_eq!(rows_after_schema, 0);

    complete_prompt_cache_conversation_materialization_for_test(&pool).await;

    let row = sqlx::query_as::<_, (String, String, i64, i64, i64, i64, f64)>(
        r#"
        SELECT conversation_id, prompt_cache_key, request_count, success_count,
               failure_count, total_tokens, cost
        FROM prompt_cache_conversations
        WHERE prompt_cache_key = 'conversation-backfill-key'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load backfilled prompt-cache conversation");
    assert_eq!(row.0.len(), PROMPT_CACHE_CONVERSATION_ID_LENGTH);
    assert!(
        row.0
            .chars()
            .all(|character| PROXY_INVOKE_ID_ALPHABET.contains(&character))
    );
    assert_eq!(row.1, "conversation-backfill-key");
    assert_eq!(row.2, 2);
    assert_eq!(row.3, 1);
    assert_eq!(row.4, 1);
    assert_eq!(row.5, 20);
    assert!((row.6 - 0.4).abs() < 1e-9);

    let invalid_conversation_id = sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversations (
            conversation_id, prompt_cache_key
        ) VALUES ('ABC!2A', 'invalid-conversation-id-key')
        "#,
    )
    .execute(&pool)
    .await;
    assert!(invalid_conversation_id.is_err());

    sqlx::query("DELETE FROM prompt_cache_conversations")
        .execute(&pool)
        .await
        .expect("remove backfilled identity to exercise marker recovery");
    ensure_schema(&pool)
        .await
        .expect("rerun prompt-cache conversation structure migration");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;
    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key = 'conversation-backfill-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("count backfilled prompt-cache conversations");
    assert_eq!(row_count, 1);
}
