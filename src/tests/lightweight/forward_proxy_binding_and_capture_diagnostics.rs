use super::*;

#[test]
fn legacy_bound_proxy_keys_still_route_to_matching_stable_endpoints() {
    let legacy_proxy_url = "http://127.0.0.1:7890";
    let stable_proxy_key =
        normalize_single_proxy_key(legacy_proxy_url).expect("legacy proxy url should normalize");
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![legacy_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    let scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![legacy_proxy_url.to_string()],
    );
    let selected = manager
        .select_proxy_for_scope(&scope)
        .expect("legacy bound key should still select proxy");

    assert_eq!(selected.key, stable_proxy_key);
}

#[test]
fn legacy_vless_and_trojan_bound_proxy_keys_route_to_matching_stable_endpoints() {
    let explicit_vless_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=none&type=tcp#东京节点";
    let normalized_vless_proxy_url = normalize_share_link_scheme(explicit_vless_proxy_url, "vless")
        .expect("normalize vless url");
    let explicit_legacy_vless_proxy_key = {
        let parsed = Url::parse(&normalized_vless_proxy_url).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let omitted_default_vless_proxy_key = stable_forward_proxy_key(&canonical_share_link_identity(
        &Url::parse("vless://11111111-1111-1111-1111-111111111111@vless.example.com:443#东京节点")
            .expect("parse omitted-default vless url"),
    ));
    let vless_aliases =
        legacy_bound_proxy_key_aliases(&normalized_vless_proxy_url, ForwardProxyProtocol::Vless);
    assert!(vless_aliases.contains(&explicit_legacy_vless_proxy_key));
    assert!(vless_aliases.contains(&omitted_default_vless_proxy_key));
    assert!(
        legacy_bound_proxy_key_aliases(&normalized_vless_proxy_url, ForwardProxyProtocol::Trojan)
            .is_empty()
    );
    let stable_vless_proxy_key =
        normalize_single_proxy_key(explicit_vless_proxy_url).expect("stable vless proxy key");
    assert_ne!(explicit_legacy_vless_proxy_key, stable_vless_proxy_key);
    assert_ne!(omitted_default_vless_proxy_key, stable_vless_proxy_key);

    let mut vless_manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![explicit_vless_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut vless_manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11080").expect("parse synthesized vless endpoint url"),
        );
    }
    let vless_scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![omitted_default_vless_proxy_key.clone()],
    );
    let selected_vless = vless_manager
        .select_proxy_for_scope(&vless_scope)
        .expect("legacy vless bound key should still select proxy");
    assert_eq!(selected_vless.key, stable_vless_proxy_key);

    let explicit_trojan_proxy_url =
        "trojan://password@trojan.example.com:443?security=tls&type=tcp#东京节点";
    let normalized_trojan_proxy_url =
        normalize_share_link_scheme(explicit_trojan_proxy_url, "trojan")
            .expect("normalize trojan url");
    let explicit_legacy_trojan_proxy_key = {
        let parsed = Url::parse(&normalized_trojan_proxy_url).expect("parse normalized trojan url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let omitted_default_trojan_proxy_key =
        stable_forward_proxy_key(&canonical_share_link_identity(
            &Url::parse("trojan://password@trojan.example.com:443#东京节点")
                .expect("parse omitted-default trojan url"),
        ));
    let trojan_aliases =
        legacy_bound_proxy_key_aliases(&normalized_trojan_proxy_url, ForwardProxyProtocol::Trojan);
    assert!(trojan_aliases.contains(&explicit_legacy_trojan_proxy_key));
    assert!(trojan_aliases.contains(&omitted_default_trojan_proxy_key));
    assert!(
        legacy_bound_proxy_key_aliases(&normalized_trojan_proxy_url, ForwardProxyProtocol::Vless)
            .is_empty()
    );
    let stable_trojan_proxy_key =
        normalize_single_proxy_key(explicit_trojan_proxy_url).expect("stable trojan proxy key");
    assert_ne!(explicit_legacy_trojan_proxy_key, stable_trojan_proxy_key);
    assert_ne!(omitted_default_trojan_proxy_key, stable_trojan_proxy_key);

    let mut trojan_manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![explicit_trojan_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut trojan_manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11081").expect("parse synthesized trojan endpoint url"),
        );
    }
    let trojan_scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![omitted_default_trojan_proxy_key.clone()],
    );
    let selected_trojan = trojan_manager
        .select_proxy_for_scope(&trojan_scope)
        .expect("legacy trojan bound key should still select proxy");
    assert_eq!(selected_trojan.key, stable_trojan_proxy_key);
}

