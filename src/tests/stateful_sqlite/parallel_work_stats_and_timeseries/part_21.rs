async fn seed_pending_timeseries_delta(state: &AppState) {
    let record = api_invocation_from_runtime_record(&test_proxy_capture_record(
        "timeseries-projection-p1-priority",
        "2026-08-21 12:00:12",
    ));
    insert_timeseries_invocation(
        &state.pool,
        &record.invoke_id,
        &record.occurred_at,
        "success",
        Some(12.0),
    )
    .await;
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);
    let event_id = state
        .terminal_projection_hub
        .register_pending(&record, None)
        .expect("terminal event should fit in the projection journal");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &record.invoke_id,
        &record.occurred_at,
        91_001,
    );
}

struct RuntimeAccountCapture<'a> {
    account_id: i64,
    account_name: &'a str,
    attempt_count: i64,
    distinct_account_count: i64,
    total_ms: f64,
    first_byte_ms: f64,
    first_token_ms: f64,
    response_ms: f64,
}

async fn persist_runtime_account_capture(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
    capture: RuntimeAccountCapture<'_>,
) {
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        prompt_cache_key: Some("pck-runtime-activity".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.88"),
        None,
        Some("pck-runtime-activity"),
        true,
        Some(capture.account_id),
        Some(capture.account_name),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("runtime-proxy"),
        Some(capture.attempt_count as usize),
        Some(capture.distinct_account_count as usize),
        None,
        None,
        capture.total_ms,
        capture.first_byte_ms,
        capture.first_token_ms,
        capture.response_ms,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, record)
        .await
        .expect("store runtime account activity snapshot in memory");
}

