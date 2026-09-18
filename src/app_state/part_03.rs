fn dashboard_current_snapshot_content_eq(
    left: &DashboardActivityLiveSnapshot,
    right: &DashboardActivityLiveSnapshot,
) -> bool {
    left.in_progress_invocation_count == right.in_progress_invocation_count
        && left.in_progress_phase_counts == right.in_progress_phase_counts
        && left.retry_invocation_count == right.retry_invocation_count
        && left.in_progress_wait_sum_ms == right.in_progress_wait_sum_ms
        && left.in_progress_wait_sample_count == right.in_progress_wait_sample_count
        && left.accounts.len() == right.accounts.len()
        && left.accounts.iter().all(|left| {
            right
                .accounts
                .iter()
                .find(|right| right.account_key == left.account_key)
                .is_some_and(|right| {
                    left.upstream_account_id == right.upstream_account_id
                        && left.upstream_account_name == right.upstream_account_name
                        && left.in_progress_invocation_count == right.in_progress_invocation_count
                        && left.in_progress_phase_counts == right.in_progress_phase_counts
                        && left.retry_invocation_count == right.retry_invocation_count
                        && left.in_progress_wait_sum_ms == right.in_progress_wait_sum_ms
                        && left.in_progress_wait_sample_count == right.in_progress_wait_sample_count
                })
        })
}

fn dashboard_legacy_snapshot_content_eq(
    left: &DashboardActivityLiveSnapshot,
    right: &DashboardActivityLiveSnapshot,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.revision = 0;
    right.revision = 0;
    left.generated_at.clear();
    right.generated_at.clear();
    left == right
}

fn dashboard_network_slice_content_eq(
    left: &DashboardNetworkProjectionSlice,
    right: &DashboardNetworkProjectionSlice,
) -> bool {
    left.network_live_bucket == right.network_live_bucket
        && left.network_realtime_rate == right.network_realtime_rate
        && left.recent == right.recent
        && left.current_snapshot == right.current_snapshot
        && left.current_snapshot_by_account == right.current_snapshot_by_account
        && left.accounts.len() == right.accounts.len()
        && left.accounts.iter().all(|left| {
            right
                .accounts
                .iter()
                .find(|right| right.account_key == left.account_key)
                .is_some_and(|right| {
                    left.upload_bytes_per_second == right.upload_bytes_per_second
                        && left.download_bytes_per_second == right.download_bytes_per_second
                        && left.network_live_bucket == right.network_live_bucket
                })
        })
}

