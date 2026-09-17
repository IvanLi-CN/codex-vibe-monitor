#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_compact_estimates_cost_and_flows_into_stats_without_rewrite() {
    let (upstream_base, captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut pricing = state.pricing_catalog.write().await;
        *pricing = PricingCatalog {
            version: "compact-unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.1-codex-max".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: Some(0.5),
                    cache_read_per_1m: Some(0.5),
                    cache_write_per_1m: None,
                    reasoning_per_1m: Some(7.0),
                    source: "custom".to_string(),
                },
            )]),
        };
    }
    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let captured = captured_requests.lock().await;
    let captured_request = captured
        .first()
        .cloned()
        .expect("upstream should receive a compact request body");
    drop(captured);
    assert_eq!(captured_request["serviceTier"], "flex");
    assert!(captured_request.get("service_tier").is_none());

    assert_compact_capture_persisted(&state).await;
    assert_compact_stats(&state).await;

    upstream_handle.abort();
}
async fn assert_compact_capture_persisted(state: &Arc<AppState>) {
    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            "SELECT CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, model, CASE WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' THEN json_extract(payload, '$.requestedServiceTier') WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens, cost, price_version FROM codex_invocations ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query compact capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("compact capture record should be persisted");
    assert_eq!(row.endpoint.as_deref(), Some("/v1/responses/compact"));
    assert_eq!(row.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(row.requested_service_tier.as_deref(), Some("flex"));
    assert_eq!(row.input_tokens, Some(139));
    assert_eq!(row.cache_input_tokens, Some(11));
    assert_eq!(row.output_tokens, Some(438));
    assert_eq!(row.reasoning_tokens, Some(64));
    assert_eq!(row.total_tokens, Some(577));
    assert_eq!(row.price_version.as_deref(), Some("compact-unit-test"));
    assert_f64_close(row.cost.expect("compact cost should be present"), 0.0020235);
}

async fn assert_compact_stats(state: &Arc<AppState>) {
    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("compact fetch_stats should succeed");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.success_count, 1);
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.total_tokens, 577);
    assert_f64_close(stats.total_cost, 0.0020235);
    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: None,
            upstream_account_id: None,
        }),
    )
    .await
    .expect("compact fetch_summary should succeed");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 577);
    assert_f64_close(summary.total_cost, 0.0020235);
    let Json(timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
            upstream_account_id: None,
        }),
    )
    .await
    .expect("compact fetch_timeseries should succeed");
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        577
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.0020235,
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_compact_uses_dedicated_handshake_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base_and_proxy_timeouts(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(100),
        Duration::from_millis(400),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses/compact?mode=delay"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    upstream_handle.abort();
}

#[test]
fn pool_upstream_first_chunk_timeout_uses_compact_budget_for_compact_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/responses/compact".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(400));
}

#[test]
fn pool_upstream_first_chunk_timeout_uses_responses_budget_for_responses_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(1200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(1200));
}

#[test]
fn image_routes_use_dedicated_send_and_first_chunk_budget() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_image_handshake_timeout = Duration::from_millis(900);
    let timeouts = pool_routing_timeouts_from_config(&config);

    for (path, target) in [
        (
            "/v1/images/generations",
            ProxyCaptureTarget::ImageGenerations,
        ),
        ("/v1/images/edits", ProxyCaptureTarget::ImageEdits),
    ] {
        assert_eq!(
            proxy_upstream_send_timeout_for_capture_target(&timeouts, Some(target)),
            Duration::from_millis(900),
        );
        assert_eq!(
            pool_upstream_first_chunk_timeout(
                &timeouts,
                &path.parse().expect("valid image uri"),
                &Method::POST,
            ),
            Duration::from_millis(900),
        );
    }
}

#[test]
fn pool_upstream_send_timeout_uses_responses_budget_for_responses_route() {
    let handshake_timeout = Duration::from_millis(100);
    let responses_timeout = Duration::from_millis(1200);

    let timeout = pool_upstream_send_timeout(
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
        handshake_timeout,
        responses_timeout,
    );

    assert_eq!(timeout, responses_timeout);
}

#[test]
fn pool_upstream_first_chunk_timeout_keeps_default_budget_for_non_responses_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(1200);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(200));
}

#[test]
fn pool_upstream_send_timeout_caps_non_responses_route_by_first_byte_budget() {
    let handshake_timeout = Duration::from_millis(1200);
    let first_byte_timeout = Duration::from_millis(100);

    let timeout = pool_upstream_send_timeout(
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
        handshake_timeout,
        first_byte_timeout,
    );

    assert_eq!(timeout, first_byte_timeout);
}

