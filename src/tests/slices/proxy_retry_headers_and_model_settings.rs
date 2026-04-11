#[tokio::test]
async fn pricing_settings_api_keeps_empty_catalog_after_reload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-empty".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect("put pricing settings should allow empty catalog");

    assert_eq!(updated.catalog_version, "custom-empty");
    assert!(updated.entries.is_empty());

    let first_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should load after update");
    assert_eq!(first_reload.version, "custom-empty");
    assert!(first_reload.models.is_empty());

    let second_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should stay empty across reloads");
    assert_eq!(second_reload.version, "custom-empty");
    assert!(second_reload.models.is_empty());
}

#[tokio::test]
async fn pricing_settings_api_rejects_invalid_payload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let err = put_pricing_settings(
        State(state),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "   ".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect_err("blank catalog version should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn seed_default_pricing_catalog_migrates_legacy_file_when_present() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-custom-v1",
  "models": {
"gpt-legacy": {
  "input_per_1m": 9.9,
  "output_per_1m": 19.9,
  "cache_input_per_1m": 0.99,
  "reasoning_per_1m": null
}
  }
}"#,
    )
    .expect("write legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog from legacy file");

    let _ = fs::remove_file(&legacy_path);

    let migrated = load_pricing_catalog(&pool)
        .await
        .expect("load migrated pricing catalog");
    assert_eq!(migrated.version, "legacy-custom-v1");
    assert_eq!(migrated.models.len(), 1);
    let model = migrated
        .models
        .get("gpt-legacy")
        .expect("legacy model should be migrated");
    assert_eq!(model.input_per_1m, 9.9);
    assert_eq!(model.output_per_1m, 19.9);
    assert_eq!(model.cache_input_per_1m, Some(0.99));
    assert_eq!(model.source, "custom");
}

#[tokio::test]
async fn seed_default_pricing_catalog_falls_back_when_legacy_file_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-empty-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-empty",
  "models": {}
}"#,
    )
    .expect("write empty legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog should fall back to defaults");

    let _ = fs::remove_file(&legacy_path);

    let seeded = load_pricing_catalog(&pool)
        .await
        .expect("load seeded pricing catalog");
    assert_eq!(seeded.version, DEFAULT_PRICING_CATALOG_VERSION);
    assert!(
        seeded.models.contains_key("gpt-5.2-codex"),
        "default pricing catalog should be seeded"
    );
}

#[tokio::test]
async fn seed_default_pricing_catalog_auto_inserts_new_models_for_legacy_default_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert!(catalog.models.contains_key("gpt-5.4"));
    assert!(catalog.models.contains_key("gpt-5.4-pro"));
}

