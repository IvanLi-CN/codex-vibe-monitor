use super::test_support::{
    fetch_prompt_cache_conversations, materialize_prompt_cache_hourly_rollups,
};
use super::*;
use serde_json::json;

#[tokio::test]
async fn prompt_cache_conversations_expose_persisted_master_statistics() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let key = "pck-persisted-statistics";
    let first_at = now - ChronoDuration::minutes(2);
    let last_at = now - ChronoDuration::minutes(1);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        status: &str,
        key: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_input_tokens: i64,
        reported_cache_write_tokens: i64,
        reasoning_tokens: i64,
        total_tokens: i64,
        cost: f64,
        cost_input: f64,
        cost_cache_write: f64,
        cost_cache_read: f64,
        cost_output: f64,
        cost_reasoning: f64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status,
                input_tokens, output_tokens, cache_input_tokens,
                reported_cache_write_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output,
                cost_reasoning, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_input_tokens)
        .bind(reported_cache_write_tokens)
        .bind(reasoning_tokens)
        .bind(total_tokens)
        .bind(cost)
        .bind(cost_input)
        .bind(cost_cache_write)
        .bind(cost_cache_read)
        .bind(cost_output)
        .bind(cost_reasoning)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .bind(format_utc_iso_millis(occurred_at))
        .execute(pool)
        .await
        .expect("insert persisted-statistics invocation");
    }

    insert_row(
        &state.pool,
        "pck-stats-success",
        first_at,
        "success",
        key,
        100,
        20,
        30,
        10,
        5,
        120,
        0.12,
        0.01,
        0.02,
        0.03,
        0.04,
        0.05,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-stats-failure",
        last_at,
        "failed",
        key,
        200,
        40,
        60,
        20,
        10,
        240,
        0.24,
        0.11,
        0.12,
        0.13,
        0.14,
        0.15,
    )
    .await;

    ensure_prompt_cache_conversation_row(&state.pool, key)
        .await
        .expect("create persisted-statistics conversation identity");
    let first_at_db = format_naive(first_at.with_timezone(&Shanghai).naive_local());
    let last_at_db = format_naive(last_at.with_timezone(&Shanghai).naive_local());
    let pending_aggregate = PromptCacheConversationAggregateRow {
        prompt_cache_key: key.to_owned(),
        request_count: 2,
        total_tokens: 360,
        total_cost: 0.36,
        created_at: first_at_db.clone(),
        last_activity_at: last_at_db.clone(),
        cursor_created_at: None,
        sort_anchor_at: None,
        last_terminal_at: None,
        last_in_flight_at: None,
    };
    let pending_conversation = {
        let mut connection = state
            .pool
            .acquire()
            .await
            .expect("acquire prompt-cache hydration connection");
        hydrate_prompt_cache_conversations_on_connection(
            &state,
            &mut connection,
            InvocationSourceScope::ProxyOnly,
            vec![pending_aggregate],
            last_at,
            PromptCacheConversationDetailLevel::Compact,
            None,
            None,
            &[],
        )
        .await
        .expect("delayed prompt cache conversation statistics should hydrate")
        .into_iter()
        .next()
        .expect("delayed-statistics conversation should be included")
    };
    assert!(pending_conversation.conversation_id.is_some());
    assert_eq!(pending_conversation.success_count, None);
    assert_eq!(pending_conversation.first_invocation_at, None);
    assert_eq!(pending_conversation.last_invocation_at, None);

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache conversation statistics should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .expect("persisted-statistics conversation should be included");

    assert!(conversation.conversation_id.is_some());
    assert_eq!(conversation.success_count, Some(1));
    assert_eq!(conversation.failure_count, Some(1));
    assert_eq!(conversation.input_tokens, Some(300));
    assert_eq!(conversation.output_tokens, Some(60));
    assert_eq!(conversation.cache_input_tokens, Some(90));
    assert_eq!(conversation.reported_cache_write_tokens, Some(30));
    assert_eq!(conversation.reasoning_tokens, Some(15));
    assert!((conversation.cost_input.expect("input cost") - 0.12).abs() < 1e-9);
    assert!((conversation.cost_cache_write.expect("cache-write cost") - 0.14).abs() < 1e-9);
    assert!((conversation.cost_cache_read.expect("cache-read cost") - 0.16).abs() < 1e-9);
    assert!((conversation.cost_output.expect("output cost") - 0.18).abs() < 1e-9);
    assert!((conversation.cost_reasoning.expect("reasoning cost") - 0.20).abs() < 1e-9);
    assert_eq!(
        conversation.first_invocation_at.as_deref(),
        Some(first_at_db.as_str())
    );
    assert_eq!(
        conversation.last_invocation_at.as_deref(),
        Some(last_at_db.as_str())
    );

    sqlx::query(
        "UPDATE prompt_cache_conversations SET cost_input = ?1 WHERE prompt_cache_key = ?2",
    )
    .bind(-0.02_f64)
    .bind(key)
    .execute(&state.pool)
    .await
    .expect("corrupt persisted prompt-cache cost for validation regression");
    invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    let Json(invalid_response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("invalid persisted prompt-cache statistics should still hydrate");
    let invalid_conversation = invalid_response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .expect("invalid-statistics conversation should remain visible");
    assert_eq!(invalid_conversation.success_count, None);
    assert_eq!(invalid_conversation.cost_input, None);

    let snapshot_at = first_at + ChronoDuration::seconds(30);
    let Json(snapshot_response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("snapshot prompt cache conversation statistics should succeed");
    let snapshot_conversation = snapshot_response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .expect("snapshot should retain the pre-boundary conversation");
    assert_eq!(
        snapshot_conversation.conversation_id,
        conversation.conversation_id
    );
    assert_eq!(snapshot_conversation.success_count, None);
    assert_eq!(snapshot_conversation.failure_count, None);
    assert_eq!(snapshot_conversation.first_invocation_at, None);
    assert_eq!(snapshot_conversation.last_invocation_at, None);
}

#[tokio::test]
async fn prompt_cache_conversations_hide_statistics_beyond_snapshot_row_boundary() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_second = Utc
        .timestamp_opt(Utc::now().timestamp() - 240, 0)
        .single()
        .expect("snapshot second should be valid");
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);
    let key = "pck-snapshot-statistics";

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        key: &str,
        status: &str,
        total_tokens: i64,
    ) -> i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(0.01_f64)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert snapshot statistics invocation")
        .last_insert_rowid()
    }

    let first_id = insert_row(
        &state.pool,
        "snapshot-statistics-before",
        snapshot_second,
        snapshot_second - ChronoDuration::seconds(1),
        key,
        "success",
        10,
    )
    .await;
    let second_id = insert_row(
        &state.pool,
        "snapshot-statistics-after",
        snapshot_second,
        requested_snapshot_at + ChronoDuration::milliseconds(200),
        key,
        "failed",
        20,
    )
    .await;
    assert!(second_id > first_id);

    ensure_prompt_cache_conversation_row(&state.pool, key)
        .await
        .expect("create snapshot-statistics conversation identity");
    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(requested_snapshot_at)),
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("snapshot statistics response should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .unwrap_or_else(|| {
            panic!(
                "snapshot-statistics conversation should be included; total_matched={:?}, keys={:?}",
                response.total_matched,
                response
                    .conversations
                    .iter()
                    .map(|conversation| conversation.prompt_cache_key.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 10);
    assert_eq!(conversation.success_count, None);
    assert_eq!(conversation.failure_count, None);
    assert_eq!(conversation.first_invocation_at, None);
    assert_eq!(conversation.last_invocation_at, None);
}

#[tokio::test]
async fn prompt_cache_conversations_hide_statistics_when_invocation_created_after_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::minutes(1);
    let requested_snapshot_at = snapshot_at + ChronoDuration::milliseconds(123);
    let key = "pck-invocation-created-after-snapshot";

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        key: &str,
        total_tokens: i64,
    ) -> i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, 'success', ?4, 0.01, ?5, '{}', ?6)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert interleaved snapshot invocation")
        .last_insert_rowid()
    }

    let first_id = insert_row(
        &state.pool,
        "interleaved-before-1",
        snapshot_at - ChronoDuration::seconds(30),
        snapshot_at - ChronoDuration::seconds(40),
        key,
        10,
    )
    .await;
    let post_snapshot_created_id = insert_row(
        &state.pool,
        "interleaved-after-created",
        snapshot_at - ChronoDuration::seconds(20),
        snapshot_at + ChronoDuration::seconds(1),
        key,
        20,
    )
    .await;
    let latest_visible_id = insert_row(
        &state.pool,
        "interleaved-before-2",
        snapshot_at - ChronoDuration::seconds(10),
        snapshot_at - ChronoDuration::seconds(1),
        key,
        30,
    )
    .await;
    assert!(first_id < post_snapshot_created_id);
    assert!(post_snapshot_created_id < latest_visible_id);

    ensure_prompt_cache_conversation_row(&state.pool, key)
        .await
        .expect("create interleaved snapshot conversation identity");
    sqlx::query(
        "UPDATE prompt_cache_conversations SET created_at = ?1 WHERE prompt_cache_key = ?2",
    )
    .bind(format_utc_iso_millis(
        snapshot_at - ChronoDuration::seconds(2),
    ))
    .bind(key)
    .execute(&state.pool)
    .await
    .expect("set interleaved master creation time before snapshot");
    materialize_prompt_cache_hourly_rollups(&state.pool).await;
    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(requested_snapshot_at)),
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("interleaved snapshot statistics response should succeed");
    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .expect("interleaved snapshot conversation should be included");

    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 40);
    assert_eq!(conversation.success_count, None);
    assert_eq!(conversation.failure_count, None);
    assert_eq!(conversation.first_invocation_at, None);
    assert_eq!(conversation.last_invocation_at, None);
}

