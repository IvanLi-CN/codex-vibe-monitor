use super::*;

#[tokio::test]
async fn forward_proxy_timeseries_keeps_hourly_attempt_history_after_retention() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1081".to_string()],
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

    let historical_bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::days(45)).timestamp(), 3600, 0);
    let historical_attempt_at = Utc
        .timestamp_opt(historical_bucket_start + 5 * 60, 0)
        .single()
        .expect("historical attempt timestamp should be valid");
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        historical_bucket_start,
        2,
        0.55,
        0.75,
        0.65,
        0.70,
    )
    .await;
    seed_long_term_invocation_source_for_pool_attempt(
        &state.pool,
        "forward-proxy-timeseries-historical-success",
        historical_attempt_at + ChronoDuration::minutes(10),
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-historical-success",
        historical_attempt_at + ChronoDuration::minutes(10),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_long_term_invocation_source_for_pool_attempt(
        &state.pool,
        "forward-proxy-timeseries-historical-failure",
        historical_attempt_at + ChronoDuration::minutes(20),
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-historical-failure",
        historical_attempt_at + ChronoDuration::minutes(20),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let mut retention_config = state.config.clone();
    retention_config.invocation_success_full_days = 400;
    retention_config.invocation_max_days = 400;
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let summary = run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 2);
    let hourly_rollup_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count materialized pool upstream hourly archive rows");
    assert!(
        hourly_rollup_rows >= 1,
        "retention should materialize owner-facing hourly history alongside raw archive batches"
    );
    refresh_long_term_stats(&state.pool, 400)
        .await
        .expect("materialize long-term stats before archive cleanup");
    let long_term_deleted = cleanup_expired_archive_batches(&state.pool, &retention_config, false)
        .await
        .expect("cleanup archive after long-term stats materialization");
    assert_eq!(long_term_deleted, 1);
    assert_eq!(
        summary.archive_batches_deleted, 0,
        "initial retention must wait for long-term stats materialization before deleting archives"
    );
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
        WHERE proxy_binding_key_snapshot = ?1
          AND occurred_at < ?2
        "#,
    )
    .bind(&manual_binding_key)
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

#[tokio::test]
async fn forward_proxy_timeseries_preserves_materialized_history_when_same_month_archive_reappears()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1086".to_string()],
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

    seed_long_term_invocation_source_for_pool_attempt(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-success",
        first_attempt_at,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-success",
        first_attempt_at,
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_long_term_invocation_source_for_pool_attempt(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-failure",
        first_attempt_at + ChronoDuration::minutes(5),
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-recreated-month-failure",
        first_attempt_at + ChronoDuration::minutes(5),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let mut retention_config = state.config.clone();
    retention_config.invocation_success_full_days = 400;
    retention_config.invocation_max_days = 400;
    retention_config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let first_summary =
        run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
            .await
            .expect("run first retention pass");
    assert_eq!(first_summary.pool_upstream_request_attempt_rows_archived, 2);

    refresh_long_term_stats(&state.pool, 400)
        .await
        .expect("materialize long-term stats before first archive cleanup");
    let first_long_term_deleted =
        cleanup_expired_archive_batches(&state.pool, &retention_config, false)
            .await
            .expect("cleanup first archive after long-term stats materialization");
    assert_eq!(first_long_term_deleted, 1);

    let remaining_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining archived pool upstream request attempt batches after first pass");
    assert_eq!(remaining_after_first, 0);

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
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let second_summary =
        run_data_retention_maintenance(&state.pool, &retention_config, Some(false), None)
            .await
            .expect("run second retention pass");
    assert_eq!(
        second_summary.pool_upstream_request_attempt_rows_archived,
        1
    );

    let remaining_after_second: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining archived pool upstream request attempt batches after second pass");
    assert_eq!(
        remaining_after_second, 1,
        "same-month retention appends should keep the reused monthly archive batch available"
    );

    let pending_cache_batches = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending cached pool node health archives after append");
    let pending_hourly_batches =
        pending_pool_upstream_node_health_hourly_archive_batches(&state.pool)
            .await
            .expect("count pending hourly pool node health archives after append");
    assert_eq!(pending_cache_batches, 0);
    assert_eq!(pending_hourly_batches, 0);

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

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    let first_bucket_start = format_utc_iso(
        Utc.timestamp_opt(first_bucket_start, 0)
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
        Utc.timestamp_opt(second_bucket_start, 0)
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
async fn forward_proxy_timeseries_reads_pending_archived_node_health_without_materializing_cache() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
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
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-upgrade-success",
        older_attempt_at,
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-upgrade-failure",
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

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&state.pool)
        .await
        .expect("clear cached node health archive rows");
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

    let pending_before = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending node health archive batches before catch-up");
    assert_eq!(pending_before, 2);

    let _hourly_rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let Json(response) = fetch_forward_proxy_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch forward proxy timeseries should read pending archive data without materializing cache");

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
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(&manual_binding_key)
    .fetch_one(&state.pool)
    .await
    .expect("count cached node health rows for manual binding key");
    assert_eq!(cached_rows, 0);
}

#[tokio::test]
async fn forward_proxy_live_stats_best_effort_skip_unreadable_pending_node_health_archives() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1094".to_string()],
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
async fn forward_proxy_timeseries_ignores_cached_rows_for_pending_node_health_archives() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1095".to_string()],
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

    let cached_rows_before: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(&manual_binding_key)
    .fetch_one(&state.pool)
    .await
    .expect("count cached node health rows before replay reset");
    assert_eq!(cached_rows_before, 2);

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

    let pending_before = pending_pool_upstream_node_health_archive_batches(&state.pool)
        .await
        .expect("count pending node health archive batches before fetch");
    assert!(pending_before > 0);

    let Json(response) = fetch_forward_proxy_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch forward proxy timeseries should ignore cached rows for pending archives");

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
    let cached_rows_after: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(&manual_binding_key)
    .fetch_one(&state.pool)
    .await
    .expect("count cached node health rows after fetch");
    assert_eq!(cached_rows_after, cached_rows_before);
}

