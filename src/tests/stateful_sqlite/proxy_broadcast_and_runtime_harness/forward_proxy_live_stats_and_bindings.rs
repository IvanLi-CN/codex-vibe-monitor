use super::*;

#[tokio::test]
async fn forward_proxy_live_stats_returns_fixed_24_hour_buckets_with_zero_fill() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    let manual_runtime_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");
    let manual_binding_key = {
        let manager = state.forward_proxy.lock().await;
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("manual binding node should exist")
    };

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        range_start_epoch - 3600,
        4,
        0.25,
        0.42,
        0.34,
        0.35,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        range_start_epoch + 5 * 3600,
        2,
        0.45,
        0.82,
        0.61,
        0.80,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        range_start_epoch + 10 * 3600,
        1,
        1.20,
        1.20,
        1.20,
        1.20,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-success",
        Utc.timestamp_opt(range_start_epoch + 5 * 3600 + 300, 0)
            .single()
            .expect("manual success timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-failure",
        Utc.timestamp_opt(range_start_epoch + 10 * 3600 + 300, 0)
            .single()
            .expect("manual failure timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-out-of-range-success",
        Utc.timestamp_opt(range_start_epoch - 3600 + 300, 0)
            .single()
            .expect("manual out-of-range timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state.clone()))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.nodes.len(), 2);

    for node in &response.nodes {
        assert_eq!(
            node.last24h.len(),
            24,
            "node {} should include fixed 24 buckets",
            node.key
        );
        assert_eq!(
            node.weight24h.len(),
            24,
            "node {} should include fixed 24 weight buckets",
            node.key
        );
        assert_eq!(response.range_end, node.last24h[23].bucket_end);
        assert_eq!(response.range_start, node.last24h[0].bucket_start);
    }

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should be present");
    let manual_success_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum();
    let manual_failure_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum();
    let manual_zero_buckets = manual
        .last24h
        .iter()
        .filter(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
        .count();
    assert_eq!(
        manual_success_total, 1,
        "out-of-range attempts should be excluded"
    );
    assert!(
        manual_failure_total >= 1,
        "expected at least one in-range failure attempt"
    );
    assert!(
        manual_zero_buckets >= 21,
        "expected most buckets to be zero-filled, got {manual_zero_buckets}"
    );
    assert!(
        manual
            .weight24h
            .iter()
            .any(|bucket| bucket.sample_count == 0 && (bucket.last_weight - 0.35).abs() < 1e-6)
    );

    let direct = response
        .nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct node should be present");
    assert!(
        direct
            .last24h
            .iter()
            .all(|bucket| bucket.success_count == 0 && bucket.failure_count == 0),
        "direct node should remain zero-filled without direct attempts"
    );
    assert_eq!(direct.weight24h.len(), 24);

    let sampled_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 2
                && (bucket.min_weight - 0.45).abs() < 1e-6
                && (bucket.max_weight - 0.82).abs() < 1e-6
                && (bucket.avg_weight - 0.61).abs() < 1e-6
                && (bucket.last_weight - 0.80).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with aggregated stats");
    let sampled_bucket_carry = manual
        .weight24h
        .get(sampled_bucket_index + 1)
        .expect("expected carry-forward bucket after sampled manual weight bucket");
    assert_eq!(sampled_bucket_carry.sample_count, 0);
    assert!((sampled_bucket_carry.last_weight - 0.80).abs() < 1e-6);

    let recovered_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 1
                && (bucket.min_weight - 1.20).abs() < 1e-6
                && (bucket.max_weight - 1.20).abs() < 1e-6
                && (bucket.avg_weight - 1.20).abs() < 1e-6
                && (bucket.last_weight - 1.20).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with recovered last value");
    let recovered_bucket_carry = manual
        .weight24h
        .get(recovered_bucket_index + 1)
        .expect("expected carry-forward bucket after recovered manual weight bucket");
    assert_eq!(recovered_bucket_carry.sample_count, 0);
    assert!((recovered_bucket_carry.last_weight - 1.20).abs() < 1e-6);

    let display_names = response
        .nodes
        .iter()
        .map(|node| node.display_name.as_str())
        .collect::<Vec<_>>();
    let mut sorted_display_names = display_names.clone();
    sorted_display_names.sort();
    assert_eq!(
        display_names, sorted_display_names,
        "live stats nodes should stay display-name sorted"
    );
}

#[tokio::test]
async fn forward_proxy_binding_nodes_preserve_direct_hourly_buckets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "binding-direct-success",
        Utc.timestamp_opt(range_start_epoch + 5 * 3600 + 300, 0)
            .single()
            .expect("direct success timestamp should be valid"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "binding-direct-failure",
        Utc.timestamp_opt(range_start_epoch + 10 * 3600 + 300, 0)
            .single()
            .expect("direct failure timestamp should be valid"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "binding-direct-out-of-range-success",
        Utc.timestamp_opt(range_start_epoch - 3600 + 300, 0)
            .single()
            .expect("direct out-of-range timestamp should be valid"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let extra_proxy_keys = Vec::<String>::new();
    let nodes = build_forward_proxy_binding_nodes_response(state.as_ref(), &extra_proxy_keys)
        .await
        .expect("build forward proxy binding nodes should succeed");
    let direct = nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct binding node should be present");

    assert_eq!(direct.protocol_label, "DIRECT");
    assert_eq!(direct.last24h.len(), 24);
    assert_eq!(
        direct
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        1,
        "in-range direct successes should remain visible",
    );
    assert_eq!(
        direct
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
        1,
        "in-range direct failures should remain visible",
    );
}

#[tokio::test]
async fn forward_proxy_live_stats_returns_empty_nodes_when_no_endpoints_are_configured() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.nodes.len(), 1);
    let direct = &response.nodes[0];
    assert_eq!(direct.key, FORWARD_PROXY_DIRECT_KEY);
    assert_eq!(direct.display_name, "Direct");
    assert_eq!(direct.last24h.len(), 24);
    assert_eq!(direct.weight24h.len(), 24);
    assert!(
        direct
            .last24h
            .iter()
            .all(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
    );
}

pub(crate) async fn seed_long_term_invocation_source_for_pool_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
) {
    let occurred_at = format_naive(occurred_at.with_timezone(&Shanghai).naive_local());
    insert_retention_invocation(
        pool,
        invoke_id,
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some(r#"{"upstreamAccountId":41}"#),
        r#"{"ok":true}"#,
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
}
