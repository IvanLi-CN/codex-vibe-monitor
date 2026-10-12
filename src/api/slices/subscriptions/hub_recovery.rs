use super::*;

impl SubscriptionHub {
    pub(crate) async fn handle_runtime_mutation_batch(
        &self,
        state: Arc<AppState>,
        mutations: Vec<SequencedRuntimeMutation>,
    ) {
        let received_count = mutations.len();
        let mutations = coalesce_runtime_mutations(mutations);
        if mutations.is_empty() {
            return;
        }
        self.runtime_mutation_bus
            .record_router_batch(received_count, mutations.len());
        self.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        for mutation in &mutations {
            match &mutation.mutation {
                RuntimeMutation::PromptCacheBindingChanged { prompt_cache_key } => {
                    if let Err(err) = self
                        .apply_prompt_cache_binding_projection(state.clone(), prompt_cache_key)
                        .await
                    {
                        warn!(
                            ?err,
                            prompt_cache_key,
                            "failed to apply bounded prompt cache binding projection"
                        );
                    }
                }
                RuntimeMutation::StickyRouteChanged {
                    sticky_key,
                    previous_upstream_account_id,
                    upstream_account_id,
                } => {
                    if let Err(err) = self
                        .apply_prompt_cache_sticky_route_projection(
                            state.clone(),
                            sticky_key,
                            *previous_upstream_account_id,
                            *upstream_account_id,
                        )
                        .await
                    {
                        warn!(
                            ?err,
                            sticky_key, "failed to apply prompt cache sticky route projection"
                        );
                    }
                }
                RuntimeMutation::AccountEffectiveRoutingRulesChanged {
                    version,
                    upserts,
                    removed_account_ids,
                } => {
                    self.apply_account_effective_routing_rules_change(
                        version,
                        upserts,
                        removed_account_ids,
                    )
                    .await;
                }
                RuntimeMutation::Invocation(_)
                | RuntimeMutation::AttemptChanged { .. }
                | RuntimeMutation::ModelRoutingChanged => {}
            }
        }

        // The dependency index contains only active selections. Disconnection removes a topic
        // from the index and leaves its retained frame dirty, so the router never scans retained
        // caches or clones a runtime event into the hot path.
        let affected = {
            let guard = self.state.lock().await;
            Self::collect_runtime_topic_work(&guard, &mutations)
        };
        self.runtime_mutation_bus.record_topic_work(affected.len());

        for work in affected {
            // Dashboard activity, summary, and network are materialized by their dedicated
            // runtime projection slices. A generic mutation must never send them back through
            // the DB-backed topic builder.
            if work.topic.uses_summary_live_overlay()
                || work.topic.uses_dashboard_activity_live_overlay()
                || work.topic.uses_timeseries_live_projection()
                || work.topic.uses_dashboard_network_live_snapshot()
            {
                continue;
            }
            if work.topic.uses_parallel_work_live_projection() {
                if let Err(err) = self
                    .schedule_parallel_work_topic_projection(
                        state.clone(),
                        work.topic.clone(),
                        &mutations,
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule parallel-work runtime projection"
                    );
                }
                continue;
            }
            if work.topic.uses_summary_topic_refresh() && work.terminal_event_count > 0 {
                if let Err(err) = self
                    .schedule_summary_topic_refresh(
                        state.clone(),
                        work.topic.clone(),
                        work.terminal_event_count,
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule summary topic refresh"
                    );
                }
                continue;
            }
            if work.topic.uses_conversation_overview_refresh() && work.includes_invocation_mutation
            {
                if let Err(err) = self
                    .schedule_conversation_overview_topic_refresh(state.clone(), work.topic.clone())
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule conversation overview topic refresh"
                    );
                }
                continue;
            }
            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), work.topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %work.topic.name(),
                    "failed to refresh subscription topic after typed runtime mutation"
                );
            }
        }
    }

    pub(crate) async fn mark_runtime_mutation_gap_and_recover(
        &self,
        state: Arc<AppState>,
        skipped: u64,
        reason: &'static str,
    ) {
        let (active_topic_count, recovery_scheduled, account_attempt_topics) = {
            let mut guard = self.state.lock().await;
            guard.runtime_topic_recovery_generation =
                guard.runtime_topic_recovery_generation.saturating_add(1);
            let recovery_generation = guard.runtime_topic_recovery_generation;
            let active_topic_keys = guard
                .active_topics
                .iter()
                .filter(|(topic_key, _)| {
                    guard
                        .active_subscribers
                        .get(*topic_key)
                        .copied()
                        .unwrap_or_default()
                        > 0
                })
                .map(|(topic_key, _)| topic_key.clone())
                .collect::<Vec<_>>();
            let mut account_attempt_topics = Vec::new();
            for topic_key in &active_topic_keys {
                let Some(cached) = guard.topics.get_mut(topic_key) else {
                    continue;
                };
                cached.dirty = true;
                cached.refresh_scheduled = false;
                cached.invalidate_upstream_account_attempt_refresh();
                cached.latest_live_snapshot = None;
                cached.continuity_reset_cursor = Some(cached.cursor);
                cached.runtime_topic_recovery_generation = recovery_generation;
                cached.runtime_topic_recovery_retry_at = None;
                if cached.topic.uses_upstream_account_attempt_refresh() {
                    account_attempt_topics.push(cached.topic.clone());
                }
                if matches!(
                    cached.topic,
                    SubscriptionTopic::PromptCacheWindow { .. }
                        | SubscriptionTopic::PromptCacheStickyWindow { .. }
                        | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) {
                    // Prompt Cache keeps its last-good frame. Its server-push reconciler performs
                    // the bounded cold rebuild later instead of doing a full window build from
                    // this cursor-gap handler.
                    cached.prompt_cache_pending_records.clear();
                    cached.prompt_cache_refresh_scheduled = false;
                    cached.prompt_cache_reconcile_required = true;
                    cached.prompt_cache_pressure_deferred = false;
                }
            }
            let recovery_scheduled = Self::enqueue_runtime_topic_recovery_locked(&mut guard);
            (
                active_topic_keys.len(),
                recovery_scheduled,
                account_attempt_topics,
            )
        };
        warn!(
            skipped,
            reason,
            recovery = "dirty_last_good",
            active_topic_count,
            "runtime mutation cursor continuity lost; scheduling bounded topic recovery"
        );
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            let recovery_state = state.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(recovery_state).await;
            });
        }
        for topic in account_attempt_topics {
            if let Err(err) = self
                .schedule_upstream_account_attempt_topic_refresh(state.clone(), topic.clone())
                .await
            {
                warn!(
                    ?err,
                    topic = %topic.name(),
                    recovery = "dirty_last_good",
                    "failed to schedule account attempt topic after runtime mutation gap"
                );
            }
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    pub(crate) async fn invalidate_active_upstream_account_attempt_topics_for_recovery(
        &self,
    ) -> Vec<SubscriptionTopic> {
        let mut guard = self.state.lock().await;
        guard.runtime_topic_recovery_generation =
            guard.runtime_topic_recovery_generation.saturating_add(1);
        let recovery_generation = guard.runtime_topic_recovery_generation;
        let active_subscribers = guard.active_subscribers.clone();
        guard
            .topics
            .iter_mut()
            .filter_map(|(topic_key, cached)| {
                if !cached.topic.uses_upstream_account_attempt_refresh()
                    || active_subscribers
                        .get(topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                {
                    return None;
                }

                // Retain the last-good frame and reject any older build before the dedicated
                // account-attempt builder publishes its next authoritative snapshot.
                cached.dirty = true;
                cached.refresh_scheduled = false;
                cached.invalidate_upstream_account_attempt_refresh();
                cached.latest_live_snapshot = None;
                cached.continuity_reset_cursor = Some(cached.cursor);
                cached.runtime_topic_recovery_generation = recovery_generation;
                cached.runtime_topic_recovery_retry_at = None;
                Some(cached.topic.clone())
            })
            .collect()
    }

    pub(crate) async fn schedule_invalidated_upstream_account_attempt_topics(
        &self,
        state: Arc<AppState>,
        account_attempt_topics: Vec<SubscriptionTopic>,
        recovery_source: &'static str,
    ) {
        for topic in account_attempt_topics {
            if let Err(err) = self
                .schedule_upstream_account_attempt_topic_refresh(state.clone(), topic.clone())
                .await
            {
                warn!(
                    ?err,
                    topic = %topic.name(),
                    recovery_source,
                    recovery = "dirty_last_good",
                    "failed to schedule recovered account attempt topic"
                );
            }
        }
    }

    pub(crate) async fn mark_internal_broadcast_gap_and_recover(
        &self,
        state: Arc<AppState>,
        skipped: u64,
    ) {
        let account_attempt_topics = self
            .invalidate_active_upstream_account_attempt_topics_for_recovery()
            .await;

        warn!(
            skipped,
            account_attempt_topic_count = account_attempt_topics.len(),
            recovery = "dirty_last_good",
            "subscription broadcast continuity lost; scheduling account attempt topic recovery"
        );
        self.schedule_invalidated_upstream_account_attempt_topics(
            state,
            account_attempt_topics,
            "broadcast_gap",
        )
        .await;
    }

    pub(crate) async fn schedule_dirty_topic_recovery(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
        if topic.uses_upstream_account_attempt_refresh() {
            if let Err(err) = self
                .schedule_upstream_account_attempt_topic_refresh(state, topic.clone())
                .await
            {
                warn!(
                    ?err,
                    topic = %topic.name(),
                    recovery = "dirty_last_good",
                    "failed to schedule account attempt topic recovery"
                );
            }
            return;
        }
        if matches!(
            &topic,
            SubscriptionTopic::PromptCacheWindow { .. }
                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
        ) {
            if self
                .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                .await
            {
                Self::spawn_prompt_cache_topic_reconcile(state, topic);
            }
            return;
        }

        let recovery_scheduled = {
            let mut guard = self.state.lock().await;
            Self::enqueue_runtime_topic_recovery_locked(&mut guard)
        };
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    pub(crate) fn next_runtime_topic_recovery_retry_delay_locked(
        guard: &SubscriptionHubState,
    ) -> Option<Duration> {
        let now = Instant::now();
        guard
            .active_topics
            .iter()
            .filter_map(|(topic_key, topic)| {
                if guard
                    .active_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                    || matches!(
                        topic,
                        SubscriptionTopic::PromptCacheWindow { .. }
                            | SubscriptionTopic::PromptCacheStickyWindow { .. }
                            | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    )
                {
                    return None;
                }
                guard.topics.get(topic_key).and_then(|cached| {
                    cached
                        .dirty
                        .then_some(cached.runtime_topic_recovery_retry_at)?
                })
            })
            .filter(|retry_at| *retry_at > now)
            .map(|retry_at| retry_at.duration_since(now))
            .min()
    }

    pub(crate) fn enqueue_runtime_topic_recovery_locked(guard: &mut SubscriptionHubState) -> bool {
        let active_topic_keys = guard
            .active_topics
            .keys()
            .filter(|topic_key| {
                guard
                    .active_subscribers
                    .get(*topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0
            })
            .cloned()
            .collect::<Vec<_>>();
        for topic_key in active_topic_keys {
            if guard.runtime_topic_recovery_queue.len() >= RUNTIME_TOPIC_RECOVERY_QUEUE_CAPACITY {
                break;
            }
            if guard.runtime_topic_recovery_queued.contains(&topic_key) {
                continue;
            }
            let Some(cached) = guard.topics.get(&topic_key) else {
                continue;
            };
            if !cached.dirty
                || cached
                    .runtime_topic_recovery_retry_at
                    .is_some_and(|retry_at| retry_at > Instant::now())
            {
                continue;
            }
            if matches!(
                cached.topic,
                SubscriptionTopic::PromptCacheWindow { .. }
                    | SubscriptionTopic::PromptCacheStickyWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            ) || cached.topic.uses_upstream_account_attempt_refresh()
            {
                continue;
            }
            let recovery_generation = cached.runtime_topic_recovery_generation;
            guard
                .runtime_topic_recovery_queued
                .insert(topic_key.clone());
            guard
                .runtime_topic_recovery_queue
                .push_back((topic_key, recovery_generation));
        }
        if guard.runtime_topic_recovery_running || guard.runtime_topic_recovery_queue.is_empty() {
            return false;
        }
        guard.runtime_topic_recovery_running = true;
        true
    }

    pub(crate) async fn run_runtime_topic_recovery(self: Arc<Self>, state: Arc<AppState>) {
        loop {
            let (topics, retry_delay) = {
                let mut guard = self.state.lock().await;
                let mut topics = Vec::with_capacity(RUNTIME_TOPIC_RECOVERY_BATCH_SIZE);
                while topics.len() < RUNTIME_TOPIC_RECOVERY_BATCH_SIZE {
                    if guard.runtime_topic_recovery_queue.is_empty() {
                        Self::enqueue_runtime_topic_recovery_locked(&mut guard);
                    }
                    let Some((topic_key, recovery_generation)) =
                        guard.runtime_topic_recovery_queue.pop_front()
                    else {
                        break;
                    };
                    guard.runtime_topic_recovery_queued.remove(&topic_key);
                    if guard
                        .active_subscribers
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                    {
                        continue;
                    }
                    let Some(cached) = guard.topics.get(&topic_key) else {
                        continue;
                    };
                    if cached.dirty
                        && cached.runtime_topic_recovery_generation == recovery_generation
                    {
                        topics.push(cached.topic.clone());
                    }
                }
                let retry_delay = (topics.is_empty()
                    && guard.runtime_topic_recovery_queue.is_empty())
                .then(|| Self::next_runtime_topic_recovery_retry_delay_locked(&guard))
                .flatten();
                if topics.is_empty()
                    && guard.runtime_topic_recovery_queue.is_empty()
                    && retry_delay.is_none()
                {
                    guard.runtime_topic_recovery_running = false;
                }
                (topics, retry_delay)
            };
            if topics.is_empty() {
                if let Some(delay) = retry_delay {
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.runtime_topic_recovery_notify.notified() => {}
                    }
                    continue;
                }
                return;
            }
            for topic in topics {
                match self
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    Err(err) => {
                        self.defer_runtime_topic_recovery_retry(&topic).await;
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            recovery = "dirty_last_good",
                            "bounded runtime mutation recovery retained last-good topic frame"
                        );
                    }
                    Ok(_) if self.parallel_work_reconcile_pending(&topic).await => {
                        self.defer_runtime_topic_recovery_retry(&topic).await;
                    }
                    Ok(_) => {}
                }
            }
            tokio::task::yield_now().await;
        }
    }

    pub(crate) async fn defer_runtime_topic_recovery_retry(
        &self,
        topic: &SubscriptionTopic,
    ) -> Duration {
        let Ok(topic_key) = topic.cache_key() else {
            return RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF;
        };
        let mut guard = self.state.lock().await;
        let has_pending_mutations = guard
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|mutations| !mutations.is_empty());
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            if has_pending_mutations {
                cached.dirty = true;
            }
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
        }
        RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF
    }

    pub(crate) async fn handle_internal_broadcast(
        &self,
        state: Arc<AppState>,
        payload: BroadcastPayload,
    ) {
        match payload {
            BroadcastPayload::DashboardNetworkSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_network_slice(*slice).await;
                } else {
                    self.handle_dashboard_network_slice(state, slice).await;
                }
                return;
            }
            BroadcastPayload::DashboardCurrentSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_current_slice(*slice).await;
                }
                return;
            }
            BroadcastPayload::DashboardActivityLive { .. }
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto =>
            {
                return;
            }
            BroadcastPayload::DashboardTerminalSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_terminal_slice(*slice).await;
                }
                return;
            }
            BroadcastPayload::PoolAttemptsSnapshotUnavailable { invoke_id } => {
                let account_attempt_topics = self
                    .invalidate_active_upstream_account_attempt_topics_for_recovery()
                    .await;
                warn!(
                    %invoke_id,
                    account_attempt_topic_count = account_attempt_topics.len(),
                    recovery = "dirty_last_good",
                    "pool attempt snapshot unavailable; scheduling active account topic recovery"
                );
                self.schedule_invalidated_upstream_account_attempt_topics(
                    state,
                    account_attempt_topics,
                    "snapshot_unavailable",
                )
                .await;
                return;
            }
            _ => {}
        }
        let affected = {
            let mut guard = self.state.lock().await;
            let active_subscribers = guard.active_subscribers.clone();
            guard
                .topics
                .values_mut()
                .filter(|cached| cached.topic.is_affected_by(&payload))
                .filter_map(|cached| {
                    let topic_key = cached.topic.cache_key().ok()?;
                    let active = active_subscribers
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        > 0;
                    if !active {
                        cached.dirty = true;
                        cached.latest_live_snapshot = None;
                        return None;
                    }
                    if cached.dirty
                        && !(cached.topic.uses_upstream_account_attempt_refresh()
                            && matches!(payload, BroadcastPayload::PoolAttempts { .. }))
                    {
                        // Runtime cursor recovery owns this selection. Retain last-good until
                        // its bounded work commits instead of allowing another broadcast path
                        // to publish a partial frame.
                        return None;
                    }
                    if matches!(payload, BroadcastPayload::DashboardActivityLive { .. })
                        && (cached.topic.uses_summary_live_overlay()
                            || cached.topic.uses_dashboard_activity_live_overlay())
                        && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
                    {
                        cached.latest_live_snapshot = Some(snapshot.as_ref().clone());
                    }
                    if matches!(payload, BroadcastPayload::DashboardActivityLive { .. }) {
                        self.dashboard_topology_counters
                            .record_business_payload(cached.topic.name());
                    }
                    Some(cached.clone())
                })
                .collect::<Vec<_>>()
        };

        for cached in affected {
            if cached.topic.uses_summary_live_overlay()
                && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
            {
                if let Err(err) = self
                    .apply_summary_live_overlay(&cached.topic, snapshot.as_ref().clone())
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to apply summary live overlay"
                    );
                }
                continue;
            }

            if cached.topic.uses_dashboard_activity_live_overlay()
                && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
            {
                if let Err(err) = self
                    .apply_dashboard_activity_live_overlay(
                        state.clone(),
                        &cached.topic,
                        snapshot.as_ref().clone(),
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to apply dashboard activity live overlay"
                    );
                }
                continue;
            }

            if cached.topic.uses_upstream_account_attempt_refresh()
                && matches!(payload, BroadcastPayload::PoolAttempts { .. })
            {
                if let Err(err) = self
                    .schedule_upstream_account_attempt_topic_refresh(
                        state.clone(),
                        cached.topic.clone(),
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to schedule upstream account attempt topic refresh"
                    );
                }
                continue;
            }

            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), cached.topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %cached.topic.name(),
                    "failed to refresh subscription topic"
                );
            }
        }
    }

    pub(crate) async fn handle_dashboard_network_slice(
        &self,
        state: Arc<AppState>,
        slice: Box<DashboardNetworkProjectionSlice>,
    ) {
        let topics = {
            let guard = self.state.lock().await;
            guard
                .topics
                .values()
                .filter(|cached| {
                    matches!(
                        cached.topic,
                        SubscriptionTopic::DashboardActivityCurrent { .. }
                            | SubscriptionTopic::DashboardNetworkTimeseriesWindow { .. }
                    )
                })
                .filter(|cached| {
                    cached
                        .topic
                        .cache_key()
                        .ok()
                        .and_then(|key| guard.active_subscribers.get(&key).copied())
                        .unwrap_or_default()
                        > 0
                })
                .map(|cached| cached.topic.clone())
                .collect::<Vec<_>>()
        };

        for topic in topics {
            self.dashboard_topology_counters
                .record_business_payload(topic.name());
            let result = match &topic {
                SubscriptionTopic::DashboardActivityCurrent { .. } => {
                    if let Some(live) = state
                        .proxy_runtime_invocations
                        .legacy_live_snapshot_for_network(slice.as_ref())
                    {
                        self.apply_dashboard_activity_live_overlay(state.clone(), &topic, live)
                            .await
                    } else {
                        Ok(())
                    }
                }
                SubscriptionTopic::DashboardNetworkTimeseriesWindow { .. } => {
                    self.apply_dashboard_network_slice_to_timeseries(&topic, slice.as_ref())
                        .await
                }
                _ => Ok(()),
            };
            if let Err(err) = result {
                warn!(?err, topic = %topic.name(), "failed to apply dashboard network slice");
            }
        }
    }
}
pub(crate) fn summary_delta_workload_identity(delta: &DashboardActivityTerminalDelta) -> String {
    format!(
        "{}\0{}\0{}\0{}",
        delta.persisted_row_id.unwrap_or_default(),
        delta.invoke_id,
        delta.occurred_at,
        delta.terminal_sequence,
    )
}

