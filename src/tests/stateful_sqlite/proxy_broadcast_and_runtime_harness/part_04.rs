async fn setup_unbootstrapped_forward_proxy_nodes(
    state: &Arc<AppState>,
    proxy_url: &str,
) -> (String, String) {
    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        state,
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
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

async fn seed_intersecting_edge_hours(
    state: &AppState,
    manual_runtime_key: &str,
    manual_binding_key: &str,
    bucket0: i64,
) {
    for (invoke_id, bucket_offset, status) in [
        (
            "forward-proxy-timeseries-edge-leading",
            0,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-timeseries-edge-middle",
            3_600,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "forward-proxy-timeseries-edge-trailing",
            7_200,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
    ] {
        seed_pool_upstream_attempt_at(
            &state.pool,
            invoke_id,
            Utc.timestamp_opt(bucket0 + bucket_offset + 30 * 60, 0)
                .single()
                .expect("edge-hour attempt timestamp should be valid"),
            Some(manual_binding_key),
            status,
        )
        .await;
    }
    for (bucket_start_epoch, weight) in [
        (bucket0, 0.80),
        (bucket0 + 3_600, 0.70),
        (bucket0 + 7_200, 0.60),
    ] {
        seed_forward_proxy_weight_bucket_at(
            &state.pool,
            ForwardProxyWeightBucket {
                proxy_key: manual_runtime_key,
                bucket_start_epoch,
                sample_count: 1,
                min_weight: weight,
                max_weight: weight,
                avg_weight: weight,
                last_weight: weight,
            },
        )
        .await;
    }
}

fn assert_intersecting_edge_hours(
    response: &ForwardProxyTimeseriesResponse,
    manual_runtime_key: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket0: i64,
) {
    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    assert_eq!(response.range_start, format_utc_iso(range_start));
    assert_eq!(response.range_end, format_utc_iso(range_end));
    assert_eq!(manual.buckets.len(), 3);
    assert_eq!(manual.weight_buckets.len(), 3);
    for (index, expected_weight) in [(0, 0.80), (1, 0.70), (2, 0.60)] {
        assert_eq!(manual.weight_buckets[index].sample_count, 1);
        assert_f64_close(manual.weight_buckets[index].last_weight, expected_weight);
        let expected_start = format_utc_iso(
            Utc.timestamp_opt(bucket0 + index as i64 * 3_600, 0)
                .single()
                .expect("edge-hour bucket start should be valid"),
        );
        assert_eq!(manual.buckets[index].bucket_start, expected_start);
    }
    assert_eq!(manual.buckets[0].success_count, 1);
    assert_eq!(manual.buckets[0].failure_count, 0);
    assert_eq!(manual.buckets[1].success_count, 1);
    assert_eq!(manual.buckets[1].failure_count, 0);
    assert_eq!(manual.buckets[2].success_count, 0);
    assert_eq!(manual.buckets[2].failure_count, 1);
}

async fn count_cached_node_health_rows(pool: &SqlitePool, binding_key: &str) -> i64 {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(binding_key)
    .fetch_one(pool)
    .await
    .expect("count cached node health rows")
}

#[tokio::test]
pub(crate) async fn forward_proxy_live_stats_best_effort_skip_unreadable_pending_node_health_archives()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1094").await;

    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-unreadable-archived",
        Utc::now() - ChronoDuration::days(70),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    let mut retention_config = state.config.clone();
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 120;
    let summary = run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 1);

    let archive_path = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load archived pool attempt path");
    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&state.pool)
        .await
        .expect("clear cached node health archive rows");
    reset_pending_node_health_replay(&state).await;
    std::fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt archived pool attempt batch");

    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-unreadable-live-success",
        Utc::now() - ChronoDuration::minutes(10),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let response = build_forward_proxy_live_stats_response(state.as_ref())
        .await
        .expect("live stats should stay available when pending node health archive is unreadable");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    assert_eq!(manual.stats.one_hour.attempts, 1);
    assert_eq!(manual.stats.one_hour.success_rate, Some(1.0));
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_ignores_cached_rows_for_pending_node_health_archives()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_forward_proxy_nodes(&state, "socks5://127.0.0.1:1095").await;

    let older_attempt_at = Utc
        .timestamp_opt(
            Utc::now().timestamp() - ChronoDuration::days(68).num_seconds(),
            0,
        )
        .single()
        .expect("older archived attempt timestamp should be valid");
    let newer_attempt_at = older_attempt_at
        .checked_add_signed(ChronoDuration::minutes(10))
        .expect("newer archived attempt timestamp should be valid");
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-pending-cache-success",
        older_attempt_at,
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-pending-cache-failure",
        newer_attempt_at,
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let mut retention_config = state.config.clone();
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 120;
    let summary = run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 2);

    let cached_rows_before = count_cached_node_health_rows(&state.pool, &manual_binding_key).await;
    assert_eq!(cached_rows_before, 2);

    reset_pending_node_health_replay(&state).await;

    let pending_before = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending node health archive batches before fetch");
    assert!(pending_before > 0);

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
        .expect("count pending node health archive batches after fetch");
    assert_eq!(pending_after, pending_before);
    let cached_rows_after = count_cached_node_health_rows(&state.pool, &manual_binding_key).await;
    assert_eq!(cached_rows_after, cached_rows_before);
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_includes_intersecting_edge_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let (manual_runtime_key, manual_binding_key) =
        setup_unbootstrapped_forward_proxy_nodes(&state, "socks5://127.0.0.1:1082").await;

    let bucket0 = align_bucket_epoch((Utc::now() - ChronoDuration::hours(6)).timestamp(), 3600, 0);
    let bucket1 = bucket0 + 3_600;
    let bucket2 = bucket1 + 3_600;
    seed_intersecting_edge_hours(&state, &manual_runtime_key, &manual_binding_key, bucket0).await;

    let range_start = Utc
        .timestamp_opt(bucket0 + 15 * 60, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket2 + 45 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    assert_intersecting_edge_hours(
        &response,
        &manual_runtime_key,
        range_start,
        range_end,
        bucket0,
    );
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_keeps_single_partial_hour_ranges_non_empty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1083".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
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

    let bucket_start_epoch =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(4)).timestamp(), 3600, 0);
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-single-hour",
        Utc.timestamp_opt(bucket_start_epoch + 10 * 60, 0)
            .single()
            .expect("partial bucket attempt timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        ForwardProxyWeightBucket {
            proxy_key: &manual_runtime_key,
            bucket_start_epoch,
            sample_count: 1,
            min_weight: 0.58,
            max_weight: 0.58,
            avg_weight: 0.58,
            last_weight: 0.58,
        },
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket_start_epoch + 5 * 60, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket_start_epoch + 20 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    assert_eq!(manual.buckets.len(), 1);
    assert_eq!(manual.weight_buckets.len(), 1);
    assert_eq!(manual.buckets[0].success_count, 1);
    assert_eq!(manual.buckets[0].failure_count, 0);
    assert_eq!(manual.weight_buckets[0].sample_count, 1);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.58);
}

