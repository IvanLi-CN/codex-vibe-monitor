struct TopicRefreshContext {
    state: Arc<AppState>,
    topic: SubscriptionTopic,
    topic_key: String,
    schema_epoch: String,
    descriptor: SubscriptionTopicDescriptor,
    emit_live: bool,
    require_active_owner: bool,
    expected_upstream_account_attempt_refresh_generation: Option<u64>,
    refresh_generation: Option<u64>,
    refresh_had_cached_topic: bool,
    started: Instant,
    built_payload: BuiltSubscriptionTopicPayload,
    prompt_cache_build: Option<PromptCacheBaselineBuild>,
    parallel_work_build: Option<ParallelWorkBaselineBuild>,
}

struct PromptRefreshReplay {
    applied_terminal_ids: HashSet<String>,
    deferred_replay: Vec<PromptCacheTopicDelta>,
    deferred_hydration_keys: BTreeSet<String>,
    deferred_reconcile: bool,
}

struct TopicRefreshCommitState {
    serialized_payload: Vec<u8>,
    dashboard_materializer: Option<DashboardTopicMaterializer>,
    current_slice: Option<Arc<DashboardCurrentProjectionSlice>>,
    network_slice: Option<Arc<DashboardNetworkProjectionSlice>>,
    terminal_slice: Option<Arc<DashboardTerminalProjectionSlice>>,
    prompt_replay: PromptRefreshReplay,
}

type CreatedTopicResult = Result<
    (
        CachedSubscriptionTopic,
        Option<SubscriptionDispatchEvent>,
        Option<SubscriptionTopic>,
        Option<SubscriptionTopic>,
    ),
    ApiError,
>;