#[test]
fn legacy_vless_bound_proxy_keys_still_match_when_query_param_names_change() {
    let legacy_type_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=tls&type=ws&host=cdn.example.com&path=%2Fws&sni=edge.example.com&fingerprint=chrome#东京节点";
    let current_net_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=tls&net=ws&host=cdn.example.com&path=%2Fws&serverName=edge.example.com&fp=chrome#东京节点";
    let normalized_legacy_type_proxy_url =
        normalize_share_link_scheme(legacy_type_proxy_url, "vless")
            .expect("normalize legacy vless url");
    let normalized_current_net_proxy_url =
        normalize_share_link_scheme(current_net_proxy_url, "vless")
            .expect("normalize current vless url");

    let legacy_bound_proxy_key = {
        let parsed = Url::parse(&normalized_legacy_type_proxy_url)
            .expect("parse legacy normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(current_net_proxy_url)
        .expect("normalize stable vless proxy key");
    assert_ne!(legacy_bound_proxy_key, stable_proxy_key);

    let aliases = legacy_bound_proxy_key_aliases(
        &normalized_current_net_proxy_url,
        ForwardProxyProtocol::Vless,
    );
    assert!(
        aliases.contains(&legacy_bound_proxy_key),
        "legacy alias list should include the historical type=ws key"
    );

    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![current_net_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11082")
                .expect("parse synthesized synonym-compatible endpoint url"),
        );
    }

    assert!(
        manager.has_selectable_bound_proxy_keys(std::slice::from_ref(&legacy_bound_proxy_key)),
        "legacy key with synonymous query params should remain selectable"
    );

    let scope =
        ForwardProxyRouteScope::from_group_binding(Some("东京组"), vec![legacy_bound_proxy_key]);
    let selected = manager
        .select_proxy_for_scope(&scope)
        .expect("legacy bound key with synonymous query params should still route");
    assert_eq!(selected.key, stable_proxy_key);
}

#[test]
fn forward_proxy_manager_v2_clamps_persisted_runtime_weight_on_startup() {
    let proxy_url = "http://127.0.0.1:7890".to_string();
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    let manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![ForwardProxyRuntimeState {
            proxy_key: proxy_key.clone(),
            display_name: proxy_url.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: Some(proxy_url.clone()),
            weight: 99.0,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }],
        ForwardProxyAlgo::V2,
    );

    let manual_runtime = manager
        .runtime
        .get(&proxy_key)
        .expect("manual runtime should exist");
    assert_eq!(manual_runtime.weight, FORWARD_PROXY_V2_WEIGHT_MAX);
}

#[test]
fn forward_proxy_manager_v2_counts_only_selectable_positive_candidates() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "http://127.0.0.1:7891".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_keys = manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(selectable_keys.len(), 2);
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_keys[0])
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&selectable_keys[1])
        .expect("selectable runtime should exist")
        .weight = -5.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = 1.0;

    manager.ensure_non_zero_weight();

    let positive_selectable = selectable_keys
        .iter()
        .filter_map(|key| manager.runtime.get(key))
        .filter(|runtime| runtime.weight > 0.0)
        .count();
    assert_eq!(positive_selectable, 2);
}

#[test]
fn forward_proxy_manager_v2_probe_ignores_non_selectable_penalties() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("selectable endpoint should exist");
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_key)
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = -2.0;

    assert!(!manager.should_probe_penalized_proxy());
    assert!(manager.mark_probe_started().is_none());
}

#[test]
fn forward_proxy_manager_v2_success_with_high_latency_still_gains_weight() {
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
    let before = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;

    manager.record_attempt(&proxy_key, true, Some(45_000.0), false);

    let after = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;
    assert!(after > before, "v2 success should increase weight");
}

#[test]
fn forward_proxy_manager_v2_success_recovers_penalized_proxy() {
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
fn classify_invocation_failure_marks_downstream_closed_as_client_abort() {
    let result = classify_invocation_failure(
        Some("http_200"),
        Some("[downstream_closed] downstream closed while streaming upstream response"),
    );
    assert_eq!(result.failure_class, FailureClass::ClientAbort);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("downstream_closed"));
}

#[tokio::test]
async fn proxy_error_response_exposes_image_timeout_code_only_for_504() {
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
async fn persist_proxy_capture_runtime_record_preserves_warning_success_diagnostics() {
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
async fn persist_proxy_capture_record_preserves_warning_success_diagnostics() {
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
async fn persist_proxy_capture_runtime_record_keeps_response_size_when_preview_disabled() {
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
                write_fence: None,
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
async fn persist_proxy_capture_record_omits_response_preview_and_raw_path_when_disabled() {
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
                write_fence: None,
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
fn proxy_capture_invocation_status_keeps_downstream_close_as_client_abort_shape() {
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
fn proxy_capture_is_pure_downstream_close_requires_a_clean_upstream_success() {
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
fn proxy_capture_invocation_failure_kind_prefers_logical_stream_failure_over_disconnect() {
    let pure_downstream_closed =
        proxy_capture_is_pure_downstream_close(StatusCode::OK, false, true, true);
    assert!(!pure_downstream_closed);
    assert_eq!(
        proxy_capture_invocation_failure_kind(StatusCode::OK, false, true, pure_downstream_closed),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
}

#[test]
fn pool_capture_attempt_status_keeps_late_disconnect_after_logical_failure_as_http_failure() {
    let pure_downstream_closed =
        proxy_capture_is_pure_downstream_close(StatusCode::OK, false, true, true);
    assert_eq!(
        pool_capture_attempt_status(StatusCode::OK, false, true, pure_downstream_closed),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
}