#[test]
fn classify_compact_support_observation_is_conservative() {
    let compact_uri: Uri = "/v1/responses/compact".parse().expect("valid compact uri");

    let supported = classify_compact_support_observation(&compact_uri, Some(StatusCode::OK), None)
        .expect("compact success observation");
    assert_eq!(supported.status, COMPACT_SUPPORT_STATUS_SUPPORTED);

    let unsupported = classify_compact_support_observation(
        &compact_uri,
        Some(StatusCode::SERVICE_UNAVAILABLE),
        Some("No available channel for model gpt-5.4-openai-compact under group default (distributor)"),
    )
    .expect("compact unsupported observation");
    assert_eq!(unsupported.status, COMPACT_SUPPORT_STATUS_UNSUPPORTED);

    let unknown = classify_compact_support_observation(
        &compact_uri,
        None,
        Some("upstream handshake timed out after 300000ms"),
    )
    .expect("compact unknown observation");
    assert_eq!(unknown.status, COMPACT_SUPPORT_STATUS_UNKNOWN);

    assert!(
        classify_compact_support_observation(
            &"/v1/responses".parse().expect("valid responses uri"),
            Some(StatusCode::OK),
            None,
        )
        .is_none()
    );
}

#[tokio::test]
async fn pool_routing_settings_backfill_defaults_and_persist_timeout_updates() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard().await;
    let mut config = test_config();
    config.request_timeout = Duration::from_secs(61);
    config.pool_upstream_responses_attempt_timeout = Duration::from_secs(121);
    config.pool_upstream_responses_total_timeout = Duration::from_secs(301);
    config.openai_proxy_handshake_timeout = Duration::from_secs(71);
    config.openai_proxy_compact_handshake_timeout = Duration::from_secs(305);
    config.openai_proxy_image_handshake_timeout = Duration::from_secs(306);
    config.openai_proxy_request_read_timeout = Duration::from_secs(181);
    let state = test_state_from_config(config.clone(), true).await;

    assert_pool_routing_settings_backfill_defaults(&state).await;

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: Some(false),
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: Some(135),
            compact_first_byte_timeout_secs: Some(325),
            image_first_byte_timeout_secs: Some(300),
            responses_stream_timeout_secs: Some(405),
            compact_stream_timeout_secs: Some(505),
        }),
    };
    let Json(updated) =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("update pool routing timeouts");
    assert_eq!(updated.timeouts.responses_first_byte_timeout_secs, 135);
    assert_eq!(updated.timeouts.compact_first_byte_timeout_secs, 325);
    assert_eq!(updated.timeouts.image_first_byte_timeout_secs, 300);
    assert_eq!(updated.timeouts.responses_stream_timeout_secs, 405);
    assert_eq!(updated.timeouts.compact_stream_timeout_secs, 505);
    assert!(!updated.priority_handoff_admission_enabled);

    let Json(reloaded) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("reload pool routing settings after admission update");
    assert!(!reloaded.priority_handoff_admission_enabled);

    sqlx::query(
        "UPDATE pool_routing_settings SET priority_handoff_admission_enabled = 1 WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("simulate stale persisted admission setting");
    let Json(local_mirror) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("read local admission mirror after stale persistence");
    assert!(!local_mirror.priority_handoff_admission_enabled);

    let _ = update_pool_routing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(UpdatePoolRoutingSettingsRequest {
            api_key: None,
            maintenance: None,
            request_compression_algorithm: None,
            request_compression_level_preset: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            cache_hit_protection: None,
            priority_handoff_admission_enabled: Some(true),
            timeouts: None,
        }),
    )
    .await
    .expect("restore priority handoff admission setting");

    let resolved = resolve_pool_routing_timeouts(&state.pool, &state.config)
        .await
        .expect("resolve updated pool routing timeouts");
    assert_eq!(resolved.default_first_byte_timeout, Duration::from_secs(61));
    assert_eq!(
        resolved.responses_first_byte_timeout,
        Duration::from_secs(135)
    );
    assert_eq!(
        resolved.compact_first_byte_timeout,
        Duration::from_secs(325)
    );
    assert_eq!(resolved.image_first_byte_timeout, Duration::from_secs(300));
    assert_eq!(resolved.responses_stream_timeout, Duration::from_secs(405));
    assert_eq!(resolved.compact_stream_timeout, Duration::from_secs(505));
    assert_eq!(resolved.default_send_timeout, Duration::from_secs(71));
    assert_eq!(resolved.request_read_timeout, Duration::from_secs(181));
}

