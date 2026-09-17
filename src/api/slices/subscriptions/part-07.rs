fn refresh_summary_projection_journal(
    state: &mut SubscriptionHubState,
    projection: &SummaryProjection,
) {
    state.summary_delta_journal.base_cursor =
        state
            .summary_delta_journal
            .base_cursor
            .max(SummaryDeltaCursor(
                projection.durable_terminal_sequence_watermark(),
            ));
    state.summary_delta_journal.cursor = state
        .summary_delta_journal
        .cursor
        .max(state.summary_delta_journal.base_cursor);
    state.summary_delta_journal.entries.retain(|entry| {
        !projection.contains_persisted_live_terminal_delta(&entry.delta)
            && !entry.delta.persisted_row_id.is_some_and(|row_id| {
                projection.contains_global_rollup_covered_live_terminal_identity(
                    row_id,
                    &entry.delta.invoke_id,
                    &entry.delta.occurred_at,
                ) || projection.contains_global_all_time_covered_live_terminal_identity(
                    row_id,
                    &entry.delta.invoke_id,
                    &entry.delta.occurred_at,
                ) && projection
                    .global_rollup_covers_live_terminal_identity(row_id, &entry.delta.occurred_at)
            })
    });
    state
        .summary_delta_journal
        .replayed_entries
        .retain(|entry| {
            !projection.contains_persisted_live_terminal_delta(entry)
                && !entry.persisted_row_id.is_some_and(|row_id| {
                    projection.contains_global_rollup_covered_live_terminal_identity(
                        row_id,
                        &entry.invoke_id,
                        &entry.occurred_at,
                    ) || projection.contains_global_all_time_covered_live_terminal_identity(
                        row_id,
                        &entry.invoke_id,
                        &entry.occurred_at,
                    ) && projection
                        .global_rollup_covers_live_terminal_identity(row_id, &entry.occurred_at)
                })
        });
    state.summary_delta_journal.bytes = state
        .summary_delta_journal
        .entries
        .iter()
        .map(|entry| entry.delta.estimated_bytes)
        .sum();
    state.summary_delta_journal.replayed_bytes = state
        .summary_delta_journal
        .replayed_entries
        .iter()
        .map(|entry| entry.estimated_bytes)
        .sum();
}

fn refresh_summary_projection_overlays(
    state: &mut SubscriptionHubState,
    projection: &SummaryProjection,
) {
    state.summary_terminal_overlay_all_time.retain(|delta| {
        let identity_covered = projection.all_time_terminal_delta_scope_covers(
            delta.upstream_account_id,
            delta.persisted_row_id,
            &delta.invoke_id,
            &delta.occurred_at,
            delta.terminal_sequence,
        );
        let global_watermark_covered = delta.terminal_sequence > 0
            && projection.all_time_terminal_coverage_complete()
            && projection.all_time_terminal_scope_covers(None, "", "", delta.terminal_sequence);
        !(identity_covered || global_watermark_covered)
    });
    state.summary_terminal_overlay_all_time_bytes = state
        .summary_terminal_overlay_all_time
        .iter()
        .map(|delta| delta.estimated_bytes)
        .sum();
    state
        .summary_terminal_overlay_all_time_overflowed_through_account
        .retain(|account_id, overflowed_through| {
            !projection.all_time_terminal_scope_covers(
                Some(*account_id),
                "",
                "",
                *overflowed_through,
            )
        });
    if state
        .summary_terminal_overlay_all_time_overflowed_through_unknown_account
        .is_some_and(|value| projection.all_time_terminal_account_scopes_cover(value))
    {
        state.summary_terminal_overlay_all_time_overflowed_through_unknown_account = None;
        tracing::info!(
            "all-time summary account overflow markers recovered after a complete account projection refresh"
        );
    }
    if projection.all_time_terminal_coverage_complete()
        && state
            .summary_terminal_overlay_all_time_overflowed_through_sequence
            .is_some_and(|value| projection.all_time_terminal_sequence_watermark() >= value)
    {
        state.summary_terminal_overlay_all_time_overflowed_through_sequence = None;
        tracing::info!(
            durable_terminal_sequence_watermark = projection.all_time_terminal_sequence_watermark(),
            "all-time summary terminal overlay recovered after a durable projection refresh"
        );
    }
    state.summary_delta_journal.gap_proofs.retain(|proof| {
        let (Some(row_id), Some(invoke_id)) = (proof.row_id, proof.invoke_id.as_deref()) else {
            return true;
        };
        !projection.contains_persisted_live_terminal_identity(row_id, invoke_id, &proof.occurred_at)
    });
    if state.summary_delta_journal.gap_proofs.is_empty()
        && !state.summary_delta_journal.source_compaction_gap
    {
        state.summary_delta_journal.overflowed_through_sequence = None;
        state.summary_delta_journal.gap_proof_budget_exhausted = false;
    }
}

