#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_fast_mode() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Fast Mode Merge").await;

    let mut fill_missing_rule = test_tag_routing_rule();
    fill_missing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
    let fill_missing_tag = insert_test_tag(&pool, "fast-fill", &fill_missing_rule)
        .await
        .expect("insert fill-missing tag");

    let mut force_remove_rule = test_tag_routing_rule();
    force_remove_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;
    let force_remove_tag = insert_test_tag(&pool, "fast-remove", &force_remove_rule)
        .await
        .expect("insert force-remove tag");

    sync_account_tag_links(
        &pool,
        account_id,
        &[fill_missing_tag.summary.id, force_remove_tag.summary.id],
    )
    .await
    .expect("attach fast-mode tags");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
}

#[tokio::test]
pub(crate) async fn list_tags_only_returns_system_tags_and_disables_writes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "System Tag Account").await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let custom_tag_id = insert_legacy_custom_tag(
        &state.pool,
        "fast-mode-round-trip",
        &test_tag_routing_rule(),
    )
    .await;

    let Json(listed) = list_tags(
        State(state.clone()),
        Query(ListTagsQuery {
            search: None,
            has_accounts: None,
            allow_cut_in: None,
            allow_cut_out: None,
        }),
    )
    .await
    .expect("list tags");
    assert!(!listed.writes_enabled);
    assert!(
        listed
            .items
            .iter()
            .all(|item| item.system_key.as_deref().is_some())
    );
    assert!(
        listed
            .items
            .iter()
            .any(|item| item.system_key.as_deref() == Some("unsupported_model:gpt-5.5"))
    );
    assert!(listed.items.iter().all(|item| item.id != custom_tag_id));
}

#[tokio::test]
pub(crate) async fn sticky_key_preview_uses_the_final_attempt_request_compression() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Sticky compression preview").await;
    let occurred_at = format_utc_iso(Utc::now());
    let payload = json!({
        "stickyKey": "sticky-compression",
        "upstreamAccountId": account_id,
        "requestCompressionAlgorithm": "gzip",
    })
    .to_string();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (invoke_id, occurred_at, source, payload, raw_response)
        VALUES ('sticky-compression-invocation', ?1, 'proxy', ?2, '')
        "#,
    )
    .bind(&occurred_at)
    .bind(&payload)
    .execute(&state.pool)
    .await
    .expect("insert sticky invocation");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            upstream_account_id, attempt_index, distinct_account_index, same_account_retry_index,
            started_at, finished_at, status, phase, http_status,
            upstream_request_compression_algorithm, created_at
        )
        VALUES (
            'STICKYCOMP1', 'sticky-compression-invocation', ?1, '/v1/responses', 'pool',
            'sticky-compression', ?2, 1, 1, 0,
            ?1, ?1, 'success', 'completed', 200,
            'br', ?1
        ), (
            'STICKYCOMP2', 'sticky-compression-invocation', ?1, '/v1/responses', 'pool',
            'sticky-compression', ?2, 2, 1, 1,
            ?1, ?1, 'success', 'completed', 200,
            'zstd', ?1
        )
        "#,
    )
    .bind(&occurred_at)
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert sticky attempts");

    let rows = query_account_sticky_key_recent_invocations(
        &state.pool,
        account_id,
        &["sticky-compression".to_string()],
        5,
        None,
    )
    .await
    .expect("query sticky preview");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].request_compression_algorithm.as_deref(),
        Some("zstd")
    );

    upsert_sticky_route(&state.pool, "sticky-compression", account_id, &occurred_at)
        .await
        .expect("upsert sticky route");
    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(5),
    )
    .await
    .expect("build sticky response");

    assert_eq!(
        response.conversations[0].recent_invocations[0]
            .request_compression_algorithm
            .as_deref(),
        Some("zstd")
    );
}

