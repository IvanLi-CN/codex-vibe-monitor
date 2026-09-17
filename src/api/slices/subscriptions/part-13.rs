struct PromptCacheProjectionBatch {
    records: Vec<PromptCacheTopicDelta>,
    reconcile_required: bool,
    working_key_hydration_keys: BTreeSet<String>,
    working_reconcile_required: bool,
}

struct PromptCacheProjectionQueues {
    scheduled: Vec<SubscriptionTopic>,
    key_hydrations: Vec<SubscriptionTopic>,
    reconciles: Vec<SubscriptionTopic>,
}

type WorkingConversationHydrationSnapshot = (
    BTreeSet<String>,
    BTreeMap<String, PromptCacheTopicDelta>,
    bool,
    usize,
    usize,
    Option<PromptCacheConversationBlockedBindingFilter>,
    HashSet<String>,
);

type WorkingConversationHydratedResponses = Vec<(String, Option<PromptCacheConversationResponse>)>;

struct WorkingConversationHydrationCommitRequest<'a> {
    topic_key: &'a str,
    pending_records_at_hydration_start: &'a BTreeMap<String, PromptCacheTopicDelta>,
    hydration_keys: &'a [String],
    hydrated: WorkingConversationHydratedResponses,
    total_matched: i64,
}

struct WorkingConversationHydrationCommitActions {
    dispatch: Option<SubscriptionDispatchEvent>,
    next_hydration: Option<SubscriptionTopic>,
    next_materialization: Option<SubscriptionTopic>,
    recovery: Option<SubscriptionTopic>,
}

struct WorkingConversationHydrationStateUpdate {
    changed: bool,
    next_hydration: Option<SubscriptionTopic>,
    next_materialization: Option<SubscriptionTopic>,
    recovery: Option<SubscriptionTopic>,
}

fn queue_requires_prompt_cache_reconcile(
    cached: &CachedSubscriptionTopic,
    is_working: bool,
    reconcile_required: bool,
    working_reconcile_required: bool,
) -> bool {
    if is_working {
        working_reconcile_required || cached.dirty
    } else {
        reconcile_required || cached.dirty
    }
}

fn collect_prompt_cache_projection_batch(
    state: &AppState,
    mutations: &[SequencedRuntimeMutation],
) -> PromptCacheProjectionBatch {
    let mut batch = PromptCacheProjectionBatch {
        records: Vec::new(),
        reconcile_required: false,
        working_key_hydration_keys: BTreeSet::new(),
        working_reconcile_required: false,
    };
    for mutation in mutations {
        let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
            continue;
        };
        if mutation.prompt_cache_key.is_none() && mutation.sticky_key.is_none() {
            continue;
        }
        let runtime_projection = (mutation.kind != RuntimeMutationKind::RuntimeRemoved)
            .then(|| {
                state
                    .proxy_runtime_invocations
                    .prompt_cache_projection_by_identity(
                        &mutation.identity.invoke_id,
                        &mutation.identity.occurred_at,
                    )
            })
            .flatten();
        match PromptCacheTopicDelta::from_runtime_mutation(mutation, runtime_projection.as_ref()) {
            Ok(Some(record)) => batch.records.push(record),
            Ok(None) => {
                batch.reconcile_required = true;
                if mutation.is_terminal
                    && mutation.row_id.is_some_and(|row_id| row_id > 0)
                    && let Some(prompt_cache_key) = mutation.prompt_cache_key.as_deref()
                {
                    batch
                        .working_key_hydration_keys
                        .insert(prompt_cache_key.to_string());
                } else {
                    batch.working_reconcile_required = true;
                }
            }
            Err(err) => {
                batch.reconcile_required = true;
                batch.working_reconcile_required = true;
                warn!(?err, "failed to build active prompt cache topic delta");
            }
        }
    }
    batch
}

async fn hydrate_working_conversation_candidates(
    state: &Arc<AppState>,
    pending_keys: BTreeSet<String>,
    candidate_refill_required: bool,
    page_size: usize,
    recent_invocation_limit: usize,
    blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    visible_keys: HashSet<String>,
) -> Result<
    (
        DateTime<Utc>,
        InvocationSourceScope,
        Vec<String>,
        WorkingConversationHydratedResponses,
        i64,
    ),
    ApiError,
