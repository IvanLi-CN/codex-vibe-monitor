use super::*;

#[tokio::test]
async fn interrupted_websocket_turn_persists_partial_cache_usage_from_upstream_events() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pool-ws-partial-interrupted".to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: Some("api.openai.com".to_string()),
        request_model: Some("gpt-6-sol".to_string()),
    };
    let mut tracker = WsUsageTracker::new(
        api_key_account(Url::parse("https://api.openai.com/").expect("valid base")),
        trace,
        None,
        None,
        None,
    );
    tracker.start_turn_at(Instant::now(), Utc::now().to_rfc3339());
    tracker
        .observe_upstream_text(
            &state,
            r#"{"type":"response.output_text.delta","response_id":"resp_partial_interrupt","delta":"hello"}"#,
        )
        .await;
    tracker
        .observe_upstream_text(
            &state,
            r#"{"type":"response.in_progress","response":{"id":"resp_partial_interrupt","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
        )
        .await;

    let invoke_id = tracker
        .runtime_snapshot_invoke_id
        .clone()
        .expect("first-token runtime identity");
    let occurred_at = tracker.turn_occurred_at.clone().expect("turn timestamp");
    let pre_terminal_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(&invoke_id)
    .bind(&occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count before interrupted terminal");
    assert_eq!(pre_terminal_count, 0);

    tracker
        .persist_interrupted_turn(state.as_ref(), "test websocket interruption")
        .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let persisted = sqlx::query_as::<_, (String, Option<i64>, Option<i64>)>(
        r#"
        SELECT status, cache_input_tokens, reported_cache_write_tokens
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load interrupted websocket usage");
    assert_eq!(persisted.0, "failed");
    assert_eq!(persisted.1, Some(325));
    assert_eq!(persisted.2, None);
}
