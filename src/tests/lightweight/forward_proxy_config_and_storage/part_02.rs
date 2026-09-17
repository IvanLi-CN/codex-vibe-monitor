#[test]
pub(crate) fn forward_proxy_manager_v2_success_recovers_penalized_proxy() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );
    let proxy_key = manager
        .endpoints
        .first()
        .expect("endpoint should exist")
        .key
        .clone();
    if let Some(runtime) = manager.runtime.get_mut(&proxy_key) {
        runtime.weight = -2.0;
        runtime.consecutive_failures = 5;
        runtime.latency_ema_ms = Some(30_000.0);
    }

    manager.record_attempt(&proxy_key, true, Some(30_000.0), false);

    let runtime = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist");
    assert!(
        runtime.weight >= FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR,
        "successful recovery should restore minimum v2 weight"
    );
    assert_eq!(
        runtime.consecutive_failures, 0,
        "successful attempt should reset failure streak"
    );
}

#[test]
pub(crate) fn classify_invocation_failure_marks_downstream_closed_as_client_abort() {
    let result = classify_invocation_failure(
        Some("http_200"),
        Some("[downstream_closed] downstream closed while streaming upstream response"),
    );
    assert_eq!(result.failure_class, FailureClass::ClientAbort);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("downstream_closed"));
}

