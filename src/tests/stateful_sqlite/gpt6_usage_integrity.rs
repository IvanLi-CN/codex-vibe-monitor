use super::*;

#[tokio::test]
async fn concurrent_reported_cache_write_column_migration_is_database_serialized() {
    let temp_dir = make_temp_test_dir("reported-cache-write-concurrent-migration");
    let db_path = temp_dir.join("state.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(
        &db_url,
        std::time::Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .expect("build migration sqlite options");
    let pool_a = SqlitePoolOptions::new()
        .connect_with(connect_options.clone())
        .await
        .expect("open first migration pool");
    let pool_b = SqlitePoolOptions::new()
        .connect_with(connect_options)
        .await
        .expect("open second migration pool");
    let legacy_create_sql = codex_invocations_create_sql("codex_invocations")
        .replace("            reported_cache_write_tokens INTEGER,\n", "");
    sqlx::query(&legacy_create_sql)
        .execute(&pool_a)
        .await
        .expect("create legacy invocation schema");

    let (result_a, result_b) = tokio::join!(
        ensure_reported_cache_write_tokens_column(&pool_a),
        ensure_reported_cache_write_tokens_column(&pool_b),
    );

    result_a.expect("first concurrent migration should succeed");
    result_b.expect("second concurrent migration should succeed");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('codex_invocations') WHERE name = 'reported_cache_write_tokens'",
    )
    .fetch_one(&pool_a)
    .await
    .expect("inspect migrated invocation schema");
    assert_eq!(column_count, 1);

    pool_a.close().await;
    pool_b.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn runtime_update_preserves_reported_cache_write_when_missing_and_accepts_zero() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut initial = test_proxy_capture_record(
        "gpt6-cache-write-update-preserves-exact",
        "2026-09-24 12:00:00",
    );
    initial.status = "running".to_string();
    initial.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        reported_cache_write_tokens: Some(50),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };

    let mut tx = state.pool.begin().await.expect("begin initial usage write");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), initial.clone(), false)
        .await
        .expect("persist initial exact cache-write usage");
    tx.commit().await.expect("commit initial usage write");

    let mut update = initial.clone();
    update.usage.reported_cache_write_tokens = None;
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin omitted usage update");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), update.clone(), false)
        .await
        .expect("persist update without exact cache-write usage");
    tx.commit().await.expect("commit omitted usage update");

    let preserved = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT reported_cache_write_tokens FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&initial.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load exact cache-write usage after omitted update");
    assert_eq!(preserved, Some(50));

    update.usage.reported_cache_write_tokens = Some(0);
    let mut tx = state.pool.begin().await.expect("begin exact zero update");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), update, false)
        .await
        .expect("persist explicit zero cache-write usage");
    tx.commit().await.expect("commit exact zero update");

    let explicit_zero = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT reported_cache_write_tokens FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&initial.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load exact zero cache-write usage");
    assert_eq!(explicit_zero, Some(0));
}

#[test]
fn websocket_usage_event_accepts_cache_read_only_terminal_usage() {
    let event = parse_ws_usage_event(
        r#"{"type":"response.completed","response":{"id":"resp_cache_read_only","model":"gpt-6-sol","status":"completed","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
    );

    assert!(event.is_some());
}

#[test]
fn websocket_usage_event_accepts_partial_usage_and_usage_free_failure() {
    let partial = parse_ws_usage_event(
        r#"{"type":"response.in_progress","response":{"id":"resp_partial","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
    )
    .expect("partial websocket usage event");
    assert_eq!(partial.usage.cache_input_tokens, Some(325));
    assert!(!ws_text_event_is_terminal(
        r#"{"type":"response.in_progress"}"#
    ));

    let failed = parse_ws_usage_event(
        r#"{"type":"response.failed","response":{"id":"resp_failed","status":"failed"}}"#,
    )
    .expect("usage-free response.failed event");
    assert!(ws_text_event_is_terminal(r#"{"type":"response.failed"}"#));

    let mut accumulator = WebSocketUsageAccumulator::default();
    accumulator.update(partial.usage);
    let accumulated = accumulator.update(failed.usage);
    assert_eq!(accumulated.cache_input_tokens, Some(325));
}

#[test]
fn proxy_stream_usage_observed_accepts_cache_only_counts() {
    let response_info = ResponseCaptureInfo {
        model: Some("gpt-6-sol".to_string()),
        contains_encrypted_content: false,
        usage: ParsedUsage {
            cache_input_tokens: Some(325),
            ..ParsedUsage::default()
        },
        usage_missing_reason: None,
        service_tier: None,
        compaction_response_kind: None,
        stream_terminal_event: None,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
    };

    assert!(crate::proxy::proxy_stream_usage_observed(&response_info));

    let cache_write_only = ResponseCaptureInfo {
        usage: ParsedUsage {
            reported_cache_write_tokens: Some(50),
            ..ParsedUsage::default()
        },
        ..response_info
    };
    assert!(crate::proxy::proxy_stream_usage_observed(&cache_write_only));
}

#[test]
fn parse_stream_response_payload_cache_read_only_update_preserves_prior_usage() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens":1200,"output_tokens":40,"total_tokens":1240,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":50},"output_tokens_details":{"reasoning_tokens":10}}}}"#,
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());

    assert_eq!(parsed.usage.input_tokens, Some(1_200));
    assert_eq!(parsed.usage.output_tokens, Some(40));
    assert_eq!(parsed.usage.total_tokens, Some(1_240));
    assert_eq!(parsed.usage.cache_input_tokens, Some(325));
    assert_eq!(parsed.usage.reasoning_tokens, Some(10));
    assert_eq!(parsed.usage.reported_cache_write_tokens, Some(50));
}

