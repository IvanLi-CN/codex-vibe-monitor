use super::*;
use tower::ServiceExt;

fn public_discovery_headers() -> Vec<HeaderMap> {
    vec![
        HeaderMap::new(),
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer unknown-pool-key"),
        )]),
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Basic unrelated-credential"),
        )]),
    ]
}

#[tokio::test]
async fn proxy_openai_v1_models_public_catalog_uses_only_enabled_presets() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.enabled_preset_models = vec!["gpt-5.4".to_string(), "custom-model".to_string()];
        settings.hijack_enabled = false;
        settings.merge_upstream_enabled = true;
    }
    let app = build_proxy_routes(Router::new()).with_state(state.clone());

    for headers in public_discovery_headers() {
        let mut request = axum::http::Request::builder()
            .uri("/v1/models?client_version=public-discovery")
            .body(Body::empty())
            .expect("build public discovery request");
        *request.headers_mut() = headers;
        let response = app.clone().oneshot(request).await.expect("serve models");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get(PROXY_MODEL_MERGE_STATUS_HEADER)
                .is_none()
        );
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read public catalog"),
        )
        .expect("decode public catalog");
        assert_eq!(payload["object"], "list");
        assert_eq!(extract_model_ids(&payload), vec!["gpt-5.4", "custom-model"]);
        assert!(
            payload["data"]
                .as_array()
                .expect("model records")
                .iter()
                .all(|model| model["object"] == "model" && model["owned_by"] == "proxy")
        );
    }

    let invocation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&state.pool)
        .await
        .expect("count invocations");
    assert_eq!(invocation_count, 0);
    assert_eq!(state.proxy_request_in_flight.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn proxy_openai_v1_models_public_catalog_survives_closed_database_and_empty_presets() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;
    state
        .proxy_model_settings
        .write()
        .await
        .enabled_preset_models
        .clear();
    state.pool.close().await;
    let app = build_proxy_routes(Router::new()).with_state(state.clone());

    for headers in public_discovery_headers() {
        let mut request = axum::http::Request::builder()
            .uri("/v1/models")
            .body(Body::empty())
            .expect("build public discovery request");
        *request.headers_mut() = headers;
        let response = app.clone().oneshot(request).await.expect("serve models");
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read empty public catalog"),
        )
        .expect("decode empty public catalog");
        assert_eq!(payload, json!({"object": "list", "data": []}));
    }
    assert_eq!(state.proxy_request_in_flight.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn proxy_openai_v1_models_public_catalog_keeps_other_routes_authenticated() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;
    let app = build_proxy_routes(Router::new()).with_state(state);

    for (method, path) in [
        (Method::POST, "/v1/models"),
        (Method::GET, "/v1/models/"),
        (Method::GET, "/v1/models/gpt-5.4"),
        (Method::POST, "/v1/responses"),
        (Method::POST, "/v1/chat/completions"),
    ] {
        for headers in public_discovery_headers() {
            let mut request = axum::http::Request::builder()
                .method(method.clone())
                .uri(path)
                .body(Body::empty())
                .expect("build protected request");
            *request.headers_mut() = headers;
            let response = app.clone().oneshot(request).await.expect("serve route");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {path}"
            );
        }
    }
}
