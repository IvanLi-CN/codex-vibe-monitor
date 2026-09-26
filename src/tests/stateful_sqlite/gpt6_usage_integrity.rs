use super::*;

#[test]
fn estimate_proxy_cost_rejects_invalid_gpt_6_variants_even_with_exact_custom_prices() {
    let mut catalog = default_pricing_catalog();
    for model in [
        "gpt-6-astra-2026-02-29",
        "gpt-6-astra-preview",
        "gpt-6-terra-2026-02-29",
        "gpt-6-terra-preview",
    ] {
        catalog.models.insert(
            model.to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        );
    }
    let usage = ParsedUsage {
        input_tokens: Some(100),
        output_tokens: Some(20),
        total_tokens: Some(120),
        ..ParsedUsage::default()
    };

    for model in [
        "gpt-6-astra-2026-02-29",
        "gpt-6-astra-preview",
        "gpt-6-terra-2026-02-29",
        "gpt-6-terra-preview",
    ] {
        let (cost, estimated, _) = estimate_proxy_cost(
            &catalog,
            Some(model),
            &usage,
            None,
            ProxyPricingMode::ResponseTier,
        );
        assert!(cost.is_none(), "{model} should remain unpriced");
        assert!(!estimated, "{model} should not be estimated");
    }

    let (terra_cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-6-terra"),
        &usage,
        None,
        ProxyPricingMode::ResponseTier,
    );
    assert!(
        estimated,
        "the exact Terra compatibility row remains usable"
    );
    assert!(terra_cost.is_some());
}

#[test]
fn gpt_6_api_key_uses_actual_response_tier_over_request_hint() {
    let (tier, mode) = resolve_proxy_billing_service_tier_and_pricing_mode_for_model(
        Some("gpt-6-astra"),
        None,
        Some("priority"),
        Some("fast"),
        Some("api_key_codex"),
    );
    assert_eq!(tier.as_deref(), Some("fast"));
    assert_eq!(mode, ProxyPricingMode::ResponseTier);

    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        total_tokens: Some(1_100),
        ..ParsedUsage::default()
    };
    let (standard_cost, _, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        Some("standard"),
        ProxyPricingMode::ResponseTier,
    );
    let (fast_cost, _, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        tier.as_deref(),
        mode,
    );
    assert!(
        (fast_cost.expect("Fast response has a price")
            - standard_cost.expect("Standard response has a price") * 2.0)
            .abs()
            < 1e-12
    );
}

#[test]
fn gpt_6_api_key_request_tier_hint_without_response_tier_stays_standard() {
    let (tier, mode) = resolve_proxy_billing_service_tier_and_pricing_mode_for_model(
        Some("gpt-6-astra"),
        None,
        Some("priority"),
        None,
        Some("api_key_codex"),
    );
    assert_eq!(tier.as_deref(), Some("priority"));
    assert_eq!(mode, ProxyPricingMode::RequestedTier);

    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        total_tokens: Some(1_100),
        ..ParsedUsage::default()
    };
    let (hinted_cost, _, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        tier.as_deref(),
        mode,
    );
    let (standard_cost, _, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        Some("standard"),
        ProxyPricingMode::ResponseTier,
    );
    assert!(
        (hinted_cost.expect("request hint has a price")
            - standard_cost.expect("Standard response has a price"))
        .abs()
            < 1e-12
    );
}

#[test]
fn gpt_6_default_actual_tier_does_not_fall_back_to_standard() {
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        total_tokens: Some(1_100),
        ..ParsedUsage::default()
    };
    let (cost, estimated, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    assert!(
        cost.is_none(),
        "unsupported actual tier must have unknown cost"
    );
    assert!(!estimated, "unsupported actual tier must not be estimated");
}

