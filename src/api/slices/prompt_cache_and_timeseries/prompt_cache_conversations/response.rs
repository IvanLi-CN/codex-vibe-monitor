use super::*;
use anyhow::anyhow;

pub(crate) fn prompt_cache_runtime_record_source_matches(
    record: &ApiInvocation,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope == InvocationSourceScope::All || record.source == SOURCE_PROXY
}

pub(crate) fn prompt_cache_runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        record
            .status
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

pub(crate) fn prompt_cache_runtime_record_is_in_working_window(
    record: &ApiInvocation,
    range_start_bound: &str,
) -> bool {
    prompt_cache_runtime_record_is_in_flight(record)
        || parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
            db_occurred_at_lower_bound(occurred_at).as_str() >= range_start_bound
        })
}

pub(crate) fn prompt_cache_runtime_record_matches_blocked_binding_filter(
    record: &ApiInvocation,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> bool {
    let Some(blocked_binding_filter) = blocked_binding_filter else {
        return true;
    };
    if !blocked_binding_filter.is_active() {
        return true;
    }
    let Some(blocked_binding) = record.blocked_binding.as_ref() else {
        return false;
    };
    blocked_binding_filter
        .upstream_account_id
        .is_none_or(|value| blocked_binding.upstream_account_id == value)
        && blocked_binding_filter
            .constraint_source
            .is_none_or(|value| blocked_binding.constraint_source == value)
}

pub(crate) fn prompt_cache_runtime_record_sort_anchor(record: &ApiInvocation) -> String {
    record.occurred_at.clone()
}

pub(crate) fn max_optional_timestamp(
    left: Option<String>,
    right: Option<String>,
) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) fn merge_runtime_prompt_cache_aggregate(
    aggregate: &mut PromptCacheConversationAggregateRow,
    runtime: PromptCacheConversationAggregateRow,
) {
    aggregate.request_count += runtime.request_count;
    aggregate.total_tokens += runtime.total_tokens;
    aggregate.total_cost += runtime.total_cost;
    if runtime.created_at < aggregate.created_at {
        aggregate.created_at = runtime.created_at;
    }
    if runtime.last_activity_at > aggregate.last_activity_at {
        aggregate.last_activity_at = runtime.last_activity_at;
    }
    aggregate.sort_anchor_at =
        max_optional_timestamp(aggregate.sort_anchor_at.take(), runtime.sort_anchor_at);
    aggregate.last_terminal_at =
        max_optional_timestamp(aggregate.last_terminal_at.take(), runtime.last_terminal_at);
    aggregate.last_in_flight_at = max_optional_timestamp(
        aggregate.last_in_flight_at.take(),
        runtime.last_in_flight_at,
    );
}

pub(crate) fn runtime_prompt_cache_aggregate_from_record(
    record: &ApiInvocation,
) -> Option<PromptCacheConversationAggregateRow> {
    let prompt_cache_key = record.prompt_cache_key.as_deref()?.trim();
    if prompt_cache_key.is_empty() {
        return None;
    }
    let occurred_at = record.occurred_at.clone();
    let is_in_flight = prompt_cache_runtime_record_is_in_flight(record);
    Some(PromptCacheConversationAggregateRow {
        prompt_cache_key: prompt_cache_key.to_string(),
        request_count: 1,
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        total_cost: record.cost.unwrap_or_default(),
        created_at: occurred_at.clone(),
        last_activity_at: occurred_at.clone(),
        cursor_created_at: Some(occurred_at.clone()),
        sort_anchor_at: Some(prompt_cache_runtime_record_sort_anchor(record)),
        last_terminal_at: (!is_in_flight).then(|| occurred_at.clone()),
        last_in_flight_at: is_in_flight.then_some(occurred_at),
    })
}

