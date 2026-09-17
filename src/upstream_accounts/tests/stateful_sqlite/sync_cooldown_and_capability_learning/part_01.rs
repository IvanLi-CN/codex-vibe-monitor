use super::*;
use std::time::{Duration, Instant};

#[test]
pub(crate) fn explicit_model_failure_recognizes_standard_not_found_and_rate_limit_shapes() {
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
pub(crate) fn model_route_failure_messages_are_sanitized_before_persistence() {
    let raw = format!("\u{0000}{}", "upstream model failure ".repeat(40));
    let sanitized = sanitize_account_action_message(&raw).expect("non-empty sanitized message");
    assert!(sanitized.len() <= 240);
    assert!(!sanitized.chars().any(char::is_control));
}

pub(crate) async fn insert_model_failure_attempt(
    state: &Arc<AppState>,
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
pub(crate) async fn model_routing_timeline_queries_use_epoch_and_latest_event_indexes() {
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
    let attempt_id = insert_timeline_index_fixture(
        &state.pool,
        account_id,
        model,
        &recent_local,
        &recent_utc,
        &expired_local,
    )
    .await;

    let cutoff_epoch_ms = (Utc::now() - ChronoDuration::minutes(15)).timestamp_millis();
    assert_timeline_query_indexes(&state.pool, cutoff_epoch_ms).await;

    assert_timeline_results(
        state,
        account_id,
        model,
        attempt_id,
        recent_utc,
        expired_utc,
    )
    .await;
}

async fn insert_timeline_index_fixture(
    pool: &sqlx::SqlitePool,
    account_id: i64,
    model: &str,
    recent_local: &str,
    recent_utc: &str,
    expired_local: &str,
) -> i64 {
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
    .bind(recent_local)
    .bind(model)
    .bind(account_id)
    .fetch_one(pool)
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
    .bind(recent_local)
    .bind(attempt_id)
    .bind(model)
    .bind(recent_utc)
    .bind(recent_utc)
    .bind(expired_local)
    .execute(pool)
    .await
    .expect("insert linked and standalone timeline events");
    attempt_id
}

async fn assert_timeline_query_indexes(pool: &sqlx::SqlitePool, cutoff_epoch_ms: i64) {
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
    .fetch_all(pool)
    .await
    .expect("explain production attempt timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(attempt_plan.iter().any(|detail| {
        detail.contains(
            "SEARCH attempts USING INDEX idx_pool_upstream_request_attempts_timeline_epoch",
        )
    }));
    assert!(attempt_plan.iter().any(|detail| {
        detail.contains("SEARCH latest USING INDEX idx_pool_upstream_account_events_attempt_latest")
    }));
    assert!(
        attempt_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE"))
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
    .fetch_all(pool)
    .await
    .expect("explain production standalone event timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(event_plan.iter().any(|detail| {
        detail.contains(
            "SEARCH event USING INDEX idx_pool_upstream_account_events_timeline_unlinked_epoch",
        )
    }));
    assert!(
        event_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE"))
    );
}

async fn assert_timeline_results(
    state: Arc<AppState>,
    account_id: i64,
    model: &str,
    attempt_id: i64,
    recent_utc: String,
    expired_utc: String,
) {
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
    let attempt_key = format!("attempt:{attempt_id}");
    let attempt = live
        .records
        .iter()
        .find(|record| record.id == attempt_key)
        .expect("local-timestamp attempt remains visible");
    assert_eq!(attempt.model_route_state_after.as_deref(), Some("degraded"));
    assert!(
        live.records
            .iter()
            .any(|record| record.kind == "event" && record.occurred_at == recent_utc)
    );
    assert!(
        live.records
            .iter()
            .all(|record| record.occurred_at != expired_utc)
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
    assert_eq!(first_page.items[0].id, attempt_key);
    let cursor = first_page.next_cursor.expect("second history page exists");
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
    let cursor = second_page.next_cursor.expect("third history page exists");
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
pub(crate) async fn model_routing_live_query_stays_bounded_for_dense_recent_timeline() {
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
    let fixture = DenseTimelineFixture {
        account_id,
        model,
        now,
        timeline_rows: TIMELINE_ROWS,
        batch_size: INSERT_BATCH_SIZE,
        attempt_id_base: ATTEMPT_ID_BASE,
        linked_event_id_base: LINKED_EVENT_ID_BASE,
        standalone_event_id_base: STANDALONE_EVENT_ID_BASE,
    };
    insert_dense_timeline_attempts(&mut transaction, &fixture).await;
    insert_dense_timeline_events(&mut transaction, &fixture).await;
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

struct DenseTimelineFixture<'a> {
    account_id: i64,
    model: &'a str,
    now: chrono::DateTime<Utc>,
    timeline_rows: usize,
    batch_size: usize,
    attempt_id_base: i64,
    linked_event_id_base: i64,
    standalone_event_id_base: i64,
}

async fn insert_dense_timeline_attempts(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    fixture: &DenseTimelineFixture<'_>,
) {
    for batch_start in (0..fixture.timeline_rows).step_by(fixture.batch_size) {
        let batch_end = (batch_start + fixture.batch_size).min(fixture.timeline_rows);
        let mut attempts = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) ",
        );
        attempts.push_values(batch_start..batch_end, |mut row, index| {
            let occurred_at =
                format_utc_iso(fixture.now - ChronoDuration::seconds((index % 720) as i64));
            row.push_bind(fixture.attempt_id_base + index as i64)
                .push_bind(format!("timeline-load-attempt-{index}"))
                .push_bind(occurred_at)
                .push_bind("/v1/responses")
                .push_bind("pool")
                .push_bind(fixture.model)
                .push_bind(fixture.account_id)
                .push_bind("timeline-load")
                .push_bind(1_i64)
                .push_bind(1_i64)
                .push_bind(0_i64)
                .push_bind("success");
        });
        attempts
            .build()
            .execute(&mut **transaction)
            .await
            .expect("insert dense timeline attempts");
    }
}

