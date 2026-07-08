use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingRuntimeCache {
    pub(crate) api_key: Option<String>,
    pub(crate) timeouts: PoolRoutingTimeoutSettingsResolved,
}

#[derive(Debug, Default)]
pub(crate) struct PoolAccountSelectionRuntime {
    pub(crate) selected_at: std::sync::Mutex<HashMap<i64, String>>,
}

impl PoolAccountSelectionRuntime {
    pub(crate) fn record_selected(&self, account_id: i64, selected_at: String) {
        if let Ok(mut guard) = self.selected_at.lock() {
            match guard.get(&account_id) {
                Some(existing) if existing >= &selected_at => {}
                _ => {
                    guard.insert(account_id, selected_at);
                }
            }
        }
    }

    pub(crate) fn latest_selected_at(
        &self,
        account_id: i64,
        persisted: Option<&str>,
    ) -> Option<String> {
        let runtime = self
            .selected_at
            .lock()
            .ok()
            .and_then(|guard| guard.get(&account_id).cloned());
        match (runtime, persisted) {
            (Some(runtime), Some(persisted)) if runtime.as_str() < persisted => {
                Some(persisted.to_string())
            }
            (Some(runtime), _) => Some(runtime),
            (None, Some(persisted)) => Some(persisted.to_string()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct RuntimeInvocationKey {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl RuntimeInvocationKey {
    pub(crate) fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
        Self {
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeInvocationEntry {
    pub(crate) record: ApiInvocation,
    pub(crate) updated_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreUpsertOutcome {
    pub(crate) running_count: usize,
    pub(crate) pruned_count: usize,
    pub(crate) skipped_terminal: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreShutdownSummary {
    pub(crate) running_count: usize,
    pub(crate) oldest_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreRemoveOutcome {
    pub(crate) removed: bool,
    pub(crate) already_terminal: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ProxyRuntimeInvocationStore {
    pub(crate) inner: std::sync::Mutex<ProxyRuntimeInvocationStoreInner>,
}

#[derive(Debug, Default)]
pub(crate) struct ProxyRuntimeInvocationStoreInner {
    pub(crate) records: HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    pub(crate) terminal_tombstones: HashMap<RuntimeInvocationKey, Instant>,
}

pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE: Duration =
    Duration::from_secs(6 * 60 * 60);
pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS: usize = 10_000;
pub(crate) const PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS: usize = 50_000;

impl ProxyRuntimeInvocationStore {
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
            return RuntimeInvocationStoreUpsertOutcome {
                running_count: guard.records.len(),
                pruned_count,
                skipped_terminal: true,
            };
        }
        guard.records.insert(
            key,
            RuntimeInvocationEntry {
                record,
                updated_at: now,
            },
        );
        let pruned_count =
            pruned_count + prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        RuntimeInvocationStoreUpsertOutcome {
            running_count: guard.records.len(),
            pruned_count,
            skipped_terminal: false,
        }
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
            let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
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
        guard.terminal_tombstones.insert(key, now);
        let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        RuntimeInvocationStoreRemoveOutcome {
            removed,
            already_terminal: false,
        }
    }

    pub(crate) fn clear_terminal_tombstone(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        guard
            .terminal_tombstones
            .remove(&RuntimeInvocationKey::new(invoke_id, occurred_at))
            .is_some()
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
        let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
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
            let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now());
        }
        removed
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
        guard.terminal_tombstones.insert(key, now);
        let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        removed
    }

    pub(crate) fn snapshot(&self) -> Vec<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return Vec::new();
        };
        let _ = prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now());
        guard
            .records
            .values()
            .map(|entry| entry.record.clone())
            .collect()
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
    prune_bounded_runtime_invocations_locked(
        &mut store.records,
        now,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS,
    ) + prune_bounded_runtime_tombstones_locked(
        &mut store.terminal_tombstones,
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
) -> usize {
    let before = records.len();
    records.retain(|_, entry| now.duration_since(entry.updated_at) <= max_age);
    if records.len() > max_records {
        let mut ranked_keys = records
            .iter()
            .map(|(key, entry)| (key.clone(), entry.updated_at))
            .collect::<Vec<_>>();
        ranked_keys.sort_by_key(|(_, updated_at)| *updated_at);
        let excess = records.len().saturating_sub(max_records);
        for (key, _) in ranked_keys.into_iter().take(excess) {
            records.remove(&key);
        }
    }
    before.saturating_sub(records.len())
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
    pub(crate) sqlite_batch_writer: Arc<SqliteBatchWriter>,
    pub(crate) pool_account_selection_runtime: Arc<PoolAccountSelectionRuntime>,
    pub(crate) proxy_runtime_invocations: Arc<ProxyRuntimeInvocationStore>,
    pub(crate) oauth_installation_seed: [u8; 32],
    pub(crate) hourly_rollup_sync_lock: Arc<Mutex<()>>,
    pub(crate) http_clients: HttpClients,
    pub(crate) broadcaster: broadcast::Sender<BroadcastPayload>,
    pub(crate) broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    pub(crate) proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    pub(crate) proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    pub(crate) proxy_summary_quota_broadcast_handle: Arc<Mutex<Vec<JoinHandle<()>>>>,
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
    pub(crate) maintenance_stats_cache: Arc<Mutex<StatsMaintenanceCacheState>>,
    pub(crate) system_status_cache: Arc<Mutex<SystemStatusCacheState>>,
    pub(crate) pool_routing_reservations:
        Arc<std::sync::Mutex<HashMap<String, PoolRoutingReservation>>>,
    pub(crate) pool_routing_runtime_cache: Arc<Mutex<Option<PoolRoutingRuntimeCache>>>,
    pub(crate) pool_live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    pub(crate) pool_group_429_retry_delay_override: Option<Duration>,
    pub(crate) pool_no_available_wait: PoolNoAvailableWaitSettings,
    pub(crate) upstream_accounts: Arc<UpstreamAccountsRuntime>,
}

#[derive(Debug, Clone)]
pub(crate) struct PricingCatalog {
    pub(crate) version: String,
    pub(crate) models: HashMap<String, ModelPricing>,
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
    pub(crate) reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    pub(crate) source: String,
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

impl PricingSettingsUpdateRequest {
    pub(crate) fn normalized(self) -> Result<PricingCatalog, (StatusCode, String)> {
        let version = normalize_pricing_catalog_version(self.catalog_version).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "catalogVersion must be a non-empty string".to_string(),
            )
        })?;
        let mut models = HashMap::new();
        for entry in self.entries {
            let model_id = entry.model.trim();
            if model_id.is_empty() || model_id.len() > 128 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid model id: {}", entry.model),
                ));
            }
            if !entry.input_per_1m.is_finite()
                || !entry.output_per_1m.is_finite()
                || entry.input_per_1m < 0.0
                || entry.output_per_1m < 0.0
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid pricing values for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_input_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheInputPer1m for model: {model_id}"),
                ));
            }
            if let Some(reasoning) = entry.reasoning_per_1m
                && (!reasoning.is_finite() || reasoning < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid reasoningPer1m for model: {model_id}"),
                ));
            }

            let inserted = models.insert(
                model_id.to_string(),
                ModelPricing {
                    input_per_1m: entry.input_per_1m,
                    output_per_1m: entry.output_per_1m,
                    cache_input_per_1m: entry.cache_input_per_1m,
                    reasoning_per_1m: entry.reasoning_per_1m,
                    source: normalize_pricing_source(entry.source),
                },
            );
            if inserted.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("duplicate model id: {model_id}"),
                ));
            }
        }
        Ok(PricingCatalog { version, models })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingSettingsResponse {
    pub(crate) catalog_version: String,
    pub(crate) entries: Vec<PricingEntry>,
}

