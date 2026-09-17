struct DashboardInvocationSpec<'a> {
    id: i64,
    invoke_id: &'a str,
    occurred_at: &'a str,
    account_id: i64,
    account_name: &'a str,
    status: &'a str,
    total_tokens: i64,
    cost: f64,
    ttfb_ms: f64,
    total_ms: f64,
}

fn dashboard_runtime_invocation(spec: DashboardInvocationSpec<'_>) -> crate::api::ApiInvocation {
    let mut invocation = summary_projection_test_invocation();
    invocation.id = spec.id;
    invocation.invoke_id = spec.invoke_id.to_string();
    invocation.occurred_at = spec.occurred_at.to_string();
    invocation.created_at = spec.occurred_at.to_string();
    invocation.source = SOURCE_PROXY.to_string();
    invocation.proxy_display_name = None;
    invocation.model = Some("gpt-5".to_string());
    invocation.request_model = Some("gpt-5".to_string());
    invocation.response_model = Some("gpt-5".to_string());
    invocation.input_tokens = Some(0);
    invocation.output_tokens = Some(0);
    invocation.cache_input_tokens = Some(0);
    invocation.reasoning_tokens = Some(0);
    invocation.reasoning_effort = None;
    invocation.total_tokens = Some(spec.total_tokens);
    invocation.cost = Some(spec.cost);
    invocation.cache_write_tokens = Some(0);
    invocation.status = Some(spec.status.to_string());
    invocation.live_phase = (spec.status == "running").then(|| "requesting".to_string());
    invocation.downstream_status_code = None;
    invocation.failure_class = None;
    invocation.is_actionable = None;
    invocation.upstream_request_id = None;
    invocation.compaction_request_kind = None;
    invocation.compaction_response_kind = None;
    invocation.image_intent = None;
    invocation.requester_ip = None;
    invocation.prompt_cache_key = Some(format!("pck-{}", spec.invoke_id));
    invocation.route_mode = None;
    invocation.upstream_account_id = Some(spec.account_id);
    invocation.upstream_account_name = Some(spec.account_name.to_string());
    invocation.response_content_encoding = None;
    invocation.request_compression_algorithm = None;
    invocation.transport = None;
    invocation.pool_attempt_count = None;
    invocation.pool_distinct_account_count = None;
    invocation.pool_attempt_terminal_reason = None;
    invocation.requested_service_tier = None;
    invocation.service_tier = None;
    invocation.billing_service_tier = None;
    invocation.cost_estimated = None;
    invocation.price_version = None;
    invocation.t_total_ms = Some(spec.total_ms);
    invocation.t_req_read_ms = Some(0.0);
    invocation.t_req_parse_ms = Some(0.0);
    invocation.t_upstream_connect_ms = Some(0.0);
    invocation.t_upstream_ttfb_ms = Some(spec.ttfb_ms);
    invocation.t_upstream_stream_ms = None;
    invocation.t_resp_parse_ms = None;
    invocation.t_persist_ms = None;
    invocation
}

async fn seed_dashboard_account(state: &AppState, id: i64, display_name: &str) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, group_name, \
         plan_type, status, enabled, created_at, updated_at) \
         VALUES (?1, 'api_key_codex', 'codex', ?2, 'Primary', 'enterprise', 'active', 1, ?3, ?3)",
    )
    .bind(id)
    .bind(display_name)
    .bind(created_at)
    .execute(&state.pool)
    .await
    .expect("insert dashboard activity account");
}

async fn seed_dashboard_invocation(
    state: &AppState,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
    status: &str,
    account_id: i64,
) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) \
         VALUES (?1, ?2, ?3, ?4, ?5, 100, 0.01, ?6, '{}')",
    )
    .bind(id)
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(status)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&state.pool)
    .await
    .expect("insert dashboard activity invocation");
}

async fn fetch_today_dashboard(state: Arc<AppState>) -> DashboardActivityResponse {
    fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("fetch dashboard activity")
    .0
}

