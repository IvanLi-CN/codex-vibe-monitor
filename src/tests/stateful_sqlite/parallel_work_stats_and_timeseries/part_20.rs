async fn seed_dashboard_account(state: &AppState, id: i64, name: &str) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_accounts \
         (id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at) \
         VALUES (?1, 'api_key_codex', 'codex', ?2, 'Primary', 'enterprise', 'active', 1, ?3, ?3)",
    )
    .bind(id)
    .bind(name)
    .bind(created_at)
    .execute(&state.pool)
    .await
    .expect("insert dashboard account");
}

async fn seed_dashboard_live_invocation(
    state: &AppState,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
    total_tokens: i64,
) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) \
         VALUES (?1, ?2, ?3, ?4, 'success', ?5, 0.01, ?6, '{}')",
    )
    .bind(id)
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(total_tokens)
    .bind(
        json!({
            "promptCacheKey": format!("pck-{invoke_id}"),
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("insert dashboard live invocation");
}

async fn fetch_archived_dashboard_activity(
    state: Arc<AppState>,
    include_recent: Option<bool>,
) -> DashboardActivityResponse {
    fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent,
        }),
    )
    .await
    .expect("fetch archived dashboard activity")
    .0
}

async fn seed_upstream_activity_invocation(
    state: &AppState,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
    source: &str,
) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) \
         VALUES (?1, ?2, ?3, ?4, 'completed', 120, 0.12, ?5, '{}')",
    )
    .bind(id)
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(
        json!({
            "promptCacheKey": "pck-upstream-mixed",
            "upstreamAccountId": 42,
            "upstreamAccountName": "Pool Alpha"
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("insert mixed-source upstream account invocation");
}

async fn seed_pool_running_activity_rows(state: &AppState, occurred_at: &str) {
    for (id, invoke_id, status, payload) in [
        (
            7_700_i64,
            "pool-running-selected-before-payload-update-previous-failed",
            "failed",
            json!({
                "promptCacheKey": "pck-pool-fallback",
                "routeMode": "pool",
                "upstreamAccountId": 77
            })
            .to_string(),
        ),
        (
            7_701_i64,
            "pool-running-selected-before-payload-update",
            "running",
            json!({ "promptCacheKey": "pck-pool-fallback", "routeMode": "pool" }).to_string(),
        ),
    ] {
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0.0, ?6, '{}')",
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(payload)
        .execute(&state.pool)
        .await
        .expect("insert pool activity invocation");
    }
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, sticky_key, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, status, phase, created_at) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, ?5, 1, 1, 0, '127.0.0.1', ?2, 'running', 'streaming_response', datetime('now'))",
    )
    .bind("pool-running-selected-before-payload-update")
    .bind(occurred_at)
    .bind("pck-pool-fallback")
    .bind(77_i64)
    .bind("route-pool-fallback")
    .execute(&state.pool)
    .await
    .expect("insert selected pool attempt for running invocation");
}

async fn assert_pool_running_account_summary(state: Arc<AppState>) {
    let Json(account_summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(77),
        }),
    )
    .await
    .expect("fetch fallback account summary");
    assert_eq!(account_summary.in_progress_conversation_count, Some(0));
    assert_eq!(
        account_summary.in_progress_retry_conversation_count,
        Some(0)
    );
    let phase_counts = account_summary
        .in_progress_phase_counts
        .expect("account summary should include live phase counts");
    assert_eq!(phase_counts.queued, 0);
    assert_eq!(phase_counts.requesting, 0);
    assert_eq!(phase_counts.responding, 0);
}

fn pool_running_runtime_invocation(occurred_at: String) -> crate::api::ApiInvocation {
    let mut invocation = summary_projection_test_invocation();
    invocation.id = 7_701;
    invocation.invoke_id = "pool-running-selected-before-payload-update".to_string();
    invocation.occurred_at = occurred_at.clone();
    invocation.created_at = occurred_at;
    invocation.source = SOURCE_PROXY.to_string();
    invocation.model = Some("gpt-5".to_string());
    invocation.request_model = Some("gpt-5".to_string());
    invocation.response_model = Some("gpt-5".to_string());
    invocation.status = Some("running".to_string());
    invocation.live_phase = None;
    invocation.endpoint = Some("/v1/responses".to_string());
    invocation.requester_ip = Some("127.0.0.1".to_string());
    invocation.prompt_cache_key = Some("pck-pool-fallback".to_string());
    invocation.sticky_key = Some("pck-pool-fallback".to_string());
    invocation.route_mode = Some("pool".to_string());
    invocation.pool_attempt_count = None;
    invocation.upstream_account_id = None;
    invocation.upstream_account_name = None;
    invocation.total_tokens = Some(0);
    invocation.cost = Some(0.0);
    invocation.t_total_ms = None;
    invocation.t_req_read_ms = None;
    invocation.t_req_parse_ms = None;
    invocation.t_upstream_connect_ms = None;
    invocation.t_upstream_ttfb_ms = Some(120.0);
    invocation.t_upstream_stream_ms = None;
    invocation.t_resp_parse_ms = None;
    invocation.t_persist_ms = None;
    invocation.first_token_ms = Some(120.0);
    invocation
}