#[test]
fn estimate_gpt_6_returns_unknown_cost_for_negative_output_tokens() {
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(-1),
        total_tokens: Some(999),
        ..ParsedUsage::default()
    };
    let (cost, estimated, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        None,
        ProxyPricingMode::ResponseTier,
    );

    assert!(cost.is_none(), "negative output must have unknown cost");
    assert!(!estimated, "negative output must not be estimated");
}

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
async fn prepared_archive_identity_versions_preserve_exact_cache_write_guard() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let record = test_proxy_capture_record(
        "gpt6-prepared-identity-version-guard",
        "2026-09-24 12:00:00",
    );
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin prepared identity fixture write");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, false)
        .await
        .expect("persist prepared identity fixture row");
    tx.commit().await.expect("commit prepared identity fixture");
    let row_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id = 'gpt6-prepared-identity-version-guard'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load prepared identity row id");

    let mut connection = state
        .pool
        .acquire()
        .await
        .expect("acquire identity connection");
    let candidate_identity = invocation_archive_source_identity_sha256_candidate_v2_for_test(
        connection.as_mut(),
        crate::maintenance::InvocationArchiveIdentityDatabase::Main,
        &[row_id],
    )
    .await
    .expect("calculate candidate v2 identity");
    assert!(
        invocation_archive_source_identity_matches_for_test(
            connection.as_mut(),
            crate::maintenance::InvocationArchiveIdentityDatabase::Main,
            &[row_id],
            &candidate_identity,
        )
        .await
        .expect("verify candidate v2 identity")
    );
    drop(connection);

    sqlx::query("UPDATE codex_invocations SET reported_cache_write_tokens = 13 WHERE id = ?1")
        .bind(row_id)
        .execute(&state.pool)
        .await
        .expect("set exact cache-write value");

    let mut connection = state
        .pool
        .acquire()
        .await
        .expect("reacquire identity connection");
    let candidate_identity = invocation_archive_source_identity_sha256_candidate_v2_for_test(
        connection.as_mut(),
        crate::maintenance::InvocationArchiveIdentityDatabase::Main,
        &[row_id],
    )
    .await
    .expect("calculate candidate v2 identity with exact usage");
    assert!(
        invocation_archive_source_identity_matches_for_test(
            connection.as_mut(),
            crate::maintenance::InvocationArchiveIdentityDatabase::Main,
            &[row_id],
            &candidate_identity,
        )
        .await
        .expect("verify candidate v2 identity with exact usage")
    );

    let legacy_identity = invocation_archive_source_identity_sha256_legacy_for_test(
        connection.as_mut(),
        crate::maintenance::InvocationArchiveIdentityDatabase::Main,
        &[row_id],
    )
    .await
    .expect("calculate pre-column v2 identity");
    assert!(
        !invocation_archive_source_identity_matches_for_test(
            connection.as_mut(),
            crate::maintenance::InvocationArchiveIdentityDatabase::Main,
            &[row_id],
            &legacy_identity,
        )
        .await
        .expect("verify pre-column v2 identity with non-null exact usage")
    );
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
async fn websocket_terminal_insert_race_rebuilds_existing_hourly_rollup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "gpt6-websocket-insert-race-rollup";
    let occurred_at = "2026-09-24 12:00:00";
    let mut record = test_proxy_capture_record(invoke_id, occurred_at);
    record.model = Some("gpt-6-sol".to_string());
    record.usage = ParsedUsage {
        input_tokens: Some(200),
        output_tokens: Some(20),
        cache_input_tokens: Some(10),
        reported_cache_write_tokens: Some(7),
        total_tokens: Some(220),
        ..ParsedUsage::default()
    };
    record.cost = Some(0.02);
    record.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark websocket payload"),
    );

    let bucket_start_epoch = crate::maintenance::invocation_bucket_start_epoch(occurred_at)
        .expect("resolve hourly rollup bucket");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete,
            total_tokens, total_cost
        ) VALUES (?1, 'proxy', 1, 1, 0, 1, 110, 0.01, 1, 110, 0.01)
        "#,
    )
    .bind(bucket_start_epoch)
    .execute(&state.pool)
    .await
    .expect("seed the pre-race rollup");
    sqlx::query(
        r#"
        CREATE TRIGGER seed_websocket_insert_race
        BEFORE INSERT ON codex_invocations
        WHEN NEW.invoke_id = 'gpt6-websocket-insert-race-rollup'
             AND NEW.input_tokens > 100
        BEGIN
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, model, input_tokens, output_tokens,
                cache_input_tokens, reasoning_tokens, total_tokens, cost, status,
                error_message, failure_kind, failure_class, is_actionable, payload,
                raw_response, price_version
            ) VALUES (
                NEW.invoke_id, NEW.occurred_at, NEW.source, NEW.model, 100, 10,
                5, NULL, 110, 0.01, NEW.status, NEW.error_message, NEW.failure_kind,
                NEW.failure_class, NEW.is_actionable, NEW.payload, NEW.raw_response,
                NEW.price_version
            );
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install deterministic insert-race trigger");

    let mut tx = state.pool.begin().await.expect("begin terminal write");
    persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, true)
        .await
        .expect("persist terminal invocation after insert race");
    tx.commit().await.expect("commit terminal write");

    let persisted_usage = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT input_tokens, reported_cache_write_tokens FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation refreshed after insert race");
    assert_eq!(persisted_usage, (Some(200), Some(7)));

    let rollup = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT total_count, total_tokens, total_cost FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = 'proxy'",
    )
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load rebuilt hourly rollup");
    assert_eq!(rollup.0, 1);
    assert_eq!(rollup.1, 220);
    assert_f64_close(rollup.2, 0.02);
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
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: Some(60),
        reasoning_tokens: Some(11),
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

    let persisted = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<f64>)>(
        "SELECT cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, cost FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load rich usage after stale refresh attempt");
    assert_eq!(persisted.0, Some(325));
    assert_eq!(persisted.1, Some(50));
    assert_eq!(persisted.2, Some(10));
    assert_eq!(persisted.3, Some(0.02));
}

