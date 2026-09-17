pub(crate) static APP_CONFIG_ENV_LOCK: once_cell::sync::Lazy<AsyncMutex<()>> =
    once_cell::sync::Lazy::new(|| AsyncMutex::new(()));
pub(crate) const LARGE_STACK_ASYNC_TEST_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct CurrentDirGuard {
    original: PathBuf,
}

pub(crate) struct EnvVarGuard {
    previous: Vec<(String, Option<OsString>)>,
}

impl CurrentDirGuard {
    pub(crate) fn change_to(path: &Path) -> Self {
        let original = env::current_dir().expect("read current dir");
        env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl EnvVarGuard {
    pub(crate) fn set(cases: &[(&str, Option<&str>)]) -> Self {
        let previous = cases
            .iter()
            .map(|(name, _)| ((*name).to_string(), env::var_os(name)))
            .collect::<Vec<_>>();

        for (name, value) in cases {
            match value {
                Some(value) => unsafe { env::set_var(name, value) },
                None => unsafe { env::remove_var(name) },
            }
        }

        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => unsafe { env::set_var(&name, value) },
                None => unsafe { env::remove_var(&name) },
            }
        }
    }
}

pub(crate) fn run_async_test_with_large_stack<F, Fut>(name: &'static str, test_fn: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(LARGE_STACK_ASYNC_TEST_BYTES)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build current-thread runtime");
            runtime.block_on(test_fn());
        })
        .expect("spawn large-stack async test thread")
        .join()
        .expect("join large-stack async test thread");
}

#[tokio::test]
pub(crate) async fn acquire_proxy_request_concurrency_permit_tracks_multiple_in_flight_requests() {
    let config = test_config();
    let state = test_state_from_config(config, true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");

    let permit =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 1002, &Method::POST, &uri).await;
    let permit2 =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 1003, &Method::POST, &uri).await;
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        2
    );

    drop(permit);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );

    drop(permit2);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
}

#[tokio::test]
pub(crate) async fn acquire_proxy_request_concurrency_permit_tracks_100_in_flight_without_local_rejection()
 {
    let config = test_config();
    let state = test_state_from_config(config, true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");

    let mut permits = Vec::new();
    for proxy_request_id in 10_000..10_100 {
        permits.push(
            acquire_proxy_request_concurrency_permit(
                state.as_ref(),
                proxy_request_id,
                &Method::POST,
                &uri,
            )
            .await,
        );
    }

    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        100,
        "deprecated proxy concurrency config must not cap observable /v1 in-flight requests"
    );

    drop(permits);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
}

#[tokio::test]
pub(crate) async fn proxy_openai_v1_invalid_pool_key_bypasses_admission_backpressure() {
    let config = test_config();
    let state = test_state_from_config(config, true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");
    let mut rx = state.broadcaster.subscribe();

    let permit =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 2001, &Method::POST, &uri).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer invalid-pool-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(Bytes::from_static(br#"{"model":"gpt-5","input":"hello"}"#)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read invalid pool key body"),
    )
    .expect("decode invalid pool key payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE)
    );
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1,
        "invalid pool keys should not consume an admission slot while another request is in flight"
    );

    let mut running_record: Option<ApiInvocation> = None;
    let mut terminal_record: Option<ApiInvocation> = None;
    for _ in 0..4 {
        let Ok(Ok(BroadcastPayload::Records { records })) =
            tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
        else {
            continue;
        };
        for record in records {
            match record.status.as_deref() {
                Some(INVOCATION_STATUS_RUNNING) => running_record = Some(record),
                Some("failed") => terminal_record = Some(record),
                _ => {}
            }
        }
        if running_record.is_some() && terminal_record.is_some() {
            break;
        }
    }
    let running_record = running_record
        .expect("tracked proxy request should emit a running shell before route validation");
    let terminal_record =
        terminal_record.expect("route validation failure should terminalize the running shell");
    assert_eq!(terminal_record.invoke_id, running_record.invoke_id);
    assert_eq!(terminal_record.occurred_at, running_record.occurred_at);
    assert_eq!(
        terminal_record.failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ROUTING_BLOCKED)
    );

    drop(permit);
}

