#[tokio::test]
pub(crate) async fn load_forward_proxy_runtime_states_maps_legacy_proxy_keys_to_stable_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#东京专线".to_string();
    let normalized_proxy = normalize_single_proxy_url(&proxy_url).expect("normalize proxy url");
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: normalized_proxy.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(normalized_proxy),
            weight: 0.42,
            success_ema: 0.8,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 1,
        },
    )
    .await
    .expect("persist legacy runtime state");

    let runtime = load_forward_proxy_runtime_states(&state.pool)
        .await
        .expect("load runtime states");
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0].proxy_key, stable_proxy_key);
    assert_eq!(runtime[0].weight, 0.42);
}

#[tokio::test]
pub(crate) async fn load_forward_proxy_runtime_states_maps_legacy_vless_hash_keys_from_current_settings()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线".to_string();
    save_forward_proxy_settings(
        &state.pool,
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
    )
    .await
    .expect("persist current forward proxy settings");

    let normalized_proxy =
        normalize_share_link_scheme(&proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    assert_ne!(legacy_proxy_key, stable_proxy_key);
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key,
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: None,
            weight: 0.42,
            success_ema: 0.8,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 1,
        },
    )
    .await
    .expect("persist legacy hashed runtime state");

    let runtime = load_forward_proxy_runtime_states(&state.pool)
        .await
        .expect("load runtime states");
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0].proxy_key, stable_proxy_key);
    assert_eq!(runtime[0].weight, 0.42);
}

#[tokio::test]
pub(crate) async fn forward_proxy_binding_nodes_reuse_legacy_pool_attempt_stats_for_stable_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#东京专线".to_string();
    let normalized_proxy = normalize_single_proxy_url(&proxy_url).expect("normalize proxy url");
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: stable_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(normalized_proxy.clone()),
            weight: 0.8,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state");

    let now_epoch = Utc::now().timestamp();
    let bucket_start_epoch = align_bucket_epoch(now_epoch, 3600, 0);
    for (invoke_id, status) in [
        (
            "binding-stable-legacy-success",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "binding-stable-legacy-failure",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            Utc.timestamp_opt(bucket_start_epoch + 300, 0)
                .single()
                .expect("stable pool attempt timestamp should be valid"),
            Some(&stable_proxy_key),
            status,
        )
        .await;
    }

    let nodes = build_forward_proxy_binding_nodes_response(
        state.as_ref(),
        std::slice::from_ref(&stable_proxy_key),
    )
    .await
    .expect("build binding nodes response");
    let node = nodes
        .into_iter()
        .find(|item| item.key == stable_proxy_key)
        .expect("stable node should be returned");
    let bucket = node
        .last24h
        .into_iter()
        .find(|item| item.success_count == 1 || item.failure_count == 1)
        .expect("matching bucket should exist");
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[test]
pub(crate) fn forward_proxy_manager_reuses_legacy_vless_hash_runtime_state() {
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线";
    let normalized_proxy =
        normalize_share_link_scheme(proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(proxy_url).expect("normalize proxy key");
    assert_ne!(legacy_proxy_key, stable_proxy_key);

    let manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.to_string()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: None,
            weight: 0.37,
            success_ema: 0.9,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 2,
        }],
    );

    let runtime = manager
        .runtime
        .get(&stable_proxy_key)
        .expect("stable runtime should be preserved");
    assert_eq!(runtime.weight, 0.37);
    assert_eq!(
        runtime.endpoint_url.as_deref(),
        Some(normalized_proxy.as_str())
    );
    assert!(!manager.runtime.contains_key(&legacy_proxy_key));
}

