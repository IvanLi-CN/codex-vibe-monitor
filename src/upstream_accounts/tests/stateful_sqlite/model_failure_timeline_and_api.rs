use super::*;
use std::time::{Duration, Instant};

#[test]
fn explicit_model_failure_recognizes_standard_not_found_and_rate_limit_shapes() {
    assert!(is_explicit_model_failure(
        StatusCode::NOT_FOUND,
        Some(r#"{"code":"model_not_found","message":"model gpt-5.5 does not exist"}"#),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::NOT_FOUND,
        Some("model gpt-5.4 does not exist"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::NOT_FOUND,
        Some("model gpt-5.5 does not exist"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for this account"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("project model rate limit exceeded"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("organization model quota exceeded"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("account model limit reached"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for IP 203.0.113.1"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for endpoint /v1/responses"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for model gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5 quota"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("model rate limit exceeded"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: gpt-5.5"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: foo"),
        Some("foo"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported_model: pool upstream responded with 400: unsupported model: foo"),
        Some("foo"),
    ));
}

#[test]
fn model_route_failure_messages_are_sanitized_before_persistence() {
    let raw = format!("\u{0000}{}", "upstream model failure ".repeat(40));
    let sanitized = sanitize_account_action_message(&raw).expect("non-empty sanitized message");
    assert!(sanitized.len() <= 240);
    assert!(!sanitized.chars().any(char::is_control));
}

pub(crate) async fn insert_model_failure_attempt(
    state: &AppState,
    account_id: i64,
    invoke_id: &str,
    model: Option<&str>,
) -> i64 {
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, 'failed')",
    )
    .bind(invoke_id)
    .bind(format_utc_iso(Utc::now()))
    .bind(model)
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert temporary model failure attempt")
    .last_insert_rowid()
}

#[tokio::test]
async fn model_routing_timeline_queries_use_epoch_and_latest_event_indexes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    crate::ensure_schema(&state.pool)
        .await
        .expect("timeline schema migration should be idempotent");
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Timeline index account",
        "timeline-index-api-key",
        None,
        None,
    )
    .await;
    let model = "gpt-timeline-index";
    let now = Utc::now();
    let recent_local = format_naive(now.with_timezone(&Shanghai).naive_local());
    let recent_utc = format_utc_iso(now - ChronoDuration::seconds(1));
    let expired_utc = format_utc_iso(now - ChronoDuration::minutes(20));
    let expired_local = format_naive(
        (now - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let attempt_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id,
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
            status
        ) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'timeline-index', 1, 1, 0, 'success')
        RETURNING id
        "#,
    )
    .bind("timeline-index-attempt")
    .bind(&recent_local)
    .bind(model)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("insert local-timestamp timeline attempt");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, attempt_id, model,
            model_route_state_before, model_route_state_after, created_at
        ) VALUES
            (?1, ?2, 'model_route_state_changed', 'call', ?3, ?4, 'available', 'degraded', ?2),
            (?1, ?5, 'model_route_state_changed', 'call', ?3, ?4, 'degraded', 'available', ?5),
            (?1, ?6, 'model_route_state_changed', 'call', NULL, ?4, 'available', 'degraded', ?6),
            (?1, ?7, 'model_route_state_changed', 'call', NULL, ?4, 'available', 'degraded', ?7)
        "#,
    )
    .bind(account_id)
    .bind(&recent_local)
    .bind(attempt_id)
    .bind(model)
    .bind(&recent_utc)
    .bind(&recent_utc)
    .bind(&expired_local)
    .execute(&state.pool)
    .await
    .expect("insert linked and standalone timeline events");

    let cutoff_epoch_ms = (Utc::now() - ChronoDuration::minutes(15)).timestamp_millis();
    let attempt_plan = build_model_routing_attempt_timeline_query(
        "EXPLAIN QUERY PLAN ",
        None,
        None,
        cutoff_epoch_ms,
        10,
        None,
        None,
    )
    .build_query_as::<(i64, i64, i64, String)>()
    .fetch_all(&state.pool)
    .await
    .expect("explain production attempt timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(
        attempt_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH attempts USING INDEX idx_pool_upstream_request_attempts_timeline_epoch",
            )
        }),
        "attempt timeline query must use the epoch range index: {attempt_plan:?}"
    );
    assert!(
        attempt_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH latest USING INDEX idx_pool_upstream_account_events_attempt_latest",
            )
        }),
        "latest event subquery must use the attempt index: {attempt_plan:?}"
    );
    assert!(
        attempt_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")),
        "attempt timeline query must not sort through a temporary B-tree: {attempt_plan:?}"
    );

    let event_plan = build_model_routing_event_timeline_query(
        "EXPLAIN QUERY PLAN ",
        None,
        None,
        cutoff_epoch_ms,
        10,
        None,
        None,
    )
    .build_query_as::<(i64, i64, i64, String)>()
    .fetch_all(&state.pool)
    .await
    .expect("explain production standalone event timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(
        event_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH event USING INDEX idx_pool_upstream_account_events_timeline_unlinked_epoch",
            )
        }),
        "standalone event timeline query must use the epoch range index: {event_plan:?}"
    );
    assert!(
        event_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")),
        "standalone event timeline query must not sort through a temporary B-tree: {event_plan:?}"
    );

    let Json(live) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("15m".to_string()),
            model: Some(model.to_string()),
            state: None,
            limit: Some(10),
        }),
    )
    .await
    .expect("load mixed-format model routing timeline");
    assert!(
        live.records
            .iter()
            .any(|record| record.id == format!("attempt:{attempt_id}")),
        "the local-timestamp attempt should remain visible in the 15-minute window"
    );
    assert_eq!(
        live.records
            .iter()
            .find(|record| record.id == format!("attempt:{attempt_id}"))
            .and_then(|record| record.model_route_state_after.as_deref()),
        Some("degraded"),
        "the attempt must use its latest linked transition across timestamp formats"
    );
    assert!(
        live.records
            .iter()
            .any(|record| record.kind == "event" && record.occurred_at == recent_utc),
        "the RFC3339 standalone event should remain visible in the 15-minute window"
    );
    assert!(
        live.records
            .iter()
            .all(|record| record.occurred_at != expired_utc),
        "the expired standalone event must remain outside the 15-minute window"
    );

    let Json(first_page) = list_upstream_account_model_routing_events(
        State(state.clone()),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: None,
            page_size: Some(1),
        }),
    )
    .await
    .expect("load the first mixed-format model routing history page");
    assert_eq!(first_page.items.len(), 1);
    assert_eq!(first_page.items[0].id, format!("attempt:{attempt_id}"));
    let cursor = first_page
        .next_cursor
        .expect("the standalone event should produce a second history page");

    let Json(second_page) = list_upstream_account_model_routing_events(
        State(state.clone()),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: Some(cursor),
            page_size: Some(1),
        }),
    )
    .await
    .expect("load the second mixed-format model routing history page");
    assert_eq!(second_page.items.len(), 1);
    assert_eq!(second_page.items[0].kind, "event");
    assert_eq!(second_page.items[0].occurred_at, recent_utc);
    assert_ne!(first_page.items[0].id, second_page.items[0].id);
    let cursor = second_page
        .next_cursor
        .expect("the older standalone event should produce a third history page");

    let Json(third_page) = list_upstream_account_model_routing_events(
        State(state),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: Some(cursor),
            page_size: Some(1),
        }),
    )
    .await
    .expect("load the third mixed-format model routing history page");
    assert_eq!(third_page.items.len(), 1);
    assert_eq!(third_page.items[0].kind, "event");
    assert_eq!(third_page.items[0].occurred_at, expired_utc);
    assert_ne!(first_page.items[0].id, third_page.items[0].id);
    assert_ne!(second_page.items[0].id, third_page.items[0].id);
    assert!(third_page.next_cursor.is_none());
}