#[tokio::test]
pub(crate) async fn proxy_openai_v1_missing_pool_key_terminalizes_admitted_running_shell() {
    let state = test_state_from_config(test_config(), true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");
    let mut rx = state.broadcaster.subscribe();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Body::from(Bytes::from_static(br#"{"model":"gpt-5","input":"hello"}"#)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0,
        "missing pool keys should release observable in-flight tracking after response"
    );

    let mut running_record: Option<ApiInvocation> = None;
    let mut terminal_record: Option<ApiInvocation> = None;
    for _ in 0..4 {
        let Ok(Ok(BroadcastPayload::Records { records })) =
            tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
        else {
            continue;
        };
        for record in records {
            match record.status.as_deref() {
                Some(INVOCATION_STATUS_RUNNING) => running_record = Some(record),
                Some("failed") => terminal_record = Some(record),
                _ => {}
            }
        }
        if running_record.is_some() && terminal_record.is_some() {
            break;
        }
    }

    let running_record = running_record
        .expect("tracked proxy request should emit a running shell before missing-key validation");
    let terminal_record =
        terminal_record.expect("missing pool key should terminalize the running shell");
    assert_eq!(terminal_record.invoke_id, running_record.invoke_id);
    assert_eq!(terminal_record.occurred_at, running_record.occurred_at);
    assert_eq!(
        terminal_record.failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ROUTING_BLOCKED)
    );
}

#[tokio::test]
pub(crate) async fn proxy_openai_v1_models_rejects_non_pool_bearer_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-direct-upstream"),
        )]),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read non-pool models error body"),
    )
    .expect("decode non-pool models error payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE)
    );
}

#[test]
pub(crate) fn proxy_openai_v1_via_pool_keeps_in_flight_tracking_until_downstream_stream_finishes() {
    run_async_test_with_large_stack(
        "proxy_openai_v1_via_pool_keeps_in_flight_tracking_until_downstream_stream_finishes",
        assert_streaming_via_pool_lifecycle,
    );
}

async fn assert_streaming_via_pool_lifecycle() {
    let (addr, upstream_handle) = spawn_streaming_upstream().await;
    let (state, response) = request_streaming_via_pool(addr).await;

    assert_streaming_request_is_active(state.as_ref(), &response);
    assert_streaming_request_completes(state.as_ref(), response).await;
    upstream_handle.abort();
}

async fn spawn_streaming_upstream() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async move {
            let stream = futures_util::stream::once(async {
                Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"phase":"streaming""#))
            })
            .chain(futures_util::stream::once(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok::<Bytes, Infallible>(Bytes::from_static(br#","done":true}"#))
            }));
            Response::builder()
                .status(StatusCode::OK)
                .header(http_header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(stream))
                .expect("build streaming response")
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind streaming upstream");
    let addr = listener.local_addr().expect("streaming upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("streaming upstream should run");
    });
    (addr, handle)
}

async fn request_streaming_via_pool(addr: std::net::SocketAddr) -> (Arc<AppState>, Response) {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{addr}")).expect("valid streaming upstream base url");
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-stream-slot-key").await;
    insert_test_pool_api_key_account(&state, "Streaming Slot", "route-stream-slot").await;
    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        1003,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-stream-slot-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(Bytes::from_static(br#"{"model":"gpt-5","input":"hi"}"#)),
        runtime_timeouts,
        None,
    )
    .await
    .expect("streaming via-pool request should succeed");
    (state, response)
}

fn assert_streaming_request_is_active(state: &AppState, response: &Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == "pool-via-1003"),
        "via-pool runtime snapshot should remain visible while the response is streaming"
    );
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1,
        "proxy request should remain in-flight until downstream streaming finishes"
    );
}

async fn assert_streaming_request_completes(state: &AppState, response: Response) {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read streaming via-pool response");
    assert_eq!(
        body,
        Bytes::from_static(br#"{"phase":"streaming","done":true}"#)
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let removed = state
                .proxy_runtime_invocations
                .snapshot()
                .iter()
                .all(|record| record.invoke_id != "pool-via-1003");
            if removed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completed via-pool runtime snapshot should become terminal");
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0,
        "in-flight tracking should release after downstream streaming completes"
    );
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .all(|record| record.invoke_id != "pool-via-1003"),
        "completed via-pool requests must remove synthetic runtime snapshots"
    );
}

#[test]
pub(crate) fn named_range_today_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 10, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("today", now, tz).expect("today bounds");
    // Midnight before DST jump is still PST (-08:00).
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 10, 8, 0, 0).single().unwrap()
    );
    // Next midnight is PDT (-07:00) after the DST jump.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