fn assert_runtime_account_activity(activity: serde_json::Value) {
    let accounts = activity["accounts"]
        .as_array()
        .expect("runtime activity accounts");
    assert_eq!(accounts.len(), 1);
    let account = &accounts[0];
    assert_eq!(account["upstreamAccountId"], 89);
    assert_eq!(account["displayName"], "Runtime Pool Retry");
    assert_eq!(account["requestCount"], 0);
    assert_eq!(account["successCount"], 0);
    assert_eq!(account["failureCount"], 0);
    assert_eq!(account["totalTokens"], 0);
    assert_eq!(account["totalCost"], 0.0);
    assert_eq!(
        account["recentInvocations"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(account["inProgressInvocationCount"], 1);
    assert_eq!(account["retryInvocationCount"], 1);
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_keeps_memory_runtime_rows_range_bounded() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    seed_runtime_accounts(&state, &created_at).await;

    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    persist_runtime_account_capture(
        &state,
        "runtime-account-activity-running",
        &occurred_at,
        RuntimeAccountCapture {
            account_id: 88,
            account_name: "Runtime Pool",
            attempt_count: 2,
            distinct_account_count: 1,
            total_ms: 11.0,
            first_byte_ms: 2.0,
            first_token_ms: 33.0,
            response_ms: 44.0,
        },
    )
    .await;

    let persisted_running_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'runtime-account-activity-running'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count persisted memory-only running row");
    assert_eq!(persisted_running_count, 0);
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    persist_runtime_account_capture(
        &state,
        "runtime-account-activity-running",
        &occurred_at,
        RuntimeAccountCapture {
            account_id: 89,
            account_name: "Runtime Pool Retry",
            attempt_count: 3,
            distinct_account_count: 2,
            total_ms: 12.0,
            first_byte_ms: 3.0,
            first_token_ms: 34.0,
            response_ms: 45.0,
        },
    )
    .await;

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity with memory overlay");

    assert_runtime_account_activity(
        serde_json::to_value(activity).expect("serialize runtime activity response"),
    );
}

async fn seed_runtime_accounts(state: &AppState, created_at: &str) {
    for (id, display_name, error_message) in [
        (88_i64, "Runtime Pool", "insert runtime upstream account"),
        (
            89_i64,
            "Runtime Pool Retry",
            "insert retry runtime upstream account",
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("Primary")
        .bind("team")
        .bind("active")
        .bind(1_i64)
        .bind(created_at)
        .bind(created_at)
        .execute(&state.pool)
        .await
        .expect(error_message);
    }
}

#[tokio::test]
pub(crate) async fn runtime_summary_phase_ignores_zero_placeholder_before_positive_timing() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        prompt_cache_key: Some("pck-runtime-phase".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let record = build_running_proxy_capture_record(
        "runtime-phase-zero-connect",
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.90"),
        None,
        Some("pck-runtime-phase"),
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        9.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("store runtime phase snapshot in memory");

    let Json(stats) = fetch_stats(State(state))
        .await
        .expect("fetch stats with memory runtime phase snapshot");
    assert_eq!(stats.in_progress_conversation_count, Some(1));
    let phase_counts = stats
        .in_progress_phase_counts
        .expect("stats should include memory runtime phase counts");
    assert_eq!(phase_counts.queued, 0);
    assert_eq!(phase_counts.requesting, 1);
    assert_eq!(phase_counts.responding, 0);
}

#[tokio::test]
pub(crate) async fn account_scoped_historical_stats_include_unmaterialized_archived_hours() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let (account_id, archived_hour_local) = seed_account_scoped_archive(&state).await;

    let Json(all_summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("fetch account all-time summary with unmaterialized archive");

    assert_eq!(all_summary.total_count, 2);
    assert_eq!(all_summary.success_count, 1);
    assert_eq!(all_summary.failure_count, 1);
    assert_eq!(all_summary.total_tokens, 30);
    assert_f64_close(all_summary.total_cost, 0.30);

    let historical_range = "30d".to_string();
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some(historical_range.clone()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("fetch account historical summary with unmaterialized archive");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert_f64_close(summary.total_cost, 0.30);
    assert_f64_close(
        summary
            .non_success_cost
            .expect("account historical non-success cost should include archived failures"),
        0.20,
    );
    assert_eq!(summary.non_success_tokens, Some(20));
    let usage_breakdown = summary
        .usage_breakdown
        .expect("account historical summary should include usage breakdown");
    assert_eq!(usage_breakdown.cache_write_tokens, 0);
    assert_eq!(usage_breakdown.cache_read_tokens, 0);
    assert_eq!(usage_breakdown.output_tokens, 0);
    let usage_costs = usage_breakdown
        .costs
        .expect("account historical usage breakdown should preserve known total cost");
    assert_f64_close(usage_costs.unknown, 0.30);

    let Json(timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: historical_range,
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("fetch account historical timeseries with unmaterialized archive");

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let archived_point = timeseries
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("account historical timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 2);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 1);
    assert_eq!(archived_point.total_tokens, 30);
    assert_f64_close(archived_point.total_cost, 0.30);
    assert_f64_close(archived_point.non_success_cost, 0.20);

    let account_usage_replay_markers: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1")
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
            .fetch_one(&state.pool)
            .await
            .expect("load account usage replay marker count");
    assert_eq!(account_usage_replay_markers, 0);
}

async fn seed_account_scoped_archive(state: &AppState) -> (i64, chrono::NaiveDateTime) {
    let account_id = 42_i64;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(11, 0, 0)
    .expect("valid account archived hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived account success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(35))
            .expect("archived account failure time"),
    );
    let archived_other_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("archived other account time"),
    );
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "account-stats-unmaterialized-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "account-stats-unmaterialized-success",
                occurred_at: archived_success_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0),
                payload: Some(r#"{"upstreamAccountId":42}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "account-stats-unmaterialized-failed",
                occurred_at: archived_failed_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":42}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 3_i64,
                invoke_id: "account-stats-unmaterialized-other-account",
                occurred_at: archived_other_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 90_i64,
                cost: 0.90_f64,
                ttfb_ms: Some(90.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
        ],
    )
    .await;
    (account_id, archived_hour_local)
}

#[tokio::test]
pub(crate) async fn combined_totals_count_legacy_null_status_failures_when_error_metadata_exists() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            failure_kind,
            total_tokens,
            cost,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(204_i64)
    .bind("summary-null-status-failure")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind("upstream exploded")
    .bind("unknown_future_failure_kind")
    .bind(5_i64)
    .bind(0.05_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy null-status failure row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 1);
    assert_eq!(totals.total_tokens, 5);
    assert_f64_close(totals.total_cost, 0.05);
}

#[tokio::test]
pub(crate) async fn combined_totals_count_legacy_null_status_failures_when_only_downstream_error_exists()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
    .bind(205_i64)
    .bind("summary-null-status-downstream-only")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind(6_i64)
    .bind(0.06_f64)
    .bind(
        json!({
            "downstreamErrorMessage": "downstream closed while streaming upstream response"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy null-status downstream-only failure row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 1);
    assert_eq!(totals.total_tokens, 6);
    assert_f64_close(totals.total_cost, 0.06);
}

#[tokio::test]
pub(crate) async fn combined_totals_treat_legacy_http_200_without_error_as_success() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(203_i64)
    .bind("summary-http-200-success-like")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind(9_i64)
    .bind(0.09_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy http_200 invocation row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 9);
    assert_f64_close(totals.total_cost, 0.09);
}

#[tokio::test]
pub(crate) async fn combined_totals_count_legacy_http_200_failures_when_only_downstream_error_exists()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
    .bind(206_i64)
    .bind("summary-http-200-downstream-only")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind(8_i64)
    .bind(0.08_f64)
    .bind(
        json!({
            "downstreamErrorMessage": "socket closed after response"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy http_200 downstream-only failure row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 1);
    assert_eq!(totals.total_tokens, 8);
    assert_f64_close(totals.total_cost, 0.08);
}

#[test]
pub(crate) fn resolve_failure_classification_keeps_unknown_legacy_http_200_failure_kinds_actionable()
 {
    let classification = resolve_failure_classification(
        Some("http_200"),
        Some(""),
        Some("unknown_future_failure_kind"),
        None,
        None,
    );

    assert_eq!(classification.failure_class, FailureClass::ServiceFailure);
    assert_eq!(
        classification.failure_kind.as_deref(),
        Some("unknown_future_failure_kind"),
    );
    assert!(classification.is_actionable);
}

#[test]
pub(crate) fn resolve_failure_classification_keeps_completed_rows_with_failure_kind_as_failures() {
    let classification = resolve_failure_classification(
        Some("completed"),
        Some(""),
        Some("unknown_future_failure_kind"),
        None,
        None,
    );

    assert_eq!(classification.failure_class, FailureClass::ServiceFailure);
    assert_eq!(
        classification.failure_kind.as_deref(),
        Some("unknown_future_failure_kind"),
    );
    assert!(classification.is_actionable);
}

fn legacy_http_200_record(
    id: i64,
    error_message: &str,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> InvocationHourlySourceRecord {
    InvocationHourlySourceRecord {
        id,
        occurred_at: "2026-03-28 00:00:00".to_string(),
        source: SOURCE_PROXY.to_string(),
        status: Some("http_200".to_string()),
        detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
        cost: None,
        upstream_account_id: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        error_message: Some(error_message.to_string()),
        failure_kind: failure_kind.map(str::to_string),
        failure_class: failure_class.map(str::to_string),
        is_actionable,
        payload: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    }
}

#[test]
pub(crate) fn invocation_archive_pruned_success_details_require_empty_legacy_http_200_error_message()
 {
    let failed_legacy_http_200 =
        legacy_http_200_record(1, "upstream parse failed", None, None, None);
    assert!(
        !invocation_archive_has_pruned_success_details(&[failed_legacy_http_200]),
        "legacy http_200 rows with a non-empty error message must not suppress archive rollups",
    );

    let success_like_legacy_http_200 = legacy_http_200_record(2, "   ", None, None, None);
    assert!(
        invocation_archive_has_pruned_success_details(&[success_like_legacy_http_200]),
        "legacy http_200 rows with an empty error message should still count as pruned success-like rows",
    );

    let structured_failure_legacy_http_200 = legacy_http_200_record(
        3,
        "   ",
        Some("upstream_response_failed"),
        Some("service_failure"),
        Some(1),
    );
    assert!(
        !invocation_archive_has_pruned_success_details(&[structured_failure_legacy_http_200]),
        "legacy http_200 rows with structured failure metadata must not be treated as pruned successes",
    );
}

#[tokio::test]
pub(crate) async fn dashboard_activity_response_duration_includes_success_without_cost() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
            t_upstream_stream_ms, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(98_001_i64)
    .bind("dashboard-activity-unpriced-response")
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(0_i64)
    .bind(0_i64)
    .bind(Option::<f64>::None)
    .bind(750.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-dashboard-activity-unpriced-response",
            "responseModel": "custom-unpriced-model",
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert unpriced response-duration invocation");

    let activity =
        load_dashboard_activity_snapshot(state.as_ref(), "today", Shanghai, 1, true, false, None)
            .await
            .expect("load dashboard activity with unpriced response duration");
    let model = activity
        .summary()
        .model_performance
        .models
        .iter()
        .find(|model| model.model == "custom-unpriced-model")
        .expect("unpriced model performance row");
    assert_f64_close(
        model
            .metrics
            .avg_response_ms
            .expect("unpriced response duration sample"),
        750.0,
    );
}

#[tokio::test]
pub(crate) async fn dashboard_account_model_performance_ignores_stale_retry_timing() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, model, status, total_tokens, cost,
            first_token_ms, t_upstream_stream_ms, payload, raw_response
        ) VALUES (?1, ?2, ?3, 'gpt-retry', 'success', 100, 0.01, ?4, ?5, ?6, '{}')
        "#,
    )
    .bind("account-stale-retry-timing")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(900.0_f64)
    .bind(800.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-account-stale-retry",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("insert account retry invocation");
    for (public_id, index, stream_ms, first_byte_ms) in [
        ("ACCOUNTSTALE1", 1_i64, Some(800.0_f64), Some(100.0_f64)),
        ("ACCOUNTSTALE2", 2_i64, None, None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                attempt_index, distinct_account_index, same_account_retry_index,
                requester_ip, started_at, finished_at, status, phase,
                stream_latency_ms, first_byte_latency_ms, created_at
            ) VALUES (?1, ?2, ?3, '/v1/responses', 'pool', ?4, 1, 0,
                      '127.0.0.1', ?3, ?3, 'success', 'completed', ?5, ?6, ?3)
            "#,
        )
        .bind(public_id)
        .bind("account-stale-retry-timing")
        .bind(&occurred_at)
        .bind(index)
        .bind(stream_ms)
        .bind(first_byte_ms)
        .execute(&state.pool)
        .await
        .expect("insert account retry attempt");
    }

    let activity =
        load_dashboard_activity_snapshot(state.as_ref(), "today", Shanghai, 2, true, false, None)
            .await
            .expect("load account model performance snapshot");
    let account = activity
        .accounts()
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("account model performance row");
    assert_eq!(account.model_performance.total.avg_first_token_ms, None);
    assert_eq!(account.model_performance.total.avg_response_ms, None);
}

#[tokio::test]
pub(crate) async fn minute_projection_flush_defers_while_p1_terminal_write_is_active() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pending_timeseries_delta(&state).await;

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let p1_write = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_p1_priority",
        &coordinator,
    )
    .await
    .expect("flush should yield without a database error");
    assert!(matches!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Deferred(_)
    ));
    assert_eq!(
        state
            .terminal_projection_hub
            .pending_timeseries_deltas(10)
            .len(),
        1,
        "the P1 preemption path must leave the delta available for retry"
    );
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count minute projections before retry");
    assert_eq!(
        projection_count, 0,
        "deferred work must not open a write transaction"
    );

    drop(p1_write);
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_p1_priority_retry",
        &coordinator,
    )
    .await
    .expect("flush should succeed after P1 completes");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    assert!(
        state
            .terminal_projection_hub
            .pending_timeseries_deltas(10)
            .is_empty(),
        "the retried flush should acknowledge the terminal delta"
    );
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count minute projections after retry");
    assert!(
        projection_count > 0,
        "retry should persist the minute projections"
    );
    let aggregate_json = sqlx::query_scalar::<_, String>(
        "SELECT aggregate_json FROM timeseries_minute_projection_v2 WHERE source_scope = 'all' AND upstream_account_key = -1 AND coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load the all-scope minute aggregate after retry");
    let aggregate: serde_json::Value =
        serde_json::from_str(&aggregate_json).expect("valid persisted minute aggregate");
    assert_eq!(
        aggregate["total_count"].as_i64(),
        Some(1),
        "retry must persist the terminal aggregate, not only a ready projection row"
    );
}

use super::*;
