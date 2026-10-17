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

#[tokio::test]
async fn id_scoped_recovery_does_not_touch_rows_outside_the_startup_fence() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for invoke_id in ["startup-fence-before", "startup-fence-after"] {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, payload, raw_response) VALUES (?, ?, 'proxy', 'running', 0, '{}', '{}')",
        )
        .bind(invoke_id)
        .bind(if invoke_id.ends_with("before") {
            "2026-10-18T00:00:00Z"
        } else {
            "2026-10-18T00:00:01Z"
        })
        .execute(&state.pool)
        .await
        .expect("insert startup recovery fixture");
    }
    let ids = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id LIKE 'startup-fence-%' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load startup recovery fixture ids");
    assert_eq!(ids.len(), 2);

    let recovered = crate::recover_proxy_invocations_with_scope(
        &state.pool,
        crate::ProxyInvocationRecoveryScope::Ids(&ids[..1]),
    )
    .await
    .expect("recover only fenced invocation ids");
    assert_eq!(recovered.len(), 1);

    let statuses = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, status FROM codex_invocations WHERE invoke_id LIKE 'startup-fence-%' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load startup recovery fixture statuses");
    assert_eq!(statuses[0].0, ids[0]);
    assert_eq!(statuses[0].1, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(statuses[1].0, ids[1]);
    assert_eq!(statuses[1].1, "running");
}
