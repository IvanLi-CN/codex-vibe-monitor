#[tokio::test]
async fn refresh_forward_proxy_subscriptions_triggers_bootstrap_probe_for_added_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;

    refresh_forward_proxy_subscriptions(state.clone(), true, None)
        .await
        .expect("refresh subscriptions should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt from subscription refresh"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn refresh_forward_proxy_subscriptions_skips_probe_for_known_subscription_keys() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");

    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let known_keys = HashSet::from([proxy_key.clone()]);
    refresh_forward_proxy_subscriptions(state.clone(), true, Some(known_keys))
        .await
        .expect("refresh subscriptions should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let probe_count_after = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        probe_count_after, probe_count_before,
        "known subscription keys should suppress startup-style reprobe"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_bootstrap_probe_failure_penalizes_runtime_weight() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let failure_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let failure_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;
    assert!(
        failure_count > failure_count_before,
        "expected at least one failed bootstrap probe attempt"
    );

    let runtime_weight = read_forward_proxy_runtime_weight(&state.pool, &proxy_key)
        .await
        .expect("runtime weight should exist");
    assert!(
        runtime_weight < 1.0,
        "expected failed bootstrap probe to penalize runtime weight; got {runtime_weight}"
    );

    proxy_handle.abort();
}

#[test]
fn forward_proxy_manager_keeps_one_positive_weight() {
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    assert!(manager.runtime.values().any(|entry| entry.weight > 0.0));
}

#[test]
fn forward_proxy_algo_from_str_supports_v1_and_v2() {
    assert_eq!(
        ForwardProxyAlgo::from_str("v1").expect("v1 should parse"),
        ForwardProxyAlgo::V1
    );
    assert_eq!(
        ForwardProxyAlgo::from_str("V2").expect("v2 should parse"),
        ForwardProxyAlgo::V2
    );
    assert!(ForwardProxyAlgo::from_str("unexpected").is_err());
}

#[test]
fn forward_proxy_algo_config_defaults_to_latest_v2() {
    let algo = resolve_forward_proxy_algo_config(None, None).expect("default algo should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_accepts_primary_env() {
    let algo =
        resolve_forward_proxy_algo_config(Some("v2"), None).expect("primary env should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_rejects_legacy_env() {
    let err =
        resolve_forward_proxy_algo_config(None, Some("v1")).expect_err("legacy env should fail");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_algo_config_rejects_when_both_env_vars_are_set() {
    let err = resolve_forward_proxy_algo_config(Some("v2"), Some("v1"))
        .expect_err("legacy env should still win as a hard failure");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_manager_v2_keeps_two_positive_weights() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    let positive_count = manager
        .endpoints
        .iter()
        .filter_map(|endpoint| manager.runtime.get(&endpoint.key))
        .filter(|entry| entry.weight > 0.0)
        .count();
    assert_eq!(positive_count, 1);
}

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
        manager.has_selectable_bound_proxy_keys(&[legacy_bound_proxy_key.clone()]),
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

#[test]
fn proxy_capture_invocation_status_marks_downstream_closed_as_failed() {
    assert_eq!(
        proxy_capture_invocation_status(StatusCode::OK, false, true),
        "failed"
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

#[test]
fn should_prebuffer_for_body_sticky_probe_respects_memory_threshold() {
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

#[test]
fn classify_invocation_failure_marks_invalid_key_as_client_failure() {
    let result = classify_invocation_failure(Some("http_401"), Some("Invalid API key format"));
    assert_eq!(result.failure_class, FailureClass::ClientFailure);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("invalid_api_key"));
}

#[test]
fn classify_invocation_failure_marks_upstream_errors_as_service_failure() {
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
fn classify_invocation_failure_treats_running_and_pending_as_none() {
    for status in ["running", "pending"] {
        let result = classify_invocation_failure(Some(status), None);
        assert_eq!(result.failure_class, FailureClass::None);
        assert!(!result.is_actionable);
        assert_eq!(result.failure_kind, None);
    }
}

#[test]
fn classify_invocation_failure_marks_upstream_response_failed_as_service_failure() {
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
fn classify_invocation_failure_marks_http_429_as_service_failure() {
    let result = classify_invocation_failure(Some("http_429"), Some("rate limited"));
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("http_429"));
}

#[test]
fn resolve_failure_classification_recomputes_actionable_for_missing_legacy_class() {
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
fn resolve_failure_classification_overrides_legacy_default_none_for_failures() {
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
fn failure_scope_parse_defaults_to_service() {
    assert_eq!(
        FailureScope::parse(None).expect("default scope"),
        FailureScope::Service
    );
}

#[test]
fn failure_scope_parse_rejects_unknown_value() {
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
fn app_config_from_sources_ignores_removed_xyai_env_vars() {
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
fn app_config_from_sources_reads_database_path_env() {
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
fn startup_pending_attempt_recovery_skips_all_retention_run_once_modes() {
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
fn app_config_from_sources_rejects_legacy_database_path_env() {
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
fn app_config_from_sources_reads_renamed_public_envs() {
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
        (ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS, Some("33")),
        (ENV_QUOTA_SNAPSHOT_FULL_DAYS, Some("34")),
        (ENV_PROXY_RAW_COMPRESSION, Some("none")),
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
    assert_eq!(config.stats_source_snapshots_retention_days, 33);
    assert_eq!(config.quota_snapshot_full_days, 34);
    assert_eq!(config.proxy_raw_compression, RawCompressionCodec::None);
    assert_eq!(config.proxy_raw_hot_secs, 1234);
}

#[test]
fn app_config_from_sources_rejects_all_legacy_public_env_renames() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();

    for (legacy_name, canonical_name) in LEGACY_ENV_RENAMES {
        let mut cases = LEGACY_ENV_RENAMES
            .iter()
            .map(|(legacy, _)| (*legacy, None))
            .collect::<Vec<_>>();
        let target = cases
            .iter_mut()
            .find(|(name, _)| *name == *legacy_name)
            .expect("legacy env should be present in helper list");
        *target = (*legacy_name, Some("legacy-value"));
        let _env = EnvVarGuard::set(&cases);

        let err = AppConfig::from_sources(&CliArgs::default())
            .expect_err("legacy env should fail fast with a rename hint");
        assert_eq!(
            err.to_string(),
            format!("{legacy_name} is not supported; rename it to {canonical_name}")
        );
    }
}

#[test]
fn app_config_from_sources_uses_proxy_timeout_defaults() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
        ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for name in names {
        unsafe { env::remove_var(name) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout defaults should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS)
    );
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS)
    );
    assert_eq!(
        config.pool_upstream_responses_total_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS)
    );
}

#[test]
fn app_config_from_sources_reads_proxy_timeout_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
        ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    unsafe {
        env::set_var("OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS", "61");
        env::set_var("OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS", "181");
        env::set_var("OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS", "182");
        env::set_var(ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS, "183");
        env::set_var(ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS, "301");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout envs should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(61)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(181)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(182)
    );
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(183)
    );
    assert_eq!(
        config.pool_upstream_responses_total_timeout,
        Duration::from_secs(301)
    );
}

#[test]
fn app_config_from_sources_rejects_zero_pool_upstream_responses_attempt_timeout() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS, Some("0"))]);

    let err = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero responses attempt timeout should be rejected");
    assert_eq!(
        err.to_string(),
        format!("{ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS} must be greater than 0")
    );
}

#[test]
fn app_config_from_sources_rejects_zero_pool_upstream_responses_total_timeout() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS, Some("0"))]);

    let err = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero responses total timeout should be rejected");
    assert_eq!(
        err.to_string(),
        format!("{ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS} must be greater than 0")
    );
}