fn assert_live_overlay_response(response: &DashboardActivityResponse, live_occurred_at: &str) {
    let accounts = response
        .accounts
        .as_ref()
        .expect("cached accounts should be included");
    let range_end = parse_to_utc_datetime(&response.range_end).expect("cached range end parses");
    let live_utc = parse_to_utc_datetime(live_occurred_at).expect("live occurred_at parses");
    assert!(range_end >= live_utc);
    assert!(response.snapshot_id >= live_utc.timestamp_millis());
    let existing = accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("cached snapshot should include existing account");
    assert!(
        existing
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-existing-running")
    );
    let live_only = accounts
        .iter()
        .find(|account| account.account_key == "upstream:99")
        .expect("cached snapshot should include newly live-only account");
    assert_eq!(live_only.display_name, "Live Only During Cache");
    assert_eq!(live_only.request_count, 1);
    assert_eq!(live_only.in_progress_invocation_count, Some(1));
    assert_f64_close(
        live_only.tokens_per_minute.expect("tokens per minute"),
        321.0,
    );
    assert_f64_close(live_only.spend_rate.expect("spend rate"), 0.32);
    assert_f64_close(
        live_only
            .current_first_response_byte_total_avg_ms
            .expect("current first response byte"),
        80.0,
    );
    assert_f64_close(
        live_only
            .current_avg_total_ms
            .expect("current total latency"),
        640.0,
    );
    assert!(
        live_only
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-live-only-running")
    );
    let in_progress = accounts
        .iter()
        .map(|account| account.in_progress_invocation_count.unwrap_or(0))
        .sum::<i64>();
    assert_eq!(
        response.summary.stats.in_progress_conversation_count,
        Some(in_progress)
    );
    assert_f64_close(
        response.summary.tokens_per_minute.expect("summary tpm"),
        321.0,
    );
    assert_f64_close(
        response.summary.spend_rate.expect("summary spend rate"),
        0.32,
    );
}

async fn assert_single_today_cache_selection(state: &AppState) {
    let cache = state.dashboard_activity_snapshot_cache.lock().await;
    let selections = cache.entries.keys().collect::<Vec<_>>();
    assert_eq!(selections.len(), 1);
    let selection = selections[0];
    assert_eq!(selection.range, "today");
    assert!(!selection.range_anchor.is_empty());
    assert_eq!(selection.time_zone, "Asia/Shanghai");
    assert_eq!(selection.recent_limit, 2);
    assert!(selection.include_accounts);
    assert!(selection.include_recent);
    assert!(cache.in_flight.is_empty());
}

fn assert_terminal_overlay_response(response: DashboardActivityResponse) {
    let accounts = response
        .accounts
        .expect("cached terminal accounts should be included");
    let existing = accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("cached snapshot should include existing terminal account");
    assert!(
        existing
            .recent_invocations
            .iter()
            .any(|row| row.invoke_id == "dashboard-cache-terminal-existing-runtime")
    );
    let terminal_only = accounts
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

async fn dashboard_subscription_payload(state: Arc<AppState>) -> Value {
    let prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare dashboard activity subscription snapshot");
    extract_subscription_snapshot_payload(prepared)
}

async fn broadcast_dashboard_projection_slices(state: Arc<AppState>) {
    let current = state
        .proxy_runtime_invocations
        .capture_memory_snapshot()
        .expect("capture dashboard current projection slice");
    let network = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture dashboard network projection slice");
    let current_slice = crate::DashboardCurrentProjectionSlice::from(&current.snapshot);
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardCurrentSlice {
                slice: Box::new(current_slice),
            },
        )
        .await;
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardNetworkSlice {
                slice: Box::new(network.slice),
            },
        )
        .await;
}

fn assert_live_subscription_payload(payload: &Value) {
    let accounts = payload
        .get("accounts")
        .and_then(Value::as_array)
        .expect("subscription accounts");
    let live_only = accounts
        .iter()
        .find(|account| account.get("accountKey").and_then(Value::as_str) == Some("upstream:99"))
        .expect("live overlay account should be injected into cached topic snapshot");
    assert_eq!(
        live_only
            .get("inProgressInvocationCount")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        live_only.get("tokensPerMinute").and_then(Value::as_f64),
        Some(321.0)
    );
    assert_eq!(
        live_only.get("spendRate").and_then(Value::as_f64),
        Some(0.32)
    );
}

