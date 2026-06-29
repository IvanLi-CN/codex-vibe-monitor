use super::*;

pub(crate) async fn fetch_prompt_cache_conversations_cached(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    let started_at = Instant::now();
    loop {
        let mut wait_on: Option<watch::Receiver<bool>> = None;
        let mut flight_guard: Option<PromptCacheConversationFlightGuard> = None;
        let build_generation: u64;
        {
            let mut cache = state.prompt_cache_conversation_cache.lock().await;
            let generation = cache.generation;
            if let Some(entry) = cache.entries.get(&selection)
                && entry.generation == generation
                && entry.cached_at.elapsed()
                    <= Duration::from_secs(PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS)
            {
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                tracing::debug!(
                    endpoint = "/api/prompt-cache/conversations",
                    selection_mode = ?selection.selection_mode(),
                    selected_limit = selection.selected_limit(),
                    selected_activity_hours = selection.selected_activity_hours(),
                    selected_activity_minutes = selection.selected_activity_minutes(),
                    selected_key_count = entry.response.conversations.len() as i64,
                    row_count = entry.response.conversations.len() as i64,
                    cache_hit_or_miss = "cache_hit",
                    elapsed_ms,
                    "prompt cache conversations cache hit"
                );
                return Ok(entry.response.clone());
            }

            let in_flight_generation = cache
                .in_flight
                .get(&selection)
                .map(|flight| flight.generation);
            match in_flight_generation {
                Some(current_generation) if current_generation == generation => {
                    if let Some(in_flight) = cache.in_flight.get(&selection) {
                        wait_on = Some(in_flight.signal.subscribe());
                    }
                }
                Some(_) => {
                    cache.in_flight.remove(&selection);
                }
                None => {}
            }

            if wait_on.is_none() {
                let (signal, _receiver) = watch::channel(false);
                cache.in_flight.insert(
                    selection,
                    PromptCacheConversationInFlight { signal, generation },
                );
                build_generation = generation;
                flight_guard = Some(PromptCacheConversationFlightGuard::new(
                    state.prompt_cache_conversation_cache.clone(),
                    selection,
                    generation,
                ));
            } else {
                build_generation = generation;
            }
        }

        if let Some(mut receiver) = wait_on {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            tracing::debug!(
                endpoint = "/api/prompt-cache/conversations",
                selection_mode = ?selection.selection_mode(),
                selected_limit = selection.selected_limit(),
                selected_activity_hours = selection.selected_activity_hours(),
                selected_activity_minutes = selection.selected_activity_minutes(),
                selected_key_count = 0_i64,
                row_count = 0_i64,
                cache_hit_or_miss = "wait_on_in_flight",
                elapsed_ms,
                "prompt cache conversations waited on in-flight build"
            );
            continue;
        }

        let result = build_prompt_cache_conversations_response(state, selection).await;

        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }

        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        let stale_result = result.is_ok() && cache.generation != build_generation;
        let in_flight = match cache.in_flight.remove(&selection) {
            Some(in_flight) if in_flight.generation == build_generation => Some(in_flight),
            Some(in_flight) => {
                cache.in_flight.insert(selection, in_flight);
                None
            }
            None => None,
        };
        if let Some(in_flight) = in_flight {
            if let Ok(response) = &result {
                if !stale_result && cache.generation == build_generation {
                    cache.entries.insert(
                        selection,
                        PromptCacheConversationsCacheEntry {
                            cached_at: Instant::now(),
                            generation: build_generation,
                            response: response.clone(),
                        },
                    );
                }
            }
            let _ = in_flight.signal.send(true);
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        match &result {
            Ok(response) if elapsed_ms >= 250 => {
                tracing::warn!(
                    endpoint = "/api/prompt-cache/conversations",
                    selection_mode = ?selection.selection_mode(),
                    selected_limit = selection.selected_limit(),
                    selected_activity_hours = selection.selected_activity_hours(),
                    selected_activity_minutes = selection.selected_activity_minutes(),
                    selected_key_count = response.conversations.len() as i64,
                    row_count = response.conversations.len() as i64,
                    cache_hit_or_miss = if stale_result { "cache_miss_stale" } else { "cache_miss_build" },
                    elapsed_ms,
                    "prompt cache conversations build exceeded slow-path threshold"
                );
            }
            Ok(response) => {
                tracing::debug!(
                    endpoint = "/api/prompt-cache/conversations",
                    selection_mode = ?selection.selection_mode(),
                    selected_limit = selection.selected_limit(),
                    selected_activity_hours = selection.selected_activity_hours(),
                    selected_activity_minutes = selection.selected_activity_minutes(),
                    selected_key_count = response.conversations.len() as i64,
                    row_count = response.conversations.len() as i64,
                    cache_hit_or_miss = if stale_result { "cache_miss_stale" } else { "cache_miss_build" },
                    elapsed_ms,
                    "prompt cache conversations build completed"
                );
            }
            Err(err) => {
                tracing::warn!(
                    endpoint = "/api/prompt-cache/conversations",
                    selection_mode = ?selection.selection_mode(),
                    selected_limit = selection.selected_limit(),
                    selected_activity_hours = selection.selected_activity_hours(),
                    selected_activity_minutes = selection.selected_activity_minutes(),
                    selected_key_count = 0_i64,
                    row_count = 0_i64,
                    cache_hit_or_miss = "cache_miss_error",
                    elapsed_ms,
                    error = %err,
                    "prompt cache conversations build failed"
                );
            }
        }
        return result;
    }
}

pub(crate) fn compact_prompt_cache_conversations_response(
    mut response: PromptCacheConversationsResponse,
    recent_invocation_limit: Option<i64>,
) -> PromptCacheConversationsResponse {
    let recent_invocation_limit = recent_invocation_limit.unwrap_or(2).max(0) as usize;
    for conversation in &mut response.conversations {
        conversation.upstream_accounts.clear();
        conversation.last24h_requests.clear();
        conversation.recent_invocations.truncate(recent_invocation_limit);
    }
    response
}