fn test_config() -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        poll_interval: Duration::from_secs(10),
        request_timeout: Duration::from_secs(30),
        pool_upstream_responses_attempt_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ),
        pool_upstream_responses_total_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
        ),
        openai_proxy_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_compact_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_request_read_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
        ),
        openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        proxy_request_concurrency_limit: DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT,
        proxy_request_concurrency_wait_timeout: Duration::from_millis(
            DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS,
        ),
        proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
        proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
        proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
        proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
        xray_binary: DEFAULT_XRAY_BINARY.to_string(),
        xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
        forward_proxy_algo: ForwardProxyAlgo::V1,
        max_parallel_polls: 2,
        shared_connection_parallelism: 1,
        http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
        cors_allowed_origins: Vec::new(),
        list_limit_max: 100,
        user_agent: "codex-test".to_string(),
        static_dir: None,
        retention_enabled: DEFAULT_RETENTION_ENABLED,
        retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
        retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
        retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
        retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
        archive_dir: PathBuf::from("target/archive-tests"),
        codex_invocation_archive_layout: DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT,
        codex_invocation_archive_segment_granularity:
            DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
        invocation_archive_codec: DEFAULT_INVOCATION_ARCHIVE_CODEC,
        invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
        invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
        invocation_archive_ttl_days: DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
        forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_retention_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_archive_ttl_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
        stats_source_snapshots_retention_days: DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
        quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        crs_stats: None,
        upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID.to_string(),
        upstream_accounts_oauth_issuer: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
            .expect("valid oauth issuer"),
        upstream_accounts_usage_base_url: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL)
            .expect("valid usage base url"),
        upstream_accounts_login_session_ttl: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
        ),
        upstream_accounts_sync_interval: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
        ),
        upstream_accounts_refresh_lead_time: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
        ),
        upstream_accounts_history_retention_days: DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        upstream_accounts_moemail: None,
    }
}