async fn insert_dense_timeline_events(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    fixture: &DenseTimelineFixture<'_>,
) {
    for batch_start in (0..fixture.timeline_rows).step_by(fixture.batch_size) {
        let batch_end = (batch_start + fixture.batch_size).min(fixture.timeline_rows);
        insert_dense_linked_events(transaction, fixture, batch_start, batch_end).await;
        insert_dense_standalone_events(transaction, fixture, batch_start, batch_end).await;
    }
}

async fn insert_dense_linked_events(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    fixture: &DenseTimelineFixture<'_>,
    batch_start: usize,
    batch_end: usize,
) {
    let mut linked_events = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "INSERT INTO pool_upstream_account_events (id, account_id, occurred_at, action, source, attempt_id, model, model_route_state_before, model_route_state_after, created_at) ",
    );
    linked_events.push_values(
        (batch_start..batch_end)
            .flat_map(|attempt_index| (0..3).map(move |event_index| (attempt_index, event_index))),
        |mut row, (attempt_index, event_index)| {
            let occurred_at =
                format_utc_iso(fixture.now - ChronoDuration::seconds((attempt_index % 720) as i64));
            row.push_bind(fixture.linked_event_id_base + (attempt_index * 3 + event_index) as i64)
                .push_bind(fixture.account_id)
                .push_bind(occurred_at.clone())
                .push_bind("model_route_state_changed")
                .push_bind("call")
                .push_bind(fixture.attempt_id_base + attempt_index as i64)
                .push_bind(fixture.model)
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
        .execute(&mut **transaction)
        .await
        .expect("insert dense linked timeline events");
}

async fn insert_dense_standalone_events(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    fixture: &DenseTimelineFixture<'_>,
    batch_start: usize,
    batch_end: usize,
) {
    let mut standalone_events = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "INSERT INTO pool_upstream_account_events (id, account_id, occurred_at, action, source, attempt_id, model, model_route_state_before, model_route_state_after, created_at) ",
    );
    standalone_events.push_values(batch_start..batch_end, |mut row, index| {
        let occurred_at =
            format_utc_iso(fixture.now - ChronoDuration::seconds((index % 720) as i64));
        row.push_bind(fixture.standalone_event_id_base + index as i64)
            .push_bind(fixture.account_id)
            .push_bind(occurred_at.clone())
            .push_bind("model_route_state_changed")
            .push_bind("call")
            .push_bind(Option::<i64>::None)
            .push_bind(fixture.model)
            .push_bind("available")
            .push_bind("degraded")
            .push_bind(occurred_at);
    });
    standalone_events
        .build()
        .execute(&mut **transaction)
        .await
        .expect("insert dense standalone timeline events");
}