pub(crate) fn named_range_yesterday_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 11, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("yesterday", now, tz).expect("yesterday bounds");
    // Yesterday start: Sun 2024-03-10 00:00 PST => 08:00Z.
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 10, 8, 0, 0).single().unwrap()
    );
    // Yesterday end: Mon 2024-03-11 00:00 PDT => 07:00Z after the DST jump.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
pub(crate) fn named_range_this_week_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 6, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("thisWeek", now, tz).expect("thisWeek bounds");
    // Start of week: Mon 2024-03-04 00:00 PST => 08:00Z.
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 4, 8, 0, 0).single().unwrap()
    );
    // End of week: Mon 2024-03-11 00:00 PDT => 07:00Z.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
pub(crate) fn next_reporting_bucket_epoch_respects_dst_for_multi_hour_buckets() {
    let tz = chrono_tz::America::New_York;
    let timestamp = Utc
        .with_ymd_and_hms(2024, 3, 10, 9, 30, 0)
        .single()
        .expect("valid dt");

    let bucket_start_epoch =
        align_reporting_bucket_epoch(timestamp.timestamp(), 6 * 3_600, tz).expect("align bucket");
    let bucket_end_epoch =
        next_reporting_bucket_epoch(bucket_start_epoch, 6 * 3_600, tz).expect("next bucket");

    let bucket_start_local = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid bucket start")
        .with_timezone(&tz);
    let bucket_end_local = Utc
        .timestamp_opt(bucket_end_epoch, 0)
        .single()
        .expect("valid bucket end")
        .with_timezone(&tz);

    assert_eq!(bucket_start_local.hour(), 0);
    assert_eq!(bucket_start_local.minute(), 0);
    assert_eq!(bucket_end_local.hour(), 6);
    assert_eq!(bucket_end_local.minute(), 0);
}

#[test]
pub(crate) fn parse_summary_window_accepts_yesterday_calendar_window() {
    let window = parse_summary_window(
        &SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: None,
            upstream_account_id: None,
        },
        50,
    )
    .expect("parse yesterday summary window");

    match window {
        SummaryWindow::Calendar(value) => assert_eq!(value, "yesterday"),
        other => panic!("expected calendar window, got {other:?}"),
    }
}

#[test]
pub(crate) fn parse_summary_window_accepts_previous_seven_full_days_window() {
    let window = parse_summary_window(
        &SummaryQuery {
            window: Some("previous7d".to_string()),
            limit: None,
            time_zone: None,
            upstream_account_id: None,
        },
        50,
    )
    .expect("parse previous seven full days summary window");

    match window {
        SummaryWindow::PreviousFullDays(day_count) => assert_eq!(day_count, 7),
        other => panic!("expected previous full days window, got {other:?}"),
    }
}

#[test]
pub(crate) fn previous_full_days_range_ends_at_current_local_midnight() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2026, 4, 30, 19, 45, 0)
        .single()
        .expect("valid now");
    let (start, end) =
        previous_full_days_range_bounds(7, now, tz).expect("previous full days bounds");

    assert_eq!(
        start.with_timezone(&tz).date_naive(),
        chrono::NaiveDate::from_ymd_opt(2026, 4, 23).expect("valid date")
    );
    assert_eq!(
        end.with_timezone(&tz).date_naive(),
        chrono::NaiveDate::from_ymd_opt(2026, 4, 30).expect("valid date")
    );
    assert_eq!(end.with_timezone(&tz).hour(), 0);
    assert_eq!(end.with_timezone(&tz).minute(), 0);
}

#[test]
pub(crate) fn exclusive_epoch_upper_bound_preserves_fractional_current_second() {
    let exact_second = Utc
        .with_ymd_and_hms(2026, 4, 11, 0, 0, 0)
        .single()
        .expect("valid exact second");
    let fractional_second = exact_second + ChronoDuration::nanoseconds(1);

    assert_eq!(
        exclusive_epoch_upper_bound(exact_second),
        exact_second.timestamp()
    );
    assert_eq!(
        exclusive_epoch_upper_bound(fractional_second),
        exact_second.timestamp() + 1
    );
}

