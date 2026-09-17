async fn seed_sticky_preflight_block(state: &Arc<AppState>, sticky_account: i64) {
    let now_iso = format_utc_iso(Utc::now());
    let lock_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, allow_cut_out, allow_cut_in,
            priority_tier, fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
            upstream_429_max_retries, available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, 0, 0, 1, 'normal', 'keep_original', 0, 0, 0, '[]', ?3, ?3)
        RETURNING id
        "#,
    )
    .bind("sticky-preflight-lock")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert sticky lock tag");
    sqlx::query(
        "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?3)",
    )
    .bind(sticky_account)
    .bind(lock_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach sticky lock tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-preflight-blocked",
        sticky_account,
        &now_iso,
    )
    .await
    .expect("seed sticky route");
}

#[tokio::test]
pub(crate) async fn failover_preserves_assigned_account_when_sticky_owner_is_preflight_blocked() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid url")).await;
    let sticky_account = insert_test_pool_oauth_account(
        &state,
        "Sticky Missing Binding",
        "sticky-preflight-missing",
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        sticky_account,
        Some("sticky-preflight-missing"),
    )
    .await;
    let _fallback_account =
        insert_test_pool_oauth_account(&state, "Fallback Healthy Account", "fallback-healthy")
            .await;
    seed_sticky_preflight_block(&state, sticky_account).await;

    let err = send_pool_request_with_failover(
        state.clone(),
        700701,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from_static(
            br#"{"input":"hello"}"#,
        ))),
        Duration::from_secs(5),
        Some(PoolUpstreamAttemptTraceContext {
            invoke_id: "sticky-preflight-blocked-invoke".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-preflight-blocked".to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        }),
        None,
        Some("sticky-preflight-blocked"),
        None,
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect_err("sticky preflight block should fail");

    assert_eq!(
        err.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(
        err.account.as_ref().map(|account| account.account_id),
        Some(sticky_account)
    );
    assert!(err.message.contains(
        "upstream account group \"sticky-preflight-missing\" has no bound forward proxy nodes"
    ));

    wait_for_pool_attempt_row_count(&state.pool, 0).await;
    let attempt_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count sticky preflight blocked attempts");
    assert_eq!(attempt_count, 0);
}

use super::*;