#[tokio::test]
async fn seed_default_pricing_catalog_normalizes_gpt_5_3_codex_source_for_legacy_default_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    save_pricing_catalog(&pool, &default_pricing_catalog())
        .await
        .expect("seed default pricing catalog");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET source = 'temporary'
        WHERE model = 'gpt-5.3-codex'
        "#,
    )
    .execute(&pool)
    .await
    .expect("force legacy gpt-5.3-codex source");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog");
    let pricing = catalog
        .models
        .get("gpt-5.3-codex")
        .expect("gpt-5.3-codex pricing present");
    assert_eq!(pricing.source, "official");
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_auto_insert_new_models_for_custom_catalog_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind("custom-ci")
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set custom pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert!(!catalog.models.contains_key("gpt-5.4"));
    assert!(!catalog.models.contains_key("gpt-5.4-pro"));
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_override_existing_pricing_for_new_models() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    // Simulate a legacy default catalog version so startup seeding will call
    // ensure_pricing_models_present, which must not overwrite existing rows.
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set legacy pricing catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            cache_input_per_1m = ?3,
            source = 'custom'
        WHERE model = 'gpt-5.4'
        "#,
    )
    .bind(99.0)
    .bind(199.0)
    .bind(Some(9.9))
    .execute(&pool)
    .await
    .expect("override gpt-5.4 pricing for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            source = 'custom'
        WHERE model = 'gpt-5.4-pro'
        "#,
    )
    .bind(88.0)
    .bind(188.0)
    .execute(&pool)
    .await
    .expect("override gpt-5.4-pro pricing for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    let gpt_5_4 = catalog.models.get("gpt-5.4").expect("gpt-5.4 should exist");
    assert_eq!(gpt_5_4.input_per_1m, 99.0);
    assert_eq!(gpt_5_4.output_per_1m, 199.0);
    assert_eq!(gpt_5_4.cache_input_per_1m, Some(9.9));
    assert_eq!(gpt_5_4.source, "custom");

    let gpt_5_4_pro = catalog
        .models
        .get("gpt-5.4-pro")
        .expect("gpt-5.4-pro should exist");
    assert_eq!(gpt_5_4_pro.input_per_1m, 88.0);
    assert_eq!(gpt_5_4_pro.output_per_1m, 188.0);
    assert_eq!(gpt_5_4_pro.cache_input_per_1m, None);
    assert_eq!(gpt_5_4_pro.source, "custom");
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_passthrough_when_hijack_disabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(
        ids,
        vec!["upstream-model-a".to_string(), "gpt-5.2-codex".to_string()]
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_returns_preset_when_hijack_enabled_without_merge() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(PROXY_MODEL_MERGE_STATUS_HEADER)
            .is_none()
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(
        ids,
        vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()]
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_returns_gpt_5_4_models_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_merges_upstream_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec![
                "gpt-5.2-codex".to_string(),
                "gpt-5.1-codex-mini".to_string(),
            ],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
    let ids = extract_model_ids(&payload);

    assert!(ids.contains(&"upstream-model-a".to_string()));
    assert!(ids.contains(&"gpt-5.2-codex".to_string()));
    assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
    assert!(!ids.contains(&"gpt-5.3-codex".to_string()));
    assert_eq!(
        ids.iter()
            .filter(|id| id.as_str() == "gpt-5.2-codex")
            .count(),
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_bypass_hijack_for_pool_route_requests() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(PROXY_MODEL_MERGE_STATUS_HEADER)
            .is_none()
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read models body");
    let payload: Value = serde_json::from_slice(&body).expect("decode models payload");
    assert_eq!(
        extract_model_ids(&payload),
        vec!["upstream-model-a".to_string(), "gpt-5.2-codex".to_string()]
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_pool_failures_do_not_return_untracked_cvm_id() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_retrying_models_upstream(99, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().get(CVM_INVOKE_ID_HEADER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read models failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode models failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
    assert!(payload.get("cvmId").is_none());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_merges_upstream_after_429_retry() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_retrying_models_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: 1,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
    let ids = extract_model_ids(&payload);
    assert!(ids.contains(&"upstream-model-after-retry".to_string()));
    assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 2);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_falls_back_to_preset_when_merge_upstream_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models?mode=error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_retries_429_then_falls_back_once_exhausted() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_retrying_models_upstream(99, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: 2,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read fallback response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    assert_eq!(
        extract_model_ids(&payload),
        vec!["gpt-5.1-codex-mini".to_string()]
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 3);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        3
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_models_falls_back_when_merge_body_decode_times_out() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        })),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models?mode=slow-body".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "merge fallback should return quickly when decode times out"
    );
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read fallback response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_preserves_streaming_response() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(http_header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read stream body");
    assert_eq!(&body[..], b"chunk-achunk-b");

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_returns_bad_gateway_when_first_stream_chunk_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-first-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("upstream stream error before first chunk")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_propagates_stream_error_after_first_chunk() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-mid-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_preserves_redirect_without_following() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(http_header::LOCATION),
        Some(&HeaderValue::from_static("/v1/echo?from=redirect"))
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_blocks_cross_origin_redirect() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect-external".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_persists_record_on_redirect_rewrite_error() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        source: String,
        status: Option<String>,
        error_message: Option<String>,
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"hi"}]}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT source, status, error_message, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("http_502"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("cross-origin redirect is not allowed"))
    );
    assert!(row.t_total_ms.is_some_and(|v| v > 0.0));
    assert!(row.t_req_read_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_req_parse_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_upstream_connect_ms.is_some_and(|v| v >= 0.0));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_persist_and_broadcast_emits_records_summary_and_quota() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;

    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "proxy-sse-broadcast-success";
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist+broadcast should succeed");

    let mut saw_record = false;
    let mut captured_record: Option<ApiInvocation> = None;
    let mut saw_quota = false;
    let mut summary_windows = HashSet::new();
    let expected_summary_windows = summary_broadcast_specs().len();
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for proxy broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                if let Some(record) = records
                    .into_iter()
                    .find(|record| record.invoke_id == invoke_id)
                {
                    saw_record = true;
                    captured_record = Some(record);
                }
            }
            BroadcastPayload::Summary { window, summary } => {
                summary_windows.insert(window.clone());
                if window == "all" {
                    assert_eq!(summary.total_count, 1);
                }
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Version { .. } | BroadcastPayload::PoolAttempts { .. } => {}
        }

        if saw_record && saw_quota && summary_windows.len() == expected_summary_windows {
            break;
        }
    }

    assert!(saw_record, "records payload should be broadcast");
    assert!(saw_quota, "quota payload should be broadcast");
    assert_eq!(
        summary_windows.len(),
        expected_summary_windows,
        "all summary windows should be broadcast"
    );
    let record = captured_record.expect("target records payload should include invoke id");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-broadcast-1"));
    assert_eq!(record.route_mode.as_deref(), Some("pool"));
    assert_eq!(record.upstream_account_id, Some(17));
    assert_eq!(
        record.upstream_account_name.as_deref(),
        Some("pool-account-17")
    );
    assert_eq!(
        record.response_content_encoding.as_deref(),
        Some("gzip, br")
    );
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
    assert!(record.failure_kind.is_none());
}

