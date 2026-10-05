use super::*;

#[tokio::test]
async fn counted_http_transport_reports_network_bytes_through_dashboard_projection() {
    let app = Router::new().route(
        "/",
        any(|| async {
            let body = stream::iter(vec![
                Ok::<Bytes, Infallible>(Bytes::from_static(b"stream-")),
                Ok::<Bytes, Infallible>(Bytes::from_static(b"response")),
            ]);
            (StatusCode::OK, Body::from_stream(body))
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind counted dashboard upstream test server");
    let address = listener
        .local_addr()
        .expect("read counted dashboard upstream address");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("counted dashboard upstream test server should run");
    });
    let state = test_state_from_config(test_config(), true).await;
    let target_url = Url::parse(&format!("http://{address}/")).expect("valid counted target");
    let reporter = UpstreamTrafficReporter::new(
        state.clone(),
        "network-projection-http",
        "2026-09-28 16:00:00",
        Some(42),
        Some("api.example.test"),
        "/",
    );

    let response = send_counted_upstream_http_request(
        Method::POST,
        &target_url,
        &HeaderMap::new(),
        Body::from("client-request"),
        None,
        Some(reporter),
    )
    .await
    .expect("counted HTTP request should succeed");
    let response_body = axum::body::to_bytes(response.response.into_body(), usize::MAX)
        .await
        .expect("read counted dashboard response");
    assert_eq!(response_body.as_ref(), b"stream-response");

    let now = Utc::now();
    let global = state
        .dashboard_network_speed_cache
        .snapshot_open_bucket(DashboardNetworkScopeKey::Global, now)
        .totals;
    let account = state
        .dashboard_network_speed_cache
        .snapshot_open_bucket(DashboardNetworkScopeKey::Account(42), now)
        .totals;
    assert!(global.upload_bytes > 0);
    assert!(global.download_bytes > 0);
    assert_eq!(account, global);

    let pending = state
        .proxy_runtime_invocations
        .pending_dashboard_publish_window();
    assert!(pending.is_some_and(|window| { window.slice == DashboardProjectionSlice::Network }));
    let projection = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture dashboard network projection");
    let projected_global = projection
        .slice
        .network_live_bucket
        .as_ref()
        .expect("projected global live bucket");
    assert_eq!(projected_global.upload_bytes, global.upload_bytes);
    assert_eq!(projected_global.download_bytes, global.download_bytes);
    let projected_account = projection
        .slice
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("projected account network bucket");
    let projected_account_bucket = projected_account
        .network_live_bucket
        .as_ref()
        .expect("projected account live bucket");
    assert_eq!(projected_account_bucket.upload_bytes, account.upload_bytes);
    assert_eq!(
        projected_account_bucket.download_bytes,
        account.download_bytes
    );

    upstream_handle.abort();
}
