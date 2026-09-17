#[tokio::test]
pub(crate) async fn upstream_account_activity_ignores_stale_persisted_in_progress_rows_for_totals()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_stale_activity_account(&state.pool).await;
    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    seed_stale_running_rows(&state.pool, base_local).await;
    seed_recent_activity_rows(&state.pool, base_local).await;

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

async fn seed_stale_activity_account(pool: &SqlitePool) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query("INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at) VALUES (42, 'api_key_codex', 'codex', 'Pool Alpha', 'Primary', 'enterprise', 'active', 1, ?1, ?1)")
        .bind(created_at).execute(pool).await
        .expect("insert upstream account for stale persisted rows");
}

async fn seed_stale_running_rows(pool: &SqlitePool, base_local: NaiveDateTime) {
    for id in 0..20_i64 {
        let occurred_at =
            format_naive(base_local - ChronoDuration::days(6) + ChronoDuration::seconds(id));
        let payload = json!({
            "promptCacheKey": format!("pck-stale-upstream-running-{id}"),
            "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha",
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, ?3, ?4, 'running', 0, 0, ?5, '{}')")
            .bind(96_000 + id).bind(format!("stale-upstream-running-{id}"))
            .bind(occurred_at).bind(SOURCE_PROXY).bind(payload).execute(pool).await
            .expect("insert stale persisted running invocation");
    }
}

async fn seed_recent_activity_rows(pool: &SqlitePool, base_local: NaiveDateTime) {
    for (id, invoke_id, minutes_ago, total_tokens, cost) in [
        (97_001, "recent-upstream-success-old", 2, 120, 0.12),
        (97_002, "recent-upstream-success-new", 1, 240, 0.24),
    ] {
        let payload = json!({
            "promptCacheKey": format!("pck-{invoke_id}"),
            "upstreamAccountId": 42, "upstreamAccountName": "Pool Alpha",
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, ?3, ?4, 'success', ?5, ?6, ?7, '{}')")
            .bind(id).bind(invoke_id)
            .bind(format_naive(base_local - ChronoDuration::minutes(minutes_ago)))
            .bind(SOURCE_PROXY).bind(total_tokens).bind(cost).bind(payload)
            .execute(pool).await.expect("insert recent terminal invocation");
    }
}

#[tokio::test]
pub(crate) async fn dashboard_activity_summary_rates_and_in_progress_are_account_sum() {
    let scenario = DashboardActivityScenario::new().await;
    scenario.seed_database().await;
    scenario.install_runtime_overlay();
    let activity = scenario.load_snapshot().await;
    assert_dashboard_accounts(&activity);
    scenario.persist_terminal_and_assert_dedup().await;
    assert_dashboard_summary(&activity);
    assert_dashboard_performance(&activity);
    assert_alpha_performance(&activity);
    scenario.assert_summary_only(&activity).await;
    scenario.assert_full_response().await;
    scenario.assert_progressive_recent().await;
    scenario.assert_yesterday().await;
}

struct DashboardActivityScenario {
    state: Arc<AppState>,
    base_local: NaiveDateTime,
    latest_complete_minute_start: NaiveDateTime,
}

impl DashboardActivityScenario {
    async fn new() -> Self {
        let state =
            test_state_with_openai_base(Url::parse("https://api.openai.com/").unwrap()).await;
        seed_dashboard_accounts(&state.pool).await;
        let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
        let current_minute_start = base_local
            .with_second(0)
            .and_then(|value| value.with_nanosecond(0))
            .expect("valid current minute start");
        Self {
            state,
            base_local,
            latest_complete_minute_start: current_minute_start - ChronoDuration::minutes(1),
        }
    }

    async fn seed_database(&self) {
        self.seed_base_invocations().await;
        self.seed_performance_invocations().await;
        self.seed_historical_and_runtime_shell().await;
    }

    async fn seed_base_invocations(&self) {
        let rows = [
            (
                8_000,
                "dashboard-activity-unassigned-failed",
                None,
                "failed",
                500,
                0.05,
                None,
                5,
            ),
            (
                8_001,
                "dashboard-activity-alpha-success",
                Some(42),
                "success",
                1_000,
                0.10,
                Some(100.0),
                10,
            ),
            (
                8_002,
                "dashboard-activity-beta-running",
                Some(77),
                "running",
                2_000,
                0.20,
                Some(200.0),
                20,
            ),
            (
                8_003,
                "dashboard-activity-unassigned-running",
                None,
                "running",
                3_000,
                0.30,
                Some(300.0),
                30,
            ),
        ];
        for (id, invoke_id, account_id, status, tokens, cost, ttfb, offset) in rows {
            let prompt_cache_key = if invoke_id == "dashboard-activity-unassigned-failed" {
                "dashboard-activity-unassigned-running"
            } else {
                invoke_id
            };
            let mut payload = json!({ "promptCacheKey": format!("pck-{prompt_cache_key}") });
            if let Some(account_id) = account_id {
                payload["upstreamAccountId"] = json!(account_id);
            }
            sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}')")
                .bind(id).bind(invoke_id)
                .bind(format_naive(self.latest_complete_minute_start + ChronoDuration::seconds(offset)))
                .bind(SOURCE_PROXY).bind(status).bind(tokens).bind(cost).bind(ttfb)
                .bind(payload.to_string()).execute(&self.state.pool).await
                .expect("insert dashboard activity invocation");
        }
    }

    async fn seed_performance_invocations(&self) {
        let alpha_payload = json!({
            "promptCacheKey": "pck-dashboard-activity-alpha-success", "upstreamAccountId": 42,
            "responseModel": "gpt-5.6-performance", "reasoningEffort": "high",
        })
        .to_string();
        sqlx::query("UPDATE codex_invocations SET payload=?1, output_tokens=600, t_req_read_ms=10, t_req_parse_ms=20, t_upstream_connect_ms=30, t_upstream_stream_ms=2000, t_total_ms=2300 WHERE invoke_id='dashboard-activity-alpha-success'")
            .bind(alpha_payload).execute(&self.state.pool).await
            .expect("add alpha performance samples");
        let zero_payload = json!({
            "promptCacheKey": "pck-dashboard-activity-alpha-zero-cost", "upstreamAccountId": 42,
            "responseModel": "gpt-5.6-zero",
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost, t_upstream_ttfb_ms, t_upstream_stream_ms, t_total_ms, payload, raw_response) VALUES (8007, 'dashboard-activity-alpha-zero-cost', ?1, ?2, 'success', 400, 120, 0, 40, 400, 500, ?3, '{}')")
            .bind(format_naive(self.latest_complete_minute_start + ChronoDuration::seconds(40)))
            .bind(SOURCE_PROXY).bind(zero_payload).execute(&self.state.pool).await
            .expect("insert zero-cost dashboard activity invocation");
    }

    async fn seed_historical_and_runtime_shell(&self) {
        let yesterday_payload = json!({
            "promptCacheKey": "pck-dashboard-activity-yesterday-success", "upstreamAccountId": 42,
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (8004, 'dashboard-activity-yesterday-success', ?1, ?2, 'success', 4000, 0.40, ?3, '{}')")
            .bind(format_naive(self.base_local - ChronoDuration::days(1)))
            .bind(SOURCE_PROXY).bind(yesterday_payload).execute(&self.state.pool).await
            .expect("insert yesterday dashboard activity invocation");
        let runtime_payload = json!({
            "promptCacheKey": "pck-dashboard-activity-runtime-today-running",
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (8005, 'dashboard-activity-runtime-today-running', ?1, ?2, 'running', 0, 0, ?3, '{}')")
            .bind(format_naive(self.base_local)).bind(SOURCE_PROXY).bind(runtime_payload)
            .execute(&self.state.pool).await.expect("insert stale runtime db shell");
    }

    fn runtime_today(&self) -> crate::api::ApiInvocation {
        crate::api::ApiInvocation {
            id: 8_005,
            invoke_id: "dashboard-activity-runtime-today-running".to_string(),
            occurred_at: format_naive(self.base_local),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: Some("gpt-5".to_string()),
            request_model: Some("gpt-5".to_string()),
            response_model: Some("gpt-5".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            reasoning_tokens: Some(0),
            reasoning_effort: None,
            total_tokens: Some(9_999),
            cost: Some(9.99),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: Some(10),
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
            t_upstream_ttfb_ms: Some(999.0),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: format_naive(self.base_local),
        }
    }

    fn install_runtime_overlay(&self) {
        let runtime_today = self.runtime_today();
        self.state
            .proxy_runtime_invocations
            .upsert(runtime_today.clone());
        let mut before_today = runtime_today.clone();
        before_today.id = 8_006;
        before_today.invoke_id = "dashboard-activity-runtime-before-today-running".to_string();
        before_today.occurred_at = format_naive(self.base_local - ChronoDuration::days(2));
        before_today.total_tokens = Some(12_345);
        before_today.cost = Some(12.34);
        before_today.prompt_cache_key =
            Some("pck-dashboard-activity-runtime-before-today-running".to_string());
        self.state.proxy_runtime_invocations.upsert(before_today);
        self.install_terminal_overlay(
            runtime_today.clone(),
            42,
            "Pool Alpha",
            "dashboard-activity-runtime-terminal",
            1,
        );
        self.install_terminal_overlay(
            runtime_today,
            88,
            "Runtime Only",
            "dashboard-activity-runtime-terminal-only-account",
            2,
        );
    }

    fn install_terminal_overlay(
        &self,
        mut row: crate::api::ApiInvocation,
        account_id: i64,
        account_name: &str,
        invoke_id: &str,
        seconds_ago: i64,
    ) {
        row.id = 0;
        row.invoke_id = invoke_id.to_string();
        row.occurred_at = format_naive(self.base_local - ChronoDuration::seconds(seconds_ago));
        row.status = Some("success".to_string());
        row.upstream_account_id = Some(account_id);
        row.upstream_account_name = Some(account_name.to_string());
        row.prompt_cache_key = Some(format!("pck-{invoke_id}"));
        row.total_tokens = Some(0);
        row.cost = Some(0.0);
        self.state.proxy_runtime_invocations.upsert_terminal(row);
    }

    async fn load_snapshot(&self) -> DashboardActivitySnapshot {
        load_dashboard_activity_snapshot(
            self.state.as_ref(),
            "today",
            Shanghai,
            4,
            true,
            true,
            None,
        )
        .await
        .expect("load dashboard activity snapshot")
    }

    async fn persist_terminal_and_assert_dedup(&self) {
        let payload = json!({
            "promptCacheKey": "pck-dashboard-activity-runtime-terminal", "upstreamAccountId": 42,
        })
        .to_string();
        sqlx::query("INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES ('dashboard-activity-runtime-terminal', ?1, ?2, 'success', 0, 0, ?3, '{}')")
            .bind(format_naive(self.base_local - ChronoDuration::seconds(1)))
            .bind(SOURCE_PROXY).bind(payload).execute(&self.state.pool).await
            .expect("persist runtime terminal");
        let activity = self.load_snapshot().await;
        let alpha = activity
            .accounts()
            .iter()
            .find(|account| account.upstream_account_id == Some(42))
            .expect("alpha account");
        assert_eq!(
            alpha
                .recent_invocations
                .iter()
                .filter(|row| row.invoke_id == "dashboard-activity-runtime-terminal")
                .count(),
            1
        );
    }

    async fn dashboard_response(
        &self,
        range: &str,
        recent_limit: usize,
        include_accounts: bool,
        include_recent: Option<bool>,
    ) -> DashboardActivityResponse {
        fetch_dashboard_activity(
            State(self.state.clone()),
            Query(DashboardActivityQuery {
                range: range.to_string(),
                recent_limit: Some(recent_limit as i64),
                time_zone: Some("Asia/Shanghai".to_string()),
                include_accounts,
                include_recent,
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("fetch {range} dashboard activity: {error:?}"))
        .0
    }

    async fn assert_summary_only(&self, activity: &DashboardActivitySnapshot) {
        let scope = resolve_default_source_scope(&self.state.pool)
            .await
            .expect("source scope");
        let summary_only = load_dashboard_activity_summary_only_snapshot(
            self.state.as_ref(),
            "today",
            scope,
            activity.exact_range(),
        )
        .await
        .expect("load summary-only dashboard activity snapshot");
        let response = self
            .dashboard_response("today", 4, false, Some(false))
            .await;
        assert!(summary_only.accounts().is_empty());
        assert!(response.accounts.is_none());
        assert_eq!(
            summary_only.summary().stats.in_progress_conversation_count,
            activity.summary().stats.in_progress_conversation_count
        );
        assert_eq!(
            summary_only
                .summary()
                .stats
                .in_progress_retry_conversation_count,
            activity
                .summary()
                .stats
                .in_progress_retry_conversation_count
        );
        assert_f64_close(
            summary_only.summary().tokens_per_minute.unwrap(),
            activity.summary().tokens_per_minute.unwrap(),
        );
        assert_f64_close(
            response.summary.tokens_per_minute.unwrap(),
            activity.summary().tokens_per_minute.unwrap(),
        );
        assert_f64_close(
            summary_only.summary().spend_rate.unwrap(),
            activity.summary().spend_rate.unwrap(),
        );
        assert_f64_close(
            response.summary.spend_rate.unwrap(),
            activity.summary().spend_rate.unwrap(),
        );
        assert_f64_close(
            summary_only
                .summary()
                .current_first_response_byte_total_avg_ms
                .unwrap(),
            40.0,
        );
        assert_f64_close(summary_only.summary().current_avg_total_ms.unwrap(), 500.0);
    }

    async fn assert_full_response(&self) {
        let response = self.dashboard_response("today", 4, true, None).await;
        let accounts = response.accounts.as_ref().expect("full response accounts");
        assert!(response.live_revision > 0);
        assert_eq!(
            response.summary.stats.in_progress_conversation_count,
            Some(
                accounts
                    .iter()
                    .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                    .sum()
            )
        );
        assert_eq!(response.rate_window.window_minutes, 1);
        assert_eq!(response.rate_window.mode, "rolling_60s_live_mean");
        assert_eq!(response.rate_window.end, response.range_end);
        let start = DateTime::parse_from_rfc3339(&response.rate_window.start).unwrap();
        let end = DateTime::parse_from_rfc3339(&response.rate_window.end).unwrap();
        assert_eq!(end - start, ChronoDuration::seconds(60));
    }

    async fn assert_progressive_recent(&self) {
        let started = std::time::Instant::now();
        let progressive = self.dashboard_response("today", 2, true, Some(false)).await;
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let accounts = progressive.accounts.as_ref().expect("progressive accounts");
        assert!(
            accounts
                .iter()
                .all(|account| account.recent_invocations.is_empty())
        );
        assert!(
            accounts
                .iter()
                .all(|account| account.account_key != "upstream:88")
        );
        self.assert_recent_batch(&progressive).await;
        let wide = fetch_dashboard_activity_recent(
            State(self.state.clone()),
            Query(DashboardActivityRecentQuery {
                range_start: (Utc::now() - ChronoDuration::days(8)).to_rfc3339(),
                range_end: Utc::now().to_rfc3339(),
                snapshot_id: Utc::now().timestamp_millis(),
                recent_limit: Some(2),
            }),
        )
        .await;
        assert!(wide.is_err());
    }

    async fn assert_recent_batch(&self, progressive: &DashboardActivityResponse) {
        let Json(recent) = fetch_dashboard_activity_recent(
            State(self.state.clone()),
            Query(DashboardActivityRecentQuery {
                range_start: progressive.range_start.clone(),
                range_end: progressive.range_end.clone(),
                snapshot_id: progressive.snapshot_id,
                recent_limit: Some(2),
            }),
        )
        .await
        .expect("fetch dashboard activity recent batch");
        assert_eq!(recent.snapshot_id, progressive.snapshot_id);
        assert_eq!(recent.range_start, progressive.range_start);
        assert_eq!(recent.range_end, progressive.range_end);
        assert!(
            recent
                .accounts
                .iter()
                .all(|account| account.recent_invocations.len() <= 2)
        );
        assert!(
            recent
                .accounts
                .iter()
                .any(|account| account.account_key == "upstream:42")
        );
        assert!(recent_account_contains(
            &recent,
            "upstream:42",
            "dashboard-activity-runtime-terminal"
        ));
        assert!(recent_account_contains(
            &recent,
            "upstream:88",
            "dashboard-activity-runtime-terminal-only-account"
        ));
    }

    async fn assert_yesterday(&self) {
        let response = self.dashboard_response("yesterday", 4, true, None).await;
        assert_eq!(response.live_revision, 0);
        assert_eq!(response.summary.stats.in_progress_conversation_count, None);
        assert_eq!(
            response.summary.stats.in_progress_retry_conversation_count,
            None
        );
        assert_eq!(response.summary.stats.total_tokens, 4_000);
        let accounts = response.accounts.expect("yesterday accounts");
        assert_eq!(accounts.len(), 1);
        assert!(
            accounts
                .iter()
                .all(|account| account.in_progress_invocation_count.is_none())
        );
        assert!(
            accounts
                .iter()
                .all(|account| account.retry_invocation_count.is_none())
        );
    }
}

async fn seed_dashboard_accounts(pool: &SqlitePool) {
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [(42, "Pool Alpha"), (77, "Pool Beta")] {
        sqlx::query("INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at) VALUES (?1, 'api_key_codex', 'codex', ?2, 'Primary', 'enterprise', 'active', 1, ?3, ?3)")
            .bind(account_id).bind(display_name).bind(&created_at).execute(pool).await
            .expect("insert dashboard activity upstream account");
    }
}

fn assert_dashboard_accounts(activity: &DashboardActivitySnapshot) {
    let accounts = activity.accounts();
    assert_eq!(accounts.len(), 4);
    let unassigned = accounts
        .iter()
        .find(|account| account.is_unassigned)
        .expect("unassigned");
    assert_eq!(unassigned.retry_invocation_count, Some(1));
    let runtime = unassigned
        .recent_invocations
        .iter()
        .find(|row| row.invoke_id == "dashboard-activity-runtime-today-running")
        .expect("runtime row");
    assert_eq!(runtime.total_tokens, 9_999);
    assert_eq!(runtime.input_tokens, Some(10));
    assert_eq!(runtime.output_tokens, Some(20));
    assert_eq!(runtime.t_upstream_ttfb_ms, Some(999.0));
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
        .unwrap();
    assert_eq!(
        alpha.recent_invocations[0].invoke_id,
        "dashboard-activity-runtime-terminal"
    );
    let runtime_only = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(88))
        .unwrap();
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
}

fn assert_dashboard_summary(activity: &DashboardActivitySnapshot) {
    let accounts = activity.accounts();
    let summary = activity.summary();
    assert_eq!(
        summary.stats.total_count,
        accounts
            .iter()
            .map(|account| account.request_count)
            .sum::<i64>()
    );
    assert_eq!(
        summary.stats.total_tokens,
        accounts
            .iter()
            .map(|account| account.total_tokens)
            .sum::<i64>()
    );
    assert_eq!(summary.stats.total_tokens, 1_900);
    assert_f64_close(
        summary.stats.total_cost,
        accounts.iter().map(|account| account.total_cost).sum(),
    );
    assert_eq!(
        summary.stats.in_progress_conversation_count,
        Some(
            accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum()
        )
    );
    assert_eq!(summary.stats.in_progress_conversation_count, Some(4));
    assert_eq!(
        summary.stats.in_progress_retry_conversation_count,
        Some(
            accounts
                .iter()
                .map(|account| account.retry_invocation_count.unwrap_or(0))
                .sum()
        )
    );
    assert_f64_close(
        summary.tokens_per_minute.unwrap(),
        accounts
            .iter()
            .map(|account| account.tokens_per_minute.unwrap_or(0.0))
            .sum(),
    );
    assert_f64_close(
        summary.spend_rate.unwrap(),
        accounts
            .iter()
            .map(|account| account.spend_rate.unwrap_or(0.0))
            .sum(),
    );
    assert_f64_close(summary.tokens_per_minute.unwrap(), 0.0);
    assert_f64_close(summary.spend_rate.unwrap(), 0.0);
    assert_f64_close(
        summary.current_first_response_byte_total_avg_ms.unwrap(),
        40.0,
    );
    assert_f64_close(summary.current_avg_total_ms.unwrap(), 500.0);
}

fn assert_dashboard_performance(activity: &DashboardActivitySnapshot) {
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
    assert_f64_close(performance.total.streaming_response_rate.unwrap(), 300.0);
    assert_f64_close(performance.total.avg_response_ms.unwrap(), 1_200.0);
    assert_f64_close(
        performance.total.avg_first_response_byte_total_ms.unwrap(),
        100.0,
    );
    assert_f64_close(
        performance.total.wall_clock_usage_duration_ms.unwrap(),
        2_800.0,
    );
    assert_f64_close(
        performance.total.cumulative_usage_duration_ms.unwrap(),
        2_800.0,
    );
    assert_f64_close(performance.total.parallelism.unwrap(), 1.0);
    let cumulative = performance
        .models
        .iter()
        .map(|model| model.metrics.cumulative_usage_duration_ms.unwrap())
        .sum();
    assert_f64_close(cumulative, 2_800.0);
    assert_f64_close(
        performance.models[0]
            .metrics
            .wall_clock_usage_duration_ms
            .unwrap(),
        2_300.0,
    );
    assert_f64_close(
        performance.models[1]
            .metrics
            .wall_clock_usage_duration_ms
            .unwrap(),
        500.0,
    );
}

fn assert_alpha_performance(activity: &DashboardActivitySnapshot) {
    let alpha = activity
        .accounts()
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("alpha account");
    assert_f64_close(alpha.tokens_per_minute.unwrap(), 0.0);
    assert_f64_close(alpha.spend_rate.unwrap(), 0.0);
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
            .unwrap(),
        2_800.0,
    );
    assert_f64_close(
        alpha
            .model_performance
            .total
            .cumulative_usage_duration_ms
            .unwrap(),
        2_800.0,
    );
    let cumulative = alpha
        .model_performance
        .models
        .iter()
        .map(|model| model.metrics.cumulative_usage_duration_ms.unwrap())
        .sum();
    assert_f64_close(cumulative, 2_800.0);
    assert_f64_close(alpha.model_performance.total.parallelism.unwrap(), 1.0);
}

fn recent_account_contains(
    response: &DashboardActivityRecentResponse,
    account_key: &str,
    invoke_id: &str,
) -> bool {
    response.accounts.iter().any(|account| {
        account.account_key == account_key
            && account
                .recent_invocations
                .iter()
                .any(|row| row.invoke_id == invoke_id)
    })
}

use super::*;