#[tokio::test]
pub(crate) async fn forward_proxy_binding_nodes_reuse_legacy_pool_attempt_stats_from_current_settings()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11082")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let normalized_proxy =
        normalize_share_link_scheme(&proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();

    let now_epoch = Utc::now().timestamp();
    let bucket_start_epoch = align_bucket_epoch(now_epoch, 3600, 0);
    for (invoke_id, status) in [
        (
            "binding-current-settings-legacy-success",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "binding-current-settings-legacy-failure",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            Utc.timestamp_opt(bucket_start_epoch + 300, 0)
                .single()
                .expect("legacy hashed pool attempt timestamp should be valid"),
            Some(&legacy_proxy_key),
            status,
        )
        .await;
    }

    let nodes = build_forward_proxy_binding_nodes_response(state.as_ref(), &[])
        .await
        .expect("build binding nodes response");
    let node = nodes
        .into_iter()
        .find(|item| item.key == binding_key)
        .expect("logical binding node should be returned");
    assert!(node.alias_keys.contains(&legacy_proxy_key));
    let bucket = node
        .last24h
        .into_iter()
        .find(|item| item.success_count == 1 || item.failure_count == 1)
        .expect("matching bucket should exist");
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
pub(crate) async fn forward_proxy_binding_nodes_map_historical_runtime_keys_to_current_logical_nodes()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let current_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fcurrent&sni=current.example.com#东京专线".to_string();
    let legacy_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Flegacy&sni=legacy.example.com#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![current_proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11081")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let legacy_proxy_key =
        normalize_single_proxy_key(&legacy_proxy_url).expect("normalize legacy runtime proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                normalize_share_link_scheme(&legacy_proxy_url, "vless")
                    .expect("normalize legacy share link"),
            ),
            weight: 0.55,
            success_ema: 0.78,
            latency_ema_ms: Some(180.0),
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state for metadata history");

    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&current_proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();
    let nodes = build_forward_proxy_binding_nodes_response(
        state.as_ref(),
        std::slice::from_ref(&legacy_proxy_key),
    )
    .await
    .expect("build binding nodes response");
    let current_node = nodes
        .iter()
        .find(|item| item.key == binding_key)
        .expect("current logical node should be returned");
    assert!(current_node.alias_keys.contains(&legacy_proxy_key));
    assert!(
        !nodes
            .iter()
            .any(|item| item.key == legacy_proxy_key && item.source == "missing"),
        "historical runtime key should fold into the current logical node instead of rendering as missing"
    );
}

#[test]
pub(crate) fn parse_proxy_urls_from_subscription_body_supports_xray_links() {
    let vmess_payload = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111"
    }))
    .expect("serialize vmess payload");
    let vmess_link = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload)
    );
    let vless_link =
        "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls";
    let subscription_raw = format!("{vmess_link}\n{vless_link}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(subscription_raw.as_bytes());
    let parsed = parse_proxy_urls_from_subscription_body(&encoded);
    assert!(parsed.iter().any(|item| item.starts_with("vmess://")));
    assert!(parsed.iter().any(|item| item.starts_with("vless://")));
}

#[test]
pub(crate) fn parse_shadowsocks_share_link_decodes_percent_encoded_credentials() {
    let link = format!(
        "{}{}{}",
        "ss://2022-blake3-aes-128-gcm:", "%2B%2F%3D", "@127.0.0.1:8388#ss%20node"
    );
    let parsed = parse_shadowsocks_share_link(&link).expect("parse ss2022 link");
    assert_eq!(parsed.method, "2022-blake3-aes-128-gcm");
    assert_eq!(parsed.password, "+/=");
    assert_eq!(parsed.display_name, "ss node");
    assert_eq!(parsed.host, "127.0.0.1");
    assert_eq!(parsed.port, 8388);
}

#[test]
pub(crate) fn parse_shadowsocks_share_link_decodes_percent_encoded_base64_userinfo() {
    let userinfo =
        base64::engine::general_purpose::STANDARD.encode("chacha20-ietf-poly1305:pass+/=");
    let link = format!("ss://{}@127.0.0.1:8388#node", userinfo.replace('=', "%3D"));
    let parsed = parse_shadowsocks_share_link(&link).expect("parse ss base64 userinfo link");
    assert_eq!(parsed.method, "chacha20-ietf-poly1305");
    assert_eq!(parsed.password, "pass+/=");
}

#[test]
pub(crate) fn decode_subscription_payload_supports_base64_blob() {
    let encoded = base64::engine::general_purpose::STANDARD
        .encode("http://127.0.0.1:7890\nsocks5://127.0.0.1:1080");
    let decoded = decode_subscription_payload(&encoded);
    assert!(decoded.contains("http://127.0.0.1:7890"));
    assert!(decoded.contains("socks5://127.0.0.1:1080"));
}

#[test]
pub(crate) fn forward_proxy_validation_timeout_is_split_by_kind() {
    assert_eq!(
        forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
        Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
    );
    assert_eq!(
        forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl),
        Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
    );
}