#[test]
pub(crate) fn local_naive_to_utc_does_not_fall_back_to_and_utc_on_dst_gap() {
    let tz = chrono_tz::America::Los_Angeles;
    let naive = NaiveDate::from_ymd_opt(2024, 3, 10)
        .unwrap()
        .and_hms_opt(2, 30, 0)
        .unwrap();

    assert!(matches!(tz.from_local_datetime(&naive), LocalResult::None));
    let resolved = local_naive_to_utc(naive, tz);
    assert_ne!(resolved, naive.and_utc());

    let local = resolved.with_timezone(&tz);
    assert_eq!(local.hour(), 3);
    assert_eq!(local.minute(), 0);
    assert_eq!(local.second(), 0);
}

#[test]
pub(crate) fn resolve_invocation_proxy_display_name_prefers_selected_forward_proxy() {
    let selected_proxy = SelectedForwardProxy {
        key: "proxy-a".to_string(),
        source: "manual".to_string(),
        display_name: "Tokyo-Edge-1".to_string(),
        endpoint_url: Some(Url::parse("http://127.0.0.1:7890").expect("valid proxy url")),
        endpoint_url_raw: Some("http://127.0.0.1:7890".to_string()),
        egress_ip: None,
    };

    assert_eq!(
        resolve_invocation_proxy_display_name(Some(&selected_proxy)).as_deref(),
        Some("Tokyo-Edge-1")
    );
}

#[test]
pub(crate) fn resolve_invocation_proxy_display_name_returns_none_without_selected_forward_proxy() {
    assert_eq!(resolve_invocation_proxy_display_name(None).as_deref(), None);
}

#[test]
pub(crate) fn normalize_single_proxy_url_supports_scheme_less_host_port() {
    assert_eq!(
        normalize_single_proxy_url("127.0.0.1:7890"),
        Some("http://127.0.0.1:7890".to_string())
    );
    assert_eq!(
        normalize_single_proxy_url("socks5://127.0.0.1:1080"),
        Some("socks5://127.0.0.1:1080".to_string())
    );
    assert_eq!(normalize_single_proxy_url("vmess://example"), None);
}

#[test]
pub(crate) fn normalize_single_proxy_url_supports_xray_share_links() {
    let vmess_payload = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "vmess-node"
    }))
    .expect("serialize vmess payload");
    let vmess_link = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload)
    );
    assert!(normalize_single_proxy_url(&vmess_link).is_some());
    assert!(normalize_single_proxy_url("vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless").is_some());
    assert!(normalize_single_proxy_url("trojan://password@trojan.example.com:443?type=ws&path=%2Fws&host=cdn.trojan.example.com").is_some());
    assert!(
        normalize_single_proxy_url("ss://YWVzLTI1Ni1nY206cGFzc0AxMjcuMC4wLjE6ODM4OA==").is_some()
    );
}

#[test]
pub(crate) fn stable_proxy_keys_ignore_share_link_display_name_only_changes() {
    assert_vmess_key_ignores_display_name();
    assert_vless_key_ignores_display_name();
    assert_trojan_key_ignores_display_name();
    assert_shadowsocks_and_bound_keys_ignore_display_name();
}

fn assert_vmess_key_ignores_display_name() {
    let vmess_link = |display_name| {
        let payload = serde_json::to_string(&json!({
            "add": "vmess.example.com",
            "port": "443",
            "id": "11111111-1111-1111-1111-111111111111",
            "aid": "0",
            "net": "ws",
            "host": "cdn.vmess.example.com",
            "path": "/ws",
            "tls": "tls",
            "ps": display_name
        }))
        .expect("serialize vmess payload");
        format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(payload)
        )
    };
    assert_eq!(
        normalize_single_proxy_key(&vmess_link("东京节点")),
        normalize_single_proxy_key(&vmess_link("Tokyo Edge"))
    );
}

fn assert_vless_key_ignores_display_name() {
    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=ws&host=cdn.vless.example.com&path=%2Fws&security=tls#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=none&type=tcp#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com&sni=edge.vless.example.com&fingerprint=chrome#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&net=ws&path=%2Fws&host=cdn.vless.example.com&serverName=edge.vless.example.com&fp=chrome#Tokyo%20Edge"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=tcp&headerType=http&host=cdn-a.example.com#节点A"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=tcp&headerType=http&host=cdn-b.example.com#节点B"
        ),
    );
}