#[tokio::test]
pub(crate) async fn dashboard_activity_cached_snapshot_overlays_new_live_accounts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for (account_id, display_name) in [
        (42_i64, "Cached Existing"),
        (99_i64, "Live Only During Cache"),
    ] {
        seed_dashboard_account(&state, account_id, display_name).await;
    }

    let base_local = Utc::now().with_timezone(&Shanghai).naive_local();
    let initial_at = format_naive(
        base_local
            .checked_sub_signed(ChronoDuration::minutes(2))
            .expect("valid cached existing invocation time"),
    );
    seed_dashboard_invocation(
        &state,
        90_001,
        "dashboard-cache-existing-success",
        &initial_at,
        "success",
        42,
    )
    .await;

    let initial_response = fetch_today_dashboard(state.clone()).await;
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
    seed_dashboard_invocation(
        &state,
        90_004,
        "dashboard-cache-existing-running",
        &cached_existing_recent_at,
        "running",
        42,
    )
    .await;

    let live_only_occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_dashboard_invocation(
        &state,
        90_002,
        "dashboard-cache-live-only-running",
        &live_only_occurred_at,
        "running",
        99,
    )
    .await;
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(
            &dashboard_runtime_invocation(DashboardInvocationSpec {
                id: 90_003,
                invoke_id: "dashboard-cache-live-only-current-rate",
                occurred_at: &live_only_occurred_at,
                account_id: 99,
                account_name: "Live Only During Cache",
                status: "success",
                total_tokens: 321,
                cost: 0.32,
                ttfb_ms: 80.0,
                total_ms: 640.0,
            }),
            Utc::now(),
        );
    reconcile_dashboard_runtime_projection_once(state.as_ref())
        .await
        .expect("reconcile dashboard runtime projection after direct database fixture writes");

    let cached_response = fetch_today_dashboard(state.clone()).await;
    assert_live_overlay_response(&cached_response, &live_only_occurred_at);
    assert_single_today_cache_selection(&state).await;
}

#[tokio::test]
pub(crate) async fn dashboard_activity_cached_snapshot_refreshes_terminal_recent_without_live_account()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for (account_id, display_name) in [
        (42_i64, "Cached Terminal Existing"),
        (88_i64, "Cached Terminal Only"),
    ] {
        seed_dashboard_account(&state, account_id, display_name).await;
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
    seed_dashboard_invocation(
        &state,
        90_101,
        "dashboard-cache-terminal-existing-persisted",
        &existing_persisted_at,
        "success",
        42,
    )
    .await;

    let initial_response = fetch_today_dashboard(state.clone()).await;
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
    state
        .proxy_runtime_invocations
        .upsert_terminal(dashboard_runtime_invocation(DashboardInvocationSpec {
            id: 90_102,
            invoke_id: "dashboard-cache-terminal-existing-runtime",
            occurred_at: &terminal_existing_at,
            account_id: 42,
            account_name: "Cached Terminal Existing",
            status: "success",
            total_tokens: 0,
            cost: 0.0,
            ttfb_ms: 0.0,
            total_ms: 0.0,
        }));
    state
        .proxy_runtime_invocations
        .upsert_terminal(dashboard_runtime_invocation(DashboardInvocationSpec {
            id: 90_103,
            invoke_id: "dashboard-cache-terminal-only-runtime",
            occurred_at: &terminal_only_at,
            account_id: 88,
            account_name: "Cached Terminal Only",
            status: "success",
            total_tokens: 0,
            cost: 0.0,
            ttfb_ms: 0.0,
            total_ms: 0.0,
        }));

    assert_terminal_overlay_response(fetch_today_dashboard(state.clone()).await);
}

