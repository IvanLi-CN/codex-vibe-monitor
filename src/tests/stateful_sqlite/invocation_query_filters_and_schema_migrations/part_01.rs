#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_nonstream_json_error_preserves_prefixed_metadata() {
    #[derive(sqlx::FromRow)]
    struct PersistedErrorRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-prefixed-json-error");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-prefixed-json-error"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":false,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large prefixed json error response body");

    let mut row: Option<PersistedErrorRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedErrorRow>(
            r#"
            SELECT
                status,
                error_message,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large prefixed json error row");
        if row
            .as_ref()
            .and_then(|record| record.error_message.as_deref())
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large prefixed json error row should exist");
    assert_eq!(row.status.as_deref(), Some("http_400"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| message.contains("prefix metadata should survive"))
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large prefixed json error payload summary");
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
    assert!(
        payload["usageMissingReason"]
            .as_str()
            .is_some_and(|reason| reason.contains(PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED))
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
pub(crate) async fn locate_invocation_returns_account_scoped_anchor_pages() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_locator_invocations(&state.pool).await;

    for (request_id, expected_page, expected_index, expected_absolute_index) in [
        ("locate-120", 1, 0, 0),
        ("locate-060", 2, 10, 60),
        ("locate-000", 3, 20, 120),
    ] {
        let response = locate_invocation_page(
            state.clone(),
            &LocateInvocationQuery {
                invoke_id: Some(request_id.to_string()),
                request_id: Some(request_id.to_string()),
                attempt_id: None,
                upstream_account_id: Some(17),
                page_size: Some(50),
            },
        )
        .await
        .expect("locator query should succeed")
        .expect("target should exist");

        assert_eq!(response.page, expected_page);
        assert_eq!(response.page_size, 50);
        assert_eq!(response.target_index, expected_index);
        assert_eq!(response.target_absolute_index, expected_absolute_index);
        assert_eq!(response.invoke_id, request_id);
        assert_eq!(response.records[expected_index].invoke_id, request_id);
        assert!(response.records.len() <= 50);
    }

    assert_locator_snapshot_is_account_scoped(state).await;
}

async fn seed_locator_invocations(pool: &Pool<Sqlite>) {
    for index in 0..121 {
        let invoke_id = format!("locate-{index:03}");
        let occurred_at = format!("2026-03-10 08:{:02}:{:02}", index / 60, index % 60);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response
            )
            VALUES (?1, ?2, ?3, 'success', ?4, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(r#"{"upstreamAccountId":17,"upstreamAccountName":"anchor-account"}"#)
        .execute(pool)
        .await
        .expect("insert locator seed row");
    }
}

async fn assert_locator_snapshot_is_account_scoped(state: Arc<AppState>) {
    let account_mismatch = locate_invocation_page(
        state.clone(),
        &LocateInvocationQuery {
            invoke_id: Some("locate-060".to_string()),
            request_id: Some("locate-060".to_string()),
            attempt_id: None,
            upstream_account_id: Some(18),
            page_size: Some(50),
        },
    )
    .await
    .expect("account-scoped miss should not fail");
    assert!(account_mismatch.is_none());

    let anchored = locate_invocation_page(
        state.clone(),
        &LocateInvocationQuery {
            invoke_id: Some("locate-060".to_string()),
            request_id: Some("locate-060".to_string()),
            attempt_id: None,
            upstream_account_id: Some(17),
            page_size: Some(50),
        },
    )
    .await
    .expect("anchor should resolve")
    .expect("anchor should exist");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES ('locate-newer', '2026-03-10 09:00:00', ?1, 'success', ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":17,"upstreamAccountName":"anchor-account"}"#)
    .execute(&state.pool)
    .await
    .expect("insert post-snapshot row");
    let Json(snapshot_page) = list_invocations(
        State(state),
        Query(ListQuery {
            upstream_account_id: Some(17),
            page: Some(anchored.page),
            page_size: Some(anchored.page_size),
            snapshot_id: Some(anchored.snapshot_id),
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("snapshot page should remain readable");
    assert_eq!(snapshot_page.total, 121);
    assert!(
        snapshot_page
            .records
            .iter()
            .all(|record| record.invoke_id != "locate-newer")
    );
}

#[tokio::test]
pub(crate) async fn invocation_queries_support_short_invoke_and_attempt_ids() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_short_id_invocations(&state.pool).await;

    assert_short_id_queries(state).await;
}

async fn seed_short_id_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, occurred_at, model) in [
        ("invoke-short-a", "2026-03-10 08:00:00", "gpt-5.4"),
        ("invoke-short-b", "2026-03-10 08:01:00", "gpt-5.4-mini"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, 'success', ?5, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind(r#"{"upstreamAccountId":17,"upstreamAccountName":"anchor-account"}"#)
        .execute(pool)
        .await
        .expect("insert short-id invocation row");
    }

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            created_at
        )
        VALUES (
            ?1, ?2, ?3, '/v1/responses', ?4, 'pck-short-a', 17, 'route-short-a', 1, 1, 0,
            '203.0.113.10', ?3, ?3, 'success', 'completed', datetime('now')
        )
        "#,
    )
    .bind("4V7MYPJG")
    .bind("invoke-short-a")
    .bind("2026-03-10 08:00:00")
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .execute(pool)
    .await
    .expect("insert short attempt row");
}

async fn assert_short_id_queries(state: Arc<AppState>) {
    let Json(invoke_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            invoke_id: Some("invoke-short-a".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("invokeId list query should succeed");
    assert_eq!(invoke_filtered.total, 1);
    assert_eq!(invoke_filtered.records[0].invoke_id, "invoke-short-a");

    let Json(attempt_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            attempt_id: Some("4V7MYPJG".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("attemptId list query should succeed");
    assert_eq!(attempt_filtered.total, 1);
    assert_eq!(attempt_filtered.records[0].invoke_id, "invoke-short-a");

    let Json(summary) = fetch_invocation_summary(
        State(state.clone()),
        Query(ListQuery {
            attempt_id: Some("4V7MYPJG".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("attemptId summary query should succeed");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);

    let Json(suggestions) = fetch_invocation_suggestions(
        State(state.clone()),
        Query(ListQuery {
            attempt_id: Some("4V7MYPJG".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("attemptId suggestions query should succeed");
    assert_eq!(suggestions.model.items.len(), 1);
    assert_eq!(suggestions.model.items[0].value, "gpt-5.4");
    assert_eq!(suggestions.model.items[0].count, 1);

    let Json(mismatch) = list_invocations(
        State(state),
        Query(ListQuery {
            invoke_id: Some("invoke-short-b".to_string()),
            attempt_id: Some("4V7MYPJG".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("mismatched invokeId + attemptId query should succeed");
    assert_eq!(mismatch.total, 0);
    assert!(mismatch.records.is_empty());
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_nonstream_usage_survives_response_raw_truncation() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-compact-truncated-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(96);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_truncated",
        "input": [{ "role": "user", "content": "compact this thread" }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact proxy response body");

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_path,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query truncated compact capture row");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("truncated compact capture row should exist");
    assert_truncated_compact_row(&row);

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[derive(sqlx::FromRow)]
struct PersistedCompactRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    response_raw_truncated: i64,
    response_raw_truncated_reason: Option<String>,
    payload: Option<String>,
}

fn assert_truncated_compact_row(row: &PersistedCompactRow) {
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(139));
    assert_eq!(row.output_tokens, Some(438));
    assert_eq!(row.total_tokens, Some(577));
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );
    assert!(row.response_raw_size.is_some_and(|size| size > 96));

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read truncated compact raw response");
    assert!(raw_bytes.len() <= 96);

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert!(payload["usageMissingReason"].is_null());
}

#[tokio::test]
pub(crate) async fn resolve_default_source_scope_always_all() {
    let pool = test_current_schema_pool().await;

    let scope_before = resolve_default_source_scope(&pool)
        .await
        .expect("scope before insert");
    assert_eq!(scope_before, InvocationSourceScope::All);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, raw_response
        )
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind("proxy-test-1")
    .bind("2026-02-22 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert proxy invocation");

    let scope_after = resolve_default_source_scope(&pool)
        .await
        .expect("scope after insert");
    assert_eq!(scope_after, InvocationSourceScope::All);
}

#[derive(sqlx::FromRow)]
pub(crate) struct PromptCacheBindingTimeoutMigrationRow {
    responses_first_byte_timeout_secs: Option<i64>,
    compact_first_byte_timeout_secs: Option<i64>,
    responses_stream_timeout_secs: Option<i64>,
    compact_stream_timeout_secs: Option<i64>,
    allow_switch_upstream: Option<i64>,
    codex_imagegen_rewrite_mode: Option<String>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct PromptCacheBindingPolicyMigrationRow {
    allow_switch_upstream: Option<i64>,
    fast_mode_rewrite_mode: Option<String>,
    image_tool_rewrite_mode: Option<String>,
    codex_imagegen_rewrite_mode: Option<String>,
    available_models_json: Option<String>,
    forward_proxy_key: Option<String>,
    forward_proxy_keys_json: Option<String>,
}

#[tokio::test]
pub(crate) async fn ensure_schema_adds_codex_imagegen_column_without_losing_existing_binding_policies()
 {
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
pub(crate) async fn ensure_schema_preserves_prompt_cache_binding_timeouts_when_adding_policy_columns()
 {
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
pub(crate) async fn ensure_schema_migrates_pre_timeout_prompt_cache_binding_table() {
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
pub(crate) async fn ensure_schema_creates_prompt_cache_conversation_operation_events_table() {
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
pub(crate) async fn ensure_schema_creates_sticky_affinity_generation_and_routing_source_storage() {
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
pub(crate) async fn ensure_schema_migrates_legacy_hourly_rollup_replay_identity_without_upgrading_unverified_markers()
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

use super::*;
