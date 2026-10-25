use super::*;
use chrono::{DateTime, Utc};

impl SubscriptionHub {
    pub(crate) async fn register_topic_subscribers(
        self: &Arc<Self>,
        topics: &[SubscriptionTopic],
    ) -> Result<TopicSubscriptionLease, ApiError> {
        let owns_dashboard_live = topics.iter().any(|topic| {
            topic.uses_dashboard_activity_live_overlay()
                || topic.uses_summary_live_overlay()
                || topic.uses_timeseries_live_projection()
                || topic.uses_dashboard_network_live_snapshot()
        });
        let topic_keys = topics
            .iter()
            .map(SubscriptionTopic::cache_key)
            .collect::<Result<HashSet<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();
        let topic_names = topics
            .iter()
            .map(SubscriptionTopic::name)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut guard = self.state.lock().await;
        for topic in topics {
            let topic_key = topic.cache_key()?;
            guard.active_topics.insert(topic_key.clone(), topic.clone());
            Self::register_active_topic_dependencies(&mut guard, &topic_key, topic);
        }
        for topic_key in &topic_keys {
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
        for topic_name in &topic_names {
            *guard
                .active_topic_names
                .entry(topic_name.clone())
                .or_insert(0) += 1;
        }
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(usize::from(owns_dashboard_live));
        Ok(TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names,
            owns_dashboard_live,
        })
    }

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

    pub(crate) async fn release_topic_subscribers(
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

    pub(crate) async fn register_server_push_topics(
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
            spawn_performance(state.observability.clone(), async move {
                run_server_push_topic_loop(hub, state, topic_key, topic).await;
            });
        }

        Ok(ServerPushTopicLease {
            hub: self.clone(),
            topic_keys,
        })
    }

