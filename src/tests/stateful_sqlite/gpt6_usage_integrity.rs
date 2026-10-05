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
fn gpt_6_default_actual_tier_uses_standard_pricing() {
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        total_tokens: Some(1_100),
        ..ParsedUsage::default()
    };
    let (default_cost, default_estimated, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );
    let (standard_cost, standard_estimated, _) = estimate_proxy_cost(
        &default_pricing_catalog(),
        Some("gpt-6-astra"),
        &usage,
        Some("standard"),
        ProxyPricingMode::ResponseTier,
    );

    assert!(
        (default_cost.expect("default response tier has a price")
            - standard_cost.expect("standard response tier has a price"))
        .abs()
            < 1e-12,
        "default response tier must use Standard pricing"
    );
    assert!(default_estimated);
    assert!(standard_estimated);
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