impl PricingSettingsResponse {
    pub(crate) fn from_catalog(catalog: &PricingCatalog) -> Self {
        let mut entries = catalog
            .models
            .iter()
            .map(|(model, pricing)| PricingEntry {
                model: model.clone(),
                input_per_1m: pricing.input_per_1m,
                output_per_1m: pricing.output_per_1m,
                cache_input_per_1m: pricing.cache_input_per_1m,
                reasoning_per_1m: pricing.reasoning_per_1m,
                source: pricing.source.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.model.cmp(&b.model));
        Self {
            catalog_version: catalog.version.clone(),
            entries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettings {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) enabled_preset_models: Vec<String>,
}

pub(crate) fn normalize_proxy_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

pub(crate) fn decode_proxy_upstream_429_max_retries(raw: Option<i64>) -> u8 {
    raw.and_then(|value| u8::try_from(value).ok())
        .map(normalize_proxy_upstream_429_max_retries)
        .unwrap_or(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES)
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
            upstream_websocket_default_enabled:
                DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
            request_body_logging_enabled: true,
            response_body_logging_enabled: true,
            encrypted_session_owner_routing_enabled:
                DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
            enabled_preset_models: default_enabled_preset_models(),
        }
    }
}

impl ProxyModelSettings {
    pub(crate) fn normalized(self) -> Self {
        let merge_upstream_enabled = if self.hijack_enabled {
            self.merge_upstream_enabled
        } else {
            false
        };
        Self {
            hijack_enabled: self.hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries: normalize_proxy_upstream_429_max_retries(
                self.upstream_429_max_retries,
            ),
            websocket_enabled: self.websocket_enabled,
            upstream_websocket_default_enabled: self.upstream_websocket_default_enabled,
            request_body_logging_enabled: self.request_body_logging_enabled,
            response_body_logging_enabled: self.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: self.encrypted_session_owner_routing_enabled,
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyModelSettingsRow {
    pub(crate) hijack_enabled: i64,
    pub(crate) merge_upstream_enabled: i64,
    pub(crate) upstream_429_max_retries: Option<i64>,
    pub(crate) openai_proxy_websocket_enabled: Option<i64>,
    pub(crate) openai_proxy_upstream_websocket_default_enabled: Option<i64>,
    pub(crate) request_body_logging_enabled: Option<i64>,
    pub(crate) response_body_logging_enabled: Option<i64>,
    pub(crate) encrypted_session_owner_routing_enabled: Option<i64>,
    pub(crate) enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
            upstream_429_max_retries: decode_proxy_upstream_429_max_retries(
                value.upstream_429_max_retries,
            ),
            websocket_enabled: value.openai_proxy_websocket_enabled.unwrap_or(0) != 0,
            upstream_websocket_default_enabled: value
                .openai_proxy_upstream_websocket_default_enabled
                .unwrap_or(0)
                != 0,
            request_body_logging_enabled: value.request_body_logging_enabled.unwrap_or(1) != 0,
            response_body_logging_enabled: value.response_body_logging_enabled.unwrap_or(1) != 0,
            encrypted_session_owner_routing_enabled: value
                .encrypted_session_owner_routing_enabled
                .unwrap_or(DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED as i64)
                != 0,
            enabled_preset_models: decode_enabled_preset_models(
                value.enabled_preset_models_json.as_deref(),
            ),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsUpdateRequest {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    #[serde(default)]
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    #[serde(default)]
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default)]
    pub(crate) websocket_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_websocket_default_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) request_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) response_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) encrypted_session_owner_routing_enabled: Option<bool>,
    #[serde(default = "default_enabled_preset_models")]
    pub(crate) enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsResponse {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) default_hijack_enabled: bool,
    pub(crate) models: Vec<String>,
    pub(crate) enabled_models: Vec<String>,
}

