use super::*;

#[tokio::test]
async fn retired_performance_apis_return_410_through_the_real_cors_stack() {
    let mut config = test_config();
    config.http_bind = "127.0.0.1:0".parse().expect("ephemeral HTTP bind");
    let state = test_state_from_config(config, false).await;
    let (address, server) = spawn_http_server(state.clone())
        .await
        .expect("start application HTTP stack");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("local retirement client");
    for path in [
        "/api/system/performance",
        "/api/system/performance/health",
        "/api/system/performance/browser",
    ] {
        for method in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
            Method::HEAD,
        ] {
            let response = client
                .request(method.clone(), format!("http://{address}{path}"))
                .header(http_header::ORIGIN, "http://localhost:60080")
                .header(http_header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .send()
                .await
                .expect("request retired API through CORS");
            assert_eq!(response.status(), StatusCode::GONE, "{method} {path}");
            assert_eq!(
                response.headers()[http_header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "http://localhost:60080"
            );
            if method != Method::HEAD {
                let body: Value = response.json().await.expect("static tombstone JSON");
                assert_eq!(body["code"], "performance_retired");
            }
        }
    }
    let response = client
        .request(Method::OPTIONS, format!("http://{address}/health"))
        .header(http_header::ORIGIN, "http://localhost:60080")
        .header(http_header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .send()
        .await
        .expect("normal API preflight");
    assert_eq!(response.status(), StatusCode::OK);
    state.shutdown.cancel();
    server.await.expect("join application HTTP server");
    state.pool.close().await;
}
