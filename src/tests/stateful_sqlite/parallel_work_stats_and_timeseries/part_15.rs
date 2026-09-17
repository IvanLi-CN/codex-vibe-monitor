#[tokio::test]
pub(crate) async fn usage_breakdown_prefers_breakdown_specific_archive_progress_over_shared_cursor()
{
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    seed_breakdown_progress_override(&state).await;
    let summary = super::part_14::fetch_usage_summary(state, "7d").await;
    super::part_14::assert_summary_unknown_model_cost(summary, 2, 30, 0.30);
}

#[tokio::test]
pub(crate) async fn summary_topic_builder_matches_http_for_open_range() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_summary_topic_rows(&state.pool, &occurred_at).await;

    let query = SummaryQuery {
        window: Some("7d".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    let Json(http_summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: query.window.clone(),
            limit: query.limit,
            time_zone: query.time_zone.clone(),
            upstream_account_id: query.upstream_account_id,
        }),
    )
    .await
    .expect("fetch summary over http");
    let topic_summary =
        load_summary_response_from_query(state.as_ref(), &query, SummaryBuildRoute::Topic)
            .await
            .expect("build summary for topic");

    assert_eq!(
        serde_json::to_value(http_summary).expect("serialize http summary"),
        serde_json::to_value(topic_summary).expect("serialize topic summary"),
    );
}

async fn seed_summary_topic_rows(pool: &SqlitePool, occurred_at: &str) {
    for (id, invoke_id, status, tokens, cost, parts, error, kind, class) in [
        (
            470,
            "summary-topic-success",
            "success",
            120,
            Some(0.50),
            Some([0.06, 0.14, 0.04, 0.21, 0.05]),
            None,
            None,
            Some("none"),
        ),
        (
            471,
            "summary-topic-failed",
            "failed",
            80,
            Some(0.20),
            None,
            Some("HTTP 429 too many requests"),
            Some("upstream_response_failed"),
            Some("service_failure"),
        ),
    ] {
        let [input, cache_write, cache_read, output, reasoning] = parts.unwrap_or([0.0; 5]);
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
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("gpt-5.6")
        .bind(status)
        .bind(100_i64)
        .bind(40_i64)
        .bind(25_i64)
        .bind(5_i64)
        .bind(tokens)
        .bind(cost)
        .bind(parts.map(|_| input))
        .bind(parts.map(|_| cache_write))
        .bind(parts.map(|_| cache_read))
        .bind(parts.map(|_| output))
        .bind(parts.map(|_| reasoning))
        .bind(error)
        .bind(kind)
        .bind(class)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert summary topic builder row");
    }
}

#[tokio::test]
pub(crate) async fn empty_summary_response_keeps_live_in_progress_conversation_count() {
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
pub(crate) async fn summary_ignores_runtime_overlay_records_with_terminal_db_rows() {
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
    seed_runtime_overlay_summary(&state, &occurred_at, &old_running_occurred_at).await;

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
pub(crate) async fn natural_day_summary_reports_retry_wait_and_non_success_usage() {
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

    seed_natural_day_summary_rows(&state.pool, &occurred_at, &earlier_today).await;
    rebuild_summary_rollups(&state.pool).await;
    let summary = super::part_14::fetch_usage_summary(state, "today").await;

    assert_eq!(summary.in_progress_conversation_count, Some(4));
    assert_eq!(summary.in_progress_retry_conversation_count, Some(2));
    assert_f64_close(
        summary.in_progress_avg_wait_ms.expect("wait average"),
        1800.0,
    );
    assert_f64_close(summary.non_success_cost.expect("non-success cost"), 0.19);
    assert_eq!(summary.non_success_tokens, Some(190));
}

async fn seed_natural_day_summary_rows(pool: &SqlitePool, occurred_at: &str, earlier: &str) {
    for (id, invoke_id, status, tokens, cost, error, kind, ttfb, payload) in natural_day_rows() {
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, error_message, failure_kind, t_upstream_ttfb_ms, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, '{}')",
        )
        .bind(id).bind(invoke_id).bind(if id == 501 { earlier } else { occurred_at })
        .bind(SOURCE_PROXY).bind(status).bind(tokens).bind(cost).bind(error).bind(kind)
        .bind(ttfb).bind(payload).execute(pool).await
        .expect("insert natural-day summary augmentation row");
    }
}

type NaturalDaySummaryRow = (
    i64,
    &'static str,
    &'static str,
    i64,
    f64,
    Option<&'static str>,
    Option<&'static str>,
    Option<f64>,
    Option<String>,
);

fn natural_day_rows() -> Vec<NaturalDaySummaryRow> {
    vec![
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
    ]
}

async fn rebuild_summary_rollups(pool: &SqlitePool) {
    let mut tx = pool.begin().await.expect("begin rollup rebuild tx");
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
}

