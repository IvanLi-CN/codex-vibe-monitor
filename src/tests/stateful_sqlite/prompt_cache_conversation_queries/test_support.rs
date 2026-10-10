use super::*;

pub(super) async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    query: Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;
    crate::fetch_prompt_cache_conversations(State(state), query).await
}

pub(super) async fn materialize_prompt_cache_hourly_rollups(pool: &Pool<Sqlite>) {
    sync_hourly_rollups_from_live_tables(pool)
        .await
        .expect("materialize prompt-cache hourly rollups for read-only prompt-cache tests");
    complete_prompt_cache_conversation_materialization_for_test(pool).await;
}