async fn assert_pool_routing_settings_backfill_defaults(state: &Arc<AppState>) {
    let Json(initial) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("load initial pool routing settings");
    assert_eq!(initial.timeouts.responses_first_byte_timeout_secs, 121);
    assert_eq!(initial.timeouts.compact_first_byte_timeout_secs, 305);
    assert_eq!(initial.timeouts.image_first_byte_timeout_secs, 306);
    assert_eq!(initial.timeouts.responses_stream_timeout_secs, 301);
    assert_eq!(initial.timeouts.compact_stream_timeout_secs, 301);
    assert!(initial.priority_handoff_admission_enabled);

    let persisted = sqlx::query_as::<
        _,
        (
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            image_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs
        FROM pool_routing_settings
        WHERE id = 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load unresolved timeout row");
    assert_eq!(persisted.0, None);
    assert_eq!(persisted.1, None);
    assert_eq!(persisted.2, None);
    assert_eq!(persisted.3, None);
    assert_eq!(persisted.4, None);
}

#[tokio::test]
async fn pool_routing_cache_hit_settings_are_validated_and_partially_updated() {
    let state = test_state_from_config(test_config(), true).await;
    let Json(initial) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("load default routing settings");
    assert!(!initial.cache_hit_protection.enabled);
    assert_eq!(
        initial.cache_hit_protection.low_hit_rate_threshold_percent,
        10
    );
    assert_eq!(initial.cache_hit_protection.overflow_mode, "queue");
    assert_eq!(initial.cache_hit_protection.minimum_input_tokens, 3_840);

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        timeouts: None,
        cache_hit_protection: Some(UpdateCacheHitProtectionSettingsRequest {
            enabled: Some(true),
            low_hit_rate_threshold_percent: Some(15),
            overflow_mode: Some("reroute".to_string()),
        }),
        priority_handoff_admission_enabled: None,
    };
    let Json(updated) =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("save cache-hit routing settings");
    assert!(updated.cache_hit_protection.enabled);
    assert_eq!(
        updated.cache_hit_protection.low_hit_rate_threshold_percent,
        15
    );
    assert_eq!(updated.cache_hit_protection.overflow_mode, "reroute");

    let invalid = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        timeouts: None,
        cache_hit_protection: Some(UpdateCacheHitProtectionSettingsRequest {
            enabled: None,
            low_hit_rate_threshold_percent: Some(0),
            overflow_mode: None,
        }),
        priority_handoff_admission_enabled: None,
    };
    let error = update_pool_routing_settings(State(state), HeaderMap::new(), Json(invalid))
        .await
        .expect_err("reject invalid cache-hit threshold");
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("lowHitRateThresholdPercent"));
}

#[tokio::test]
async fn pool_routing_settings_timeout_updates_succeed_without_crypto_key() {
    let state = test_state_from_config(test_config(), true).await;
    let _env_guard = EnvVarGuard::set(&[(ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET, None)]);
    let read_only_runtime = Arc::new(
        UpstreamAccountsRuntime::from_env().expect("build read-only upstream accounts runtime"),
    );
    assert!(!read_only_runtime.writes_enabled());
    let state = clone_state_with_upstream_accounts(&state, read_only_runtime);

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
            image_first_byte_timeout_secs: None,
            responses_stream_timeout_secs: Some(375),
            compact_stream_timeout_secs: None,
        }),
    };
    let Json(response) =
        update_pool_routing_settings(State(state), HeaderMap::new(), Json(payload))
            .await
            .expect("timeout-only routing update should succeed without crypto key");
    assert_eq!(response.timeouts.responses_stream_timeout_secs, 375);
}

#[tokio::test]
async fn pool_routing_settings_timeout_updates_tolerate_invalid_cached_api_key_ciphertext() {
    let state = test_state_from_config(test_config(), true).await;
    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET encrypted_api_key = ?1,
            masked_api_key = ?2
        WHERE id = 1
        "#,
    )
    .bind("not-a-valid-ciphertext")
    .bind("sk-bad")
    .execute(&state.pool)
    .await
    .expect("poison stored pool api key ciphertext");

    {
        let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
        *runtime_cache = None;
    }

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
            image_first_byte_timeout_secs: None,
            responses_stream_timeout_secs: Some(375),
            compact_stream_timeout_secs: None,
        }),
    };
    let Json(response) =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("timeout-only routing update should stay writable with invalid cached api key");
    assert_eq!(response.timeouts.responses_stream_timeout_secs, 375);
    assert!(
        state.pool_routing_runtime_cache.lock().await.is_none(),
        "best-effort refresh should keep lazy resolution when the stored key cannot be decrypted"
    );

    let persisted = sqlx::query_as::<_, (Option<i64>,)>(
        r#"
        SELECT responses_stream_timeout_secs
        FROM pool_routing_settings
        WHERE id = 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load updated timeout row");
    assert_eq!(persisted.0, Some(375));
}

