#[tokio::test]
pub(crate) async fn proxy_model_settings_api_allows_forwarded_host_origin_match() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded host write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
pub(crate) async fn proxy_model_settings_api_allows_forwarded_port_non_default_origin_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com:8443"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-port"),
        HeaderValue::from_static("8443"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded port write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
pub(crate) async fn proxy_model_settings_api_allows_matching_origin_without_explicit_host_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("same-origin write without explicit host port should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
pub(crate) async fn forward_proxy_live_stats_returns_fixed_24_hour_buckets_with_zero_fill() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1080").await;

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_fixed_live_stats(
        &state,
        &manual_runtime_key,
        &manual_binding_key,
        range_start_epoch,
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state.clone()))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_fixed_live_stats_basics(&response, &manual_runtime_key);

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

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should be present");
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
pub(crate) async fn forward_proxy_binding_nodes_preserve_direct_hourly_buckets() {
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
pub(crate) async fn forward_proxy_live_stats_returns_empty_nodes_when_no_endpoints_are_configured()
{
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
    insert_retention_invocation_with_fixture(
        pool,
        RetentionInvocationFixture {
            invoke_id,
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            payload: Some(r#"{"upstreamAccountId":41}"#),
            raw_response: r#"{"ok":true}"#,
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(42),
            cost: Some(0.42),
        },
    )
    .await;
}

async fn seed_historical_forward_proxy_attempts(
    state: &AppState,
    manual_runtime_key: &str,
    manual_binding_key: &str,
    historical_bucket_start: i64,
) {
    let historical_attempt_at = Utc
        .timestamp_opt(historical_bucket_start + 5 * 60, 0)
        .single()
        .expect("historical attempt timestamp should be valid");
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        ForwardProxyWeightBucket {
            proxy_key: manual_runtime_key,
            bucket_start_epoch: historical_bucket_start,
            sample_count: 2,
            min_weight: 0.55,
            max_weight: 0.75,
            avg_weight: 0.65,
            last_weight: 0.70,
        },
    )
    .await;
    for (invoke_id, minutes, status) in [
        (
            "forward-proxy-timeseries-historical-success",
            10,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-timeseries-historical-failure",
            20,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        let occurred_at = historical_attempt_at + ChronoDuration::minutes(minutes);
        seed_long_term_invocation_source_for_pool_attempt(&state.pool, invoke_id, occurred_at)
            .await;
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            occurred_at,
            Some(manual_binding_key),
            status,
        )
        .await;
    }
}

async fn assert_historical_forward_proxy_retention(
    state: &AppState,
    manual_binding_key: &str,
    historical_bucket_start: i64,
) {
    let mut retention_config = state.config.clone();
    retention_config.invocation_success_full_days = 400;
    retention_config.invocation_max_days = 400;
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let summary = run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 2);
    let hourly_rollup_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_node_health_hourly_archive")
            .fetch_one(&state.pool)
            .await
            .expect("count materialized pool upstream hourly archive rows");
    assert!(hourly_rollup_rows >= 1);
    refresh_long_term_stats(&state.pool, 400)
        .await
        .expect("materialize long-term stats before archive cleanup");
    let long_term_deleted = cleanup_expired_archive_batches(&state.pool, &retention_config, false)
        .await
        .expect("cleanup archive after long-term stats materialization");
    assert_eq!(long_term_deleted, 1);
    assert_eq!(summary.archive_batches_deleted, 0);
    let remaining_archive_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining archived pool upstream request attempt batches");
    assert_eq!(remaining_archive_batches, 0);
    let live_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE proxy_binding_key_snapshot = ?1 AND occurred_at < ?2
        "#,
    )
    .bind(manual_binding_key)
    .bind(
        Utc.timestamp_opt(historical_bucket_start + 3_600, 0)
            .single()
            .expect("historical bucket end should be valid")
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
    .fetch_one(&state.pool)
    .await
    .expect("count live forward proxy attempts");
    assert_eq!(live_count, 0);
}

fn assert_historical_forward_proxy_response(
    response: &ForwardProxyTimeseriesResponse,
    manual_runtime_key: &str,
    historical_bucket_start: i64,
) {
    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.effective_bucket, "1h");
    assert_eq!(response.available_buckets, vec!["1h".to_string()]);
    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    let bucket_start = format_utc_iso(
        Utc.timestamp_opt(historical_bucket_start, 0)
            .single()
            .expect("historical bucket start should be valid"),
    );
    let request_bucket = manual
        .buckets
        .iter()
        .find(|bucket| bucket.bucket_start == bucket_start)
        .expect("historical request bucket should be present");
    assert_eq!(request_bucket.success_count, 1);
    assert_eq!(request_bucket.failure_count, 1);
    let weight_bucket = manual
        .weight_buckets
        .iter()
        .find(|bucket| bucket.bucket_start == bucket_start)
        .expect("historical weight bucket should be present");
    assert_eq!(weight_bucket.sample_count, 2);
    assert_f64_close(weight_bucket.min_weight, 0.55);
    assert_f64_close(weight_bucket.max_weight, 0.75);
    assert_f64_close(weight_bucket.avg_weight, 0.65);
    assert_f64_close(weight_bucket.last_weight, 0.70);
}