#[test]
pub(crate) fn remaining_timeout_budget_stops_when_elapsed_reaches_total() {
    let total = Duration::from_secs(60);
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(20)),
        Some(Duration::from_secs(40))
    );
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(60)),
        Some(Duration::ZERO)
    );
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(61)),
        None
    );
}

#[test]
pub(crate) fn timeout_budget_exhausted_treats_zero_budget_as_exhausted() {
    let total = Duration::from_secs(60);
    assert!(!timeout_budget_exhausted(total, Duration::from_secs(59)));
    assert!(timeout_budget_exhausted(total, Duration::from_secs(60)));
    assert!(timeout_budget_exhausted(total, Duration::from_secs(61)));
}

#[test]
pub(crate) fn timeout_seconds_for_message_rounds_subsecond_up_to_one() {
    assert_eq!(timeout_seconds_for_message(Duration::from_millis(1)), 1);
    assert_eq!(timeout_seconds_for_message(Duration::from_secs(5)), 5);
    assert_eq!(timeout_seconds_for_message(Duration::from_millis(5500)), 6);
}

#[test]
pub(crate) fn fallback_proxy_429_retry_delay_uses_exponential_backoff_with_cap() {
    assert_eq!(
        fallback_proxy_429_retry_delay(1),
        Duration::from_millis(500)
    );
    assert_eq!(fallback_proxy_429_retry_delay(2), Duration::from_secs(1));
    assert_eq!(fallback_proxy_429_retry_delay(3), Duration::from_secs(2));
    assert_eq!(fallback_proxy_429_retry_delay(4), Duration::from_secs(4));
    assert_eq!(fallback_proxy_429_retry_delay(5), Duration::from_secs(5));
    assert_eq!(fallback_proxy_429_retry_delay(9), Duration::from_secs(5));
}

#[test]
pub(crate) fn parse_retry_after_delay_supports_seconds_and_http_date() {
    let seconds = HeaderValue::from_static("2");
    assert_eq!(
        parse_retry_after_delay(&seconds),
        Some(Duration::from_secs(2))
    );

    let retry_at = Utc::now() + chrono::Duration::seconds(5);
    let imf_fixdate = retry_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let rfc850 = retry_at.format("%A, %d-%b-%y %H:%M:%S GMT").to_string();
    let asctime = retry_at.format("%a %b %e %H:%M:%S %Y").to_string();

    for raw in [imf_fixdate, rfc850, asctime] {
        let http_date = HeaderValue::from_str(&raw).expect("valid retry-after date header");
        let parsed =
            parse_retry_after_delay(&http_date).expect("http-date retry-after should parse");
        assert!(parsed >= Duration::from_secs(1));
        assert!(parsed <= Duration::from_secs(5));
    }
}

#[test]
pub(crate) fn parse_retry_after_delay_clamps_large_values() {
    let huge_seconds = HeaderValue::from_static("3600");
    assert_eq!(
        parse_retry_after_delay(&huge_seconds),
        Some(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS
        ))
    );

    let huge_date = (Utc::now() + chrono::Duration::seconds(3600))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let huge_header = HeaderValue::from_str(&huge_date).expect("valid retry-after date header");
    assert_eq!(
        parse_retry_after_delay(&huge_header),
        Some(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS
        ))
    );
}

#[test]
pub(crate) fn parse_retry_after_delay_rejects_invalid_or_past_values() {
    let invalid = HeaderValue::from_static("not-a-date");
    assert_eq!(parse_retry_after_delay(&invalid), None);

    let blank = HeaderValue::from_static("   ");
    assert_eq!(parse_retry_after_delay(&blank), None);

    let past = (Utc::now() - chrono::Duration::seconds(1))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let past_header = HeaderValue::from_str(&past).expect("valid past retry-after date header");
    assert_eq!(parse_retry_after_delay(&past_header), None);
}

#[test]
pub(crate) fn validation_probe_reachable_status_accepts_success_auth_and_not_found() {
    for status in [
        StatusCode::OK,
        StatusCode::NO_CONTENT,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            is_validation_probe_reachable_status(status),
            "status {status} should be reachable"
        );
    }
}

