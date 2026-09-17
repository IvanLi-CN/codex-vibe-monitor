#[derive(Default)]
struct PromptCacheBindingProjectionActions {
    dispatches: Vec<SubscriptionDispatchEvent>,
    reconciles: Vec<SubscriptionTopic>,
    key_hydrations: Vec<SubscriptionTopic>,
}

impl SubscriptionHub {
    async fn expire_dashboard_working_conversations_projection(
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
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent { frame })
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        Ok(())
    }

    async fn expire_prompt_cache_topic_window(&self, topic_key: &str) -> Result<(), ApiError> {
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(topic_key) else {
                return Ok(());
            };
            if cached.dirty {
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
            Some(self.commit_prompt_cache_topic_frame(
                cached,
                topic_key,
                serde_json::to_vec(&cached.snapshot_payload)?,
                now,
            )?)
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    fn commit_prompt_cache_topic_frame(
        &self,
        cached: &mut CachedSubscriptionTopic,
        topic_key: &str,
        payload: Vec<u8>,
        emitted_at: DateTime<Utc>,
    ) -> Result<SubscriptionDispatchEvent, ApiError> {
        let next_cursor = cached.cursor.saturating_add(1);
        let frame = Arc::new(self.serialize_frame(
            cached.descriptor.clone(),
            topic_key.to_string(),
            cached.schema_epoch.clone(),
            next_cursor,
            payload,
        )?);
        let retained_bytes = frame.retained_bytes();
        cached.cursor = next_cursor;
        cached.snapshot_frame = frame.clone();
        cached.snapshot_bytes = frame.payload_bytes.len();
        cached.replay_events.push_back(ReplayableTopicEvent {
            frame: frame.clone(),
            bytes: retained_bytes,
            emitted_at,
        });
        cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
        cached.prompt_cache_response_source = "memory";
        prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
        Ok(SubscriptionDispatchEvent { frame })
    }

    async fn apply_prompt_cache_binding_projection(
        &self,
        state: Arc<AppState>,
        prompt_cache_key: &str,
    ) -> Result<(), ApiError> {
        let active_topic_keys = self.active_prompt_cache_binding_topic_keys().await;
        if active_topic_keys.is_empty() {
            return Ok(());
        }
        let binding = load_prompt_cache_conversation_binding_response_for_key(
            state.as_ref(),
            prompt_cache_key.to_string(),
        )
        .await?;
        let mut guard = self.state.lock().await;
        let actions = self.apply_prompt_cache_binding_to_topics(
            &mut guard,
            active_topic_keys,
            prompt_cache_key,
            &binding,
        )?;
        drop(guard);
        for dispatch in actions.dispatches {
            let _ = self.broadcaster.send(dispatch);
        }
        for topic in actions.reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
        for topic in actions.key_hydrations {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        Ok(())
    }

    async fn active_prompt_cache_binding_topic_keys(&self) -> Vec<String> {
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
            .collect()
    }

    fn apply_prompt_cache_binding_to_topics(
        &self,
        guard: &mut SubscriptionHubState,
        active_topic_keys: Vec<String>,
        prompt_cache_key: &str,
        binding: &PromptCacheConversationBindingResponse,
    ) -> Result<PromptCacheBindingProjectionActions, ApiError> {
        let mut actions = PromptCacheBindingProjectionActions::default();
        let mut binding_payload = None;
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
                    actions.reconciles.push(cached.topic.clone());
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
                    .apply_binding(prompt_cache_key, binding)
                else {
                    cached
                        .prompt_cache_pending_key_hydrations
                        .insert(prompt_cache_key.to_string());
                    cached.prompt_cache_candidate_refill_required = true;
                    if !cached.prompt_cache_key_hydration_scheduled {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        actions.key_hydrations.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            } else {
                if binding_payload.is_none() {
                    binding_payload = Some(serde_json::to_value(binding)?);
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
                        actions.reconciles.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            };
            if !changed {
                continue;
            }
            if is_working_conversations {
                self.dashboard_topology_counters
                    .record_materialization(cached.topic.name(), false);
            }
            let serialized_payload = serialize_prompt_cache_materializer_payload(
                working_state.as_ref(),
                &cached.snapshot_payload,
            )?;
            actions
                .dispatches
                .push(self.commit_prompt_cache_topic_frame(
                    cached,
                    &topic_key,
                    serialized_payload,
                    Utc::now(),
                )?);
        }
        Ok(actions)
    }

    async fn apply_prompt_cache_sticky_route_projection(
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
            dispatches.push(self.commit_prompt_cache_topic_frame(
                cached,
                &topic_key,
                serde_json::to_vec(&cached.snapshot_payload)?,
                Utc::now(),
            )?);
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

    async fn clear_dashboard_activity_topic_refresh_flag(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.refresh_scheduled = false;
        }
    }

    async fn mark_dashboard_activity_topic_dirty(&self, topic: &SubscriptionTopic) {
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

    async fn schedule_dashboard_activity_topic_refresh(
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

    async fn schedule_summary_topic_refresh(
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

    fn spawn_summary_topic_refresh(
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

    async fn run_summary_topic_refresh(&self, state: Arc<AppState>, topic: SubscriptionTopic) {
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

    async fn apply_summary_live_overlay(
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
