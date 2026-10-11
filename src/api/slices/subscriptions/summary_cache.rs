use super::*;

impl SubscriptionHub {
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
                let cached = match self.ensure_cached_topic(state.clone(), topic.clone()).await {
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
                    let replay_bytes = events.iter().map(|event| event.bytes).sum::<usize>();
                    tracing::debug!(
                        topic_key,
                        replay_event_count,
                        replay_bytes,
                        "subscription replay hit"
                    );
                    for event in events {
                        initial.push(PreparedTopicFrame {
                            frame: event.frame,
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

    pub(super) async fn replay_events_for_resume(
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
            return self.refresh_topic(state, topic, false).await;
            #[cfg(not(test))]
            return Err(ApiError::from(anyhow!(
                "subscription topic setup requires an active owner"
            )));
        }
        for _ in 0..SUBSCRIPTION_INITIAL_TOPIC_BUILD_ATTEMPTS {
            let refreshed = self
                .refresh_topic_if_active(state.clone(), topic.clone(), false)
                .await?;
            if let Some(cached) = refreshed {
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
}