#[test]
pub(crate) fn validation_probe_reachable_status_rejects_non_reachable_codes() {
    for status in [
        StatusCode::PROXY_AUTHENTICATION_REQUIRED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
    ] {
        assert!(
            !is_validation_probe_reachable_status(status),
            "status {status} should not be reachable"
        );
    }
}

#[tokio::test]
pub(crate) async fn validate_proxy_url_candidate_accepts_probe_404() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let response = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url.clone())
        .await
        .expect("404 should be treated as reachable");

    assert!(response.ok);
    assert_eq!(response.message, "proxy validation succeeded");
    assert_eq!(
        response.normalized_value.as_deref(),
        Some(proxy_url.as_str())
    );
    assert_eq!(response.discovered_nodes, Some(1));
    assert!(
        response.latency_ms.unwrap_or_default() >= 0.0,
        "latency should be present"
    );

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_proxy_url_candidate_keeps_5xx_as_failure() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let err = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url)
        .await
        .expect_err("5xx should still fail validation");
    let message = format!("{err:#}");
    assert!(
        message.contains("validation probe returned status 500 Internal Server Error"),
        "expected 500 validation probe failure, got: {message}"
    );

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_accepts_probe_404() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let response = validate_subscription_candidate(state.clone(), subscription_url.clone())
        .await
        .expect("404 should be treated as reachable for subscription validation");

    assert!(response.ok);
    assert_eq!(response.message, "subscription validation succeeded");
    assert_eq!(
        response.normalized_value.as_deref(),
        Some(subscription_url.as_str())
    );
    assert_eq!(response.discovered_nodes, Some(1));
    assert!(
        response.latency_ms.unwrap_or_default() >= 0.0,
        "latency should be present"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_keeps_5xx_as_failure() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let err = validate_subscription_candidate(state.clone(), subscription_url)
        .await
        .expect_err("5xx should still fail subscription validation");
    let message = format!("{err:#}");
    assert!(
        message.contains("subscription validation scanned 1 proxy entries"),
        "expected subscription probe failure context, got: {message}"
    );
    assert!(
        message.contains("validation probe returned status 500 Internal Server Error"),
        "expected 500 validation probe failure, got: {message}"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_scans_beyond_first_three_nodes() {
    let mut handles = Vec::new();
    let mut body = String::new();
    for _ in 0..3 {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
        body.push_str(&proxy_url);
        body.push('\n');
        handles.push(proxy_handle);
    }
    let (available_proxy_url, available_proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    body.push_str(&available_proxy_url);
    body.push('\n');
    handles.push(available_proxy_handle);

    let (subscription_url, subscription_handle) = spawn_test_subscription_source(body).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let response = validate_subscription_candidate(state.clone(), subscription_url)
        .await
        .expect("available node after the first three entries should pass validation");

    assert!(response.ok);
    assert_eq!(response.discovered_nodes, Some(4));

    subscription_handle.abort();
    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_does_not_let_slow_first_node_block_success() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let release_request = Arc::new(Notify::new());
    let slow_app = Router::new().fallback({
        let request_count = request_count.clone();
        let release_request = release_request.clone();
        any(move || {
            let request_count = request_count.clone();
            let release_request = release_request.clone();
            async move {
                request_count.fetch_add(1, Ordering::SeqCst);
                release_request.notified().await;
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "status": StatusCode::NOT_FOUND.as_u16(),
                    })),
                )
            }
        })
    });
    let slow_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking forward proxy status test server");
    let slow_addr = slow_listener
        .local_addr()
        .expect("blocking forward proxy status test server addr");
    let slow_proxy_handle = tokio::spawn(async move {
        axum::serve(slow_listener, slow_app)
            .await
            .expect("blocking forward proxy status test server should run");
    });
    let slow_proxy_url = format!("http://{slow_addr}");
    let (available_proxy_url, available_proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{slow_proxy_url}\n{available_proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let started = Instant::now();
    let validation_task = tokio::spawn({
        let state = state.clone();
        let subscription_url = subscription_url.clone();
        async move { validate_subscription_candidate(state, subscription_url).await }
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while request_count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("slow first node should receive a request");

    release_request.notify_waiters();
    let response = validation_task
        .await
        .expect("validation task should finish")
        .expect("concurrent validation should pass through the available second node");

    assert!(response.ok);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "slow first node should not block concurrent subscription validation"
    );

    assert!(request_count.load(Ordering::SeqCst) >= 1);
    subscription_handle.abort();
    slow_proxy_handle.abort();
    available_proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_retries_each_node_at_most_three_times() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let (proxy_url, proxy_handle) = spawn_test_counting_forward_proxy_status(
        StatusCode::INTERNAL_SERVER_ERROR,
        request_count.clone(),
    )
    .await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let err = validate_subscription_candidate(state.clone(), subscription_url)
        .await
        .expect_err("all failed attempts should fail validation");
    let message = format!("{err:#}");

    assert_eq!(request_count.load(Ordering::SeqCst), 3);
    assert!(
        message.contains("3 attempts per entry"),
        "failure message should describe retry budget, got: {message}"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_limits_node_probe_concurrency_to_ten() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let total_requests = Arc::new(AtomicUsize::new(0));
    let release_request = Arc::new(Notify::new());
    let mut handles = Vec::new();
    let mut urls = Vec::new();
    for _ in 0..11 {
        let app = Router::new().fallback({
            let in_flight = in_flight.clone();
            let total_requests = total_requests.clone();
            let release_request = release_request.clone();
            any(move || {
                let in_flight = in_flight.clone();
                let total_requests = total_requests.clone();
                let release_request = release_request.clone();
                async move {
                    let request_index = total_requests.fetch_add(1, Ordering::SeqCst) + 1;
                    if request_index <= 10 {
                        in_flight.fetch_add(1, Ordering::SeqCst);
                        release_request.notified().await;
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                    }
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "status": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                        })),
                    )
                }
            })
        });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind blocking concurrency test server");
        let addr = listener
            .local_addr()
            .expect("blocking concurrency test server addr");
        handles.push(tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("blocking concurrency test server should run");
        }));
        urls.push(format!("http://{addr}"));
    }
    let subscription_body = urls.join("\n");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{subscription_body}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let validation_task = tokio::spawn({
        let state = state.clone();
        let subscription_url = subscription_url.clone();
        async move { validate_subscription_candidate(state, subscription_url).await }
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if total_requests.load(Ordering::SeqCst) >= 10 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first ten nodes should start probing");
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(in_flight.load(Ordering::SeqCst), 10);
    assert_eq!(
        total_requests.load(Ordering::SeqCst),
        10,
        "the eleventh node should wait for a concurrency slot"
    );

    release_request.notify_waiters();
    let err = validation_task
        .await
        .expect("validation task should finish")
        .expect_err("all nodes return 500");
    let message = format!("{err:#}");
    assert!(
        message.contains("concurrency 10"),
        "failure message should describe concurrency, got: {message}"
    );
    assert!(
        total_requests.load(Ordering::SeqCst) >= 11,
        "queued nodes should run after slots are released"
    );

    subscription_handle.abort();
    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
pub(crate) async fn validate_subscription_candidate_preserves_total_validation_deadline_during_scan()
 {
    let release_request = Arc::new(Notify::new());
    let app = Router::new().fallback({
        let release_request = release_request.clone();
        any(move || {
            let release_request = release_request.clone();
            async move {
                release_request.notified().await;
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "status": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    })),
                )
            }
        })
    });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking deadline test server");
    let addr = listener
        .local_addr()
        .expect("blocking deadline test server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("blocking deadline test server should run");
    });
    let proxy_url = format!("http://{addr}");
    let endpoints: Vec<_> = (0..11)
        .map(|index| ForwardProxyEndpoint {
            key: format!("deadline-{index}"),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            display_name: format!("deadline-{index}"),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&proxy_url).expect("valid deadline proxy url")),
            raw_url: Some(proxy_url.clone()),
        })
        .collect();
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let started = Instant::now();
    let err = validate_subscription_endpoints_concurrently(
        state,
        endpoints,
        Duration::from_millis(100),
        started,
    )
    .await
    .expect_err("total validation deadline should stop the scan");
    let elapsed = started.elapsed();
    let message = format!("{err:#}");

    assert!(
        elapsed < Duration::from_secs(2),
        "total validation deadline should stop in-flight probes, elapsed: {elapsed:?}"
    );
    assert!(
        message.contains("validation request timed out after 1s"),
        "deadline failure should use validation timeout wording, got: {message}"
    );

    release_request.notify_waiters();
    proxy_handle.abort();
}

use super::*;