impl ProxyModelSettingsResponse {
    pub(crate) fn from_settings(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
            fast_mode_rewrite_mode: "disabled".to_string(),
            upstream_429_max_retries: value.upstream_429_max_retries,
            websocket_enabled: value.websocket_enabled,
            upstream_websocket_default_enabled: value.upstream_websocket_default_enabled,
            request_body_logging_enabled: value.request_body_logging_enabled,
            response_body_logging_enabled: value.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: value.encrypted_session_owner_routing_enabled,
            default_hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            models: PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            enabled_models: value.enabled_preset_models,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsResponse {
    pub(crate) proxy: ProxyModelSettingsResponse,
    pub(crate) forward_proxy: ForwardProxySettingsResponse,
    pub(crate) pricing: PricingSettingsResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskKind {
    SchedulerPoll,
    RetentionArchive,
    StartupBackfill,
    ForwardProxySubscriptionRefresh,
}

impl SystemTaskKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SchedulerPoll => "scheduler_poll",
            Self::RetentionArchive => "retention_archive",
            Self::StartupBackfill => "startup_backfill",
            Self::ForwardProxySubscriptionRefresh => "forward_proxy_subscription_refresh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskStatus {
    Running,
    Success,
    Failed,
    Skipped,
}

impl SystemTaskStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SystemStatusCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) response: SystemStatusResponse,
}

