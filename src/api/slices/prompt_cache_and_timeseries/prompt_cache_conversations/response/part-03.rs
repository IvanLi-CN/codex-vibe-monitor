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

    let snapshot_at = resolve_prompt_cache_conversation_snapshot_at(request.snapshot_at.as_deref())
        .map_err(ApiError::bad_request)?;
    let mut transaction = state.pool.begin().await?;
    let response = build_prompt_cache_conversations_response_for_request_on_connection(
        state,
        request,
        transaction.as_mut(),
        snapshot_at,
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(response)
}

pub(crate) async fn build_prompt_cache_conversations_response_for_request_on_connection(
    state: &AppState,
    request: PromptCacheConversationsRequest,
    connection: &mut SqliteConnection,
    snapshot_at: DateTime<Utc>,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> Result<PromptCacheConversationsResponse, ApiError> {
    let (response, _) = build_prompt_cache_conversations_response_for_request_with_runtime_overlay_terminal_identities_on_connection(
        state,
        request,
        connection,
        snapshot_at,
        snapshot_boundary_row_id_ceiling,
    )
    .await?;
    Ok(response)
}

struct PaginatedConversationInputs {
    selection: PromptCacheConversationSelection,
    page_size: i64,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    range_start_bound: String,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: String,
    cursor: Option<(String, String, String, Option<i64>)>,
}

fn resolve_paginated_conversation_inputs(
    request: &PromptCacheConversationsRequest,
    snapshot_at: DateTime<Utc>,
) -> Result<PaginatedConversationInputs, ApiError> {
    let selection = request.selection;
    if !matches!(
        selection,
        PromptCacheConversationSelection::ActivityWindowMinutes(_)
    ) {
        return Err(ApiError::bad_request(anyhow!(
            "paginated prompt cache conversations only support activityMinutes working conversations"
        )));
    }
    let range_start = snapshot_at - selection.activity_window_duration();
    let snapshot_hour_start_epoch = align_bucket_epoch(snapshot_at.timestamp(), 3_600, 0);
    let snapshot_hour_start_bound = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_hour_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid snapshot hour start epoch"))?,
    );
    Ok(PaginatedConversationInputs {
        selection,
        page_size: request.page_size.unwrap_or(20),
        range_start,
        range_end: snapshot_at,
        range_start_bound: db_occurred_at_lower_bound(range_start),
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
        cursor: request
            .cursor
            .as_deref()
            .map(decode_prompt_cache_conversation_cursor)
            .transpose()
            .map_err(ApiError::bad_request)?,
    })
}

struct PaginatedConversationData {
    snapshot_filter: PromptCacheConversationSnapshotFilter,
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    runtime_overlay_records: Vec<ApiInvocation>,
    transient_runtime_overlay_records: Vec<ApiInvocation>,
    has_more: bool,
    total_matched: i64,
    next_cursor: Option<String>,
    row_cursors_by_key: HashMap<String, String>,
}

async fn query_paginated_conversation_total_matched(
    connection: &mut SqliteConnection,
    inputs: &PaginatedConversationInputs,
    request: &PromptCacheConversationsRequest,
    snapshot_filter: &PromptCacheConversationSnapshotFilter,
    source_scope: InvocationSourceScope,
    runtime_overlay_records: &[ApiInvocation],
) -> Result<i64, ApiError> {
    let db_total_matched = query_working_prompt_cache_conversation_count_at_snapshot(
        &mut *connection,
        &inputs.range_start_bound,
        snapshot_filter,
        source_scope,
        request.blocked_binding_filter.as_ref(),
    )
    .await?;
    let runtime_overlay_keys = runtime_prompt_cache_overlay_keys(runtime_overlay_records);
    let existing_runtime_overlay_keys = query_existing_working_prompt_cache_conversation_keys(
        &mut *connection,
        &inputs.range_start_bound,
        source_scope,
        &runtime_overlay_keys,
        request.blocked_binding_filter.as_ref(),
    )
    .await?;
    Ok(db_total_matched
        + runtime_overlay_keys
            .difference(&existing_runtime_overlay_keys)
            .count() as i64)
}

fn build_paginated_conversation_cursors(
    aggregates: &[PromptCacheConversationAggregateRow],
    snapshot_boundary_row_id_ceiling: Option<i64>,
    has_more: bool,
) -> (Option<String>, HashMap<String, String>) {
    let next_cursor = has_more
        .then(|| {
            aggregates.last().map(|row| {
                build_prompt_cache_conversation_cursor(row, snapshot_boundary_row_id_ceiling)
            })
        })
        .flatten();
    let row_cursors_by_key = aggregates
        .iter()
        .map(|row| {
            (
                row.prompt_cache_key.clone(),
                build_prompt_cache_conversation_cursor(row, snapshot_boundary_row_id_ceiling),
            )
        })
        .collect();
    (next_cursor, row_cursors_by_key)
}