> {
    let range_end = Utc::now();
    let range_start_bound = db_occurred_at_lower_bound(
        range_end
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES),
    );
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let candidate_keys = if candidate_refill_required {
        query_working_prompt_cache_conversation_candidate_keys(
            state.as_ref(),
            source_scope,
            range_end,
            &range_start_bound,
            page_size as i64,
            blocked_binding_filter.as_ref(),
        )
        .await?
    } else {
        Vec::new()
    };
    let mut hydration_keys = pending_keys;
    hydration_keys.extend(
        candidate_keys
            .into_iter()
            .filter(|key| !visible_keys.contains(key)),
    );
    let hydration_keys = hydration_keys
        .into_iter()
        .take(SUBSCRIPTION_CONVERSATION_OPERATION_LIMIT)
        .collect::<Vec<_>>();
    let mut hydrated = Vec::with_capacity(hydration_keys.len());
    for prompt_cache_key in &hydration_keys {
        let response = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            prompt_cache_key,
            range_end,
            &range_start_bound,
            recent_invocation_limit as i64,
            blocked_binding_filter.as_ref(),
        )
        .await?;
        hydrated.push((prompt_cache_key.clone(), response));
    }
    let hydrated_visible_keys = hydrated
        .iter()
        .filter_map(|(prompt_cache_key, response)| {
            response.as_ref().map(|_| prompt_cache_key.clone())
        })
        .collect::<HashSet<_>>();
    let total_matched = query_working_prompt_cache_conversation_total_matched(
        state.as_ref(),
        source_scope,
        range_end,
        &range_start_bound,
        blocked_binding_filter.as_ref(),
        &hydrated_visible_keys,
    )
    .await?;
    Ok((
        range_end,
        source_scope,
        hydration_keys,
        hydrated,
        total_matched,
    ))
}

impl SubscriptionHub {
    async fn schedule_prompt_cache_topic_projection(
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

        let PromptCacheProjectionBatch {
            records,
            reconcile_required,
            working_key_hydration_keys,
            working_reconcile_required,
        } = collect_prompt_cache_projection_batch(&state, mutations);
        if records.is_empty() && !reconcile_required {
            return;
        }

        let PromptCacheProjectionQueues {
            scheduled,
            key_hydrations,
            reconciles,
        } = self
            .queue_prompt_cache_projection_topics(
                active_topic_keys,
                records,
                reconcile_required,
                working_key_hydration_keys,
                working_reconcile_required,
            )
            .await;

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

    async fn queue_prompt_cache_projection_topics(
        &self,
        active_topic_keys: Vec<String>,
        records: Vec<PromptCacheTopicDelta>,
        reconcile_required: bool,
        working_key_hydration_keys: BTreeSet<String>,
        working_reconcile_required: bool,
    ) -> PromptCacheProjectionQueues {
        let mut guard = self.state.lock().await;
        let mut queues = PromptCacheProjectionQueues {
            scheduled: Vec::new(),
            key_hydrations: Vec::new(),
            reconciles: Vec::new(),
        };
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
            let is_working = matches!(
                topic,
                SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            );
            if queue_requires_prompt_cache_reconcile(
                cached,
                is_working,
                reconcile_required,
                working_reconcile_required,
            ) {
                cached.dirty = true;
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    queues.reconciles.push(topic);
                }
                continue;
            }
            if is_working && !working_key_hydration_keys.is_empty() {
                cached
                    .prompt_cache_pending_key_hydrations
                    .extend(working_key_hydration_keys.iter().cloned());
                if !cached.prompt_cache_key_hydration_scheduled {
                    cached.prompt_cache_key_hydration_scheduled = true;
                    queues.key_hydrations.push(topic.clone());
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
                queues.scheduled.push(topic);
            }
        }
        queues
    }