#[tokio::test]
pub(crate) async fn model_routing_timeline_schema_upgrade_adds_epoch_columns_and_indexes() {
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
pub(crate) async fn model_routing_live_api_lists_api_key_attempts_and_pages_account_history() {
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

    assert_live_routing_snapshot(
        &state,
        account_id,
        model,
        display_name,
        &cache_usage_missing_since,
    )
    .await;

    seed_and_assert_deleted_routing_account(&state, model).await;

    assert_degraded_routing_filter(&state, account_id, model).await;

    assert_routing_history_pages(
        state,
        account_id,
        model,
        display_name,
        first_attempt,
        second_attempt,
    )
    .await;
}

async fn assert_live_routing_snapshot(
    state: &Arc<AppState>,
    account_id: i64,
    model: &str,
    display_name: &str,
    missing_since: &str,
) {
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
    let account = &live.groups[0].accounts[0];
    assert_eq!(account.account_id, account_id);
    assert_eq!(account.account_display_name.as_deref(), Some(display_name));
    assert_eq!(
        account.route.cache_usage_missing_since.as_deref(),
        Some(missing_since)
    );
    assert_eq!(
        account.route.cache_usage_missing_reason.as_deref(),
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
}

async fn seed_and_assert_deleted_routing_account(state: &Arc<AppState>, model: &str) {
    let available_account_id = insert_test_pool_api_key_account_with_options(
        state,
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
        state,
        available_account_id,
        "routing-live-api-available-attempt",
        Some(model),
    )
    .await;
    let deleted_account_id = insert_test_pool_api_key_account_with_options(
        state,
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
        state,
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

    let Json(snapshot) = get_model_routing_live(
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
        snapshot
            .groups
            .iter()
            .flat_map(|group| group.accounts.iter())
            .all(|account| account.account_id != deleted_account_id)
    );
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| record.account_id != deleted_account_id)
    );
}

async fn assert_degraded_routing_filter(state: &Arc<AppState>, account_id: i64, model: &str) {
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
    let Json(snapshot) = get_model_routing_live(
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
    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].accounts.len(), 1);
    assert_eq!(snapshot.groups[0].accounts[0].account_id, account_id);
    assert_eq!(snapshot.records.len(), 2);
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| record.account_id == account_id)
    );
}

async fn assert_routing_history_pages(
    state: Arc<AppState>,
    account_id: i64,
    model: &str,
    display_name: &str,
    first_attempt: i64,
    second_attempt: i64,
) {
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

pub(crate) async fn enable_cache_hit_protection(state: &Arc<AppState>) {
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_low_rate_threshold_percent = 10, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit protection");
    refresh_pool_routing_runtime_cache(state)
        .await
        .expect("publish cache-hit protection settings");
}

pub(crate) async fn cache_hit_route_state(
    state: &AppState,
    account_id: i64,
    model: &str,
) -> (
    String,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
    Option<String>,
) {
    sqlx::query_as(
        "SELECT state, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache-hit route state")
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_observation_respects_sample_boundary_and_threshold() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Boundary",
        "cache-boundary-key",
        None,
        Some("https://cache-boundary.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-boundary";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    assert_cache_boundary_ignores_small_sample(&state, account_id, model).await;
    assert_cache_boundary_threshold_sample(&state, account_id, model).await;
    assert_cache_boundary_low_sample(&state, account_id, model).await;
    assert_cache_boundary_missing_usage(&state, account_id, model).await;
    assert_cache_boundary_recovers_after_valid_sample(&state, account_id, model).await;
}

async fn assert_cache_boundary_ignores_small_sample(
    state: &AppState,
    account_id: i64,
    model: &str,
) {
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_839),
        Some(0),
        8,
    )
    .await
    .expect("ignore undersized sample");
    let route = cache_hit_route_state(state, account_id, model).await;
    assert_eq!(route.1, None);
    assert_eq!(route.5, None);
}

async fn assert_cache_boundary_threshold_sample(
    state: &Arc<AppState>,
    account_id: i64,
    model: &str,
) {
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        8,
    )
    .await
    .expect("observe threshold-equal sample");
    let route = cache_hit_route_state(state, account_id, model).await;
    assert_eq!(route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.1, None);
    assert_eq!(route.5, Some(10));
}

async fn assert_cache_boundary_low_sample(state: &Arc<AppState>, account_id: i64, model: &str) {
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(383),
        8,
    )
    .await
    .expect("observe low cache-hit sample");
    let route = cache_hit_route_state(state, account_id, model).await;
    assert_eq!(route.0, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.1, Some(4));
    assert_eq!(route.2, Some(8));
    assert_eq!(route.5, Some(9));
    let visible = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load visible cache-hit route")
        .into_iter()
        .find(|candidate| candidate.model == model)
        .expect("cache-hit route is visible");
    assert_eq!(visible.cache_concurrency_limit, Some(4));
    assert_eq!(visible.cache_recovery_limit, Some(8));
    assert_eq!(visible.cache_last_hit_rate_percent, Some(9));
    assert!(!visible.probe_required);
}