fn make_temp_test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn set_file_mtime_seconds_ago(path: &Path, seconds: u64) {
    let modified_at = std::time::SystemTime::now() - Duration::from_secs(seconds);
    let modified_at = filetime::FileTime::from_system_time(modified_at);
    filetime::set_file_mtime(path, modified_at).expect("set file mtime");
}

fn write_gzip_test_file(path: &Path, content: &[u8]) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content).expect("write gzip payload");
    let bytes = encoder.finish().expect("finish gzip payload");
    fs::write(path, bytes).expect("write gzip file");
}

#[test]
fn archive_batch_file_path_resolves_relative_archive_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.archive_dir = PathBuf::from("archives");

    let path = archive_batch_file_path(&config, "codex_invocations", "2026-03")
        .expect("resolve archive batch path");

    assert_eq!(
        path,
        PathBuf::from(
            "/tmp/codex-retention/archives/codex_invocations/2026/codex_invocations-2026-03.sqlite.gz",
        )
    );
}

#[test]
fn resolved_proxy_raw_dir_resolves_relative_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    assert_eq!(
        config.resolved_proxy_raw_dir(),
        PathBuf::from("/tmp/codex-retention/proxy_raw_payloads")
    );
}

#[test]
fn store_raw_payload_file_anchors_relative_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-store-db-parent");
    let cwd = temp_dir.join("cwd");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&cwd).expect("create cwd dir");
    fs::create_dir_all(&db_root).expect("create db root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-test",
        "request",
        b"{\"ok\":true}",
    ));
    let expected = db_root.join("proxy_raw_payloads/proxy-test-request.bin");

    assert_eq!(
        meta.path.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert!(
        expected.exists(),
        "raw payload should be written beside the database"
    );
    assert!(
        !cwd.join("proxy_raw_payloads/proxy-test-request.bin")
            .exists(),
        "raw payload should not follow the current working directory"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_keeps_current_dir_compat_for_legacy_relative_paths() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-read-legacy-cwd");
    let cwd = temp_dir.join("cwd");
    let fallback_root = temp_dir.join("fallback");
    let relative_path = PathBuf::from("proxy_raw_payloads/legacy-request.bin");
    let cwd_path = cwd.join(&relative_path);
    let fallback_path = fallback_root.join(&relative_path);
    fs::create_dir_all(cwd_path.parent().expect("cwd parent")).expect("create cwd raw dir");
    fs::create_dir_all(fallback_path.parent().expect("fallback parent"))
        .expect("create fallback raw dir");
    fs::write(&cwd_path, b"cwd-copy").expect("write cwd raw file");
    fs::write(&fallback_path, b"fallback-copy").expect("write fallback raw file");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let raw = read_proxy_raw_bytes(
        relative_path.to_str().expect("utf-8 path"),
        Some(&fallback_root),
    )
    .expect("read legacy cwd-relative raw file");

    assert_eq!(raw, b"cwd-copy");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_transparently_decompresses_gzip_files() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-gzip");
    let raw_path = temp_dir.join("request.bin.gz");
    write_gzip_test_file(&raw_path, b"{\"hello\":\"gzip\"}");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read gzip raw payload");

    assert_eq!(raw, b"{\"hello\":\"gzip\"}");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_keeps_plain_bin_payloads_that_start_with_gzip_magic() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-bin-gzip-magic");
    let raw_path = temp_dir.join("request.bin");
    let bytes = vec![0x1f, 0x8b, b'n', b'o', b't', b'-', b'g', b'z'];
    fs::write(&raw_path, &bytes).expect("write plain raw payload");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read plain raw payload");

    assert_eq!(raw, bytes);
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_matches_plain_and_gzip_files() {
    let temp_dir = make_temp_test_dir("search-raw-script");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    let gzip_path = root.join("cold.bin.gz");
    fs::write(&plain_path, b"line-1\nshared-token\n").expect("write plain raw");
    write_gzip_test_file(&gzip_path, b"line-a\nshared-token\n");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("shared-token")
    .output()
    .expect("run search-raw script");

    assert!(
        output.status.success(),
        "search-raw should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:2:shared-token", plain_path.display())),
        "plain raw file should match, got: {stdout}"
    );
    assert!(
        stdout.contains(&format!("{}:2:shared-token", gzip_path.display())),
        "gzip raw file should match, got: {stdout}"
    );

    let miss_output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("absent-token")
    .output()
    .expect("run search-raw miss case");
    assert_eq!(
        miss_output.status.code(),
        Some(1),
        "search-raw should return 1 when no file matches"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_resolves_root_from_database_and_proxy_envs() {
    let temp_dir = make_temp_test_dir("search-raw-script-env-root");
    let db_root = temp_dir.join("db");
    let db_path = db_root.join("codex_vibe_monitor.db");
    let raw_root = db_root.join("proxy_raw_payloads");
    fs::create_dir_all(&raw_root).expect("create resolved raw root");
    fs::write(&db_path, "").expect("create db file");
    let plain_path = raw_root.join("resolved.bin");
    fs::write(&plain_path, b"resolved-token\n").expect("write resolved raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("resolved-token")
    .output()
    .expect("run search-raw with env-derived root");

    assert!(
        output.status.success(),
        "search-raw should resolve root from envs: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:1:resolved-token", plain_path.display())),
        "env-derived root should find the plain file, got: {stdout}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_reports_missing_root_as_configuration_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-missing-root");
    let db_path = temp_dir.join("missing/codex_vibe_monitor.db");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("anything")
    .output()
    .expect("run search-raw with missing root");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing root should be treated as configuration error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("root directory not found"),
        "missing root should explain the configuration error, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_reports_corrupt_gzip_as_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-corrupt-gzip");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let gzip_path = root.join("broken.bin.gz");
    fs::write(&gzip_path, b"not-gzip").expect("write corrupt gzip file");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("needle")
    .output()
    .expect("run search-raw with corrupt gzip");

    assert_eq!(
        output.status.code(),
        Some(2),
        "corrupt gzip should be treated as hard error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to decompress"),
        "corrupt gzip should explain the decompression failure, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
fn search_raw_script_reports_plain_file_read_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-plain-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    fs::write(&plain_path, b"permission-token\n").expect("write plain raw");

    let mut permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&plain_path, permissions).expect("chmod plain raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable plain file");

    let mut repaired_permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o644);
    fs::set_permissions(&plain_path, repaired_permissions).expect("restore plain raw permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "plain grep errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("grep failed"),
        "plain grep failure should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
fn search_raw_script_reports_find_enumeration_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-find-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    let readable_dir = root.join("readable");
    let blocked_dir = root.join("blocked");
    fs::create_dir_all(&readable_dir).expect("create readable raw dir");
    fs::create_dir_all(&blocked_dir).expect("create blocked raw dir");
    fs::write(readable_dir.join("plain.bin"), b"permission-token\n").expect("write readable raw");

    let mut permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&blocked_dir, permissions).expect("chmod blocked dir");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable directory");

    let mut repaired_permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o755);
    fs::set_permissions(&blocked_dir, repaired_permissions)
        .expect("restore blocked dir permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "find enumeration errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to enumerate raw files"),
        "find failures should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

fn sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

async fn retention_test_pool_and_config(prefix: &str) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create retention sqlite file");
    let db_url = sqlite_url_for_path(&db_path);
    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("connect retention sqlite");
    ensure_schema(&pool).await.expect("ensure retention schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    config.invocation_archive_ttl_days = 365;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

fn cleanup_temp_test_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn current_process_rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn shanghai_local_days_ago(days: i64, hour: u32, minute: u32, second: u32) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(naive)
}

fn shanghai_local_now_minus_secs(secs: i64) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai).naive_local();
    format_naive(now_local - ChronoDuration::seconds(secs))
}

