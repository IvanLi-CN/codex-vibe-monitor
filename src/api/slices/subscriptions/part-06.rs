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
            .map(|entry| format!("{}\0{}", entry.invoke_id, entry.occurred_at))
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
}