#[tokio::test]
async fn websocket_terminal_refresh_retains_new_unsupported_tier_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "gpt6-websocket-refresh-unsupported-tier";
    let occurred_at = "2026-09-24 14:00:00";
    let mut initial = test_proxy_capture_record(invoke_id, occurred_at);
    initial.model = Some("gpt-6-sol".to_string());
    initial.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    initial.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","serviceTier":"standard","billingServiceTier":"standard","streamTerminalEvent":"response.completed"}"#.to_string(),
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
    let mut snapshot_tx = state.pool.begin().await.expect("begin identity read");
    let existing =
        load_persisted_invocation_identity_tx(snapshot_tx.as_mut(), invoke_id, occurred_at)
            .await
            .expect("load existing invocation")
            .expect("existing invocation");
    snapshot_tx.commit().await.expect("finish identity read");
    let mut incoming = test_proxy_capture_record(invoke_id, occurred_at);
    incoming.model = Some("gpt-6-sol".to_string());
    incoming.usage = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(325),
        reported_cache_write_tokens: Some(50),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    incoming.cost = None;
    incoming.cost_breakdown = None;
    incoming.cost_estimated = false;
    incoming.price_version = None;
    incoming.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","serviceTier":"batch","billingServiceTier":"batch","streamTerminalEvent":"response.done"}"#.to_string(),
        )
        .expect("mark richer websocket payload"),
    );
    preserve_websocket_terminal_rollup_metadata(&mut incoming, &existing);
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin richer terminal refresh");
    assert!(
        refresh_websocket_terminal_usage_tx(tx.as_mut(), existing.id, &existing, &incoming)
            .await
            .expect("apply richer terminal refresh")
    );
    tx.commit().await.expect("commit richer terminal refresh");

    let persisted = sqlx::query_as::<_, (Option<f64>, String)>(
        "SELECT cost, payload FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load refreshed websocket terminal");
    assert!(
        persisted.0.is_none(),
        "unsupported tier must have unknown cost"
    );
    let payload: Value = serde_json::from_str(&persisted.1).expect("decode refreshed payload");
    assert_eq!(payload["serviceTier"].as_str(), Some("batch"));
    assert_eq!(payload["billingServiceTier"].as_str(), Some("batch"));
}
