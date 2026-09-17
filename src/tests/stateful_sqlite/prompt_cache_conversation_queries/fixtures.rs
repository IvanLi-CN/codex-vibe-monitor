use super::*;

pub(crate) async fn prompt_cache_test_state() -> Arc<AppState> {
    test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await
}

pub(crate) fn prompt_cache_query() -> PromptCacheConversationsQuery {
    PromptCacheConversationsQuery {
        limit: None,
        activity_hours: None,
        activity_minutes: None,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    }
}

pub(crate) async fn fetch_prompt_cache_test_response(
    state: Arc<AppState>,
    query: PromptCacheConversationsQuery,
) -> PromptCacheConversationsResponse {
    let Json(response) = fetch_prompt_cache_conversations(State(state), Query(query))
        .await
        .expect("prompt cache conversation query should succeed");
    response
}

pub(crate) fn prompt_cache_conversation<'a>(
    response: &'a PromptCacheConversationsResponse,
    key: &str,
) -> &'a PromptCacheConversationResponse {
    response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == key)
        .unwrap_or_else(|| panic!("prompt cache conversation {key} should be included"))
}