#[tokio::test]
pub(crate) async fn sticky_key_recent_preview_uses_compatible_composite_index_without_sql_sort() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Sticky preview index").await;
    let index_sql = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sql
        FROM sqlite_master
        WHERE type = 'index'
          AND name = 'idx_codex_invocations_account_sticky_key_recent'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load sticky preview index");
    assert!(index_sql.contains("$.upstreamAccountId"));
    assert!(index_sql.contains("$.stickyKey"));
    assert!(index_sql.contains("$.promptCacheKey"));
    assert!(index_sql.contains("occurred_at DESC"));
    assert!(index_sql.contains("id DESC"));

    let explain_keys = vec!["sticky-primary".to_string(), "sticky-fallback".to_string()];
    let explain_query =
        build_account_sticky_key_recent_invocations_query(account_id, &explain_keys, 5, None);
    let explain_sql = explain_query.sql().to_string();
    let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {explain_sql}"))
        .bind(account_id)
        .bind("sticky-primary")
        .bind("sticky-fallback")
        .bind(5_i64)
        .fetch_all(&state.pool)
        .await
        .expect("load sticky preview explain plan")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan.contains("idx_codex_invocations_account_sticky_key_recent"),
        "unexpected sticky preview plan: {plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "sticky preview should not sort in SQLite: {plan}"
    );

    let windowed_query = build_account_sticky_key_recent_invocations_query(
        account_id,
        &explain_keys,
        5,
        Some("2026-08-20 10:00:00"),
    );
    let windowed_sql = windowed_query.sql().to_string();
    let windowed_plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {windowed_sql}"))
        .bind(account_id)
        .bind("2026-08-20 10:00:00")
        .bind("sticky-primary")
        .bind("sticky-fallback")
        .bind(5_i64)
        .fetch_all(&state.pool)
        .await
        .expect("load windowed sticky preview explain plan")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        windowed_plan.contains("idx_codex_invocations_account_sticky_key_recent"),
        "unexpected windowed sticky preview plan: {windowed_plan}"
    );
    assert!(
        !windowed_plan.contains("USE TEMP B-TREE"),
        "windowed sticky preview should not sort in SQLite: {windowed_plan}"
    );

    assert_sticky_preview_rows(&state.pool, account_id).await;
}

async fn assert_sticky_preview_rows(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (invoke_id, occurred_at, source, payload, raw_response)
        VALUES
            ('sticky-primary-old', '2026-08-20 10:00:00', 'proxy', ?1, ''),
            ('sticky-primary-tie-old', '2026-08-20 10:01:00', 'proxy', ?1, ''),
            ('sticky-primary-tie-new', '2026-08-20 10:01:00', 'proxy', ?1, ''),
            ('sticky-primary-new', '2026-08-20 10:02:00', 'proxy', ?1, ''),
            ('sticky-fallback-new', '2026-08-20 10:03:00', 'proxy', ?2, '')
        "#,
    )
    .bind(
        json!({
            "upstreamAccountId": account_id,
            "stickyKey": "sticky-primary",
        })
        .to_string(),
    )
    .bind(
        json!({
            "upstreamAccountId": account_id,
            "promptCacheKey": "sticky-fallback",
        })
        .to_string(),
    )
    .execute(pool)
    .await
    .expect("insert sticky preview invocations");

    let rows = query_account_sticky_key_recent_invocations(
        pool,
        account_id,
        &["sticky-primary".to_string(), "sticky-fallback".to_string()],
        3,
        None,
    )
    .await
    .expect("load indexed sticky preview");
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].sticky_key, "sticky-fallback");
    assert_eq!(rows[0].invoke_id, "sticky-fallback-new");
    assert_eq!(rows[1].sticky_key, "sticky-primary");
    assert_eq!(rows[1].invoke_id, "sticky-primary-new");
    assert_eq!(rows[2].invoke_id, "sticky-primary-tie-new");
    assert_eq!(rows[3].invoke_id, "sticky-primary-tie-old");
}