fn recreated_month_retention_config(state: &AppState) -> AppConfig {
    let mut config = state.config.clone();
    config.invocation_success_full_days = 400;
    config.invocation_max_days = 400;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    config
}

async fn seed_recreated_month_first_attempts(
    state: &AppState,
    manual_binding_key: &str,
    first_attempt_at: DateTime<Utc>,
) {
    for (invoke_id, offset, status) in [
        (
            "forward-proxy-timeseries-recreated-month-success",
            0,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-timeseries-recreated-month-failure",
            5,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        let occurred_at = first_attempt_at + ChronoDuration::minutes(offset);
        seed_long_term_invocation_source_for_pool_attempt(&state.pool, invoke_id, occurred_at)
            .await;
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            occurred_at,
            Some(manual_binding_key),
            status,
        )
        .await;
    }
}

async fn archive_first_recreated_month_attempts(state: &AppState, config: &AppConfig) {
    let summary = run_data_retention_maintenance(&state.pool, config, Some(false), None)
        .await
        .expect("run first retention pass");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 2);
    refresh_long_term_stats(&state.pool, 400)
        .await
        .expect("materialize long-term stats before first archive cleanup");
    let deleted = cleanup_expired_archive_batches(&state.pool, config, false)
        .await
        .expect("cleanup first archive after long-term stats materialization");
    assert_eq!(deleted, 1);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining archive batches after first pass");
    assert_eq!(remaining, 0);
}

async fn append_recreated_month_attempt(
    state: &AppState,
    manual_binding_key: &str,
    second_attempt_at: DateTime<Utc>,
    config: &AppConfig,
) {
    seed_long_term_invocation_source_for_pool_attempt(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-late-success",
        second_attempt_at,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-late-success",
        second_attempt_at,
        Some(manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    let summary = run_data_retention_maintenance(&state.pool, config, Some(false), None)
        .await
        .expect("run second retention pass");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 1);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining archive batches after second pass");
    assert_eq!(remaining, 1);
    assert_eq!(
        pending_pool_upstream_node_health_archive_batches(&state.pool)
            .await
            .expect("count pending cached node health archives after append"),
        0
    );
    assert_eq!(
        pending_pool_upstream_node_health_hourly_archive_batches(&state.pool)
            .await
            .expect("count pending hourly node health archives after append"),
        0
    );
}

async fn prepare_pending_node_health_archives(state: &AppState, manual_binding_key: &str) -> u64 {
    let older_attempt_at = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(80)).timestamp(), 3600, 0)
                + 5 * 60,
            0,
        )
        .single()
        .expect("older archived attempt timestamp should be valid");
    let newer_attempt_at = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(35)).timestamp(), 3600, 0)
                + 10 * 60,
            0,
        )
        .single()
        .expect("newer archived attempt timestamp should be valid");
    for (invoke_id, occurred_at, status) in [
        (
            "forward-proxy-timeseries-upgrade-success",
            older_attempt_at,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-timeseries-upgrade-failure",
            newer_attempt_at,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            occurred_at,
            Some(manual_binding_key),
            status,
        )
        .await;
    }
    let mut retention_config = state.config.clone();
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 120;
    let summary = run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 2);
    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&state.pool)
        .await
        .expect("clear cached node health archive rows");
    reset_pending_node_health_replay(state).await;
    let pending_before = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending node health archive batches before catch-up");
    assert_eq!(pending_before, 2);
    pending_before
}