fn utc_naive_from_shanghai_local_days_ago(
    days: i64,
    hour: u32,
    minute: u32,
    second: u32,
) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let local_naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(local_naive_to_utc(local_naive, Shanghai).naive_utc())
}

#[allow(clippy::too_many_arguments)]
async fn insert_retention_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    source: &str,
    status: &str,
    payload: Option<&str>,
    raw_response: &str,
    request_raw_path: Option<&Path>,
    response_raw_path: Option<&Path>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            status,
            payload,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            response_raw_path,
            response_raw_codec,
            response_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(Some("gpt-5.2-codex"))
    .bind(Some(12_i64))
    .bind(Some(3_i64))
    .bind(total_tokens)
    .bind(cost)
    .bind(status)
    .bind(payload)
    .bind(raw_response)
    .bind(request_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        request_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        request_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .bind(response_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        response_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        response_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .execute(pool)
    .await
    .expect("insert retention invocation");
}

#[allow(clippy::too_many_arguments)]
async fn insert_retention_pool_upstream_request_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: &str,
    http_status: Option<i64>,
    failure_kind: Option<&str>,
    started_at: Option<&str>,
    finished_at: Option<&str>,
) {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
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
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        )
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind("/v1/responses")
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(Some("sticky-retention"))
    .bind(upstream_account_id)
    .bind(None::<String>)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(Some("203.0.113.1"))
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(http_status)
    .bind(failure_kind)
    .bind(Some("retention test"))
    .bind(Some(12.5_f64))
    .bind(Some(6.2_f64))
    .bind(Some(30.0_f64))
    .bind(Some("req_retention"))
    .execute(pool)
    .await
    .expect("insert retention pool attempt");
}