async fn assert_cache_boundary_missing_usage(state: &Arc<AppState>, account_id: i64, model: &str) {
    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("constrain cache-owned route with missing usage");
    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-usage route")
        .into_iter()
        .find(|candidate| candidate.model == model)
        .expect("missing-usage route is visible");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.cache_concurrency_limit, Some(1));
    assert!(route.cache_usage_missing_since.is_some());
    assert_eq!(
        route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load constrained missing-usage limit"),
        Some(1)
    );
    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("keep the same missing-usage episode constrained");
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND action = ?2",
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count missing cache-usage events");
    assert_eq!(event_count, 1);
}

async fn assert_cache_boundary_recovers_after_valid_sample(
    state: &Arc<AppState>,
    account_id: i64,
    model: &str,
) {
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("clear missing-usage marker with a valid sample");
    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load route after valid cache observation")
        .into_iter()
        .find(|candidate| candidate.model == model)
        .expect("observed route is visible");
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());
    assert_eq!(route.cache_concurrency_limit, Some(2));
}

#[tokio::test]
pub(crate) async fn cache_usage_missing_does_not_claim_an_observation_only_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Observation Only",
        "cache-observation-only-key",
        None,
        Some("https://cache-observation-only.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-observation-only";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed observation-only route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("record a healthy cache observation");

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 1)
        .await
        .expect("ignore missing usage for observation-only route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load observation-only route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("observation-only route is visible");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.cache_last_hit_rate_percent, Some(10));
    assert_eq!(route.cache_concurrency_limit, None);
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());
    let missing_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 AND action = ?3",
    )
    .bind(account_id)
    .bind(model)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count observation-only missing usage events");
    assert_eq!(missing_event_count, 0);
}

#[tokio::test]
pub(crate) async fn disabling_cache_hit_protection_clears_only_cache_owned_route_state() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Settings Cleanup",
        "cache-settings-cleanup-key",
        None,
        Some("https://cache-settings-cleanup.example.com/backend-api/codex"),
    )
    .await;
    let cache_model = "gpt-cache-settings-cleanup";
    let missing_only_model = "gpt-cache-settings-missing-only";
    let failure_model = "gpt-cache-settings-non-cache-failure";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(cache_model))
        .await
        .expect("seed cache model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(cache_model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("create cache-owned protection state");
    observe_model_route_cache_hit(&state.pool, account_id, Some(cache_model), None, None, 1)
        .await
        .expect("mark cache-owned route usage unavailable");
    observe_model_route_seen(&state.pool, account_id, Some(missing_only_model))
        .await
        .expect("seed missing-only cache model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 4, cache_recovery_limit = 8 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(missing_only_model)
    .execute(&state.pool)
    .await
    .expect("seed cache limit without a cache failure");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(missing_only_model),
        None,
        None,
        1,
    )
    .await
    .expect("mark missing-only cache route usage unavailable");
    let missing_only_before_disable = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route before disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route is visible");
    assert_eq!(
        missing_only_before_disable.state,
        MODEL_ROUTE_STATE_DEGRADED
    );
    assert_eq!(
        missing_only_before_disable.priority,
        MODEL_ROUTE_PRIORITY_DEMOTED
    );
    assert!(missing_only_before_disable.last_failure_kind.is_none());
    assert!(
        missing_only_before_disable
            .cache_usage_missing_since
            .is_some()
    );
    observe_model_route_seen(&state.pool, account_id, Some(failure_model))
        .await
        .expect("seed independent failed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', last_failure_kind = 'upstream_transport_error', last_failure_message = 'transport failure', cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .bind((Utc::now() + chrono::Duration::seconds(30)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed non-cache cooldown");
    assert_disabling_cache_protection(
        &state,
        account_id,
        cache_model,
        missing_only_model,
        failure_model,
    )
    .await;
}