#[tokio::test]
async fn model_routing_live_query_stays_bounded_for_dense_recent_timeline() {
    const TIMELINE_ROWS: usize = 4_000;
    const INSERT_BATCH_SIZE: usize = 250;
    const ATTEMPT_ID_BASE: i64 = 10_000_000;
    const LINKED_EVENT_ID_BASE: i64 = 20_000_000;
    const STANDALONE_EVENT_ID_BASE: i64 = 40_000_000;

    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Timeline load account",
        "timeline-load-api-key",
        None,
        None,
    )
    .await;
    let model = "gpt-timeline-load";
    let now = Utc::now();
    let mut transaction = state
        .pool
        .begin()
        .await
        .expect("begin dense timeline fixture transaction");

    for batch_start in (0..TIMELINE_ROWS).step_by(INSERT_BATCH_SIZE) {
        let batch_end = (batch_start + INSERT_BATCH_SIZE).min(TIMELINE_ROWS);
        let mut attempts = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) ",
        );
        attempts.push_values(batch_start..batch_end, |mut row, index| {
            let occurred_at = format_utc_iso(now - ChronoDuration::seconds((index % 720) as i64));
            row.push_bind(ATTEMPT_ID_BASE + index as i64)
                .push_bind(format!("timeline-load-attempt-{index}"))
                .push_bind(occurred_at)
                .push_bind("/v1/responses")
                .push_bind("pool")
                .push_bind(model)
                .push_bind(account_id)
                .push_bind("timeline-load")
                .push_bind(1_i64)
                .push_bind(1_i64)
                .push_bind(0_i64)
                .push_bind("success");
        });
        attempts
            .build()
            .execute(&mut *transaction)
            .await
            .expect("insert dense timeline attempts");

        let mut linked_events = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO pool_upstream_account_events (id, account_id, occurred_at, action, source, attempt_id, model, model_route_state_before, model_route_state_after, created_at) ",
        );
        linked_events.push_values(
            (batch_start..batch_end).flat_map(|attempt_index| {
                (0..3).map(move |event_index| (attempt_index, event_index))
            }),
            |mut row, (attempt_index, event_index)| {
                let occurred_at =
                    format_utc_iso(now - ChronoDuration::seconds((attempt_index % 720) as i64));
                row.push_bind(LINKED_EVENT_ID_BASE + (attempt_index * 3 + event_index) as i64)
                    .push_bind(account_id)
                    .push_bind(occurred_at.clone())
                    .push_bind("model_route_state_changed")
                    .push_bind("call")
                    .push_bind(ATTEMPT_ID_BASE + attempt_index as i64)
                    .push_bind(model)
                    .push_bind("available")
                    .push_bind(if event_index == 2 {
                        "degraded"
                    } else {
                        "available"
                    })
                    .push_bind(occurred_at);
            },
        );
        linked_events
            .build()
            .execute(&mut *transaction)
            .await
            .expect("insert dense linked timeline events");

        let mut standalone_events = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO pool_upstream_account_events (id, account_id, occurred_at, action, source, attempt_id, model, model_route_state_before, model_route_state_after, created_at) ",
        );
        standalone_events.push_values(batch_start..batch_end, |mut row, index| {
            let occurred_at = format_utc_iso(now - ChronoDuration::seconds((index % 720) as i64));
            row.push_bind(STANDALONE_EVENT_ID_BASE + index as i64)
                .push_bind(account_id)
                .push_bind(occurred_at.clone())
                .push_bind("model_route_state_changed")
                .push_bind("call")
                .push_bind(Option::<i64>::None)
                .push_bind(model)
                .push_bind("available")
                .push_bind("degraded")
                .push_bind(occurred_at);
        });
        standalone_events
            .build()
            .execute(&mut *transaction)
            .await
            .expect("insert dense standalone timeline events");
    }
    transaction
        .commit()
        .await
        .expect("commit dense timeline fixture");

    let started = Instant::now();
    let Json(live) = get_model_routing_live(
        State(state),
        Query(ModelRoutingLiveQuery {
            window: Some("15m".to_string()),
            model: None,
            state: None,
            limit: Some(1),
        }),
    )
    .await
    .expect("load the production dense 15-minute timeline");
    let elapsed = started.elapsed();

    assert_eq!(live.records.len(), 1);
    assert_eq!(live.records[0].kind, "attempt");
    assert!(
        elapsed < Duration::from_secs(2),
        "the dense production timeline request must remain bounded; elapsed={elapsed:?}"
    );
}