#[tokio::test]
pub(crate) async fn account_scoped_natural_day_summary_keeps_augmentation_fields_scoped() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_account_scoped_augmentation_rows(&state.pool, &occurred_at).await;

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
    assert_f64_close(summary.non_success_cost.expect("non-success cost"), 0.22);
    assert_f64_close(
        summary.in_progress_avg_wait_ms.expect("in-progress wait"),
        1700.0,
    );
}

async fn seed_account_scoped_augmentation_rows(pool: &SqlitePool, occurred_at: &str) {
    for (id, invoke_id, account_id, status, tokens, cost, ttfb, payload) in [
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
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(tokens)
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
        .bind(ttfb)
        .bind(payload)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert account scoped summary augmentation row");
        assert!(account_id > 0);
    }
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_groups_active_accounts_and_hides_yesterday_live_counts()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_upstream_activity_account(&state.pool).await;
    seed_upstream_activity_invocations(&state.pool).await;
    let activity = fetch_account_activity(state.clone(), "today", None).await;
    let account = activity.accounts.first().expect("activity account");
    assert_upstream_activity_identity(account);
    assert_upstream_activity_metrics(account);
    assert_upstream_activity_routing_and_recent(account);
    assert_activity_query_variants(state).await;
}

fn assert_upstream_activity_identity(account: &UpstreamAccountActivityAccountResponse) {
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
}

fn assert_upstream_activity_metrics(account: &UpstreamAccountActivityAccountResponse) {
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
}

fn assert_upstream_activity_routing_and_recent(account: &UpstreamAccountActivityAccountResponse) {
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
}

async fn assert_activity_query_variants(state: Arc<AppState>) {
    let expanded_activity = fetch_account_activity(state.clone(), "today", Some(6)).await;
    let expanded_account = expanded_activity
        .accounts
        .first()
        .expect("expanded activity account");
    assert_eq!(expanded_account.recent_invocations.len(), 6);
    assert_eq!(
        expanded_account.recent_invocations[5].invoke_id,
        "upstream-activity-running"
    );

    let compact_activity = fetch_account_activity(state.clone(), "today", Some(2)).await;
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

    let yesterday_activity = fetch_account_activity(state, "yesterday", Some(4)).await;

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

async fn fetch_account_activity(
    state: Arc<AppState>,
    range: &str,
    recent_limit: Option<i64>,
) -> UpstreamAccountActivityResponse {
    fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: range.to_string(),
            recent_limit,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("fetch {range} account activity: {error:?}"))
    .0
}

async fn seed_upstream_activity_account(pool: &SqlitePool) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query("INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at) VALUES (42, 'api_key_codex', 'codex', 'Pool Alpha', 'Primary', 'enterprise', 'active', 1, ?1, ?1)")
        .bind(&created_at).execute(pool).await.expect("insert upstream activity account");
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_in = 0, policy_priority_tier = 'no_new', policy_fast_mode_rewrite_mode = 'force_add', policy_concurrency_limit = 3, policy_upstream_429_retry_enabled = 1, policy_upstream_429_max_retries = 2 WHERE id = 42")
        .execute(pool).await.expect("set upstream activity routing policy");
}

struct ActivityInvocationFixture {
    id: i64,
    invoke_id: &'static str,
    status: &'static str,
    total_tokens: i64,
    cost: f64,
    cache_input_tokens: i64,
    prompt_cache_key: &'static str,
    timings: [Option<f64>; 5],
    error_message: Option<&'static str>,
    failure_kind: Option<&'static str>,
}

impl ActivityInvocationFixture {
    fn new(
        id: i64,
        invoke_id: &'static str,
        status: &'static str,
        total_tokens: i64,
        cost: f64,
        cache_input_tokens: i64,
        prompt_cache_key: &'static str,
    ) -> Self {
        Self {
            id,
            invoke_id,
            status,
            total_tokens,
            cost,
            cache_input_tokens,
            prompt_cache_key,
            timings: [None; 5],
            error_message: None,
            failure_kind: None,
        }
    }

    fn with_timings(mut self, timings: [f64; 5]) -> Self {
        self.timings = timings.map(Some);
        self
    }

    fn with_failure(mut self, message: &'static str, kind: &'static str) -> Self {
        self.error_message = Some(message);
        self.failure_kind = Some(kind);
        self
    }
}

