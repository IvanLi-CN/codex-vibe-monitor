use super::*;

impl SubscriptionHub {
    pub(crate) async fn prompt_cache_projection_health(
        &self,
    ) -> PromptCacheTopicProjectionHealthSnapshot {
        let guard = self.state.lock().await;
        let prompt_topics = guard.topics.iter().filter(|(_, cached)| {
            matches!(
                cached.topic,
                SubscriptionTopic::PromptCacheWindow { .. }
                    | SubscriptionTopic::PromptCacheStickyWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            )
        });
        let mut snapshot = PromptCacheTopicProjectionHealthSnapshot {
            mode: "memory".to_string(),
            response_source: "memory".to_string(),
            recovery_state: "healthy".to_string(),
            ..PromptCacheTopicProjectionHealthSnapshot::default()
        };
        for (topic_key, cached) in prompt_topics {
            let active = guard
                .active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                > 0;
            snapshot.active_topic_count = snapshot
                .active_topic_count
                .saturating_add(u64::from(active));
            snapshot.dirty_key_count = snapshot
                .dirty_key_count
                .saturating_add(cached.prompt_cache_pending_records.len() as u64);
            snapshot.coalesced_event_count = snapshot
                .coalesced_event_count
                .saturating_add(cached.prompt_cache_coalesced_event_count);
            snapshot.full_hydration_count = snapshot
                .full_hydration_count
                .saturating_add(cached.prompt_cache_full_hydration_count);
            snapshot.bounded_key_hydration_count = snapshot
                .bounded_key_hydration_count
                .saturating_add(cached.prompt_cache_bounded_key_hydration_count);
            if active && cached.prompt_cache_response_source == "database_bounded_key_hydrate" {
                snapshot.bounded_cold_recovery_topic_count =
                    snapshot.bounded_cold_recovery_topic_count.saturating_add(1);
            }
            snapshot.live_path_db_read_count = snapshot
                .live_path_db_read_count
                .saturating_add(cached.prompt_cache_full_hydration_count.saturating_sub(1));
            snapshot.baseline_age_ms = snapshot.baseline_age_ms.max(
                cached
                    .prompt_cache_baseline_at
                    .map(|started| started.elapsed().as_millis() as u64)
                    .unwrap_or_default(),
            );
            if active && cached.dirty {
                snapshot.dirty_last_good_topic_count =
                    snapshot.dirty_last_good_topic_count.saturating_add(1);
                if cached.prompt_cache_pressure_deferred {
                    snapshot.pressure_deferred_topic_count =
                        snapshot.pressure_deferred_topic_count.saturating_add(1);
                } else {
                    snapshot.failed_or_stale_topic_count =
                        snapshot.failed_or_stale_topic_count.saturating_add(1);
                }
                snapshot.response_source = "dirty_last_good".to_string();
            } else if cached.prompt_cache_response_source != "memory" {
                snapshot.response_source = cached.prompt_cache_response_source.to_string();
            }
        }
        snapshot.recovery_state = if snapshot.failed_or_stale_topic_count > 0 {
            "failed_or_stale"
        } else if snapshot.live_path_db_read_count > 0 {
            "hot_db_read"
        } else if snapshot.bounded_cold_recovery_topic_count > 0 {
            "bounded_cold_recovery"
        } else if snapshot.pressure_deferred_topic_count > 0 {
            "pressure_deferred"
        } else {
            "healthy"
        }
        .to_string();
        snapshot
    }

    pub(crate) fn new() -> Self {
        let (broadcaster, _) = broadcast::channel(1_024);
        Self {
            state: Mutex::new(SubscriptionHubState::default()),
            summary_projection_refresh: tokio::sync::Mutex::new(()),
            summary_coverage_recovery: tokio::sync::Mutex::new(()),
            managed_task_refresh_serial: tokio::sync::Mutex::new(()),
            broadcaster,
            runtime_mutation_bus: Arc::new(RuntimeMutationBus::new()),
            runtime_topic_recovery_notify: Arc::new(Notify::new()),
            internal_broadcast_listener_count: AtomicUsize::new(0),
            serialization_count: AtomicU64::new(0),
            dashboard_topology_counters: DashboardDeliveryTopologyCounters::default(),
            #[cfg(test)]
            dashboard_topology_sse_frame_observations: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn summary_snapshot(&self, key: &SummarySnapshotKey) -> Option<StatsResponse> {
        self.state
            .lock()
            .await
            .summary_snapshots
            .get(key)
            .and_then(SummarySnapshotEntry::fresh_response)
    }

    pub(crate) async fn summary_projection(&self) -> Option<Arc<SummaryProjection>> {
        self.state.lock().await.summary_projection.clone()
    }

    pub(crate) async fn renew_summary_projection_freshness_if_generation_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let guard = self.state.lock().await;
        guard.summary_projection.as_ref().is_some_and(|projection| {
            projection.renew_freshness_if_generation_matches(generation_fence)
        })
    }

    pub(crate) async fn renew_summary_projection_freshness_if_live_tail_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let guard = self.state.lock().await;
        guard.summary_projection.as_ref().is_some_and(|projection| {
            projection.renew_freshness_if_live_tail_matches(generation_fence)
        })
    }