impl SubscriptionHub {
    fn store_summary_projection_locked(
        state: &mut SubscriptionHubState,
        projection: SummaryProjection,
    ) -> bool {
        // Projection builders run under different maintenance single-flight locks. A slower
        // builder can therefore finish after a newer rolling or coverage publication has already
        // assigned a higher revision. Keep the hub monotonically increasing so an older snapshot
        // can never overwrite a newer immutable projection during that race.
        if Self::summary_projection_publication_is_stale(state, &projection) {
            return false;
        }
        refresh_summary_projection_journal(state, &projection);
        refresh_summary_projection_overlays(state, &projection);
        state.summary_projection = Some(Arc::new(projection));
        true
    }

    fn summary_projection_publication_is_stale(
        state: &SubscriptionHubState,
        projection: &SummaryProjection,
    ) -> bool {
        let Some(current) = state.summary_projection.as_ref() else {
            return false;
        };
        if current.revision() <= projection.revision() {
            return false;
        }
        tracing::debug!(
            current_revision = current.revision(),
            rejected_revision = projection.revision(),
            "discarding stale summary projection publication"
        );
        true
    }

    async fn summary_projection_terminal_overlay(
        &self,
        projection: &SummaryProjection,
        all_time: bool,
        upstream_account_id: Option<i64>,
    ) -> Result<(Vec<DashboardActivityTerminalDelta>, HashSet<u64>), ApiError> {
        let state = self.state.lock().await;
        Self::summary_projection_terminal_overlay_from_state(
            &state,
            projection,
            all_time,
            upstream_account_id,
        )
    }

    pub(crate) async fn summary_projection_with_terminal_overlay(
        &self,
        all_time: bool,
        upstream_account_id: Option<i64>,
    ) -> Result<
        Option<(
            Arc<SummaryProjection>,
            Vec<DashboardActivityTerminalDelta>,
            HashSet<u64>,
            Vec<DeltaGapProof>,
        )>,
        ApiError,
    > {
        let state = self.state.lock().await;
        let Some(projection) = state.summary_projection.clone() else {
            return Ok(None);
        };
        let (overlay, initial_slice_suppressions) =
            Self::summary_projection_terminal_overlay_from_state(
                &state,
                projection.as_ref(),
                all_time,
                upstream_account_id,
            )?;
        let gaps = state
            .summary_delta_journal
            .gap_proofs
            .iter()
            .cloned()
            .collect();
        Ok(Some((
            projection,
            overlay,
            initial_slice_suppressions,
            gaps,
        )))
    }

