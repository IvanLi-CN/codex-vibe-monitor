struct BoundedWorkingConversationHydration {
    snapshot_at: DateTime<Utc>,
    snapshot_filter: PromptCacheConversationSnapshotFilter,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: String,
    transient_runtime_overlay_records: Vec<ApiInvocation>,
    aggregates: Vec<PromptCacheConversationAggregateRow>,
}

async fn load_bounded_working_conversation_hydration(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    prompt_cache_key: &str,
    range_start_bound: &str,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
    runtime_overlay_records: &[ApiInvocation],
) -> Result<BoundedWorkingConversationHydration, ApiError> {
    let hydration_snapshot_at = Utc::now();
    let snapshot_hour_start_epoch = align_bucket_epoch(hydration_snapshot_at.timestamp(), 3_600, 0);
    let snapshot_hour_start_bound = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_hour_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bounded hydration hour start epoch"))?,
    );
    let snapshot_filter = resolve_prompt_cache_conversation_snapshot_filter(
        &mut *connection,
        hydration_snapshot_at,
        source_scope,
        None,
    )
    .await?;
    let transient_runtime_overlay_records =
        transient_runtime_prompt_cache_overlay_records_on_connection(
            &mut *connection,
            source_scope,
            Some(&snapshot_filter),
            runtime_overlay_records,
        )
        .await?;
    let mut aggregates = query_prompt_cache_working_conversation_lifecycle_aggregate_for_key(
        &mut *connection,
        PromptCacheWorkingConversationLifecycleQuery {
            range_start_bound,
            source_scope,
            prompt_cache_key,
            blocked_binding_filter,
            snapshot_filter: &snapshot_filter,
            snapshot_hour_start_epoch,
            snapshot_hour_start_bound: &snapshot_hour_start_bound,
        },
    )
    .await?
    .into_iter()
    .collect::<Vec<_>>();
    if aggregates.is_empty() {
        aggregates = transient_runtime_overlay_records
            .iter()
            .filter_map(runtime_prompt_cache_aggregate_from_record)
            .collect();
    }
    Ok(BoundedWorkingConversationHydration {
        snapshot_at: hydration_snapshot_at,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
        transient_runtime_overlay_records,
        aggregates,
    })
}

pub(crate) async fn hydrate_working_prompt_cache_conversation_for_key(
    state: &AppState,
    source_scope: InvocationSourceScope,
    prompt_cache_key: &str,
    _range_end: DateTime<Utc>,
    range_start_bound: &str,
    recent_invocation_limit: i64,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> Result<Option<PromptCacheConversationResponse>, ApiError> {
    // A bounded key hydrate repairs the current in-memory projection. Pin every durable read to
    // one transaction-local boundary so a P1 commit cannot be present in the working aggregate
    // yet absent from its details after the runtime overlay is deduplicated.
    let runtime_overlay_records = runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        range_start_bound,
        blocked_binding_filter,
    )
    .into_iter()
    .filter(|record| record.prompt_cache_key.as_deref() == Some(prompt_cache_key))
    .collect::<Vec<_>>();
    let mut transaction = state.pool.begin().await?;
    // P1 can persist a new terminal before P2 refreshes the working-set row. Always use the
    // snapshot-bounded lifecycle aggregate and then add only the selected runtime tail.
    let bounded = load_bounded_working_conversation_hydration(
        transaction.as_mut(),
        source_scope,
        prompt_cache_key,
        range_start_bound,
        blocked_binding_filter,
        &runtime_overlay_records,
    )
    .await?;
    let BoundedWorkingConversationHydration {
        snapshot_at: hydration_snapshot_at,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
        transient_runtime_overlay_records,
        mut aggregates,
    } = bounded;
    if aggregates.is_empty() {
        transaction.commit().await?;
        return Ok(None);
    }
    apply_prompt_cache_lifecycle_aggregate_totals_on_connection(
        transaction.as_mut(),
        source_scope,
        &mut aggregates,
        &transient_runtime_overlay_records,
        Some(&snapshot_filter),
        Some(snapshot_hour_start_epoch),
        Some(&snapshot_hour_start_bound),
    )
    .await?;
    let hydration_snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: snapshot_filter.snapshot_upper_bound(),
        snapshot_created_at_upper_bound: snapshot_filter.snapshot_created_at_upper_bound(),
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound: &snapshot_hour_start_bound,
        snapshot_boundary_row_id_ceiling: snapshot_filter.snapshot_boundary_row_id_ceiling,
    };
    let response = hydrate_prompt_cache_conversations_on_connection(
        state,
        transaction.as_mut(),
        PromptCacheConversationHydrationRequest {
            source_scope,
            aggregates,
            range_end: hydration_snapshot_at,
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(recent_invocation_limit),
            snapshot: Some(&hydration_snapshot),
            runtime_overlay_records: &transient_runtime_overlay_records,
        },
    )
    .await?
    .into_iter()
    .next();
    transaction.commit().await?;
    Ok(response)
}

pub(crate) async fn query_working_prompt_cache_conversation_candidate_keys(
    state: &AppState,
    source_scope: InvocationSourceScope,
    _range_end: DateTime<Utc>,
    range_start_bound: &str,
    page_size: i64,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> Result<Vec<String>, ApiError> {
    let mut connection = state.pool.acquire().await?;
    let aggregates = query_prompt_cache_working_conversation_aggregates_page(
        &mut *connection,
        PromptCacheWorkingConversationPageQuery {
            range_start_bound,
            source_scope,
            blocked_binding_filter,
            cursor: None,
            limit: page_size.max(0),
        },
    )
    .await?;
    let runtime_overlay_records = runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        range_start_bound,
        blocked_binding_filter,
    );
    let aggregates = merge_runtime_prompt_cache_aggregates(
        aggregates,
        &runtime_overlay_records,
        None,
        page_size.max(0),
    );
    Ok(aggregates
        .into_iter()
        .map(|aggregate| aggregate.prompt_cache_key)
        .collect())
}

pub(crate) async fn query_working_prompt_cache_conversation_total_matched(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    range_start_bound: &str,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
    additional_visible_keys: &HashSet<String>,
) -> Result<i64, ApiError> {
    let snapshot = PromptCacheConversationSnapshotFilter {
        snapshot_upper_bound: format_utc_iso_precise(range_end),
        snapshot_created_at_upper_bound: None,
        snapshot_boundary_row_id_ceiling: None,
    };
    let runtime_overlay_records = runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        range_start_bound,
        blocked_binding_filter,
    );
    let mut visible_keys = runtime_prompt_cache_overlay_keys(&runtime_overlay_records);
    visible_keys.extend(additional_visible_keys.iter().cloned());
    let mut connection = state.pool.acquire().await?;
    let db_total_matched = query_working_prompt_cache_conversation_count_at_snapshot(
        &mut *connection,
        range_start_bound,
        &snapshot,
        source_scope,
        blocked_binding_filter,
    )
    .await?;
    let existing_visible_keys = query_existing_working_prompt_cache_conversation_keys(
        &mut *connection,
        range_start_bound,
        source_scope,
        &visible_keys,
        blocked_binding_filter,
    )
    .await?;
    Ok(db_total_matched + visible_keys.difference(&existing_visible_keys).count() as i64)
}