#[tokio::test]
async fn forward_proxy_timeseries_includes_intersecting_edge_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1082".to_string()],
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

    let bucket0 = align_bucket_epoch((Utc::now() - ChronoDuration::hours(6)).timestamp(), 3600, 0);
    let bucket1 = bucket0 + 3_600;
    let bucket2 = bucket1 + 3_600;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-edge-leading",
        Utc.timestamp_opt(bucket0 + 30 * 60, 0)
            .single()
            .expect("bucket0 partial attempt timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-edge-middle",
        Utc.timestamp_opt(bucket1 + 30 * 60, 0)
            .single()
            .expect("bucket1 full attempt timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_pool_upstream_attempt_at(
        &state.pool,
        "forward-proxy-timeseries-edge-trailing",
        Utc.timestamp_opt(bucket2 + 30 * 60, 0)
            .single()
            .expect("bucket2 partial attempt timestamp should be valid"),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        bucket0,
        1,
        0.80,
        0.80,
        0.80,
        0.80,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        bucket1,
        1,
        0.70,
        0.70,
        0.70,
        0.70,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_runtime_key,
        bucket2,
        1,
        0.60,
        0.60,
        0.60,
        0.60,
    )
    .await;

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

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_runtime_key)
        .expect("manual node should remain queryable");
    assert_eq!(response.range_start, format_utc_iso(range_start));
    assert_eq!(response.range_end, format_utc_iso(range_end));
    assert_eq!(manual.buckets.len(), 3);
    assert_eq!(manual.weight_buckets.len(), 3);
    assert_eq!(
        manual
            .buckets
            .iter()
            .map(|bucket| bucket.bucket_start.as_str())
            .collect::<Vec<_>>(),
        vec![
            format_utc_iso(
                Utc.timestamp_opt(bucket0, 0)
                    .single()
                    .expect("leading bucket start should be valid")
            ),
            format_utc_iso(
                Utc.timestamp_opt(bucket1, 0)
                    .single()
                    .expect("middle bucket start should be valid")
            ),
            format_utc_iso(
                Utc.timestamp_opt(bucket2, 0)
                    .single()
                    .expect("trailing bucket start should be valid")
            ),
        ]
    );
    assert_eq!(manual.buckets[0].success_count, 1);
    assert_eq!(manual.buckets[0].failure_count, 0);
    assert_eq!(manual.buckets[1].success_count, 1);
    assert_eq!(manual.buckets[1].failure_count, 0);
    assert_eq!(manual.buckets[2].success_count, 0);
    assert_eq!(manual.buckets[2].failure_count, 1);
    assert_eq!(manual.weight_buckets[0].sample_count, 1);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.80);
    assert_eq!(manual.weight_buckets[1].sample_count, 1);
    assert_f64_close(manual.weight_buckets[1].last_weight, 0.70);
    assert_eq!(manual.weight_buckets[2].sample_count, 1);
    assert_f64_close(manual.weight_buckets[2].last_weight, 0.60);
}

#[tokio::test]
async fn forward_proxy_timeseries_keeps_single_partial_hour_ranges_non_empty() {
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
        &manual_runtime_key,
        bucket_start_epoch,
        1,
        0.58,
        0.58,
        0.58,
        0.58,
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
async fn forward_proxy_live_stats_use_real_pool_attempts_and_ignore_forward_proxy_health_checks() {
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
async fn forward_proxy_owner_facing_surfaces_include_direct_real_attempts() {
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
async fn forward_proxy_timeseries_keeps_archived_direct_history_after_direct_is_disabled() {
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
async fn forward_proxy_binding_nodes_use_real_pool_attempts_and_ignore_forward_proxy_health_checks()
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
async fn forward_proxy_timeseries_seeds_leading_weight_buckets_from_first_historical_sample() {
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
        &manual_key,
        bucket1,
        1,
        0.35,
        0.35,
        0.35,
        0.35,
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

#[tokio::test]
async fn forward_proxy_timeseries_preserves_retired_proxy_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1085".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .cloned()
        .expect("manual node should exist");

    let bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(5)).timestamp(), 3600, 0);
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual.key,
        Utc.timestamp_opt(bucket_start + 15 * 60, 0)
            .single()
            .expect("historical attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual.key,
        bucket_start,
        1,
        0.72,
        0.72,
        0.72,
        0.72,
    )
    .await;

    let _ = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec![],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket_start, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket_start + 30 * 60, 0)
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

    let archived = response
        .nodes
        .iter()
        .find(|node| node.key == manual.key)
        .expect("retired proxy should remain queryable via historical metadata");
    assert_eq!(archived.display_name, manual.display_name);
    assert_eq!(archived.source, manual.source);
    assert_eq!(archived.endpoint_url, manual.endpoint_url);
}
