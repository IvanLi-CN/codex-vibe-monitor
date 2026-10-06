use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    ApiInvocation, MemoryComponentEstimate, PromptCacheConversationInvocationPreviewResponse,
    normalize_trimmed_optional_string_local, prompt_cache_invocation_preview_from_runtime_record,
};

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

/// Typed data needed by the active Prompt Cache projection. This deliberately excludes the
/// complete runtime record and its raw/detail fields.
#[derive(Debug, Clone)]
pub(crate) struct PromptCacheRuntimeProjection {
    pub(crate) row_id: i64,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) preview: PromptCacheConversationInvocationPreviewResponse,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reported_cache_write_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
}

impl PromptCacheRuntimeProjection {
    pub(crate) fn from_record(record: &ApiInvocation) -> Option<Self> {
        let prompt_cache_key =
            normalize_trimmed_optional_string_local(record.prompt_cache_key.clone());
        let sticky_key = normalize_trimmed_optional_string_local(record.sticky_key.clone());
        let preview_key = prompt_cache_key.clone().or_else(|| sticky_key.clone())?;
        let mut preview = prompt_cache_invocation_preview_from_runtime_record(record, preview_key);
        preview.cost = non_negative_finite_f64(record.cost);
        Some(Self {
            row_id: record.id,
            prompt_cache_key,
            sticky_key,
            preview,
            input_tokens: non_negative_i64(record.input_tokens),
            output_tokens: non_negative_i64(record.output_tokens),
            cache_input_tokens: non_negative_i64(record.cache_input_tokens),
            reported_cache_write_tokens: non_negative_i64(record.reported_cache_write_tokens),
            reasoning_tokens: non_negative_i64(record.reasoning_tokens),
            cost_input: non_negative_finite_f64(record.cost_input),
            cost_cache_write: non_negative_finite_f64(record.cost_cache_write),
            cost_cache_read: non_negative_finite_f64(record.cost_cache_read),
            cost_output: non_negative_finite_f64(record.cost_output),
            cost_reasoning: non_negative_finite_f64(record.cost_reasoning),
        })
    }
}

fn non_negative_i64(value: Option<i64>) -> Option<i64> {
    value.map(|value| value.max(0))
}

fn non_negative_finite_f64(value: Option<f64>) -> Option<f64> {
    value.and_then(|value| value.is_finite().then_some(value.max(0.0)))
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
pub(crate) struct ProxyRuntimeInvocationStoreInner {
    pub(crate) records: HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    pub(crate) terminal_tombstones: HashMap<RuntimeInvocationKey, Instant>,
    pub(crate) projection_tombstones: HashMap<RuntimeInvocationKey, Instant>,
}

pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE: Duration =
    Duration::from_secs(6 * 60 * 60);
pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS: usize = 10_000;
pub(crate) const PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS: usize = 50_000;

#[derive(Debug)]
pub(crate) struct RuntimeInvocationStore {
    inner: Mutex<ProxyRuntimeInvocationStoreInner>,
    #[cfg(test)]
    full_record_clone_count: AtomicU64,
}

impl Default for RuntimeInvocationStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(ProxyRuntimeInvocationStoreInner::default()),
            #[cfg(test)]
            full_record_clone_count: AtomicU64::new(0),
        }
    }
}

impl RuntimeInvocationStore {
    pub(crate) fn lock(
        &self,
    ) -> std::sync::LockResult<MutexGuard<'_, ProxyRuntimeInvocationStoreInner>> {
        self.inner.lock()
    }

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
            return RuntimeInvocationStoreUpsertOutcome {
                running_count: guard.records.len(),
                pruned_count,
                skipped_terminal: true,
            };
        }
        guard.projection_tombstones.remove(&key);
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
    ) -> (RuntimeInvocationStoreRemoveOutcome, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (
                RuntimeInvocationStoreRemoveOutcome {
                    removed: false,
                    already_terminal: false,
                },
                0,
            );
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let already_terminal = guard.terminal_tombstones.contains_key(&key);
        if already_terminal {
            let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
            return (
                RuntimeInvocationStoreRemoveOutcome {
                    removed: false,
                    already_terminal: true,
                },
                pruned_count,
            );
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
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        (
            RuntimeInvocationStoreRemoveOutcome {
                removed,
                already_terminal: false,
            },
            pruned_count,
        )
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

    pub(crate) fn contains_terminal(&self, invoke_id: &str, occurred_at: &str) -> (bool, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (false, 0);
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let contains_terminal = guard.terminal_tombstones.contains_key(&key)
            || guard
                .records
                .get(&key)
                .is_some_and(|entry| runtime_store_record_is_terminal(&entry.record));
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        (contains_terminal, pruned_count)
    }

    pub(crate) fn remove_non_terminal(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> (Option<ApiInvocation>, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (None, 0);
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
            guard.projection_tombstones.insert(key, Instant::now());
        }
        let pruned_count = if removed.is_some() {
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now())
        } else {
            0
        };
        (removed, pruned_count)
    }

    pub(crate) fn remove_non_terminal_by_invoke_id(
        &self,
        invoke_id: &str,
    ) -> (Vec<(RuntimeInvocationKey, ApiInvocation)>, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (Vec::new(), 0);
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
        (removed, pruned_count)
    }

    pub(crate) fn remove_persisted_terminal_overlay(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> (bool, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (false, 0);
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let removed = guard.records.remove(&key).is_some();
        guard.terminal_tombstones.insert(key, now);
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        (removed, pruned_count)
    }

    pub(crate) fn snapshot(&self) -> (Vec<ApiInvocation>, usize) {
        let Ok(mut guard) = self.inner.lock() else {
            return (Vec::new(), 0);
        };
        let pruned_count =
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now());
        let snapshot = guard
            .records
            .values()
            .map(|entry| entry.record.clone())
            .collect();
        (snapshot, pruned_count)
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

fn prune_bounded_runtime_invocation_store_locked(
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

fn prune_bounded_runtime_invocations_locked(
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

fn prune_bounded_runtime_tombstones_locked(
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
