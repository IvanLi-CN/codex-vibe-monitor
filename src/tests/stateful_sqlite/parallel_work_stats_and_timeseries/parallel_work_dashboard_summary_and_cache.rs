use super::*;

#[tokio::test]
async fn summary_topic_builder_matches_http_for_open_range() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (
        id,
        invoke_id,
        status,
        total_tokens,
        cost,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
        error_message,
        failure_kind,
        failure_class,
    ) in [
        (
            470_i64,
            "summary-topic-success",
            "success",
            120_i64,
            Some(0.50_f64),
            Some(0.06_f64),
            Some(0.14_f64),
            Some(0.04_f64),
            Some(0.21_f64),
            Some(0.05_f64),
            None,
            None,
            Some("none"),
        ),
        (
            471_i64,
            "summary-topic-failed",
            "failed",
            80_i64,
            Some(0.20_f64),
            None,
            None,
            None,
            None,
            None,
            Some("HTTP 429 too many requests"),
            Some("upstream_response_failed"),
            Some("service_failure"),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, status,
                input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                error_message, failure_kind, failure_class, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("gpt-5.6")
        .bind(status)
        .bind(100_i64)
        .bind(40_i64)
        .bind(25_i64)
        .bind(5_i64)
        .bind(total_tokens)
        .bind(cost)
        .bind(cost_input)
        .bind(cost_cache_write)
        .bind(cost_cache_read)
        .bind(cost_output)
        .bind(cost_reasoning)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert summary topic builder row");
    }

    let Json(http_summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch summary over http");
    let topic_summary = load_summary_response_from_query(
        state.as_ref(),
        &SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryBuildRoute::Topic,
    )
    .await
    .expect("build summary for topic");

    assert_eq!(
        serde_json::to_value(http_summary).expect("serialize http summary"),
        serde_json::to_value(topic_summary).expect("serialize topic summary"),
    );
}

#[tokio::test]
async fn empty_summary_response_keeps_live_in_progress_conversation_count() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (id, invoke_id, status, payload) in [
        (
            451_i64,
            "empty-summary-running-a",
            "running",
            Some(json!({ "promptCacheKey": "pck-empty-live-a" }).to_string()),
        ),
        (
            452_i64,
            "empty-summary-pending-a",
            "pending",
            Some(json!({ "promptCacheKey": "pck-empty-live-a" }).to_string()),
        ),
        (
            453_i64,
            "empty-summary-running-b",
            "running",
            Some(json!({ "promptCacheKey": "pck-empty-live-b" }).to_string()),
        ),
        (
            454_i64,
            "empty-summary-success-c",
            "success",
            Some(json!({ "promptCacheKey": "pck-empty-finished-c" }).to_string()),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert empty summary in-progress row");
    }

    let source_scope = resolve_default_source_scope(&state.pool)
        .await
        .expect("resolve source scope");
    let response = build_empty_summary_response(state.as_ref(), source_scope, None)
        .await
        .expect("build empty summary response");

    assert_eq!(response.total_count, 0);
    assert_eq!(response.success_count, 0);
    assert_eq!(response.failure_count, 0);
    assert_eq!(response.total_tokens, 0);
    assert_f64_close(response.total_cost, 0.0);
    assert_eq!(response.in_progress_conversation_count, Some(3));
    assert_eq!(response.in_progress_retry_conversation_count, Some(0));
    assert_eq!(response.in_progress_avg_wait_ms, None);
    assert_eq!(response.non_success_cost, Some(0.0));
    assert_eq!(response.non_success_tokens, None);
    assert!(response.maintenance.is_some());
}

#[tokio::test]
async fn summary_ignores_runtime_overlay_records_with_terminal_db_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let old_running_occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        prompt_cache_key: Some("pck-summary-runtime-terminal".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };

    let stale_runtime_record = build_running_proxy_capture_record(
        "summary-runtime-terminal",
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.42"),
        Some("summary-runtime-terminal"),
        Some("pck-summary-runtime-terminal"),
        true,
        Some(42),
        Some("Runtime Summary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        Some("runtime-summary-proxy"),
        Some(2),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        33.0,
        910.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, stale_runtime_record)
        .await
        .expect("store stale runtime summary snapshot");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            t_upstream_ttfb_ms,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(481_i64)
    .bind("summary-runtime-terminal")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.1_f64)
    .bind(910.0_f64)
    .bind(json!({ "promptCacheKey": "pck-summary-runtime-terminal" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert terminal DB row for stale runtime snapshot");

    let active_runtime_record = build_running_proxy_capture_record(
        "summary-runtime-active",
        &old_running_occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.43"),
        Some("summary-runtime-active"),
        Some("pck-summary-runtime-active"),
        true,
        Some(43),
        Some("Runtime Summary Active"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        Some("runtime-summary-proxy"),
        Some(1),
        Some(1),
        None,
        None,
        11.0,
        3.0,
        34.0,
        333.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, active_runtime_record)
        .await
        .expect("store active runtime summary snapshot");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch today summary with runtime terminal exclusion");

    assert_eq!(summary.in_progress_conversation_count, Some(1));
    assert_eq!(summary.in_progress_retry_conversation_count, Some(0));
    assert_f64_close(
        summary
            .in_progress_avg_wait_ms
            .expect("active runtime wait should be reported"),
        333.0,
    );
}

#[tokio::test]
async fn natural_day_summary_reports_retry_wait_and_non_success_usage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_utc = Utc::now();
    let today_start_utc = start_of_local_day(now_utc, Shanghai);
    let current_hour_utc = Utc
        .timestamp_opt(align_bucket_epoch(now_utc.timestamp(), 3_600, 0), 0)
        .single()
        .expect("valid current hour");
    let previous_complete_hour_utc =
        (current_hour_utc - ChronoDuration::hours(1)).max(today_start_utc);
    let occurred_at = format_naive(now_utc.with_timezone(&Shanghai).naive_local());
    let earlier_today_utc = previous_complete_hour_utc + ChronoDuration::minutes(30);
    let earlier_today_utc = if earlier_today_utc < now_utc {
        earlier_today_utc
    } else {
        (now_utc - ChronoDuration::seconds(1)).max(today_start_utc)
    };
    let earlier_today = format_naive(earlier_today_utc.with_timezone(&Shanghai).naive_local());

    for (
        id,
        invoke_id,
        status,
        total_tokens,
        cost,
        error_message,
        failure_kind,
        ttfb_ms,
        payload,
    ) in [
        (
            501_i64,
            "summary-retry-terminal",
            "failed",
            120_i64,
            0.12_f64,
            Some("upstream response failed"),
            Some("upstream_response_failed"),
            Some(910.0_f64),
            Some(json!({ "promptCacheKey": "pck-retry-a" }).to_string()),
        ),
        (
            502_i64,
            "summary-retry-pending",
            "pending",
            5_i64,
            0.005_f64,
            None,
            None,
            None,
            Some(json!({ "promptCacheKey": "pck-retry-a" }).to_string()),
        ),
        (
            503_i64,
            "summary-retry-running",
            "running",
            30_i64,
            0.03_f64,
            None,
            None,
            Some(2400.0_f64),
            Some(json!({ "promptCacheKey": "pck-retry-a" }).to_string()),
        ),
        (
            504_i64,
            "summary-interrupted-terminal",
            "interrupted",
            70_i64,
            0.07_f64,
            Some("downstream closed while streaming upstream response"),
            Some("downstream_closed"),
            Some(600.0_f64),
            Some(json!({ "promptCacheKey": "pck-interrupted-b" }).to_string()),
        ),
        (
            505_i64,
            "summary-interrupted-pending",
            "pending",
            20_i64,
            0.02_f64,
            None,
            None,
            Some(1800.0_f64),
            Some(json!({ "promptCacheKey": "pck-interrupted-b" }).to_string()),
        ),
        (
            506_i64,
            "summary-success-terminal",
            "success",
            90_i64,
            0.09_f64,
            None,
            None,
            Some(500.0_f64),
            Some(json!({ "promptCacheKey": "pck-success-c" }).to_string()),
        ),
        (
            507_i64,
            "summary-success-running",
            "running",
            10_i64,
            0.01_f64,
            None,
            None,
            Some(1200.0_f64),
            Some(json!({ "promptCacheKey": "pck-success-c" }).to_string()),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                t_upstream_ttfb_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(if id == 501 {
            &earlier_today
        } else {
            &occurred_at
        })
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(error_message)
        .bind(failure_kind)
        .bind(ttfb_ms)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert natural-day summary augmentation row");
    }

    let mut tx = state.pool.begin().await.expect("begin rollup rebuild tx");
    recompute_invocation_hourly_rollups_for_ids_tx(
        tx.as_mut(),
        &[501, 502, 503, 504, 505, 506, 507],
    )
    .await
    .expect("rebuild summary rollups for direct test rows");
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, 507)
        .await
        .expect("mark directly rebuilt summary rollups as live cursor covered");
    tx.commit().await.expect("commit rollup rebuild tx");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch today summary with augmentation fields");

    assert_eq!(summary.in_progress_conversation_count, Some(4));
    assert_eq!(summary.in_progress_retry_conversation_count, Some(2));
    assert_f64_close(
        summary
            .in_progress_avg_wait_ms
            .expect("in-progress wait average should exist"),
        (2400.0 + 1800.0 + 1200.0) / 3.0,
    );
    assert_f64_close(
        summary
            .non_success_cost
            .expect("non-success cost should exist"),
        0.19,
    );
    assert_eq!(summary.non_success_tokens, Some(190));
}

#[tokio::test]
async fn account_scoped_natural_day_summary_keeps_augmentation_fields_scoped() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (id, invoke_id, account_id, status, total_tokens, cost, ttfb_ms, payload) in [
        (
            551_i64,
            "account-scope-failed-terminal",
            42_i64,
            "failed",
            200_i64,
            0.22_f64,
            Some(900.0_f64),
            json!({ "promptCacheKey": "pck-account-a", "upstreamAccountId": 42 }).to_string(),
        ),
        (
            552_i64,
            "account-scope-running",
            42_i64,
            "running",
            50_i64,
            0.05_f64,
            Some(1700.0_f64),
            json!({ "promptCacheKey": "pck-account-a", "upstreamAccountId": 42 }).to_string(),
        ),
        (
            553_i64,
            "other-account-interrupted",
            17_i64,
            "interrupted",
            999_i64,
            9.99_f64,
            Some(600.0_f64),
            json!({ "promptCacheKey": "pck-other-b", "upstreamAccountId": 17 }).to_string(),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                t_upstream_ttfb_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(if status == "failed" {
            Some("account scoped failure")
        } else {
            None
        })
        .bind(if status == "failed" {
            Some("upstream_response_failed")
        } else if status == "interrupted" {
            Some("downstream_closed")
        } else {
            None
        })
        .bind(ttfb_ms)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert account scoped summary augmentation row");
        assert!(account_id > 0);
    }

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("fetch account-scoped summary with augmentation fields");

    assert_eq!(summary.in_progress_conversation_count, Some(1));
    assert_eq!(summary.in_progress_retry_conversation_count, Some(1));
    assert_eq!(summary.non_success_tokens, Some(200));
    assert_f64_close(
        summary
            .non_success_cost
            .expect("account-scoped non-success cost should exist"),
        0.22,
    );
    assert_f64_close(
        summary
            .in_progress_avg_wait_ms
            .expect("account-scoped in-progress wait should exist"),
        1700.0,
    );
}

#[tokio::test]
async fn upstream_account_activity_groups_active_accounts_and_hides_yesterday_live_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pool Alpha")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream activity account");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_allow_cut_in = 0,
            policy_priority_tier = 'no_new',
            policy_fast_mode_rewrite_mode = 'force_add',
            policy_concurrency_limit = 3,
            policy_upstream_429_retry_enabled = 1,
            policy_upstream_429_max_retries = 2
        WHERE id = ?1
        "#,
    )
    .bind(42_i64)
    .execute(&state.pool)
    .await
    .expect("set upstream activity routing policy");
    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    for (
        id,
        invoke_id,
        status,
        total_tokens,
        cost,
        cache_input_tokens,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        ttfb_ms,
        total_ms,
        payload,
        error_message,
        failure_kind,
    ) in [
        (
            601_i64,
            "upstream-activity-running",
            "running",
            100_i64,
            0.10_f64,
            20_i64,
            Some(80.0_f64),
            Some(4.0_f64),
            Some(2_100.0_f64),
            Some(410.0_f64),
            Some(1_000.0_f64),
            json!({ "promptCacheKey": "pck-upstream-a", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            None,
            None,
        ),
        (
            602_i64,
            "upstream-activity-success",
            "success",
            300_i64,
            0.30_f64,
            60_i64,
            Some(83.434948_f64),
            Some(2.762219_f64),
            Some(2_123.797426_f64),
            Some(0.006021_f64),
            Some(2_000.0_f64),
            json!({ "promptCacheKey": "pck-upstream-a", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            None,
            None,
        ),
        (
            603_i64,
            "upstream-activity-failed",
            "failed",
            200_i64,
            0.20_f64,
            40_i64,
            Some(51.0_f64),
            Some(3.0_f64),
            Some(1_500.0_f64),
            Some(450.0_f64),
            Some(3_000.0_f64),
            json!({ "promptCacheKey": "pck-upstream-a", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            Some("upstream failed"),
            Some("upstream_response_failed"),
        ),
        (
            604_i64,
            "upstream-activity-pending-retry",
            "pending",
            50_i64,
            0.05_f64,
            10_i64,
            None,
            None,
            None,
            None,
            None,
            json!({ "promptCacheKey": "pck-upstream-a", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            None,
            None,
        ),
        (
            605_i64,
            "upstream-activity-extra-running",
            "running",
            75_i64,
            0.07_f64,
            15_i64,
            Some(74.0_f64),
            Some(4.0_f64),
            Some(1_800.0_f64),
            Some(405.0_f64),
            Some(1_500.0_f64),
            json!({ "promptCacheKey": "pck-upstream-b", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            None,
            None,
        ),
        (
            606_i64,
            "upstream-activity-extra-success",
            "success",
            125_i64,
            0.12_f64,
            25_i64,
            Some(49.329286_f64),
            Some(1.973188_f64),
            Some(3_474.776073_f64),
            Some(0.002344_f64),
            Some(2_000.0_f64),
            json!({ "promptCacheKey": "pck-upstream-c", "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha" }).to_string(),
            None,
            None,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                cache_input_tokens,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                error_message,
                failure_kind,
                t_upstream_ttfb_ms,
                t_total_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(
            base_local
                .checked_sub_signed(ChronoDuration::seconds((607_i64 - id) * 10))
                .expect("valid upstream account activity time"),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(cache_input_tokens)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(error_message)
        .bind(failure_kind)
        .bind(ttfb_ms)
        .bind(total_ms)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert upstream account activity invocation");
    }

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity");

    assert_eq!(activity.accounts.len(), 1);
    let account = activity.accounts.first().expect("activity account");
    assert_eq!(account.upstream_account_id, 42);
    assert_eq!(account.display_name, "Pool Alpha");
    assert!(account.latest_conversation_created_at.is_some());
    assert_eq!(
        account.last_invocation_at.as_deref(),
        account
            .recent_invocations
            .first()
            .map(|row| row.occurred_at.as_str())
    );
    assert_eq!(account.group_name.as_deref(), Some("Primary"));
    assert_eq!(account.plan_type.as_deref(), Some("enterprise"));
    assert!(account.enabled);
    assert_eq!(account.display_status, "active");
    assert_eq!(account.enable_status, "enabled");
    assert_eq!(account.work_status, "idle");
    assert_eq!(account.health_status, "normal");
    assert_eq!(account.sync_state, "idle");
    assert_eq!(account.last_error, None);
    assert_eq!(account.last_action_reason_message, None);
    assert_eq!(account.request_count, 3);
    assert_eq!(account.success_count, 2);
    assert_eq!(account.failure_count, 1);
    assert_eq!(account.non_success_count, 1);
    assert_eq!(account.success_tokens, 425);
    assert_eq!(account.non_success_tokens, 200);
    assert_eq!(account.failure_tokens, 200);
    assert_f64_close(account.failure_cost, 0.20);
    assert_f64_close(account.total_cost, 0.62);
    assert_f64_close(
        account
            .first_byte_avg_ms
            .expect("first response byte total avg should exist"),
        (83.434948_f64
            + 2.762219_f64
            + 2_123.797426_f64
            + 0.006021_f64
            + 49.329286_f64
            + 1.973188_f64
            + 3_474.776073_f64
            + 0.002344_f64)
            / 2.0,
    );
    assert_f64_close(
        account
            .avg_total_ms
            .expect("success total latency avg should exist"),
        2_000.0,
    );
    assert_eq!(account.in_progress_invocation_count, Some(3));
    assert_eq!(account.retry_invocation_count, Some(1));
    let effective_routing_rule =
        serde_json::to_value(&account.effective_routing_rule).expect("serialize routing rule");
    assert!(
        effective_routing_rule
            .get("blockNewConversations")
            .is_none()
    );
    assert_eq!(
        effective_routing_rule["allowCutIn"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(effective_routing_rule["priorityTier"], "no_new");
    assert_eq!(effective_routing_rule["fastModeRewriteMode"], "force_add");
    assert_eq!(effective_routing_rule["concurrencyLimit"], 3);
    assert_eq!(
        effective_routing_rule["upstream429RetryEnabled"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(effective_routing_rule["upstream429MaxRetries"], 2);
    assert_eq!(account.recent_invocations.len(), 4);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "upstream-activity-extra-success"
    );
    assert_eq!(
        account.recent_invocations[0].prompt_cache_key.as_deref(),
        Some("pck-upstream-c")
    );
    assert_eq!(
        account.recent_invocations[1].invoke_id,
        "upstream-activity-extra-running"
    );
    assert_eq!(
        account.recent_invocations[2].invoke_id,
        "upstream-activity-pending-retry"
    );
    assert_eq!(
        account.recent_invocations[3].invoke_id,
        "upstream-activity-failed"
    );

    let Json(expanded_activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(6),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch expanded upstream account activity");

    let expanded_account = expanded_activity
        .accounts
        .first()
        .expect("expanded activity account");
    assert_eq!(expanded_account.recent_invocations.len(), 6);
    assert_eq!(
        expanded_account.recent_invocations[5].invoke_id,
        "upstream-activity-running"
    );

    let Json(compact_activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch compact upstream account activity");

    let compact_account = compact_activity
        .accounts
        .first()
        .expect("compact activity account");
    assert_eq!(compact_account.recent_invocations.len(), 2);

    let invalid_limit = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(17),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await;
    assert!(invalid_limit.is_err());

    let Json(yesterday_activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "yesterday".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch yesterday upstream account activity");

    assert!(
        yesterday_activity
            .accounts
            .iter()
            .all(|account| account.in_progress_invocation_count.is_none())
    );
    assert!(
        yesterday_activity
            .accounts
            .iter()
            .all(|account| account.retry_invocation_count.is_none())
    );
}

#[tokio::test]
async fn upstream_account_activity_ignores_stale_persisted_in_progress_rows_for_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pool Alpha")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account for stale persisted running rows");

    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    for id in 0..20_i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(96_000_i64 + id)
        .bind(format!("stale-upstream-running-{id}"))
        .bind(format_naive(
            base_local
                .checked_sub_signed(ChronoDuration::days(6))
                .and_then(|value| value.checked_add_signed(ChronoDuration::seconds(id)))
                .expect("valid stale running time"),
        ))
        .bind(SOURCE_PROXY)
        .bind("running")
        .bind(0_i64)
        .bind(0.0_f64)
        .bind(
            json!({
                "promptCacheKey": format!("pck-stale-upstream-running-{id}"),
                "upstreamAccountId": 42_i64,
                "upstreamAccountName": "Pool Alpha"
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert stale persisted running invocation");
    }
    for (id, invoke_id, minutes_ago, total_tokens, cost) in [
        (
            97_001_i64,
            "recent-upstream-success-old",
            2_i64,
            120_i64,
            0.12_f64,
        ),
        (
            97_002_i64,
            "recent-upstream-success-new",
            1_i64,
            240_i64,
            0.24_f64,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(
            base_local
                .checked_sub_signed(ChronoDuration::minutes(minutes_ago))
                .expect("valid recent success time"),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
        .bind(cost)
        .bind(
            json!({
                "promptCacheKey": format!("pck-{invoke_id}"),
                "upstreamAccountId": 42_i64,
                "upstreamAccountName": "Pool Alpha"
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert recent terminal invocation");
    }

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream activity with stale persisted running rows");

    assert_eq!(activity.accounts.len(), 1);
    let account = activity
        .accounts
        .first()
        .expect("stale persisted running account");
    assert_eq!(account.upstream_account_id, 42);
    assert_eq!(account.request_count, 2);
    assert_eq!(account.success_count, 2);
    assert_eq!(account.failure_count, 0);
    assert_eq!(account.recent_invocations.len(), 2);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "recent-upstream-success-new"
    );
    assert_eq!(
        account.recent_invocations[1].invoke_id,
        "recent-upstream-success-old"
    );
    assert!(
        account
            .recent_invocations
            .iter()
            .all(|row| !row.invoke_id.starts_with("stale-upstream-running-"))
    );
}

#[tokio::test]
async fn dashboard_activity_summary_rates_and_in_progress_are_account_sum() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [(42_i64, "Pool Alpha"), (77_i64, "Pool Beta")] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("Primary")
        .bind("enterprise")
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&state.pool)
        .await
        .expect("insert dashboard activity upstream account");
    }

    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    let current_minute_start = base_local
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .expect("valid current minute start");
    let latest_complete_minute_start = current_minute_start - ChronoDuration::minutes(1);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(8_000_i64)
    .bind("dashboard-activity-unassigned-failed")
    .bind(format_naive(
        latest_complete_minute_start
            .checked_add_signed(ChronoDuration::seconds(5))
            .expect("valid unassigned failure time"),
    ))
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind(500_i64)
    .bind(0.05_f64)
    .bind(json!({ "promptCacheKey": "pck-dashboard-activity-unassigned-running" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert previous unassigned failure");
    for (id, invoke_id, upstream_account_id, status, total_tokens, cost, ttfb_ms, offset_seconds) in [
        (
            8_001_i64,
            "dashboard-activity-alpha-success",
            Some(42_i64),
            "success",
            1_000_i64,
            0.10_f64,
            Some(100.0_f64),
            10_i64,
        ),
        (
            8_002_i64,
            "dashboard-activity-beta-running",
            Some(77_i64),
            "running",
            2_000_i64,
            0.20_f64,
            Some(200.0_f64),
            20_i64,
        ),
        (
            8_003_i64,
            "dashboard-activity-unassigned-running",
            None,
            "running",
            3_000_i64,
            0.30_f64,
            Some(300.0_f64),
            30_i64,
        ),
    ] {
        let mut payload = json!({ "promptCacheKey": format!("pck-{invoke_id}") });
        if let Some(account_id) = upstream_account_id {
            payload["upstreamAccountId"] = json!(account_id);
        }
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                t_upstream_ttfb_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(
            latest_complete_minute_start
                .checked_add_signed(ChronoDuration::seconds(offset_seconds))
                .expect("valid dashboard activity time"),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(ttfb_ms)
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert dashboard activity invocation");
    }
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET payload = ?1,
            output_tokens = ?2,
            t_req_read_ms = ?3,
            t_req_parse_ms = ?4,
            t_upstream_connect_ms = ?5,
            t_upstream_stream_ms = ?6,
            t_total_ms = ?7
        WHERE invoke_id = ?8
        "#,
    )
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-activity-alpha-success",
            "upstreamAccountId": 42_i64,
            "responseModel": "gpt-5.6-performance",
            "reasoningEffort": "high",
        })
        .to_string(),
    )
    .bind(600_i64)
    .bind(10.0_f64)
    .bind(20.0_f64)
    .bind(30.0_f64)
    .bind(2_000.0_f64)
    .bind(2_300.0_f64)
    .bind("dashboard-activity-alpha-success")
    .execute(&state.pool)
    .await
    .expect("add alpha performance samples");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
            t_upstream_ttfb_ms, t_upstream_stream_ms, t_total_ms, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind(8_007_i64)
    .bind("dashboard-activity-alpha-zero-cost")
    .bind(format_naive(
        latest_complete_minute_start
            .checked_add_signed(ChronoDuration::seconds(40))
            .expect("valid zero-cost dashboard activity time"),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(400_i64)
    .bind(120_i64)
    .bind(0.0_f64)
    .bind(40.0_f64)
    .bind(400.0_f64)
    .bind(500.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-activity-alpha-zero-cost",
            "upstreamAccountId": 42_i64,
            "responseModel": "gpt-5.6-zero",
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert zero-cost billed dashboard activity invocation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(8_004_i64)
    .bind("dashboard-activity-yesterday-success")
    .bind(format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::days(1))
            .expect("valid yesterday dashboard activity time"),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(4_000_i64)
    .bind(0.40_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-activity-yesterday-success",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert yesterday dashboard activity invocation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(8_005_i64)
    .bind("dashboard-activity-runtime-today-running")
    .bind(format_naive(base_local))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(json!({ "promptCacheKey": "pck-dashboard-activity-runtime-today-running" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert stale runtime db shell for dashboard activity");
    let runtime_today = crate::api::ApiInvocation {
        id: 8_005_i64,
        invoke_id: "dashboard-activity-runtime-today-running".to_string(),
        occurred_at: format_naive(base_local),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: None,
        model: Some("gpt-5".to_string()),
        request_model: Some("gpt-5".to_string()),
        response_model: Some("gpt-5".to_string()),
        input_tokens: Some(10_i64),
        output_tokens: Some(20_i64),
        cache_input_tokens: Some(0_i64),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(0_i64),
        reasoning_effort: None,
        total_tokens: Some(9_999_i64),
        cost: Some(9.99_f64),
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        cache_write_tokens: Some(10_i64),
        status: Some("running".to_string()),
        live_phase: None,
        error_message: None,
        downstream_status_code: None,
        failure_kind: None,
        blocked_binding: None,
        blocked_binding_json: None,
        stream_terminal_event: None,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        failure_class: None,
        is_actionable: None,
        endpoint: Some("/v1/chat/completions".to_string()),
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
        requester_ip: None,
        prompt_cache_key: Some("pck-dashboard-activity-runtime-today-running".to_string()),
        sticky_key: None,
        route_mode: None,
        upstream_account_id: None,
        upstream_account_name: None,
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        proxy_weight_delta: None,
        cost_estimated: None,
        price_version: None,
        cost_audit: None,
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: None,
        response_raw_truncated_reason: None,
        detail_level: "full".to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: Some(999.0_f64),
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        created_at: format_naive(base_local),
    };
    state
        .proxy_runtime_invocations
        .upsert(runtime_today.clone());
    let mut runtime_before_today = runtime_today.clone();
    runtime_before_today.id = 8_006_i64;
    runtime_before_today.invoke_id = "dashboard-activity-runtime-before-today-running".to_string();
    runtime_before_today.occurred_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::days(2))
            .expect("valid pre-range runtime time"),
    );
    runtime_before_today.total_tokens = Some(12_345_i64);
    runtime_before_today.cost = Some(12.34_f64);
    runtime_before_today.prompt_cache_key =
        Some("pck-dashboard-activity-runtime-before-today-running".to_string());
    state.proxy_runtime_invocations.upsert(runtime_before_today);
    let mut runtime_terminal = runtime_today.clone();
    runtime_terminal.id = 0;
    runtime_terminal.invoke_id = "dashboard-activity-runtime-terminal".to_string();
    runtime_terminal.occurred_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::seconds(1))
            .expect("valid runtime terminal time"),
    );
    runtime_terminal.status = Some("success".to_string());
    runtime_terminal.live_phase = None;
    runtime_terminal.upstream_account_id = Some(42);
    runtime_terminal.upstream_account_name = Some("Pool Alpha".to_string());
    runtime_terminal.prompt_cache_key = Some("pck-dashboard-activity-runtime-terminal".to_string());
    runtime_terminal.total_tokens = Some(0);
    runtime_terminal.cost = Some(0.0);
    state
        .proxy_runtime_invocations
        .upsert_terminal(runtime_terminal);
    let mut terminal_only_account = runtime_today.clone();
    terminal_only_account.id = 0;
    terminal_only_account.invoke_id =
        "dashboard-activity-runtime-terminal-only-account".to_string();
    terminal_only_account.occurred_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::seconds(2))
            .expect("valid terminal-only account time"),
    );
    terminal_only_account.status = Some("success".to_string());
    terminal_only_account.live_phase = None;
    terminal_only_account.upstream_account_id = Some(88);
    terminal_only_account.upstream_account_name = Some("Runtime Only".to_string());
    terminal_only_account.prompt_cache_key =
        Some("pck-dashboard-activity-runtime-terminal-only-account".to_string());
    terminal_only_account.total_tokens = Some(0);
    terminal_only_account.cost = Some(0.0);
    state
        .proxy_runtime_invocations
        .upsert_terminal(terminal_only_account);

    let activity =
        load_dashboard_activity_snapshot(state.as_ref(), "today", Shanghai, 4, true, true, None)
            .await
            .expect("load dashboard activity snapshot");

    let accounts = activity.accounts();
    assert_eq!(accounts.len(), 4);
    assert!(accounts.iter().any(|account| account.is_unassigned));
    let unassigned = accounts
        .iter()
        .find(|account| account.is_unassigned)
        .expect("unassigned account bucket");
    assert_eq!(unassigned.retry_invocation_count, Some(1));
    let runtime_recent = unassigned
        .recent_invocations
        .iter()
        .find(|row| row.invoke_id == "dashboard-activity-runtime-today-running")
        .expect("runtime overlay should hydrate the dashboard recent invocation");
    assert_eq!(runtime_recent.total_tokens, 9_999);
    assert_eq!(runtime_recent.input_tokens, Some(10));
    assert_eq!(runtime_recent.output_tokens, Some(20));
    assert_eq!(runtime_recent.t_upstream_ttfb_ms, Some(999.0));
    assert_eq!(
        unassigned
            .recent_invocations
            .iter()
            .filter(|row| row.invoke_id == "dashboard-activity-runtime-today-running")
            .count(),
        1
    );
    let alpha = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("alpha account bucket");
    assert_eq!(
        alpha.recent_invocations[0].invoke_id,
        "dashboard-activity-runtime-terminal"
    );
    assert_eq!(alpha.recent_invocations[0].status, "success");
    let runtime_only = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(88))
        .expect("terminal-only runtime account bucket");
    assert_eq!(runtime_only.display_name, "Runtime Only");
    assert_eq!(runtime_only.request_count, 0);
    assert_eq!(
        runtime_only.recent_invocations[0].invoke_id,
        "dashboard-activity-runtime-terminal-only-account"
    );
    assert!(
        accounts
            .iter()
            .all(|account| account.recent_invocations.len() <= 4)
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("dashboard-activity-runtime-terminal")
    .bind(format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::seconds(1))
            .expect("valid persisted runtime terminal time"),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-activity-runtime-terminal",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("persist runtime terminal after initial dashboard reconcile");
    let persisted_activity =
        load_dashboard_activity_snapshot(state.as_ref(), "today", Shanghai, 4, true, true, None)
            .await
            .expect("reload dashboard activity after terminal persistence");
    let persisted_alpha = persisted_activity
        .accounts()
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("persisted alpha account bucket");
    assert_eq!(
        persisted_alpha
            .recent_invocations
            .iter()
            .filter(|row| row.invoke_id == "dashboard-activity-runtime-terminal")
            .count(),
        1
    );
    assert_eq!(
        activity.summary().stats.total_count,
        accounts
            .iter()
            .map(|account| account.request_count)
            .sum::<i64>(),
    );
    assert_eq!(
        activity.summary().stats.total_tokens,
        accounts
            .iter()
            .map(|account| account.total_tokens)
            .sum::<i64>(),
    );
    assert_eq!(activity.summary().stats.total_tokens, 1_900);
    assert_f64_close(
        activity.summary().stats.total_cost,
        accounts
            .iter()
            .map(|account| account.total_cost)
            .sum::<f64>(),
    );
    assert_eq!(
        activity.summary().stats.in_progress_conversation_count,
        Some(
            accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum::<i64>(),
        ),
    );
    assert_eq!(
        activity.summary().stats.in_progress_conversation_count,
        Some(4)
    );
    assert_eq!(
        activity
            .summary()
            .stats
            .in_progress_retry_conversation_count,
        Some(
            accounts
                .iter()
                .map(|account| account.retry_invocation_count.unwrap_or(0))
                .sum::<i64>(),
        ),
    );
    assert_f64_close(
        activity
            .summary()
            .tokens_per_minute
            .expect("summary token rate"),
        accounts
            .iter()
            .map(|account| account.tokens_per_minute.unwrap_or(0.0))
            .sum::<f64>(),
    );
    assert_f64_close(
        activity.summary().spend_rate.expect("summary spend rate"),
        accounts
            .iter()
            .map(|account| account.spend_rate.unwrap_or(0.0))
            .sum::<f64>(),
    );
    assert_f64_close(
        activity
            .summary()
            .tokens_per_minute
            .expect("summary rolling tpm"),
        0.0,
    );
    assert_f64_close(
        activity
            .summary()
            .spend_rate
            .expect("summary rolling spend rate"),
        0.0,
    );
    assert_f64_close(
        activity
            .summary()
            .current_first_response_byte_total_avg_ms
            .expect("summary current first-byte value"),
        40.0,
    );
    assert_f64_close(
        activity
            .summary()
            .current_avg_total_ms
            .expect("summary current total value"),
        500.0,
    );
    let performance = &activity.summary().model_performance;
    assert!(performance.available);
    assert_eq!(performance.models.len(), 2);
    assert_eq!(performance.models[0].model, "gpt-5.6-performance");
    assert_eq!(
        performance.models[0].reasoning_effort.as_deref(),
        Some("high")
    );
    assert_eq!(performance.models[1].model, "gpt-5.6-zero");
    let range_minutes = (activity.exact_range().end - activity.exact_range().start)
        .num_milliseconds() as f64
        / 60_000.0;
    assert_f64_close(performance.total.tokens_per_minute, 1_400.0 / range_minutes);
    assert_f64_close(
        activity
            .summary()
            .tokens_per_minute
            .expect("summary token rate"),
        0.0,
    );
    assert_f64_close(
        performance
            .total
            .streaming_response_rate
            .expect("streaming response rate"),
        300.0,
    );
    assert_f64_close(
        performance
            .total
            .avg_response_ms
            .expect("response duration"),
        1_200.0,
    );
    assert_f64_close(
        performance
            .total
            .avg_first_response_byte_total_ms
            .expect("first byte duration"),
        100.0,
    );
    assert_f64_close(
        performance
            .total
            .wall_clock_usage_duration_ms
            .expect("wall-clock usage duration"),
        2_800.0,
    );
    assert_f64_close(
        performance
            .total
            .cumulative_usage_duration_ms
            .expect("cumulative usage duration"),
        2_800.0,
    );
    assert_f64_close(
        performance.total.parallelism.expect("summary parallelism"),
        1.0,
    );
    assert_f64_close(
        performance.models[0]
            .metrics
            .cumulative_usage_duration_ms
            .expect("first model cumulative usage duration")
            + performance.models[1]
                .metrics
                .cumulative_usage_duration_ms
                .expect("second model cumulative usage duration"),
        2_800.0,
    );
    assert_f64_close(
        performance.models[0]
            .metrics
            .wall_clock_usage_duration_ms
            .expect("first model wall-clock usage duration"),
        2_300.0,
    );
    assert_f64_close(
        performance.models[1]
            .metrics
            .wall_clock_usage_duration_ms
            .expect("second model wall-clock usage duration"),
        500.0,
    );
    let alpha = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("alpha account");
    assert_f64_close(alpha.tokens_per_minute.expect("alpha rolling tpm"), 0.0);
    assert_f64_close(alpha.spend_rate.expect("alpha rolling spend rate"), 0.0);
    assert_eq!(alpha.model_performance.models.len(), 2);
    assert_eq!(
        alpha.model_performance.models[0].model,
        "gpt-5.6-performance"
    );
    assert_f64_close(
        alpha
            .model_performance
            .total
            .wall_clock_usage_duration_ms
            .expect("alpha wall-clock usage duration"),
        2_800.0,
    );
    assert_f64_close(
        alpha
            .model_performance
            .total
            .cumulative_usage_duration_ms
            .expect("alpha cumulative usage duration"),
        2_800.0,
    );
    assert_f64_close(
        alpha.model_performance.models[0]
            .metrics
            .cumulative_usage_duration_ms
            .expect("alpha primary model cumulative usage duration")
            + alpha.model_performance.models[1]
                .metrics
                .cumulative_usage_duration_ms
                .expect("alpha secondary model cumulative usage duration"),
        2_800.0,
    );
    assert_f64_close(
        alpha
            .model_performance
            .total
            .parallelism
            .expect("alpha parallelism"),
        1.0,
    );
    let source_scope = resolve_default_source_scope(&state.pool)
        .await
        .expect("resolve source scope");
    let summary_only_activity = load_dashboard_activity_summary_only_snapshot(
        state.as_ref(),
        "today",
        source_scope,
        activity.exact_range(),
    )
    .await
    .expect("load summary-only dashboard activity snapshot");
    assert!(summary_only_activity.accounts().is_empty());
    assert_eq!(
        summary_only_activity.summary().model_performance.available,
        activity.summary().model_performance.available,
    );
    let Json(summary_only_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch summary-only dashboard activity snapshot");
    assert!(summary_only_response.accounts.is_none());
    assert_eq!(
        summary_only_activity
            .summary()
            .stats
            .in_progress_conversation_count,
        activity.summary().stats.in_progress_conversation_count,
    );
    assert_eq!(
        summary_only_activity
            .summary()
            .stats
            .in_progress_retry_conversation_count,
        activity
            .summary()
            .stats
            .in_progress_retry_conversation_count,
    );
    let summary_only_tokens_per_minute = summary_only_activity
        .summary()
        .tokens_per_minute
        .expect("summary-only tokens per minute");
    let summary_only_response_tokens_per_minute = summary_only_response
        .summary
        .tokens_per_minute
        .expect("summary-only response tokens per minute");
    assert_f64_close(
        summary_only_tokens_per_minute,
        activity
            .summary()
            .tokens_per_minute
            .expect("full dashboard tokens per minute"),
    );
    assert_f64_close(
        summary_only_response_tokens_per_minute,
        activity
            .summary()
            .tokens_per_minute
            .expect("full dashboard response tokens per minute"),
    );
    let summary_only_spend_rate = summary_only_activity
        .summary()
        .spend_rate
        .expect("summary-only spend rate");
    let summary_only_response_spend_rate = summary_only_response
        .summary
        .spend_rate
        .expect("summary-only response spend rate");
    assert_f64_close(
        summary_only_spend_rate,
        activity
            .summary()
            .spend_rate
            .expect("full dashboard spend rate"),
    );
    assert_f64_close(
        summary_only_response_spend_rate,
        activity
            .summary()
            .spend_rate
            .expect("full dashboard response spend rate"),
    );
    assert_f64_close(
        summary_only_activity
            .summary()
            .current_first_response_byte_total_avg_ms
            .expect("summary-only current first-byte value"),
        40.0,
    );
    assert_f64_close(
        summary_only_activity
            .summary()
            .current_avg_total_ms
            .expect("summary-only current total value"),
        500.0,
    );

    let Json(full_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: None,
        }),
    )
    .await
    .expect("fetch full dashboard activity snapshot");
    let full_accounts = full_response
        .accounts
        .expect("full response accounts included");
    assert_f64_close(
        summary_only_response_tokens_per_minute,
        full_response
            .summary
            .tokens_per_minute
            .expect("full response tokens per minute"),
    );
    assert_f64_close(
        summary_only_response_spend_rate,
        full_response
            .summary
            .spend_rate
            .expect("full response spend rate"),
    );
    assert!(full_response.live_revision > 0);
    assert_eq!(
        full_response.summary.stats.in_progress_conversation_count,
        Some(
            full_accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum::<i64>(),
        ),
    );
    assert_eq!(full_response.rate_window.window_minutes, 1);
    assert_eq!(full_response.rate_window.mode, "rolling_60s_live_mean");
    assert_eq!(full_response.rate_window.end, full_response.range_end);
    let rate_window_start = DateTime::parse_from_rfc3339(&full_response.rate_window.start)
        .expect("dashboard activity rate window start should be RFC3339")
        .with_timezone(&Utc);
    let rate_window_end = DateTime::parse_from_rfc3339(&full_response.rate_window.end)
        .expect("dashboard activity rate window end should be RFC3339")
        .with_timezone(&Utc);
    assert_eq!(
        rate_window_end - rate_window_start,
        ChronoDuration::seconds(60),
    );

    let progressive_started_at = std::time::Instant::now();
    let Json(progressive_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch progressive dashboard activity summary");
    assert!(
        progressive_started_at.elapsed() < std::time::Duration::from_secs(1),
        "progressive dashboard activity summary should complete within one second for the local fixture"
    );
    let progressive_accounts = progressive_response
        .accounts
        .as_ref()
        .expect("progressive response accounts included");
    assert!(
        progressive_accounts
            .iter()
            .all(|account| account.recent_invocations.is_empty())
    );
    assert!(
        progressive_accounts
            .iter()
            .all(|account| account.account_key != "upstream:88")
    );
    let Json(recent_response) = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: progressive_response.range_start.clone(),
            range_end: progressive_response.range_end.clone(),
            snapshot_id: progressive_response.snapshot_id,
            recent_limit: Some(2),
        }),
    )
    .await
    .expect("fetch dashboard activity recent batch");
    assert_eq!(
        recent_response.snapshot_id,
        progressive_response.snapshot_id
    );
    assert_eq!(
        recent_response.range_start,
        progressive_response.range_start
    );
    assert_eq!(recent_response.range_end, progressive_response.range_end);
    assert!(
        recent_response
            .accounts
            .iter()
            .all(|account| account.recent_invocations.len() <= 2)
    );
    assert!(
        recent_response
            .accounts
            .iter()
            .any(|account| account.account_key == "upstream:42")
    );
    assert!(recent_response.accounts.iter().any(|account| {
        account.account_key == "upstream:42"
            && account
                .recent_invocations
                .iter()
                .any(|row| row.invoke_id == "dashboard-activity-runtime-terminal")
    }));
    assert!(recent_response.accounts.iter().any(|account| {
        account.account_key == "upstream:88"
            && account
                .recent_invocations
                .iter()
                .any(|row| row.invoke_id == "dashboard-activity-runtime-terminal-only-account")
    }));
    let wide_recent_result = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: (Utc::now() - ChronoDuration::days(8)).to_rfc3339(),
            range_end: Utc::now().to_rfc3339(),
            snapshot_id: Utc::now().timestamp_millis(),
            recent_limit: Some(2),
        }),
    )
    .await;
    assert!(wide_recent_result.is_err());

    let Json(yesterday_activity) = fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "yesterday".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: None,
        }),
    )
    .await
    .expect("fetch yesterday dashboard activity snapshot");
    assert_eq!(yesterday_activity.live_revision, 0);
    assert_eq!(
        yesterday_activity
            .summary
            .stats
            .in_progress_conversation_count,
        None,
    );
    assert_eq!(
        yesterday_activity
            .summary
            .stats
            .in_progress_retry_conversation_count,
        None,
    );
    let yesterday_accounts = yesterday_activity
        .accounts
        .expect("yesterday accounts included");
    assert_eq!(yesterday_activity.summary.stats.total_tokens, 4_000);
    assert_eq!(yesterday_accounts.len(), 1);
    assert!(
        yesterday_accounts
            .iter()
            .all(|account| account.in_progress_invocation_count.is_none())
    );
    assert!(
        yesterday_accounts
            .iter()
            .all(|account| account.retry_invocation_count.is_none())
    );
}

#[tokio::test]
async fn dashboard_activity_cached_snapshot_overlays_new_live_accounts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [
        (42_i64, "Cached Existing"),
        (99_i64, "Live Only During Cache"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("Primary")
        .bind("enterprise")
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&state.pool)
        .await
        .expect("insert dashboard activity cache account");
    }

    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(90_001_i64)
    .bind("dashboard-cache-existing-success")
    .bind(format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::minutes(2))
            .expect("valid cached existing invocation time"),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.01_f64)
    .bind(json!({ "upstreamAccountId": 42_i64 }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert cached existing invocation");

    let Json(initial_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("populate dashboard activity snapshot cache");
    let initial_accounts = initial_response
        .accounts
        .expect("initial accounts should be included");
    assert!(
        initial_accounts
            .iter()
            .all(|account| account.account_key != "upstream:99")
    );
    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        assert_eq!(cache.entries.len(), 1);
    }

    let cached_existing_recent_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(90_004_i64)
    .bind("dashboard-cache-existing-running")
    .bind(cached_existing_recent_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-cache-existing-running",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert cached existing account running invocation");

    let live_only_occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(90_002_i64)
    .bind("dashboard-cache-live-only-running")
    .bind(live_only_occurred_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-cache-live-only-running",
            "upstreamAccountId": 99_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live-only running invocation");
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(
            &crate::api::ApiInvocation {
                id: 90_003_i64,
                invoke_id: "dashboard-cache-live-only-current-rate".to_string(),
                occurred_at: live_only_occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                proxy_display_name: None,
                model: Some("gpt-5".to_string()),
                request_model: Some("gpt-5".to_string()),
                response_model: Some("gpt-5".to_string()),
                input_tokens: Some(100_i64),
                output_tokens: Some(221_i64),
                cache_input_tokens: Some(0_i64),
                reported_cache_write_tokens: None,
                reasoning_tokens: Some(0_i64),
                reasoning_effort: None,
                total_tokens: Some(321_i64),
                cost: Some(0.32_f64),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                cache_write_tokens: Some(0_i64),
                status: Some("success".to_string()),
                live_phase: None,
                error_message: None,
                downstream_status_code: None,
                failure_kind: None,
                blocked_binding: None,
                blocked_binding_json: None,
                stream_terminal_event: None,
                upstream_error_code: None,
                upstream_error_message: None,
                downstream_error_message: None,
                upstream_request_id: None,
                failure_class: None,
                is_actionable: None,
                endpoint: Some("/v1/responses".to_string()),
                compaction_request_kind: None,
                compaction_response_kind: None,
                image_intent: None,
                requester_ip: None,
                prompt_cache_key: Some("pck-dashboard-cache-live-only-current-rate".to_string()),
                sticky_key: None,
                route_mode: None,
                upstream_account_id: Some(99_i64),
                upstream_account_name: Some("Live Only During Cache".to_string()),
                response_content_encoding: None,
                request_compression_algorithm: None,
                transport: None,
                pool_attempt_count: None,
                pool_distinct_account_count: None,
                pool_attempt_terminal_reason: None,
                requested_service_tier: None,
                service_tier: None,
                billing_service_tier: None,
                proxy_weight_delta: None,
                cost_estimated: None,
                price_version: None,
                cost_audit: None,
                request_raw_path: None,
                request_raw_size: None,
                request_raw_truncated: None,
                request_raw_truncated_reason: None,
                response_raw_path: None,
                response_raw_size: None,
                response_raw_truncated: None,
                response_raw_truncated_reason: None,
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                detail_pruned_at: None,
                detail_prune_reason: None,
                t_total_ms: Some(640.0_f64),
                t_req_read_ms: Some(0.0_f64),
                t_req_parse_ms: Some(0.0_f64),
                t_upstream_connect_ms: Some(0.0_f64),
                t_upstream_ttfb_ms: Some(80.0_f64),
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
                created_at: live_only_occurred_at.clone(),
            },
            Utc::now(),
        );
    reconcile_dashboard_runtime_projection_once(state.as_ref())
        .await
        .expect("reconcile dashboard runtime projection after direct database fixture writes");

    let Json(cached_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("fetch dashboard activity snapshot from cache with live overlay");
    let cached_accounts = cached_response
        .accounts
        .expect("cached accounts should be included");
    let cached_range_end =
        parse_to_utc_datetime(&cached_response.range_end).expect("cached range end should parse");
    let live_only_utc =
        parse_to_utc_datetime(&live_only_occurred_at).expect("live-only occurred_at should parse");
    assert!(cached_range_end >= live_only_utc);
    assert!(cached_response.snapshot_id >= live_only_utc.timestamp_millis());
    let existing = cached_accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("cached snapshot should include existing account");
    assert!(
        existing
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-existing-running")
    );
    let live_only = cached_accounts
        .iter()
        .find(|account| account.account_key == "upstream:99")
        .expect("cached snapshot should include newly live-only account");
    assert_eq!(live_only.display_name, "Live Only During Cache");
    assert_eq!(live_only.request_count, 1);
    assert_eq!(live_only.in_progress_invocation_count, Some(1));
    assert_f64_close(
        live_only
            .tokens_per_minute
            .expect("live-only account tokens per minute"),
        321.0,
    );
    assert_f64_close(
        live_only.spend_rate.expect("live-only account spend rate"),
        0.32,
    );
    assert_f64_close(
        live_only
            .current_first_response_byte_total_avg_ms
            .expect("live-only account current first response byte"),
        80.0,
    );
    assert_f64_close(
        live_only
            .current_avg_total_ms
            .expect("live-only account current total latency"),
        640.0,
    );
    assert!(
        live_only
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-live-only-running")
    );
    assert_eq!(
        cached_response.summary.stats.in_progress_conversation_count,
        Some(
            cached_accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum::<i64>(),
        ),
    );
    assert_f64_close(
        cached_response
            .summary
            .tokens_per_minute
            .expect("cached summary tokens per minute"),
        321.0,
    );
    assert_f64_close(
        cached_response
            .summary
            .spend_rate
            .expect("cached summary spend rate"),
        0.32,
    );
    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        let selections = cache.entries.keys().cloned().collect::<Vec<_>>();
        assert_eq!(selections.len(), 1);
        let baseline = selections.first().expect("dashboard cache selection");
        for selection in &selections {
            assert_eq!(selection.range, "today");
            assert_eq!(selection.range_anchor, baseline.range_anchor);
            assert_eq!(selection.time_zone, baseline.time_zone);
            assert_eq!(selection.source_scope, baseline.source_scope);
            assert_eq!(selection.recent_limit, baseline.recent_limit);
            assert_eq!(selection.include_accounts, baseline.include_accounts);
            assert_eq!(selection.include_recent, baseline.include_recent);
        }
        assert!(cache.in_flight.is_empty());
    }
}

#[tokio::test]
async fn dashboard_activity_cached_snapshot_refreshes_terminal_recent_without_live_account() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [
        (42_i64, "Cached Terminal Existing"),
        (88_i64, "Cached Terminal Only"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("Primary")
        .bind("enterprise")
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&state.pool)
        .await
        .expect("insert cached terminal account");
    }

    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    let local_day_start = base_local
        .date()
        .and_hms_opt(0, 0, 0)
        .expect("valid local day start");
    let existing_persisted_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::minutes(2))
            .expect("valid cached terminal persisted invocation time")
            .max(local_day_start),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(90_101_i64)
    .bind("dashboard-cache-terminal-existing-persisted")
    .bind(existing_persisted_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.01_f64)
    .bind(json!({ "upstreamAccountId": 42_i64 }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert cached terminal existing invocation");

    let Json(initial_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("populate dashboard activity terminal snapshot cache");
    let initial_accounts = initial_response
        .accounts
        .expect("initial terminal cache accounts should be included");
    assert!(
        initial_accounts
            .iter()
            .any(|account| account.account_key == "upstream:42")
    );
    assert!(
        initial_accounts
            .iter()
            .all(|account| account.account_key != "upstream:88")
    );

    let terminal_existing_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::seconds(1))
            .expect("valid cached terminal existing time")
            .max(local_day_start),
    );
    let terminal_only_at = format_naive(base_local);
    let terminal_invocation =
        |id: i64,
         invoke_id: &str,
         occurred_at: &str,
         upstream_account_id: i64,
         upstream_account_name: &str| crate::api::ApiInvocation {
            id,
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: Some("gpt-5".to_string()),
            request_model: Some("gpt-5".to_string()),
            response_model: Some("gpt-5".to_string()),
            input_tokens: Some(0_i64),
            output_tokens: Some(0_i64),
            cache_input_tokens: Some(0_i64),
            reported_cache_write_tokens: None,
            reasoning_tokens: Some(0_i64),
            reasoning_effort: None,
            total_tokens: Some(0_i64),
            cost: Some(0.0_f64),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: Some(0_i64),
            status: Some("success".to_string()),
            live_phase: None,
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: None,
            prompt_cache_key: Some(format!("pck-{invoke_id}")),
            sticky_key: None,
            route_mode: None,
            upstream_account_id: Some(upstream_account_id),
            upstream_account_name: Some(upstream_account_name.to_string()),
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: None,
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            price_version: None,
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: Some(0.0_f64),
            t_req_read_ms: Some(0.0_f64),
            t_req_parse_ms: Some(0.0_f64),
            t_upstream_connect_ms: Some(0.0_f64),
            t_upstream_ttfb_ms: Some(0.0_f64),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: occurred_at.to_string(),
        };
    state
        .proxy_runtime_invocations
        .upsert_terminal(terminal_invocation(
            90_102_i64,
            "dashboard-cache-terminal-existing-runtime",
            terminal_existing_at.as_str(),
            42_i64,
            "Cached Terminal Existing",
        ));
    state
        .proxy_runtime_invocations
        .upsert_terminal(terminal_invocation(
            90_103_i64,
            "dashboard-cache-terminal-only-runtime",
            terminal_only_at.as_str(),
            88_i64,
            "Cached Terminal Only",
        ));

    let Json(cached_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("fetch cached dashboard activity with terminal runtime overlay");
    let cached_accounts = cached_response
        .accounts
        .expect("cached terminal accounts should be included");
    let existing = cached_accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("cached snapshot should include existing terminal account");
    assert!(
        existing
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-terminal-existing-runtime")
    );
    let terminal_only = cached_accounts
        .iter()
        .find(|account| account.account_key == "upstream:88")
        .expect("cached snapshot should include terminal-only account");
    assert_eq!(terminal_only.display_name, "Cached Terminal Only");
    assert_eq!(terminal_only.request_count, 0);
    assert_eq!(terminal_only.in_progress_invocation_count, Some(0));
    assert!(
        terminal_only
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-terminal-only-runtime")
    );
}
