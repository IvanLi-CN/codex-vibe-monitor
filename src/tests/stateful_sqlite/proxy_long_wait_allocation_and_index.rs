use super::*;
use sqlx::Row;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn ensure_schema_adds_idempotent_success_attempt_route_lookup_index() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("create current schema and route lookup index");
    ensure_schema(&pool)
        .await
        .expect("rerun current schema and route lookup index");

    let index_names = sqlx::query("PRAGMA index_list('pool_upstream_request_attempts')")
        .fetch_all(&pool)
        .await
        .expect("inspect upstream attempt indexes")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        index_names
            .iter()
            .filter(|name| name == &&"idx_pool_attempts_account_model_success".to_string())
            .count(),
        1,
        "success route lookup index should be present exactly once"
    );

    let plan = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id, started_at \
         FROM pool_upstream_request_attempts \
         WHERE upstream_account_id = ?1 \
           AND request_model = ?2 COLLATE NOCASE \
           AND status = 'success' \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(1_i64)
    .bind("gpt-5.6")
    .fetch_all(&pool)
    .await
    .expect("explain successful route lookup")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>();
    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_pool_attempts_account_model_success")),
        "route lookup should use the success covering index: {plan:?}"
    );
    assert!(
        plan.iter()
            .all(|detail| !detail.contains("USE TEMP B-TREE")),
        "route lookup should not require a temporary sort: {plan:?}"
    );
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_releases_lease_after_failure() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "failed-allocator-lease";
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF',?1,?2)")
        .bind(prompt_cache_key).bind(i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY) - 1)
        .execute(&state.pool).await.unwrap();

    let error = allocate_proxy_invoke_id_with_active_lease(&state, Some(prompt_cache_key))
        .await
        .expect_err("exhausted prompt-cache sequence should fail explicitly");
    assert!(error.to_string().contains("sequence overflow"));
    let cache = state.prompt_cache_conversation_cache.lock().await;
    assert!(
        !cache
            .identity_cache
            .active_prompt_cache_keys
            .contains_key(prompt_cache_key)
    );
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_releases_lease_after_cancellation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "cancelled-allocator-lease";
    let blocker = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let allocation_state = state.clone();
    let allocation_task = tokio::spawn(async move {
        allocate_proxy_invoke_id_with_active_lease(&allocation_state, Some(prompt_cache_key)).await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let active = state
                .prompt_cache_conversation_cache
                .lock()
                .await
                .identity_cache
                .active_prompt_cache_keys
                .get(prompt_cache_key)
                .copied();
            if active == Some(1) {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("allocator should register its active lease before waiting");

    allocation_task.abort();
    let _ = allocation_task.await;
    drop(blocker);

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let active = state
                .prompt_cache_conversation_cache
                .lock()
                .await
                .identity_cache
                .active_prompt_cache_keys
                .contains_key(prompt_cache_key);
            if !active {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancelled allocator should release its active lease");
}