fn assert_archived_dashboard_account(response: DashboardActivityResponse, last_at: &str) {
    let account = response
        .accounts
        .expect("accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("archived account summary");
    assert_eq!(
        (
            account.request_count,
            account.success_count,
            account.failure_count
        ),
        (2, 1, 1)
    );
    assert_eq!(account.total_tokens, 30);
    assert_f64_close(account.total_cost, 0.30);
    assert_eq!(
        account.latest_conversation_created_at.as_deref(),
        Some(last_at)
    );
    assert_eq!(account.last_invocation_at.as_deref(), Some(last_at));
    assert!(account.recent_invocations.is_empty());
}

fn assert_compatible_archived_dashboard_account(response: DashboardActivityResponse) {
    let account = response
        .accounts
        .expect("compatible accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("compatible archived account summary");
    assert_eq!(account.request_count, 2);
    assert_eq!(account.total_tokens, 30);
    assert_eq!(account.recent_invocations.len(), 2);
}

#[tokio::test]
pub(crate) async fn dashboard_activity_recent_excludes_archived_rows_for_all_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    seed_dashboard_account(&state, 42, "Archive Overlap").await;

    let range_start = Utc::now() - ChronoDuration::days(1);
    let range_end = range_start + ChronoDuration::minutes(10);
    let live_overlap_at = format_naive(
        (range_start + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let live_selected_at = format_naive(
        (range_start + ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_newer_at = format_naive(
        (range_start + ChronoDuration::minutes(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (id, invoke_id, occurred_at, total_tokens) in [
        (
            91_001_i64,
            "dashboard-recent-live-overlap",
            live_overlap_at.as_str(),
            10_i64,
        ),
        (
            91_002_i64,
            "dashboard-recent-live-selected",
            live_selected_at.as_str(),
            20_i64,
        ),
    ] {
        seed_dashboard_live_invocation(&state, id, invoke_id, occurred_at, total_tokens).await;
    }

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-recent-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 91_001_i64,
            invoke_id: "dashboard-recent-live-overlap",
            occurred_at: archive_newer_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.01_f64,
            ttfb_ms: None,
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-recent-live-overlap","upstreamAccountId":42}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: range_start.to_rfc3339(),
            range_end: range_end.to_rfc3339(),
            snapshot_id: range_end.timestamp_millis(),
            recent_limit: Some(1),
        }),
    )
    .await
    .expect("fetch dashboard activity recent with overlapping archive row");
    let account = response
        .accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("recent response should include account");
    assert_eq!(account.recent_invocations.len(), 1);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "dashboard-recent-live-selected"
    );
}

#[tokio::test]
pub(crate) async fn dashboard_activity_recent_chunks_large_preview_hydration_id_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let range_start = Utc::now() - ChronoDuration::hours(1);
    let range_end = range_start + ChronoDuration::hours(2);
    for account_id in 1_i64..=64_i64 {
        for invocation_index in 0_i64..16_i64 {
            let occurred_at = format_naive(
                (range_start + ChronoDuration::seconds(account_id * 20_i64 + invocation_index))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            );
            let invoke_id = format!("dashboard-large-preview-{account_id}-{invocation_index}");
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
            )
            .bind(93_000_i64 + account_id * 100_i64 + invocation_index)
            .bind(invoke_id.as_str())
            .bind(occurred_at.as_str())
            .bind(SOURCE_PROXY)
            .bind("success")
            .bind(1_i64)
            .bind(0.01_f64)
            .bind(
                json!({
                    "promptCacheKey": format!("pck-{invoke_id}"),
                    "upstreamAccountId": account_id,
                })
                .to_string(),
            )
            .bind("{}")
            .execute(&state.pool)
            .await
            .expect("insert large preview hydration invocation");
        }
    }

    let Json(response) = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: range_start.to_rfc3339(),
            range_end: range_end.to_rfc3339(),
            snapshot_id: range_end.timestamp_millis(),
            recent_limit: Some(16),
        }),
    )
    .await
    .expect("fetch dashboard activity recent with chunked preview hydration");
    assert_eq!(response.accounts.len(), 64);
    assert!(
        response
            .accounts
            .iter()
            .all(|account| account.recent_invocations.len() == 16)
    );
}

