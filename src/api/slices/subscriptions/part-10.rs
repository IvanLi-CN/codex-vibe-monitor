impl SubscriptionHub {
    async fn handle_runtime_mutation_batch(
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

        self.apply_runtime_mutation_side_effects(state.clone(), &mutations)
            .await;

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

    async fn apply_runtime_mutation_side_effects(
        &self,
        state: Arc<AppState>,
        mutations: &[SequencedRuntimeMutation],
    ) {
        for mutation in mutations {
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
                    .await
                }
                RuntimeMutation::Invocation(_)
                | RuntimeMutation::AttemptChanged { .. }
                | RuntimeMutation::ModelRoutingChanged => {}
            }
        }
    }

    async fn mark_runtime_mutation_gap_and_recover(
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

    async fn invalidate_active_upstream_account_attempt_topics_for_recovery(
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

    async fn schedule_invalidated_upstream_account_attempt_topics(
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

    async fn mark_internal_broadcast_gap_and_recover(&self, state: Arc<AppState>, skipped: u64) {
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

    async fn schedule_dirty_topic_recovery(&self, state: Arc<AppState>, topic: SubscriptionTopic) {
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

    fn next_runtime_topic_recovery_retry_delay_locked(
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

    fn enqueue_runtime_topic_recovery_locked(guard: &mut SubscriptionHubState) -> bool {
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

    async fn run_runtime_topic_recovery(self: Arc<Self>, state: Arc<AppState>) {
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

    async fn defer_runtime_topic_recovery_retry(&self, topic: &SubscriptionTopic) -> Duration {
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

        self.dispatch_internal_broadcast_topics(state, payload, affected)
            .await;
    }

    async fn dispatch_internal_broadcast_topics(
        &self,
        state: Arc<AppState>,
        payload: BroadcastPayload,
        affected: Vec<CachedSubscriptionTopic>,
    ) {
        for cached in affected {
            if cached.topic.uses_summary_live_overlay()
                && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
            {
                if let Err(err) = self
                    .apply_summary_live_overlay(&cached.topic, snapshot.as_ref().clone())
                    .await
                {
                    warn!(?err, topic = %cached.topic.name(), "failed to apply summary live overlay");
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
                    warn!(?err, topic = %cached.topic.name(), "failed to apply dashboard activity live overlay");
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
                    warn!(?err, topic = %cached.topic.name(), "failed to schedule upstream account attempt topic refresh");
                }
                continue;
            }
            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), cached.topic.clone(), true)
                .await
            {
                warn!(?err, topic = %cached.topic.name(), "failed to refresh subscription topic");
            }
        }
    }

    async fn handle_dashboard_network_slice(
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
