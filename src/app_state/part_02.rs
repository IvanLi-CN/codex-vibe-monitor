impl RuntimeProjectionHub {
    pub(crate) fn new(mode: RuntimeProjectionMode) -> Self {
        Self {
            inner: std::sync::Mutex::new(ProxyRuntimeInvocationStoreInner::default()),
            mode,
            dashboard_network_speed_cache: std::sync::OnceLock::new(),
            dashboard: std::sync::Mutex::new(DashboardRuntimeProjectionState::default()),
            dashboard_publish_notify: tokio::sync::Notify::new(),
            live_path_db_read_count: AtomicU64::new(0),
            build_count: AtomicU64::new(0),
            dashboard_topology_counters: DashboardRuntimeTopologyCounters::default(),
            producer_running: AtomicBool::new(false),
            request_semantic_parse_count: AtomicU64::new(0),
            request_whole_body_materialization_count: AtomicU64::new(0),
            request_rewrite_buffer_peak_bytes: AtomicU64::new(0),
            request_pipeline_last: std::sync::Mutex::new(RequestPipelineLastState::default()),
            #[cfg(test)]
            full_record_clone_count: AtomicU64::new(0),
        }
    }

    pub(crate) fn mode(&self) -> RuntimeProjectionMode {
        self.mode
    }

    pub(crate) fn bind_dashboard_network_speed_cache(
        &self,
        cache: Arc<DashboardNetworkSpeedCache>,
    ) -> Result<()> {
        if let Some(existing) = self.dashboard_network_speed_cache.get() {
            return if Arc::ptr_eq(existing, &cache) {
                Ok(())
            } else {
                Err(anyhow!(
                    "runtime projection hub is already bound to another dashboard network cache"
                ))
            };
        }
        self.dashboard_network_speed_cache
            .set(cache)
            .map_err(|_| anyhow!("dashboard network speed cache is already bound"))
    }

    pub(crate) fn dashboard_live_projection(&self) -> DashboardLiveProjection<'_> {
        DashboardLiveProjection { hub: self }
    }

    fn update_dashboard_runtime_record(
        &self,
        key: RuntimeInvocationKey,
        record: Option<&ApiInvocation>,
        trigger: &'static str,
    ) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        if record.is_none() {
            dashboard.baseline_records.remove(&key);
        }
        let previous = dashboard.projection_records.remove(&key);
        if let Some(previous) = previous.as_ref() {
            update_dashboard_live_core(
                dashboard
                    .live_core
                    .get_or_insert_with(empty_dashboard_live_core),
                previous,
                false,
            );
        }
        let next = record.and_then(|record| {
            if dashboard.source_scope == InvocationSourceScope::ProxyOnly
                && record.source != SOURCE_PROXY
            {
                None
            } else {
                dashboard_projection_record_from_invocation(key.clone(), record, previous.as_ref())
            }
        });
        if let Some(next) = next {
            update_dashboard_live_core(
                dashboard
                    .live_core
                    .get_or_insert_with(empty_dashboard_live_core),
                &next,
                true,
            );
            dashboard.projection_records.insert(key, next);
        }
        dashboard
            .live_core
            .get_or_insert_with(empty_dashboard_live_core)
            .accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        mark_dashboard_state_dirty(&mut dashboard, trigger, Instant::now());
        self.dashboard_publish_notify.notify_one();
    }

    fn sync_dashboard_runtime_key(&self, key: &RuntimeInvocationKey, trigger: &'static str) {
        let Ok(runtime) = self.inner.lock() else {
            return;
        };
        let record = runtime.records.get(key).map(|entry| &entry.record);
        self.update_dashboard_runtime_record(key.clone(), record, trigger);
    }

    fn rebuild_dashboard_runtime_records(&self, trigger: &'static str) {
        let Ok(runtime) = self.inner.lock() else {
            return;
        };
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let source_scope = dashboard.source_scope;
        let mut records = dashboard.baseline_records.clone();
        for key in runtime.terminal_tombstones.keys() {
            records.remove(key);
        }
        for key in runtime.projection_tombstones.keys() {
            records.remove(key);
        }
        for (key, entry) in &runtime.records {
            if runtime.terminal_tombstones.contains_key(key) {
                records.remove(key);
                continue;
            }
            if source_scope == InvocationSourceScope::ProxyOnly
                && entry.record.source != SOURCE_PROXY
            {
                continue;
            }
            let previous = records.get(key);
            match dashboard_projection_record_from_invocation(key.clone(), &entry.record, previous)
            {
                Some(projected) => {
                    records.insert(key.clone(), projected);
                }
                None => {
                    records.remove(key);
                }
            }
        }
        let mut core = empty_dashboard_live_core();
        for record in records.values() {
            update_dashboard_live_core(&mut core, record, true);
        }
        core.accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        dashboard.projection_records = records;
        dashboard.live_core = Some(core);
        mark_dashboard_state_dirty(&mut dashboard, trigger, Instant::now());
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn mark_dashboard_dirty(&self, trigger: &'static str) {
        self.mark_dashboard_dirty_at(trigger, Instant::now());
    }

    pub(crate) fn mark_dashboard_dirty_at(&self, _trigger: &'static str, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        mark_dashboard_state_dirty(&mut dashboard, _trigger, now);
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn mark_dashboard_network_dirty(&self) {
        self.mark_dashboard_network_dirty_at(Instant::now());
    }

    fn mark_dashboard_network_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let DashboardRuntimeProjectionState {
            network_dirty_generation,
            pending_network_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            network_dirty_generation,
            pending_network_deadline,
            now,
            DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    fn mark_dashboard_terminal_dirty(&self) {
        self.mark_dashboard_terminal_dirty_at(Instant::now());
    }

    fn mark_dashboard_terminal_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let DashboardRuntimeProjectionState {
            terminal_dirty_generation,
            pending_terminal_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            terminal_dirty_generation,
            pending_terminal_deadline,
            now,
            DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) async fn wait_for_dashboard_publish_signal(&self) {
        self.dashboard_publish_notify.notified().await;
    }

    pub(crate) fn pending_dashboard_deadline(&self) -> Option<Instant> {
        self.dashboard
            .lock()
            .ok()
            .and_then(|dashboard| dashboard.pending_deadline)
    }

    pub(crate) fn pending_dashboard_publish_window(
        &self,
    ) -> Option<DashboardProjectionPublishWindow> {
        let dashboard = self.dashboard.lock().ok()?;
        [
            (
                DashboardProjectionSlice::Current,
                dashboard.pending_deadline,
                dashboard.dirty_generation,
            ),
            (
                DashboardProjectionSlice::Network,
                dashboard.pending_network_deadline,
                dashboard.network_dirty_generation,
            ),
            (
                DashboardProjectionSlice::Terminal,
                dashboard.pending_terminal_deadline,
                dashboard.terminal_dirty_generation,
            ),
        ]
        .into_iter()
        .filter_map(|(slice, deadline, generation)| {
            deadline.map(|deadline| DashboardProjectionPublishWindow {
                slice,
                deadline,
                generation,
            })
        })
        .min_by_key(|window| window.deadline)
    }

    pub(crate) fn has_pending_dashboard_terminal_publish(&self) -> bool {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.pending_terminal_deadline.is_some())
            .unwrap_or(false)
    }

    pub(crate) fn begin_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) -> Option<DashboardProjectionPublishWindow> {
        let mut dashboard = self.dashboard.lock().ok()?;
        let generation = match window.slice {
            DashboardProjectionSlice::Current => {
                if dashboard.pending_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_deadline = None;
                dashboard.dirty_generation
            }
            DashboardProjectionSlice::Network => {
                if dashboard.pending_network_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_network_deadline = None;
                dashboard.network_dirty_generation
            }
            DashboardProjectionSlice::Terminal => {
                if dashboard.pending_terminal_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_terminal_deadline = None;
                dashboard.terminal_dirty_generation
            }
        };
        Some(DashboardProjectionPublishWindow {
            slice: window.slice,
            deadline: window.deadline,
            generation,
        })
    }

    pub(crate) fn complete_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) {
        if let Ok(dashboard) = self.dashboard.lock() {
            let generation = match window.slice {
                DashboardProjectionSlice::Current => dashboard.dirty_generation,
                DashboardProjectionSlice::Network => dashboard.network_dirty_generation,
                DashboardProjectionSlice::Terminal => dashboard.terminal_dirty_generation,
            };
            debug_assert!(generation >= window.generation);
        }
    }

    pub(crate) fn is_memory_ready(&self) -> bool {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.memory_ready && dashboard.degraded_reason.is_none())
            .unwrap_or(false)
    }

    pub(crate) fn dashboard_generation(&self) -> u64 {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.dirty_generation)
            .unwrap_or_default()
    }

    pub(crate) fn capture_memory_snapshot(&self) -> Result<DashboardProjectionCapture> {
        let candidate = self.dashboard_live_projection().snapshot()?;
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .current
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        let changed = dashboard
            .last_good
            .as_ref()
            .is_none_or(|current| !dashboard_current_snapshot_content_eq(current, &candidate));
        if !changed {
            let revision = dashboard
                .last_good
                .as_ref()
                .expect("unchanged projection has a last-good snapshot")
                .revision;
            let mut snapshot = candidate;
            snapshot.revision = revision;
            dashboard.last_good = Some(snapshot.clone());
            dashboard.memory_ready = true;
            dashboard.degraded_reason = None;
            dashboard.last_snapshot_origin = Some("memory");
            return Ok(DashboardProjectionCapture {
                snapshot,
                changed: false,
                snapshot_origin: "memory",
            });
        }

        dashboard.current_revision = dashboard.current_revision.saturating_add(1);
        let mut snapshot = candidate;
        snapshot.revision = dashboard.current_revision;
        self.dashboard_topology_counters
            .current
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        dashboard.last_good = Some(snapshot.clone());
        dashboard.last_good_at = Some(Instant::now());
        dashboard.last_snapshot_origin = Some("memory");
        dashboard.degraded_reason = None;
        dashboard.memory_ready = true;
        Ok(DashboardProjectionCapture {
            snapshot,
            changed: true,
            snapshot_origin: "memory",
        })
    }

    pub(crate) fn capture_network_slice(&self) -> Result<DashboardNetworkProjectionCapture> {
        let dashboard_network_speed_cache = self
            .dashboard_network_speed_cache
            .get()
            .ok_or_else(|| anyhow!("dashboard network speed cache is not bound"))?;
        let (network_open_buckets, known_account_ids) = {
            let dashboard = self
                .dashboard
                .lock()
                .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
            let network_open_buckets = dashboard
                .persistence_baseline
                .as_ref()
                .map(|baseline| baseline.network_open_buckets.clone())
                .unwrap_or_default();
            let known_account_ids = dashboard
                .network_last_good
                .as_ref()
                .map(|slice| {
                    slice
                        .accounts
                        .iter()
                        .map(|account| account.upstream_account_id)
                        .collect()
                })
                .unwrap_or_default();
            (network_open_buckets, known_account_ids)
        };
        let candidate = DashboardNetworkProjectionSlice::from_memory(
            dashboard_network_speed_cache.as_ref(),
            &network_open_buckets,
            &known_account_ids,
        );
        self.dashboard_topology_counters
            .network
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        let changed = dashboard
            .network_last_good
            .as_ref()
            .is_none_or(|current| !dashboard_network_slice_content_eq(current, &candidate));
        if !changed {
            return Ok(DashboardNetworkProjectionCapture {
                slice: dashboard
                    .network_last_good
                    .clone()
                    .expect("unchanged network projection has a last-good snapshot"),
                changed: false,
            });
        }

        dashboard.network_revision = dashboard.network_revision.saturating_add(1);
        let mut snapshot = candidate;
        snapshot.revision = dashboard.network_revision;
        self.dashboard_topology_counters
            .network
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        dashboard.network_last_good = Some(snapshot.clone());
        Ok(DashboardNetworkProjectionCapture {
            slice: snapshot,
            changed: true,
        })
    }

    pub(crate) fn record_dashboard_terminal_delta(&self, delta: DashboardActivityTerminalDelta) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        if dashboard.terminal_pending_deltas.len() >= DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING
            || dashboard
                .terminal_pending_delta_bytes
                .saturating_add(delta.estimated_bytes)
                > DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING_BYTES
        {
            dashboard.degraded_reason = Some("terminal_slice_hard_limit");
            tracing::warn!(
                pending_delta_count = dashboard.terminal_pending_deltas.len(),
                pending_delta_estimated_bytes = dashboard.terminal_pending_delta_bytes,
                "dashboard terminal projection slice reached its hard limit"
            );
            return;
        }
        dashboard.terminal_pending_delta_bytes = dashboard
            .terminal_pending_delta_bytes
            .saturating_add(delta.estimated_bytes);
        dashboard.terminal_pending_deltas.push_back(delta);
        let now = Instant::now();
        let DashboardRuntimeProjectionState {
            terminal_dirty_generation,
            pending_terminal_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            terminal_dirty_generation,
            pending_terminal_deadline,
            now,
            DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn discard_dashboard_terminal_delta(&self, invoke_id: &str, occurred_at: &str) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let mut removed_bytes = 0usize;
        dashboard.terminal_pending_deltas.retain(|delta| {
            let retain = delta.invoke_id != invoke_id || delta.occurred_at != occurred_at;
            if !retain {
                removed_bytes = removed_bytes.saturating_add(delta.estimated_bytes);
            }
            retain
        });
        dashboard.terminal_pending_delta_bytes = dashboard
            .terminal_pending_delta_bytes
            .saturating_sub(removed_bytes);
    }

    pub(crate) fn capture_terminal_slice(&self) -> Option<DashboardTerminalProjectionCapture> {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return None;
        };
        self.dashboard_topology_counters
            .terminal
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        if dashboard.terminal_published_generation == dashboard.terminal_dirty_generation {
            return None;
        }
        dashboard.terminal_published_generation = dashboard.terminal_dirty_generation;
        let deltas = std::mem::take(&mut dashboard.terminal_pending_deltas)
            .into_iter()
            .collect::<Vec<_>>();
        dashboard.terminal_pending_delta_bytes = 0;
        if deltas.is_empty() {
            return None;
        }
        dashboard.terminal_revision = dashboard.terminal_revision.saturating_add(1);
        self.dashboard_topology_counters
            .terminal
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        Some(DashboardTerminalProjectionCapture {
            revision: dashboard.terminal_revision,
            deltas,
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_terminal_slice_count(&self) -> usize {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.terminal_pending_deltas.len())
            .unwrap_or_default()
    }

    pub(crate) fn legacy_live_snapshot(
        &self,
        mut current: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        if let Ok(dashboard) = self.dashboard.lock()
            && let Some(network) = dashboard.network_last_good.as_ref()
        {
            apply_dashboard_network_slice_to_live_snapshot(&mut current, network);
        }
        self.commit_legacy_live_snapshot(current)
    }

    pub(crate) fn legacy_live_snapshot_for_network(
        &self,
        network: &DashboardNetworkProjectionSlice,
    ) -> Option<DashboardActivityLiveSnapshot> {
        let mut current = self.dashboard.lock().ok()?.last_good.clone()?;
        apply_dashboard_network_slice_to_live_snapshot(&mut current, network);
        Some(self.commit_legacy_live_snapshot(current))
    }

    fn commit_legacy_live_snapshot(
        &self,
        mut candidate: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            candidate.revision = reserve_dashboard_activity_live_revision();
            return candidate;
        };
        if let Some(previous) = dashboard.legacy_last_good.as_ref()
            && dashboard_legacy_snapshot_content_eq(previous, &candidate)
        {
            return previous.clone();
        }
        candidate.revision = reserve_dashboard_activity_live_revision();
        dashboard.legacy_last_good = Some(candidate.clone());
        candidate
    }

    fn runtime_records_for_persistence_baseline(
        &self,
        baseline: &DashboardRuntimeProjectionBaseline,
    ) -> Result<(
        HashMap<RuntimeInvocationKey, DashboardRuntimeBaselineRecord>,
        HashMap<RuntimeInvocationKey, DashboardRuntimeBaselineRecord>,
    )> {
        let mut projection_records = baseline
            .records
            .iter()
            .cloned()
            .map(|record| (record.key.clone(), record))
            .collect::<HashMap<_, _>>();
        let baseline_records = projection_records.clone();
        let runtime = self
            .inner
            .lock()
            .map_err(|_| anyhow!("runtime invocation store lock is poisoned"))?;
        for key in runtime.terminal_tombstones.keys() {
            projection_records.remove(key);
        }
        for key in runtime.projection_tombstones.keys() {
            projection_records.remove(key);
        }
        for (key, entry) in &runtime.records {
            if runtime.terminal_tombstones.contains_key(key) {
                projection_records.remove(key);
                continue;
            }
            if baseline.source_scope == InvocationSourceScope::ProxyOnly
                && entry.record.source != SOURCE_PROXY
            {
                continue;
            }
            let previous = projection_records.get(key);
            match dashboard_projection_record_from_invocation(key.clone(), &entry.record, previous)
            {
                Some(record) => {
                    projection_records.insert(key.clone(), record);
                }
                None => {
                    projection_records.remove(key);
                }
            }
        }
        Ok((projection_records, baseline_records))
    }

    pub(crate) fn install_persistence_baseline_if_generation(
        &self,
        _snapshot: DashboardActivityLiveSnapshot,
        mut baseline: DashboardRuntimeProjectionBaseline,
        snapshot_origin: &'static str,
        expected_generation: u64,
    ) -> Result<Option<DashboardProjectionCapture>> {
        let (projection_records, baseline_records) =
            self.runtime_records_for_persistence_baseline(&baseline)?;
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        if dashboard.dirty_generation < expected_generation {
            tracing::warn!(
                expected_generation,
                actual_generation = dashboard.dirty_generation,
                "rejecting persistence baseline with an impossible generation rollback"
            );
            return Ok(None);
        }
        let snapshot_origin = if dashboard.dirty_generation > expected_generation {
            "reconcile_replayed"
        } else {
            snapshot_origin
        };
        let mut core = empty_dashboard_live_core();
        for record in projection_records.values() {
            update_dashboard_live_core(&mut core, record, true);
        }
        core.accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        let mut snapshot = core.clone();
        let changed = dashboard
            .last_good
            .as_ref()
            .is_none_or(|current| !dashboard_current_snapshot_content_eq(current, &snapshot));
        let snapshot = if changed {
            dashboard.current_revision = dashboard.current_revision.saturating_add(1);
            snapshot.revision = dashboard.current_revision;
            self.dashboard_topology_counters
                .current
                .revision_count
                .fetch_add(1, Ordering::Relaxed);
            dashboard.last_good = Some(snapshot.clone());
            dashboard.last_good_at = Some(Instant::now());
            snapshot
        } else {
            snapshot.revision = dashboard
                .last_good
                .as_ref()
                .expect("unchanged baseline has a last-good snapshot")
                .revision;
            dashboard.last_good = Some(snapshot.clone());
            snapshot
        };
        dashboard.last_snapshot_origin = Some(snapshot_origin);
        dashboard.degraded_reason = None;
        dashboard.reconcile_error = None;
        dashboard.last_reconcile_defer_reason = None;
        dashboard.source_scope = baseline.source_scope;
        dashboard.baseline_records = baseline_records;
        dashboard.projection_records = projection_records;
        dashboard.live_core = Some(core);
        baseline.records.clear();
        dashboard.persistence_baseline = Some(baseline);
        dashboard.memory_ready = false;
        Ok(Some(DashboardProjectionCapture {
            snapshot,
            changed,
            snapshot_origin,
        }))
    }

    pub(crate) fn last_good_capture(
        &self,
        snapshot_origin: &'static str,
    ) -> Option<DashboardProjectionCapture> {
        let mut dashboard = self.dashboard.lock().ok()?;
        let snapshot = dashboard.last_good.clone()?;
        dashboard.last_snapshot_origin = Some(snapshot_origin);
        Some(DashboardProjectionCapture {
            snapshot,
            changed: false,
            snapshot_origin,
        })
    }

    pub(crate) fn mark_degraded(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.degraded_reason = Some(reason);
        }
    }

    pub(crate) fn record_reconcile_failure(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.reconcile_error = Some(reason);
            dashboard.last_reconcile_defer_reason = None;
        }
    }

    pub(crate) fn record_reconcile_deferred(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.last_reconcile_defer_reason = Some(reason);
        }
    }

    pub(crate) fn record_live_path_db_read(&self) {
        self.live_path_db_read_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_build(&self) {
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .current
            .build_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_current_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .current
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_network_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .network
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_terminal_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .terminal
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn dashboard_topology_counters(&self) -> DashboardRuntimeTopologyCounterSnapshot {
        self.dashboard_topology_counters.snapshot()
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard_topology_counters.reset();
    }

    pub(crate) fn set_producer_running(&self, running: bool) {
        self.producer_running.store(running, Ordering::Release);
    }

    pub(crate) fn record_request_pipeline(
        &self,
        snapshot_kind: &str,
        semantic_parse_count: u8,
        whole_body_materialization_count: u8,
        rewrite_buffer_bytes: usize,
        fallback_reason: Option<&str>,
    ) {
        self.request_semantic_parse_count
            .fetch_add(u64::from(semantic_parse_count), Ordering::Relaxed);
        self.request_whole_body_materialization_count.fetch_add(
            u64::from(whole_body_materialization_count),
            Ordering::Relaxed,
        );
        self.request_rewrite_buffer_peak_bytes
            .fetch_max(rewrite_buffer_bytes as u64, Ordering::Relaxed);
        if let Ok(mut last) = self.request_pipeline_last.lock() {
            last.snapshot_kind.clear();
            last.snapshot_kind.push_str(snapshot_kind);
            last.fallback_reason = fallback_reason.map(str::to_string);
        }
    }

    pub(crate) fn request_pipeline_health_snapshot(&self) -> RequestPipelineHealthSnapshot {
        let last = self.request_pipeline_last.lock().ok();
        let parse_window = crate::proxy::request_semantic_cpu_attribution_snapshot();
        RequestPipelineHealthSnapshot {
            mode: request_semantic_pipeline_mode().as_str().to_string(),
            last_snapshot_kind: last
                .as_ref()
                .map(|last| last.snapshot_kind.as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("none")
                .to_string(),
            semantic_parse_count: self.request_semantic_parse_count.load(Ordering::Relaxed),
            whole_body_materialization_count: self
                .request_whole_body_materialization_count
                .load(Ordering::Relaxed),
            rewrite_buffer_peak_bytes: self
                .request_rewrite_buffer_peak_bytes
                .load(Ordering::Relaxed),
            last_fallback_reason: last.and_then(|last| last.fallback_reason.clone()),
            parse_window_count: parse_window.count,
            parse_window_cpu_ms: parse_window.cpu_ms,
            parse_window_wall_ms: parse_window.wall_ms,
            parse_window_bytes: parse_window.bytes,
        }
    }

    pub(crate) fn health_snapshot(
        &self,
        active_subscriber_count: usize,
    ) -> RuntimeProjectionHealthSnapshot {
        let slice_counters = self.dashboard_topology_counters.snapshot();
        let dashboard = self.dashboard.lock().ok();
        let last_good_age_ms = dashboard
            .as_ref()
            .and_then(|state| state.last_good_at)
            .map(|captured_at| captured_at.elapsed().as_millis() as u64);
        let state = match dashboard.as_ref() {
            Some(state) if state.degraded_reason.is_some() || state.reconcile_error.is_some() => {
                "degraded"
            }
            Some(state) if state.memory_ready || state.last_good.is_some() => "healthy",
            _ => "cold",
        };
        RuntimeProjectionHealthSnapshot {
            mode: self.mode.as_str().to_string(),
            state: state.to_string(),
            producer_state: if self.producer_running.load(Ordering::Acquire) {
                "running".to_string()
            } else {
                "idle".to_string()
            },
            active_subscriber_count: active_subscriber_count as u64,
            live_path_db_read_count: self.live_path_db_read_count.load(Ordering::Relaxed),
            build_count: self.build_count.load(Ordering::Relaxed),
            revision: dashboard
                .as_ref()
                .and_then(|state| state.last_good.as_ref())
                .map_or(0, |snapshot| snapshot.revision),
            snapshot_origin: dashboard
                .as_ref()
                .and_then(|state| state.last_snapshot_origin)
                .unwrap_or("none")
                .to_string(),
            last_good_age_ms,
            degraded_reason: dashboard
                .as_ref()
                .and_then(|state| state.degraded_reason.or(state.reconcile_error))
                .map(str::to_string),
            last_defer_reason: dashboard
                .as_ref()
                .and_then(|state| state.last_reconcile_defer_reason)
                .map(str::to_string),
            slice_counters,
        }
    }

    pub(crate) fn apply_network_overlay_to_snapshot(
        &self,
        snapshot: &mut DashboardActivityLiveSnapshot,
    ) {
        if let Ok(dashboard) = self.dashboard.lock()
            && let Some(network) = dashboard.network_last_good.as_ref()
        {
            apply_dashboard_network_slice_to_live_snapshot(snapshot, network);
        }
    }
}

pub(crate) struct DashboardLiveProjection<'a> {
    hub: &'a RuntimeProjectionHub,
}

impl DashboardLiveProjection<'_> {
    pub(crate) fn snapshot(&self) -> Result<DashboardActivityLiveSnapshot> {
        self.hub
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))
            .map(|dashboard| {
                dashboard
                    .live_core
                    .clone()
                    .unwrap_or_else(empty_dashboard_live_core)
            })
    }
}