async fn assert_pending_node_health_read(
    state: Arc<AppState>,
    manual_runtime_key: &str,
    manual_binding_key: &str,
    pending_before: u64,
) {
    let _hourly_rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let response = fetch_forward_proxy_timeseries_90d(state.clone()).await;
    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    let success_total: i64 = manual
        .buckets
        .iter()
        .map(|bucket| bucket.success_count)
        .sum();
    let failure_total: i64 = manual
        .buckets
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum();
    assert_eq!(success_total, 1);
    assert_eq!(failure_total, 1);
    let pending_after = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending node health archive batches after direct archive read");
    assert_eq!(pending_after, pending_before);
    let cached_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_node_health_archive WHERE proxy_binding_key_snapshot = ?1",
    )
    .bind(manual_binding_key)
    .fetch_one(&state.pool)
    .await
    .expect("count cached node health rows for manual binding key");
    assert_eq!(cached_rows, 0);
}

fn assert_recreated_month_response(
    response: &ForwardProxyTimeseriesResponse,
    manual_runtime_key: &str,
    first_bucket_epoch: i64,
    second_bucket_epoch: i64,
) {
    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    let first_bucket_start = format_utc_iso(
        Utc.timestamp_opt(first_bucket_epoch, 0)
            .single()
            .expect("first bucket start should be valid"),
    );
    let first_bucket = manual
        .buckets
        .iter()
        .find(|bucket| bucket.bucket_start == first_bucket_start)
        .expect("first preserved request bucket should be present");
    assert_eq!(first_bucket.success_count, 1);
    assert_eq!(first_bucket.failure_count, 1);
    let second_bucket_start = format_utc_iso(
        Utc.timestamp_opt(second_bucket_epoch, 0)
            .single()
            .expect("second bucket start should be valid"),
    );
    let second_bucket = manual
        .buckets
        .iter()
        .find(|bucket| bucket.bucket_start == second_bucket_start)
        .expect("late re-archived request bucket should be present");
    assert_eq!(second_bucket.success_count, 1);
    assert_eq!(second_bucket.failure_count, 0);
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_keeps_hourly_attempt_history_after_retention() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1081").await;

    let historical_bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::days(45)).timestamp(), 3600, 0);
    seed_historical_forward_proxy_attempts(
        &state,
        &manual_runtime_key,
        &manual_binding_key,
        historical_bucket_start,
    )
    .await;
    assert_historical_forward_proxy_retention(&state, &manual_binding_key, historical_bucket_start)
        .await;

    let response = fetch_forward_proxy_timeseries_90d(state).await;

    assert_historical_forward_proxy_response(
        &response,
        &manual_runtime_key,
        historical_bucket_start,
    );
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_preserves_materialized_history_when_same_month_archive_reappears()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1086").await;

    let archive_month_prefix = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(45))
    .format("%Y-%m")
    .to_string();
    let first_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-12 09:30:00"))
        .expect("first attempt timestamp should parse");
    let second_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-16 10:30:00"))
        .expect("second attempt timestamp should parse");
    let first_bucket_start = align_bucket_epoch(first_attempt_at.timestamp(), 3600, 0);
    let second_bucket_start = align_bucket_epoch(second_attempt_at.timestamp(), 3600, 0);

    let retention_config = recreated_month_retention_config(&state);
    seed_recreated_month_first_attempts(&state, &manual_binding_key, first_attempt_at).await;
    archive_first_recreated_month_attempts(&state, &retention_config).await;
    append_recreated_month_attempt(
        &state,
        &manual_binding_key,
        second_attempt_at,
        &retention_config,
    )
    .await;

    let response = fetch_forward_proxy_timeseries_90d(state).await;

    assert_recreated_month_response(
        &response,
        &manual_runtime_key,
        first_bucket_start,
        second_bucket_start,
    );
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_reads_pending_archived_node_health_without_materializing_cache()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1093").await;

    let pending_before = prepare_pending_node_health_archives(&state, &manual_binding_key).await;
    assert_pending_node_health_read(
        state,
        &manual_runtime_key,
        &manual_binding_key,
        pending_before,
    )
    .await;
}