#[derive(Debug, Default)]
pub(crate) struct SystemStatusCacheState {
    pub(crate) latest: Option<SystemStatusCacheEntry>,
}

pub(crate) fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(String::as_str).collect();
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled_set.contains(**model))
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn decode_enabled_preset_models(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized)
            .map(normalize_enabled_preset_models)
            .unwrap_or_else(|_| default_enabled_preset_models()),
        None => default_enabled_preset_models(),
    }
}

pub(crate) fn default_pricing_source_custom() -> String {
    "custom".to_string()
}

pub(crate) fn normalize_pricing_catalog_version(raw: String) -> Option<String> {
    let normalized = raw.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn normalize_pricing_source(raw: String) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_pricing_source_custom()
    } else {
        normalized
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HttpClients {
    pub(crate) shared: Client,
    pub(crate) pool_upstream: Client,
    pub(crate) proxy: Client,
    pub(crate) timeout: Duration,
    pub(crate) user_agent: String,
}

impl HttpClients {
    pub(crate) fn build(config: &AppConfig) -> Result<Self> {
        let timeout = config.request_timeout;
        let user_agent = config.user_agent.clone();

        let shared = Self::builder(Some(timeout), &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct shared HTTP client")?;

        // Pool live upstream traffic can legitimately stream well past REQUEST_TIMEOUT_SECS.
        // Handshake and upload budgets are enforced by route-specific timeout wrappers instead.
        let pool_upstream = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct pool upstream HTTP client")?;

        let proxy = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct proxy HTTP client")?;

        Ok(Self {
            shared,
            pool_upstream,
            proxy,
            timeout,
            user_agent,
        })
    }

    pub(crate) fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
        if force_new_connection {
            let client = Self::builder(Some(self.timeout), &self.user_agent)
                .pool_max_idle_per_host(0)
                .build()
                .context("failed to construct dedicated HTTP client")?;
            Ok(client)
        } else {
            Ok(self.shared.clone())
        }
    }

    pub(crate) fn client_for_pool_upstream(&self) -> Client {
        self.pool_upstream.clone()
    }

    pub(crate) fn client_for_forward_proxy(&self, endpoint_url: Option<&Url>) -> Result<Client> {
        let Some(endpoint_url) = endpoint_url else {
            return Ok(self.proxy.clone());
        };

        Self::builder(None, &self.user_agent)
            .pool_max_idle_per_host(2)
            .connect_timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .proxy(
                Proxy::all(endpoint_url.as_str())
                    .with_context(|| format!("invalid forward proxy endpoint: {endpoint_url}"))?,
            )
            .build()
            .context("failed to construct forward proxy HTTP client")
    }

    pub(crate) fn builder(timeout: Option<Duration>, user_agent: &str) -> ClientBuilder {
        let builder = Client::builder()
            .user_agent(user_agent)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(90))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(30))
            .http2_keep_alive_while_idle(true);

        if let Some(timeout) = timeout {
            builder.timeout(timeout)
        } else {
            builder
        }
    }
}