    fn spawn_prompt_cache_topic_reconcile(state: Arc<AppState>, topic: SubscriptionTopic) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            loop {
                let Some(delay) = hub.prompt_cache_topic_reconcile_delay(&topic).await else {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                };
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = state.shutdown.cancelled() => {
                        hub.finish_prompt_cache_topic_reconcile(&topic).await;
                        return;
                    }
                }
                let Ok(topic_key) = topic.cache_key() else {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                };
                if !hub.has_active_topic_key(&topic_key).await {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                }
                let gate = crate::db_pressure::global_db_pressure_gate();
                let observed_eligibility = gate.eligibility_generation();
                match gate.try_begin_background("prompt_cache_topic_reconcile") {
                    Ok(_permit) => {
                        let result = hub
                            .refresh_topic_if_active(state.clone(), topic.clone(), true)
                            .await;
                        hub.finish_prompt_cache_topic_reconcile(&topic).await;
                        if let Err(err) = result {
                            warn!(
                                ?err,
                                topic = topic.name(),
                                response_source = "last_good",
                                "bounded prompt cache topic reconcile failed"
                            );
                            hub.mark_topic_dirty(&topic).await;
                            hub.set_prompt_cache_pressure_deferred(&topic, false).await;
                        }
                        return;
                    }
                    Err(reason) => {
                        hub.set_prompt_cache_pressure_deferred(&topic, true).await;
                        tracing::debug!(
                            topic = %topic.name(),
                            reconcile_outcome = "pressure_deferred",
                            defer_reason = %reason,
                            "prompt cache topic reconcile deferred"
                        );
                        tokio::select! {
                            _ = wait_for_prompt_cache_reconcile_eligibility(
                                gate,
                                observed_eligibility,
                                reason,
                            ) => {}
                            _ = state.shutdown.cancelled() => {
                                hub.finish_prompt_cache_topic_reconcile(&topic).await;
                                return;
                            }
                        }
                    }
                }
            }
        });
    }

    async fn finish_prompt_cache_topic_reconcile(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_reconcile_scheduled = false;
        }
    }

    async fn set_prompt_cache_pressure_deferred(
        &self,
        topic: &SubscriptionTopic,
        pressure_deferred: bool,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_pressure_deferred = pressure_deferred;
        }
    }

    fn spawn_prompt_cache_topic_materialization(state: Arc<AppState>, topic: SubscriptionTopic) {
        let hub = state.subscription_hub.clone();
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
                    SubscriptionHub::spawn_prompt_cache_topic_reconcile(state, topic);
                }
            }
        });
    }

    fn spawn_dashboard_working_conversation_key_hydration(
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            loop {
                let gate = crate::db_pressure::global_db_pressure_gate();
                let observed_eligibility = gate.eligibility_generation();
                match gate.try_begin_background("dashboard_working_conversation_key_hydrate") {
                    Ok(_permit) => {
                        if let Err(err) = hub
                            .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
                            .await
                        {
                            warn!(
                                ?err,
                                topic = topic.name(),
                                response_source = "last_good",
                                "bounded dashboard working conversation hydrate failed"
                            );
                            hub.finish_dashboard_working_conversation_key_hydration(&topic)
                                .await;
                            if hub
                                .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                                .await
                            {
                                SubscriptionHub::spawn_prompt_cache_topic_reconcile(
                                    state.clone(),
                                    topic,
                                );
                            }
                        }
                        return;
                    }
                    Err(reason) => {
                        hub.set_prompt_cache_pressure_deferred(&topic, true).await;
                        tracing::debug!(
                            topic = %topic.name(),
                            hydrate_outcome = "pressure_deferred",
                            defer_reason = %reason,
                            "dashboard working conversation key hydrate deferred"
                        );
                        tokio::select! {
                            _ = wait_for_prompt_cache_reconcile_eligibility(
                                gate,
                                observed_eligibility,
                                reason,
                            ) => {}
                            _ = state.shutdown.cancelled() => {
                                hub.finish_dashboard_working_conversation_key_hydration(&topic)
                                    .await;
                                return;
                            }
                        }
                    }
                }
            }
        });
    }

    async fn finish_dashboard_working_conversation_key_hydration(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_key_hydration_scheduled = false;
        }
    }

    async fn take_working_conversation_hydration_snapshot(
        &self,
        topic_key: &str,
    ) -> Result<Option<WorkingConversationHydrationSnapshot>, ApiError> {
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(topic_key) else {
            return Ok(None);
        };
        if active == 0 || cached.dirty {
            cached.prompt_cache_key_hydration_scheduled = false;
            return Ok(None);
        }
        let Some(DashboardTopicMaterializer::WorkingConversations { state }) =
            cached.dashboard_materializer.as_ref()
        else {
            cached.prompt_cache_key_hydration_scheduled = false;
            return Ok(None);
        };
        let state = state
            .lock()
            .expect("working conversations materializer state lock");
        Ok(Some((
            cached.prompt_cache_pending_key_hydrations.clone(),
            cached.prompt_cache_pending_records.clone(),
            cached.prompt_cache_candidate_refill_required,
            state.page_size,
            state.recent_invocation_limit,
            state.blocked_binding_filter.clone(),
            state.visible_keys(),
        )))
    }

    async fn hydrate_dashboard_working_conversation_keys(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let Some((
            pending_keys,
            pending_records_at_hydration_start,
            candidate_refill_required,
            page_size,
            recent_invocation_limit,
            blocked_binding_filter,
            visible_keys,
        )) = self
            .take_working_conversation_hydration_snapshot(&topic_key)
            .await?
        else {
            return Ok(());
        };

        let (_range_end, _source_scope, hydration_keys, hydrated, total_matched) =
            hydrate_working_conversation_candidates(
                &state,
                pending_keys,
                candidate_refill_required,
                page_size,
                recent_invocation_limit,
                blocked_binding_filter.clone(),
                visible_keys,
            )
            .await?;

        let actions = self
            .commit_working_conversation_hydration(WorkingConversationHydrationCommitRequest {
                topic_key: &topic_key,
                pending_records_at_hydration_start: &pending_records_at_hydration_start,
                hydration_keys: &hydration_keys,
                hydrated,
                total_matched,
            })
            .await?;
        let WorkingConversationHydrationCommitActions {
            dispatch,
            next_hydration,
            next_materialization,
            recovery,
        } = actions;
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        if let Some(topic) = next_hydration {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        if let Some(topic) = next_materialization {
            Self::spawn_prompt_cache_topic_materialization(state.clone(), topic);
        }
        if let Some(topic) = recovery {
            Self::spawn_prompt_cache_topic_reconcile(state, topic);
        }
        Ok(())
    }

    async fn commit_working_conversation_hydration(
        &self,
        request: WorkingConversationHydrationCommitRequest<'_>,
    ) -> Result<WorkingConversationHydrationCommitActions, ApiError> {
        let WorkingConversationHydrationCommitRequest {
            topic_key,
            pending_records_at_hydration_start,
            hydration_keys,
            hydrated,
            total_matched,
        } = request;
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(topic_key) else {
            return Ok(WorkingConversationHydrationCommitActions {
                dispatch: None,
                next_hydration: None,
                next_materialization: None,
                recovery: None,
            });
        };
        if active == 0 || cached.dirty {
            cached.prompt_cache_key_hydration_scheduled = false;
            return Ok(WorkingConversationHydrationCommitActions {
                dispatch: None,
                next_hydration: None,
                next_materialization: None,
                recovery: None,
            });
        }
        let Some(DashboardTopicMaterializer::WorkingConversations { state: current }) =
            cached.dashboard_materializer.as_ref()
        else {
            cached.prompt_cache_key_hydration_scheduled = false;
            return Ok(WorkingConversationHydrationCommitActions {
                dispatch: None,
                next_hydration: None,
                next_materialization: None,
                recovery: None,
            });
        };
        let current = current.clone();
        let mut projection = current
            .lock()
            .expect("working conversations materializer state lock");
        let WorkingConversationHydrationStateUpdate {
            changed,
            next_hydration,
            next_materialization,
            recovery,
        } = apply_working_conversation_hydration_state(
            cached,
            &mut projection,
            pending_records_at_hydration_start,
            hydration_keys,
            hydrated,
            total_matched,
        )?;
        let dispatch = if changed {
            self.dashboard_topology_counters
                .record_materialization(cached.topic.name(), false);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.to_string(),
                cached.schema_epoch.clone(),
                cached.cursor.saturating_add(1),
                projection.serialize()?,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = frame.cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "database_bounded_key_hydrate";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent { frame })
        } else {
            None
        };
        Ok(WorkingConversationHydrationCommitActions {
            dispatch,
            next_hydration,
            next_materialization,
            recovery,
        })
    }

    async fn materialize_prompt_cache_topic(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let (dispatch, bounded_hydration_topic) = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            cached.prompt_cache_refresh_scheduled = false;
            if active == 0 {
                cached.prompt_cache_pending_records.clear();
                return Ok(());
            }
            let records = std::mem::take(&mut cached.prompt_cache_pending_records)
                .into_values()
                .collect::<Vec<_>>();
            if records.is_empty() {
                return Ok(());
            }
            let working_state = match cached.dashboard_materializer.as_ref() {
                Some(DashboardTopicMaterializer::WorkingConversations { state }) => {
                    Some(state.clone())
                }
                _ => None,
            };
            let (applied, hydration_topic) = apply_prompt_cache_materialization_records(
                topic,
                cached,
                working_state.as_ref(),
                &records,
            )?;
            let dispatch = if applied != Some(true) {
                None
            } else {
                let serialized_payload = serialize_prompt_cache_materializer_payload(
                    working_state.as_ref(),
                    &cached.snapshot_payload,
                )?;
                Some(self.commit_prompt_cache_topic_frame(
                    cached,
                    &topic_key,
                    serialized_payload,
                    Utc::now(),
                )?)
            };
            (dispatch, hydration_topic)
        };
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    async fn prompt_cache_reconcile_required(&self, topic_key: &str) -> bool {
        self.state
            .lock()
            .await
            .topics
            .get(topic_key)
            .is_some_and(|cached| {
                cached.prompt_cache_reconcile_required
                    || cached.dirty
                    || !cached.prompt_cache_applied_terminal_ids.is_empty()
            })
    }
}