#[test]
fn websocket_usage_updates_accumulate_without_clearing_reported_details() {
    let initial = parse_usage_value(&serde_json::json!({
        "input_tokens": 1200,
        "output_tokens": 40,
        "total_tokens": 1240,
        "input_tokens_details": {
            "cached_tokens": 300,
            "cache_write_tokens": 50
        },
        "output_tokens_details": {"reasoning_tokens": 10}
    }));
    let cache_read_only = parse_usage_value(&serde_json::json!({
        "input_tokens_details": {"cached_tokens": 325}
    }));
    let mut usage = WebSocketUsageAccumulator::default();
    usage.update(initial);
    let accumulated = usage.update(cache_read_only);

    assert_eq!(accumulated.input_tokens, Some(1_200));
    assert_eq!(accumulated.output_tokens, Some(40));
    assert_eq!(accumulated.total_tokens, Some(1_240));
    assert_eq!(accumulated.cache_input_tokens, Some(325));
    assert_eq!(accumulated.reported_cache_write_tokens, Some(50));
    assert_eq!(accumulated.reasoning_tokens, Some(10));

    let terminal = parse_usage_value(&serde_json::json!({
        "input_tokens": 1300,
        "output_tokens": 100,
        "total_tokens": 1400
    }));
    usage.update(terminal);
    usage.update(ParsedUsage::default());
    let interrupted = usage.snapshot();

    assert_eq!(interrupted.input_tokens, Some(1_300));
    assert_eq!(interrupted.output_tokens, Some(100));
    assert_eq!(interrupted.total_tokens, Some(1_400));
    assert_eq!(interrupted.cache_input_tokens, Some(325));
    assert_eq!(interrupted.reported_cache_write_tokens, Some(50));
    assert_eq!(interrupted.reasoning_tokens, Some(10));
}

#[test]
fn estimate_gpt_6_returns_unknown_cost_when_cache_read_exceeds_input_without_exact_cache_write() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        cache_input_tokens: Some(1_100),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(0),
        total_tokens: Some(1_100),
    };
    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-6-astra"),
        &usage,
        None,
        ProxyPricingMode::ResponseTier,
    );

    assert!(cost.is_none());
    assert!(!estimated);
}

