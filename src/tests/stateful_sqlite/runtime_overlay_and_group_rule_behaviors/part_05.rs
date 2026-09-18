pub(crate) async fn test_upstream_capture_target_compact_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> Response {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());
    let response_payload = json!({
        "id": "resp_compact_test",
        "object": "response.compaction",
        "output": [
            {
                "id": "cmp_001",
                "type": "compaction",
                "encrypted_content": "encrypted-summary"
            }
        ],
        "usage": {
            "input_tokens": 139,
            "input_tokens_details": {
                "cached_tokens": 11
            },
            "output_tokens": 438,
            "output_tokens_details": {
                "reasoning_tokens": 64
            },
            "total_tokens": 577
        },
        "received": payload,
    });

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-first-chunk"))
    {
        return chunked_json_response_with_delayed_first_chunk(
            response_payload,
            Duration::from_millis(250),
        );
    }
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-stream-end"))
    {
        return chunked_json_response_with_delayed_final_chunk(
            response_payload,
            Duration::from_millis(400),
        );
    }

    (StatusCode::OK, Json(response_payload)).into_response()
}

pub(crate) fn chunked_json_response_with_delayed_first_chunk(
    payload: Value,
    delay: Duration,
) -> Response {
    let response_bytes = serde_json::to_vec(&payload).expect("serialize streamed json response");
    let split_at = response_bytes
        .len()
        .saturating_div(2)
        .clamp(1, response_bytes.len() - 1);
    let first_chunk = Bytes::copy_from_slice(&response_bytes[..split_at]);
    let second_chunk = Bytes::copy_from_slice(&response_bytes[split_at..]);
    let chunked = stream::unfold(0u8, move |state| {
        let first_chunk = first_chunk.clone();
        let second_chunk = second_chunk.clone();
        async move {
            match state {
                0 => {
                    tokio::time::sleep(delay).await;
                    Some((Ok::<Bytes, Infallible>(first_chunk), 1))
                }
                1 => Some((Ok::<Bytes, Infallible>(second_chunk), 2)),
                _ => None,
            }
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from_stream(chunked),
    )
        .into_response()
}

pub(crate) fn chunked_json_response_with_delayed_final_chunk(
    payload: Value,
    delay: Duration,
) -> Response {
    let response_bytes = serde_json::to_vec(&payload).expect("serialize streamed json response");
    let split_at = response_bytes
        .len()
        .saturating_div(2)
        .clamp(1, response_bytes.len() - 1);
    let first_chunk = Bytes::copy_from_slice(&response_bytes[..split_at]);
    let second_chunk = Bytes::copy_from_slice(&response_bytes[split_at..]);
    let chunked = stream::unfold(0u8, move |state| {
        let first_chunk = first_chunk.clone();
        let second_chunk = second_chunk.clone();
        async move {
            match state {
                0 => Some((Ok::<Bytes, Infallible>(first_chunk), 1)),
                1 => {
                    tokio::time::sleep(delay).await;
                    Some((Ok::<Bytes, Infallible>(second_chunk), 2))
                }
                _ => None,
            }
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from_stream(chunked),
    )
        .into_response()
}

pub(crate) async fn spawn_capture_target_body_upstream()
-> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(test_upstream_capture_target_echo),
        )
        .route("/v1/responses", post(test_upstream_capture_target_echo))
        .route(
            "/v1/responses/compact",
            post(test_upstream_capture_target_compact_echo),
        )
        .with_state(captured.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind capture-target upstream test server");
    let addr = listener
        .local_addr()
        .expect("capture-target upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("capture-target upstream test server should run");
    });

    (format!("http://{addr}/"), captured, handle)
}

pub(crate) fn extract_model_ids(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect()
}

#[test]
pub(crate) fn build_proxy_upstream_url_preserves_path_prefix_and_query() {
    let base = Url::parse("https://proxy.example.com/gateway").expect("valid base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "https://proxy.example.com/gateway/v1/models?limit=10"
    );
}

#[test]
pub(crate) fn build_proxy_upstream_url_supports_ipv6_literal_base() {
    let base = Url::parse("http://[::1]:8080/gateway/").expect("valid ipv6 base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "http://[::1]:8080/gateway/v1/models?limit=10"
    );
}

#[test]
pub(crate) fn path_has_forbidden_dot_segment_detects_plain_and_encoded_variants() {
    assert!(path_has_forbidden_dot_segment("/v1/../models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e/models"));
    assert!(path_has_forbidden_dot_segment("/v1/.%2E/models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%2fadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%5cadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%252e%252e%252fadmin"));
    assert!(!path_has_forbidden_dot_segment("/v1/%2efoo/models"));
    assert!(!path_has_forbidden_dot_segment("/v1/models"));
}

#[test]
pub(crate) fn build_proxy_upstream_url_rejects_dot_segment_paths() {
    let base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let uri: Uri = "/v1/%2e%2e%2fadmin?scope=test"
        .parse()
        .expect("valid uri with dot segments");
    let err = build_proxy_upstream_url(&base, &uri).expect_err("dot segments should fail");
    assert!(
        err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED),
        "error should indicate forbidden dot segments: {err}"
    );
}

#[test]
pub(crate) fn has_invalid_percent_encoding_detects_malformed_sequences() {
    assert!(has_invalid_percent_encoding("/v1/%zz/models"));
    assert!(has_invalid_percent_encoding("/v1/%/models"));
    assert!(has_invalid_percent_encoding("/v1/%2/models"));
    assert!(!has_invalid_percent_encoding("/v1/%2F/models"));
    assert!(!has_invalid_percent_encoding("/v1/models"));
}

#[test]
pub(crate) fn should_transport_proxy_header_filters_only_transport_hop_by_hop_headers() {
    assert!(should_transport_proxy_header(&http_header::AUTHORIZATION));
    assert!(should_transport_proxy_header(&http_header::CONTENT_LENGTH));
    assert!(should_transport_proxy_header(&http_header::ACCEPT_ENCODING));
    assert!(!should_transport_proxy_header(&http_header::HOST));
    assert!(!should_transport_proxy_header(&http_header::CONNECTION));
    assert!(!should_transport_proxy_header(
        &http_header::TRANSFER_ENCODING
    ));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "forwarded"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "via"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-for"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-host"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-proto"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-port"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-real-ip"
    )));
}