fn apply_dashboard_network_slice_to_live_snapshot(
    current: &mut DashboardActivityLiveSnapshot,
    network: &DashboardNetworkProjectionSlice,
) {
    current.network_live_bucket = network.network_live_bucket.clone();
    current.network_realtime_rate = network.network_realtime_rate.clone();
    for account in &mut current.accounts {
        let Some(network_account) = network
            .accounts
            .iter()
            .find(|candidate| candidate.account_key == account.account_key)
        else {
            account.upload_bytes_per_second = 0.0;
            account.download_bytes_per_second = 0.0;
            account.network_live_bucket = None;
            continue;
        };
        account.upload_bytes_per_second = network_account.upload_bytes_per_second;
        account.download_bytes_per_second = network_account.download_bytes_per_second;
        account.network_live_bucket = network_account.network_live_bucket.clone();
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProxyRuntimeInvocationStoreInner {
    pub(crate) records: HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    pub(crate) terminal_tombstones: HashMap<RuntimeInvocationKey, Instant>,
    projection_tombstones: HashMap<RuntimeInvocationKey, Instant>,
}

pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE: Duration =
    Duration::from_secs(6 * 60 * 60);
pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS: usize = 10_000;
pub(crate) const PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS: usize = 50_000;

impl RuntimeProjectionHub {
    pub(crate) fn runtime_record_count(&self) -> usize {
        self.inner
            .lock()
            .map(|guard| guard.records.len())
            .unwrap_or_default()
    }

    pub(crate) fn memory_estimate(&self) -> MemoryComponentEstimate {
        let Ok(guard) = self.inner.lock() else {
            return MemoryComponentEstimate::default();
        };
        let record_bytes = guard
            .records
            .values()
            .map(|entry| entry.record.estimated_memory_bytes())
            .sum::<usize>();
        let key_bytes = guard
            .records
            .keys()
            .chain(guard.terminal_tombstones.keys())
            .chain(guard.projection_tombstones.keys())
            .map(|key| key.invoke_id.capacity() + key.occurred_at.capacity())
            .sum::<usize>();
        MemoryComponentEstimate {
            entries: guard
                .records
                .len()
                .saturating_add(guard.terminal_tombstones.len())
                .saturating_add(guard.projection_tombstones.len()),
            bytes: record_bytes.saturating_add(key_bytes).saturating_add(
                (guard.records.capacity()
                    + guard.terminal_tombstones.capacity()
                    + guard.projection_tombstones.capacity())
                .saturating_mul(std::mem::size_of::<usize>() * 2),
            ),
            detail_items: guard.records.len(),
        }
    }

    pub(crate) fn upsert(&self, record: ApiInvocation) -> RuntimeInvocationStoreUpsertOutcome {
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let Ok(mut guard) = self.inner.lock() else {
            return RuntimeInvocationStoreUpsertOutcome {
                running_count: 0,
                pruned_count: 0,
                skipped_terminal: false,
            };
        };
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let terminal_overlay_exists = guard
            .records
            .get(&key)
            .is_some_and(|entry| runtime_store_record_is_terminal(&entry.record));
        if guard.terminal_tombstones.contains_key(&key) || terminal_overlay_exists {
            let outcome = RuntimeInvocationStoreUpsertOutcome {
                running_count: guard.records.len(),
                pruned_count,
                skipped_terminal: true,
            };
            drop(guard);
            if pruned_count > 0 {
                self.rebuild_dashboard_runtime_records("runtime_prune");
            }
            return outcome;
        }
        guard.projection_tombstones.remove(&key);
        guard.records.insert(
            key.clone(),
            RuntimeInvocationEntry {
                record,
                updated_at: now,
            },
        );
        let pruned_count =
            pruned_count + prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let outcome = RuntimeInvocationStoreUpsertOutcome {
            running_count: guard.records.len(),
            pruned_count,
            skipped_terminal: false,
        };
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.sync_dashboard_runtime_key(&key, "runtime_upsert");
        }
        outcome
    }

    pub(crate) fn upsert_terminal(
        &self,
        record: ApiInvocation,
    ) -> RuntimeInvocationStoreRemoveOutcome {
        let Ok(mut guard) = self.inner.lock() else {
            return RuntimeInvocationStoreRemoveOutcome {
                removed: false,
                already_terminal: false,
            };
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let already_terminal = guard.terminal_tombstones.contains_key(&key);
        if already_terminal {
            let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
            drop(guard);
            if pruned_count > 0 {
                self.rebuild_dashboard_runtime_records("runtime_prune");
            }
            return RuntimeInvocationStoreRemoveOutcome {
                removed: false,
                already_terminal: true,
            };
        }
        let removed = guard
            .records
            .insert(
                key.clone(),
                RuntimeInvocationEntry {
                    record,
                    updated_at: now,
                },
            )
            .is_some();
        guard.terminal_tombstones.insert(key.clone(), now);
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let outcome = RuntimeInvocationStoreRemoveOutcome {
            removed,
            already_terminal: false,
        };
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.update_dashboard_runtime_record(key, None, "terminal_delta");
        }
        self.mark_dashboard_terminal_dirty();
        outcome
    }

    pub(crate) fn clear_terminal_tombstone(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let removed = guard
            .terminal_tombstones
            .remove(&RuntimeInvocationKey::new(invoke_id, occurred_at))
            .is_some();
        drop(guard);
        if removed {
            self.sync_dashboard_runtime_key(
                &RuntimeInvocationKey::new(invoke_id, occurred_at),
                "terminal_rollback",
            );
            self.mark_dashboard_terminal_dirty();
        }
        removed
    }

    pub(crate) fn contains_terminal(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let contains_terminal = guard.terminal_tombstones.contains_key(&key)
            || guard
                .records
                .get(&key)
                .is_some_and(|entry| runtime_store_record_is_terminal(&entry.record));
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        contains_terminal
    }

    pub(crate) fn remove_non_terminal(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return None;
        };
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let should_remove = guard
            .records
            .get(&key)
            .is_some_and(|entry| !runtime_store_record_is_terminal(&entry.record));
        let removed = if should_remove {
            guard.records.remove(&key).map(|entry| entry.record)
        } else {
            None
        };
        if removed.is_some() {
            guard
                .projection_tombstones
                .insert(key.clone(), Instant::now());
        }
        let pruned_count = if removed.is_some() {
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now())
        } else {
            0
        };
        drop(guard);
        if removed.is_some() {
            self.update_dashboard_runtime_record(key, None, "runtime_remove");
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed
    }

    pub(crate) fn remove_non_terminal_by_invoke_id(&self, invoke_id: &str) -> Vec<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return Vec::new();
        };
        let keys = guard
            .records
            .iter()
            .filter(|(key, entry)| {
                key.invoke_id == invoke_id && !runtime_store_record_is_terminal(&entry.record)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let removed = keys
            .iter()
            .filter_map(|key| {
                guard
                    .records
                    .remove(key)
                    .map(|entry| (key.clone(), entry.record))
            })
            .collect::<Vec<_>>();
        if !removed.is_empty() {
            let now = Instant::now();
            for (key, _) in &removed {
                guard.projection_tombstones.insert(key.clone(), now);
            }
        }
        let pruned_count = if !removed.is_empty() {
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now())
        } else {
            0
        };
        drop(guard);
        if !removed.is_empty() {
            for (key, _) in &removed {
                self.update_dashboard_runtime_record(key.clone(), None, "runtime_remove");
            }
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed.into_iter().map(|(_, record)| record).collect()
    }

    pub(crate) fn remove_persisted_terminal_overlay(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let removed = guard.records.remove(&key).is_some();
        guard.terminal_tombstones.insert(key.clone(), now);
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.update_dashboard_runtime_record(key, None, "terminal_persisted");
        }
        self.mark_dashboard_terminal_dirty();
        removed
    }

    pub(crate) fn snapshot(&self) -> Vec<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return Vec::new();
        };
        let pruned_count =
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now());
        let snapshot = guard
            .records
            .values()
            .map(|entry| entry.record.clone())
            .collect();
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        snapshot
    }

    pub(crate) fn record_by_identity(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<ApiInvocation> {
        #[cfg(test)]
        self.full_record_clone_count.fetch_add(1, Ordering::Relaxed);
        let guard = self.inner.lock().ok()?;
        guard
            .records
            .get(&RuntimeInvocationKey::new(invoke_id, occurred_at))
            .map(|entry| entry.record.clone())
    }

    pub(crate) fn prompt_cache_projection_by_identity(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<PromptCacheRuntimeProjection> {
        let guard = self.inner.lock().ok()?;
        guard
            .records
            .get(&RuntimeInvocationKey::new(invoke_id, occurred_at))
            .and_then(|entry| PromptCacheRuntimeProjection::from_record(&entry.record))
    }

    #[cfg(test)]
    pub(crate) fn reset_full_record_clone_count(&self) {
        self.full_record_clone_count.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn full_record_clone_count(&self) -> u64 {
        self.full_record_clone_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn backdate_for_test(&self, invoke_id: &str, occurred_at: &str, age: Duration) {
        let Some(updated_at) = Instant::now().checked_sub(age) else {
            return;
        };
        if let Ok(mut guard) = self.inner.lock()
            && let Some(entry) = guard
                .records
                .get_mut(&RuntimeInvocationKey::new(invoke_id, occurred_at))
        {
            entry.updated_at = updated_at;
        }
    }

    pub(crate) fn shutdown_summary(&self) -> RuntimeInvocationStoreShutdownSummary {
        let Ok(guard) = self.inner.lock() else {
            return RuntimeInvocationStoreShutdownSummary {
                running_count: 0,
                oldest_age_ms: None,
            };
        };
        let now = Instant::now();
        RuntimeInvocationStoreShutdownSummary {
            running_count: guard.records.len(),
            oldest_age_ms: guard
                .records
                .values()
                .map(|entry| now.duration_since(entry.updated_at).as_millis() as u64)
                .max(),
        }
    }
}

impl ApiInvocation {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        fn option_string_bytes(value: &Option<String>) -> usize {
            value.as_ref().map_or(0, String::capacity)
        }

        self.invoke_id.capacity()
            + self.occurred_at.capacity()
            + self.source.capacity()
            + self.detail_level.capacity()
            + option_string_bytes(&self.proxy_display_name)
            + option_string_bytes(&self.model)
            + option_string_bytes(&self.request_model)
            + option_string_bytes(&self.response_model)
            + option_string_bytes(&self.reasoning_effort)
            + option_string_bytes(&self.status)
            + option_string_bytes(&self.live_phase)
            + option_string_bytes(&self.error_message)
            + option_string_bytes(&self.failure_kind)
            + option_string_bytes(&self.blocked_binding_json)
            + option_string_bytes(&self.stream_terminal_event)
            + option_string_bytes(&self.upstream_error_code)
            + option_string_bytes(&self.upstream_error_message)
            + option_string_bytes(&self.downstream_error_message)
            + option_string_bytes(&self.upstream_request_id)
            + option_string_bytes(&self.failure_class)
            + option_string_bytes(&self.endpoint)
            + option_string_bytes(&self.compaction_request_kind)
            + option_string_bytes(&self.compaction_response_kind)
            + option_string_bytes(&self.image_intent)
            + option_string_bytes(&self.requester_ip)
            + option_string_bytes(&self.prompt_cache_key)
            + option_string_bytes(&self.sticky_key)
            + option_string_bytes(&self.route_mode)
            + option_string_bytes(&self.upstream_account_name)
            + option_string_bytes(&self.response_content_encoding)
            + option_string_bytes(&self.request_compression_algorithm)
            + option_string_bytes(&self.transport)
            + option_string_bytes(&self.pool_attempt_terminal_reason)
            + option_string_bytes(&self.requested_service_tier)
            + option_string_bytes(&self.service_tier)
            + option_string_bytes(&self.billing_service_tier)
            + option_string_bytes(&self.price_version)
            + option_string_bytes(&self.request_raw_path)
            + option_string_bytes(&self.request_raw_truncated_reason)
            + option_string_bytes(&self.response_raw_path)
            + option_string_bytes(&self.response_raw_truncated_reason)
            + option_string_bytes(&self.detail_pruned_at)
            + option_string_bytes(&self.detail_prune_reason)
            + self.created_at.capacity()
            + std::mem::size_of::<Self>()
    }
}

pub(crate) fn runtime_store_record_is_terminal(record: &ApiInvocation) -> bool {
    !matches!(
        record
            .status
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

pub(crate) fn prune_bounded_runtime_invocation_store_locked(
    store: &mut ProxyRuntimeInvocationStoreInner,
    now: Instant,
) -> usize {
    let pruned_keys = prune_bounded_runtime_invocations_locked(
        &mut store.records,
        now,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS,
    );
    let pruned_count = pruned_keys.len();
    for key in pruned_keys {
        store.projection_tombstones.insert(key, now);
    }
    pruned_count
        + prune_bounded_runtime_tombstones_locked(
            &mut store.terminal_tombstones,
            now,
            PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
            PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS,
        )
        + prune_bounded_runtime_tombstones_locked(
            &mut store.projection_tombstones,
            now,
            PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
            PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS,
        )
}

pub(crate) fn prune_bounded_runtime_invocations_locked(
    records: &mut HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    now: Instant,
    max_age: Duration,
    max_records: usize,
) -> Vec<RuntimeInvocationKey> {
    let mut pruned_keys = Vec::new();
    records.retain(|key, entry| {
        let retain = now.duration_since(entry.updated_at) <= max_age;
        if !retain {
            pruned_keys.push(key.clone());
        }
        retain
    });
    if records.len() > max_records {
        let mut ranked_keys = records
            .iter()
            .map(|(key, entry)| (key.clone(), entry.updated_at))
            .collect::<Vec<_>>();
        ranked_keys.sort_by_key(|(_, updated_at)| *updated_at);
        let excess = records.len().saturating_sub(max_records);
        for (key, _) in ranked_keys.into_iter().take(excess) {
            records.remove(&key);
            pruned_keys.push(key);
        }
    }
    pruned_keys
}

pub(crate) fn prune_bounded_runtime_tombstones_locked(
    tombstones: &mut HashMap<RuntimeInvocationKey, Instant>,
    now: Instant,
    max_age: Duration,
    max_records: usize,
) -> usize {
    let before = tombstones.len();
    tombstones.retain(|_, terminal_at| now.duration_since(*terminal_at) <= max_age);
    if tombstones.len() > max_records {
        let mut ranked_keys = tombstones
            .iter()
            .map(|(key, terminal_at)| (key.clone(), *terminal_at))
            .collect::<Vec<_>>();
        ranked_keys.sort_by_key(|(_, terminal_at)| *terminal_at);
        let excess = tombstones.len().saturating_sub(max_records);
        for (key, _) in ranked_keys.into_iter().take(excess) {
            tombstones.remove(&key);
        }
    }
    before.saturating_sub(tombstones.len())
}

#[derive(Debug)]
pub(crate) struct AppState {
    pub(crate) config: AppConfig,
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) process_started_at_utc: DateTime<Utc>,
    pub(crate) sqlite_batch_writer: Arc<SqliteBatchWriter>,
    pub(crate) pool_account_selection_runtime: Arc<PoolAccountSelectionRuntime>,
    pub(crate) proxy_runtime_invocations: Arc<RuntimeProjectionHub>,
    pub(crate) dashboard_network_speed_cache: Arc<DashboardNetworkSpeedCache>,
    pub(crate) oauth_installation_seed: [u8; 32],
    pub(crate) hourly_rollup_sync_lock: Arc<Mutex<()>>,
    pub(crate) http_clients: HttpClients,
    pub(crate) broadcaster: broadcast::Sender<BroadcastPayload>,
    pub(crate) broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    pub(crate) subscription_hub: Arc<SubscriptionHub>,
    pub(crate) proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    pub(crate) proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    pub(crate) proxy_summary_quota_broadcast_handle: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub(crate) dashboard_activity_live_broadcast_seq: Arc<AtomicU64>,
    pub(crate) dashboard_activity_live_broadcast_running: Arc<AtomicBool>,
    pub(crate) startup_ready: Arc<AtomicBool>,
    pub(crate) shutdown: CancellationToken,
    pub(crate) semaphore: Arc<Semaphore>,
    pub(crate) proxy_request_in_flight: Arc<AtomicUsize>,
    pub(crate) proxy_raw_async_semaphore: Arc<Semaphore>,
    pub(crate) proxy_model_settings: Arc<RwLock<ProxyModelSettings>>,
    pub(crate) proxy_model_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) forward_proxy: Arc<Mutex<ForwardProxyManager>>,
    pub(crate) xray_supervisor: Arc<Mutex<XraySupervisor>>,
    pub(crate) forward_proxy_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) forward_proxy_subscription_refresh_lock: Arc<Mutex<()>>,
    pub(crate) pricing_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) pricing_catalog: Arc<RwLock<PricingCatalog>>,
    pub(crate) prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) dashboard_activity_snapshot_cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    pub(crate) terminal_projection_hub: Arc<TerminalProjectionHub>,
    pub(crate) long_term_projection_runtime: Arc<Mutex<LongTermProjectionRuntime>>,
    pub(crate) memory_diagnostics: Arc<MemoryDiagnosticsRuntime>,
    pub(crate) maintenance_stats_cache: Arc<Mutex<StatsMaintenanceCacheState>>,
    pub(crate) system_status_cache: Arc<Mutex<SystemStatusCacheState>>,
    pub(crate) pool_routing_reservations:
        Arc<std::sync::Mutex<HashMap<String, PoolRoutingReservation>>>,
    pub(crate) pool_routing_availability: PoolRoutingAvailabilitySignal,
    pub(crate) pool_routing_runtime_cache: Arc<Mutex<Option<PoolRoutingRuntimeCache>>>,
    #[cfg(test)]
    pub(crate) pool_routing_test_data_version_connection:
        Arc<Mutex<Option<sqlx::pool::PoolConnection<Sqlite>>>>,
    pub(crate) pool_model_routing_cache_write_lock: Arc<Mutex<()>>,
    pub(crate) pool_live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    pub(crate) pool_group_429_retry_delay_override: Option<Duration>,
    #[cfg(test)]
    pub(crate) fallback_proxy_429_retry_delay_override: Option<Duration>,
    pub(crate) pool_no_available_wait: PoolNoAvailableWaitSettings,
    pub(crate) upstream_accounts: Arc<UpstreamAccountsRuntime>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingAvailabilitySignal {
    sender: tokio::sync::watch::Sender<u64>,
}