#[tokio::test]
async fn prompt_cache_conversations_hide_statistics_when_master_created_after_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::minutes(5);
    let key = "pck-master-created-after-snapshot";
    let occurred_at = snapshot_at - ChronoDuration::seconds(10);

    let invocation_id = sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("master-created-after-snapshot")
    .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(json!({ "promptCacheKey": key }).to_string())
    .bind("{}")
    .bind(format_utc_iso_millis(occurred_at))
    .execute(&state.pool)
    .await
    .expect("insert pre-snapshot invocation")
    .last_insert_rowid();
    ensure_prompt_cache_conversation_row(&state.pool, key)
        .await
        .expect("create master conversation row");
    sqlx::query(
        "UPDATE prompt_cache_conversations SET created_at = ?1 WHERE prompt_cache_key = ?2",
    )
    .bind(format_utc_iso_millis(
        snapshot_at - ChronoDuration::seconds(1),
    ))
    .bind(key)
    .execute(&state.pool)
    .await
    .expect("set master row creation time before snapshot");
    materialize_prompt_cache_hourly_rollups(&state.pool).await;
    sqlx::query(
        "UPDATE prompt_cache_conversations SET created_at = ?1 WHERE prompt_cache_key = ?2",
    )
    .bind(format_utc_iso_millis(
        snapshot_at + ChronoDuration::seconds(1),
    ))
    .bind(key)
    .execute(&state.pool)
    .await
    .expect("move master row creation time beyond snapshot");

    let snapshot_upper_bound = db_occurred_at_upper_bound(snapshot_at);
    let snapshot_hour_start_epoch = align_bucket_epoch(snapshot_at.timestamp(), 3_600, 0);
    let snapshot_hour_start_bound = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_hour_start_epoch, 0)
            .single()
            .expect("snapshot hour start should be valid"),
    );
    let snapshot_created_at_upper_bound = format_utc_iso_precise(snapshot_at);
    let snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: &snapshot_upper_bound,
        snapshot_created_at_upper_bound: Some(&snapshot_created_at_upper_bound),
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound: &snapshot_hour_start_bound,
        snapshot_boundary_row_id_ceiling: Some(invocation_id),
    };
    let aggregate = PromptCacheConversationAggregateRow {
        prompt_cache_key: key.to_string(),
        request_count: 1,
        total_tokens: 12,
        total_cost: 0.12,
        created_at: format_naive(occurred_at.with_timezone(&Shanghai).naive_local()),
        last_activity_at: format_naive(occurred_at.with_timezone(&Shanghai).naive_local()),
        cursor_created_at: None,
        sort_anchor_at: None,
        last_terminal_at: Some(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        )),
        last_in_flight_at: None,
    };
    let mut connection = state
        .pool
        .acquire()
        .await
        .expect("acquire snapshot hydration connection");
    let conversations = hydrate_prompt_cache_conversations_on_connection(
        &state,
        &mut connection,
        InvocationSourceScope::ProxyOnly,
        vec![aggregate],
        snapshot_at,
        PromptCacheConversationDetailLevel::Compact,
        None,
        Some(&snapshot),
        &[],
    )
    .await
    .expect("snapshot hydration should succeed");
    let conversation = conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == key)
        .expect("pre-snapshot invocation should keep its conversation row");
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.success_count, None);
    assert_eq!(conversation.first_invocation_at, None);
}