async fn load_paginated_conversation_data(
    state: &AppState,
    connection: &mut SqliteConnection,
    request: &PromptCacheConversationsRequest,
    inputs: &PaginatedConversationInputs,
    source_scope: InvocationSourceScope,
    snapshot_at: DateTime<Utc>,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> Result<PaginatedConversationData, ApiError> {
    let snapshot_filter = resolve_prompt_cache_conversation_snapshot_filter(
        &mut *connection,
        snapshot_at,
        source_scope,
        snapshot_boundary_row_id_ceiling.or_else(|| {
            inputs
                .cursor
                .as_ref()
                .and_then(|(_, _, _, boundary)| *boundary)
        }),
    )
    .await?;
    let db_page_limit = inputs.page_size + 1;
    let aggregates = query_prompt_cache_working_conversation_aggregates_page(
        &mut *connection,
        PromptCacheWorkingConversationPageQuery {
            range_start_bound: &inputs.range_start_bound,
            source_scope,
            blocked_binding_filter: request.blocked_binding_filter.as_ref(),
            cursor: inputs.cursor.as_ref(),
            limit: db_page_limit,
        },
    )
    .await?;
    let runtime_overlay_records = runtime_prompt_cache_overlay_records_at_snapshot(
        state,
        source_scope,
        &inputs.range_start_bound,
        request.blocked_binding_filter.as_ref(),
        snapshot_at,
    );
    let mut aggregates = merge_runtime_prompt_cache_aggregates(
        aggregates,
        &runtime_overlay_records,
        inputs.cursor.as_ref(),
        db_page_limit,
    );
    let transient_runtime_overlay_records =
        transient_runtime_prompt_cache_overlay_records_on_connection(
            &mut *connection,
            source_scope,
            Some(&snapshot_filter),
            &runtime_overlay_records,
        )
        .await?;
    apply_prompt_cache_lifecycle_aggregate_totals_on_connection(
        &mut *connection,
        source_scope,
        &mut aggregates,
        &transient_runtime_overlay_records,
        Some(&snapshot_filter),
        Some(inputs.snapshot_hour_start_epoch),
        Some(&inputs.snapshot_hour_start_bound),
    )
    .await?;
    let has_more = aggregates.len() as i64 > inputs.page_size;
    if has_more {
        aggregates.truncate(inputs.page_size as usize);
    }
    let total_matched = query_paginated_conversation_total_matched(
        &mut *connection,
        inputs,
        request,
        &snapshot_filter,
        source_scope,
        &runtime_overlay_records,
    )
    .await?;
    let (next_cursor, row_cursors_by_key) = build_paginated_conversation_cursors(
        &aggregates,
        snapshot_filter.snapshot_boundary_row_id_ceiling,
        has_more,
    );
    Ok(PaginatedConversationData {
        snapshot_filter,
        aggregates,
        runtime_overlay_records,
        transient_runtime_overlay_records,
        has_more,
        total_matched,
        next_cursor,
        row_cursors_by_key,
    })
}

pub(crate) async fn build_prompt_cache_conversations_response_for_request_with_runtime_overlay_terminal_identities_on_connection(
    state: &AppState,
    request: PromptCacheConversationsRequest,
    connection: &mut SqliteConnection,
    snapshot_at: DateTime<Utc>,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> Result<(PromptCacheConversationsResponse, HashSet<String>), ApiError> {
    let inputs = resolve_paginated_conversation_inputs(&request, snapshot_at)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let page = load_paginated_conversation_data(
        state,
        connection,
        &request,
        &inputs,
        source_scope,
        snapshot_at,
        snapshot_boundary_row_id_ceiling,
    )
    .await?;
    let PaginatedConversationData {
        snapshot_filter,
        aggregates,
        runtime_overlay_records,
        transient_runtime_overlay_records,
        has_more,
        total_matched,
        next_cursor,
        row_cursors_by_key,
    } = page;
    let hydration_snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: snapshot_filter.snapshot_upper_bound(),
        snapshot_created_at_upper_bound: snapshot_filter.snapshot_created_at_upper_bound(),
        snapshot_hour_start_epoch: inputs.snapshot_hour_start_epoch,
        snapshot_hour_start_bound: &inputs.snapshot_hour_start_bound,
        snapshot_boundary_row_id_ceiling: snapshot_filter.snapshot_boundary_row_id_ceiling,
    };
    let mut conversations = hydrate_prompt_cache_conversations_on_connection(
        state,
        &mut *connection,
        PromptCacheConversationHydrationRequest {
            source_scope,
            aggregates,
            range_end: inputs.range_end,
            detail_level: request.detail_level,
            recent_invocation_limit: request.recent_invocation_limit,
            snapshot: Some(&hydration_snapshot),
            runtime_overlay_records: &transient_runtime_overlay_records,
        },
    )
    .await?;
    for conversation in &mut conversations {
        conversation.cursor = row_cursors_by_key
            .get(&conversation.prompt_cache_key)
            .cloned();
    }

    let response = PromptCacheConversationsResponse {
        range_start: format_utc_iso(inputs.range_start),
        range_end: format_utc_iso_precise(inputs.range_end),
        snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
        selection_mode: inputs.selection.selection_mode(),
        selected_limit: inputs.selection.selected_limit(),
        selected_activity_hours: inputs.selection.selected_activity_hours(),
        selected_activity_minutes: inputs.selection.selected_activity_minutes(),
        implicit_filter: PromptCacheConversationImplicitFilter {
            kind: None,
            filtered_count: 0,
        },
        total_matched: Some(total_matched),
        has_more,
        next_cursor,
        conversations,
    };
    let runtime_overlay_terminal_identities = runtime_overlay_records
        .iter()
        .filter(|record| !prompt_cache_runtime_record_is_in_flight(record))
        .map(|record| format!("{}\0{}", record.invoke_id, record.occurred_at))
        .collect();
    Ok((response, runtime_overlay_terminal_identities))
}