#[tokio::test]
pub(crate) async fn dashboard_activity_account_recent_excludes_archived_rows_for_all_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    seed_dashboard_account(&state, 42, "Archive Overlap").await;

    let base_utc = Utc::now() - ChronoDuration::minutes(30);
    let live_overlap_at = format_naive(base_utc.with_timezone(&Shanghai).naive_local());
    let live_selected_at = format_naive(
        (base_utc + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_newer_at = format_naive(
        (base_utc + ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (id, invoke_id, occurred_at, total_tokens) in [
        (
            94_001_i64,
            "dashboard-account-recent-live-overlap",
            live_overlap_at.as_str(),
            10_i64,
        ),
        (
            94_002_i64,
            "dashboard-account-recent-live-selected",
            live_selected_at.as_str(),
            20_i64,
        ),
    ] {
        seed_dashboard_live_invocation(&state, id, invoke_id, occurred_at, total_tokens).await;
    }

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-account-recent-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 94_001_i64,
            invoke_id: "dashboard-account-recent-live-overlap",
            occurred_at: archive_newer_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.01_f64,
            ttfb_ms: None,
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-account-recent-live-overlap","upstreamAccountId":42}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "1d".to_string(),
            recent_limit: Some(1),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("fetch dashboard activity with account recent archive/live overlap");
    let account = response
        .accounts
        .expect("dashboard activity should include accounts")
        .into_iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("dashboard activity should include overlap account");
    assert_eq!(account.recent_invocations.len(), 1);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "dashboard-account-recent-live-selected"
    );
}

#[tokio::test]
pub(crate) async fn dashboard_activity_progressive_summary_keeps_archived_account_aggregates() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    let archived_hour = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(3))
    .with_minute(0)
    .and_then(|value| value.with_second(0))
    .expect("valid archived hour");
    let success_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let failure_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("archived failure time"),
    );
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-aggregate",
        &[
            SeedInvocationArchiveBatchRow {
                id: 91_001,
                invoke_id: "dashboard-progressive-archive-success",
                occurred_at: success_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10,
                cost: 0.10,
                ttfb_ms: Some(100.0),
                payload: Some(r#"{"upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 91_002,
                invoke_id: "dashboard-progressive-archive-failure",
                occurred_at: failure_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20,
                cost: 0.20,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1),
            },
        ],
    )
    .await;

    let response = fetch_archived_dashboard_activity(state.clone(), Some(false)).await;
    assert_archived_dashboard_account(response, &failure_at);

    let compatible_response = fetch_archived_dashboard_activity(state, None).await;
    assert_compatible_archived_dashboard_account(compatible_response);
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_does_not_populate_dashboard_snapshot_cache() {
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
    .bind("Cache Isolation")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert cache isolation account");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(97_001_i64)
    .bind("upstream-cache-isolation")
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(123_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-upstream-cache-isolation",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert cache isolation invocation");

    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        assert!(cache.entries.is_empty());
        assert!(cache.in_flight.is_empty());
    }

    let Json(response) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity without dashboard cache coupling");
    assert_eq!(response.accounts.len(), 1);
    assert_eq!(response.accounts[0].upstream_account_id, 42_i64);

    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        assert!(cache.entries.is_empty());
        assert!(cache.in_flight.is_empty());
    }
}

