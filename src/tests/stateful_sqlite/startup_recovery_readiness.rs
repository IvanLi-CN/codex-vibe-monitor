use super::*;

#[tokio::test]
async fn business_routes_reject_before_readiness_while_health_remains_available() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state.startup_ready.store(false, Ordering::Release);
    let app = build_app_router(state.clone());

    let business_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/system/status")
                .body(Body::empty())
                .expect("build business request"),
        )
        .await
        .expect("serve business request");
    assert_eq!(business_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        axum::body::to_bytes(business_response.into_body(), usize::MAX)
            .await
            .expect("read business response body"),
        Bytes::from_static(b"starting")
    );

    let health_response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build health request"),
        )
        .await
        .expect("serve health request");
    assert_eq!(health_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        axum::body::to_bytes(health_response.into_body(), usize::MAX)
            .await
            .expect("read health response body"),
        Bytes::from_static(b"starting")
    );
}