#[tokio::test]
pub(crate) async fn proxy_error_response_exposes_image_timeout_code_only_for_504() {
    let response = build_proxy_error_response(
        ProxyErrorResponse {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: format!(
                "[{PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT}] upstream handshake timed out"
            ),
            cvm_id: Some("image-timeout-id".to_string()),
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        },
        "image-timeout-id",
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read image timeout response");
    let payload: Value = serde_json::from_slice(&body).expect("decode image timeout response");
    assert_eq!(
        payload["code"].as_str(),
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
    );
    assert_eq!(payload["cvmId"].as_str(), Some("image-timeout-id"));

    let response = build_proxy_error_response(
        ProxyErrorResponse {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        },
        "unavailable-id",
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read generic proxy error response");
    let payload: Value =
        serde_json::from_slice(&body).expect("decode generic proxy error response");
    assert!(payload.get("code").is_none());
}

#[tokio::test]
pub(crate) async fn persist_proxy_capture_runtime_record_preserves_warning_success_diagnostics() {
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        status: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let persisted = persist_proxy_capture_runtime_record(
        &pool,
        ProxyCaptureRecord {
            invoke_id: "proxy-test-downstream-closed-runtime".to_string(),
            occurred_at: "2026-04-10 00:00:00".to_string(),
            model: Some("gpt-5.4".to_string()),
            usage: ParsedUsage::default(),
            cost: None,
            cost_breakdown: None,
            cost_estimated: false,
            price_version: None,
            status: INVOCATION_STATUS_WARNING_SUCCESS.to_string(),
            error_message: None,
            failure_kind: Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string()),
            payload: None,
            raw_response: "{}".to_string(),
            response_body_preview_enabled: true,
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta::default(),
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                first_token_ms: None,
                t_upstream_stream_ms: 0.0,
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
    .expect("persist runtime record")
    .expect("persisted invocation should be returned");

    assert_eq!(
        persisted.status.as_deref(),
        Some(INVOCATION_STATUS_WARNING_SUCCESS)
    );
    assert_eq!(persisted.failure_class.as_deref(), Some("none"));
    assert_eq!(persisted.is_actionable, Some(false));
    assert_eq!(
        persisted.failure_kind.as_deref(),
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    );

    let row = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT status, failure_kind, failure_class, is_actionable
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("proxy-test-downstream-closed-runtime")
    .fetch_one(&pool)
    .await
    .expect("load persisted runtime invocation");

    assert_eq!(
        row.status.as_deref(),
        Some(INVOCATION_STATUS_WARNING_SUCCESS)
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    );
    assert_eq!(row.failure_class.as_deref(), Some("none"));
    assert_eq!(row.is_actionable, Some(0));
}

#[tokio::test]
pub(crate) async fn persist_proxy_capture_record_preserves_warning_success_diagnostics() {
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        status: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let persisted = persist_proxy_capture_record(
        &pool,
        Instant::now(),
        ProxyCaptureRecord {
            invoke_id: "proxy-test-downstream-closed-terminal".to_string(),
            occurred_at: "2026-04-10 00:00:00".to_string(),
            model: Some("gpt-5.4".to_string()),
            usage: ParsedUsage::default(),
            cost: None,
            cost_breakdown: None,
            cost_estimated: false,
            price_version: None,
            status: INVOCATION_STATUS_WARNING_SUCCESS.to_string(),
            error_message: None,
            failure_kind: Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string()),
            payload: None,
            raw_response: "{}".to_string(),
            response_body_preview_enabled: true,
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta::default(),
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                first_token_ms: None,
                t_upstream_stream_ms: 0.0,
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
    .expect("persist terminal record")
    .expect("persisted invocation should be returned");

    assert_eq!(
        persisted.status.as_deref(),
        Some(INVOCATION_STATUS_WARNING_SUCCESS)
    );
    assert_eq!(persisted.failure_class.as_deref(), Some("none"));
    assert_eq!(persisted.is_actionable, Some(false));
    assert_eq!(
        persisted.failure_kind.as_deref(),
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    );

    let row = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT status, failure_kind, failure_class, is_actionable
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("proxy-test-downstream-closed-terminal")
    .fetch_one(&pool)
    .await
    .expect("load persisted terminal invocation");

    assert_eq!(
        row.status.as_deref(),
        Some(INVOCATION_STATUS_WARNING_SUCCESS)
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    );
    assert_eq!(row.failure_class.as_deref(), Some("none"));
    assert_eq!(row.is_actionable, Some(0));
}

#[tokio::test]
pub(crate) async fn persist_proxy_capture_runtime_record_keeps_response_size_when_preview_disabled()
{
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        raw_response: String,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
    }

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let persisted = persist_proxy_capture_runtime_record(
        &pool,
        ProxyCaptureRecord {
            invoke_id: "proxy-test-runtime-response-body-disabled".to_string(),
            occurred_at: "2026-04-10 00:00:00".to_string(),
            model: Some("gpt-5.4".to_string()),
            usage: ParsedUsage::default(),
            cost: None,
            cost_breakdown: None,
            cost_estimated: false,
            price_version: None,
            status: "running".to_string(),
            error_message: None,
            failure_kind: None,
            payload: Some("{\"endpoint\":\"/v1/responses\"}".to_string()),
            raw_response: "{\"ok\":true}".to_string(),
            response_body_preview_enabled: false,
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta {
                path: Some("proxy_raw_payloads/runtime-disabled-response.bin".to_string()),
                size_bytes: 17,
                truncated: false,
                truncated_reason: None,
            },
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                first_token_ms: None,
                t_upstream_stream_ms: 0.0,
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
    .expect("persist runtime record")
    .expect("persisted invocation should be returned");

    assert_eq!(persisted.response_raw_path, None);
    assert_eq!(persisted.response_raw_size, Some(17));
    let row = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT raw_response, response_raw_path, response_raw_size
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("proxy-test-runtime-response-body-disabled")
    .fetch_one(&pool)
    .await
    .expect("load persisted runtime invocation");

    assert_eq!(row.raw_response, "");
    assert!(row.response_raw_path.is_none());
    assert_eq!(row.response_raw_size, Some(17));
}

#[tokio::test]
pub(crate) async fn persist_proxy_capture_record_omits_response_preview_and_raw_path_when_disabled()
{
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        raw_response: String,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
    }

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let persisted = persist_proxy_capture_record(
        &pool,
        Instant::now(),
        ProxyCaptureRecord {
            invoke_id: "proxy-test-response-body-disabled".to_string(),
            occurred_at: "2026-04-10 00:00:00".to_string(),
            model: Some("gpt-5.4".to_string()),
            usage: ParsedUsage::default(),
            cost: None,
            cost_breakdown: None,
            cost_estimated: false,
            price_version: None,
            status: "success".to_string(),
            error_message: None,
            failure_kind: None,
            payload: Some("{\"endpoint\":\"/v1/responses\"}".to_string()),
            raw_response: "{\"ok\":true}".to_string(),
            response_body_preview_enabled: false,
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta {
                path: Some("proxy_raw_payloads/disabled-response.bin".to_string()),
                size_bytes: 17,
                truncated: false,
                truncated_reason: None,
            },
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                first_token_ms: None,
                t_upstream_stream_ms: 0.0,
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
    .expect("persist terminal record")
    .expect("persisted invocation should be returned");

    assert_eq!(persisted.response_raw_path, None);
    assert_eq!(persisted.response_raw_size, Some(17));
    let row = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT raw_response, response_raw_path, response_raw_size
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("proxy-test-response-body-disabled")
    .fetch_one(&pool)
    .await
    .expect("load persisted invocation");

    assert_eq!(row.raw_response, "");
    assert!(row.response_raw_path.is_none());
    assert_eq!(row.response_raw_size, Some(17));
}

#[test]
pub(crate) fn proxy_capture_invocation_status_keeps_downstream_close_as_client_abort_shape() {
    assert_eq!(
        proxy_capture_invocation_status(StatusCode::OK, true, true),
        "http_200"
    );
    assert_eq!(
        proxy_capture_invocation_status(StatusCode::OK, false, false),
        "success"
    );
    assert_eq!(
        proxy_capture_invocation_status(StatusCode::OK, true, false),
        "http_200"
    );
}

#[test]
pub(crate) fn proxy_capture_is_pure_downstream_close_requires_a_clean_upstream_success() {
    assert!(proxy_capture_is_pure_downstream_close(
        StatusCode::OK,
        false,
        false,
        true,
    ));
    assert!(!proxy_capture_is_pure_downstream_close(
        StatusCode::BAD_GATEWAY,
        false,
        false,
        true,
    ));
    assert!(!proxy_capture_is_pure_downstream_close(
        StatusCode::OK,
        true,
        false,
        true,
    ));
    assert!(!proxy_capture_is_pure_downstream_close(
        StatusCode::OK,
        false,
        true,
        true,
    ));
}

#[test]
pub(crate) fn proxy_capture_invocation_failure_kind_prefers_logical_stream_failure_over_disconnect()
{
    let pure_downstream_closed =
        proxy_capture_is_pure_downstream_close(StatusCode::OK, false, true, true);
    assert!(!pure_downstream_closed);
    assert_eq!(
        proxy_capture_invocation_failure_kind(StatusCode::OK, false, true, pure_downstream_closed),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
}

#[test]
pub(crate) fn pool_capture_attempt_status_keeps_late_disconnect_after_logical_failure_as_http_failure()
 {
    let pure_downstream_closed =
        proxy_capture_is_pure_downstream_close(StatusCode::OK, false, true, true);
    assert_eq!(
        pool_capture_attempt_status(StatusCode::OK, false, true, pure_downstream_closed),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
}

#[test]
pub(crate) fn should_prebuffer_for_body_sticky_probe_respects_memory_threshold() {
    assert!(should_prebuffer_for_body_sticky_probe(
        false,
        Some("application/json"),
        Some(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES),
    ));
    assert!(!should_prebuffer_for_body_sticky_probe(
        false,
        Some("application/json"),
        Some(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1),
    ));
    assert!(!should_prebuffer_for_body_sticky_probe(
        true,
        Some("application/json"),
        Some(128),
    ));
}

#[tokio::test]
pub(crate) async fn capture_snapshot_reader_spills_large_body_to_file_and_preserves_bytes() {
    let body_bytes = Bytes::from(vec![
        b'x';
        POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1024
    ]);

    let snapshot = read_request_body_snapshot_with_partial_limit(
        Body::from(body_bytes.clone()),
        body_bytes.len() + 1024,
        Duration::from_secs(5),
        42,
    )
    .await
    .expect("large body should read into replay snapshot");

    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");
    assert_eq!(
        pool_request_snapshot_body_bytes(&snapshot),
        body_bytes.len()
    );
    assert_eq!(
        snapshot.to_bytes().await.expect("read replay snapshot"),
        body_bytes
    );
}

#[tokio::test]
pub(crate) async fn pool_replay_snapshot_from_bytes_uses_file_for_large_body() {
    let small = Bytes::from(vec![b'a'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES]);
    let large = Bytes::from(vec![b'b'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1]);

    let small_snapshot = pool_replay_snapshot_from_bytes(45, small.clone())
        .await
        .expect("build small replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&small_snapshot), "memory");
    assert_eq!(
        small_snapshot
            .to_bytes()
            .await
            .expect("read small snapshot"),
        small
    );

    let large_snapshot = pool_replay_snapshot_from_bytes(46, large.clone())
        .await
        .expect("build large replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&large_snapshot), "file");
    assert_eq!(
        pool_request_snapshot_body_bytes(&large_snapshot),
        large.len()
    );
    assert_eq!(
        large_snapshot
            .to_bytes()
            .await
            .expect("read large snapshot"),
        large
    );
}

#[tokio::test]
pub(crate) async fn pool_replay_snapshot_from_vec_uses_file_for_large_body() {
    let large = vec![b'c'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 8];

    let snapshot = pool_replay_snapshot_from_vec(47, large.clone())
        .await
        .expect("build large replay snapshot");

    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");
    assert_eq!(pool_request_snapshot_body_bytes(&snapshot), large.len());
    assert_eq!(
        snapshot.to_bytes().await.expect("read large vec snapshot"),
        Bytes::from(large)
    );
}

#[test]
pub(crate) fn build_pool_replay_temp_path_is_unique_for_same_proxy_request() {
    let first = build_pool_replay_temp_path(48);
    let second = build_pool_replay_temp_path(48);

    assert_ne!(first, second);
}

#[tokio::test]
pub(crate) async fn capture_snapshot_reader_keeps_partial_body_on_limit_error() {
    let err = read_request_body_snapshot_with_partial_limit(
        Body::from(Bytes::from_static(b"abcdef")),
        3,
        Duration::from_secs(5),
        43,
    )
    .await
    .expect_err("body should exceed test limit");

    assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(err.failure_kind, PROXY_FAILURE_BODY_TOO_LARGE);
    assert_eq!(err.partial_body, b"abc");
}

#[tokio::test]
pub(crate) async fn capture_snapshot_reader_bounds_partial_body_on_large_limit_error() {
    let partial_limit = 64 * 1024;
    let body_bytes = Bytes::from(vec![b'x'; partial_limit + 128]);

    let err = read_request_body_snapshot_with_partial_limit(
        Body::from(body_bytes),
        partial_limit + 64,
        Duration::from_secs(5),
        44,
    )
    .await
    .expect_err("body should exceed test limit");

    assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(err.failure_kind, PROXY_FAILURE_BODY_TOO_LARGE);
    assert_eq!(err.partial_body.len(), partial_limit);
}

#[test]
pub(crate) fn classify_invocation_failure_marks_invalid_key_as_client_failure() {
    let result = classify_invocation_failure(Some("http_401"), Some("Invalid API key format"));
    assert_eq!(result.failure_class, FailureClass::ClientFailure);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("invalid_api_key"));
}

#[test]
pub(crate) fn classify_invocation_failure_marks_upstream_errors_as_service_failure() {
    let result = classify_invocation_failure(
        Some("http_502"),
        Some(
            "[failed_contact_upstream] failed to contact upstream: error sending request for url (https://example.com/v1/responses)",
        ),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
pub(crate) fn classify_invocation_failure_treats_running_and_pending_as_none() {
    for status in ["running", "pending"] {
        let result = classify_invocation_failure(Some(status), None);
        assert_eq!(result.failure_class, FailureClass::None);
        assert!(!result.is_actionable);
        assert_eq!(result.failure_kind, None);
    }
}

#[test]
pub(crate) fn classify_invocation_failure_marks_upstream_response_failed_as_service_failure() {
    let result = classify_invocation_failure(
        Some("http_200"),
        Some(
            "[upstream_response_failed] server_error: An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message.",
        ),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
}

#[test]
pub(crate) fn classify_invocation_failure_marks_http_429_as_service_failure() {
    let result = classify_invocation_failure(Some("http_429"), Some("rate limited"));
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("http_429"));
}

#[test]
pub(crate) fn resolve_failure_classification_recomputes_actionable_for_missing_legacy_class() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        None,
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
}

#[test]
pub(crate) fn resolve_failure_classification_overrides_legacy_default_none_for_failures() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        Some(FailureClass::None.as_str()),
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
pub(crate) fn failure_scope_parse_defaults_to_service() {
    assert_eq!(
        FailureScope::parse(None).expect("default scope"),
        FailureScope::Service
    );
}

#[test]
pub(crate) fn failure_scope_parse_rejects_unknown_value() {
    let err = FailureScope::parse(Some("unexpected")).expect_err("invalid scope should fail");
    match err {
        ApiError::BadRequest(err) => {
            assert!(
                err.to_string()
                    .contains("unsupported failure scope: unexpected"),
                "error should mention rejected scope"
            );
        }
        other => panic!("expected BadRequest, got: {other:?}"),
    }
}

#[test]
pub(crate) fn app_config_from_sources_ignores_removed_xyai_env_vars() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let cases = [
        ("XY_BASE_URL", "not-a-valid-url"),
        ("XY_VIBE_QUOTA_ENDPOINT", "%%%"),
        ("XY_SESSION_COOKIE_NAME", "legacy-cookie"),
        ("XY_SESSION_COOKIE_VALUE", "legacy-secret"),
        ("XY_LEGACY_POLL_ENABLED", "definitely-not-bool"),
        ("XY_SNAPSHOT_MIN_INTERVAL_SECS", "not-a-number"),
    ];
    let previous = cases
        .iter()
        .map(|(name, _)| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for (name, value) in cases {
        unsafe { env::set_var(name, value) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("removed XYAI env vars should be ignored");
    assert_eq!(config.database_path, PathBuf::from("codex_vibe_monitor.db"));
}

#[test]
pub(crate) fn app_config_from_sources_reads_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::remove_var(LEGACY_ENV_DATABASE_PATH);
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let config = result.expect("DATABASE_PATH should configure the database path");
    assert_eq!(config.database_path, PathBuf::from("/tmp/codex-env.sqlite"));
}

#[test]
pub(crate) fn startup_pending_attempt_recovery_skips_all_retention_run_once_modes() {
    let mut cli = CliArgs::default();
    assert!(should_recover_pending_pool_attempts_on_startup(&cli));

    cli.command = Some(CliCommand::Maintenance(MaintenanceCliArgs {
        command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
    }));
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));

    cli.command = None;
    cli.retention_run_once = true;
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));

    cli.retention_dry_run = true;
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));
}

#[test]
pub(crate) fn app_config_from_sources_rejects_legacy_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
        env::set_var(LEGACY_ENV_DATABASE_PATH, "/tmp/codex-legacy.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let err = result.expect_err("legacy database env should fail fast");
    assert!(
        err.to_string()
            .contains("XY_DATABASE_PATH is not supported; rename it to DATABASE_PATH"),
        "error should point to the DATABASE_PATH migration"
    );
}

#[test]
pub(crate) fn app_config_from_sources_reads_renamed_public_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let mut cases = LEGACY_ENV_RENAMES
        .iter()
        .map(|(legacy, _)| (*legacy, None))
        .collect::<Vec<_>>();
    cases.extend([
        (ENV_POLL_INTERVAL_SECS, Some("11")),
        (ENV_REQUEST_TIMEOUT_SECS, Some("61")),
        (ENV_XRAY_BINARY, Some("/usr/local/bin/xray-custom")),
        (ENV_XRAY_RUNTIME_DIR, Some("/tmp/xray-runtime")),
        (ENV_MAX_PARALLEL_POLLS, Some("7")),
        (ENV_SHARED_CONNECTION_PARALLELISM, Some("3")),
        (ENV_HTTP_BIND, Some("127.0.0.1:39090")),
        (
            ENV_CORS_ALLOWED_ORIGINS,
            Some("https://app.example.com, http://localhost:5173"),
        ),
        (ENV_LIST_LIMIT_MAX, Some("321")),
        (ENV_USER_AGENT, Some("custom-agent/1.0")),
        (ENV_STATIC_DIR, Some("/tmp/static")),
        (ENV_RETENTION_ENABLED, Some("true")),
        (ENV_RETENTION_DRY_RUN, Some("true")),
        (ENV_RETENTION_INTERVAL_SECS, Some("7200")),
        (ENV_RETENTION_BATCH_ROWS, Some("2222")),
        (ENV_ARCHIVE_DIR, Some("/tmp/archive")),
        (ENV_INVOCATION_SUCCESS_FULL_DAYS, Some("31")),
        (ENV_INVOCATION_MAX_DAYS, Some("91")),
        (ENV_CODEX_INVOCATION_ARCHIVE_LAYOUT, Some("segment_v1")),
        (
            ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
            Some("day"),
        ),
        (ENV_INVOCATION_ARCHIVE_CODEC, Some("gzip")),
        (ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS, Some("32")),
        (ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS, Some("7")),
        (
            ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
            Some("30"),
        ),
        (ENV_QUOTA_SNAPSHOT_FULL_DAYS, Some("34")),
        (ENV_PROXY_RAW_COMPRESSION, Some("none")),
        (ENV_PROXY_RAW_IMMEDIATE_GZIP_BYTES, Some("2097152")),
        (ENV_PROXY_RAW_HOT_SECS, Some("1234")),
        (ENV_FORWARD_PROXY_ALGO, Some("v2")),
    ]);
    let _env = EnvVarGuard::set(&cases);

    let config =
        AppConfig::from_sources(&CliArgs::default()).expect("renamed public envs should parse");

    assert_eq!(config.poll_interval, Duration::from_secs(11));
    assert_eq!(config.request_timeout, Duration::from_secs(61));
    assert_eq!(config.xray_binary, "/usr/local/bin/xray-custom");
    assert_eq!(config.xray_runtime_dir, PathBuf::from("/tmp/xray-runtime"));
    assert_eq!(config.forward_proxy_algo, ForwardProxyAlgo::V2);
    assert_eq!(config.max_parallel_polls, 7);
    assert_eq!(config.shared_connection_parallelism, 3);
    assert_eq!(
        config.http_bind,
        "127.0.0.1:39090".parse().expect("valid socket address")
    );
    assert_eq!(
        config.cors_allowed_origins,
        vec![
            "https://app.example.com".to_string(),
            "http://localhost:5173".to_string(),
        ]
    );
    assert_eq!(config.list_limit_max, 321);
    assert_eq!(config.user_agent, "custom-agent/1.0");
    assert_eq!(config.static_dir, Some(PathBuf::from("/tmp/static")));
    assert!(config.retention_enabled);
    assert!(config.retention_dry_run);
    assert_eq!(config.retention_interval, Duration::from_secs(7200));
    assert_eq!(config.retention_batch_rows, 2222);
    assert_eq!(config.archive_dir, PathBuf::from("/tmp/archive"));
    assert_eq!(config.invocation_success_full_days, 31);
    assert_eq!(config.invocation_max_days, 91);
    assert_eq!(
        config.codex_invocation_archive_layout,
        ArchiveBatchLayout::SegmentV1
    );
    assert_eq!(
        config.codex_invocation_archive_segment_granularity,
        ArchiveSegmentGranularity::Day
    );
    assert_eq!(config.invocation_archive_codec, ArchiveFileCodec::Gzip);
    assert_eq!(config.forward_proxy_attempts_retention_days, 32);
    assert_eq!(config.pool_upstream_request_attempts_retention_days, 7);
    assert_eq!(config.pool_upstream_request_attempts_archive_ttl_days, 30);
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS)
    );
    assert_eq!(config.quota_snapshot_full_days, 34);
    assert_eq!(config.proxy_raw_compression, RawCompressionCodec::None);
    assert_eq!(config.proxy_raw_immediate_gzip_bytes, Some(2 * 1024 * 1024));
    assert_eq!(config.proxy_raw_hot_secs, 1234);
}

use super::*;