async fn assert_disabling_cache_protection(
    state: &Arc<AppState>,
    account_id: i64,
    cache_model: &str,
    missing_only_model: &str,
    failure_model: &str,
) {
    let Json(updated) = update_pool_routing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(UpdatePoolRoutingSettingsRequest {
            api_key: None,
            maintenance: None,
            request_compression_algorithm: None,
            request_compression_level_preset: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            timeouts: None,
            cache_hit_protection: Some(UpdateCacheHitProtectionSettingsRequest {
                enabled: Some(false),
                low_hit_rate_threshold_percent: None,
                overflow_mode: None,
            }),
            priority_handoff_admission_enabled: None,
        }),
    )
    .await
    .expect("disable cache-hit protection");
    assert!(!updated.cache_hit_protection.enabled);
    let cache_route = cache_hit_route_state(state, account_id, cache_model).await;
    assert_eq!(cache_route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(cache_route.1, None);
    assert_eq!(cache_route.2, None);
    assert_eq!(cache_route.3, 0);
    assert_eq!(cache_route.4, 0);
    assert_eq!(cache_route.5, None);
    let cache_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load cache route after disabling protection")
        .into_iter()
        .find(|route| route.model == cache_model)
        .expect("cache route is visible");
    assert!(cache_route.cache_usage_missing_since.is_none());
    assert!(cache_route.cache_usage_missing_reason.is_none());
    let missing_only_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route after disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route remains visible");
    assert_eq!(missing_only_route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(missing_only_route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(missing_only_route.cache_concurrency_limit, None);
    assert_eq!(missing_only_route.cache_recovery_limit, None);
    assert!(missing_only_route.cache_usage_missing_since.is_none());
    assert!(missing_only_route.cache_usage_missing_reason.is_none());
    let non_cache_route = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT state, priority, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .fetch_one(&state.pool)
    .await
    .expect("load non-cache cooldown after cache settings update");
    assert_eq!(non_cache_route.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(non_cache_route.1, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert!(non_cache_route.2.is_some());
    let cleanup_event = sqlx::query_as::<_, (String, String, String)>(
        "SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(cache_model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache settings cleanup event");
    assert_eq!(cleanup_event.0, UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET);
    assert_eq!(cleanup_event.1, "cache_hit_protection_disabled");
    assert_eq!(cleanup_event.2, cache_model);
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_concurrency_halves_then_recovers() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Concurrency",
        "cache-concurrency-key",
        None,
        Some("https://cache-concurrency.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-concurrency";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    for expected_limit in [4, 2, 1] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            8,
        )
        .await
        .expect("apply low cache-hit protection");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    assert_eq!(cache_hit_route_state(&state, account_id, model).await.3, 1);

    for expected_limit in [2, 3, 4, 5, 6, 7] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(3_840),
            8,
        )
        .await
        .expect("apply healthy cache-hit observation");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(3_840),
        8,
    )
    .await
    .expect("fully recover cache-hit route");
    let recovered = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(recovered.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(recovered.1, None);
    assert_eq!(recovered.2, None);
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_serializes_concurrent_observations() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Serialized Observations",
        "cache-serialized-observations-key",
        None,
        Some("https://cache-serialized-observations.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-serialized-observations";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    let first = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let second = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let (first, second) = tokio::join!(first, second);
    first.expect("first low-hit observation should persist");
    second.expect("second low-hit observation should persist");

    let route = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(route.1, Some(2));
    assert_eq!(route.2, Some(8));
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_atomically_reserves_single_model_slot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reservation",
        "cache-reservation-key",
        None,
        Some("https://cache-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-reservation";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    assert_single_cache_model_reservation(&state, account_id, model).await;
}

async fn assert_single_cache_model_reservation(
    state: &Arc<AppState>,
    account_id: i64,
    model: &str,
) {
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("limit the combination to one request");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load model concurrency limit"),
        Some(1)
    );
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("switch cache-hit overflow mode to reroute");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("publish cache-hit overflow mode");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first selection should complete"),
        second.expect("second selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );
    let audit = resolutions
        .iter()
        .find_map(|resolution| match resolution {
            PoolAccountResolution::NoCandidate(audit) => Some(audit),
            _ => None,
        })
        .expect("capacity conflict should retain a no-candidate audit");
    assert_eq!(audit.terminal_reason_code, "modelConcurrencyLimit");
    assert_eq!(audit.candidate_count, 1);
    assert_eq!(audit.eligible_candidate_count, 1);
    assert_eq!(audit.reservation_conflict_count, 1);
    assert_eq!(audit.candidates[0].reason_code, "modelConcurrencyLimit");

    release_pool_routing_reservation(state, "cache-hit-reservation-a");
    release_pool_routing_reservation(state, "cache-hit-reservation-b");
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_reserves_sticky_fast_path_before_returning_it() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reservation",
        "cache-sticky-reservation-key",
        None,
        Some("https://cache-sticky-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-sticky-reservation";
    let sticky_key = "cache-hit-sticky-reservation";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("seed sticky model concurrency limit");
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first sticky selection should complete"),
        second.expect("second sticky selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );

    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-b");
}
