#[tokio::test]
pub(crate) async fn prompt_cache_views_ignore_sticky_only_internal_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, payload) in [
        (
            "sticky-only",
            json!({
                "stickyKey": "sticky-only",
                "upstreamScope": "internal",
                "routeMode": "pool"
            }),
        ),
        (
            "prompt-cache",
            json!({
                "promptCacheKey": "pck-real",
                "upstreamScope": "external"
            }),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            (Utc::now() - ChronoDuration::minutes(5))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert prompt cache test invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before sticky-only prompt-cache read");

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
    .expect("prompt cache conversations should succeed");
    let keys = response
        .conversations
        .iter()
        .map(|item| item.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["pck-real".to_string()]);

    let Json(list_response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let sticky_record = list_response
        .records
        .into_iter()
        .find(|record| record.invoke_id == "sticky-only")
        .expect("sticky-only record should exist");
    assert!(sticky_record.prompt_cache_key.is_none());
}

use super::*;