#[tokio::test]
async fn model_routing_timeline_schema_upgrade_adds_epoch_columns_and_indexes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Timeline upgrade account",
        "timeline-upgrade-api-key",
        None,
        None,
    )
    .await;
    let now = Utc::now();
    let legacy_attempt_time = format_naive(now.with_timezone(&Shanghai).naive_local());
    let legacy_event_time = format_utc_iso(now - ChronoDuration::seconds(1));

    for index in [
        "idx_pool_upstream_request_attempts_timeline_epoch",
        "idx_pool_upstream_account_events_attempt_latest",
        "idx_pool_upstream_account_events_timeline_unlinked_epoch",
    ] {
        sqlx::query(&format!("DROP INDEX {index}"))
            .execute(&state.pool)
            .await
            .expect("remove current timeline index before upgrade migration");
    }
    sqlx::query("ALTER TABLE pool_upstream_request_attempts DROP COLUMN occurred_epoch_ms")
        .execute(&state.pool)
        .await
        .expect("restore the pre-epoch attempts table");
    sqlx::query("ALTER TABLE pool_upstream_account_events DROP COLUMN occurred_epoch_ms")
        .execute(&state.pool)
        .await
        .expect("restore the pre-epoch events table");

    let attempt_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id,
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
            status
        ) VALUES ('timeline-upgrade-attempt', ?1, '/v1/responses', 'pool', 'gpt-timeline-upgrade', ?2, 'timeline-upgrade', 1, 1, 0, 'success')
        RETURNING id
        "#,
    )
    .bind(&legacy_attempt_time)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("insert pre-migration timeline attempt");
    let event_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, attempt_id, model,
            model_route_state_before, model_route_state_after, created_at
        ) VALUES (?1, ?2, 'model_route_state_changed', 'call', ?3, 'gpt-timeline-upgrade', 'available', 'degraded', ?2)
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(&legacy_event_time)
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("insert pre-migration linked event");

    crate::ensure_schema(&state.pool)
        .await
        .expect("upgrade legacy timeline tables with epoch columns and indexes");
    let attempt_epoch_ms: i64 = sqlx::query_scalar(
        "SELECT occurred_epoch_ms FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("read generated epoch from migrated attempt");
    let event_epoch_ms: i64 = sqlx::query_scalar(
        "SELECT occurred_epoch_ms FROM pool_upstream_account_events WHERE id = ?1",
    )
    .bind(event_id)
    .fetch_one(&state.pool)
    .await
    .expect("read generated epoch from migrated event");
    assert_eq!(attempt_epoch_ms, now.timestamp() * 1_000);
    assert_eq!(
        event_epoch_ms,
        (now - ChronoDuration::seconds(1)).timestamp() * 1_000
    );

    let timeline_indexes = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name IN ('idx_pool_upstream_request_attempts_timeline_epoch', 'idx_pool_upstream_account_events_attempt_latest', 'idx_pool_upstream_account_events_timeline_unlinked_epoch') ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await
    .expect("list migrated timeline indexes");
    assert_eq!(timeline_indexes.len(), 3);
    crate::ensure_schema(&state.pool)
        .await
        .expect("rerun upgraded timeline schema idempotently");
}