fn apply_prompt_cache_materialization_records(
    topic: &SubscriptionTopic,
    cached: &mut CachedSubscriptionTopic,
    working_state: Option<&Arc<StdMutex<DashboardWorkingConversationsMaterializerState>>>,
    records: &[PromptCacheTopicDelta],
) -> Result<(Option<bool>, Option<SubscriptionTopic>), ApiError> {
    let mut hydration_topic = None;
    let applied = if let Some(working_state) = working_state {
        let update = working_state
            .lock()
            .expect("working conversations materializer state lock")
            .apply_deltas(
                records,
                &mut cached.prompt_cache_applied_terminal_ids,
                cached.prompt_cache_baseline_row_id,
            )?;
        match update {
            WorkingConversationsProjectionUpdate::Unchanged => Some(false),
            WorkingConversationsProjectionUpdate::Changed => Some(true),
            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(keys) => {
                for record in records {
                    cached
                        .prompt_cache_pending_records
                        .insert(record.identity.clone(), record.clone());
                }
                cached.prompt_cache_pending_key_hydrations.extend(keys);
                if !cached.prompt_cache_key_hydration_scheduled {
                    cached.prompt_cache_key_hydration_scheduled = true;
                    hydration_topic = Some(cached.topic.clone());
                }
                None
            }
            WorkingConversationsProjectionUpdate::NeedsReconcile => {
                for record in records {
                    cached
                        .prompt_cache_pending_records
                        .insert(record.identity.clone(), record.clone());
                }
                return Err(ApiError::from(anyhow!(
                    "working conversations projection requires bounded reconcile"
                )));
            }
        }
    } else {
        match apply_prompt_cache_records_to_payload(
            topic,
            &mut cached.snapshot_payload,
            records,
            &mut cached.prompt_cache_applied_terminal_ids,
            cached.prompt_cache_baseline_row_id,
        ) {
            Ok(applied) => Some(applied),
            Err(err) => {
                for record in records {
                    cached
                        .prompt_cache_pending_records
                        .insert(record.identity.clone(), record.clone());
                }
                return Err(err);
            }
        }
    };
    Ok((applied, hydration_topic))
}

fn serialize_prompt_cache_materializer_payload(
    working_state: Option<&Arc<StdMutex<DashboardWorkingConversationsMaterializerState>>>,
    snapshot_payload: &Value,
) -> Result<Vec<u8>, ApiError> {
    match working_state {
        Some(working_state) => working_state
            .lock()
            .expect("working conversations materializer state lock")
            .serialize(),
        None => serde_json::to_vec(snapshot_payload).map_err(ApiError::from),
    }
}