#[test]
pub(crate) fn connection_scoped_header_names_parses_connection_tokens() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("keep-alive, x-foo"),
    );
    headers.append(http_header::CONNECTION, HeaderValue::from_static("x-bar"));
    let names = connection_scoped_header_names(&headers);
    assert!(names.contains(&http_header::HeaderName::from_static("keep-alive")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-foo")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-bar")));
}

#[test]
pub(crate) fn request_may_have_body_uses_method_and_headers() {
    let empty = HeaderMap::new();
    assert!(!request_may_have_body(&Method::GET, &empty));
    assert!(request_may_have_body(&Method::POST, &empty));

    let mut with_length = HeaderMap::new();
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    assert!(!request_may_have_body(&Method::GET, &with_length));
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("10"));
    assert!(request_may_have_body(&Method::GET, &with_length));
}

#[test]
pub(crate) fn parse_cors_allowed_origins_normalizes_and_deduplicates() {
    let parsed = parse_cors_allowed_origins(
        "https://EXAMPLE.com:443, http://127.0.0.1:8080, https://example.com",
    )
    .expect("parse should succeed");
    assert_eq!(
        parsed,
        vec![
            "https://example.com".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ]
    );
}

#[test]
pub(crate) fn origin_allowed_accepts_loopback_and_configured_origins() {
    let configured = HashSet::from(["https://api.example.com".to_string()]);
    assert!(origin_allowed("http://127.0.0.1:60080", &configured));
    assert!(origin_allowed("https://api.example.com", &configured));
    assert!(!origin_allowed("https://evil.example.com", &configured));
}

