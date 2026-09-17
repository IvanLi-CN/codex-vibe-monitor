fn apply_working_conversation_hydration_state(
    cached: &mut CachedSubscriptionTopic,
    projection: &mut DashboardWorkingConversationsMaterializerState,
    pending_records_at_hydration_start: &BTreeMap<String, PromptCacheTopicDelta>,
    hydration_keys: &[String],
    hydrated: WorkingConversationHydratedResponses,
    total_matched: i64,
) -> Result<WorkingConversationHydrationStateUpdate, ApiError> {
    let changed_pending_keys = prompt_cache_hydration_changed_pending_keys(
        pending_records_at_hydration_start,
        &cached.prompt_cache_pending_records,
        hydration_keys,
    );
    let unresolved_eligible_delta = hydrated.iter().any(|(prompt_cache_key, response)| {
        response.is_none()
            && cached.prompt_cache_pending_records.values().any(|record| {
                record.prompt_cache_key.as_deref() == Some(prompt_cache_key.as_str())
                    && projection.delta_is_eligible(record, Utc::now())
            })
    });
    if !changed_pending_keys.is_empty() {
        cached
            .prompt_cache_pending_key_hydrations
            .extend(changed_pending_keys);
        cached.prompt_cache_key_hydration_scheduled = true;
        cached.prompt_cache_pressure_deferred = false;
        return Ok(WorkingConversationHydrationStateUpdate {
            changed: false,
            next_hydration: Some(cached.topic.clone()),
            next_materialization: None,
            recovery: None,
        });
    }
    if unresolved_eligible_delta {
        cached.dirty = true;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_key_hydration_scheduled = false;
        cached.prompt_cache_pressure_deferred = false;
        let recovery = (!cached.prompt_cache_reconcile_scheduled).then(|| {
            cached.prompt_cache_reconcile_scheduled = true;
            cached.topic.clone()
        });
        return Ok(WorkingConversationHydrationStateUpdate {
            changed: false,
            next_hydration: None,
            next_materialization: None,
            recovery,
        });
    }
    let mut changed = false;
    for (prompt_cache_key, response) in hydrated {
        cached.prompt_cache_pending_records.retain(|_, record| {
            record.prompt_cache_key.as_deref() != Some(prompt_cache_key.as_str())
        });
        cached
            .prompt_cache_pending_key_hydrations
            .remove(&prompt_cache_key);
        changed |= projection.replace_hydrated_conversation(&prompt_cache_key, response);
    }
    changed |= projection.set_total_matched(total_matched);
    let desired_visible_count = usize::try_from(total_matched.max(0))
        .unwrap_or(usize::MAX)
        .min(projection.page_size);
    cached.prompt_cache_candidate_refill_required =
        projection.visible_keys().len() < desired_visible_count;
    cached.prompt_cache_bounded_key_hydration_count = cached
        .prompt_cache_bounded_key_hydration_count
        .saturating_add(hydration_keys.len() as u64);
    cached.prompt_cache_pressure_deferred = false;
    cached.prompt_cache_key_hydration_scheduled = false;
    let next_hydration = (!cached.prompt_cache_pending_key_hydrations.is_empty()
        || cached.prompt_cache_candidate_refill_required)
        .then(|| {
            cached.prompt_cache_key_hydration_scheduled = true;
            cached.topic.clone()
        });
    let next_materialization = (!cached.prompt_cache_pending_records.is_empty()
        && !cached.prompt_cache_refresh_scheduled)
        .then(|| {
            cached.prompt_cache_refresh_scheduled = true;
            cached.topic.clone()
        });
    Ok(WorkingConversationHydrationStateUpdate {
        changed,
        next_hydration,
        next_materialization,
        recovery: None,
    })
}