    pub(crate) async fn renew_summary_projection_freshness_if_coverage_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let guard = self.state.lock().await;
        guard.summary_projection.as_ref().is_some_and(|projection| {
            projection.renew_freshness_if_coverage_matches(generation_fence)
        })
    }

    // Only acknowledged terminal deltas are admitted to this queue. When it remains bounded
    // and contains an identity the immutable baseline has not absorbed, its values are the
    // exact delta between that baseline and the rolling read model.
    pub(crate) async fn register_summary_delta_pending(
        &self,
        delta: DashboardActivityTerminalDelta,
    ) {
        let mut state = self.state.lock().await;
        if !state.summary_delta_journal.register_pending(delta) {
            tracing::warn!(
                pending_terminal_count = state.summary_delta_journal.pending.len(),
                pending_terminal_bytes = state.summary_delta_journal.pending_bytes,
                overflowed_through = ?state.summary_delta_journal.overflowed_through_sequence,
                "pending Summary Delta Journal registration reached a bounded gap"
            );
        }
    }

    pub(crate) async fn rollback_summary_delta_pending(&self, terminal_sequence: Option<u64>) {
        self.state
            .lock()
            .await
            .summary_delta_journal
            .rollback_pending(terminal_sequence);
    }

    pub(crate) async fn renew_summary_projection_freshness_from_delta_journal(&self) -> bool {
        let state = self.state.lock().await;
        let Some(projection) = state.summary_projection.as_ref().cloned() else {
            return false;
        };
        if state
            .summary_delta_journal
            .overflowed_through_sequence
            .is_some()
        {
            return false;
        }
        let has_unabsorbed_delta = state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| &entry.delta)
            .chain(state.summary_delta_journal.replayed_entries.iter())
            .any(|delta| !projection.contains_persisted_live_terminal_delta(delta));
        if !has_unabsorbed_delta {
            return false;
        }
        projection.renew_freshness_from_delta_journal();
        true
    }

    pub(crate) async fn renew_summary_projection_freshness_from_delta_journal_if_coverage_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let state = self.state.lock().await;
        let Some(projection) = state.summary_projection.as_ref().cloned() else {
            return false;
        };
        if !projection.coverage_sources_match(generation_fence) {
            return false;
        }
        // A reconstructed tail whose target changed is still awaiting fenced publication.
        // Renewing from its partial overlay would skip the next durable replay and hide a
        // terminal committed between reconstruction and publication.
        if state.summary_delta_journal.reconciliation_completed {
            return false;
        }
        if state
            .summary_delta_journal
            .overflowed_through_sequence
            .is_some()
        {
            return false;
        }
        let has_unabsorbed_delta = state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| &entry.delta)
            .chain(state.summary_delta_journal.replayed_entries.iter())
            .any(|delta| !projection.contains_persisted_live_terminal_delta(delta));
        if !has_unabsorbed_delta {
            return false;
        }
        projection.renew_freshness_from_delta_journal();
        true
    }

    pub(crate) async fn summary_delta_journal_gap_count(&self) -> usize {
        self.state
            .lock()
            .await
            .summary_delta_journal
            .gap_proofs
            .len()
    }

    pub(crate) async fn set_summary_live_tail_readiness(
        &self,
        readiness: SummaryLiveTailReadiness,
    ) {
        self.state.lock().await.summary_live_tail_readiness = readiness;
    }

    pub(crate) async fn reconcile_summary_delta_tail(
        &self,
        deltas: &[DashboardActivityTerminalDelta],
        target_terminal_watermark: u64,
        source_tail_complete: bool,
        expected_projection_revision: u64,
        expected_generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(projection) = state.summary_projection.as_ref().cloned() else {
            return false;
        };
        if projection.revision() != expected_projection_revision
            || projection.generation_fence() != expected_generation_fence
        {
            return false;
        }
        state
            .summary_delta_journal
            .retain_replayed_for_reconciliation(
                deltas,
                projection.as_ref(),
                target_terminal_watermark,
                source_tail_complete,
            )
    }

    pub(crate) async fn renew_summary_projection_after_live_tail_reconciliation(
        &self,
        expected_projection_revision: u64,
    ) -> bool {
        let mut state = self.state.lock().await;
        if (!state.summary_delta_journal.reconciliation_completed
            && state
                .summary_delta_journal
                .last_reconciled_terminal_watermark
                == 0)
            || !state.summary_delta_journal.gap_proofs.is_empty()
        {
            return false;
        }
        let Some(projection) = state.summary_projection.as_ref().cloned() else {
            return false;
        };
        if projection.revision() != expected_projection_revision {
            return false;
        }
        state.summary_delta_journal.reconciliation_completed = false;
        state
            .summary_delta_journal
            .last_reconciled_terminal_watermark = 0;
        projection.renew_freshness_from_delta_journal();
        true
    }

    pub(crate) async fn renew_summary_projection_after_live_tail_reconciliation_if_generation_matches(
        &self,
        expected_projection_revision: u64,
        expected_generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let mut state = self.state.lock().await;
        if (!state.summary_delta_journal.reconciliation_completed
            && state
                .summary_delta_journal
                .last_reconciled_terminal_watermark
                == 0)
            || !state.summary_delta_journal.gap_proofs.is_empty()
        {
            return false;
        }
        let Some(projection) = state.summary_projection.as_ref().cloned() else {
            return false;
        };
        if projection.revision() != expected_projection_revision
            || !projection
                .generation_fence()
                .coverage_sources_match(expected_generation_fence)
            || !projection
                .generation_fence()
                .live_tail_cursor()
                .at_or_behind(expected_generation_fence.live_tail_cursor())
        {
            return false;
        }
        let mut next = Arc::unwrap_or_clone(projection);
        next.advance_live_tail_fence(expected_generation_fence.live_tail_cursor());
        let next_revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision = next_revision;
        next = next.with_revision(next_revision);
        Self::store_summary_projection_locked(&mut state, next);
        state.summary_delta_journal.reconciliation_completed = false;
        state
            .summary_delta_journal
            .last_reconciled_terminal_watermark = 0;
        true
    }

    pub(crate) async fn acknowledge_summary_delta(&self, delta: DashboardActivityTerminalDelta) {
        let mut state = self.state.lock().await;
        let slice_high_watermark = delta.terminal_sequence;
        let account_scoped = delta.upstream_account_id.is_some();
        let conflicting_identity = state
            .summary_delta_journal
            .contains_conflicting_row_identity(&delta);
        let absorbed_by_projection = state.summary_projection.as_ref().is_some_and(|projection| {
            projection.durable_terminal_sequence_watermark() >= delta.terminal_sequence
                && delta.persisted_row_id.is_some_and(|row_id| {
                    projection.contains_persisted_live_terminal_identity(
                        row_id,
                        &delta.invoke_id,
                        &delta.occurred_at,
                    )
                })
        });
        let acknowledged = if absorbed_by_projection {
            state.summary_delta_journal.absorb_committed_delta(&delta);
            true
        } else {
            state
                .summary_delta_journal
                .acknowledge_pending(delta.clone())
        };
        if !acknowledged {
            tracing::warn!(
                pending_terminal_count = state.summary_delta_journal.entries.len(),
                pending_terminal_bytes = state.summary_delta_journal.bytes,
                overflowed_through = ?state.summary_delta_journal.overflowed_through_sequence,
                "rolling Summary Delta Journal reached a bounded gap"
            );
        }
        // A committed row may still be retained in the all-time overlay when its rolling
        // sequence is outside the contiguous journal (for example after a bounded overflow),
        // but a conflicting row identity is ambiguous and must never become an exact overlay.
        if delta.persisted_row_id.is_some() && !conflicting_identity {
            append_summary_all_time_delta(&mut state, &delta, slice_high_watermark);
        }
        tracing::debug!(
            stage = "delta_journal_ack",
            entry_count = state.summary_delta_journal.entries.len(),
            gap_count = state.summary_delta_journal.gap_proofs.len(),
            account_scoped,
            "acknowledged Summary Delta Journal entry"
        );
    }

    // Journal replay happens only after the SQLite transaction has committed. Replayed records
    // have no compatible in-process dashboard sequence after restart, so do not manufacture one
    // or trigger a full rolling read. Their exact values remain a bounded rolling overlay.
    pub(crate) async fn acknowledge_replayed_summary_delta(
        &self,
        mut delta: DashboardActivityTerminalDelta,
    ) {
        // A replay is reconstructed outside the live terminal sequence.  Preserve its durable
        // identity, but never let a stale process-local sequence make an all-time watermark look
        // like proof that this row was already absorbed.
        delta.terminal_sequence = 0;
        let mut state = self.state.lock().await;
        let replayed_delta = delta.clone();
        let conflicting_identity = state
            .summary_delta_journal
            .contains_conflicting_row_identity(&delta);
        let absorbed_by_projection = state.summary_projection.as_ref().is_some_and(|projection| {
            delta.persisted_row_id.is_some_and(|row_id| {
                projection.contains_persisted_live_terminal_identity(
                    row_id,
                    &delta.invoke_id,
                    &delta.occurred_at,
                )
            })
        });
        let replayed_admitted = if absorbed_by_projection {
            state.summary_delta_journal.absorb_committed_delta(&delta);
            false
        } else if !state.summary_delta_journal.append_replayed(delta) {
            tracing::warn!(
                replayed_terminal_count = state.summary_delta_journal.replayed_entries.len(),
                replayed_terminal_bytes = state.summary_delta_journal.replayed_bytes,
                gap_count = state.summary_delta_journal.gap_proofs.len(),
                "replayed Summary Delta Journal entry reached a bounded gap"
            );
            false
        } else {
            true
        };
        // Restart replay has no process-local terminal sequence (zero), but it still represents a
        // committed source row. Keep it in the independent all-time overlay and deduplicate by
        // the full durable identity rather than by sequence zero.
        if replayed_delta.persisted_row_id.is_some()
            && !conflicting_identity
            && (replayed_admitted || absorbed_by_projection)
        {
            append_summary_all_time_delta(
                &mut state,
                &replayed_delta,
                replayed_delta.terminal_sequence,
            );
        }
    }

    pub(crate) async fn summary_delta_journal_counts(&self) -> (usize, usize) {
        let state = self.state.lock().await;
        (
            state
                .summary_delta_journal
                .entries
                .len()
                .saturating_add(state.summary_delta_journal.replayed_entries.len()),
            state.summary_delta_journal.gap_proofs.len(),
        )
    }

    pub(crate) async fn summary_delta_journal_unabsorbed_identities(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| summary_delta_workload_identity(&entry.delta))
            .chain(
                state
                    .summary_delta_journal
                    .replayed_entries
                    .iter()
                    .map(summary_delta_workload_identity),
            )
            .collect()
    }

    pub(crate) async fn summary_delta_journal_unabsorbed_deltas(
        &self,
    ) -> Vec<DashboardActivityTerminalDelta> {
        let state = self.state.lock().await;
        state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| entry.delta.clone())
            .chain(state.summary_delta_journal.replayed_entries.iter().cloned())
            .collect()
    }

    pub(crate) async fn summary_delta_journal_absorbed_deltas_count(
        &self,
        before: &[DashboardActivityTerminalDelta],
    ) -> usize {
        let state = self.state.lock().await;
        let Some(projection) = state.summary_projection.as_ref() else {
            return 0;
        };
        let remaining = state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| summary_delta_workload_identity(&entry.delta))
            .chain(
                state
                    .summary_delta_journal
                    .replayed_entries
                    .iter()
                    .map(summary_delta_workload_identity),
            )
            .collect::<HashSet<_>>();
        before
            .iter()
            .filter(|delta| {
                !remaining.contains(&summary_delta_workload_identity(delta))
                    && (projection.contains_persisted_live_terminal_delta(delta)
                        || delta.persisted_row_id.is_some_and(|row_id| {
                            projection.contains_global_rollup_covered_live_terminal_identity(
                                row_id,
                                &delta.invoke_id,
                                &delta.occurred_at,
                            ) || projection.contains_global_all_time_covered_live_terminal_identity(
                                row_id,
                                &delta.invoke_id,
                                &delta.occurred_at,
                            ) && projection.global_rollup_covers_live_terminal_identity(
                                row_id,
                                &delta.occurred_at,
                            )
                        }))
            })
            .count()
    }

    pub(crate) async fn summary_source_change_cursor(&self) -> u64 {
        self.state.lock().await.summary_source_change_cursor
    }

    pub(crate) async fn advance_summary_source_change_cursor(&self, cursor: u64) {
        let mut state = self.state.lock().await;
        state.summary_source_change_cursor = state.summary_source_change_cursor.max(cursor);
    }

    pub(crate) async fn record_summary_source_change_gap(&self, cursor: u64) {
        let mut state = self.state.lock().await;
        state
            .summary_delta_journal
            .note_unknown_source_cursor_gap(cursor);
        if let Some(projection) = state.summary_projection.as_ref() {
            projection.renew_freshness_from_delta_journal();
        }
    }

    pub(crate) async fn clear_summary_source_compaction_gap(&self) {
        let mut state = self.state.lock().await;
        state.summary_delta_journal.clear_source_compaction_gap();
    }

    pub(crate) async fn record_summary_terminal_sequence_gap(&self, sequence: u64) {
        let mut state = self.state.lock().await;
        state
            .summary_delta_journal
            .note_unknown_terminal_gap(sequence);
        if let Some(projection) = state.summary_projection.as_ref() {
            projection.renew_freshness_from_delta_journal();
        }
    }

    pub(crate) async fn record_summary_source_change_scoped_gap(
        &self,
        cursor: u64,
        upstream_account_id: Option<i64>,
        occurred_at: String,
        row_id: Option<i64>,
        invoke_id: Option<String>,
    ) {
        let mut state = self.state.lock().await;
        state.summary_delta_journal.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(cursor),
            terminal_sequence: None,
            source_gap: true,
            compaction_gap: false,
            upstream_account_id,
            occurred_at,
            row_id,
            invoke_id,
        });
        if let Some(projection) = state.summary_projection.as_ref() {
            projection.renew_freshness_from_delta_journal();
        }
    }

    pub(crate) async fn mark_summary_projection_historical_recovery_required(&self) {
        let mut state = self.state.lock().await;
        let Some(projection) = state.summary_projection.as_ref() else {
            return;
        };
        let mut projection = (**projection).clone();
        projection.mark_historical_live_recovery_required();
        state.summary_projection = Some(Arc::new(projection));
    }

    pub(crate) async fn mark_summary_bootstrap_fallback_fence(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) {
        self.state.lock().await.summary_bootstrap_fallback_fence = Some(generation_fence);
    }

    pub(crate) async fn clear_summary_bootstrap_fallback_fence(&self) {
        self.state.lock().await.summary_bootstrap_fallback_fence = None;
    }

    pub(crate) async fn summary_bootstrap_fallback_fence(
        &self,
    ) -> Option<SummaryProjectionGenerationFence> {
        self.state.lock().await.summary_bootstrap_fallback_fence
    }

    pub(crate) async fn revoke_summary_projection_stale_coverage(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(current) = state.summary_projection.as_ref() else {
            return false;
        };
        let mut projection = (**current).clone();
        if !projection.revoke_stale_all_time_coverage(generation_fence) {
            return false;
        }
        let revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision = revision;
        projection = projection.with_revision(revision);
        Self::store_summary_projection_locked(&mut state, projection)
    }
    pub(crate) async fn summary_terminal_overlay_identities(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state
            .summary_delta_journal
            .entries
            .iter()
            .map(|entry| &entry.delta)
            .chain(state.summary_delta_journal.replayed_entries.iter())
            .map(|entry| {
                entry.persisted_row_id.map_or_else(
                    || format!("{}\0{}", entry.invoke_id, entry.occurred_at),
                    |row_id| format!("{row_id}\0{}\0{}", entry.occurred_at, entry.invoke_id),
                )
            })
            .collect()
    }

    pub(crate) async fn note_summary_http_interest(&self, all_time: bool) {
        let mut guard = self.state.lock().await;
        guard.summary_http_interest_at = Some(Instant::now());
        if all_time {
            guard.summary_http_all_time_interest_at = Some(Instant::now());
        }
    }

    pub(crate) async fn has_summary_owner(&self) -> bool {
        let guard = self.state.lock().await;
        guard
            .active_topic_names
            .get("stats.summary.current")
            .copied()
            .unwrap_or_default()
            > 0
            || guard
                .summary_http_interest_at
                .is_some_and(|at| at.elapsed() <= SUMMARY_SNAPSHOT_MAX_STALE)
    }

    pub(crate) async fn has_summary_all_time_owner(&self) -> bool {
        let guard = self.state.lock().await;
        guard.active_topics.iter().any(|(topic_key, topic)| {
            matches!(topic, SubscriptionTopic::SummaryCurrent { window, .. } if window == "all")
                && guard
                    .active_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0
        }) || guard
            .summary_http_all_time_interest_at
            .is_some_and(|at| at.elapsed() <= SUMMARY_SNAPSHOT_MAX_STALE)
    }

    pub(crate) fn try_lock_summary_projection_refresh(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, tokio::sync::TryLockError> {
        self.summary_projection_refresh.try_lock()
    }

    pub(crate) fn try_lock_summary_coverage_recovery(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, tokio::sync::TryLockError> {
        self.summary_coverage_recovery.try_lock()
    }

    pub(crate) async fn next_summary_projection_revision(&self) -> u64 {
        let mut state = self.state.lock().await;
        state.summary_projection_revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision
    }

    pub(crate) async fn summary_dashboard_recovery(
        &self,
    ) -> Option<Arc<SummaryDashboardRecoverySnapshot>> {
        self.state.lock().await.summary_dashboard_recovery.clone()
    }

    pub(crate) async fn store_summary_dashboard_recovery(
        &self,
        snapshot: SummaryDashboardRecoverySnapshot,
    ) {
        self.state.lock().await.summary_dashboard_recovery = Some(Arc::new(snapshot));
    }

    pub(crate) async fn clear_summary_dashboard_recovery(&self) {
        self.state.lock().await.summary_dashboard_recovery = None;
    }

    pub(crate) async fn set_summary_compacted_source_recovery_state(
        &self,
        state_value: SummaryCompactedSourceRecoveryState,
    ) {
        self.state
            .lock()
            .await
            .summary_compacted_source_recovery_state = state_value;
    }

    pub(crate) async fn summary_compacted_source_recovery_state(
        &self,
    ) -> SummaryCompactedSourceRecoveryState {
        self.state
            .lock()
            .await
            .summary_compacted_source_recovery_state
    }

    pub(crate) async fn store_summary_projection(&self, projection: SummaryProjection) {
        let mut state = self.state.lock().await;
        Self::store_summary_projection_locked(&mut state, projection);
    }

    /// Publish a projection only if it is still based on the hub revision observed by the
    /// caller.  The revision check, allocation, and swap happen under one state lock so a slow
    /// coverage reducer cannot publish an older clone after a concurrent rolling refresh.
    pub(crate) async fn store_summary_projection_if_revision(
        &self,
        mut projection: SummaryProjection,
        expected_revision: u64,
    ) -> bool {
        let mut state = self.state.lock().await;
        let current_revision = state
            .summary_projection
            .as_ref()
            .map(|current| current.revision())
            .unwrap_or_default();
        if current_revision != expected_revision {
            tracing::debug!(
                current_revision,
                expected_revision,
                "discarding summary projection based on an older hub revision"
            );
            return false;
        }
        let next_revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision = next_revision;
        projection = projection.with_revision(next_revision);
        Self::store_summary_projection_locked(&mut state, projection)
    }

    /// Publish only when the hub revision and historical coverage fence observed by the caller
    /// still describe the same immutable base. A live terminal may legitimately advance between
    /// the base projection and coverage finalization: the finalizer reads that bounded tail and
    /// installs the newer live cursor in the candidate before this CAS. The durable writer
    /// transaction around the fence read still prevents a coverage mutation from racing the
    /// swap, while the revision check prevents another in-memory publication from being lost.
    pub(crate) async fn store_summary_projection_if_revision_and_generation(
        &self,
        mut projection: SummaryProjection,
        expected_revision: u64,
        expected_generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(current_projection) = state.summary_projection.as_ref() else {
            if expected_revision != 0 {
                return false;
            }
            return false;
        };
        if current_projection.revision() != expected_revision
            || current_projection.generation_fence().live_tail_cursor()
                != expected_generation_fence.live_tail_cursor()
            || !current_projection
                .generation_fence()
                .coverage_sources_at_or_behind(expected_generation_fence)
        {
            tracing::debug!(
                current_revision = current_projection.revision(),
                expected_revision,
                "discarding summary projection based on an older generation fence"
            );
            return false;
        }
        let next_revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision = next_revision;
        projection = projection.with_revision(next_revision);
        Self::store_summary_projection_locked(&mut state, projection)
    }

    pub(crate) async fn store_summary_projection_if_revision_and_coverage_generation(
        &self,
        mut projection: SummaryProjection,
        expected_revision: u64,
        expected_generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(current_projection) = state.summary_projection.as_ref() else {
            return false;
        };
        let current_fence = current_projection.generation_fence();
        if current_projection.revision() != expected_revision
            || !current_fence.coverage_sources_match(expected_generation_fence)
            || !current_fence
                .live_tail_cursor()
                .at_or_behind(expected_generation_fence.live_tail_cursor())
        {
            return false;
        }
        let next_revision = state.summary_projection_revision.saturating_add(1);
        state.summary_projection_revision = next_revision;
        projection = projection.with_revision(next_revision);
        Self::store_summary_projection_locked(&mut state, projection)
    }

    pub(crate) fn store_summary_projection_locked(
        state: &mut SubscriptionHubState,
        projection: SummaryProjection,
    ) -> bool {
        // Projection builders run under different maintenance single-flight locks. A slower
        // builder can therefore finish after a newer rolling or coverage publication has already
        // assigned a higher revision. Keep the hub monotonically increasing so an older snapshot
        // can never overwrite a newer immutable projection during that race.
        if state
            .summary_projection
            .as_ref()
            .is_some_and(|current| current.revision() > projection.revision())
        {
            tracing::debug!(
                current_revision = state
                    .summary_projection
                    .as_ref()
                    .map(|current| current.revision()),
                rejected_revision = projection.revision(),
                "discarding stale summary projection publication"
            );
            return false;
        }
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
                    ) && projection.global_rollup_covers_live_terminal_identity(
                        row_id,
                        &entry.delta.occurred_at,
                    )
                })
        });
        state
            .summary_delta_journal
            .replayed_entries
            .retain(|entry| {
                !projection.contains_persisted_live_terminal_delta(entry)
                    // A compact global rollup identity proves this row is represented by the
                    // rolling projection as well.  An all-time overlay identity alone does not:
                    // finalization may absorb a replay for `all` while current/rolling still
                    // need the exact row from this bounded journal.
                    && !entry.persisted_row_id.is_some_and(|row_id| {
                        projection.contains_global_rollup_covered_live_terminal_identity(
                            row_id,
                            &entry.invoke_id,
                            &entry.occurred_at,
                        ) || projection.contains_global_all_time_covered_live_terminal_identity(
                            row_id,
                            &entry.invoke_id,
                            &entry.occurred_at,
                        ) && projection.global_rollup_covers_live_terminal_identity(
                            row_id,
                            &entry.occurred_at,
                        )
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
        // A durable watermark alone does not identify which proof was absorbed. Gap retirement
        // is performed by the identity-aware reconciliation path below; keeping unknown proofs
        // here prevents a late generic publication from clearing an unrelated selection.
        state.summary_terminal_overlay_all_time.retain(|delta| {
            let identity_covered = projection.all_time_terminal_delta_scope_covers(
                delta.upstream_account_id,
                delta.persisted_row_id,
                &delta.invoke_id,
                &delta.occurred_at,
                delta.terminal_sequence,
            );
            // Restart-replayed rows intentionally carry sequence zero because the original
            // process-local sequence is not durable.  A terminal watermark cannot prove that a
            // sequence-zero row was absorbed; only its full durable identity may retire it.
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
            .is_some_and(|overflowed_through| {
                projection.all_time_terminal_account_scopes_cover(overflowed_through)
            })
        {
            state.summary_terminal_overlay_all_time_overflowed_through_unknown_account = None;
            tracing::info!(
                "all-time summary account overflow markers recovered after a complete account projection refresh"
            );
        }
        if projection.all_time_terminal_coverage_complete()
            && state
                .summary_terminal_overlay_all_time_overflowed_through_sequence
                .is_some_and(|overflowed_through| {
                    projection.all_time_terminal_sequence_watermark() >= overflowed_through
                })
        {
            state.summary_terminal_overlay_all_time_overflowed_through_sequence = None;
            tracing::info!(
                durable_terminal_sequence_watermark =
                    projection.all_time_terminal_sequence_watermark(),
                "all-time summary terminal overlay recovered after a durable projection refresh"
            );
        }
        // A scoped legacy-source proof is consumed once the rebuilt immutable projection contains
        // the exact row identity.  Broad proofs (without a row id) remain fail-closed until their
        // generation watermark is explicitly absorbed.
        let gap_count = state.summary_delta_journal.gap_proofs.len();
        state.summary_delta_journal.gap_proofs.retain(|proof| {
            let Some(row_id) = proof.row_id else {
                return true;
            };
            let Some(invoke_id) = proof.invoke_id.as_deref() else {
                return true;
            };
            !projection.contains_persisted_live_terminal_identity(
                row_id,
                invoke_id,
                &proof.occurred_at,
            )
        });
        if state.summary_delta_journal.gap_proofs.len() != gap_count {
            state.summary_delta_journal.gap_generation =
                state.summary_delta_journal.gap_generation.saturating_add(1);
        }
        if state.summary_delta_journal.gap_proofs.is_empty()
            && !state.summary_delta_journal.source_compaction_gap
        {
            state.summary_delta_journal.overflowed_through_sequence = None;
            state.summary_delta_journal.gap_proof_budget_exhausted = false;
        }
        state.summary_projection = Some(Arc::new(projection));
        true
    }

    pub(crate) async fn summary_projection_terminal_overlay(
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

    pub(crate) fn summary_projection_terminal_overlay_from_state(
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
        if all_time && overflowed && !overflow_is_covered {
            return Err(ApiError::unavailable(anyhow!(
                "summary terminal overlay exceeded its bounded memory budget"
            )));
        }
        let overlay = if all_time {
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
                .collect::<Vec<_>>()
        } else {
            state
                .summary_delta_journal
                .entries
                .iter()
                .map(|entry| &entry.delta)
                .chain(state.summary_delta_journal.replayed_entries.iter())
                .filter(|entry| !projection.contains_persisted_live_terminal_delta(entry))
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut initial_slice_suppressions = overlay
            .iter()
            .map(|delta| delta.terminal_sequence)
            .collect::<HashSet<_>>();
        if let Some(slice) = state.dashboard_terminal_slice.as_deref() {
            initial_slice_suppressions.extend(
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
        Ok((overlay, initial_slice_suppressions))
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

    pub(crate) fn serialize_frame(
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

    pub(crate) fn serialize_delta_frame(
        &self,
        descriptor: SubscriptionTopicDescriptor,
        topic_key: String,
        schema_epoch: String,
        cursor: u64,
        base_cursor: u64,
        payload_bytes: Vec<u8>,
    ) -> Result<SerializedTopicFrame, ApiError> {
        let frame = serialize_topic_delta_frame(
            descriptor,
            topic_key,
            schema_epoch,
            cursor,
            base_cursor,
            payload_bytes,
        )?;
        self.serialization_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .record_serialization(&frame.descriptor.topic, frame.retained_bytes());
        Ok(frame)
    }

    #[cfg(test)]
    pub(crate) fn serialization_count(&self) -> u64 {
        self.serialization_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) async fn dashboard_topic_uses_typed_materializer(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
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

    pub(crate) fn record_dashboard_topology_frame_delivery(&self, frame: &SerializedTopicFrame) {
        self.dashboard_topology_counters
            .record_shared_frame_delivery(&frame.descriptor.topic);
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard_topology_counters.reset();
    }

    #[cfg(test)]
    pub(crate) async fn record_dashboard_topology_sse_frame_delivery(
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
    pub(crate) async fn dashboard_topology_sse_frame_observations(
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
    pub(crate) async fn reset_dashboard_topology_sse_frame_observations(&self) {
        self.dashboard_topology_sse_frame_observations
            .lock()
            .await
            .clear();
    }

    pub(crate) fn record_dashboard_topology_lag(&self, topic_names: &[String], skipped: u64) {
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

    pub(crate) async fn has_active_topic_key(&self, topic_key: &str) -> bool {
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

    pub(crate) fn register_active_topic_dependencies(
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

    pub(crate) fn release_active_topic_dependencies(
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

    pub(crate) fn active_topic_keys_for_dependency(
        guard: &SubscriptionHubState,
        dependency: &RuntimeTopicDependency,
    ) -> Vec<String> {
        guard
            .active_topic_dependencies
            .get(dependency)
            .map(|topic_keys| topic_keys.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn collect_runtime_topic_work(
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
}