#[tokio::test]
pub(crate) async fn dashboard_activity_progressive_summary_keeps_archive_created_at_across_batches()
{
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    let archived_hour = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(3))
    .with_minute(0)
    .and_then(|value| value.with_second(0))
    .expect("valid archived hour");
    let first_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived time"),
    );
    let second_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::hours(5))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("second archived time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-created-at-early",
        &[SeedInvocationArchiveBatchRow {
            id: 92_001,
            invoke_id: "dashboard-progressive-archive-created-at-early",
            occurred_at: first_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10,
            cost: 0.10,
            ttfb_ms: Some(100.0),
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-progressive-archive-created-at","upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-created-at-late",
        &[SeedInvocationArchiveBatchRow {
            id: 92_002,
            invoke_id: "dashboard-progressive-archive-created-at-late",
            occurred_at: second_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20,
            cost: 0.20,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-progressive-archive-created-at","upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch progressive dashboard activity with cross-batch created_at");
    let account = response
        .accounts
        .expect("accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("archived account summary");
    assert_eq!(
        account.latest_conversation_created_at.as_deref(),
        Some(first_at.as_str())
    );
    assert_eq!(
        account.last_invocation_at.as_deref(),
        Some(second_at.as_str())
    );
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_uses_rollup_created_at_when_working_set_row_is_missing()
 {
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
    .expect("insert upstream account");

    let now = Utc::now();
    let rollup_bucket_start = now - ChronoDuration::seconds(3);
    let expected_created_at =
        format_naive(rollup_bucket_start.with_timezone(&Shanghai).naive_local());
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        rollup_bucket_start,
        "pck-upstream-history-only",
        1,
    )
    .await;

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
    .bind(9_100_i64)
    .bind("upstream-history-only")
    .bind(format_naive(now.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind("completed")
    .bind(120_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-upstream-history-only",
            "upstreamAccountId": 42,
            "upstreamAccountName": "Pool Alpha"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert upstream account invocation");

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity");

    assert_eq!(activity.accounts.len(), 1);
    assert_eq!(
        activity.accounts[0]
            .latest_conversation_created_at
            .as_deref(),
        Some(expected_created_at.as_str())
    );
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_all_scope_uses_actual_created_at_for_mixed_source_conversations()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_dashboard_account(&state, 42, "Pool Alpha").await;

    let now = Utc::now();
    let all_created_bucket_start = now - ChronoDuration::seconds(4);
    let proxy_created_bucket_start = now - ChronoDuration::seconds(3);
    let all_created_at = format_naive(
        all_created_bucket_start
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let proxy_created_at = format_naive(
        proxy_created_bucket_start
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let last_activity_at = format_naive(now.with_timezone(&Shanghai).naive_local());

    insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
        &state.pool,
        all_created_bucket_start,
        "pck-upstream-mixed",
        SOURCE_XY,
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
        &state.pool,
        proxy_created_bucket_start,
        "pck-upstream-mixed",
        SOURCE_PROXY,
        1,
    )
    .await;
    insert_prompt_cache_working_set_live_row(
        &state.pool,
        "pck-upstream-mixed",
        &all_created_at,
        &last_activity_at,
        Some(&proxy_created_at),
        Some(&last_activity_at),
        true,
    )
    .await;

    for (id, invoke_id, source, occurred_at) in [
        (
            9_200_i64,
            "upstream-mixed-all",
            SOURCE_XY,
            format_naive(
                (now - ChronoDuration::seconds(1))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
        ),
        (
            9_201_i64,
            "upstream-mixed-proxy",
            SOURCE_PROXY,
            format_naive(now.with_timezone(&Shanghai).naive_local()),
        ),
    ] {
        seed_upstream_activity_invocation(&state, id, invoke_id, &occurred_at, source).await;
    }

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch mixed-source upstream account activity");

    assert_eq!(activity.accounts.len(), 1);
    assert_eq!(
        activity.accounts[0]
            .latest_conversation_created_at
            .as_deref(),
        Some(all_created_at.as_str())
    );
}

#[tokio::test]
pub(crate) async fn upstream_account_activity_uses_pool_attempt_account_for_running_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_dashboard_account(&state, 77, "Pool Fallback").await;
    seed_pool_running_activity_rows(&state, &occurred_at).await;
    state
        .proxy_runtime_invocations
        .upsert(pool_running_runtime_invocation(occurred_at));

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream activity with running fallback");

    assert_eq!(activity.accounts.len(), 1);
    let account = activity
        .accounts
        .first()
        .expect("fallback activity account");
    assert_eq!(account.upstream_account_id, 77);
    assert_eq!(account.display_name, "Pool Fallback");
    assert_eq!(account.request_count, 1);
    assert_eq!(account.failure_count, 1);
    assert_eq!(account.total_tokens, 0);
    assert_f64_close(account.total_cost, 0.0);
    assert_eq!(account.tokens_per_minute, Some(0.0));
    assert_eq!(account.spend_rate, Some(0.0));
    assert_eq!(account.in_progress_invocation_count, Some(1));
    let account_phase_counts = account
        .in_progress_phase_counts
        .expect("account should include live phase counts");
    assert_eq!(account_phase_counts.queued, 0);
    assert_eq!(account_phase_counts.requesting, 0);
    assert_eq!(account_phase_counts.responding, 1);
    assert_eq!(account.retry_invocation_count, Some(1));
    let recent_invocation = account
        .recent_invocations
        .first()
        .expect("fallback recent invocation");
    assert_eq!(
        recent_invocation.invoke_id,
        "pool-running-selected-before-payload-update"
    );
    assert_eq!(recent_invocation.live_phase.as_deref(), Some("responding"));

    let mut runtime_terminal = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == "pool-running-selected-before-payload-update")
        .expect("running fallback runtime record");
    runtime_terminal.status = Some("success".to_string());
    runtime_terminal.live_phase = None;
    state
        .proxy_runtime_invocations
        .upsert_terminal(runtime_terminal);

    let Json(terminal_activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream activity with terminal fallback");
    let terminal_account = terminal_activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 77)
        .expect("terminal fallback activity account");
    let terminal_recent = terminal_account
        .recent_invocations
        .iter()
        .filter(|row| row.invoke_id == "pool-running-selected-before-payload-update")
        .collect::<Vec<_>>();
    assert_eq!(terminal_recent.len(), 1);
    assert_eq!(terminal_recent[0].status, "success");

    assert_pool_running_account_summary(state).await;
}

use super::*;