fn buffer_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let pending_is_empty = {
        let pending = state
            .parallel_work_prebaseline_mutations
            .entry(topic_key.to_string())
            .or_default();
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            let identity = format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            );
            if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
                pending.remove(&identity);
            } else {
                pending.insert(identity, mutation.clone());
            }
        }
        pending.is_empty()
    };
    if pending_is_empty {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

fn remove_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let Some(pending) = state.parallel_work_prebaseline_mutations.get_mut(topic_key) else {
        return;
    };
    for mutation in mutations {
        let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
            continue;
        };
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            pending.remove(&format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            ));
        }
    }
    if pending.is_empty() {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

pub(crate) fn rolling_dashboard_window_requires_rebase(
    base_start: Option<DateTime<Utc>>,
    current_start: Option<DateTime<Utc>>,
) -> bool {
    match (base_start, current_start) {
        (Some(base_start), Some(current_start)) => {
            current_start < base_start
                || current_start - base_start
                    >= ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64)
        }
        (base_start, current_start) => base_start != current_start,
    }
}

pub(crate) fn mark_dashboard_terminal_window_rebase_topics(guard: &mut SubscriptionHubState) {
    let active_subscribers = guard.active_subscribers.clone();
    for (topic_key, cached) in &mut guard.topics {
        if cached.dirty
            || !cached
                .dashboard_materializer
                .as_ref()
                .is_some_and(DashboardTopicMaterializer::requires_terminal_window_rebase)
        {
            continue;
        }
        cached.dirty = true;
        cached.refresh_scheduled = active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        cached.latest_live_snapshot = None;
    }
}