#[test]
pub(crate) fn same_origin_settings_write_allows_missing_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
pub(crate) fn same_origin_settings_write_rejects_cross_site_without_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
pub(crate) fn same_origin_settings_write_allows_matching_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
pub(crate) fn same_origin_settings_write_allows_matching_origin_without_explicit_host_port() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

fn locator_runtime_record(
    invoke_id: &str,
    occurred_at: &str,
    account_id: i64,
) -> ProxyCaptureRecord {
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let account_name = if account_id == 17 {
        "pool-account-17"
    } else {
        "pool-account-18"
    };
    build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        None,
        None,
        None,
        true,
        Some(account_id),
        Some(account_name),
        Some("api_key_codex"),
        None,
        None,
        None,
        None,
        None,
        None,
        1.0,
        1.0,
        1.0,
        1.0,
    ))
}

async fn assert_runtime_only_locator(state: &Arc<AppState>, invoke_id: &str) {
    let response = locate_invocation_page(
        state.clone(),
        &LocateInvocationQuery {
            invoke_id: Some(invoke_id.to_string()),
            request_id: Some(invoke_id.to_string()),
            attempt_id: None,
            upstream_account_id: Some(17),
            page_size: Some(50),
        },
    )
    .await
    .expect("runtime locator should succeed")
    .expect("runtime target should exist");
    assert_eq!(
        (response.target_absolute_index, response.target_index),
        (0, 0)
    );
    assert_eq!(response.records[0].invoke_id, invoke_id);
    assert_eq!(response.records[0].id, 0);
    let Json(page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            upstream_account_id: Some(17),
            page: Some(response.page),
            page_size: Some(response.page_size),
            snapshot_id: Some(response.snapshot_id),
            anchor_id: Some(response.anchor_id),
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("anchored page should preserve runtime overlay");
    assert!(
        page.records
            .iter()
            .any(|record| record.invoke_id == invoke_id)
    );
}

async fn seed_stable_locator_rows(pool: &SqlitePool) {
    for (invoke_id, occurred_at, status) in [
        ("locate-stale-account", "2026-03-17 18:16:00", "running"),
        ("locate-stable-target", "2026-03-17 18:15:00", "success"),
    ] {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response) \
             VALUES (?1, ?2, ?3, ?4, ?5, '{}')",
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(r#"{"upstreamAccountId":17,"upstreamAccountName":"pool-account-17"}"#)
        .execute(pool)
        .await
        .expect("insert locator row");
    }
}

async fn assert_stable_locator_suppresses_moved_runtime(state: &Arc<AppState>) {
    let response = locate_invocation_page(
        state.clone(),
        &LocateInvocationQuery {
            invoke_id: Some("locate-stable-target".to_string()),
            request_id: Some("locate-stable-target".to_string()),
            attempt_id: None,
            upstream_account_id: Some(17),
            page_size: Some(50),
        },
    )
    .await
    .expect("stable target locator should succeed")
    .expect("stable target should exist");
    assert!(
        response
            .records
            .iter()
            .all(|record| record.invoke_id != "locate-stale-account")
    );
    let Json(page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            upstream_account_id: Some(17),
            page: Some(response.page),
            page_size: Some(response.page_size),
            snapshot_id: Some(response.snapshot_id),
            anchor_id: Some(response.anchor_id),
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("stable anchor page should preserve stale-row suppression");
    assert!(
        page.records
            .iter()
            .all(|record| record.invoke_id != "locate-stale-account")
    );
}

#[tokio::test]
pub(crate) async fn locate_invocation_finds_runtime_only_account_record() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "locate-runtime-only";
    let record = locator_runtime_record(invoke_id, "2026-03-17 18:13:34", 17);
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("runtime snapshot should be available");

    assert_runtime_only_locator(&state, invoke_id).await;

    seed_stable_locator_rows(&state.pool).await;
    let moved_runtime_record =
        locator_runtime_record("locate-stale-account", "2026-03-17 18:16:00", 18);
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, moved_runtime_record)
        .await
        .expect("moved runtime snapshot should be available");

    assert_stable_locator_suppresses_moved_runtime(&state).await;
}

use super::*;
