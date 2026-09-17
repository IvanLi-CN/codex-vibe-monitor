impl SubscriptionHub {
    #[cfg(test)]
    pub(crate) async fn register_test_topic_name(
        self: &Arc<Self>,
        topic_name: &str,
    ) -> TopicSubscriptionLease {
        let mut guard = self.state.lock().await;
        *guard
            .active_topic_names
            .entry(topic_name.to_string())
            .or_insert(0) += 1;
        let owns_dashboard_live = topic_name == "dashboard.activity.current"
            || guard.topics.values().any(|cached| {
                cached.topic.name() == topic_name
                    && (cached.topic.uses_dashboard_activity_live_overlay()
                        || cached.topic.uses_summary_live_overlay()
                        || cached.topic.uses_timeseries_live_projection()
                        || cached.topic.uses_dashboard_network_live_snapshot())
            });
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(usize::from(owns_dashboard_live));
        let topic_keys = guard
            .topics
            .iter()
            .filter(|(_, cached)| cached.topic.name() == topic_name)
            .map(|(topic_key, _)| topic_key.clone())
            .collect::<Vec<_>>();
        for topic_key in &topic_keys {
            if let Some(topic) = guard
                .topics
                .get(topic_key)
                .map(|cached| cached.topic.clone())
            {
                guard.active_topics.insert(topic_key.clone(), topic.clone());
                Self::register_active_topic_dependencies(&mut guard, topic_key, &topic);
            }
            let active_subscriber_count = {
                let count = guard
                    .active_subscribers
                    .entry(topic_key.clone())
                    .or_insert(0);
                *count += 1;
                *count
            };
            if let Some(topic) = guard.active_topics.get(topic_key) {
                self.dashboard_topology_counters
                    .set_active_subscriber_count(topic.name(), active_subscriber_count);
                if active_subscriber_count > 1 && guard.topics.contains_key(topic_key) {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                }
            }
        }
        TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names: vec![topic_name.to_string()],
            owns_dashboard_live,
        }
    }

    async fn release_topic_subscribers(
        &self,
        topic_keys: Vec<String>,
        topic_names: Vec<String>,
        owns_dashboard_live: bool,
    ) {
        let mut guard = self.state.lock().await;
        for topic_key in topic_keys {
            let topic_name = guard
                .active_topics
                .get(&topic_key)
                .or_else(|| guard.topics.get(&topic_key).map(|cached| &cached.topic))
                .map(|topic| topic.name());
            let active_subscriber_count = match guard.active_subscribers.get_mut(&topic_key) {
                Some(count) if *count > 1 => {
                    *count -= 1;
                    *count
                }
                Some(_) => {
                    guard.active_subscribers.remove(&topic_key);
                    if let Some(topic) = guard.active_topics.remove(&topic_key) {
                        Self::release_active_topic_dependencies(&mut guard, &topic_key, &topic);
                    }
                    guard.prompt_cache_prebaseline_records.remove(&topic_key);
                    guard
                        .prompt_cache_prebaseline_key_hydrations
                        .remove(&topic_key);
                    guard.parallel_work_prebaseline_mutations.remove(&topic_key);
                    guard.runtime_topic_recovery_generation =
                        guard.runtime_topic_recovery_generation.saturating_add(1);
                    let recovery_generation = guard.runtime_topic_recovery_generation;
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        // A disconnected owner cannot consume the mutation stream. Rebuild this
                        // selection before it can resume instead of scanning retained caches for
                        // every runtime event.
                        cached.dirty = true;
                        cached.runtime_topic_recovery_generation = recovery_generation;
                        cached.runtime_topic_recovery_retry_at = None;
                        cached.refresh_scheduled = false;
                        cached.invalidate_upstream_account_attempt_refresh();
                        cached.latest_live_snapshot = None;
                        if matches!(
                            cached.topic,
                            SubscriptionTopic::PromptCacheWindow { .. }
                                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                        ) {
                            cached.prompt_cache_pending_records.clear();
                            cached.prompt_cache_applied_terminal_ids.clear();
                            cached.prompt_cache_refresh_scheduled = false;
                            cached.prompt_cache_baseline_at = None;
                        }
                    }
                    0
                }
                None => 0,
            };
            if let Some(topic_name) = topic_name {
                self.dashboard_topology_counters
                    .set_active_subscriber_count(topic_name, active_subscriber_count);
            }
        }
        for topic_name in topic_names {
            match guard.active_topic_names.get_mut(&topic_name) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    guard.active_topic_names.remove(&topic_name);
                }
                None => {}
            }
        }
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_sub(usize::from(owns_dashboard_live));
    }

    async fn register_server_push_topics(
        self: &Arc<Self>,
        state: Arc<AppState>,
        topics: Vec<SubscriptionTopic>,
    ) -> Result<ServerPushTopicLease, ApiError> {
        let mut unique_topics = HashMap::new();
        for topic in topics {
            unique_topics.entry(topic.cache_key()?).or_insert(topic);
        }

        let topic_keys = unique_topics.keys().cloned().collect::<Vec<_>>();
        let mut topics_to_start = Vec::new();
        {
            let mut guard = self.state.lock().await;
            for (topic_key, topic) in unique_topics {
                *guard
                    .server_push_subscribers
                    .entry(topic_key.clone())
                    .or_insert(0) += 1;
                if guard.server_push_tasks.insert(topic_key.clone()) {
                    topics_to_start.push((topic_key, topic));
                }
            }
        }

        for (topic_key, topic) in topics_to_start {
            let hub = self.clone();
            let state = state.clone();
            tokio::spawn(async move {
                run_server_push_topic_loop(hub, state, topic_key, topic).await;
            });
        }

        Ok(ServerPushTopicLease {
            hub: self.clone(),
            topic_keys,
        })
    }

    async fn release_server_push_topics(&self, topic_keys: Vec<String>) {
        let mut guard = self.state.lock().await;
        for topic_key in topic_keys {
            match guard.server_push_subscribers.get_mut(&topic_key) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    guard.server_push_subscribers.remove(&topic_key);
                }
                None => {}
            }
        }
    }

    async fn stop_server_push_task_if_idle(&self, topic_key: &str) -> bool {
        let mut guard = self.state.lock().await;
        if guard
            .server_push_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or(0)
            > 0
        {
            return false;
        }
        guard.server_push_tasks.remove(topic_key);
        true
    }

    async fn clear_server_push_task(&self, topic_key: &str) {
        let mut guard = self.state.lock().await;
        guard.server_push_tasks.remove(topic_key);
    }

    pub(crate) async fn prepare_connection(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        let resume_by_topic_key = resume
            .into_iter()
            .map(|item| (item.topic_key.clone(), item))
            .collect::<HashMap<_, _>>();
        let mut initial = Vec::new();
        let mut last_sent_cursors = HashMap::new();
        let mut outcomes = Vec::new();

        for descriptor in descriptors {
            let topic = SubscriptionTopic::from_descriptor(&descriptor)?;
            let topic_key = topic.cache_key()?;
            let needs_initial_account_attempt_recheck = {
                let guard = self.state.lock().await;
                guard
                    .topics
                    .get(&topic_key)
                    .is_none_or(|cached| cached.dirty)
            };
            let cached = self
                .ensure_cached_topic(state.clone(), topic.clone())
                .await?;
            let resume_cursor = resume_by_topic_key.get(&topic_key);
            let continuity_reset = resume_cursor
                .zip(cached.continuity_reset_cursor)
                .is_some_and(|(resume, reset_cursor)| resume.cursor < reset_cursor);
            let (frames, outcome) = self
                .prepare_topic_initial_frames(
                    &topic_key,
                    &topic,
                    &cached,
                    resume_cursor,
                    continuity_reset,
                )
                .await?;
            initial.extend(frames);
            last_sent_cursors.insert(topic_key.clone(), cached.cursor);
            outcomes.push(outcome);
            if cached.dirty {
                self.schedule_dirty_topic_recovery(state.clone(), topic.clone())
                    .await;
            }
            if needs_initial_account_attempt_recheck
                && topic.uses_upstream_account_attempt_refresh()
            {
                // The topic is registered before its first database build. A PoolAttempts
                // broadcast can therefore arrive while the cache entry does not exist yet;
                // one fixed-window recheck closes that construction gap without polling.
                self.schedule_upstream_account_attempt_topic_refresh(state.clone(), topic.clone())
                    .await?;
            }
        }

        Ok(PreparedSubscriptionConnection {
            initial,
            last_sent_cursors,
            outcomes,
        })
    }

    async fn prepare_topic_initial_frames(
        &self,
        topic_key: &str,
        topic: &SubscriptionTopic,
        cached: &CachedSubscriptionTopic,
        resume_cursor: Option<&SubscriptionResumeCursor>,
        continuity_reset: bool,
    ) -> Result<(Vec<PreparedTopicFrame>, TopicInitOutcome), ApiError> {
        let replay_attempt = if continuity_reset {
            Err(ReplayMissReason::ContinuityReset)
        } else {
            self.replay_events_for_resume(topic_key, topic.schema_epoch(), resume_cursor)
                .await
        };
        match replay_attempt {
            Ok(Some(events)) if !events.is_empty() => {
                let replay_event_count = events.len();
                let replay_bytes = events.iter().map(|event| event.bytes).sum();
                tracing::debug!(
                    topic_key,
                    replay_event_count,
                    replay_bytes,
                    "subscription replay hit"
                );
                let frames = events
                    .into_iter()
                    .map(|event| PreparedTopicFrame {
                        frame: event.frame,
                        kind: TopicFrameKind::Replay,
                    })
                    .collect();
                Ok((
                    frames,
                    TopicInitOutcome {
                        topic_key: topic_key.to_string(),
                        disposition: TopicInitDisposition::ReplayHit,
                        replay_event_count,
                        replay_bytes,
                        cursor: cached.cursor,
                        miss_reason: None,
                    },
                ))
            }
            Ok(Some(_)) => Ok((
                Vec::new(),
                TopicInitOutcome {
                    topic_key: topic_key.to_string(),
                    disposition: TopicInitDisposition::ResumeCaughtUp,
                    replay_event_count: 0,
                    replay_bytes: 0,
                    cursor: cached.cursor,
                    miss_reason: None,
                },
            )),
            Ok(None) => Ok((
                vec![PreparedTopicFrame {
                    frame: cached.snapshot_frame.clone(),
                    kind: TopicFrameKind::Snapshot,
                }],
                TopicInitOutcome {
                    topic_key: topic_key.to_string(),
                    disposition: TopicInitDisposition::SnapshotNoResume,
                    replay_event_count: 0,
                    replay_bytes: 0,
                    cursor: cached.cursor,
                    miss_reason: None,
                },
            )),
            Err(reason) => {
                if resume_cursor.is_some() {
                    self.dashboard_topology_counters
                        .record_reconnect_churn(topic.name());
                }
                tracing::debug!(
                    topic_key,
                    miss_reason = reason.as_str(),
                    "subscription replay miss, falling back to snapshot"
                );
                Ok((
                    vec![PreparedTopicFrame {
                        frame: cached.snapshot_frame.clone(),
                        kind: TopicFrameKind::Snapshot,
                    }],
                    TopicInitOutcome {
                        topic_key: topic_key.to_string(),
                        disposition: TopicInitDisposition::SnapshotResumeMiss,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: Some(reason.as_str()),
                    },
                ))
            }
        }
    }

    async fn replay_events_for_resume(
        &self,
        topic_key: &str,
        schema_epoch: String,
        resume: Option<&SubscriptionResumeCursor>,
    ) -> Result<Option<Vec<ReplayableTopicEvent>>, ReplayMissReason> {
        let Some(resume) = resume else {
            return Ok(None);
        };

        if resume.schema_epoch != schema_epoch {
            return Err(ReplayMissReason::SchemaEpochMismatch);
        }

        let guard = self.state.lock().await;
        let Some(cached) = guard.topics.get(topic_key) else {
            return Err(ReplayMissReason::UnknownTopic);
        };

        let mut gap = Vec::new();
        let mut gap_bytes = 0usize;
        let mut matched = false;

        for event in &cached.replay_events {
            if event.frame.cursor <= resume.cursor {
                matched = true;
                continue;
            }
            if !matched
                && resume.cursor > 0
                && event.frame.cursor > resume.cursor
                && cached
                    .replay_events
                    .front()
                    .is_some_and(|front| front.frame.cursor > resume.cursor)
            {
                return Err(ReplayMissReason::GapWindowMiss);
            }
            gap_bytes = gap_bytes.saturating_add(event.bytes);
            if gap.len() + 1 > SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS {
                return Err(ReplayMissReason::GapEventBudgetExceeded);
            }
            if gap_bytes > SUBSCRIPTION_REPLAY_MAX_GAP_BYTES {
                return Err(ReplayMissReason::GapByteBudgetExceeded);
            }
            gap.push(event.clone());
        }

        if resume.cursor > 0
            && cached
                .replay_events
                .front()
                .is_some_and(|front| front.frame.cursor > resume.cursor)
        {
            return Err(ReplayMissReason::GapWindowMiss);
        }

        Ok(Some(gap))
    }

    async fn ensure_cached_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        let topic_key = topic.cache_key()?;
        let (existing, has_active_owner) = {
            let guard = self.state.lock().await;
            (
                guard.topics.get(&topic_key).cloned(),
                guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0,
            )
        };
        if let Some(existing) = existing
            && ((has_active_owner
                && existing.dirty
                && !existing
                    .dashboard_materializer
                    .as_ref()
                    .is_some_and(DashboardTopicMaterializer::requires_terminal_window_rebase))
                || (!existing.dirty
                    && (!topic.is_closed_summary_topic()
                        || existing.calendar_anchor == subscription_calendar_anchor(&topic))))
        {
            return Ok(existing);
        }
        if !has_active_owner {
            #[cfg(test)]
            return self.refresh_topic(state, topic, false).await;
            #[cfg(not(test))]
            return Err(ApiError::from(anyhow!(
                "subscription topic setup requires an active owner"
            )));
        }
        for _ in 0..SUBSCRIPTION_INITIAL_TOPIC_BUILD_ATTEMPTS {
            if let Some(cached) = self
                .refresh_topic_if_active(state.clone(), topic.clone(), false)
                .await?
            {
                return Ok(cached);
            }
            if let Some(existing) = self.state.lock().await.topics.get(&topic_key).cloned() {
                return Ok(existing);
            }
        }
        Err(ApiError::from(anyhow!(
            "subscription topic changed before its bounded initial snapshot was ready"
        )))
    }

    async fn refresh_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, false, None)
            .await
            .map(|cached| cached.expect("unguarded topic refresh should always commit"))
    }

    async fn refresh_topic_if_active(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, true, None)
            .await
    }

    async fn refresh_upstream_account_attempt_topic_if_active(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        refresh_generation: u64,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, true, Some(refresh_generation))
            .await
    }

    async fn build_prompt_cache_consistent_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, PromptCacheBaselineBuild), ApiError> {
        if matches!(
            topic,
            SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
        ) {
            return self
                .build_working_conversations_baseline(state, topic)
                .await;
        }

        for _ in 0..3 {
            let mut observer = state.pool.acquire().await?;
            let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(&mut *observer)
                    .await?;
            let payload = topic.build_cached_payload(state.clone()).await?;
            let candidate_identities = {
                let guard = self.state.lock().await;
                let topic_key = topic.cache_key()?;
                guard
                    .prompt_cache_prebaseline_records
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|records| records.values())
                    .chain(
                        guard
                            .topics
                            .get(&topic_key)
                            .into_iter()
                            .flat_map(|cached| cached.prompt_cache_pending_records.values()),
                    )
                    .map(|delta| delta.identity.clone())
                    .collect::<HashSet<_>>()
            };
            let persisted_identities = load_persisted_invocation_identities(
                &mut observer,
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            if version_before == version_after {
                return Ok((
                    payload,
                    PromptCacheBaselineBuild {
                        baseline_row_id,
                        persisted_identities,
                        runtime_overlay_terminal_identities: HashSet::new(),
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "prompt cache baseline changed during build"
        )))
    }

    async fn build_working_conversations_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, PromptCacheBaselineBuild), ApiError> {
        let SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size,
            recent_invocation_limit,
            blocked_binding_upstream_account_id,
            blocked_binding_constraint_source,
        } = topic
        else {
            unreachable!("working conversation baseline helper requires matching topic");
        };
        let mut transaction = state.pool.begin().await?;
        let baseline_row_id =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                .fetch_one(transaction.as_mut())
                .await?;
        let snapshot_at = Utc::now();
        let filter = PromptCacheConversationBlockedBindingFilter {
            upstream_account_id: *blocked_binding_upstream_account_id,
            constraint_source: *blocked_binding_constraint_source,
        };
        let blocked_binding_filter = filter.is_active().then_some(filter);
        let (response, runtime_overlay_terminal_identities) =
            build_prompt_cache_conversations_response_for_request_with_runtime_overlay_terminal_identities_on_connection(
                state.as_ref(),
                PromptCacheConversationsRequest {
                    selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                        SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                    ),
                    detail_level: PromptCacheConversationDetailLevel::Full,
                    recent_invocation_limit: Some(*recent_invocation_limit),
                    page_size: Some(*page_size),
                    cursor: None,
                    snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
                    blocked_binding_filter: blocked_binding_filter.clone(),
                },
                transaction.as_mut(),
                snapshot_at,
                Some(baseline_row_id),
            )
            .await?;
        let source_scope = resolve_default_source_scope(&state.pool).await?;
        let range_start_bound = db_occurred_at_lower_bound(
            snapshot_at
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let runtime_records = runtime_prompt_cache_overlay_records_at_snapshot(
            state.as_ref(),
            source_scope,
            &range_start_bound,
            blocked_binding_filter.as_ref(),
            snapshot_at,
        );
        let in_flight_records = Self::load_working_conversation_in_flight_records(
            transaction.as_mut(),
            source_scope,
            &response,
            runtime_records,
        )
        .await?;
        let payload = working_conversation_payload(
            response,
            *page_size as usize,
            *recent_invocation_limit as usize,
            blocked_binding_filter,
            in_flight_records,
        );
        let candidate_identities = self.prompt_cache_prebaseline_identities(topic).await?;
        let persisted_identities = load_persisted_invocation_identities(
            transaction.as_mut(),
            &candidate_identities,
            baseline_row_id,
        )
        .await?;
        transaction.commit().await?;
        Ok((
            payload,
            PromptCacheBaselineBuild {
                baseline_row_id,
                persisted_identities,
                runtime_overlay_terminal_identities,
            },
        ))
    }

    async fn load_working_conversation_in_flight_records(
        connection: &mut sqlx::SqliteConnection,
        source_scope: InvocationSourceScope,
        response: &PromptCacheConversationsResponse,
        runtime_records: Vec<ApiInvocation>,
    ) -> Result<Vec<PromptCacheInFlightPhaseRecord>, ApiError> {
        let selected_keys = response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.clone())
            .collect::<Vec<_>>();
        let mut records = query_prompt_cache_in_flight_phase_records(
            connection,
            source_scope,
            &selected_keys,
            None,
        )
        .await?;
        records.extend(runtime_records.into_iter().filter_map(|record| {
            prompt_cache_runtime_record_is_in_flight(&record).then(|| {
                Some(PromptCacheInFlightPhaseRecord {
                    identity: format!("{}\0{}", record.invoke_id, record.occurred_at),
                    prompt_cache_key: record.prompt_cache_key.as_deref()?.to_string(),
                    phase: runtime_record_live_phase(&record).map(str::to_string),
                })
            })?
        }));
        Ok(records)
    }

    async fn prompt_cache_prebaseline_identities(
        &self,
        topic: &SubscriptionTopic,
    ) -> Result<HashSet<String>, ApiError> {
        let guard = self.state.lock().await;
        let topic_key = topic.cache_key()?;
        Ok(guard
            .prompt_cache_prebaseline_records
            .get(&topic_key)
            .into_iter()
            .flat_map(|records| records.values())
            .chain(
                guard
                    .topics
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|cached| cached.prompt_cache_pending_records.values()),
            )
            .map(|delta| delta.identity.clone())
            .collect())
    }

    async fn build_parallel_work_consistent_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, ParallelWorkBaselineBuild), ApiError> {
        for _ in 0..3 {
            let mut observer = state.pool.acquire().await?;
            let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(&mut *observer)
                    .await?;
            let payload = topic.build_cached_payload(state.clone()).await?;
            let candidate_identities = {
                let guard = self.state.lock().await;
                let topic_key = topic.cache_key()?;
                let mut identities = guard
                    .parallel_work_prebaseline_mutations
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|mutations| mutations.values())
                    .map(|mutation| {
                        format!(
                            "{}\0{}",
                            mutation.identity.invoke_id, mutation.identity.occurred_at
                        )
                    })
                    .collect::<HashSet<_>>();
                if let Some(DashboardTopicMaterializer::ParallelWork { base }) = guard
                    .topics
                    .get(&topic_key)
                    .and_then(|cached| cached.dashboard_materializer.as_ref())
                {
                    identities.extend(
                        base.lock()
                            .expect("parallel-work materializer state lock")
                            .runtime_mutations
                            .keys()
                            .cloned(),
                    );
                }
                identities
            };
            let persisted_identities = load_persisted_invocation_identities(
                &mut observer,
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            if version_before == version_after {
                return Ok((
                    payload,
                    ParallelWorkBaselineBuild {
                        persisted_identities,
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "parallel-work baseline changed during build"
        )))
    }
}

fn working_conversation_payload(
    response: PromptCacheConversationsResponse,
    page_size: usize,
    recent_invocation_limit: usize,
    blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    in_flight_records: Vec<PromptCacheInFlightPhaseRecord>,
) -> BuiltSubscriptionTopicPayload {
    let mut materializer = DashboardWorkingConversationsMaterializerState::new(
        response,
        page_size as i64,
        recent_invocation_limit as i64,
        blocked_binding_filter,
    );
    materializer.install_in_flight_phase_records(in_flight_records);
    BuiltSubscriptionTopicPayload::Dashboard(DashboardTopicMaterializer::WorkingConversations {
        state: Arc::new(StdMutex::new(materializer)),
    })
}