pub(crate) fn dashboard_activity_topic_descriptor() -> SubscriptionTopicDescriptor {
    SubscriptionTopicDescriptor {
        topic: "dashboard.activity.current".to_string(),
        params: BTreeMap::from([
            ("range".to_string(), "today".to_string()),
            ("timeZone".to_string(), "Asia/Shanghai".to_string()),
            ("recentLimit".to_string(), "2".to_string()),
            ("includeAccounts".to_string(), "true".to_string()),
            ("includeRecent".to_string(), "true".to_string()),
        ]),
    }
}

pub(crate) fn extract_subscription_snapshot_payload(
    prepared: PreparedSubscriptionConnection,
) -> Value {
    let mut initial = prepared.initial.into_iter();
    initial
        .next()
        .expect("subscription snapshot event")
        .frame
        .payload_value()
}

#[tokio::test]
pub(crate) async fn dashboard_activity_subscription_projection_slices_update_cached_frame_without_db_rebuild()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind dashboard network cache");

    let initial_payload = dashboard_subscription_payload(state.clone()).await;
    let _dashboard_lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    let initial_accounts = initial_payload
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(initial_accounts.iter().all(|account| {
        account.get("accountKey").and_then(Value::as_str) != Some("upstream:99")
    }));

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    state
        .proxy_runtime_invocations
        .upsert(dashboard_runtime_invocation(DashboardInvocationSpec {
            id: 93_001,
            invoke_id: "dashboard-subscription-live-running",
            occurred_at: &occurred_at,
            account_id: 99,
            account_name: "Live Overlay Account",
            status: "running",
            total_tokens: 0,
            cost: 0.0,
            ttfb_ms: 0.0,
            total_ms: 0.0,
        }));
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(
            &dashboard_runtime_invocation(DashboardInvocationSpec {
                id: 93_002,
                invoke_id: "dashboard-subscription-live-current",
                occurred_at: &occurred_at,
                account_id: 99,
                account_name: "Live Overlay Account",
                status: "success",
                total_tokens: 321,
                cost: 0.32,
                ttfb_ms: 80.0,
                total_ms: 640.0,
            }),
            Utc::now(),
        );

    broadcast_dashboard_projection_slices(state.clone()).await;
    let updated_payload = dashboard_subscription_payload(state.clone()).await;
    assert_live_subscription_payload(&updated_payload);
}

#[tokio::test]
pub(crate) async fn dashboard_activity_subscription_terminal_slice_updates_without_ttl_refresh() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let initial_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare initial dashboard activity subscription snapshot");
    let initial_payload = extract_subscription_snapshot_payload(initial_prepared);
    let _dashboard_lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    assert_eq!(
        initial_payload
            .get("summary")
            .and_then(Value::as_object)
            .and_then(|summary| summary.get("stats"))
            .and_then(Value::as_object)
            .and_then(|stats| stats.get("totalCount"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        0
    );

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_dashboard_invocation(
        &state,
        93_101,
        "dashboard-subscription-terminal-refresh",
        &occurred_at,
        "success",
        42,
    )
    .await;

    let terminal_record = dashboard_runtime_invocation(DashboardInvocationSpec {
        id: 93_101,
        invoke_id: "dashboard-subscription-terminal-refresh",
        occurred_at: &occurred_at,
        account_id: 42,
        account_name: "Deferred Refresh Account",
        status: "success",
        total_tokens: 100,
        cost: 0.01,
        ttfb_ms: 10.0,
        total_ms: 100.0,
    });
    let terminal_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal_record)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardTerminalSlice {
                slice: Box::new(DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![terminal_delta],
                }),
            },
        )
        .await;

    let immediate_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare dashboard activity topic after terminal slice materialization");
    let immediate_payload = extract_subscription_snapshot_payload(immediate_prepared);
    assert_eq!(
        immediate_payload
            .get("summary")
            .and_then(Value::as_object)
            .and_then(|summary| summary.get("stats"))
            .and_then(Value::as_object)
            .and_then(|stats| stats.get("totalCount"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        1
    );
}

use super::*;