#[tokio::test]
#[ignore = "manual benchmark: measures indexed sticky previews on an isolated 300k fixture"]
pub(crate) async fn benchmark_sticky_key_recent_preview_on_three_hundred_thousand_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open sticky preview benchmark sqlite");
    create_sticky_preview_benchmark_schema(&pool).await;
    let account_id = 42_i64;
    seed_sticky_preview_benchmark_rows(&pool, account_id).await;
    let selected_keys = vec![
        "sticky-00".to_string(),
        "sticky-02".to_string(),
        "sticky-04".to_string(),
    ];
    let explain_query =
        build_account_sticky_key_recent_invocations_query(account_id, &selected_keys, 5, None);
    let explain_sql = explain_query.sql().to_string();
    let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {explain_sql}"))
        .bind(account_id)
        .bind("sticky-00")
        .bind("sticky-02")
        .bind("sticky-04")
        .bind(5_i64)
        .fetch_all(&pool)
        .await
        .expect("load 300k sticky preview explain plan")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan.contains("idx_codex_invocations_account_sticky_key_recent"),
        "unexpected 300k sticky preview plan: {plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "300k sticky preview should not sort in SQLite: {plan}"
    );

    let rows =
        query_account_sticky_key_recent_invocations(&pool, account_id, &selected_keys, 5, None)
            .await
            .expect("warm indexed sticky preview");
    assert_eq!(rows.len(), 15);

    let started_at = std::time::Instant::now();
    let rows =
        query_account_sticky_key_recent_invocations(&pool, account_id, &selected_keys, 5, None)
            .await
            .expect("load indexed sticky preview");
    let elapsed = started_at.elapsed();

    assert_eq!(rows.len(), 15);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "indexed sticky preview exceeded one second on the 300k-row fixture: {elapsed:?}"
    );
    eprintln!("indexed 300k sticky preview completed in {elapsed:?}");
}

async fn create_sticky_preview_benchmark_schema(pool: &SqlitePool) {
    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY,
            invoke_id TEXT,
            occurred_at TEXT NOT NULL,
            status TEXT,
            failure_class TEXT,
            failure_kind TEXT,
            error_message TEXT,
            model TEXT,
            total_tokens INTEGER,
            cost REAL,
            source TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_input_tokens INTEGER,
            reasoning_tokens INTEGER,
            t_req_read_ms REAL,
            t_req_parse_ms REAL,
            t_upstream_connect_ms REAL,
            t_upstream_ttfb_ms REAL,
            first_token_ms REAL,
            t_upstream_stream_ms REAL,
            t_resp_parse_ms REAL,
            t_persist_ms REAL,
            t_total_ms REAL,
            payload TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark table");
    sqlx::query(
        r#"
        CREATE TABLE pool_upstream_request_attempts (
            id INTEGER PRIMARY KEY,
            invoke_id TEXT,
            occurred_at TEXT,
            status TEXT,
            upstream_request_compression_algorithm TEXT,
            attempt_index INTEGER
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark attempts table");
    sqlx::query(
        r#"
        CREATE INDEX idx_pool_upstream_request_attempts_invoke_attempt
        ON pool_upstream_request_attempts (invoke_id, attempt_index)
        "#,
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark attempts index");
}

async fn seed_sticky_preview_benchmark_rows(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        r#"
        WITH RECURSIVE
            thousands(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM thousands WHERE value < 999
            ),
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 299
            )
        INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, payload)
        SELECT
            thousands.value * 300 + hundreds.value + 1,
            printf('sticky-benchmark-%06d', thousands.value * 300 + hundreds.value + 1),
            datetime('2026-08-20 00:00:00', printf('+%d seconds', thousands.value * 300 + hundreds.value)),
            'proxy',
            json_object(
                'upstreamAccountId', CASE WHEN (thousands.value * 300 + hundreds.value) % 10 = 0 THEN ?1 ELSE ?1 + 1 END,
                CASE WHEN (thousands.value * 300 + hundreds.value) % 20 = 0 THEN 'stickyKey' ELSE 'promptCacheKey' END,
                printf('sticky-%02d', (thousands.value * 300 + hundreds.value) % 12)
            )
        FROM thousands
        CROSS JOIN hundreds
        "#,
    )
    .bind(account_id)
    .execute(pool)
    .await
    .expect("seed 300k sticky preview fixture");
    sqlx::query(
        r#"
        CREATE INDEX idx_codex_invocations_account_sticky_key_recent
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
    .expect("create sticky preview benchmark index");
}

use super::*;