    pub(crate) async fn release_server_push_topics(&self, topic_keys: Vec<String>) {
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

    pub(crate) async fn stop_server_push_task_if_idle(&self, topic_key: &str) -> bool {
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

    pub(crate) async fn clear_server_push_task(&self, topic_key: &str) {
        let mut guard = self.state.lock().await;
        guard.server_push_tasks.remove(topic_key);
    }

    pub(crate) async fn prepare_connection(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        self.prepare_connection_with_capability_at(state, descriptors, resume, false, None)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn prepare_connection_at(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
        now: DateTime<Utc>,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        self.prepare_connection_with_capability_at(state, descriptors, resume, false, Some(now))
            .await
    }

    pub(crate) async fn prepare_connection_with_capability(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
        supports_deltas: bool,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        self.prepare_connection_with_capability_at(
            state,
            descriptors,
            resume,
            supports_deltas,
            None,
        )
        .await
    }

    async fn prepare_connection_with_capability_at(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
        supports_deltas: bool,
        now: Option<DateTime<Utc>>,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        let resume_by_topic_key = resume
            .into_iter()
            .map(|item| (item.topic_key.clone(), item))
            .collect::<HashMap<_, _>>();
        let mut initial = Vec::new();
        let mut last_sent_cursors = HashMap::new();
        let mut outcomes = Vec::new();

        'descriptor: for descriptor in descriptors {
            let topic = SubscriptionTopic::from_descriptor(&descriptor)?;
            let topic_key = topic.cache_key()?;
            let needs_initial_account_attempt_recheck = {
                let guard = self.state.lock().await;
                guard
                    .topics
                    .get(&topic_key)
                    .is_none_or(|cached| cached.dirty)
            };
            let cached = 'cached: loop {
                let cached = match self
                    .ensure_cached_topic_with_now(state.clone(), topic.clone(), now)
                    .await
                {
                    Ok(cached) => cached,
                    Err(ApiError::Unavailable(_)) => {
                        // Summary is exact-or-unavailable. Keep this expected projection gap local
                        // to its topic so another Dashboard topic can still hydrate on the same SSE
                        // connection. The frame deliberately carries no payload or cursor.
                        initial.push(PreparedTopicFrame {
                            frame: Arc::new(serialize_unavailable_topic_frame(
                                topic.descriptor(),
                                topic_key.clone(),
                                topic.schema_epoch(),
                            )?),
                            kind: TopicFrameKind::Unavailable,
                        });
                        if matches!(&topic, SubscriptionTopic::SummaryCurrent { .. }) {
                            self.schedule_summary_topic_refresh(state.clone(), topic.clone(), 1)
                                .await?;
                        }
                        outcomes.push(TopicInitOutcome {
                            topic_key,
                            disposition: TopicInitDisposition::Unavailable,
                            replay_event_count: 0,
                            replay_bytes: 0,
                            cursor: 0,
                            miss_reason: None,
                        });
                        continue 'descriptor;
                    }
                    Err(err) => return Err(err),
                };
                if matches!(&topic, SubscriptionTopic::SummaryCurrent { .. }) {
                    let guard = self.state.lock().await;
                    let Some(current) = guard.topics.get(&topic_key).cloned() else {
                        break 'cached cached;
                    };
                    if current.summary_gap_generation != guard.summary_delta_journal.gap_generation
                        && current.summary_gap_degraded
                    {
                        continue 'cached;
                    }
                    break 'cached current;
                }
                break 'cached cached;
            };
            let resume_cursor = resume_by_topic_key.get(&topic_key);
            let continuity_reset = resume_cursor
                .zip(cached.continuity_reset_cursor)
                .is_some_and(|(resume, reset_cursor)| resume.cursor < reset_cursor);
            let replay_attempt = if continuity_reset {
                Err(ReplayMissReason::ContinuityReset)
            } else {
                self.replay_events_for_resume(&topic_key, topic.schema_epoch(), resume_cursor)
                    .await
            };

            match replay_attempt {
                Ok(Some(events)) if !events.is_empty() => {
                    let replay_event_count = events.len();
                    let replay_bytes = events
                        .iter()
                        .map(|event| {
                            if supports_deltas {
                                event.frame.retained_bytes()
                            } else if Arc::ptr_eq(&event.frame, &event.full_frame) {
                                event.bytes
                            } else {
                                event.full_frame.retained_bytes()
                            }
                        })
                        .sum::<usize>();
                    tracing::debug!(
                        topic_key,
                        replay_event_count,
                        replay_bytes,
                        "subscription replay hit"
                    );
                    for event in events {
                        initial.push(PreparedTopicFrame {
                            frame: if supports_deltas {
                                event.frame
                            } else {
                                event.full_frame
                            },
                            kind: TopicFrameKind::Replay,
                        });
                    }
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::ReplayHit,
                        replay_event_count,
                        replay_bytes,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
                Ok(Some(_)) => {
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::ResumeCaughtUp,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
                Ok(None) => {
                    initial.push(PreparedTopicFrame {
                        frame: cached.snapshot_frame.clone(),
                        kind: TopicFrameKind::Snapshot,
                    });
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::SnapshotNoResume,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
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
                    initial.push(PreparedTopicFrame {
                        frame: cached.snapshot_frame.clone(),
                        kind: TopicFrameKind::Snapshot,
                    });
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::SnapshotResumeMiss,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: Some(reason.as_str()),
                    });
                }
            }
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

    pub(crate) async fn replay_events_for_resume(
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

    pub(crate) async fn ensure_cached_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        self.ensure_cached_topic_with_now(state, topic, None).await
    }

    async fn ensure_cached_topic_with_now(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        now: Option<DateTime<Utc>>,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        let topic_key = topic.cache_key()?;
        let summary_selection = if let SubscriptionTopic::SummaryCurrent {
            window,
            time_zone,
            limit,
            upstream_account_id,
        } = &topic
        {
            let query = SummaryQuery {
                window: Some(window.clone()),
                limit: *limit,
                time_zone: Some(time_zone.clone()),
                upstream_account_id: *upstream_account_id,
            };
            Some((
                parse_summary_window(&query, state.config.list_limit_max as i64)?,
                parse_reporting_tz(Some(time_zone))?,
                *upstream_account_id,
            ))
        } else {
            None
        };
        let (existing, has_active_owner, summary_gap_pending, summary_gap_generation) = {
            let guard = self.state.lock().await;
            let summary_gap_pending = summary_selection.as_ref().is_some_and(
                |(window, reporting_tz, upstream_account_id)| {
                    guard.summary_projection.as_ref().is_some_and(|projection| {
                        let gaps = guard
                            .summary_delta_journal
                            .gap_proofs
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>();
                        let deltas = guard
                            .summary_delta_journal
                            .entries
                            .iter()
                            .map(|entry| &entry.delta)
                            .chain(guard.summary_delta_journal.replayed_entries.iter())
                            .filter(|delta| {
                                !projection.contains_persisted_live_terminal_delta(delta)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        summary_gap_affects_selection(
                            projection,
                            &gaps,
                            &deltas,
                            window,
                            *reporting_tz,
                            *upstream_account_id,
                        )
                    })
                },
            );
            (
                guard.topics.get(&topic_key).cloned(),
                guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0,
                summary_gap_pending,
                guard.summary_delta_journal.gap_generation,
            )
        };
        let summary_cache_stale = summary_selection.is_some()
            && existing.as_ref().is_some_and(|cached| {
                cached.summary_gap_generation != summary_gap_generation
                    && (summary_gap_pending || cached.summary_gap_degraded)
            });
        if matches!(&topic, SubscriptionTopic::SummaryCurrent { .. })
            && !has_active_owner
            && existing.as_ref().is_some_and(|cached| cached.dirty)
        {
            return Err(ApiError::unavailable(anyhow!(
                "summary topic has an unproven projection gap"
            )));
        }
        #[cfg(not(test))]
        let _ = now;
        if let Some(existing) = existing
            && !summary_cache_stale
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
            return self.refresh_topic_with_now(state, topic, false, now).await;
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
            let guard = self.state.lock().await;
            if let Some(existing) = guard.topics.get(&topic_key).cloned()
                && (summary_selection.is_none()
                    || existing.summary_gap_generation
                        == guard.summary_delta_journal.gap_generation)
            {
                return Ok(existing);
            }
        }
        Err(ApiError::from(anyhow!(
            "subscription topic changed before its bounded initial snapshot was ready"
        )))
    }

    pub(crate) async fn refresh_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        self.refresh_topic_with_now(state, topic, emit_live, None)
            .await
            .map(|cached| cached.expect("unguarded topic refresh should always commit"))
    }

    async fn refresh_topic_with_now(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        now: Option<DateTime<Utc>>,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        self.refresh_topic_inner_with_now(state, topic, emit_live, false, None, now)
            .await
            .map(|cached| cached.expect("unguarded topic refresh should always commit"))
    }

    pub(crate) async fn refresh_topic_if_active(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, true, None)
            .await
    }

    pub(crate) async fn refresh_upstream_account_attempt_topic_if_active(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        refresh_generation: u64,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, true, Some(refresh_generation))
            .await
    }

    pub(crate) async fn build_prompt_cache_consistent_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, PromptCacheBaselineBuild), ApiError> {
        if let SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size,
            recent_invocation_limit,
            blocked_binding_upstream_account_id,
            blocked_binding_constraint_source,
        } = topic
        {
            let mut transaction = state.pool.begin().await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(transaction.as_mut())
                    .await?;
            let snapshot_at = Utc::now();
            let blocked_binding_filter = PromptCacheConversationBlockedBindingFilter {
                upstream_account_id: *blocked_binding_upstream_account_id,
                constraint_source: *blocked_binding_constraint_source,
            };
            let blocked_binding_filter = blocked_binding_filter
                .is_active()
                .then_some(blocked_binding_filter);
            let (response, runtime_overlay_terminal_identities) = build_prompt_cache_conversations_response_for_request_with_runtime_overlay_terminal_identities_on_connection(
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
            let selected_keys = response
                .conversations
                .iter()
                .map(|conversation| conversation.prompt_cache_key.clone())
                .collect::<Vec<_>>();
            let mut in_flight_records = query_prompt_cache_in_flight_phase_records(
                transaction.as_mut(),
                source_scope,
                &selected_keys,
                None,
            )
            .await?;
            in_flight_records.extend(runtime_records.into_iter().filter_map(|record| {
                if !prompt_cache_runtime_record_is_in_flight(&record) {
                    return None;
                }
                let prompt_cache_key = record.prompt_cache_key.as_deref()?.to_string();
                Some(PromptCacheInFlightPhaseRecord {
                    identity: format!("{}\0{}", record.invoke_id, record.occurred_at),
                    prompt_cache_key,
                    phase: runtime_record_live_phase(&record).map(str::to_string),
                })
            }));
            let mut materializer = DashboardWorkingConversationsMaterializerState::new(
                response,
                *page_size,
                *recent_invocation_limit,
                blocked_binding_filter,
            );
            materializer.install_in_flight_phase_records(in_flight_records);
            let payload = BuiltSubscriptionTopicPayload::Dashboard(
                DashboardTopicMaterializer::WorkingConversations {
                    state: Arc::new(StdMutex::new(materializer)),
                },
            );
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
                transaction.as_mut(),
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            transaction.commit().await?;
            return Ok((
                payload,
                PromptCacheBaselineBuild {
                    baseline_row_id,
                    persisted_identities,
                    runtime_overlay_terminal_identities,
                },
            ));
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
            let baseline_runtime_records =
                capture_prompt_cache_baseline_runtime_records(state.as_ref(), topic).await?;
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
                let runtime_overlay_terminal_identities =
                    prompt_cache_baseline_runtime_terminal_identities(
                        topic,
                        &payload,
                        &baseline_runtime_records,
                    );
                return Ok((
                    payload,
                    PromptCacheBaselineBuild {
                        baseline_row_id,
                        persisted_identities,
                        runtime_overlay_terminal_identities,
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "prompt cache baseline changed during build"
        )))
    }

    pub(crate) async fn build_parallel_work_consistent_baseline(
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
            let payload = match topic {
                SubscriptionTopic::ParallelWorkCurrent {
                    range,
                    time_zone,
                    bucket,
                    upstream_account_id,
                } if range != "yesterday" => {
                    // This observer already guards the baseline. Reusing its row boundary
                    // avoids retaining a second observer inside the generic topic builder.
                    let query = ParallelWorkStatsQuery {
                        range: range.clone(),
                        bucket: bucket.clone(),
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    };
                    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                        let base = build_dashboard_parallel_work_materializer_state_at_baseline_on_connection(
                            &state,
                            query,
                            baseline_row_id,
                            &mut observer,
                        )
                        .await?;
                        BuiltSubscriptionTopicPayload::Dashboard(
                            DashboardTopicMaterializer::ParallelWork {
                                base: Arc::new(StdMutex::new(base)),
                            },
                        )
                    } else {
                        // Preserve build_cached_payload's non-Auto JSON route and its
                        // runtime-overlay boundary, while reusing the held connection.
                        let baseline = load_parallel_work_projection_baseline_at_on_connection(
                            &state,
                            query,
                            Utc::now(),
                            &mut observer,
                        )
                        .await?;
                        BuiltSubscriptionTopicPayload::Json(serde_json::to_value(
                            baseline.response,
                        )?)
                    }
                }
                _ => topic.build_cached_payload(state.clone()).await?,
            };
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

    pub(crate) async fn refresh_topic_inner(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
        expected_upstream_account_attempt_refresh_generation: Option<u64>,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner_with_now(
            state,
            topic,
            emit_live,
            require_active_owner,
            expected_upstream_account_attempt_refresh_generation,
            None,
        )
        .await
    }

    async fn refresh_topic_inner_with_now(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
        expected_upstream_account_attempt_refresh_generation: Option<u64>,
        now: Option<DateTime<Utc>>,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        let coalesce_managed_task_refresh = is_managed_task_refresh_topic(&topic);
        let topic_key = topic.cache_key()?;
        let mut result = self
            .refresh_topic_inner_once(
                state.clone(),
                topic.clone(),
                emit_live,
                require_active_owner,
                expected_upstream_account_attempt_refresh_generation,
                now,
            )
            .await?;
        while coalesce_managed_task_refresh && result.is_some() {
            let needs_followup = {
                let guard = self.state.lock().await;
                guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|cached| cached.dirty && !cached.refresh_scheduled)
            };
            if !needs_followup {
                break;
            }
            result = self
                .refresh_topic_inner_once(
                    state.clone(),
                    topic.clone(),
                    emit_live,
                    require_active_owner,
                    expected_upstream_account_attempt_refresh_generation,
                    now,
                )
                .await?;
        }
        Ok(result)
    }