#[tokio::test]
async fn websocket_terminal_usage_refresh_updates_invocation_and_hourly_rollup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "gpt6-websocket-terminal-usage-refresh";
    let occurred_at = "2026-09-24 12:00:00";
    let mut initial = test_proxy_capture_record(invoke_id, occurred_at);
    initial.model = Some("gpt-6-sol".to_string());
    initial.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(300),
        reasoning_tokens: Some(10),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    initial.cost = Some(0.01);
    initial.price_version = Some("openai-standard-2026-09-23".to_string());
    initial.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark websocket payload"),
    );

    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), initial, true)
        .await
        .expect("enqueue first websocket terminal");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let first_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load initial websocket terminal");

    let mut richer = test_proxy_capture_record(invoke_id, occurred_at);
    richer.model = Some("gpt-6-sol".to_string());
    richer.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(325),
        reported_cache_write_tokens: Some(50),
        reasoning_tokens: Some(10),
        total_tokens: Some(1_240),
    };
    richer.cost = Some(0.02);
    richer.price_version = Some("openai-standard-2026-09-23".to_string());
    richer.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.done"}"#.to_string(),
        )
        .expect("mark websocket payload"),
    );

    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), richer, true)
        .await
        .expect("enqueue richer websocket terminal");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let mut poorer = test_proxy_capture_record(invoke_id, occurred_at);
    poorer.model = Some("gpt-6-sol".to_string());
    poorer.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(300),
        reasoning_tokens: Some(10),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    poorer.cost = Some(0.01);
    poorer.price_version = Some("openai-standard-2026-09-23".to_string());
    poorer.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark poorer websocket payload"),
    );
    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), poorer, true)
        .await
        .expect("enqueue poorer duplicate websocket terminal");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let mut non_websocket_duplicate = test_proxy_capture_record(invoke_id, occurred_at);
    non_websocket_duplicate.usage.cache_input_tokens = Some(999);
    non_websocket_duplicate.cost = Some(0.99);
    persist_and_broadcast_proxy_capture_terminal_record(
        state.as_ref(),
        non_websocket_duplicate,
        false,
    )
    .await
    .expect("skip unrelated duplicate terminal");

    let refreshed = sqlx::query_as::<_, (i64, String, Option<i64>, Option<i64>, Option<f64>)>(
        r#"
        SELECT id, status, cache_input_tokens, reported_cache_write_tokens, cost
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load refreshed websocket terminal");
    assert_eq!(refreshed.0, first_id);
    assert_eq!(refreshed.1, "success");
    assert_eq!(refreshed.2, Some(325));
    assert_eq!(refreshed.3, Some(50));
    assert_eq!(refreshed.4, Some(0.02));

    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count terminal invocation rows");
    assert_eq!(count, 1);

    let rollup = sqlx::query_as::<_, (i64, f64)>(
        "SELECT total_tokens, total_cost FROM invocation_rollup_hourly WHERE source = 'proxy'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load refreshed proxy rollup");
    assert_eq!(rollup.0, 1_240);
    assert_f64_close(rollup.1, 0.02);
}

#[tokio::test]
async fn stale_poorer_websocket_refresh_cannot_overwrite_a_richer_commit() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "gpt6-websocket-stale-refresh";
    let occurred_at = "2026-09-24 13:00:00";
    let mut initial = test_proxy_capture_record(invoke_id, occurred_at);
    initial.model = Some("gpt-6-sol".to_string());
    initial.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    initial.cost = Some(0.01);
    initial.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark initial websocket payload"),
    );
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin initial terminal write");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), initial, false)
        .await
        .expect("persist initial websocket terminal");
    tx.commit().await.expect("commit initial terminal");

    let mut snapshot_tx = state.pool.begin().await.expect("begin stale identity read");
    let existing =
        load_persisted_invocation_identity_tx(snapshot_tx.as_mut(), invoke_id, occurred_at)
            .await
            .expect("load initial invocation identity")
            .expect("initial invocation exists");
    snapshot_tx
        .commit()
        .await
        .expect("finish stale identity read");

    let mut richer = test_proxy_capture_record(invoke_id, occurred_at);
    richer.model = Some("gpt-6-sol".to_string());
    richer.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(325),
        reported_cache_write_tokens: Some(50),
        reasoning_tokens: Some(10),
        total_tokens: Some(1_240),
    };
    richer.cost = Some(0.02);
    richer.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.done"}"#.to_string(),
        )
        .expect("mark richer websocket payload"),
    );

    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin richer terminal refresh");
    assert!(
        refresh_websocket_terminal_usage_tx(tx.as_mut(), existing.id, &existing, &richer)
            .await
            .expect("apply richer terminal refresh")
    );
    tx.commit().await.expect("commit richer terminal refresh");

    let mut poorer = test_proxy_capture_record(invoke_id, occurred_at);
    poorer.model = Some("gpt-6-sol".to_string());
    poorer.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(325),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    poorer.cost = Some(0.99);
    poorer.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark stale poorer websocket payload"),
    );
    assert!(websocket_terminal_usage_refresh_allowed(&existing, &poorer));

    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin stale terminal refresh");
    assert!(
        !refresh_websocket_terminal_usage_tx(tx.as_mut(), existing.id, &existing, &poorer)
            .await
            .expect("reject stale poorer terminal refresh")
    );
    tx.commit().await.expect("finish stale terminal refresh");

    let persisted = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<f64>)>(
        "SELECT reported_cache_write_tokens, reasoning_tokens, cost FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load rich usage after stale refresh attempt");
    assert_eq!(persisted.0, Some(50));
    assert_eq!(persisted.1, Some(10));
    assert_eq!(persisted.2, Some(0.02));
}
