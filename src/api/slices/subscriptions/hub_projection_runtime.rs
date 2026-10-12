use super::*;
impl SubscriptionHub {
    pub(crate) fn spawn_prompt_cache_topic_reconcile(
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
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

    pub(crate) async fn finish_prompt_cache_topic_reconcile(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_reconcile_scheduled = false;
        }
    }

    pub(crate) async fn set_prompt_cache_pressure_deferred(
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

    pub(crate) fn spawn_prompt_cache_topic_materialization(
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
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

    pub(crate) fn spawn_dashboard_working_conversation_key_hydration(
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

    pub(crate) async fn finish_dashboard_working_conversation_key_hydration(
        &self,
        topic: &SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_key_hydration_scheduled = false;
        }
    }

    pub(crate) async fn hydrate_dashboard_working_conversation_keys(
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
        )) = ({
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 || cached.dirty {
                cached.prompt_cache_key_hydration_scheduled = false;
                None
            } else {
                let Some(DashboardTopicMaterializer::WorkingConversations { state }) =
                    cached.dashboard_materializer.as_ref()
                else {
                    cached.prompt_cache_key_hydration_scheduled = false;
                    return Ok(());
                };
                let working_state = state.clone();
                let state = working_state
                    .lock()
                    .expect("working conversations materializer state lock");
                Some((
                    cached.prompt_cache_pending_key_hydrations.clone(),
                    cached.prompt_cache_pending_records.clone(),
                    cached.prompt_cache_candidate_refill_required,
                    state.page_size,
                    state.recent_invocation_limit,
                    state.blocked_binding_filter.clone(),
                    state.visible_keys(),
                ))
            }
        })
        else {
            return Ok(());
        };

        let range_end = Utc::now();
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
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
            let (response, terminal_identities) =
                hydrate_working_prompt_cache_conversation_for_key_with_terminal_identities(
                    state.as_ref(),
                    source_scope,
                    prompt_cache_key,
                    range_end,
                    &range_start_bound,
                    recent_invocation_limit as i64,
                    blocked_binding_filter.as_ref(),
                )
                .await?;
            hydrated.push((prompt_cache_key.clone(), response, terminal_identities));
        }
        let hydrated_visible_keys = hydrated
            .iter()
            .filter_map(|(prompt_cache_key, response, _)| {
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

        let (dispatch, next_hydration, next_materialization, recovery) = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 || cached.dirty {
                cached.prompt_cache_key_hydration_scheduled = false;
                return Ok(());
            }
            let Some(DashboardTopicMaterializer::WorkingConversations { state: current }) =
                cached.dashboard_materializer.as_ref()
            else {
                cached.prompt_cache_key_hydration_scheduled = false;
                return Ok(());
            };
            let current = current.clone();
            let mut projection = current
                .lock()
                .expect("working conversations materializer state lock");
            let now = Utc::now();
            let changed_pending_keys = prompt_cache_hydration_changed_pending_keys(
                &pending_records_at_hydration_start,
                &cached.prompt_cache_pending_records,
                &hydration_keys,
            );
            let unresolved_eligible_delta =
                hydrated.iter().any(|(prompt_cache_key, response, _)| {
                    response.is_none()
                        && cached.prompt_cache_pending_records.values().any(|record| {
                            record.prompt_cache_key.as_deref() == Some(prompt_cache_key.as_str())
                                && projection.delta_is_eligible(record, now)
                        })
                });
            if !changed_pending_keys.is_empty() {
                // A newer delta for an already-hydrated key is not represented by this bounded
                // snapshot. Retry that same key immediately instead of making the whole working
                // selection dirty and delaying exact recovery to the 60-second full cadence.
                cached
                    .prompt_cache_pending_key_hydrations
                    .extend(changed_pending_keys);
                cached.prompt_cache_key_hydration_scheduled = true;
                cached.prompt_cache_pressure_deferred = false;
                (None, Some(cached.topic.clone()), None, None)
            } else if unresolved_eligible_delta {
                // A bounded snapshot can race a just-arrived runtime delta or omit an eligible
                // one. Retain the last-good frame and recover instead of clearing a delta that
                // was not proven to be represented by the hydrate result.
                cached.dirty = true;
                cached.prompt_cache_reconcile_required = true;
                cached.prompt_cache_key_hydration_scheduled = false;
                cached.prompt_cache_pressure_deferred = false;
                let recovery = (!cached.prompt_cache_reconcile_scheduled).then(|| {
                    cached.prompt_cache_reconcile_scheduled = true;
                    cached.topic.clone()
                });
                (None, None, None, recovery)
            } else {
                let mut changed = false;
                for (prompt_cache_key, response, terminal_identities) in hydrated {
                    cached
                        .prompt_cache_applied_terminal_ids
                        .extend(terminal_identities);
                    for record in pending_records_at_hydration_start
                        .values()
                        .chain(cached.prompt_cache_pending_records.values())
                    {
                        if record.prompt_cache_key.as_deref() == Some(prompt_cache_key.as_str())
                            && record.is_terminal
                        {
                            cached
                                .prompt_cache_applied_terminal_ids
                                .insert(record.identity.clone());
                        }
                    }
                    // The bounded database/runtime hydrate includes every pending record for
                    // this key, so dropping those records avoids both double-application and a
                    // later full-window reconcile solely for terminal de-duplication bookkeeping.
                    cached.prompt_cache_pending_records.retain(|_, record| {
                        record.prompt_cache_key.as_deref() != Some(prompt_cache_key.as_str())
                    });
                    cached
                        .prompt_cache_pending_key_hydrations
                        .remove(&prompt_cache_key);
                    changed |=
                        projection.replace_hydrated_conversation(&prompt_cache_key, response);
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
                let dispatch = if changed {
                    let next_cursor = cached.cursor.saturating_add(1);
                    self.dashboard_topology_counters
                        .record_materialization(cached.topic.name(), false);
                    let frame = Arc::new(self.serialize_frame(
                        cached.descriptor.clone(),
                        topic_key.clone(),
                        cached.schema_epoch.clone(),
                        next_cursor,
                        projection.serialize()?,
                    )?);
                    let retained_bytes = frame.retained_bytes();
                    cached.cursor = next_cursor;
                    cached.snapshot_frame = frame.clone();
                    cached.snapshot_bytes = frame.payload_bytes.len();
                    cached.replay_events.push_back(ReplayableTopicEvent {
                        frame: frame.clone(),
                        full_frame: frame.clone(),
                        bytes: retained_bytes,
                        emitted_at: Utc::now(),
                    });
                    cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
                    cached.prompt_cache_response_source = "database_bounded_key_hydrate";
                    prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
                    Some(SubscriptionDispatchEvent {
                        frame: frame.clone(),
                        full_frame: frame,
                    })
                } else {
                    None
                };
                (dispatch, next_hydration, next_materialization, None)
            }
        };
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

    pub(crate) async fn materialize_prompt_cache_topic(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let mut bounded_hydration_topic = None;
        let dispatch = {
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
            let is_working_conversations = working_state.is_some();
            let applied = if let Some(working_state) = working_state.as_ref() {
                let update = working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .apply_deltas(
                        &records,
                        &mut cached.prompt_cache_applied_terminal_ids,
                        cached.prompt_cache_baseline_row_id,
                    )?;
                match update {
                    WorkingConversationsProjectionUpdate::Unchanged => Some(false),
                    WorkingConversationsProjectionUpdate::Changed => Some(true),
                    WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(keys) => {
                        for record in &records {
                            cached
                                .prompt_cache_pending_records
                                .insert(record.identity.clone(), record.clone());
                        }
                        cached.prompt_cache_pending_key_hydrations.extend(keys);
                        if !cached.prompt_cache_key_hydration_scheduled {
                            cached.prompt_cache_key_hydration_scheduled = true;
                            bounded_hydration_topic = Some(cached.topic.clone());
                        }
                        None
                    }
                    WorkingConversationsProjectionUpdate::NeedsReconcile => {
                        for record in &records {
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
                    &records,
                    &mut cached.prompt_cache_applied_terminal_ids,
                    cached.prompt_cache_baseline_row_id,
                ) {
                    Ok(applied) => Some(applied),
                    Err(err) => {
                        for record in &records {
                            cached
                                .prompt_cache_pending_records
                                .insert(record.identity.clone(), record.clone());
                        }
                        return Err(err);
                    }
                }
            };
            if applied != Some(true) {
                None
            } else {
                let next_cursor = cached.cursor.saturating_add(1);
                if is_working_conversations {
                    self.dashboard_topology_counters
                        .record_materialization(topic.name(), false);
                }
                let serialized_payload = if let Some(working_state) = working_state {
                    working_state
                        .lock()
                        .expect("working conversations materializer state lock")
                        .serialize()?
                } else {
                    serde_json::to_vec(&cached.snapshot_payload)?
                };
                let frame = Arc::new(self.serialize_frame(
                    cached.descriptor.clone(),
                    topic_key.clone(),
                    cached.schema_epoch.clone(),
                    next_cursor,
                    serialized_payload,
                )?);
                let retained_bytes = frame.retained_bytes();
                cached.cursor = next_cursor;
                cached.snapshot_frame = frame.clone();
                cached.snapshot_bytes = frame.payload_bytes.len();
                cached.replay_events.push_back(ReplayableTopicEvent {
                    frame: frame.clone(),
                    full_frame: frame.clone(),
                    bytes: retained_bytes,
                    emitted_at: Utc::now(),
                });
                cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
                cached.prompt_cache_response_source = "memory";
                prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
                tracing::debug!(
                    topic = topic.name(),
                    response_source = "memory",
                    coalesced_event_count = records.len(),
                    live_path_db_read_count = 0_u64,
                    baseline_age_ms = cached
                        .prompt_cache_baseline_at
                        .map(|started| started.elapsed().as_millis() as u64)
                        .unwrap_or_default(),
                    "prompt cache topic materialized from active projection"
                );
                Some(SubscriptionDispatchEvent {
                    frame: frame.clone(),
                    full_frame: frame,
                })
            }
        };
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    pub(crate) async fn prompt_cache_reconcile_required(&self, topic_key: &str) -> bool {
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

    pub(crate) async fn expire_dashboard_working_conversations_projection(
        &self,
        state: Arc<AppState>,
        topic_key: &str,
    ) -> Result<(), ApiError> {
        let mut bounded_hydration_topic = None;
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(topic_key) else {
                return Ok(());
            };
            if cached.dirty {
                return Ok(());
            }
            let Some(DashboardTopicMaterializer::WorkingConversations { state }) =
                cached.dashboard_materializer.as_ref()
            else {
                return Ok(());
            };
            let state = state.clone();
            let changed = state
                .lock()
                .expect("working conversations materializer state lock")
                .expire(Utc::now());
            if !changed {
                return Ok(());
            }
            // Expiring an item can uncover a persisted candidate outside this page. Refill only
            // the bounded active page rather than rebuilding the entire working selection.
            cached.prompt_cache_candidate_refill_required = true;
            if !cached.prompt_cache_key_hydration_scheduled {
                cached.prompt_cache_key_hydration_scheduled = true;
                bounded_hydration_topic = Some(cached.topic.clone());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            self.dashboard_topology_counters
                .record_materialization(cached.topic.name(), false);
            let frame = Arc::new(
                self.serialize_frame(
                    cached.descriptor.clone(),
                    topic_key.to_string(),
                    cached.schema_epoch.clone(),
                    next_cursor,
                    state
                        .lock()
                        .expect("working conversations materializer state lock")
                        .serialize()?,
                )?,
            );
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            })
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        Ok(())
    }

    pub(crate) async fn expire_prompt_cache_topic_window(
        &self,
        topic_key: &str,
    ) -> Result<(), ApiError> {
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(topic_key) else {
                return Ok(());
            };
            if cached.dirty {
                // A cursor gap makes the retained frame incomplete. Keep it byte-for-byte as
                // last-good until the bounded reconcile below replaces it.
                return Ok(());
            }
            let now = Utc::now();
            let activity_cutoff = match cached.topic {
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::Count(_),
                    ..
                }
                | SubscriptionTopic::PromptCacheStickyWindow {
                    selection: AccountStickyKeySelection::Count(_),
                    ..
                } => Some(now - ChronoDuration::hours(24)),
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::ActivityWindowHours(hours),
                    ..
                } => Some(now - ChronoDuration::hours(hours)),
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::ActivityWindowMinutes(minutes),
                    ..
                } => Some(now - ChronoDuration::minutes(minutes)),
                SubscriptionTopic::PromptCacheStickyWindow {
                    selection: AccountStickyKeySelection::ActivityWindow(hours),
                    ..
                } => Some(now - ChronoDuration::hours(hours)),
                _ => None,
            };
            let Some(conversations) = cached
                .snapshot_payload
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            else {
                return Ok(());
            };
            let before = conversations.len();
            if let Some(cutoff) = activity_cutoff {
                conversations.retain(|conversation| {
                    conversation
                        .get("lastActivityAt")
                        .and_then(Value::as_str)
                        .and_then(parse_to_utc_datetime)
                        .is_some_and(|last_activity| last_activity >= cutoff)
                });
            }
            let mut changed = conversations.len() != before;
            if changed {
                cached.prompt_cache_reconcile_required = true;
            }
            let request_cutoff = now - ChronoDuration::hours(24);
            for conversation in conversations {
                let Some(points) = conversation
                    .get_mut("last24hRequests")
                    .and_then(Value::as_array_mut)
                else {
                    continue;
                };
                let before = points.len();
                points.retain(|point| {
                    point
                        .get("occurredAt")
                        .and_then(Value::as_str)
                        .and_then(parse_to_utc_datetime)
                        .is_some_and(|occurred_at| occurred_at >= request_cutoff)
                });
                changed |= points.len() != before;
                let mut cumulative = 0_i64;
                for point in points {
                    cumulative = cumulative.saturating_add(
                        point
                            .get("requestTokens")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    );
                    if let Some(point) = point.as_object_mut() {
                        point.insert("cumulativeTokens".to_string(), Value::from(cumulative));
                    }
                }
            }
            if !changed {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.to_string(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&cached.snapshot_payload)?,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: now,
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            })
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    pub(crate) async fn apply_prompt_cache_binding_projection(
        &self,
        state: Arc<AppState>,
        prompt_cache_key: &str,
    ) -> Result<(), ApiError> {
        let active_topic_keys = {
            let guard = self.state.lock().await;
            let mut topic_keys = Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheWindow,
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
            return Ok(());
        }
        let binding = load_prompt_cache_conversation_binding_response_for_key(
            state.as_ref(),
            prompt_cache_key.to_string(),
        )
        .await?;
        let mut dispatches = Vec::new();
        let mut reconciles = Vec::new();
        let mut key_hydrations = Vec::new();
        let mut binding_payload = None;
        let mut guard = self.state.lock().await;
        for topic_key in active_topic_keys {
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                continue;
            }
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                continue;
            };
            if cached.dirty {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            }
            let working_state = match cached.dashboard_materializer.as_ref() {
                Some(DashboardTopicMaterializer::WorkingConversations { state }) => {
                    Some(state.clone())
                }
                _ => None,
            };
            let is_working_conversations = working_state.is_some();
            if !is_working_conversations {
                cached.prompt_cache_bounded_key_hydration_count = cached
                    .prompt_cache_bounded_key_hydration_count
                    .saturating_add(1);
            }
            let changed = if let Some(working_state) = working_state.as_ref() {
                let Some(changed) = working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .apply_binding(prompt_cache_key, &binding)
                else {
                    cached
                        .prompt_cache_pending_key_hydrations
                        .insert(prompt_cache_key.to_string());
                    cached.prompt_cache_candidate_refill_required = true;
                    if !cached.prompt_cache_key_hydration_scheduled {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        key_hydrations.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            } else {
                if binding_payload.is_none() {
                    binding_payload = Some(serde_json::to_value(&binding)?);
                }
                let Some(changed) = patch_prompt_cache_binding_payload(
                    &mut cached.snapshot_payload,
                    prompt_cache_key,
                    binding_payload
                        .as_ref()
                        .expect("serialized binding payload"),
                ) else {
                    cached.prompt_cache_reconcile_required = true;
                    if !cached.prompt_cache_reconcile_scheduled {
                        cached.prompt_cache_reconcile_scheduled = true;
                        reconciles.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            };
            if !changed {
                continue;
            }
            let next_cursor = cached.cursor.saturating_add(1);
            if is_working_conversations {
                self.dashboard_topology_counters
                    .record_materialization(cached.topic.name(), false);
            }
            let serialized_payload = if let Some(working_state) = working_state {
                working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .serialize()?
            } else {
                serde_json::to_vec(&cached.snapshot_payload)?
            };
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            dispatches.push(SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            });
        }
        drop(guard);
        for dispatch in dispatches {
            let _ = self.broadcaster.send(dispatch);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
        for topic in key_hydrations {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        Ok(())
    }

    pub(crate) async fn apply_prompt_cache_sticky_route_projection(
        &self,
        state: Arc<AppState>,
        sticky_key: &str,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    ) -> Result<(), ApiError> {
        let mut dispatches = Vec::new();
        let mut reconciles = Vec::new();
        let mut guard = self.state.lock().await;
        let active_topic_keys = Self::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::PromptCacheStickyWindow,
        );
        if active_topic_keys.is_empty() {
            return Ok(());
        }
        for topic_key in active_topic_keys {
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                continue;
            }
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                continue;
            };
            if cached.dirty {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            }
            let SubscriptionTopic::PromptCacheStickyWindow { account_id, .. } = cached.topic else {
                continue;
            };
            let Some(conversations) = cached
                .snapshot_payload
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            else {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            };
            let before = conversations.len();
            if account_id == previous_upstream_account_id && account_id != upstream_account_id {
                conversations.retain(|conversation| {
                    conversation.get("stickyKey").and_then(Value::as_str) != Some(sticky_key)
                });
            }
            if account_id == upstream_account_id
                && !conversations.iter().any(|conversation| {
                    conversation.get("stickyKey").and_then(Value::as_str) == Some(sticky_key)
                })
            {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
            }
            if conversations.len() == before {
                continue;
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&cached.snapshot_payload)?,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            dispatches.push(SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            });
        }
        drop(guard);
        for dispatch in dispatches {
            let _ = self.broadcaster.send(dispatch);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
        Ok(())
    }

    pub(crate) async fn clear_dashboard_activity_topic_refresh_flag(
        &self,
        topic: &SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.refresh_scheduled = false;
        }
    }

    pub(crate) async fn mark_dashboard_activity_topic_dirty(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.invalidate_upstream_account_attempt_refresh();
            cached.latest_live_snapshot = None;
        }
    }