#[tokio::test]
async fn proxy_capture_persist_and_broadcast_skips_duplicate_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invoke_id = "proxy-sse-broadcast-duplicate";
    let mut rx = state.broadcaster.subscribe();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("initial persist+broadcast should succeed");

    drain_broadcast_messages(&mut rx).await;

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("duplicate persist should not fail");

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(BroadcastPayload::Records { records })) => {
                assert!(
                    records.iter().all(|record| record.invoke_id != invoke_id),
                    "duplicate insert should not emit records payload for the same invoke_id"
                );
            }
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => continue,
        }
    }
}

#[tokio::test]
async fn fetch_and_store_skips_summary_and_quota_collection_when_broadcast_state_disabled() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;

    let publish = fetch_and_store(state.as_ref(), false, false)
        .await
        .expect("fetch_and_store should succeed");

    assert!(publish.summaries.is_empty());
    assert!(publish.quota_snapshot.is_none());
    assert!(!publish.collected_broadcast_state);
}

#[test]
fn should_collect_late_broadcast_state_when_subscribers_arrive_mid_poll() {
    assert!(should_collect_late_broadcast_state(1, false));
    assert!(!should_collect_late_broadcast_state(0, false));
    assert!(!should_collect_late_broadcast_state(1, true));
}

#[tokio::test]
async fn broadcast_summary_if_changed_skips_duplicate_payloads() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let first = StatsResponse {
        total_count: 1,
        success_count: 1,
        failure_count: 0,
        total_cost: 0.5,
        total_tokens: 42,
        maintenance: None,
    };

    assert!(
        broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            first.clone(),
        )
        .await
        .expect("first summary broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for first summary payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Summary { window, summary } => {
            assert_eq!(window, "1d");
            assert_eq!(summary, first);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    assert!(
        !broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            first.clone(),
        )
        .await
        .expect("duplicate summary broadcast should succeed")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    let updated = StatsResponse {
        total_count: 2,
        ..first
    };
    assert!(
        broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            updated.clone(),
        )
        .await
        .expect("changed summary broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for updated summary payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Summary { window, summary } => {
            assert_eq!(window, "1d");
            assert_eq!(summary, updated);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_quota_if_changed_skips_duplicate_payloads() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let first = QuotaSnapshotResponse {
        captured_at: "2026-03-07 10:00:00".to_string(),
        amount_limit: Some(100.0),
        used_amount: Some(10.0),
        remaining_amount: Some(90.0),
        period: Some("monthly".to_string()),
        period_reset_time: Some("2026-04-01 00:00:00".to_string()),
        expire_time: None,
        is_active: true,
        total_cost: 10.0,
        total_requests: 9,
        total_tokens: 150,
        last_request_time: Some("2026-03-07 10:00:00".to_string()),
        billing_type: Some("prepaid".to_string()),
        remaining_count: Some(91),
        used_count: Some(9),
        sub_type_name: Some("unit".to_string()),
    };

    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("first quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for first quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, first);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    assert!(
        !broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("duplicate quota broadcast should succeed")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    let updated = QuotaSnapshotResponse {
        total_requests: 10,
        ..first
    };
    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            updated.clone(),
        )
        .await
        .expect("changed quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for updated quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, updated);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test]
async fn capture_targets_reject_non_pool_requests_before_proxying() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "stream": false
            }))
            .expect("serialize request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy error body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert_eq!(
        payload["error"].as_str(),
        Some("pool route key missing or invalid")
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn read_request_body_timeout_returns_408() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;

    let slow_body = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from_static(br#"{}"#)), 1))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body read timed out")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_read_timeout]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_read_timeout")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_retries_429_and_persists_single_invocation() {
    let (upstream_base, attempts, seen_payloads, upstream_handle) =
        spawn_retrying_capture_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let request_payload = json!({
        "model": "gpt-5.2-codex",
        "stream": false,
        "input": "hello",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&request_payload).expect("serialize capture retry request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode capture retry payload");
    assert_eq!(payload["attempt"], 2);
    assert_eq!(payload["received"], request_payload);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen_payloads
            .lock()
            .expect("lock retrying capture payloads")
            .clone(),
        vec![request_payload.clone(), request_payload.clone()]
    );

    let mut invocation_count: i64 = 0;
    let mut attempt_count: i64 = 0;
    let mut rate_limit_count: i64 = 0;
    for _ in 0..20 {
        invocation_count = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("count persisted invocations");
        attempt_count = count_request_forward_proxy_attempts(&state.pool).await;
        rate_limit_count = count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await;

        if invocation_count == 1 && attempt_count == 2 && rate_limit_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(invocation_count, 1);
    assert_eq!(attempt_count, 2);
    assert_eq!(rate_limit_count, 1);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_client_body_disconnect_returns_400_with_failure_kind() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let disconnected_body = stream::iter(vec![Err::<Bytes, io::Error>(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "client disconnected",
    ))]);

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(disconnected_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("failed to read request body stream")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_stream_error_client_closed]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_stream_error_client_closed")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_stream_error_emits_failure_kind_and_persists() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.2-codex",
        "stream": true,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_stream_error]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_stream_error")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_response_failed_stream_persists_service_failure_details() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=response_failed"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let text = String::from_utf8(body.to_vec()).expect("response body should be utf8");
    assert!(text.contains("response.failed"));

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");

    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 1);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        )
        .await,
        1
    );

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_response_failed")
    );
    assert_eq!(
        payload_json["streamTerminalEvent"].as_str(),
        Some("response.failed")
    );
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("server_error")
    );
    assert!(
        payload_json["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|msg| msg.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1"))
    );
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_compact_estimates_cost_and_flows_into_stats_without_rewrite() {
    #[derive(sqlx::FromRow)]
    struct PersistedCompactRow {
        endpoint: Option<String>,
        model: Option<String>,
        requested_service_tier: Option<String>,
        input_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        reasoning_tokens: Option<i64>,
        total_tokens: Option<i64>,
        cost: Option<f64>,
        price_version: Option<String>,
    }

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

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
                model,
                CASE
                  WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(payload, '$.requestedServiceTier')
                  WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(payload, '$.requested_service_tier')
                END AS requested_service_tier,
                input_tokens,
                cache_input_tokens,
                output_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                price_version
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
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
        }),
    )
    .await
    .expect("compact fetch_summary should succeed");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 577);
    assert_f64_close(summary.total_cost, 0.0020235);

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
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

    upstream_handle.abort();
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
    let mut config = test_config();
    config.request_timeout = Duration::from_secs(61);
    config.pool_upstream_responses_attempt_timeout = Duration::from_secs(121);
    config.pool_upstream_responses_total_timeout = Duration::from_secs(301);
    config.openai_proxy_handshake_timeout = Duration::from_secs(71);
    config.openai_proxy_compact_handshake_timeout = Duration::from_secs(305);
    config.openai_proxy_request_read_timeout = Duration::from_secs(181);
    let state = test_state_from_config(config.clone(), true).await;

    let Json(initial) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("load initial pool routing settings");
    assert_eq!(initial.timeouts.responses_first_byte_timeout_secs, 121);
    assert_eq!(initial.timeouts.compact_first_byte_timeout_secs, 305);
    assert_eq!(initial.timeouts.responses_stream_timeout_secs, 301);
    assert_eq!(initial.timeouts.compact_stream_timeout_secs, 301);

    let persisted = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
        r#"
        SELECT
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
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

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: Some(135),
            compact_first_byte_timeout_secs: Some(325),
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
    assert_eq!(updated.timeouts.responses_stream_timeout_secs, 405);
    assert_eq!(updated.timeouts.compact_stream_timeout_secs, 505);

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
    assert_eq!(resolved.responses_stream_timeout, Duration::from_secs(405));
    assert_eq!(resolved.compact_stream_timeout, Duration::from_secs(505));
    assert_eq!(resolved.default_send_timeout, Duration::from_secs(71));
    assert_eq!(resolved.request_read_timeout, Duration::from_secs(181));
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
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
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
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
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
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: None,
            compact_first_byte_timeout_secs: None,
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
    config.openai_proxy_request_read_timeout = Duration::from_secs(181);
    let state = test_state_from_config(config.clone(), true).await;

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: Some(135),
            compact_first_byte_timeout_secs: Some(325),
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
fn pool_same_account_attempt_budget_limits_follow_up_accounts_for_responses_family() {
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
        1
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/responses/compact".parse().expect("valid compact uri"),
            &Method::POST,
            3,
            3,
        ),
        1
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
}