#[tokio::test]
async fn pool_routing_settings_api_key_updates_require_crypto_key() {
    let state = test_state_from_config(test_config(), true).await;
    let _env_guard = EnvVarGuard::set(&[(ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET, None)]);
    let read_only_runtime = Arc::new(
        UpstreamAccountsRuntime::from_env().expect("build read-only upstream accounts runtime"),
    );
    assert!(!read_only_runtime.writes_enabled());
    let state = clone_state_with_upstream_accounts(&state, read_only_runtime);

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: Some("pool-secret".to_string()),
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: None,
        timeouts: None,
    };
    let err = update_pool_routing_settings(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("api key routing update should stay blocked in read-only mode");
    assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(err.1.contains(ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET));
}

#[tokio::test]
async fn pool_routing_settings_reject_timeouts_above_i64_max() {
    let state = test_state_from_config(test_config(), true).await;

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
            image_first_byte_timeout_secs: None,
            responses_stream_timeout_secs: Some(i64::MAX as u64 + 1),
            compact_stream_timeout_secs: None,
        }),
    };
    let err = update_pool_routing_settings(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("timeouts above i64::MAX should be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("responsesStreamTimeoutSecs"));
    assert!(err.1.contains(&i64::MAX.to_string()));
}

#[tokio::test]
async fn proxy_request_timeouts_only_apply_pool_overrides_to_pool_routes() {
    let mut config = test_config();
    config.request_timeout = Duration::from_secs(61);
    config.pool_upstream_responses_attempt_timeout = Duration::from_secs(121);
    config.pool_upstream_responses_total_timeout = Duration::from_secs(301);
    config.openai_proxy_handshake_timeout = Duration::from_secs(71);
    config.openai_proxy_compact_handshake_timeout = Duration::from_secs(305);
    config.openai_proxy_image_handshake_timeout = Duration::from_secs(306);
    config.openai_proxy_request_read_timeout = Duration::from_secs(181);
    let state = test_state_from_config(config.clone(), true).await;

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: Some(135),
            compact_first_byte_timeout_secs: Some(325),
            image_first_byte_timeout_secs: Some(326),
            responses_stream_timeout_secs: Some(405),
            compact_stream_timeout_secs: Some(505),
        }),
    };
    let _ = update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("update pool routing timeouts");

    let direct_timeouts = resolve_proxy_request_timeouts(state.as_ref(), false)
        .await
        .expect("resolve direct request timeouts");
    assert_eq!(
        direct_timeouts.default_first_byte_timeout,
        Duration::from_secs(61)
    );
    assert_eq!(
        direct_timeouts.responses_first_byte_timeout,
        Duration::from_secs(121)
    );
    assert_eq!(
        direct_timeouts.default_send_timeout,
        Duration::from_secs(71)
    );
    assert_eq!(
        direct_timeouts.compact_first_byte_timeout,
        Duration::from_secs(305)
    );
    assert_eq!(
        direct_timeouts.image_first_byte_timeout,
        Duration::from_secs(306)
    );
    assert_eq!(
        direct_timeouts.responses_stream_timeout,
        Duration::from_secs(301)
    );
    assert_eq!(
        direct_timeouts.compact_stream_timeout,
        Duration::from_secs(301)
    );
    assert_eq!(
        direct_timeouts.request_read_timeout,
        Duration::from_secs(181)
    );

    let pool_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool request timeouts");
    assert_eq!(
        pool_timeouts.default_first_byte_timeout,
        Duration::from_secs(61)
    );
    assert_eq!(
        pool_timeouts.responses_first_byte_timeout,
        Duration::from_secs(135)
    );
    assert_eq!(pool_timeouts.default_send_timeout, Duration::from_secs(71));
    assert_eq!(
        pool_timeouts.compact_first_byte_timeout,
        Duration::from_secs(325)
    );
    assert_eq!(
        pool_timeouts.image_first_byte_timeout,
        Duration::from_secs(326)
    );
    assert_eq!(
        pool_timeouts.responses_stream_timeout,
        Duration::from_secs(405)
    );
    assert_eq!(
        pool_timeouts.compact_stream_timeout,
        Duration::from_secs(505)
    );
    assert_eq!(pool_timeouts.request_read_timeout, Duration::from_secs(181));
}

#[test]
fn pool_same_account_attempt_budget_keeps_follow_up_accounts_retryable_for_responses_family() {
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses".parse().expect("valid uri"),
            &Method::POST,
            1,
            3,
        ),
        3
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses".parse().expect("valid uri"),
            &Method::POST,
            2,
            3,
        ),
        3
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses/compact".parse().expect("valid compact uri"),
            &Method::POST,
            3,
            3,
        ),
        3
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses".parse().expect("valid uri"),
            &Method::POST,
            1,
            2,
        ),
        2
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses".parse().expect("valid uri"),
            &Method::POST,
            2,
            2,
        ),
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    );
}
