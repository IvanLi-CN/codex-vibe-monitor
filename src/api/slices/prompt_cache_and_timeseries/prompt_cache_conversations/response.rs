use super::*;

pub(crate) async fn build_prompt_cache_conversations_response_for_request(
    state: &AppState,
    request: PromptCacheConversationsRequest,
) -> Result<PromptCacheConversationsResponse, ApiError> {
    if request.page_size.is_none() && request.cursor.is_none() && request.snapshot_at.is_none() {
        let response = build_prompt_cache_conversations_response(state, request.selection)
            .await
            .map_err(ApiError::from)?;
        return Ok(match request.detail_level {
            PromptCacheConversationDetailLevel::Full => response,
            PromptCacheConversationDetailLevel::Compact => {
                compact_prompt_cache_conversations_response(response)
            }
        });
    }

    let selection = request.selection;
    let PromptCacheConversationSelection::ActivityWindowMinutes(_) = selection else {
        return Err(ApiError::bad_request(anyhow!(
            "paginated prompt cache conversations only support activityMinutes working conversations"
        )));
    };
    let page_size = request.page_size.unwrap_or(20);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_at = resolve_prompt_cache_conversation_snapshot_at(request.snapshot_at.as_deref())
        .map_err(ApiError::bad_request)?;
    let range_end = snapshot_at;
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let snapshot_hour_start_epoch = align_bucket_epoch(range_end.timestamp(), 3_600, 0);
    let snapshot_hour_start_bound = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_hour_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid snapshot hour start epoch"))?,
    );
    let cursor = request
        .cursor
        .as_deref()
        .map(decode_prompt_cache_conversation_cursor)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let snapshot_filter = resolve_prompt_cache_conversation_snapshot_filter(
        &state.pool,
        snapshot_at,
        source_scope,
        cursor
            .as_ref()
            .and_then(|(_, _, _, snapshot_boundary_row_id_ceiling)| {
                *snapshot_boundary_row_id_ceiling
            }),
    )
    .await?;
    let mut aggregates = query_prompt_cache_working_conversation_aggregates_page(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        snapshot_hour_start_epoch,
        &snapshot_hour_start_bound,
        source_scope,
        cursor.as_ref(),
        page_size + 1,
    )
    .await?;
    let has_more = aggregates.len() as i64 > page_size;
    if has_more {
        aggregates.truncate(page_size as usize);
    }
    let total_matched = query_working_prompt_cache_conversation_count_at_snapshot(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        source_scope,
    )
    .await?;
    let next_cursor = if has_more {
        aggregates.last().map(|row| {
            build_prompt_cache_conversation_cursor(
                row,
                snapshot_filter.snapshot_boundary_row_id_ceiling,
            )
        })
    } else {
        None
    };
    let row_cursors_by_key = aggregates
        .iter()
        .map(|row| {
            (
                row.prompt_cache_key.clone(),
                build_prompt_cache_conversation_cursor(
                    row,
                    snapshot_filter.snapshot_boundary_row_id_ceiling,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let hydration_snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: snapshot_filter.snapshot_upper_bound(),
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound: &snapshot_hour_start_bound,
        snapshot_boundary_row_id_ceiling: snapshot_filter.snapshot_boundary_row_id_ceiling,
    };
    let mut conversations = hydrate_prompt_cache_conversations(
        state,
        source_scope,
        aggregates,
        range_end,
        request.detail_level,
        Some(&hydration_snapshot),
    )
    .await?;
    for conversation in &mut conversations {
        conversation.cursor = row_cursors_by_key
            .get(&conversation.prompt_cache_key)
            .cloned();
    }

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso_precise(range_end),
        snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        selected_activity_minutes: selection.selected_activity_minutes(),
        implicit_filter: PromptCacheConversationImplicitFilter {
            kind: None,
            filtered_count: 0,
        },
        total_matched: Some(total_matched),
        has_more,
        next_cursor,
        conversations,
    })
}

pub(crate) async fn build_prompt_cache_conversations_response(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let display_limit = selection.display_limit();

    let (aggregates, active_filtered_count) = match selection {
        PromptCacheConversationSelection::Count(limit) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let filtered_count = query_prompt_cache_conversation_hidden_count(
                &state.pool,
                &range_start_bound,
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
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_active_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(_) => {
            let aggregates = query_prompt_cache_working_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_working_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
    };
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
        source_scope,
        aggregates,
        range_end,
        PromptCacheConversationDetailLevel::Full,
        None,
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

