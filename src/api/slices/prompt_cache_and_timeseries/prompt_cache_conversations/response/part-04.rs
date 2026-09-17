pub(crate) async fn build_prompt_cache_conversations_response(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    build_prompt_cache_conversations_response_with_recent_limit(state, selection, None).await
}

async fn query_recent_prompt_cache_conversation_selection(
    state: &AppState,
    selection: PromptCacheConversationSelection,
    display_limit: i64,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    runtime_overlay_records: &[ApiInvocation],
) -> Result<(Vec<PromptCacheConversationAggregateRow>, i64)> {
    Ok(match selection {
        PromptCacheConversationSelection::Count(limit) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let filtered_count = query_prompt_cache_conversation_hidden_count(
                &state.pool,
                range_start_bound,
                source_scope,
                limit,
                aggregates.len() as i64,
            )
            .await?;
            (aggregates, filtered_count)
        }
        PromptCacheConversationSelection::ActivityWindowHours(_) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_active_prompt_cache_conversation_count(
                &state.pool,
                range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(_) => {
            let aggregates = query_prompt_cache_working_conversation_aggregates(
                &state.pool,
                range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let mut aggregates = merge_runtime_prompt_cache_aggregates(
                aggregates,
                runtime_overlay_records,
                None,
                display_limit,
            );
            apply_prompt_cache_lifecycle_aggregate_totals(
                &state.pool,
                source_scope,
                &mut aggregates,
                runtime_overlay_records,
                None,
                None,
                None,
            )
            .await?;
            let matched_count = query_working_prompt_cache_conversation_count(
                &state.pool,
                range_start_bound,
                source_scope,
            )
            .await?;
            let runtime_overlay_keys = runtime_prompt_cache_overlay_keys(runtime_overlay_records);
            let existing_runtime_overlay_keys =
                query_existing_working_prompt_cache_conversation_keys(
                    &state.pool,
                    range_start_bound,
                    source_scope,
                    &runtime_overlay_keys,
                    None,
                )
                .await?;
            (
                aggregates,
                (matched_count
                    + runtime_overlay_keys
                        .difference(&existing_runtime_overlay_keys)
                        .count() as i64)
                    .saturating_sub(display_limit),
            )
        }
    })
}

pub(crate) async fn build_prompt_cache_conversations_response_with_recent_limit(
    state: &AppState,
    selection: PromptCacheConversationSelection,
    recent_invocation_limit: Option<i64>,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let display_limit = selection.display_limit();
    let runtime_overlay_records =
        runtime_prompt_cache_overlay_records(state, source_scope, &range_start_bound, None);

    let (aggregates, active_filtered_count) = query_recent_prompt_cache_conversation_selection(
        state,
        selection,
        display_limit,
        &range_start_bound,
        source_scope,
        &runtime_overlay_records,
    )
    .await?;
    let implicit_filter = selection.implicit_filter(active_filtered_count);

    if aggregates.is_empty() {
        return Ok(PromptCacheConversationsResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            snapshot_at: None,
            selection_mode: selection.selection_mode(),
            selected_limit: selection.selected_limit(),
            selected_activity_hours: selection.selected_activity_hours(),
            selected_activity_minutes: selection.selected_activity_minutes(),
            implicit_filter,
            total_matched: None,
            has_more: false,
            next_cursor: None,
            conversations: Vec::new(),
        });
    }
    let conversations = hydrate_prompt_cache_conversations(
        state,
        PromptCacheConversationHydrationRequest {
            source_scope,
            aggregates,
            range_end,
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit,
            snapshot: None,
            runtime_overlay_records: &runtime_overlay_records,
        },
    )
    .await?;

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        snapshot_at: None,
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        selected_activity_minutes: selection.selected_activity_minutes(),
        implicit_filter,
        total_matched: None,
        has_more: false,
        next_cursor: None,
        conversations,
    })
}