use super::*;

pub(crate) async fn setup_forward_proxy_nodes(
    state: &Arc<AppState>,
    proxy_url: &str,
) -> (String, String) {
    let settings_response = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url.to_string()],
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
    (manual_runtime_key, manual_binding_key)
}

pub(crate) async fn fetch_forward_proxy_timeseries_90d(
    state: Arc<AppState>,
) -> ForwardProxyTimeseriesResponse {
    let Json(response) = fetch_forward_proxy_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch forward proxy timeseries should succeed");
    response
}

pub(crate) async fn reset_pending_node_health_replay(state: &AppState) {
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(&state.pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .execute(&state.pool)
    .await
    .expect("clear node health archive replay progress");
}

async fn seed_fixed_live_stats(
    state: &AppState,
    manual_runtime_key: &str,
    manual_binding_key: &str,
    range_start_epoch: i64,
) {
    for (offset, sample_count, min_weight, max_weight, avg_weight, last_weight) in [
        (-3600, 4, 0.25, 0.42, 0.34, 0.35),
        (5 * 3600, 2, 0.45, 0.82, 0.61, 0.80),
        (10 * 3600, 1, 1.20, 1.20, 1.20, 1.20),
    ] {
        seed_forward_proxy_weight_bucket_at(
            &state.pool,
            ForwardProxyWeightBucket {
                proxy_key: manual_runtime_key,
                bucket_start_epoch: range_start_epoch + offset,
                sample_count,
                min_weight,
                max_weight,
                avg_weight,
                last_weight,
            },
        )
        .await;
    }
    for (invoke_id, offset, status) in [
        (
            "forward-proxy-live-success",
            5 * 3600 + 300,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-live-failure",
            10 * 3600 + 300,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
        (
            "forward-proxy-live-out-of-range-success",
            -3600 + 300,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
    ] {
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            Utc.timestamp_opt(range_start_epoch + offset, 0)
                .single()
                .expect("live stats attempt timestamp should be valid"),
            Some(manual_binding_key),
            status,
        )
        .await;
    }
}

fn assert_fixed_live_stats_basics(
    response: &ForwardProxyLiveStatsResponse,
    manual_runtime_key: &str,
) {
    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.nodes.len(), 2);
    for node in &response.nodes {
        assert_eq!(node.last24h.len(), 24);
        assert_eq!(node.weight24h.len(), 24);
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
    assert_eq!(manual_success_total, 1);
    assert!(manual_failure_total >= 1);
    assert!(manual_zero_buckets >= 21);
    assert!(
        manual
            .weight24h
            .iter()
            .any(|bucket| bucket.sample_count == 0 && (bucket.last_weight - 0.35).abs() < 1e-6)
    );
}
