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

#[tokio::test]
async fn browser_ingestion_rejections_consume_client_budget_before_parsing() {
    let valid = json!({"events":[{
        "page":"dashboard", "device":"desktop", "kind":"api_request", "valueSeconds":0.1
    }]})
    .to_string();
    let invalid_value = json!({"events":[{
        "page":"dashboard", "device":"desktop", "kind":"api_request", "valueSeconds":-1
    }]})
    .to_string();
    let invalid_schema = json!({"events":[{
        "page":"private-account", "device":"desktop", "kind":"api_request", "valueSeconds":0.1
    }]})
    .to_string();
    let oversized = format!("{}{}", " ".repeat(2049), valid);
    for (name, body, site, expected) in [
        (
            "origin",
            valid.as_str(),
            "cross-site",
            StatusCode::FORBIDDEN,
        ),
        (
            "value",
            invalid_value.as_str(),
            "same-origin",
            StatusCode::BAD_REQUEST,
        ),
        (
            "schema",
            invalid_schema.as_str(),
            "same-origin",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("syntax", "{", "same-origin", StatusCode::BAD_REQUEST),
        (
            "size",
            oversized.as_str(),
            "same-origin",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "accepted",
            valid.as_str(),
            "same-origin",
            StatusCode::NO_CONTENT,
        ),
    ] {
        let mut config = test_config();
        config.observability.enabled = true;
        let mut state = test_state_from_config(config, false).await;
        Arc::get_mut(&mut state)
            .expect("browser fixture is exclusive before HTTP server startup")
            .observability = ObservabilityRuntime::new(true);
        let (address, server) = spawn_http_server(state.clone())
            .await
            .expect("start browser quota HTTP server");
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("browser quota client");
        let url = format!("http://{address}/api/system/observability/browser");
        for index in 0..30 {
            let response = client
                .post(&url)
                .header("content-type", "application/json")
                .header("sec-fetch-site", site)
                .header("x-real-ip", "192.0.2.1")
                .body(body.to_owned())
                .send()
                .await
                .expect("send browser quota fixture");
            assert_eq!(response.status(), expected, "{name} request {index}");
        }
        for next_body in [body, valid.as_str()] {
            let response = client
                .post(&url)
                .header("content-type", "application/json")
                .header("sec-fetch-site", site)
                .header("x-real-ip", "192.0.2.1")
                .body(next_body.to_owned())
                .send()
                .await
                .expect("request exhausted browser client quota");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "{name}");
        }
        let response = client
            .post(&url)
            .header("content-type", "application/json")
            .header("sec-fetch-site", "same-origin")
            .header("x-real-ip", "192.0.2.2")
            .body(valid.clone())
            .send()
            .await
            .expect("request independent browser client quota");
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "{name}");
        let metrics = state.observability.render();
        assert!(
            metrics.contains("cvm_browser_ingest_total{"),
            "browser fixture must use an enabled independent recorder: {metrics}"
        );
        assert!(
            !metrics.contains("192.0.2.") && !metrics.contains("private-account"),
            "client identity and rejected dimensions must never be metric labels: {metrics}"
        );
        assert!(
            !metrics.contains("cvm_http_requests_total{"),
            "browser ingestion must not count itself: {metrics}"
        );
        state.shutdown.cancel();
        server.await.expect("join browser quota HTTP server");
        state.pool.close().await;
    }
}

#[tokio::test]
async fn browser_ingestion_mixed_rejections_consume_global_budget() {
    let mut config = test_config();
    config.observability.enabled = true;
    let mut state = test_state_from_config(config, false).await;
    Arc::get_mut(&mut state)
        .expect("browser fixture is exclusive before HTTP server startup")
        .observability = ObservabilityRuntime::new(true);
    let (address, server) = spawn_http_server(state.clone())
        .await
        .expect("start global browser quota HTTP server");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("global browser quota client");
    let url = format!("http://{address}/api/system/observability/browser");
    for index in 0..122 {
        let (site, value, expected) = if index >= 120 {
            ("same-origin", 0.1, StatusCode::TOO_MANY_REQUESTS)
        } else {
            match index % 3 {
                0 => ("cross-site", 0.1, StatusCode::FORBIDDEN),
                1 => ("same-origin", -1.0, StatusCode::BAD_REQUEST),
                _ => ("same-origin", 0.1, StatusCode::NO_CONTENT),
            }
        };
        let response = client
            .post(&url)
            .header("sec-fetch-site", site)
            .header("x-real-ip", format!("192.0.2.{}", index + 1))
            .json(&json!({"events":[{
                "page":"dashboard", "device":"desktop", "kind":"api_request", "valueSeconds":value
            }]}))
            .send()
            .await
            .expect("send mixed browser quota fixture");
        assert_eq!(response.status(), expected, "request {index}");
    }
    let metrics = state.observability.render();
    for (outcome, reason, count) in [
        ("accepted", "none", 40),
        ("rejected", "origin", 40),
        ("rejected", "validation", 40),
        ("rejected", "rate", 2),
    ] {
        assert!(
            metrics.contains(&format!(
                "cvm_browser_ingest_total{{outcome=\"{outcome}\",reason=\"{reason}\"}} {count}"
            )),
            "browser outcome accounting must match actual requests: {metrics}"
        );
    }
    assert!(
        !metrics.contains("cvm_http_requests_total{") && !metrics.contains("192.0.2."),
        "browser ingestion must not self-count or label clients: {metrics}"
    );
    state.shutdown.cancel();
    server.await.expect("join global browser quota HTTP server");
    state.pool.close().await;
}