#[tokio::test]
pub(crate) async fn forward_proxy_live_stats_use_real_pool_attempts_and_ignore_forward_proxy_health_checks()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1086".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
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

    insert_forward_proxy_attempt(
        &state.pool,
        &manual_runtime_key,
        true,
        Some(120.0),
        None,
        false,
    )
    .await
    .expect("insert successful forward proxy attempt");
    insert_forward_proxy_attempt(
        &state.pool,
        &manual_runtime_key,
        false,
        None,
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR),
        false,
    )
    .await
    .expect("insert failed forward proxy attempt");
    let recent_bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(1)).timestamp(), 3600, 0);
    let recent_attempt_at = Utc
        .timestamp_opt(recent_bucket_start + 20 * 60, 0)
        .single()
        .expect("recent attempt timestamp should be valid");
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-real-success",
        recent_attempt_at,
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-live-real-failure",
        recent_attempt_at + ChronoDuration::minutes(10),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let response = build_forward_proxy_live_stats_response(state.as_ref())
        .await
        .expect("forward proxy live stats should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    let bucket = manual
        .last24h
        .iter()
        .find(|bucket| bucket.success_count == 1 && bucket.failure_count == 1)
        .expect("inline-attempt bucket should be present without read-time catch-up");
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
pub(crate) async fn forward_proxy_owner_facing_surfaces_include_direct_real_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1092".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-direct-success",
        Utc::now() - ChronoDuration::minutes(50),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-direct-failure",
        Utc::now() - ChronoDuration::minutes(10),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let settings = build_forward_proxy_settings_response(state.as_ref())
        .await
        .expect("build forward proxy settings response");
    let direct_settings = settings
        .nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct node should be visible in settings");
    assert_eq!(direct_settings.stats.one_hour.attempts, 2);
    assert_eq!(direct_settings.stats.one_hour.success_rate, Some(0.5));

    let live = build_forward_proxy_live_stats_response(state.as_ref())
        .await
        .expect("build forward proxy live response");
    let direct_live = live
        .nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct node should be visible in live stats");
    let live_attempt_total: i64 = direct_live
        .last24h
        .iter()
        .map(|bucket| bucket.success_count + bucket.failure_count)
        .sum();
    assert_eq!(live_attempt_total, 2);

    let timeseries = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: Utc::now() - ChronoDuration::hours(3),
            end: Utc::now(),
            display_end: Utc::now(),
            duration: ChronoDuration::hours(3),
        },
    )
    .await
    .expect("build forward proxy timeseries response");
    let direct_timeseries = timeseries
        .nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct node should be visible in timeseries");
    let timeseries_attempt_total: i64 = direct_timeseries
        .buckets
        .iter()
        .map(|bucket| bucket.success_count + bucket.failure_count)
        .sum();
    assert_eq!(timeseries_attempt_total, 2);
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_keeps_archived_direct_history_after_direct_is_disabled()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1093".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("enable direct routing for historical direct traffic");

    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-direct-disabled-history-success",
        Utc::now() - ChronoDuration::minutes(90),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-direct-disabled-history-failure",
        Utc::now() - ChronoDuration::minutes(30),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1093".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        }),
    )
    .await
    .expect("disable direct routing after historical direct traffic");

    let timeseries = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: Utc::now() - ChronoDuration::hours(3),
            end: Utc::now(),
            display_end: Utc::now(),
            duration: ChronoDuration::hours(3),
        },
    )
    .await
    .expect("build forward proxy timeseries response after disabling direct");

    let direct_timeseries = timeseries
        .nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("archived direct node should remain visible in timeseries");
    assert_eq!(direct_timeseries.source, FORWARD_PROXY_SOURCE_DIRECT);
    assert_eq!(direct_timeseries.display_name, FORWARD_PROXY_DIRECT_LABEL);
    let timeseries_attempt_total: i64 = direct_timeseries
        .buckets
        .iter()
        .map(|bucket| bucket.success_count + bucket.failure_count)
        .sum();
    assert_eq!(timeseries_attempt_total, 2);
}