#[tokio::test]
async fn model_routing_live_api_lists_api_key_attempts_and_pages_account_history() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let display_name = "Routing live API account";
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        display_name,
        "routing-live-api-key",
        None,
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("irrelevant-to-routing")
        .execute(&state.pool)
        .await
        .expect("assign account group outside the routing read model");
    let model = "gpt-routing-live-api";
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed API Key model route");
    let cache_usage_missing_since = format_utc_iso(Utc::now());
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET last_failure_kind = 'upstream_http_5xx', last_failure_message = ?3, cache_usage_missing_since = ?4, cache_usage_missing_reason = 'missing_cache_input_tokens' WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind("upstream response contained a sensitive diagnostic")
    .bind(&cache_usage_missing_since)
    .execute(&state.pool)
    .await
    .expect("seed a sensitive failure message");
    let first_attempt =
        insert_model_failure_attempt(&state, account_id, "routing-live-api-first", Some(model))
            .await;
    let second_attempt =
        insert_model_failure_attempt(&state, account_id, "routing-live-api-second", Some(model))
            .await;

    let Json(live) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: Some(MODEL_ROUTE_STATE_AVAILABLE.to_string()),
            limit: Some(100),
        }),
    )
    .await
    .expect("load live model routing snapshot");
    assert_eq!(live.groups.len(), 1);
    assert_eq!(live.groups[0].model, model);
    assert_eq!(live.groups[0].accounts.len(), 1);
    assert_eq!(live.groups[0].accounts[0].account_id, account_id);
    assert_eq!(
        live.groups[0].accounts[0].account_display_name.as_deref(),
        Some(display_name)
    );
    assert_eq!(
        live.groups[0].accounts[0].route.cache_usage_missing_since,
        Some(cache_usage_missing_since)
    );
    assert_eq!(
        live.groups[0].accounts[0]
            .route
            .cache_usage_missing_reason
            .as_deref(),
        Some("missing_cache_input_tokens")
    );
    assert_eq!(live.records.len(), 2);
    assert!(live.records.iter().all(|record| record.kind == "attempt"));
    assert!(live.records.iter().all(|record| record.model == model));
    assert!(
        live.records
            .iter()
            .all(|record| record.account_display_name.as_deref() == Some(display_name))
    );
    let live_json = serde_json::to_value(&live).expect("serialize routing live response");
    assert!(
        live_json
            .pointer("/groups/0/accounts/0/accountGroupName")
            .is_none()
    );
    assert!(
        live_json
            .pointer("/groups/0/accounts/0/lastFailureMessage")
            .is_none()
    );
    assert!(live_json.pointer("/records/0/accountGroupName").is_none());

    let available_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Routing live API available account",
        "routing-live-api-available-key",
        None,
        None,
    )
    .await;
    observe_model_route_seen(&state.pool, available_account_id, Some(model))
        .await
        .expect("seed available API Key model route");
    insert_model_failure_attempt(
        &state,
        available_account_id,
        "routing-live-api-available-attempt",
        Some(model),
    )
    .await;
    let deleted_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Routing live API deleted account",
        "routing-live-api-deleted-key",
        None,
        None,
    )
    .await;
    observe_model_route_seen(&state.pool, deleted_account_id, Some(model))
        .await
        .expect("seed deleted API Key model route");
    insert_model_failure_attempt(
        &state,
        deleted_account_id,
        "routing-live-api-deleted-attempt",
        Some(model),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET deleted_at = datetime('now') WHERE id = ?1")
        .bind(deleted_account_id)
        .execute(&state.pool)
        .await
        .expect("soft-delete API Key routing account");

    let Json(with_deleted) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: None,
            limit: Some(100),
        }),
    )
    .await
    .expect("exclude soft-deleted API Key routing account");
    assert!(
        with_deleted
            .groups
            .iter()
            .flat_map(|group| group.accounts.iter())
            .all(|account| account.account_id != deleted_account_id)
    );
    assert!(
        with_deleted
            .records
            .iter()
            .all(|record| record.account_id != deleted_account_id)
    );

    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind(MODEL_ROUTE_STATE_DEGRADED)
    .bind(MODEL_ROUTE_PRIORITY_DEMOTED)
    .execute(&state.pool)
    .await
    .expect("mark route degraded");

    let Json(degraded) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: Some(MODEL_ROUTE_STATE_DEGRADED.to_string()),
            limit: Some(100),
        }),
    )
    .await
    .expect("filter live route decisions by current state");
    assert_eq!(degraded.groups.len(), 1);
    assert_eq!(degraded.groups[0].accounts.len(), 1);
    assert_eq!(degraded.groups[0].accounts[0].account_id, account_id);
    assert_eq!(degraded.records.len(), 2);
    assert!(
        degraded
            .records
            .iter()
            .all(|record| record.account_id == account_id)
    );

    let Json(first_page) = list_upstream_account_model_routing_events(
        State(state.clone()),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: None,
            page_size: Some(1),
        }),
    )
    .await
    .expect("load first model routing history page");
    assert_eq!(first_page.items.len(), 1);
    assert_eq!(
        first_page.items[0].account_display_name.as_deref(),
        Some(display_name)
    );
    let history_json =
        serde_json::to_value(&first_page).expect("serialize routing history response");
    assert!(history_json.pointer("/items/0/accountGroupName").is_none());
    let cursor = first_page
        .next_cursor
        .expect("two attempts should yield a next page cursor");

    let Json(second_page) = list_upstream_account_model_routing_events(
        State(state),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: Some(cursor),
            page_size: Some(1),
        }),
    )
    .await
    .expect("load second model routing history page");
    assert_eq!(second_page.items.len(), 1);
    assert_eq!(
        second_page.items[0].account_display_name.as_deref(),
        Some(display_name)
    );
    assert_ne!(first_page.items[0].id, second_page.items[0].id);
    assert!(second_page.next_cursor.is_none());
    let identifiers = [
        first_page.items[0].id.as_str(),
        second_page.items[0].id.as_str(),
    ];
    assert!(identifiers.contains(&format!("attempt:{first_attempt}").as_str()));
    assert!(identifiers.contains(&format!("attempt:{second_attempt}").as_str()));
}