impl Default for PoolRoutingAvailabilitySignal {
    fn default() -> Self {
        let (sender, _receiver) = tokio::sync::watch::channel(0);
        Self { sender }
    }
}

impl PoolRoutingAvailabilitySignal {
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.sender.subscribe()
    }

    pub(crate) fn publish(&self) {
        self.sender.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PricingCatalog {
    pub(crate) version: String,
    pub(crate) models: HashMap<String, ModelPricing>,
}

#[cfg(test)]
mod pool_routing_prompt_route_cache_tests {
    use super::*;

    fn key(index: usize) -> PoolRoutingPromptRouteCacheKey {
        PoolRoutingPromptRouteCacheKey {
            prompt_cache_key: format!("prompt-{index}"),
            request_contains_encrypted_content: false,
            encrypted_session_owner_routing_enabled: false,
        }
    }

    #[test]
    fn routing_hot_cache_retains_negative_entries_and_evicts_lru() {
        let mut cache = PoolRoutingPromptRouteCache::default();
        for index in 0..POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY {
            cache.insert(key(index), None);
        }
        assert!(cache.get(&key(0)).is_some_and(|value| value.is_none()));
        cache.insert(key(POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY), None);

        assert!(cache.get(&key(0)).is_some_and(|value| value.is_none()));
        assert!(cache.get(&key(1)).is_none());
        assert!(
            cache
                .get(&key(POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY))
                .is_some_and(|value| value.is_none())
        );
    }

    #[test]
    fn routing_hot_cache_invalidates_all_variants_for_prompt_key() {
        let mut cache = PoolRoutingPromptRouteCache::default();
        let mut encrypted = key(1);
        encrypted.request_contains_encrypted_content = true;
        cache.insert(key(1), None);
        cache.insert(encrypted, None);
        cache.insert(key(2), None);

        cache.invalidate_prompt_cache_key("prompt-1");

        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some_and(|value| value.is_none()));
    }

    #[test]
    fn routing_hot_cache_invalidates_every_sticky_model_variant() {
        let mut cache = PoolRoutingStickyRouteCache::default();
        let base = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-1".to_string(),
            model_key: None,
        };
        let model = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-1".to_string(),
            model_key: Some("gpt-5".to_string()),
        };
        let other = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-2".to_string(),
            model_key: None,
        };
        let negative = PoolRoutingStickyRouteCacheValue {
            route: None,
            affinity_generation: 7,
        };
        cache.insert(base.clone(), negative.clone());
        cache.insert(model.clone(), negative);
        cache.insert(
            other.clone(),
            PoolRoutingStickyRouteCacheValue {
                route: None,
                affinity_generation: 9,
            },
        );

        cache.invalidate_sticky_key("sticky-1");

        assert!(cache.get(&base).is_none());
        assert!(cache.get(&model).is_none());
        assert_eq!(
            cache
                .get(&other)
                .expect("other key remains cached")
                .affinity_generation,
            9
        );
    }
}

impl Default for PricingCatalog {
    fn default() -> Self {
        Self {
            version: "unavailable".to_string(),
            models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ModelPricing {
    pub(crate) input_per_1m: f64,
    pub(crate) output_per_1m: f64,
    #[serde(default)]
    pub(crate) cache_input_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_read_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_write_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    pub(crate) source: String,
}

impl ModelPricing {
    pub(crate) fn effective_cache_read_per_1m(&self) -> Option<f64> {
        self.cache_read_per_1m.or(self.cache_input_per_1m)
    }

    pub(crate) fn has_explicit_cache_pricing_split(&self) -> bool {
        self.cache_read_per_1m.is_some() && self.cache_write_per_1m.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingEntry {
    pub(crate) model: String,
    pub(crate) input_per_1m: f64,
    pub(crate) output_per_1m: f64,
    #[serde(default)]
    pub(crate) cache_input_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_read_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_write_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    pub(crate) source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingSettingsUpdateRequest {
    pub(crate) catalog_version: String,
    #[serde(default)]
    pub(crate) entries: Vec<PricingEntry>,
}
