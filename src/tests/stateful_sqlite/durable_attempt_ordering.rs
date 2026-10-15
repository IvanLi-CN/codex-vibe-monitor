use super::*;

#[tokio::test]
async fn durable_account_attempt_pages_and_locate_preserve_insertion_order_for_timestamp_ties() {
    let state = test_state_from_config(test_config(), true).await;
    let account = insert_test_pool_api_key_account(&state, "Ordering", "fixture-token").await;
    let occurred_at = shanghai_now_string();
    for index in 1..=25 {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts \
             (attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, \
              upstream_account_id, attempt_index, distinct_account_index, same_account_retry_index, status) \
             VALUES (?1, ?1, ?2, '/v1/responses', 'pool', ?3, ?4, 1, 0, 'success')",
        )
        .bind(format!("durable-tie-{index}"))
        .bind(&occurred_at)
        .bind(account)
        .bind(26 - index)
        .execute(&state.pool)
        .await
        .expect("insert equal-time facts with conflicting attempt indices");
    }
    let expected = (1..=25)
        .rev()
        .map(|index| format!("durable-tie-{index}"))
        .collect::<Vec<_>>();
    let mut pages = Vec::new();
    for page in 1..=2 {
        let Json(response) = list_upstream_account_attempts(
            State(state.clone()),
            axum::extract::Path(account),
            Query(ListUpstreamAccountAttemptsQuery {
                page: Some(page),
                page_size: Some(20),
                ..Default::default()
            }),
        )
        .await
        .expect("read durable account page");
        assert_eq!(response.total, 25);
        pages.push(
            response
                .items
                .into_iter()
                .map(|item| item.attempt_id)
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        pages.concat(),
        expected,
        "durable-only REST/SSE pages retain occurred_at DESC, id DESC"
    );
    for (target, expected_page) in [("durable-tie-6", 1), ("durable-tie-5", 2)] {
        let Json(located) = locate_upstream_account_attempt(
            State(state.clone()),
            axum::extract::Path(account),
            Query(LocateUpstreamAccountAttemptQuery {
                attempt_id: target.to_string(),
                page_size: Some(20),
            }),
        )
        .await
        .expect("locate across the legacy page boundary");
        assert_eq!(located.page, expected_page);
        assert_eq!(
            located
                .items
                .into_iter()
                .map(|item| item.attempt_id)
                .collect::<Vec<_>>(),
            pages[expected_page - 1]
        );
    }
    state.shutdown.cancel();
}