    pub(crate) async fn refresh_topic_inner_once(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
        expected_upstream_account_attempt_refresh_generation: Option<u64>,
        now: Option<DateTime<Utc>>,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        let topic_key = topic.cache_key()?;
        let schema_epoch = topic.schema_epoch();
        let descriptor = topic.descriptor();
        let started = Instant::now();
        let summary_gap_generation_at_build =
            if matches!(&topic, SubscriptionTopic::SummaryCurrent { .. }) {
                Some(self.state.lock().await.summary_delta_journal.gap_generation)
            } else {
                None
            };
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
        let coalesce_managed_task_refresh = is_managed_task_refresh_topic(&topic);
        let _managed_task_refresh_serial = if coalesce_managed_task_refresh {
            Some(self.managed_task_refresh_serial.lock().await)
        } else {
            None
        };
        // A recovery or owner disconnect may happen while a cold build is in flight. Capture
        // the cache generation before building so an old result can never clear newer dirty
        // state or replace the retained last-good frame.
        let ((refresh_generation, refresh_had_cached_topic), managed_task_refresh_generation) =
            if require_active_owner {
                let mut guard = self.state.lock().await;
                if guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                {
                    return Ok(None);
                }
                if let Some(expected_generation) =
                    expected_upstream_account_attempt_refresh_generation
                    && !guard.topics.get(&topic_key).is_some_and(|cached| {
                        cached.upstream_account_attempt_refresh_generation == expected_generation
                    })
                {
                    // A disconnect/reconnect can invalidate the dedicated refresh after its worker
                    // acquired the lease but before it begins the database build.
                    return Ok(None);
                }
                let managed_task_refresh_generation = coalesce_managed_task_refresh.then(|| {
                    guard
                        .managed_task_refresh_generations
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                });
                if coalesce_managed_task_refresh
                    && let Some(cached) = guard.topics.get_mut(&topic_key)
                {
                    if cached.refresh_scheduled {
                        return Ok(Some(cached.clone()));
                    }
                    if !cached.dirty {
                        return Ok(Some(cached.clone()));
                    }
                    cached.refresh_scheduled = true;
                }
                (
                    guard
                        .topics
                        .get(&topic_key)
                        .map(|cached| (Some(cached.runtime_topic_recovery_generation), true))
                        .unwrap_or((Some(guard.runtime_topic_recovery_generation), false)),
                    managed_task_refresh_generation,
                )
            } else {
                ((None, false), None)
            };
        let timeline_baseline =
            if emit_live && matches!(&topic, SubscriptionTopic::ManagedTaskTimeline) {
                self.state
                    .lock()
                    .await
                    .topics
                    .get(&topic_key)
                    .filter(|cached| !cached.dirty)
                    .and_then(|cached| {
                        cached
                            .snapshot_payload
                            .get("watermark")
                            .and_then(Value::as_i64)
                            .map(|revision| (cached.snapshot_payload.clone(), revision))
                    })
            } else {
                None
            };
        let (mut built_payload, prompt_cache_build, parallel_work_build) = if is_prompt_cache_topic
        {
            let (payload, build) = self
                .build_prompt_cache_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, Some(build), None)
        } else if is_open_parallel_work_topic {
            let (payload, build) = self
                .build_parallel_work_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, None, Some(build))
        } else if matches!(&topic, SubscriptionTopic::ManagedTaskTimeline) {
            let after_revision = timeline_baseline.as_ref().map(|(_, revision)| *revision);
            let event_payload =
                match build_managed_task_timeline_topic_payload(after_revision).await {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.clear_managed_task_refresh_schedule(&topic_key).await;
                        return Err(error);
                    }
                };
            let payload = if let Some((previous, _)) = &timeline_baseline {
                let snapshot_payload =
                    match merge_managed_task_timeline_payload(previous, &event_payload) {
                        Ok(payload) => payload,
                        Err(error) => {
                            self.clear_managed_task_refresh_schedule(&topic_key).await;
                            return Err(error);
                        }
                    };
                BuiltSubscriptionTopicPayload::JsonDelta {
                    snapshot_payload,
                    event_payload,
                }
            } else {
                BuiltSubscriptionTopicPayload::Json(event_payload)
            };
            (payload, None, None)
        } else if matches!(&topic, SubscriptionTopic::ManagedTaskTimelineV2) {
            let payload = match build_managed_task_timeline_revision_topic_payload().await {
                Ok(payload) => payload,
                Err(error) => {
                    self.clear_managed_task_refresh_schedule(&topic_key).await;
                    return Err(error);
                }
            };
            (BuiltSubscriptionTopicPayload::Json(payload), None, None)
        } else {
            let payload = match topic
                .build_cached_payload_at(state.clone(), now.unwrap_or_else(Utc::now))
                .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    if coalesce_managed_task_refresh {
                        let mut guard = self.state.lock().await;
                        if let Some(cached) = guard.topics.get_mut(&topic_key) {
                            cached.refresh_scheduled = false;
                        }
                    }
                    return Err(error);
                }
            };
            (payload, None, None)
        };
        self.dashboard_topology_counters.record_materialization(
            topic.name(),
            matches!(&built_payload, BuiltSubscriptionTopicPayload::Json(_))
                && topic.is_unmigrated_dashboard_hot_projection(),
        );

        let (cached, dispatch, initial_hydration, initial_reconcile) = {
            let mut guard = self.state.lock().await;
            if require_active_owner {
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0;
                let generation_matches = if refresh_had_cached_topic {
                    guard.topics.get(&topic_key).is_some_and(|cached| {
                        Some(cached.runtime_topic_recovery_generation) == refresh_generation
                    })
                } else {
                    Some(guard.runtime_topic_recovery_generation) == refresh_generation
                        && guard.topics.get(&topic_key).is_none_or(|cached| {
                            Some(cached.runtime_topic_recovery_generation) == refresh_generation
                        })
                };
                let account_attempt_generation_matches =
                    expected_upstream_account_attempt_refresh_generation.is_none_or(
                        |expected_generation| {
                            guard.topics.get(&topic_key).is_some_and(|cached| {
                                cached.upstream_account_attempt_refresh_generation
                                    == expected_generation
                            })
                        },
                    );
                if !active || !generation_matches || !account_attempt_generation_matches {
                    // A newer gap or owner generation owns the cache now. Only the owner that
                    // observed this generation may dirty it; never invalidate a newer clean
                    // frame committed by a reconnecting subscriber.
                    if !active && let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                        cached.refresh_scheduled = false;
                        cached.latest_live_snapshot = None;
                    }
                    if coalesce_managed_task_refresh
                        && let Some(cached) = guard.topics.get_mut(&topic_key)
                    {
                        cached.refresh_scheduled = false;
                    }
                    return Ok(None);
                }
                if summary_gap_generation_at_build.is_some_and(|generation| {
                    generation != guard.summary_delta_journal.gap_generation
                }) {
                    return Ok(None);
                }
            }
            if let BuiltSubscriptionTopicPayload::Json(payload) = &mut built_payload
                && let Some(live) = guard
                    .topics
                    .get(&topic_key)
                    .and_then(|cached| cached.latest_live_snapshot.as_ref())
                    .cloned()
            {
                apply_topic_live_overlay_to_payload(state.as_ref(), &topic, payload, &live)?;
            }
            if let BuiltSubscriptionTopicPayload::Dashboard(
                DashboardTopicMaterializer::ParallelWork { base },
            ) = &built_payload
            {
                let baseline_identities = parallel_work_build
                    .as_ref()
                    .map(|build| build.persisted_identities.clone())
                    .unwrap_or_default();
                let mut replay = guard
                    .topics
                    .get(&topic_key)
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
                    .remove(&topic_key)
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
                        .entry(topic_key.clone())
                        .or_default()
                        .extend(unresolved);
                }
            }
            let mut prompt_cache_pending = guard
                .prompt_cache_prebaseline_records
                .remove(&topic_key)
                .unwrap_or_default();
            let prebaseline_working_hydration_keys = guard
                .prompt_cache_prebaseline_key_hydrations
                .remove(&topic_key)
                .unwrap_or_default();
            if let Some(existing) = guard.topics.get_mut(&topic_key) {
                prompt_cache_pending.append(&mut existing.prompt_cache_pending_records);
                existing.prompt_cache_refresh_scheduled = false;
            }
            let prompt_cache_replay = prompt_cache_build
                .as_ref()
                .map(|build| {
                    prompt_cache_pending
                        .into_values()
                        .filter(|delta| {
                            prompt_cache_delta_needs_replay(delta, &build.persisted_identities)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut prompt_cache_applied_terminal_ids = prompt_cache_build
                .as_ref()
                .map(|build| build.runtime_overlay_terminal_identities.clone())
                .unwrap_or_default();
            let mut deferred_working_replay = Vec::new();
            let mut deferred_working_hydration_keys = prebaseline_working_hydration_keys;
            let mut deferred_working_reconcile = false;
            if !prompt_cache_replay.is_empty() {
                let baseline_row_id = prompt_cache_build
                    .as_ref()
                    .map_or(0, |build| build.baseline_row_id);
                match &mut built_payload {
                    BuiltSubscriptionTopicPayload::Json(payload) => {
                        apply_prompt_cache_records_to_payload(
                            &topic,
                            payload,
                            &prompt_cache_replay,
                            &mut prompt_cache_applied_terminal_ids,
                            baseline_row_id,
                        )?;
                    }
                    BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::WorkingConversations { state },
                    ) => {
                        let update = state
                            .lock()
                            .expect("working conversations materializer state lock")
                            .apply_deltas(
                                &prompt_cache_replay,
                                &mut prompt_cache_applied_terminal_ids,
                                baseline_row_id,
                            )?;
                        match update {
                            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(
                                keys,
                            ) => {
                                // The cursor-consistent base is valid, but its compact replay
                                // cannot represent this key exactly. Commit the base and retain
                                // every replay record so the bounded hydrator can replace just
                                // the affected key without losing an initial live mutation.
                                deferred_working_replay = prompt_cache_replay;
                                deferred_working_hydration_keys = keys;
                            }
                            WorkingConversationsProjectionUpdate::NeedsReconcile => {
                                // An incomplete delta cannot be proven safe to apply. Preserve
                                // it on the committed cache and use the existing bounded
                                // recovery path rather than failing setup after removing it.
                                deferred_working_replay = prompt_cache_replay;
                                deferred_working_reconcile = true;
                            }
                            WorkingConversationsProjectionUpdate::Changed
                            | WorkingConversationsProjectionUpdate::Unchanged => {}
                        }
                    }
                    BuiltSubscriptionTopicPayload::JsonDelta { .. }
                    | BuiltSubscriptionTopicPayload::Dashboard(_) => {}
                }
            }
            let serialized_payload = built_payload.serialize(
                guard.dashboard_current_slice.as_deref(),
                guard.dashboard_network_slice.as_deref(),
                guard.dashboard_terminal_slice.as_deref(),
            )?;
            let snapshot_serialized_payload = built_payload.serialize_snapshot(
                guard.dashboard_current_slice.as_deref(),
                guard.dashboard_network_slice.as_deref(),
                guard.dashboard_terminal_slice.as_deref(),
            )?;
            let refreshed_dashboard_materializer = built_payload.dashboard_materializer();
            let current_slice = guard.dashboard_current_slice.clone();
            let network_slice = guard.dashboard_network_slice.clone();
            let terminal_slice = guard.dashboard_terminal_slice.clone();
            let managed_refresh_is_current =
                managed_task_refresh_generation.is_none_or(|expected| {
                    guard
                        .managed_task_refresh_generations
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        == expected
                });
            if coalesce_managed_task_refresh && !managed_refresh_is_current {
                // A newer task observation dirtied this topic while the bounded build was in
                // flight. Keep the last-good frame intact; the outer serial loop will rebuild
                // against the newer generation instead of briefly broadcasting stale data.
                if let Some(existing) = guard.topics.get_mut(&topic_key) {
                    existing.dirty = true;
                    existing.refresh_scheduled = false;
                    return Ok(Some(existing.clone()));
                }
                return Ok(None);
            }
            if !matches!(
                &built_payload,
                BuiltSubscriptionTopicPayload::JsonDelta { .. }
            ) && deferred_working_replay.is_empty()
                && let Some(existing) = guard.topics.get_mut(&topic_key)
            {
                if managed_refresh_is_current
                    && existing.snapshot_frame.payload_bytes.as_ref()
                        == serialized_payload.as_slice()
                    && existing.dirty
                    && existing.dashboard_materializer.is_some()
                    && refreshed_dashboard_materializer.is_some()
                {
                    reuse_unchanged_cached_topic(existing, &serialized_payload)
                        .expect("matching dashboard payload must reuse the cached topic");
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                    existing.dashboard_materializer = refreshed_dashboard_materializer.clone();
                    existing.dashboard_base_revision = existing.cursor;
                    existing.dashboard_materialized_revision = refreshed_dashboard_materializer
                        .as_ref()
                        .and_then(|materializer| {
                            materializer.revision(
                                existing.cursor,
                                current_slice.as_deref(),
                                network_slice.as_deref(),
                                terminal_slice.as_deref(),
                            )
                        });
                    existing.snapshot_payload = built_payload.snapshot_payload();
                    if let Some(build) = &prompt_cache_build {
                        finish_prompt_cache_baseline_reuse(
                            existing,
                            build,
                            &prompt_cache_applied_terminal_ids,
                        );
                        observe_prompt_cache_baseline_publication(&state.observability);
                    }
                    return Ok(Some(existing.clone()));
                }
                if managed_refresh_is_current
                    && reuse_unchanged_cached_topic(existing, &serialized_payload).is_some()
                {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                    if let Some(build) = &prompt_cache_build {
                        finish_prompt_cache_baseline_reuse(
                            existing,
                            build,
                            &prompt_cache_applied_terminal_ids,
                        );
                        observe_prompt_cache_baseline_publication(&state.observability);
                    }
                    return Ok(Some(existing.clone()));
                }
            }
            let current_cursor = guard.topics.get(&topic_key).map_or(0, |entry| entry.cursor);
            let next_cursor = current_cursor.saturating_add(1);
            let continuity_reset_cursor = guard.topics.get(&topic_key).and_then(|entry| {
                if entry.dirty {
                    Some(next_cursor)
                } else {
                    entry.continuity_reset_cursor
                }
            });
            let full_frame = Arc::new(self.serialize_frame(
                descriptor.clone(),
                topic_key.clone(),
                schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let delta_payload = guard
                .topics
                .get(&topic_key)
                .filter(|_| emit_live)
                .and_then(|existing| {
                    build_dashboard_delta_payload(
                        &topic,
                        existing.snapshot_frame.payload_bytes.as_ref(),
                        full_frame.payload_bytes.as_ref(),
                    )
                })
                .filter(|delta| delta.len() < full_frame.payload_bytes.len());
            let frame = if let Some(delta) = delta_payload {
                Arc::new(self.serialize_delta_frame(
                    descriptor.clone(),
                    topic_key.clone(),
                    schema_epoch.clone(),
                    next_cursor,
                    current_cursor,
                    delta,
                )?)
            } else {
                full_frame.clone()
            };
            let snapshot_frame =
                if snapshot_serialized_payload.as_slice() == full_frame.payload_bytes.as_ref() {
                    full_frame.clone()
                } else {
                    Arc::new(self.serialize_frame(
                        descriptor.clone(),
                        topic_key.clone(),
                        schema_epoch.clone(),
                        next_cursor,
                        snapshot_serialized_payload.clone(),
                    )?)
                };
            let payload_bytes = snapshot_serialized_payload.len();
            let summary_gap_degraded = matches!(&topic, SubscriptionTopic::SummaryCurrent { .. })
                && serde_json::from_slice::<Value>(&snapshot_serialized_payload)
                    .ok()
                    .and_then(|payload| payload.get("dataQuality").cloned())
                    .is_some_and(|quality| !quality.is_null());
            let dashboard_materializer = refreshed_dashboard_materializer;
            let dashboard_materialized_revision =
                dashboard_materializer.as_ref().and_then(|materializer| {
                    materializer.revision(
                        next_cursor,
                        guard.dashboard_current_slice.as_deref(),
                        guard.dashboard_network_slice.as_deref(),
                        guard.dashboard_terminal_slice.as_deref(),
                    )
                });
            let had_key_hydration_scheduled = guard
                .topics
                .get(&topic_key)
                .is_some_and(|entry| entry.prompt_cache_key_hydration_scheduled);
            let had_reconcile_scheduled = guard
                .topics
                .get(&topic_key)
                .is_some_and(|entry| entry.prompt_cache_reconcile_scheduled);
            let schedule_initial_hydration =
                !deferred_working_hydration_keys.is_empty() && !had_key_hydration_scheduled;
            let schedule_initial_reconcile = deferred_working_reconcile && !had_reconcile_scheduled;
            let managed_task_refresh_needs_followup =
                managed_task_refresh_generation.is_some_and(|expected| {
                    guard
                        .managed_task_refresh_generations
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        != expected
                });
            let mut next = CachedSubscriptionTopic {
                topic: topic.clone(),
                descriptor: descriptor.clone(),
                schema_epoch: schema_epoch.clone(),
                cursor: next_cursor,
                snapshot_built_at: Instant::now(),
                refresh_scheduled: false,
                conversation_overview_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_scheduled),
                conversation_overview_refresh_in_flight: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_in_flight),
                conversation_overview_refresh_pending: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_pending),
                upstream_account_attempt_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.upstream_account_attempt_refresh_scheduled),
                upstream_account_attempt_refresh_in_flight: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.upstream_account_attempt_refresh_in_flight),
                upstream_account_attempt_refresh_pending: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.upstream_account_attempt_refresh_pending),
                upstream_account_attempt_refresh_generation: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.upstream_account_attempt_refresh_generation),
                dirty: deferred_working_reconcile || managed_task_refresh_needs_followup,
                runtime_topic_recovery_generation: guard
                    .topics
                    .get(&topic_key)
                    .map_or(guard.runtime_topic_recovery_generation, |entry| {
                        entry.runtime_topic_recovery_generation
                    }),
                runtime_topic_recovery_retry_at: None,
                summary_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.summary_refresh_scheduled),
                summary_refresh_in_flight: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.summary_refresh_in_flight),
                summary_pending_event_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.summary_pending_event_count),
                summary_retry_backoff_ms: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.summary_retry_backoff_ms),
                summary_gap_generation: guard.summary_delta_journal.gap_generation,
                summary_gap_degraded,
                parallel_work_refresh_scheduled: false,
                prompt_cache_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.prompt_cache_refresh_scheduled),
                prompt_cache_reconcile_scheduled: had_reconcile_scheduled
                    || deferred_working_reconcile,
                prompt_cache_key_hydration_scheduled: had_key_hydration_scheduled
                    || !deferred_working_hydration_keys.is_empty(),
                prompt_cache_pending_records: deferred_working_replay
                    .into_iter()
                    .map(|record| (record.identity.clone(), record))
                    .collect(),
                prompt_cache_pending_key_hydrations: deferred_working_hydration_keys,
                prompt_cache_candidate_refill_required: false,
                prompt_cache_applied_terminal_ids: if matches!(
                    topic,
                    SubscriptionTopic::PromptCacheWindow { .. }
                        | SubscriptionTopic::PromptCacheStickyWindow { .. }
                        | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) {
                    prompt_cache_applied_terminal_ids
                } else {
                    guard
                        .topics
                        .get(&topic_key)
                        .map(|entry| entry.prompt_cache_applied_terminal_ids.clone())
                        .unwrap_or_default()
                },
                prompt_cache_coalesced_event_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.prompt_cache_coalesced_event_count),
                prompt_cache_full_hydration_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(1, |entry| {
                        entry.prompt_cache_full_hydration_count.saturating_add(1)
                    }),
                prompt_cache_bounded_key_hydration_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.prompt_cache_bounded_key_hydration_count),
                prompt_cache_baseline_at: Some(Instant::now()),
                prompt_cache_baseline_row_id: prompt_cache_build
                    .as_ref()
                    .map_or(0, |build| build.baseline_row_id),
                prompt_cache_response_source: if guard.topics.contains_key(&topic_key) {
                    "database_reconcile"
                } else {
                    "initial_baseline"
                },
                prompt_cache_reconcile_required: deferred_working_reconcile,
                prompt_cache_pressure_deferred: false,
                latest_live_snapshot: guard
                    .topics
                    .get(&topic_key)
                    .and_then(|entry| entry.latest_live_snapshot.clone()),
                calendar_anchor: subscription_calendar_anchor(&topic),
                continuity_reset_cursor,
                dashboard_materializer,
                dashboard_base_revision: next_cursor,
                dashboard_materialized_revision,
                snapshot_payload: built_payload.snapshot_payload(),
                snapshot_frame,
                snapshot_bytes: payload_bytes,
                replay_events: guard
                    .topics
                    .get(&topic_key)
                    .filter(|entry| !entry.dirty)
                    .map(|entry| entry.replay_events.clone())
                    .unwrap_or_default(),
                replay_bytes: guard
                    .topics
                    .get(&topic_key)
                    .filter(|entry| !entry.dirty)
                    .map_or(0, |entry| entry.replay_bytes),
            };
            if emit_live {
                let replay_event = ReplayableTopicEvent {
                    frame: frame.clone(),
                    full_frame: full_frame.clone(),
                    bytes: full_frame.retained_bytes(),
                    emitted_at: Utc::now(),
                };
                next.replay_events.push_back(replay_event);
                next.replay_bytes = next
                    .replay_bytes
                    .saturating_add(full_frame.retained_bytes());
                prune_replay_window(&mut next.replay_events, &mut next.replay_bytes);
            }
            guard.topics.insert(topic_key.clone(), next.clone());
            if prompt_cache_build.is_some() {
                observe_prompt_cache_baseline_publication(&state.observability);
            }
            let dispatch = emit_live.then_some(SubscriptionDispatchEvent { frame, full_frame });
            (
                next,
                dispatch,
                schedule_initial_hydration.then(|| topic.clone()),
                schedule_initial_reconcile.then(|| topic.clone()),
            )
        };

        tracing::debug!(
            topic_key,
            schema_epoch,
            emit_live,
            snapshot_build_ms = started.elapsed().as_millis() as u64,
            payload_bytes = cached.snapshot_bytes,
            "subscription topic snapshot built"
        );

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
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        if let Some(topic) = initial_reconcile {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }

        Ok(Some(cached))
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

    pub(crate) async fn materialize_dashboard_parallel_work(&self) {
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

    pub(crate) async fn materialize_pending_dashboard_topics(
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

    pub(crate) async fn apply_account_effective_routing_rules_change(
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

    pub(crate) async fn commit_dashboard_materialized_frame(
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
            let full_frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                pending.topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let delta_payload = matches!(
                &cached.topic,
                SubscriptionTopic::TimeseriesOpenWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            )
            .then(|| {
                build_dashboard_delta_payload(
                    &cached.topic,
                    cached.snapshot_frame.payload_bytes.as_ref(),
                    full_frame.payload_bytes.as_ref(),
                )
            })
            .flatten()
            .filter(|delta| delta.len() < full_frame.payload_bytes.len());
            let frame = if let Some(delta) = delta_payload {
                Arc::new(self.serialize_delta_frame(
                    cached.descriptor.clone(),
                    pending.topic_key.clone(),
                    cached.schema_epoch.clone(),
                    next_cursor,
                    cached.cursor,
                    delta,
                )?)
            } else {
                full_frame.clone()
            };
            cached.cursor = next_cursor;
            cached.snapshot_bytes = full_frame.payload_bytes.len();
            cached.snapshot_frame = full_frame.clone();
            cached.dashboard_materialized_revision = Some(pending.revision);
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                full_frame: full_frame.clone(),
                bytes: full_frame.retained_bytes(),
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached
                .replay_bytes
                .saturating_add(full_frame.retained_bytes());
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame, full_frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }
}