pub(crate) fn collect_pending_dashboard_topic_materializations(
    guard: &mut SubscriptionHubState,
) -> Vec<PendingDashboardTopicMaterialization> {
    let active_subscribers = guard.active_subscribers.clone();
    let server_push_subscribers = guard.server_push_subscribers.clone();
    let current = guard.dashboard_current_slice.clone();
    let network = guard.dashboard_network_slice.clone();
    let terminal = guard.dashboard_terminal_slice.clone();
    guard
        .topics
        .iter_mut()
        .filter_map(|(topic_key, cached)| {
            let active = active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                > 0
                || server_push_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0;
            if !active {
                cached.dirty = true;
                return None;
            }
            if cached.dirty {
                return None;
            }
            let materializer = cached.dashboard_materializer.as_ref()?;
            let revision = materializer.revision(
                cached.dashboard_base_revision,
                current.as_deref(),
                network.as_deref(),
                terminal.as_deref(),
            )?;
            (cached.dashboard_materialized_revision != Some(revision)).then(|| {
                PendingDashboardTopicMaterialization {
                    topic_key: topic_key.clone(),
                    topic_name: cached.topic.name(),
                    revision,
                    materializer: materializer.clone(),
                }
            })
        })
        .collect()
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_managed_task_refresh_topic(topic: &SubscriptionTopic) -> bool {
    matches!(
        topic,
        SubscriptionTopic::ManagedTaskCatalog
            | SubscriptionTopic::ManagedTaskRuntime
            | SubscriptionTopic::ManagedTaskTimeline
            | SubscriptionTopic::ManagedTaskTimelineV2
            | SubscriptionTopic::ManagedTaskDetail { .. }
            | SubscriptionTopic::ManagedTaskWorkload { .. }
    )
}

impl SubscriptionHub {
    pub(crate) async fn mark_topic_dirty(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.conversation_overview_refresh_scheduled = false;
            cached.conversation_overview_refresh_in_flight = false;
            cached.conversation_overview_refresh_pending = false;
            cached.invalidate_upstream_account_attempt_refresh();
            cached.latest_live_snapshot = None;
        }
    }

    pub(crate) async fn mark_managed_task_topic_dirty(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let generation = guard
            .managed_task_refresh_generations
            .entry(topic_key.clone())
            .or_default();
        *generation = generation.saturating_add(1);
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.latest_live_snapshot = None;
            return !cached.refresh_scheduled;
        }
        true
    }

    pub(crate) async fn clear_managed_task_refresh_schedule(&self, topic_key: &str) {
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(topic_key) {
            cached.refresh_scheduled = false;
        }
    }

    pub(crate) async fn mark_prompt_cache_topic_dirty_and_schedule_reconcile(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.prompt_cache_refresh_scheduled = false;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_pressure_deferred = false;
        if active && !cached.prompt_cache_reconcile_scheduled {
            cached.prompt_cache_reconcile_scheduled = true;
            return true;
        }
        false
    }

    pub(crate) async fn prompt_cache_topic_reconcile_delay(
        &self,
        topic: &SubscriptionTopic,
    ) -> Option<Duration> {
        let topic_key = topic.cache_key().ok()?;
        let guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let cached = guard.topics.get(&topic_key)?;
        if !active || (!cached.dirty && !cached.prompt_cache_reconcile_required) {
            return None;
        }
        Some(
            cached
                .prompt_cache_baseline_at
                .map(|baseline_at| {
                    PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL.saturating_sub(baseline_at.elapsed())
                })
                .unwrap_or_default(),
        )
    }

    pub(crate) async fn begin_prompt_cache_topic_reconcile(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !active
            || cached.prompt_cache_reconcile_scheduled
            || (!cached.dirty && !cached.prompt_cache_reconcile_required)
        {
            return false;
        }
        cached.prompt_cache_reconcile_scheduled = true;
        true
    }

    pub(crate) async fn begin_conversation_overview_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !cached.conversation_overview_refresh_scheduled {
            return false;
        }
        cached.conversation_overview_refresh_in_flight = true;
        true
    }

    pub(crate) async fn finish_conversation_overview_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        let rerun = cached.conversation_overview_refresh_pending;
        cached.conversation_overview_refresh_scheduled = false;
        cached.conversation_overview_refresh_in_flight = false;
        cached.conversation_overview_refresh_pending = false;
        rerun
    }

    pub(crate) async fn rearm_conversation_overview_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if active == 0 {
            cached.dirty = true;
            return false;
        }
        cached.conversation_overview_refresh_scheduled = true;
        true
    }

    pub(crate) async fn schedule_conversation_overview_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                return Ok(());
            }
            if cached.conversation_overview_refresh_scheduled {
                if cached.conversation_overview_refresh_in_flight {
                    cached.conversation_overview_refresh_pending = true;
                }
                return Ok(());
            }
            cached.conversation_overview_refresh_scheduled = true;
            CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE
        };
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            let mut delay = delay;
            loop {
                tokio::time::sleep(delay).await;
                if !hub.begin_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                if !hub.has_active_topic_key(&topic_key).await {
                    tracing::debug!(
                        topic = %topic.name(),
                        refresh_outcome = "marked_dirty",
                        "skipping deferred conversation overview refresh without owner subscribers"
                    );
                    hub.finish_conversation_overview_topic_refresh(&topic).await;
                    hub.mark_topic_dirty(&topic).await;
                    return;
                }
                let result = hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await;
                let rerun = hub.finish_conversation_overview_topic_refresh(&topic).await;
                match result {
                    Ok(Some(_)) | Ok(None) => {}
                    Err(err) => {
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            refresh_outcome = "retained_last_good",
                            "conversation overview topic refresh failed"
                        );
                    }
                }
                if !rerun || !hub.rearm_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                delay = CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE;
            }
        });
        Ok(())
    }

    pub(crate) async fn begin_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation
            || !cached.upstream_account_attempt_refresh_scheduled
            || cached.upstream_account_attempt_refresh_in_flight
        {
            return false;
        }
        cached.upstream_account_attempt_refresh_in_flight = true;
        true
    }

    pub(crate) async fn finish_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        let rerun = cached.upstream_account_attempt_refresh_pending;
        cached.upstream_account_attempt_refresh_in_flight = false;
        cached.upstream_account_attempt_refresh_pending = false;
        if !rerun {
            cached.upstream_account_attempt_refresh_scheduled = false;
        }
        rerun
    }

    pub(crate) async fn retain_upstream_account_attempt_topic_last_good_for_retry(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.runtime_topic_recovery_retry_at = None;
        true
    }

    pub(crate) async fn rearm_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        if active == 0 {
            cached.dirty = true;
            cached.invalidate_upstream_account_attempt_refresh();
            return false;
        }
        cached.upstream_account_attempt_refresh_scheduled = true;
        true
    }

    pub(crate) async fn finish_upstream_account_attempt_topic_refresh_without_owner(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        if active {
            // A reconnect acquired the owner between the task's wake-up and this check. Keep
            // the refresh marked in-flight so matching events during the database build are
            // coalesced into its follow-up snapshot.
            cached.upstream_account_attempt_refresh_scheduled = true;
            return true;
        }
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.invalidate_upstream_account_attempt_refresh();
        cached.latest_live_snapshot = None;
        false
    }

    pub(crate) async fn schedule_upstream_account_attempt_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let generation = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                return Ok(());
            }
            if cached.upstream_account_attempt_refresh_scheduled {
                if cached.upstream_account_attempt_refresh_in_flight {
                    cached.upstream_account_attempt_refresh_pending = true;
                }
                return Ok(());
            }
            cached.upstream_account_attempt_refresh_scheduled = true;
            cached.upstream_account_attempt_refresh_generation = cached
                .upstream_account_attempt_refresh_generation
                .saturating_add(1);
            cached.upstream_account_attempt_refresh_generation
        };

        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            let mut delay = UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE;
            loop {
                tokio::time::sleep(delay).await;
                if !hub
                    .begin_upstream_account_attempt_topic_refresh(&topic, generation)
                    .await
                {
                    return;
                }
                if !hub
                    .finish_upstream_account_attempt_topic_refresh_without_owner(&topic, generation)
                    .await
                {
                    return;
                }
                let result = hub
                    .refresh_upstream_account_attempt_topic_if_active(
                        state.clone(),
                        topic.clone(),
                        true,
                        generation,
                    )
                    .await;
                let rerun = hub
                    .finish_upstream_account_attempt_topic_refresh(&topic, generation)
                    .await;
                if let Err(err) = result {
                    if !hub
                        .retain_upstream_account_attempt_topic_last_good_for_retry(
                            &topic, generation,
                        )
                        .await
                    {
                        return;
                    }
                    warn!(
                        ?err,
                        topic = %topic.name(),
                        refresh_outcome = "retained_last_good",
                        "upstream account attempt topic refresh failed"
                    );
                    if !hub
                        .rearm_upstream_account_attempt_topic_refresh(&topic, generation)
                        .await
                    {
                        return;
                    }
                    delay = RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF;
                    continue;
                }
                if !rerun
                    || !hub
                        .rearm_upstream_account_attempt_topic_refresh(&topic, generation)
                        .await
                {
                    return;
                }
                delay = UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE;
            }
        });
        Ok(())
    }

    pub(crate) async fn schedule_parallel_work_topic_projection(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        mutations: &[SequencedRuntimeMutation],
    ) -> Result<(), ApiError> {
        enum ParallelWorkSchedule {
            None,
            Materialize(Instant),
            Reconcile,
        }

        let topic_key = topic.cache_key()?;
        loop {
            let materializer = {
                let mut guard = self.state.lock().await;
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default();
                if active == 0 {
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                    }
                    return Ok(());
                }
                if !guard.topics.contains_key(&topic_key) {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if guard.topics[&topic_key].dirty {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                let cached = guard
                    .topics
                    .get_mut(&topic_key)
                    .expect("cached topic exists");
                let Some(DashboardTopicMaterializer::ParallelWork { base }) =
                    cached.dashboard_materializer.as_ref()
                else {
                    return Ok(());
                };
                base.clone()
            };

            let outcome = {
                let mut base = materializer
                    .lock()
                    .expect("parallel-work materializer state lock");
                let mut outcome = ParallelWorkMutationOutcome::default();
                for mutation in mutations {
                    let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                        continue;
                    };
                    let mutation_outcome = base.apply_runtime_mutation(mutation);
                    outcome.changed |= mutation_outcome.changed;
                    outcome.needs_account_reconcile |= mutation_outcome.needs_account_reconcile;
                }
                outcome
            };

            let schedule = {
                let mut guard = self.state.lock().await;
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default();
                if active == 0 {
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                    }
                    return Ok(());
                }
                remove_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                if !guard.topics.contains_key(&topic_key) {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if guard.topics[&topic_key].dirty {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if outcome.needs_account_reconcile {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                }
                let cached = guard
                    .topics
                    .get_mut(&topic_key)
                    .expect("cached topic exists");
                if !matches!(
                    cached.dashboard_materializer.as_ref(),
                    Some(DashboardTopicMaterializer::ParallelWork { base }) if Arc::ptr_eq(base, &materializer)
                ) {
                    None
                } else if outcome.needs_account_reconcile {
                    if cached.parallel_work_refresh_scheduled {
                        Some(ParallelWorkSchedule::None)
                    } else {
                        cached.dirty = true;
                        cached.parallel_work_refresh_scheduled = true;
                        Some(ParallelWorkSchedule::Reconcile)
                    }
                } else if !outcome.changed || cached.parallel_work_refresh_scheduled {
                    Some(ParallelWorkSchedule::None)
                } else {
                    cached.parallel_work_refresh_scheduled = true;
                    Some(ParallelWorkSchedule::Materialize(Instant::now()))
                }
            };
            let Some(schedule) = schedule else {
                tokio::task::yield_now().await;
                continue;
            };
            match schedule {
                ParallelWorkSchedule::None => {}
                ParallelWorkSchedule::Materialize(scheduled_at) => {
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        let deadline = scheduled_at + PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE;
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                        if Instant::now().saturating_duration_since(deadline)
                            > PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE
                        {
                            hub.dashboard_topology_counters
                                .record_cadence_miss("stats.parallel-work.current");
                        }
                        hub.materialize_dashboard_parallel_work().await;
                    });
                }
                ParallelWorkSchedule::Reconcile => {
                    // Account gaps are recovered from a dirty last-good frame, not by the
                    // in-memory hot materializer. Recovery health reports that state directly;
                    // this background rebaseline delay is therefore not a hot cadence miss.
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE).await;
                        let result = hub
                            .refresh_topic_if_active(state.clone(), topic.clone(), true)
                            .await;
                        if let Err(err) = result {
                            warn!(
                                ?err,
                                topic = %topic.name(),
                                "failed to reconcile account-scoped parallel-work projection"
                            );
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        } else if hub.parallel_work_reconcile_pending(&topic).await {
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        }
                    });
                }
            }
            return Ok(());
        }
    }

    pub(crate) async fn schedule_parallel_work_reconcile_retry(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let recovery_scheduled = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                > 0;
            let has_pending_mutations = guard
                .parallel_work_prebaseline_mutations
                .get(&topic_key)
                .is_some_and(|mutations| !mutations.is_empty());
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            cached.parallel_work_refresh_scheduled = false;
            if has_pending_mutations {
                cached.dirty = true;
            }
            if !active {
                return;
            }
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
            if guard.runtime_topic_recovery_running {
                false
            } else {
                guard.runtime_topic_recovery_running = true;
                true
            }
        };
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    pub(crate) async fn parallel_work_reconcile_pending(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        self.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|mutations| !mutations.is_empty())
    }

    pub(crate) async fn schedule_prompt_cache_topic_projection(
        &self,
        state: Arc<AppState>,
        mutations: &[SequencedRuntimeMutation],
    ) {
        // The active dependency lookup comes before the compact preview lookup below. Inactive
        // Prompt Cache topics therefore never allocate preview data or read runtime state.
        let active_topic_keys = {
            let guard = self.state.lock().await;
            let mut topic_keys = Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheProjection,
            );
            topic_keys.extend(Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::DashboardWorkingConversationsProjection,
            ));
            topic_keys
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        if active_topic_keys.is_empty() {
            return;
        }

        let mut records = Vec::new();
        let mut reconcile_required = false;
        let mut working_key_hydration_keys = BTreeSet::new();
        let mut working_reconcile_required = false;
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            if mutation.prompt_cache_key.is_none() && mutation.sticky_key.is_none() {
                continue;
            }
            let runtime_projection =
                (mutation.kind != RuntimeMutationKind::RuntimeRemoved).then(|| {
                    state
                        .proxy_runtime_invocations
                        .prompt_cache_projection_by_identity(
                            &mutation.identity.invoke_id,
                            &mutation.identity.occurred_at,
                        )
                });
            let runtime_projection = runtime_projection.flatten();
            match PromptCacheTopicDelta::from_runtime_mutation(
                mutation,
                runtime_projection.as_ref(),
            ) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {
                    reconcile_required = true;
                    // A terminal can leave the runtime store immediately after P1 assigns its
                    // durable row id. Working conversations can recover that one durable key
                    // exactly; treating it as a generic gap would needlessly rebuild the whole
                    // active window.
                    if mutation.is_terminal
                        && mutation.row_id.is_some_and(|row_id| row_id > 0)
                        && let Some(prompt_cache_key) = mutation.prompt_cache_key.as_deref()
                    {
                        working_key_hydration_keys.insert(prompt_cache_key.to_string());
                    } else {
                        working_reconcile_required = true;
                    }
                }
                Err(err) => {
                    reconcile_required = true;
                    working_reconcile_required = true;
                    warn!(?err, "failed to build active prompt cache topic delta");
                }
            }
        }
        if records.is_empty() && !reconcile_required {
            return;
        }

        let (scheduled, key_hydrations, reconciles) = {
            let mut guard = self.state.lock().await;
            let mut scheduled = Vec::new();
            let mut key_hydrations = Vec::new();
            let mut reconciles = Vec::new();
            for topic_key in active_topic_keys {
                let Some(topic) = guard.active_topics.get(&topic_key).cloned() else {
                    continue;
                };
                let Some(cached) = guard.topics.get_mut(&topic_key) else {
                    if !records.is_empty() {
                        let pending = guard
                            .prompt_cache_prebaseline_records
                            .entry(topic_key.clone())
                            .or_default();
                        for record in &records {
                            pending.insert(record.identity.clone(), record.clone());
                        }
                    }
                    if matches!(
                        topic,
                        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    ) && !working_key_hydration_keys.is_empty()
                    {
                        guard
                            .prompt_cache_prebaseline_key_hydrations
                            .entry(topic_key)
                            .or_default()
                            .extend(working_key_hydration_keys.iter().cloned());
                    }
                    continue;
                };
                let is_working_conversations = matches!(
                    topic,
                    SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                );
                let requires_bounded_reconcile = if is_working_conversations {
                    working_reconcile_required || cached.dirty
                } else {
                    reconcile_required || cached.dirty
                };
                if requires_bounded_reconcile {
                    cached.dirty = true;
                    cached.prompt_cache_reconcile_required = true;
                    if !cached.prompt_cache_reconcile_scheduled {
                        cached.prompt_cache_reconcile_scheduled = true;
                        reconciles.push(topic);
                    }
                    // A gap means the retained payload may be missing an earlier mutation.
                    // Do not append a later delta to that stale frame; one active selection is
                    // rebuilt asynchronously from its bounded cold source instead.
                    continue;
                }
                if is_working_conversations && !working_key_hydration_keys.is_empty() {
                    cached
                        .prompt_cache_pending_key_hydrations
                        .extend(working_key_hydration_keys.iter().cloned());
                    if !cached.prompt_cache_key_hydration_scheduled {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        key_hydrations.push(topic.clone());
                    }
                }
                if records.is_empty() {
                    continue;
                }
                let before = cached.prompt_cache_pending_records.len();
                for record in &records {
                    cached
                        .prompt_cache_pending_records
                        .insert(record.identity.clone(), record.clone());
                }
                cached.prompt_cache_coalesced_event_count =
                    cached.prompt_cache_coalesced_event_count.saturating_add(
                        records.len().saturating_sub(
                            cached
                                .prompt_cache_pending_records
                                .len()
                                .saturating_sub(before),
                        ) as u64,
                    );
                if !cached.prompt_cache_refresh_scheduled {
                    cached.prompt_cache_refresh_scheduled = true;
                    scheduled.push(topic);
                }
            }
            (scheduled, key_hydrations, reconciles)
        };

        for topic in scheduled {
            let hub = state.subscription_hub.clone();
            let state = state.clone();
            let deadline = Instant::now() + PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE;
            tokio::spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                if Instant::now().saturating_duration_since(deadline)
                    > PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE
                {
                    hub.dashboard_topology_counters
                        .record_cadence_miss(topic.name());
                }
                if let Err(err) = hub
                    .materialize_prompt_cache_topic(state.clone(), &topic)
                    .await
                {
                    warn!(
                        ?err,
                        topic = topic.name(),
                        response_source = "last_good",
                        "prompt cache in-memory topic materialization failed"
                    );
                    if hub
                        .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                        .await
                    {
                        SubscriptionHub::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
                    }
                }
            });
        }
        for topic in key_hydrations {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
    }
}