#[tokio::test]
pub(crate) async fn forward_proxy_binding_nodes_use_real_pool_attempts_and_ignore_forward_proxy_health_checks()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let proxy_url = "socks5://127.0.0.1:1087".to_string();
    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual_runtime_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");
    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&proxy_url, None)
            .expect("binding parts from proxy url"),
    )[0]
    .clone();

    insert_forward_proxy_attempt(
        &state.pool,
        &manual_runtime_key,
        true,
        Some(90.0),
        None,
        false,
    )
    .await
    .expect("insert successful forward proxy attempt");
    insert_forward_proxy_attempt(
        &state.pool,
        &manual_runtime_key,
        false,
        None,
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR),
        false,
    )
    .await
    .expect("insert failed forward proxy attempt");
    let recent_bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(1)).timestamp(), 3600, 0);
    let recent_attempt_at = Utc
        .timestamp_opt(recent_bucket_start + 25 * 60, 0)
        .single()
        .expect("recent binding attempt timestamp should be valid");
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-binding-real-success",
        recent_attempt_at,
        Some(&binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-binding-real-failure",
        recent_attempt_at + ChronoDuration::minutes(10),
        Some(&binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let nodes = build_forward_proxy_binding_nodes_response(
        state.as_ref(),
        std::slice::from_ref(&manual_runtime_key),
    )
    .await
    .expect("build binding nodes response");
    let manual = nodes
        .iter()
        .find(|node| node.key == binding_key)
        .expect("binding node should remain queryable");
    let bucket = manual
        .last24h
        .iter()
        .find(|bucket| bucket.success_count == 1 && bucket.failure_count == 1)
        .expect("binding-node bucket should be present without read-time catch-up");
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_seeds_leading_weight_buckets_from_first_historical_sample()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1084".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    {
        let mut manager = state.forward_proxy.lock().await;
        let runtime = manager
            .runtime
            .get_mut(&manual_key)
            .expect("manual runtime should exist");
        runtime.weight = 1.45;
    }

    let bucket0 = align_bucket_epoch((Utc::now() - ChronoDuration::hours(6)).timestamp(), 3600, 0);
    let bucket1 = bucket0 + 3_600;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        ForwardProxyWeightBucket {
            proxy_key: &manual_key,
            bucket_start_epoch: bucket1,
            sample_count: 1,
            min_weight: 0.35,
            max_weight: 0.35,
            avg_weight: 0.35,
            last_weight: 0.35,
        },
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket0, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket1 + 30 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should remain queryable");
    assert_eq!(manual.weight_buckets.len(), 2);
    assert_eq!(manual.weight_buckets[0].sample_count, 0);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.35);
    assert_eq!(manual.weight_buckets[1].sample_count, 1);
    assert_f64_close(manual.weight_buckets[1].last_weight, 0.35);
}

use super::*;