pub(crate) fn runtime_prompt_cache_overlay_records(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_start_bound: &str,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> Vec<ApiInvocation> {
    state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .filter(|record| prompt_cache_runtime_record_source_matches(record, source_scope))
        .filter(|record| {
            prompt_cache_runtime_record_is_in_working_window(record, range_start_bound)
        })
        .filter(|record| {
            record
                .prompt_cache_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
        })
        .filter(|record| {
            prompt_cache_runtime_record_matches_blocked_binding_filter(
                record,
                blocked_binding_filter,
            )
        })
        .collect()
}

pub(crate) fn runtime_prompt_cache_overlay_keys(
    runtime_overlay_records: &[ApiInvocation],
) -> HashSet<String> {
    runtime_overlay_records
        .iter()
        .filter_map(|record| {
            record
                .prompt_cache_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
        })
        .collect()
}

pub(crate) fn merge_runtime_prompt_cache_aggregates(
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    runtime_overlay_records: &[ApiInvocation],
    cursor: Option<&(String, String, String, Option<i64>)>,
    limit: i64,
) -> Vec<PromptCacheConversationAggregateRow> {
    if runtime_overlay_records.is_empty() {
        return aggregates;
    }

    let mut rows_by_key = aggregates
        .into_iter()
        .map(|row| (row.prompt_cache_key.clone(), row))
        .collect::<HashMap<_, _>>();
    for record in runtime_overlay_records {
        let Some(runtime) = runtime_prompt_cache_aggregate_from_record(record) else {
            continue;
        };
        if let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor
        {
            let sort_anchor_at = runtime
                .sort_anchor_at
                .as_deref()
                .unwrap_or(&runtime.last_activity_at);
            let cursor_sort_anchor_at = cursor_sort_anchor_at.as_str();
            let cursor_created_at = cursor_created_at.as_str();
            let cursor_prompt_cache_key = cursor_prompt_cache_key.as_str();
            let is_after_cursor = sort_anchor_at < cursor_sort_anchor_at
                || (sort_anchor_at == cursor_sort_anchor_at
                    && (runtime.created_at.as_str() < cursor_created_at
                        || (runtime.created_at.as_str() == cursor_created_at
                            && runtime.prompt_cache_key.as_str() < cursor_prompt_cache_key)));
            if !is_after_cursor {
                continue;
            }
        }
        match rows_by_key.entry(runtime.prompt_cache_key.clone()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_runtime_prompt_cache_aggregate(entry.get_mut(), runtime);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(runtime);
            }
        }
    }

    let mut rows = rows_by_key.into_values().collect::<Vec<_>>();
    sort_prompt_cache_working_aggregates(&mut rows);
    rows.truncate(limit.max(0) as usize);
    rows
}

pub(crate) async fn apply_prompt_cache_lifecycle_aggregate_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    aggregates: &mut [PromptCacheConversationAggregateRow],
    runtime_overlay_records: &[ApiInvocation],
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_epoch: Option<i64>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<()> {
    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let lifecycle_aggregates = query_prompt_cache_conversation_lifecycle_aggregates(
        pool,
        source_scope,
        &selected_keys,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    )
    .await?;
    let selected_key_set = selected_keys.into_iter().collect::<HashSet<_>>();
    let mut runtime_aggregates_by_key = HashMap::new();
    for record in runtime_overlay_records {
        let Some(runtime) = runtime_prompt_cache_aggregate_from_record(record) else {
            continue;
        };
        if !selected_key_set.contains(&runtime.prompt_cache_key) {
            continue;
        }
        match runtime_aggregates_by_key.entry(runtime.prompt_cache_key.clone()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_runtime_prompt_cache_aggregate(entry.get_mut(), runtime);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(runtime);
            }
        }
    }
    for aggregate in aggregates {
        let mut applied_lifecycle = false;
        if let Some(lifecycle) = lifecycle_aggregates.get(&aggregate.prompt_cache_key) {
            aggregate.request_count = lifecycle.request_count;
            aggregate.total_tokens = lifecycle.total_tokens;
            aggregate.total_cost = lifecycle.total_cost;
            aggregate.created_at = lifecycle.created_at.clone();
            aggregate.last_activity_at = lifecycle.last_activity_at.clone();
            applied_lifecycle = true;
        }
        if applied_lifecycle
            && let Some(runtime) = runtime_aggregates_by_key.remove(&aggregate.prompt_cache_key)
        {
            merge_runtime_prompt_cache_aggregate(aggregate, runtime);
        }
    }
    Ok(())
}