fn upstream_activity_rows() -> Vec<ActivityInvocationFixture> {
    vec![
        ActivityInvocationFixture::new(
            601,
            "upstream-activity-running",
            "running",
            100,
            0.10,
            20,
            "pck-upstream-a",
        )
        .with_timings([80.0, 4.0, 2100.0, 410.0, 1000.0]),
        ActivityInvocationFixture::new(
            602,
            "upstream-activity-success",
            "success",
            300,
            0.30,
            60,
            "pck-upstream-a",
        )
        .with_timings([83.434948, 2.762219, 2123.797426, 0.006021, 2000.0]),
        ActivityInvocationFixture::new(
            603,
            "upstream-activity-failed",
            "failed",
            200,
            0.20,
            40,
            "pck-upstream-a",
        )
        .with_timings([51.0, 3.0, 1500.0, 450.0, 3000.0])
        .with_failure("upstream failed", "upstream_response_failed"),
        ActivityInvocationFixture::new(
            604,
            "upstream-activity-pending-retry",
            "pending",
            50,
            0.05,
            10,
            "pck-upstream-a",
        ),
        ActivityInvocationFixture::new(
            605,
            "upstream-activity-extra-running",
            "running",
            75,
            0.07,
            15,
            "pck-upstream-b",
        )
        .with_timings([74.0, 4.0, 1800.0, 405.0, 1500.0]),
        ActivityInvocationFixture::new(
            606,
            "upstream-activity-extra-success",
            "success",
            125,
            0.12,
            25,
            "pck-upstream-c",
        )
        .with_timings([49.329286, 1.973188, 3474.776073, 0.002344, 2000.0]),
    ]
}

async fn seed_upstream_activity_invocations(pool: &SqlitePool) {
    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    for row in upstream_activity_rows() {
        let [read_ms, parse_ms, connect_ms, ttfb_ms, total_ms] = row.timings;
        let occurred_at = format_naive(base_local - ChronoDuration::seconds((607 - row.id) * 10));
        let payload = json!({ "promptCacheKey": row.prompt_cache_key, "upstreamAccountId": 42,
            "upstreamAccountName": "Pool Alpha" })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, cache_input_tokens, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, error_message, failure_kind, t_upstream_ttfb_ms, t_total_ms, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, '{}')")
            .bind(row.id).bind(row.invoke_id).bind(occurred_at).bind(SOURCE_PROXY)
            .bind(row.status).bind(row.total_tokens).bind(row.cost).bind(row.cache_input_tokens)
            .bind(read_ms).bind(parse_ms).bind(connect_ms).bind(row.error_message)
            .bind(row.failure_kind).bind(ttfb_ms).bind(total_ms).bind(payload)
            .execute(pool).await.expect("insert upstream account activity invocation");
    }
}

async fn seed_breakdown_progress_override(state: &AppState) {
    let (path, first_at) = super::part_14::seed_two_breakdown_rows(
        state,
        "summary-7d-breakdown-progress-override",
        801,
    )
    .await;
    let record = super::part_14::breakdown_rollup_record(801, first_at, 10, 0.10, 100.0);
    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[record],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed breakdown rollup");
    tx.commit().await.expect("commit breakdown rollup tx");
    super::part_14::insert_archive_progress(
        &state.pool,
        &path,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        802,
    )
    .await;
    super::part_14::insert_archive_progress(
        &state.pool,
        &path,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        801,
    )
    .await;
}

async fn seed_runtime_overlay_summary(state: &Arc<AppState>, terminal_at: &str, active_at: &str) {
    persist_summary_runtime_record(state, terminal_at, false).await;
    insert_runtime_terminal_row(&state.pool, terminal_at).await;
    persist_summary_runtime_record(state, active_at, true).await;
}

async fn persist_summary_runtime_record(state: &Arc<AppState>, occurred_at: &str, active: bool) {
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        prompt_cache_key: Some("pck-summary-runtime-terminal".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let (invoke_id, client_ip, cache_key, account_id, account_name, attempts, wait_ms) = if active {
        (
            "summary-runtime-active",
            "203.0.113.43",
            "pck-summary-runtime-active",
            43,
            "Runtime Summary Active",
            1,
            333.0,
        )
    } else {
        (
            "summary-runtime-terminal",
            "203.0.113.42",
            "pck-summary-runtime-terminal",
            42,
            "Runtime Summary",
            2,
            910.0,
        )
    };
    let record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some(client_ip),
        Some(invoke_id),
        Some(cache_key),
        true,
        Some(account_id),
        Some(account_name),
        Some("api_key_codex"),
        Some("api.openai.com"),
        Some("runtime-summary-proxy"),
        Some(attempts),
        Some(1),
        None,
        None,
        if active { 11.0 } else { 10.0 },
        if active { 3.0 } else { 2.0 },
        if active { 34.0 } else { 33.0 },
        wait_ms,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, record)
        .await
        .expect("store runtime summary snapshot");
}

async fn insert_runtime_terminal_row(pool: &SqlitePool, occurred_at: &str) {
    sqlx::query(
        "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, payload, raw_response) VALUES (481, 'summary-runtime-terminal', ?1, ?2, 'success', 100, 0.1, 910.0, ?3, '{}')",
    )
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(json!({ "promptCacheKey": "pck-summary-runtime-terminal" }).to_string())
    .execute(pool)
    .await
    .expect("insert terminal DB row for stale runtime snapshot");
}

use super::*;