    pub(crate) async fn reconcile_dashboard_terminal_window_bases(&self, state: Arc<AppState>) {
        let topics = {
            let mut guard = self.state.lock().await;
            mark_dashboard_terminal_window_rebase_topics(&mut guard);
            let active_subscribers = guard.active_subscribers.clone();
            guard
                .topics
                .iter()
                .filter(|(topic_key, cached)| {
                    cached.dirty
                        && cached.refresh_scheduled
                        && active_subscribers
                            .get(*topic_key)
                            .copied()
                            .unwrap_or_default()
                            > 0
                        && cached.dashboard_materializer.as_ref().is_some_and(
                            DashboardTopicMaterializer::requires_terminal_window_rebase,
                        )
                })
                .map(|(_, cached)| cached.topic.clone())
                .collect::<Vec<_>>()
        };

        for topic in topics {
            tracing::debug!(
                topic = %topic.name(),
                refresh_reason = "runtime_reconcile_window_rebase",
                "rebuilding typed Dashboard base after moving-window boundary"
            );
            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %topic.name(),
                    "runtime Dashboard window rebase failed; retaining last-good frame"
                );
            }
        }
    }

    pub(crate) async fn schedule_dashboard_activity_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let selection = dashboard_activity_snapshot_selection_for_topic(state.as_ref(), &topic)
            .await?
            .expect("dashboard activity refresh selection should exist for open-range topics");
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let age = cached.snapshot_built_at.elapsed();
            if age >= DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL {
                cached.refresh_scheduled = false;
                None
            } else if cached.refresh_scheduled {
                return Ok(());
            } else {
                cached.refresh_scheduled = true;
                Some(DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL.saturating_sub(age))
            }
        };

        if let Some(delay) = delay {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if !hub.has_active_topic_key(&topic_key).await {
                    tracing::debug!(
                        topic = %topic.name(),
                        refresh_outcome = "marked_dirty",
                        "skipping deferred dashboard activity refresh without owner subscribers"
                    );
                    hub.mark_dashboard_activity_topic_dirty(&topic).await;
                    return;
                }
                tracing::debug!(
                    refresh_reason = "scheduled_terminal_refresh",
                    response_source = "memory",
                    selection_fingerprint = dashboard_activity_selection_fingerprint(&selection),
                    "publishing dashboard activity read model after terminal coalescing"
                );
                match hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    Ok(Some(_)) | Ok(None) => {}
                    Err(err) => {
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            "failed to run deferred dashboard activity topic refresh"
                        );
                        hub.clear_dashboard_activity_topic_refresh_flag(&topic)
                            .await;
                    }
                }
            });
            return Ok(());
        }

        tracing::debug!(
            refresh_reason = "scheduled_terminal_refresh",
            response_source = "memory",
            selection_fingerprint = dashboard_activity_selection_fingerprint(&selection),
            "publishing dashboard activity read model after terminal coalescing"
        );
        let _ = self.refresh_topic_if_active(state, topic, true).await?;
        Ok(())
    }

    pub(crate) async fn schedule_summary_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        event_count: u64,
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
                cached.latest_live_snapshot = None;
                return Ok(());
            }
            cached.summary_pending_event_count = cached
                .summary_pending_event_count
                .saturating_add(event_count.max(1));
            if cached.summary_refresh_scheduled || cached.summary_refresh_in_flight {
                return Ok(());
            }
            cached.summary_refresh_scheduled = true;
            Duration::from_millis(
                cached
                    .summary_retry_backoff_ms
                    .max(SUMMARY_TOPIC_REFRESH_DEBOUNCE.as_millis() as u64),
            )
        };

        self.spawn_summary_topic_refresh(state, topic, delay);
        Ok(())
    }

    pub(crate) fn spawn_summary_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        delay: Duration,
    ) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            hub.run_summary_topic_refresh(state, topic).await;
        });
    }

    pub(crate) async fn run_summary_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let (event_count, active) = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            if active == 0 {
                cached.dirty = true;
                cached.latest_live_snapshot = None;
                cached.summary_refresh_scheduled = false;
                cached.summary_pending_event_count = 0;
                return;
            }
            cached.summary_refresh_in_flight = true;
            let event_count = std::mem::take(&mut cached.summary_pending_event_count);
            (event_count, active)
        };

        let started = Instant::now();
        let result = match state.sqlite_batch_writer.flush_now(&state.pool).await {
            Ok(()) => {
                self.refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
            }
            Err(err) => Err(ApiError::from(anyhow!(
                "summary topic prerequisite flush failed: {err}"
            ))),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut retry_delay = None;
        let mut coalesced_event_count = event_count;
        let mut refresh_outcome = "published";
        {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            cached.summary_refresh_in_flight = false;
            match result {
                Ok(Some(_)) => {
                    cached.summary_retry_backoff_ms = 0;
                    if cached.summary_pending_event_count > 0 {
                        cached.summary_refresh_scheduled = true;
                        retry_delay = Some(SUMMARY_TOPIC_REFRESH_DEBOUNCE);
                    } else {
                        cached.summary_refresh_scheduled = false;
                    }
                }
                Ok(None) => {
                    cached.dirty = true;
                    cached.latest_live_snapshot = None;
                    cached.summary_refresh_scheduled = false;
                    cached.summary_pending_event_count = 0;
                    refresh_outcome = "marked_dirty";
                }
                Err(err) => {
                    refresh_outcome = "retained_last_good";
                    let backoff_ms = match cached.summary_retry_backoff_ms {
                        0 => 500,
                        500 => 1_000,
                        1_000 => 2_000,
                        _ => 5_000,
                    };
                    cached.summary_retry_backoff_ms = backoff_ms;
                    cached.summary_refresh_scheduled = true;
                    retry_delay = Some(Duration::from_millis(backoff_ms));
                    warn!(
                        ?err,
                        topic = %topic.name(),
                        refresh_outcome,
                        retry_backoff_ms = backoff_ms,
                        last_good_age_ms = cached.snapshot_built_at.elapsed().as_millis() as u64,
                        "summary topic refresh failed; retaining last-good snapshot"
                    );
                }
            }
            coalesced_event_count =
                coalesced_event_count.saturating_add(cached.summary_pending_event_count);
        }
        tracing::debug!(
            topic = %topic.name(),
            active_subscriber_count = active,
            coalesced_event_count,
            build_source = "summary_exact",
            elapsed_ms,
            refresh_outcome,
            "summary topic refresh completed"
        );
        if let Some(delay) = retry_delay {
            self.spawn_summary_topic_refresh(state, topic, delay);
        }
    }

    pub(crate) async fn apply_summary_live_overlay(
        &self,
        topic: &SubscriptionTopic,
        live: DashboardActivityLiveSnapshot,
    ) -> Result<(), ApiError> {
        let SubscriptionTopic::SummaryCurrent {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(());
        };
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let account = upstream_account_id.and_then(|account_id| {
            live.accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(account_id))
        });
        let (count, retry_count, phase_counts, wait_ms) = match account {
            Some(account) => (
                account.in_progress_invocation_count,
                account.retry_invocation_count,
                account.in_progress_phase_counts,
                (account.in_progress_wait_sample_count > 0).then_some(
                    account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
                ),
            ),
            None if upstream_account_id.is_some() => {
                (0, 0, InvocationPhaseCountsResponse::default(), None)
            }
            None => (
                live.in_progress_invocation_count,
                live.retry_invocation_count,
                live.in_progress_phase_counts,
                (live.in_progress_wait_sample_count > 0).then_some(
                    live.in_progress_wait_sum_ms / live.in_progress_wait_sample_count as f64,
                ),
            ),
        };

        let topic_key = topic.cache_key()?;
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            cached.latest_live_snapshot = Some(live.clone());
            let Some(object) = cached.snapshot_payload.as_object_mut() else {
                return Ok(());
            };
            let next_values = [
                ("inProgressConversationCount", Value::from(count)),
                ("inProgressRetryConversationCount", Value::from(retry_count)),
                (
                    "inProgressAvgWaitMs",
                    wait_ms.map(Value::from).unwrap_or(Value::Null),
                ),
                ("inProgressPhaseCounts", serde_json::to_value(phase_counts)?),
            ];
            if next_values
                .iter()
                .all(|(key, value)| object.get(*key) == Some(value))
            {
                return Ok(());
            }
            for (key, value) in next_values {
                set_json_field(object, key, value);
            }
            let payload = cached.snapshot_payload.clone();
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&payload)?,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    pub(crate) async fn apply_dashboard_activity_live_overlay(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
        live: DashboardActivityLiveSnapshot,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            if !apply_dashboard_activity_live_overlay_to_payload(
                state.as_ref(),
                &mut payload,
                &live,
            )? {
                return Ok(());
            }

            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&payload)?,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload.clone();
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);

            SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            }
        };

        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    pub(crate) async fn apply_dashboard_network_slice_to_timeseries(
        &self,
        topic: &SubscriptionTopic,
        slice: &DashboardNetworkProjectionSlice,
    ) -> Result<(), ApiError> {
        let SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(());
        };
        let bucket = match upstream_account_id {
            None => slice.network_live_bucket.clone(),
            Some(upstream_account_id) => slice
                .accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(*upstream_account_id))
                .and_then(|account| account.network_live_bucket.clone()),
        };
        let Some(bucket) = bucket else {
            return Ok(());
        };
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            let Some(object) = payload.as_object_mut() else {
                return Ok(());
            };
            let bucket_value = serde_json::to_value(&bucket)?;
            let bucket_start = bucket_value.get("bucketStart").cloned();
            let Some(points) = object.get_mut("points").and_then(Value::as_array_mut) else {
                return Ok(());
            };
            let point_index = points
                .iter()
                .position(|point| point.get("bucketStart") == bucket_start.as_ref())
                .or_else(|| {
                    points.iter().position(|point| {
                        point
                            .get("isLiveBucket")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                });
            let Some(point_index) = point_index else {
                return Ok(());
            };
            let now = Utc::now();
            points[point_index] = bucket_value;
            object.insert(
                "rangeEnd".to_string(),
                Value::String(format_utc_iso_precise(now)),
            );
            object.insert(
                "snapshotId".to_string(),
                Value::from(now.timestamp_millis()),
            );
            let serialized = serde_json::to_vec(&payload)?;
            if cached.snapshot_frame.payload_bytes.as_ref() == serialized.as_slice() {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload;
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.snapshot_frame = frame.clone();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: now,
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent {
                frame: frame.clone(),
                full_frame: frame,
            }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }
}