pub(crate) fn sort_prompt_cache_working_aggregates(
    rows: &mut [PromptCacheConversationAggregateRow],
) {
    rows.sort_by(|left, right| {
        let left_sort = left
            .sort_anchor_at
            .as_deref()
            .unwrap_or(&left.last_activity_at);
        let right_sort = right
            .sort_anchor_at
            .as_deref()
            .unwrap_or(&right.last_activity_at);
        let left_cursor_created = left
            .cursor_created_at
            .as_deref()
            .unwrap_or(&left.created_at);
        let right_cursor_created = right
            .cursor_created_at
            .as_deref()
            .unwrap_or(&right.created_at);
        right_sort
            .cmp(left_sort)
            .then_with(|| right_cursor_created.cmp(left_cursor_created))
            .then_with(|| right.prompt_cache_key.cmp(&left.prompt_cache_key))
    });
}

pub(crate) async fn build_prompt_cache_conversations_response_for_request(
    state: &AppState,
    request: PromptCacheConversationsRequest,
) -> Result<PromptCacheConversationsResponse, ApiError> {
    if request.page_size.is_none() && request.cursor.is_none() && request.snapshot_at.is_none() {
        let response = build_prompt_cache_conversations_response_with_recent_limit(
            state,
            request.selection,
            request.recent_invocation_limit,
        )
        .await
        .map_err(ApiError::from)?;
        return Ok(match request.detail_level {
            PromptCacheConversationDetailLevel::Full => response,
            PromptCacheConversationDetailLevel::Compact => {
                compact_prompt_cache_conversations_response(
                    response,
                    request.recent_invocation_limit,
                )
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
    let db_page_limit = page_size + 1;
    let aggregates = query_prompt_cache_working_conversation_aggregates_page(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        snapshot_hour_start_epoch,
        &snapshot_hour_start_bound,
        source_scope,
        request.blocked_binding_filter.as_ref(),
        cursor.as_ref(),
        db_page_limit,
    )
    .await?;
    let runtime_overlay_records = runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        &range_start_bound,
        request.blocked_binding_filter.as_ref(),
    );
    let mut aggregates = merge_runtime_prompt_cache_aggregates(
        aggregates,
        &runtime_overlay_records,
        cursor.as_ref(),
        db_page_limit,
    );
    apply_prompt_cache_lifecycle_aggregate_totals(
        &state.pool,
        source_scope,
        &mut aggregates,
        &runtime_overlay_records,
        Some(&snapshot_filter),
        Some(snapshot_hour_start_epoch),
        Some(&snapshot_hour_start_bound),
    )
    .await?;
    let has_more = aggregates.len() as i64 > page_size;
    if has_more {
        aggregates.truncate(page_size as usize);
    }
    let db_total_matched = query_working_prompt_cache_conversation_count_at_snapshot(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        source_scope,
        request.blocked_binding_filter.as_ref(),
    )
    .await?;
    let runtime_overlay_keys = runtime_prompt_cache_overlay_keys(&runtime_overlay_records);
    let existing_runtime_overlay_keys = query_existing_working_prompt_cache_conversation_keys(
        &state.pool,
        &range_start_bound,
        source_scope,
        &runtime_overlay_keys,
        request.blocked_binding_filter.as_ref(),
    )
    .await?;
    let total_matched = db_total_matched
        + runtime_overlay_keys
            .difference(&existing_runtime_overlay_keys)
            .count() as i64;
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
        snapshot_created_at_upper_bound: snapshot_filter.snapshot_created_at_upper_bound(),
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
        request.recent_invocation_limit,
        Some(&hydration_snapshot),
        &runtime_overlay_records,
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
    build_prompt_cache_conversations_response_with_recent_limit(state, selection, None).await
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
            let mut aggregates = merge_runtime_prompt_cache_aggregates(
                aggregates,
                &runtime_overlay_records,
                None,
                display_limit,
            );
            apply_prompt_cache_lifecycle_aggregate_totals(
                &state.pool,
                source_scope,
                &mut aggregates,
                &runtime_overlay_records,
                None,
                None,
                None,
            )
            .await?;
            let matched_count = query_working_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            let runtime_overlay_keys = runtime_prompt_cache_overlay_keys(&runtime_overlay_records);
            let existing_runtime_overlay_keys =
                query_existing_working_prompt_cache_conversation_keys(
                    &state.pool,
                    &range_start_bound,
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
        recent_invocation_limit,
        None,
        &runtime_overlay_records,
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