    pub(crate) async fn summary_projection_with_rolling_delta(
        &self,
    ) -> Option<SummaryRollingDeltaSnapshot> {
        let state = self.state.lock().await;
        let projection = state.summary_projection.clone()?;
        let entries = state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| &entry.delta)
            .chain(state.summary_delta_journal.replayed_entries.iter())
            .filter(|entry| !projection.contains_persisted_live_terminal_delta(entry))
            .cloned()
            .collect();
        Some(SummaryRollingDeltaSnapshot {
            projection,
            entries,
            gaps: state
                .summary_delta_journal
                .gap_proofs
                .iter()
                .cloned()
                .collect(),
        })
    }

    fn summary_projection_terminal_overlay_from_state(
        state: &SubscriptionHubState,
        projection: &SummaryProjection,
        all_time: bool,
        upstream_account_id: Option<i64>,
    ) -> Result<(Vec<DashboardActivityTerminalDelta>, HashSet<u64>), ApiError> {
        let overflowed = if all_time {
            if let Some(account_id) = upstream_account_id {
                state
                    .summary_terminal_overlay_all_time_overflowed_through_account
                    .contains_key(&account_id)
                    || state
                        .summary_terminal_overlay_all_time_overflowed_through_unknown_account
                        .is_some()
            } else {
                state
                    .summary_terminal_overlay_all_time_overflowed_through_sequence
                    .is_some()
            }
        } else {
            state
                .summary_delta_journal
                .overflowed_through_sequence
                .is_some()
        };
        let overflow_is_covered = if all_time {
            if let Some(account_id) = upstream_account_id {
                let account_marker_covered = state
                    .summary_terminal_overlay_all_time_overflowed_through_account
                    .get(&account_id)
                    .is_none_or(|overflowed_through| {
                        projection.all_time_terminal_scope_covers(
                            Some(account_id),
                            "",
                            "",
                            *overflowed_through,
                        )
                    });
                let unknown_marker_covered = state
                    .summary_terminal_overlay_all_time_overflowed_through_unknown_account
                    .is_none_or(|overflowed_through| {
                        projection.all_time_terminal_scope_covers(
                            Some(account_id),
                            "",
                            "",
                            overflowed_through,
                        )
                    });
                account_marker_covered && unknown_marker_covered
            } else {
                state
                    .summary_terminal_overlay_all_time_overflowed_through_sequence
                    .is_none_or(|overflowed_through| {
                        projection.all_time_terminal_scope_covers(None, "", "", overflowed_through)
                    })
            }
        } else {
            false
        };
        if overflowed && !overflow_is_covered && all_time {
            return Err(ApiError::unavailable(anyhow!(
                "summary terminal overlay exceeded its bounded memory budget"
            )));
        }
        Ok(Self::build_summary_projection_overlay(
            state,
            projection,
            all_time,
            upstream_account_id,
        ))
    }

    fn build_summary_projection_overlay(
        state: &SubscriptionHubState,
        projection: &SummaryProjection,
        all_time: bool,
        upstream_account_id: Option<i64>,
    ) -> (Vec<DashboardActivityTerminalDelta>, HashSet<u64>) {
        let overlay: Vec<DashboardActivityTerminalDelta> = if all_time {
            state
                .summary_terminal_overlay_all_time
                .iter()
                .filter(|delta| {
                    !projection.all_time_terminal_delta_scope_covers(
                        upstream_account_id,
                        delta.persisted_row_id,
                        &delta.invoke_id,
                        &delta.occurred_at,
                        delta.terminal_sequence,
                    )
                })
                .cloned()
                .collect()
        } else {
            state
                .summary_delta_journal
                .entries
                .iter()
                .map(|entry| &entry.delta)
                .chain(state.summary_delta_journal.replayed_entries.iter())
                .filter(|entry| !projection.contains_persisted_live_terminal_delta(entry))
                .cloned()
                .collect()
        };
        let mut suppressions: HashSet<u64> = overlay
            .iter()
            .map(|delta| delta.terminal_sequence)
            .collect();
        if let Some(slice) = state.dashboard_terminal_slice.as_deref() {
            suppressions.extend(
                slice
                    .deltas
                    .iter()
                    .filter(|delta| {
                        (!all_time
                            && projection.contains_persisted_live_terminal(
                                &delta.invoke_id,
                                &delta.occurred_at,
                            ))
                            || (all_time
                                && projection.all_time_terminal_delta_scope_covers(
                                    upstream_account_id,
                                    delta.persisted_row_id,
                                    &delta.invoke_id,
                                    &delta.occurred_at,
                                    delta.terminal_sequence,
                                ))
                    })
                    .map(|delta| delta.terminal_sequence),
            );
        }
        (overlay, suppressions)
    }

    pub(crate) async fn ensure_summary_snapshot_key(&self, key: SummarySnapshotKey) -> bool {
        let mut state = self.state.lock().await;
        if state.summary_snapshots.contains_key(&key) {
            return true;
        }
        if state.summary_snapshots.len() >= SUMMARY_SNAPSHOT_MAX_KEYS {
            return false;
        }
        state
            .summary_snapshots
            .insert(key, SummarySnapshotEntry::default());
        true
    }

    pub(crate) async fn summary_snapshot_keys(&self) -> Vec<SummarySnapshotKey> {
        self.state
            .lock()
            .await
            .summary_snapshots
            .keys()
            .cloned()
            .collect()
    }

    pub(crate) async fn summary_snapshot_keys_needing_refresh(&self) -> Vec<SummarySnapshotKey> {
        self.state
            .lock()
            .await
            .summary_snapshots
            .iter()
            .filter(|(_, entry)| entry.needs_refresh())
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub(crate) async fn store_summary_snapshot(
        &self,
        key: SummarySnapshotKey,
        response: StatsResponse,
    ) {
        self.state
            .lock()
            .await
            .summary_snapshots
            .insert(key, SummarySnapshotEntry::ready(response));
    }

    #[cfg(test)]
    pub(crate) async fn store_summary_snapshot_at(
        &self,
        key: SummarySnapshotKey,
        response: StatsResponse,
        refreshed_at: Instant,
    ) {
        self.state.lock().await.summary_snapshots.insert(
            key,
            SummarySnapshotEntry {
                response: Some(response),
                refreshed_at: Some(refreshed_at),
            },
        );
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SubscriptionDispatchEvent> {
        self.broadcaster.subscribe()
    }

    pub(crate) fn publish_runtime_mutation(&self, mutation: RuntimeMutation) {
        self.runtime_mutation_bus.publish(mutation);
    }

    pub(crate) fn mark_internal_broadcast_listener_started(&self) {
        self.internal_broadcast_listener_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn has_external_broadcaster_receiver(&self, receiver_count: usize) -> bool {
        receiver_count
            > self
                .internal_broadcast_listener_count
                .load(Ordering::Relaxed)
    }

    pub(crate) fn runtime_mutation_bus_health(&self) -> RuntimeMutationBusHealth {
        self.runtime_mutation_bus.health()
    }

    pub(crate) fn runtime_mutation_bus(&self) -> Arc<RuntimeMutationBus> {
        self.runtime_mutation_bus.clone()
    }

    fn serialize_frame(
        &self,
        descriptor: SubscriptionTopicDescriptor,
        topic_key: String,
        schema_epoch: String,
        cursor: u64,
        payload_bytes: Vec<u8>,
    ) -> Result<SerializedTopicFrame, ApiError> {
        let frame =
            serialize_topic_frame(descriptor, topic_key, schema_epoch, cursor, payload_bytes)?;
        self.serialization_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .record_serialization(&frame.descriptor.topic, frame.retained_bytes());
        self.dashboard_topology_counters
            .record_cursor_advanced(&frame.descriptor.topic);
        Ok(frame)
    }

    #[cfg(test)]
    fn serialization_count(&self) -> u64 {
        self.serialization_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    async fn dashboard_topic_uses_typed_materializer(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        self.state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.dashboard_materializer.is_some())
    }

    pub(crate) fn dashboard_topology_counters(&self) -> DashboardDeliveryTopologyCounterSnapshot {
        self.dashboard_topology_counters.snapshot()
    }

    pub(crate) async fn dashboard_hot_topic_health(
        &self,
        projection: DashboardRuntimeTopologyCounterSnapshot,
    ) -> DashboardHotTopicsHealthSnapshot {
        let recovery = {
            let guard = self.state.lock().await;
            let mut recovery = DashboardHotTopicRecoveryHealth::default();
            for (topic_key, cached) in &guard.topics {
                if cached.topic.class() != SubscriptionTopicClass::HotProjection
                    || guard
                        .active_subscribers
                        .get(topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                {
                    continue;
                }
                let recovery_pending = guard.runtime_topic_recovery_queued.contains(topic_key)
                    || cached.runtime_topic_recovery_retry_at.is_some();
                let deferred = matches!(
                    cached.topic,
                    SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) && (cached.prompt_cache_pressure_deferred
                    || cached.prompt_cache_reconcile_required
                    || !cached.prompt_cache_pending_key_hydrations.is_empty());
                recovery.record(
                    &cached.topic,
                    DashboardHotTopicRecoveryState {
                        degraded: cached.dirty || recovery_pending,
                        deferred,
                    },
                );
            }
            recovery
        };
        let readiness = {
            let guard = self.state.lock().await;
            guard.summary_live_tail_readiness.clone()
        };
        let mut health = self
            .dashboard_topology_counters
            .hot_topic_health(projection, recovery);
        health.summary.summary_live_tail_reason = readiness.reason;
        health.summary.summary_live_tail_stage = readiness.stage;
        health.summary.summary_live_tail_gap_count = readiness.gap_count;
        health.summary.summary_live_tail_watermark = readiness.watermark;
        health.summary.summary_live_tail_epoch = readiness.epoch;
        health.summary.summary_live_tail_elapsed_ms = readiness.elapsed_ms;
        health
    }

    pub(crate) fn dashboard_delivery_has_degraded_signal(&self) -> bool {
        self.dashboard_topology_counters.has_degraded_signal()
    }

    fn record_dashboard_topology_frame_delivery(&self, frame: &SerializedTopicFrame) {
        self.dashboard_topology_counters
            .record_shared_frame_delivery(&frame.descriptor.topic);
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard_topology_counters.reset();
    }

    #[cfg(test)]
    async fn record_dashboard_topology_sse_frame_delivery(
        &self,
        attempt: u64,
        frame: &Arc<SerializedTopicFrame>,
    ) {
        let mut observations = self.dashboard_topology_sse_frame_observations.lock().await;
        observations
            .entry(attempt)
            .or_default()
            .entry(frame.descriptor.topic.clone())
            .or_default()
            .push(frame.clone());
    }

    #[cfg(test)]
    async fn dashboard_topology_sse_frame_observations(
        &self,
        attempt: u64,
    ) -> DashboardTopologyFramesByTopic {
        self.dashboard_topology_sse_frame_observations
            .lock()
            .await
            .get(&attempt)
            .cloned()
            .unwrap_or_default()
    }

    #[cfg(test)]
    async fn reset_dashboard_topology_sse_frame_observations(&self) {
        self.dashboard_topology_sse_frame_observations
            .lock()
            .await
            .clear();
    }

    fn record_dashboard_topology_lag(&self, topic_names: &[String], skipped: u64) {
        for topic_name in topic_names {
            self.dashboard_topology_counters
                .record_lag(topic_name, skipped);
        }
    }

    pub(crate) async fn has_active_topic_name(&self, topic_name: &str) -> bool {
        self.active_topic_subscriber_count(topic_name).await > 0
    }

    pub(crate) async fn active_topic_subscriber_count(&self, topic_name: &str) -> usize {
        let guard = self.state.lock().await;
        guard
            .active_topic_names
            .get(topic_name)
            .copied()
            .unwrap_or_default()
    }

    async fn has_active_topic_key(&self, topic_key: &str) -> bool {
        let guard = self.state.lock().await;
        guard
            .active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            > 0
    }

    pub(crate) fn has_active_topic_name_sync(&self, topic_name: &str) -> bool {
        let Ok(guard) = self.state.try_lock() else {
            return true;
        };
        guard
            .active_topic_names
            .get(topic_name)
            .copied()
            .unwrap_or_default()
            > 0
    }

    pub(crate) async fn mark_topic_name_dirty(&self, topic_name: &str) {
        let mut guard = self.state.lock().await;
        for cached in guard
            .topics
            .values_mut()
            .filter(|cached| cached.topic.name() == topic_name)
        {
            cached.dirty = true;
            cached.latest_live_snapshot = None;
        }
    }

    pub(crate) async fn has_active_dashboard_activity_live_topic(&self) -> bool {
        let guard = self.state.lock().await;
        guard.dashboard_live_subscriber_count > 0
    }

    pub(crate) async fn dashboard_activity_live_subscriber_count(&self) -> usize {
        self.state.lock().await.dashboard_live_subscriber_count
    }

    pub(crate) fn has_active_dashboard_activity_live_topic_sync(&self) -> bool {
        let Ok(guard) = self.state.try_lock() else {
            return true;
        };
        guard.dashboard_live_subscriber_count > 0
    }

    fn register_active_topic_dependencies(
        guard: &mut SubscriptionHubState,
        topic_key: &str,
        topic: &SubscriptionTopic,
    ) {
        for dependency in topic.runtime_topic_dependencies() {
            guard
                .active_topic_dependencies
                .entry(dependency)
                .or_default()
                .insert(topic_key.to_string());
        }
    }

    fn release_active_topic_dependencies(
        guard: &mut SubscriptionHubState,
        topic_key: &str,
        topic: &SubscriptionTopic,
    ) {
        for dependency in topic.runtime_topic_dependencies() {
            let Some(topic_keys) = guard.active_topic_dependencies.get_mut(&dependency) else {
                continue;
            };
            topic_keys.remove(topic_key);
            if topic_keys.is_empty() {
                guard.active_topic_dependencies.remove(&dependency);
            }
        }
    }

    fn active_topic_keys_for_dependency(
        guard: &SubscriptionHubState,
        dependency: &RuntimeTopicDependency,
    ) -> Vec<String> {
        guard
            .active_topic_dependencies
            .get(dependency)
            .map(|topic_keys| topic_keys.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn collect_runtime_topic_work(
        guard: &SubscriptionHubState,
        mutations: &[SequencedRuntimeMutation],
    ) -> Vec<RuntimeTopicWork> {
        let candidate_topic_keys = mutations
            .iter()
            .flat_map(|mutation| mutation.mutation.topic_dependencies())
            .flat_map(|dependency| Self::active_topic_keys_for_dependency(guard, &dependency))
            .collect::<HashSet<_>>();

        candidate_topic_keys
            .into_iter()
            .filter_map(|topic_key| {
                let topic = guard.active_topics.get(&topic_key)?;
                if guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                    || guard
                        .topics
                        .get(&topic_key)
                        .is_some_and(|cached| cached.dirty)
                    || !mutations
                        .iter()
                        .any(|mutation| topic.is_affected_by_runtime_mutation(&mutation.mutation))
                {
                    return None;
                }
                Some(RuntimeTopicWork {
                    topic: topic.clone(),
                    terminal_event_count: mutations
                        .iter()
                        .filter(|mutation| {
                            topic.is_affected_by_runtime_mutation(&mutation.mutation)
                                && mutation.mutation.is_terminal_invocation()
                        })
                        .count() as u64,
                    includes_invocation_mutation: mutations.iter().any(|mutation| {
                        topic.is_affected_by_runtime_mutation(&mutation.mutation)
                            && mutation.mutation.is_invocation()
                    }),
                })
            })
            .collect()
    }

    async fn register_topic_subscribers(
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
}