macro_rules! refreshed_cached_subscription_topic {
    (
        $context:expr,
        $commit:expr,
        $previous:expr,
        $guard:expr,
        $next_cursor:expr,
        $continuity_reset_cursor:expr,
        $dashboard_materialized_revision:expr,
        $had_key_hydration_scheduled:expr,
        $had_reconcile_scheduled:expr,
        $frame:expr,
        $payload_bytes:expr $(,)?
    ) => {{
        CachedSubscriptionTopic {
            topic: $context.topic.clone(),
            descriptor: $context.descriptor.clone(),
            schema_epoch: $context.schema_epoch.clone(),
            cursor: $next_cursor,
            snapshot_built_at: Instant::now(),
            refresh_scheduled: false,
            conversation_overview_refresh_scheduled: $previous
                .is_some_and(|entry| entry.conversation_overview_refresh_scheduled),
            conversation_overview_refresh_in_flight: $previous
                .is_some_and(|entry| entry.conversation_overview_refresh_in_flight),
            conversation_overview_refresh_pending: $previous
                .is_some_and(|entry| entry.conversation_overview_refresh_pending),
            upstream_account_attempt_refresh_scheduled: $previous
                .is_some_and(|entry| entry.upstream_account_attempt_refresh_scheduled),
            upstream_account_attempt_refresh_in_flight: $previous
                .is_some_and(|entry| entry.upstream_account_attempt_refresh_in_flight),
            upstream_account_attempt_refresh_pending: $previous
                .is_some_and(|entry| entry.upstream_account_attempt_refresh_pending),
            upstream_account_attempt_refresh_generation: $previous
                .map_or(0, |entry| entry.upstream_account_attempt_refresh_generation),
            dirty: $commit.prompt_replay.deferred_reconcile,
            runtime_topic_recovery_generation: $previous
                .map_or($guard.runtime_topic_recovery_generation, |entry| {
                    entry.runtime_topic_recovery_generation
                }),
            runtime_topic_recovery_retry_at: None,
            summary_refresh_scheduled: $previous
                .is_some_and(|entry| entry.summary_refresh_scheduled),
            summary_refresh_in_flight: $previous
                .is_some_and(|entry| entry.summary_refresh_in_flight),
            summary_pending_event_count: $previous
                .map_or(0, |entry| entry.summary_pending_event_count),
            summary_retry_backoff_ms: $previous.map_or(0, |entry| entry.summary_retry_backoff_ms),
            parallel_work_refresh_scheduled: false,
            prompt_cache_refresh_scheduled: $previous
                .is_some_and(|entry| entry.prompt_cache_refresh_scheduled),
            prompt_cache_reconcile_scheduled: $had_reconcile_scheduled
                || $commit.prompt_replay.deferred_reconcile,
            prompt_cache_key_hydration_scheduled: $had_key_hydration_scheduled
                || !$commit.prompt_replay.deferred_hydration_keys.is_empty(),
            prompt_cache_pending_records: $commit
                .prompt_replay
                .deferred_replay
                .into_iter()
                .map(|record| (record.identity.clone(), record))
                .collect(),
            prompt_cache_pending_key_hydrations: $commit.prompt_replay.deferred_hydration_keys,
            prompt_cache_candidate_refill_required: false,
            prompt_cache_applied_terminal_ids: if matches!(
                &$context.topic,
                SubscriptionTopic::PromptCacheWindow { .. }
                    | SubscriptionTopic::PromptCacheStickyWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            ) {
                $commit.prompt_replay.applied_terminal_ids
            } else {
                $previous
                    .map(|entry| entry.prompt_cache_applied_terminal_ids.clone())
                    .unwrap_or_default()
            },
            prompt_cache_coalesced_event_count: $previous
                .map_or(0, |entry| entry.prompt_cache_coalesced_event_count),
            prompt_cache_full_hydration_count: $previous.map_or(1, |entry| {
                entry.prompt_cache_full_hydration_count.saturating_add(1)
            }),
            prompt_cache_bounded_key_hydration_count: $previous
                .map_or(0, |entry| entry.prompt_cache_bounded_key_hydration_count),
            prompt_cache_baseline_at: Some(Instant::now()),
            prompt_cache_baseline_row_id: $context
                .prompt_cache_build
                .as_ref()
                .map_or(0, |build| build.baseline_row_id),
            prompt_cache_response_source: if $previous.is_some() {
                "database_reconcile"
            } else {
                "initial_baseline"
            },
            prompt_cache_reconcile_required: $commit.prompt_replay.deferred_reconcile,
            prompt_cache_pressure_deferred: false,
            latest_live_snapshot: $previous.and_then(|entry| entry.latest_live_snapshot.clone()),
            calendar_anchor: subscription_calendar_anchor(&$context.topic),
            continuity_reset_cursor: $continuity_reset_cursor,
            dashboard_materializer: $commit.dashboard_materializer,
            dashboard_base_revision: $next_cursor,
            dashboard_materialized_revision: $dashboard_materialized_revision,
            snapshot_payload: $context.built_payload.snapshot_payload(),
            snapshot_frame: $frame.clone(),
            snapshot_bytes: $payload_bytes,
            replay_events: $previous
                .filter(|entry| !entry.dirty)
                .map(|entry| entry.replay_events.clone())
                .unwrap_or_default(),
            replay_bytes: $previous
                .filter(|entry| !entry.dirty)
                .map_or(0, |entry| entry.replay_bytes),
        }
    }};
}
impl SubscriptionHub {
    async fn refresh_topic_inner(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
        expected_upstream_account_attempt_refresh_generation: Option<u64>,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        let Some(context) = self
            .prepare_topic_refresh(
                state,
                topic,
                emit_live,
                require_active_owner,
                expected_upstream_account_attempt_refresh_generation,
            )
            .await?
        else {
            return Ok(None);
        };
        let refresh_state = context.state.clone();
        let topic_key = context.topic_key.clone();
        let schema_epoch = context.schema_epoch.clone();
        let started = context.started;
        let emit_live = context.emit_live;
        let Some((cached, dispatch, initial_hydration, initial_reconcile)) =
            self.commit_topic_refresh(context).await?
        else {
            return Ok(None);
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch.clone());
            tracing::debug!(
                topic_key = dispatch.frame.topic_key,
                cursor = dispatch.frame.cursor,
                fanout_receivers = self.broadcaster.receiver_count(),
                "subscription topic live event dispatched"
            );
        }
        if let Some(topic) = initial_hydration {
            Self::spawn_dashboard_working_conversation_key_hydration(refresh_state.clone(), topic);
        }
        if let Some(topic) = initial_reconcile {
            Self::spawn_prompt_cache_topic_reconcile(refresh_state, topic);
        }
        tracing::debug!(
            topic_key,
            schema_epoch,
            emit_live,
            snapshot_build_ms = started.elapsed().as_millis() as u64,
            payload_bytes = cached.snapshot_bytes,
            "subscription topic snapshot built"
        );
        Ok(Some(cached))
    }

    async fn prepare_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
        expected_upstream_account_attempt_refresh_generation: Option<u64>,
    ) -> Result<Option<TopicRefreshContext>, ApiError> {
        let topic_key = topic.cache_key()?;
        let schema_epoch = topic.schema_epoch();
        let descriptor = topic.descriptor();
        let started = Instant::now();
        let is_prompt_cache_topic = matches!(
            topic,
            SubscriptionTopic::PromptCacheWindow { .. }
                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
        );
        let is_open_parallel_work_topic = matches!(
            &topic,
            SubscriptionTopic::ParallelWorkCurrent { range, .. } if range != "yesterday"
        );
        let (refresh_generation, refresh_had_cached_topic) = if require_active_owner {
            let guard = self.state.lock().await;
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                return Ok(None);
            }
            if let Some(expected_generation) = expected_upstream_account_attempt_refresh_generation
                && !guard.topics.get(&topic_key).is_some_and(|cached| {
                    cached.upstream_account_attempt_refresh_generation == expected_generation
                })
            {
                return Ok(None);
            }
            guard
                .topics
                .get(&topic_key)
                .map(|cached| (Some(cached.runtime_topic_recovery_generation), true))
                .unwrap_or((Some(guard.runtime_topic_recovery_generation), false))
        } else {
            (None, false)
        };
        let (built_payload, prompt_cache_build, parallel_work_build) = if is_prompt_cache_topic {
            let (payload, build) = self
                .build_prompt_cache_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, Some(build), None)
        } else if is_open_parallel_work_topic {
            let (payload, build) = self
                .build_parallel_work_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, None, Some(build))
        } else {
            (topic.build_cached_payload(state.clone()).await?, None, None)
        };
        self.dashboard_topology_counters.record_materialization(
            topic.name(),
            matches!(&built_payload, BuiltSubscriptionTopicPayload::Json(_))
                && topic.is_unmigrated_dashboard_hot_projection(),
        );
        Ok(Some(TopicRefreshContext {
            state,
            topic,
            topic_key,
            schema_epoch,
            descriptor,
            emit_live,
            require_active_owner,
            expected_upstream_account_attempt_refresh_generation,
            refresh_generation,
            refresh_had_cached_topic,
            started,
            built_payload,
            prompt_cache_build,
            parallel_work_build,
        }))
    }

    async fn commit_topic_refresh(
        &self,
        mut context: TopicRefreshContext,
    ) -> Result<
        Option<(
            CachedSubscriptionTopic,
            Option<SubscriptionDispatchEvent>,
            Option<SubscriptionTopic>,
            Option<SubscriptionTopic>,
        )>,
        ApiError,
    > {
        let mut guard = self.state.lock().await;
        if !self.refresh_owner_is_current(&mut guard, &context) {
            return Ok(None);
        }
        self.apply_topic_refresh_overlays(&mut guard, &mut context)?;
        let prompt_replay = self.prepare_prompt_refresh_replay(&mut guard, &mut context)?;
        let commit_state = self.serialize_topic_refresh(&guard, &context, prompt_replay)?;
        if let Some(cached) = self.try_reuse_refreshed_topic(&mut guard, &context, &commit_state) {
            return Ok(Some((cached, None, None, None)));
        }
        let result = self.create_refreshed_topic(&mut guard, context, commit_state)?;
        Ok(Some(result))
    }

    fn refresh_owner_is_current(
        &self,
        guard: &mut SubscriptionHubState,
        context: &TopicRefreshContext,
    ) -> bool {
        if !context.require_active_owner {
            return true;
        }
        let active = guard
            .active_subscribers
            .get(&context.topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let generation_matches = if context.refresh_had_cached_topic {
            guard.topics.get(&context.topic_key).is_some_and(|cached| {
                Some(cached.runtime_topic_recovery_generation) == context.refresh_generation
            })
        } else {
            Some(guard.runtime_topic_recovery_generation) == context.refresh_generation
                && guard.topics.get(&context.topic_key).is_none_or(|cached| {
                    Some(cached.runtime_topic_recovery_generation) == context.refresh_generation
                })
        };
        let account_attempt_generation_matches = context
            .expected_upstream_account_attempt_refresh_generation
            .is_none_or(|expected_generation| {
                guard.topics.get(&context.topic_key).is_some_and(|cached| {
                    cached.upstream_account_attempt_refresh_generation == expected_generation
                })
            });
        if active && generation_matches && account_attempt_generation_matches {
            return true;
        }
        if !active && let Some(cached) = guard.topics.get_mut(&context.topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.latest_live_snapshot = None;
        }
        false
    }

    fn apply_topic_refresh_overlays(
        &self,
        guard: &mut SubscriptionHubState,
        context: &mut TopicRefreshContext,
    ) -> Result<(), ApiError> {
        if let BuiltSubscriptionTopicPayload::Json(payload) = &mut context.built_payload
            && let Some(live) = guard
                .topics
                .get(&context.topic_key)
                .and_then(|cached| cached.latest_live_snapshot.as_ref())
                .cloned()
        {
            apply_topic_live_overlay_to_payload(
                context.state.as_ref(),
                &context.topic,
                payload,
                &live,
            )?;
        }
        self.replay_parallel_work_refresh(guard, context);
        Ok(())
    }

    fn replay_parallel_work_refresh(
        &self,
        guard: &mut SubscriptionHubState,
        context: &TopicRefreshContext,
    ) {
        let BuiltSubscriptionTopicPayload::Dashboard(DashboardTopicMaterializer::ParallelWork {
            base,
        }) = &context.built_payload
        else {
            return;
        };
        let baseline_identities = context
            .parallel_work_build
            .as_ref()
            .map(|build| build.persisted_identities.clone())
            .unwrap_or_default();
        let mut replay = guard
            .topics
            .get(&context.topic_key)
            .and_then(|cached| match cached.dashboard_materializer.as_ref() {
                Some(DashboardTopicMaterializer::ParallelWork { base }) => Some(
                    base.lock()
                        .expect("parallel-work materializer state lock")
                        .runtime_mutations
                        .clone(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let mut pending = guard
            .parallel_work_prebaseline_mutations
            .remove(&context.topic_key)
            .unwrap_or_default();
        replay.append(&mut pending);
        let mut base = base.lock().expect("parallel-work materializer state lock");
        base.baseline_identities = baseline_identities;
        let mut unresolved = BTreeMap::new();
        for (identity, mutation) in &replay {
            if base
                .apply_runtime_mutation(mutation)
                .needs_account_reconcile
            {
                unresolved.insert(identity.clone(), mutation.clone());
            }
        }
        if !unresolved.is_empty() {
            guard
                .parallel_work_prebaseline_mutations
                .entry(context.topic_key.clone())
                .or_default()
                .extend(unresolved);
        }
    }

    fn prepare_prompt_refresh_replay(
        &self,
        guard: &mut SubscriptionHubState,
        context: &mut TopicRefreshContext,
    ) -> Result<PromptRefreshReplay, ApiError> {
        let mut pending = guard
            .prompt_cache_prebaseline_records
            .remove(&context.topic_key)
            .unwrap_or_default();
        let prebaseline_hydration_keys = guard
            .prompt_cache_prebaseline_key_hydrations
            .remove(&context.topic_key)
            .unwrap_or_default();
        if let Some(existing) = guard.topics.get_mut(&context.topic_key) {
            pending.append(&mut existing.prompt_cache_pending_records);
            existing.prompt_cache_refresh_scheduled = false;
        }
        let replay = context
            .prompt_cache_build
            .as_ref()
            .map(|build| {
                pending
                    .into_values()
                    .filter(|delta| {
                        prompt_cache_delta_needs_replay(delta, &build.persisted_identities)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut applied_terminal_ids = context
            .prompt_cache_build
            .as_ref()
            .map(|build| build.runtime_overlay_terminal_identities.clone())
            .unwrap_or_default();
        let mut deferred_replay = Vec::new();
        let mut deferred_hydration_keys = prebaseline_hydration_keys;
        let mut deferred_reconcile = false;
        if !replay.is_empty() {
            let baseline_row_id = context
                .prompt_cache_build
                .as_ref()
                .map_or(0, |build| build.baseline_row_id);
            match &mut context.built_payload {
                BuiltSubscriptionTopicPayload::Json(payload) => {
                    apply_prompt_cache_records_to_payload(
                        &context.topic,
                        payload,
                        &replay,
                        &mut applied_terminal_ids,
                        baseline_row_id,
                    )?;
                }
                BuiltSubscriptionTopicPayload::Dashboard(
                    DashboardTopicMaterializer::WorkingConversations { state },
                ) => {
                    let update = state
                        .lock()
                        .expect("working conversations materializer state lock")
                        .apply_deltas(&replay, &mut applied_terminal_ids, baseline_row_id)?;
                    match update {
                        WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(keys) => {
                            deferred_replay = replay;
                            deferred_hydration_keys = keys;
                        }
                        WorkingConversationsProjectionUpdate::NeedsReconcile => {
                            deferred_replay = replay;
                            deferred_reconcile = true;
                        }
                        WorkingConversationsProjectionUpdate::Changed
                        | WorkingConversationsProjectionUpdate::Unchanged => {}
                    }
                }
                BuiltSubscriptionTopicPayload::Dashboard(_) => {}
            }
        }
        Ok(PromptRefreshReplay {
            applied_terminal_ids,
            deferred_replay,
            deferred_hydration_keys,
            deferred_reconcile,
        })
    }

    fn serialize_topic_refresh(
        &self,
        guard: &SubscriptionHubState,
        context: &TopicRefreshContext,
        prompt_replay: PromptRefreshReplay,
    ) -> Result<TopicRefreshCommitState, ApiError> {
        let serialized_payload = context.built_payload.serialize(
            guard.dashboard_current_slice.as_deref(),
            guard.dashboard_network_slice.as_deref(),
            guard.dashboard_terminal_slice.as_deref(),
        )?;
        Ok(TopicRefreshCommitState {
            serialized_payload,
            dashboard_materializer: context.built_payload.dashboard_materializer(),
            current_slice: guard.dashboard_current_slice.clone(),
            network_slice: guard.dashboard_network_slice.clone(),
            terminal_slice: guard.dashboard_terminal_slice.clone(),
            prompt_replay,
        })
    }

    fn try_reuse_refreshed_topic(
        &self,
        guard: &mut SubscriptionHubState,
        context: &TopicRefreshContext,
        commit: &TopicRefreshCommitState,
    ) -> Option<CachedSubscriptionTopic> {
        if !commit.prompt_replay.deferred_replay.is_empty() {
            return None;
        }
        let existing = guard.topics.get_mut(&context.topic_key)?;
        if existing.snapshot_frame.payload_bytes.as_ref() == commit.serialized_payload.as_slice()
            && existing.dirty
            && existing.dashboard_materializer.is_some()
            && commit.dashboard_materializer.is_some()
        {
            reuse_unchanged_cached_topic(existing, &commit.serialized_payload)
                .expect("matching dashboard payload must reuse the cached topic");
            self.dashboard_topology_counters
                .record_frame_reused(context.topic.name());
            existing.dashboard_materializer = commit.dashboard_materializer.clone();
            existing.dashboard_base_revision = existing.cursor;
            existing.dashboard_materialized_revision = commit
                .dashboard_materializer
                .as_ref()
                .and_then(|materializer| {
                    materializer.revision(
                        existing.cursor,
                        commit.current_slice.as_deref(),
                        commit.network_slice.as_deref(),
                        commit.terminal_slice.as_deref(),
                    )
                });
            existing.snapshot_payload = context.built_payload.snapshot_payload();
            if let Some(build) = &context.prompt_cache_build {
                finish_prompt_cache_baseline_reuse(
                    existing,
                    build,
                    &commit.prompt_replay.applied_terminal_ids,
                );
            }
            return Some(existing.clone());
        }
        if reuse_unchanged_cached_topic(existing, &commit.serialized_payload).is_some() {
            self.dashboard_topology_counters
                .record_frame_reused(context.topic.name());
            if let Some(build) = &context.prompt_cache_build {
                finish_prompt_cache_baseline_reuse(
                    existing,
                    build,
                    &commit.prompt_replay.applied_terminal_ids,
                );
            }
            return Some(existing.clone());
        }
        None
    }

    fn create_refreshed_topic(
        &self,
        guard: &mut SubscriptionHubState,
        context: TopicRefreshContext,
        commit: TopicRefreshCommitState,
    ) -> CreatedTopicResult {
        let next_cursor = guard
            .topics
            .get(&context.topic_key)
            .map_or(0, |entry| entry.cursor)
            .saturating_add(1);
        let continuity_reset_cursor = guard.topics.get(&context.topic_key).and_then(|entry| {
            if entry.dirty {
                Some(next_cursor)
            } else {
                entry.continuity_reset_cursor
            }
        });
        let frame = Arc::new(self.serialize_frame(
            context.descriptor.clone(),
            context.topic_key.clone(),
            context.schema_epoch.clone(),
            next_cursor,
            commit.serialized_payload,
        )?);
        let payload_bytes = frame.payload_bytes.len();
        let dashboard_materialized_revision =
            commit
                .dashboard_materializer
                .as_ref()
                .and_then(|materializer| {
                    materializer.revision(
                        next_cursor,
                        commit.current_slice.as_deref(),
                        commit.network_slice.as_deref(),
                        commit.terminal_slice.as_deref(),
                    )
                });
        let had_key_hydration_scheduled = guard
            .topics
            .get(&context.topic_key)
            .is_some_and(|entry| entry.prompt_cache_key_hydration_scheduled);
        let had_reconcile_scheduled = guard
            .topics
            .get(&context.topic_key)
            .is_some_and(|entry| entry.prompt_cache_reconcile_scheduled);
        let schedule_initial_hydration = !commit.prompt_replay.deferred_hydration_keys.is_empty()
            && !had_key_hydration_scheduled;
        let schedule_initial_reconcile =
            commit.prompt_replay.deferred_reconcile && !had_reconcile_scheduled;
        let previous = guard.topics.get(&context.topic_key);
        let mut next = refreshed_cached_subscription_topic!(
            context,
            commit,
            previous,
            guard,
            next_cursor,
            continuity_reset_cursor,
            dashboard_materialized_revision,
            had_key_hydration_scheduled,
            had_reconcile_scheduled,
            frame,
            payload_bytes,
        );
        if context.emit_live {
            let retained_bytes = frame.retained_bytes();
            next.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            next.replay_bytes = next.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut next.replay_events, &mut next.replay_bytes);
        }
        let dispatch = context
            .emit_live
            .then_some(SubscriptionDispatchEvent { frame });
        let initial_hydration = schedule_initial_hydration.then(|| context.topic.clone());
        let initial_reconcile = schedule_initial_reconcile.then(|| context.topic.clone());
        guard.topics.insert(context.topic_key, next.clone());
        Ok((next, dispatch, initial_hydration, initial_reconcile))
    }
    pub(crate) async fn materialize_dashboard_current_slice(
        &self,
        slice: DashboardCurrentProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_current_slice
                .as_ref()
                .is_some_and(|current| current.revision >= slice.revision)
            {
                return;
            }
            guard.dashboard_current_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    pub(crate) async fn materialize_dashboard_network_slice(
        &self,
        slice: DashboardNetworkProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_network_slice
                .as_ref()
                .is_some_and(|network| network.revision >= slice.revision)
            {
                return;
            }
            guard.dashboard_network_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    pub(crate) async fn materialize_dashboard_terminal_slice(
        &self,
        slice: DashboardTerminalProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_terminal_slice
                .as_ref()
                .is_some_and(|terminal| terminal.revision >= slice.revision)
            {
                return;
            }
            #[cfg(test)]
            let highest_sequence = slice
                .deltas
                .iter()
                .map(|delta| delta.terminal_sequence)
                .max()
                .unwrap_or_default();
            #[cfg(test)]
            for delta in &slice.deltas {
                // Test fixtures can model a pre-existing durable row without a writer task.
                // Production admission remains exclusively on the writer's post-commit ACK.
                if delta.persisted_row_id.is_some() {
                    if !guard
                        .summary_delta_journal
                        .append(delta.clone(), highest_sequence)
                    {
                        tracing::warn!(
                            pending_terminal_count = guard.summary_delta_journal.entries.len(),
                            pending_terminal_bytes = guard.summary_delta_journal.bytes,
                            overflowed_through = ?guard.summary_delta_journal.overflowed_through_sequence,
                            "rolling Summary Delta Journal reached a bounded gap"
                        );
                    }
                    append_summary_all_time_delta(&mut guard, delta, highest_sequence);
                }
            }
            guard.dashboard_terminal_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            mark_dashboard_terminal_window_rebase_topics(&mut guard);
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    async fn materialize_dashboard_parallel_work(&self) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            for cached in guard.topics.values_mut() {
                if matches!(
                    &cached.dashboard_materializer,
                    Some(DashboardTopicMaterializer::ParallelWork { .. })
                ) {
                    cached.parallel_work_refresh_scheduled = false;
                }
            }
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    async fn materialize_pending_dashboard_topics(
        &self,
        pending: Vec<PendingDashboardTopicMaterialization>,
        current: Option<Arc<DashboardCurrentProjectionSlice>>,
        network: Option<Arc<DashboardNetworkProjectionSlice>>,
        terminal: Option<Arc<DashboardTerminalProjectionSlice>>,
    ) {
        for pending in pending {
            let serialized_payload = match pending.materializer.serialize(
                current.as_deref(),
                network.as_deref(),
                terminal.as_deref(),
            ) {
                Ok(payload) => payload,
                Err(err) => {
                    warn!(
                        ?err,
                        topic = pending.topic_name,
                        "failed to materialize dashboard topic frame"
                    );
                    continue;
                }
            };
            if let Err(err) = self
                .commit_dashboard_materialized_frame(pending, serialized_payload)
                .await
            {
                warn!(?err, "failed to commit dashboard topic frame");
            }
        }
    }

    async fn apply_account_effective_routing_rules_change(
        &self,
        version: &crate::upstream_accounts::RoutingStateVersion,
        upserts: &[(i64, crate::upstream_accounts::EffectiveRoutingRule)],
        removed_account_ids: &[i64],
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            let active_subscribers = guard.active_subscribers.clone();
            let server_push_subscribers = guard.server_push_subscribers.clone();
            for (topic_key, cached) in &mut guard.topics {
                let Some(DashboardTopicMaterializer::Activity { base, .. }) =
                    cached.dashboard_materializer.as_ref()
                else {
                    continue;
                };
                if active_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                    && server_push_subscribers
                        .get(topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                {
                    cached.dirty = true;
                    cached.latest_live_snapshot = None;
                    continue;
                }
                let mut materializer = base.lock().expect("activity materializer state lock");
                if materializer.base.apply_routing_rule_change(
                    version,
                    upserts,
                    removed_account_ids,
                ) {
                    materializer.routing_revision = materializer.routing_revision.saturating_add(1);
                }
            }
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    async fn commit_dashboard_materialized_frame(
        &self,
        pending: PendingDashboardTopicMaterialization,
        serialized_payload: Vec<u8>,
    ) -> Result<(), ApiError> {
        let dispatch = {
            let mut guard = self.state.lock().await;
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let Some(cached) = guard.topics.get_mut(&pending.topic_key) else {
                return Ok(());
            };
            let expected_revision =
                cached
                    .dashboard_materializer
                    .as_ref()
                    .and_then(|materializer| {
                        materializer.revision(
                            cached.dashboard_base_revision,
                            current.as_deref(),
                            network.as_deref(),
                            terminal.as_deref(),
                        )
                    });
            if cached.dashboard_base_revision != pending.revision.base_revision
                || expected_revision != Some(pending.revision)
                || cached.dashboard_materialized_revision == Some(pending.revision)
            {
                return Ok(());
            }
            if cached.snapshot_frame.payload_bytes.as_ref() == serialized_payload.as_slice() {
                cached.dashboard_materialized_revision = Some(pending.revision);
                self.dashboard_topology_counters
                    .record_frame_reused(pending.topic_name);
                return Ok(());
            }

            let next_cursor = cached.cursor.saturating_add(1);
            self.dashboard_topology_counters
                .record_materialization(pending.topic_name, false);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                pending.topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.snapshot_frame = frame.clone();
            cached.dashboard_materialized_revision = Some(pending.revision);
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }
}