async fn insert_stats_source_snapshot_row(pool: &SqlitePool, captured_at: &str, stats_date: &str) {
    sqlx::query(
        r#"
        INSERT INTO stats_source_snapshots (
            source,
            period,
            stats_date,
            model,
            requests,
            input_tokens,
            output_tokens,
            cache_create_tokens,
            cache_read_tokens,
            all_tokens,
            cost_input,
            cost_output,
            cost_cache_write,
            cost_cache_read,
            cost_total,
            raw_response,
            captured_at,
            captured_at_epoch
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind("daily")
    .bind(stats_date)
    .bind(Some("gpt-5.2"))
    .bind(4_i64)
    .bind(10_i64)
    .bind(6_i64)
    .bind(0_i64)
    .bind(0_i64)
    .bind(16_i64)
    .bind(0.1_f64)
    .bind(0.2_f64)
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind(0.3_f64)
    .bind("{}")
    .bind(captured_at)
    .bind(
        parse_utc_naive(captured_at)
            .expect("valid utc naive")
            .and_utc()
            .timestamp(),
    )
    .execute(pool)
    .await
    .expect("insert stats source snapshot row");
}

#[tokio::test]
async fn ensure_schema_backfills_raw_codecs_and_manifest_tables() {
    let temp_dir = make_temp_test_dir("ensure-schema-raw-codecs");
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create schema sqlite file");
    let pool = SqlitePool::connect(&sqlite_url_for_path(&db_path))
        .await
        .expect("connect schema sqlite");

    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            raw_response TEXT NOT NULL,
            request_raw_path TEXT,
            response_raw_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy codex_invocations");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            raw_response,
            request_raw_path,
            response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("legacy-codec-row")
    .bind("2026-03-01 08:00:00")
    .bind("{}")
    .bind("proxy_raw_payloads/request.bin")
    .bind("proxy_raw_payloads/response.bin.gz")
    .execute(&pool)
    .await
    .expect("insert legacy codec row");

    ensure_schema(&pool).await.expect("ensure schema migration");

    let row = sqlx::query(
        "SELECT request_raw_codec, response_raw_codec FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("legacy-codec-row")
    .fetch_one(&pool)
    .await
    .expect("load migrated codec row");
    assert_eq!(
        row.get::<String, _>("request_raw_codec"),
        RAW_CODEC_IDENTITY
    );
    assert_eq!(row.get::<String, _>("response_raw_codec"), RAW_CODEC_GZIP);

    let archive_batch_columns = load_sqlite_table_columns(&pool, "archive_batches")
        .await
        .expect("load archive batch columns");
    assert!(archive_batch_columns.contains("upstream_activity_manifest_refreshed_at"));
    let manifest_columns = load_sqlite_table_columns(&pool, "archive_batch_upstream_activity")
        .await
        .expect("load manifest columns");
    assert!(manifest_columns.contains("archive_batch_id"));
    assert!(manifest_columns.contains("account_id"));
    assert!(manifest_columns.contains("last_activity_at"));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_compression_budget_stops_after_first_batch_when_budget_is_exhausted() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-catchup-budget").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.retention_batch_rows = 1;

    for (invoke_id, hour, file_name) in [
        ("budget-oldest", 8, "budget-oldest.bin"),
        ("budget-middle", 9, "budget-middle.bin"),
        ("budget-newest", 10, "budget-newest.bin"),
    ] {
        let raw_path = config.proxy_raw_dir.join(file_name);
        fs::write(&raw_path, invoke_id.as_bytes()).expect("write budget raw file");
        insert_retention_invocation(
            &pool,
            invoke_id,
            &shanghai_local_days_ago(2, hour, 0, 0),
            SOURCE_PROXY,
            "failed",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":false}",
            Some(&raw_path),
            None,
            Some(10),
            Some(0.01),
        )
        .await;
    }

    let first_pass = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        Some(Duration::ZERO),
    )
    .await
    .expect("run raw compression with zero catchup budget");
    assert_eq!(first_pass.files_considered, 1);
    assert_eq!(first_pass.files_compressed, 1);

    let remaining_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after first pass");
    assert_eq!(remaining_after_first, 2);

    let catchup = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        None,
    )
    .await
    .expect("run unrestricted raw compression catchup");
    assert_eq!(catchup.files_considered, 2);
    assert_eq!(catchup.files_compressed, 2);

    let remaining_after_catchup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after catchup");
    assert_eq!(remaining_after_catchup, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn fetch_stats_exposes_maintenance_observability_fields() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(812_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Stats maintenance account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert stats maintenance account");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("stats-maintenance-row")
    .bind(shanghai_local_days_ago(3, 8, 0, 0))
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.1_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/stats-maintenance.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(2048_i64)
    .execute(&state.pool)
    .await
    .expect("insert stats maintenance invocation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("stats-maintenance-hot-row")
    .bind(shanghai_local_now_minus_secs(20 * 60))
    .bind(SOURCE_PROXY)
    .bind(6_i64)
    .bind(0.05_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/stats-maintenance-hot.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(4096_i64)
    .execute(&state.pool)
    .await
    .expect("insert hot raw invocation that should not count as backlog");

    let Json(stats) = fetch_stats(State(state))
        .await
        .expect("fetch stats with maintenance payload");
    let maintenance = stats.maintenance.expect("stats maintenance payload");
    let raw_backlog = maintenance
        .raw_compression_backlog
        .expect("raw backlog maintenance payload");
    assert_eq!(raw_backlog.uncompressed_count, 1);
    assert_eq!(raw_backlog.uncompressed_bytes, 2048);
    assert_eq!(raw_backlog.alert_level, RawCompressionAlertLevel::Critical);
    let startup_backfill = maintenance
        .startup_backfill
        .expect("startup backfill maintenance payload");
    assert_eq!(
        startup_backfill.upstream_activity_archive_pending_accounts,
        1
    );
    assert_eq!(startup_backfill.zero_update_streak, 0);
    assert!(startup_backfill.next_run_after.is_none());
    let historical_rollup_backfill = maintenance
        .historical_rollup_backfill
        .expect("historical rollup backfill maintenance payload");
    assert_eq!(historical_rollup_backfill.pending_buckets, 0);
    assert_eq!(historical_rollup_backfill.legacy_archive_pending, 0);
    assert!(historical_rollup_backfill.last_materialized_hour.is_none());
    assert_eq!(
        historical_rollup_backfill.alert_level,
        HistoricalRollupBackfillAlertLevel::None
    );
}

#[tokio::test]
async fn fetch_stats_reuses_cached_maintenance_snapshot_within_ttl() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(813_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Cached maintenance account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert cached maintenance account");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("cached-maintenance-row")
    .bind(shanghai_local_days_ago(3, 9, 30, 0))
    .bind(SOURCE_PROXY)
    .bind(10_i64)
    .bind(0.2_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/cached-maintenance.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(4096_i64)
    .execute(&state.pool)
    .await
    .expect("insert cached maintenance invocation");

    let Json(first_stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch first stats maintenance snapshot");

    sqlx::query("UPDATE codex_invocations SET request_raw_codec = ?1 WHERE invoke_id = ?2")
        .bind(RAW_CODEC_GZIP)
        .bind("cached-maintenance-row")
        .execute(&state.pool)
        .await
        .expect("mark cached maintenance invocation compressed");
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, 0, ?2, 4, NULL, NULL, 0, 0, ?3)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_status = excluded.last_status
        "#,
    )
    .bind(STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES)
    .bind(format_utc_iso(Utc::now() + ChronoDuration::hours(1)))
    .bind(STARTUP_BACKFILL_STATUS_OK)
    .execute(&state.pool)
    .await
    .expect("update cached startup progress");

    let Json(second_stats) = fetch_stats(State(state))
        .await
        .expect("fetch cached stats maintenance snapshot");
    assert_eq!(second_stats.maintenance, first_stats.maintenance);
}

#[derive(Debug)]
struct FakeSqliteCodeDatabaseError {
    message: &'static str,
    code: &'static str,
}

impl std::fmt::Display for FakeSqliteCodeDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FakeSqliteCodeDatabaseError {}

impl DatabaseError for FakeSqliteCodeDatabaseError {
    fn message(&self) -> &str {
        self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.code))
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

fn write_backfill_response_payload(path: &Path) {
    write_backfill_response_payload_with_service_tier(path, None);
}
