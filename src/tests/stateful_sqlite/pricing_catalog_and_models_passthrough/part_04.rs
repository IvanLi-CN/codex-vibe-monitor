#[tokio::test]
pub(crate) async fn pool_routing_cache_hit_settings_are_validated_and_partially_updated() {
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
pub(crate) async fn pool_routing_settings_timeout_updates_succeed_without_crypto_key() {
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
pub(crate) async fn pool_routing_settings_timeout_updates_tolerate_invalid_cached_api_key_ciphertext()
 {
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
pub(crate) async fn pool_routing_settings_api_key_updates_require_crypto_key() {
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
pub(crate) async fn pool_routing_settings_reject_timeouts_above_i64_max() {
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
pub(crate) async fn proxy_request_timeouts_only_apply_pool_overrides_to_pool_routes() {
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
pub(crate) fn pool_same_account_attempt_budget_keeps_follow_up_accounts_retryable_for_responses_family()
 {
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

use super::*;