fn assert_trojan_key_ignores_display_name() {
    assert_eq!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=ws&path=%2Fws&host=cdn.trojan.example.com#东京节点"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?host=cdn.trojan.example.com&path=%2Fws&type=ws#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.trojan.example.com&sni=edge.trojan.example.com&fingerprint=chrome#东京节点"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&net=ws&path=%2Fws&host=cdn.trojan.example.com&serverName=edge.trojan.example.com&fp=chrome#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key("trojan://password@trojan.example.com:443#东京节点"),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&type=tcp#Tokyo%20Edge"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=kcp&seed=alpha#节点A"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=kcp&seed=beta#节点B"
        ),
    );
}

fn assert_shadowsocks_and_bound_keys_ignore_display_name() {
    let shadowsocks_url = |suffix| {
        format!(
            "{}{}{}{}",
            "ss://2022-blake3-aes-128-gcm:", "%2B%2F%3D", "@127.0.0.1:8388", suffix
        )
    };
    assert_eq!(
        normalize_single_proxy_key(&shadowsocks_url("#东京节点")),
        normalize_single_proxy_key(&shadowsocks_url("#Tokyo%20Edge")),
    );

    let stable_http_key =
        normalize_single_proxy_key("http://127.0.0.1:7890").expect("stable http proxy key");
    assert_eq!(
        normalize_bound_proxy_key(&stable_http_key),
        Some(stable_http_key.clone())
    );
    assert_eq!(
        normalize_bound_proxy_key(FORWARD_PROXY_DIRECT_KEY),
        Some(FORWARD_PROXY_DIRECT_KEY.to_string())
    );
}

#[test]
pub(crate) fn stable_proxy_keys_change_when_proxy_identity_changes() {
    let shadowsocks_url = |plugin| {
        format!(
            "{}{}{}{}{}",
            "ss://2022-blake3-aes-128-gcm:",
            "%2B%2F%3D",
            "@127.0.0.1:8388?plugin=",
            plugin,
            "#节点A"
        )
    };
    let vmess_payload_a = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "type": "none",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "节点A"
    }))
    .expect("serialize vmess payload a");
    let vmess_payload_b = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "type": "http",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "节点A"
    }))
    .expect("serialize vmess payload b");
    assert_ne!(
        normalize_single_proxy_key("http://127.0.0.1:7890"),
        normalize_single_proxy_key("http://127.0.0.1:7891"),
    );
    assert_ne!(
        normalize_single_proxy_key(&format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload_a)
        )),
        normalize_single_proxy_key(&format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload_b)
        )),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn-a.example.com#节点A"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn-b.example.com#节点A"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(&shadowsocks_url("v2ray-plugin%3Btls")),
        normalize_single_proxy_key(&shadowsocks_url("obfs-local%3Bobfs%3Dhttp")),
    );
}

#[test]
pub(crate) fn proxy_display_name_from_url_decodes_non_ascii_fragment() {
    let url = Url::parse(
        "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls#%E4%B8%9C%E4%BA%AC%E8%8A%82%E7%82%B9",
    )
    .expect("valid vless share link");
    assert_eq!(
        proxy_display_name_from_url(&url).as_deref(),
        Some("东京节点")
    );
}

#[tokio::test]
pub(crate) async fn forward_proxy_binding_nodes_restore_display_name_for_missing_bound_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_key = "fpn_deadbeefcafebabe".to_string();
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: proxy_key.clone(),
            display_name: "东京专线 A".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#%E4%B8%9C%E4%BA%AC%E4%B8%93%E7%BA%BF%20A"
                    .to_string(),
            ),
            weight: 0.8,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist forward proxy metadata history");

    let nodes = build_forward_proxy_binding_nodes_response(
        state.as_ref(),
        std::slice::from_ref(&proxy_key),
    )
    .await
    .expect("build binding nodes response");

    assert!(
        nodes
            .iter()
            .any(|node| node.key == FORWARD_PROXY_DIRECT_KEY),
        "direct binding candidate should still be present",
    );
    let missing_node = nodes
        .iter()
        .find(|node| node.key == proxy_key)
        .expect("missing bound node should be present");
    assert_eq!(missing_node.display_name, "东京专线 A");
    assert_eq!(missing_node.source, "missing");
    assert_eq!(missing_node.protocol_label, "UNKNOWN");
    assert!(!missing_node.selectable);
    assert!(!missing_node.penalized);
    assert!(missing_node.last24h.is_empty());
}

use super::*;
