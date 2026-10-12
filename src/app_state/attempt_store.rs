use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

pub(crate) const PROXY_RUNTIME_ATTEMPT_STORE_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);
pub(crate) const PROXY_RUNTIME_ATTEMPT_STORE_MAX_RECORDS: usize = 10_000;
pub(crate) const PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeAttemptSnapshot {
    pub(crate) attempt_public_id: String,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) routing_source: Option<String>,
    pub(crate) routing_selection_audit_json: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) upstream_base_url_host: Option<String>,
    pub(crate) group_name_snapshot: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) upstream_request_model: Option<String>,
    pub(crate) model_mapping_pattern: Option<String>,
    pub(crate) request_summary_json: Option<String>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_route_key: String,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    pub(crate) phase: Option<String>,
    pub(crate) http_status: Option<i64>,
    pub(crate) downstream_http_status: Option<i64>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) connect_latency_ms: Option<f64>,
    pub(crate) first_byte_latency_ms: Option<f64>,
    pub(crate) stream_latency_ms: Option<f64>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    pub(crate) upstream_request_compression_mode: Option<String>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_codec: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: bool,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeAttemptResponseCapture {
    pub(crate) raw_path: Option<String>,
    pub(crate) raw_codec: Option<String>,
    pub(crate) raw_size: Option<i64>,
    pub(crate) raw_truncated: bool,
    pub(crate) raw_truncated_reason: Option<String>,
    pub(crate) content_encoding: Option<String>,
}

impl RuntimeAttemptSnapshot {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        let optional_string_bytes =
            |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
        std::mem::size_of::<Self>()
            + self.attempt_public_id.capacity()
            + self.invoke_id.capacity()
            + self.occurred_at.capacity()
            + self.endpoint.capacity()
            + self.sticky_key.as_ref().map_or(0, String::capacity)
            + self.routing_source.as_ref().map_or(0, String::capacity)
            + self
                .routing_selection_audit_json
                .as_ref()
                .map_or(0, String::capacity)
            + self.requester_ip.as_ref().map_or(0, String::capacity)
            + self
                .upstream_base_url_host
                .as_ref()
                .map_or(0, String::capacity)
            + self
                .group_name_snapshot
                .as_ref()
                .map_or(0, String::capacity)
            + self
                .proxy_binding_key_snapshot
                .as_ref()
                .map_or(0, String::capacity)
            + self.request_model.as_ref().map_or(0, String::capacity)
            + self
                .upstream_request_model
                .as_ref()
                .map_or(0, String::capacity)
            + self
                .model_mapping_pattern
                .as_ref()
                .map_or(0, String::capacity)
            + self
                .request_summary_json
                .as_ref()
                .map_or(0, String::capacity)
            + self.upstream_route_key.capacity()
            + self.started_at.capacity()
            + optional_string_bytes(&self.finished_at)
            + self.status.capacity()
            + optional_string_bytes(&self.phase)
            + optional_string_bytes(&self.failure_kind)
            + optional_string_bytes(&self.error_message)
            + optional_string_bytes(&self.downstream_error_message)
            + optional_string_bytes(&self.upstream_request_id)
            + optional_string_bytes(&self.compact_support_status)
            + optional_string_bytes(&self.compact_support_reason)
            + optional_string_bytes(&self.upstream_request_compression_algorithm)
            + optional_string_bytes(&self.upstream_request_compression_mode)
            + optional_string_bytes(&self.response_raw_path)
            + optional_string_bytes(&self.response_raw_codec)
            + optional_string_bytes(&self.response_raw_truncated_reason)
            + optional_string_bytes(&self.response_content_encoding)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeAttemptStoreHealth {
    pub(crate) records: usize,
    pub(crate) bytes: usize,
    pub(crate) evicted_records: u64,
    pub(crate) rejected_records: u64,
}

#[derive(Debug)]
struct RuntimeAttemptEntry {
    snapshot: RuntimeAttemptSnapshot,
    retained_at: Instant,
    bytes: usize,
}

#[derive(Debug, Default)]
struct RuntimeAttemptStoreInner {
    entries: HashMap<(String, String, String), RuntimeAttemptEntry>,
    terminal_keys: HashMap<(String, String, String), Instant>,
    bytes: usize,
    evicted_records: u64,
    rejected_records: u64,
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeAttemptStore {
    inner: Mutex<RuntimeAttemptStoreInner>,
}

impl RuntimeAttemptStore {
    fn key(
        invoke_id: &str,
        occurred_at: &str,
        attempt_public_id: &str,
    ) -> (String, String, String) {
        (
            invoke_id.to_string(),
            occurred_at.to_string(),
            attempt_public_id.to_string(),
        )
    }

    pub(crate) fn upsert(&self, snapshot: RuntimeAttemptSnapshot) -> bool {
        self.upsert_inner(snapshot, false)
    }

    fn upsert_inner(&self, snapshot: RuntimeAttemptSnapshot, terminal_priority: bool) -> bool {
        let key = Self::key(
            &snapshot.invoke_id,
            &snapshot.occurred_at,
            &snapshot.attempt_public_id,
        );
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        if !terminal_priority && inner.terminal_keys.contains_key(&key) {
            return false;
        }

        let bytes = Self::entry_memory_bytes(&snapshot);
        if bytes > PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES {
            inner.rejected_records = inner.rejected_records.saturating_add(1);
            return false;
        }

        let previous = inner.entries.remove(&key);
        let snapshot = match previous.as_ref() {
            Some(previous) => merge_runtime_attempt_snapshot(&previous.snapshot, snapshot),
            None => snapshot,
        };
        let bytes = Self::entry_memory_bytes(&snapshot);
        if let Some(previous) = previous.as_ref() {
            inner.bytes = inner.bytes.saturating_sub(previous.bytes);
        }
        inner.bytes = inner.bytes.saturating_add(bytes);
        inner.entries.insert(
            key.clone(),
            RuntimeAttemptEntry {
                snapshot,
                retained_at: Instant::now(),
                bytes,
            },
        );
        if self.evict_to_bound(&mut inner, &key, terminal_priority) {
            return true;
        }
        if let Some(rejected) = inner.entries.remove(&key) {
            inner.bytes = inner.bytes.saturating_sub(rejected.bytes);
        }
        if let Some(previous) = previous {
            inner.bytes = inner.bytes.saturating_add(previous.bytes);
            inner.entries.insert(key, previous);
        }
        inner.rejected_records = inner.rejected_records.saturating_add(1);
        false
    }

    pub(crate) fn upsert_terminal(&self, snapshot: RuntimeAttemptSnapshot) -> bool {
        self.upsert_inner(snapshot, true)
    }

    pub(crate) fn get(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        attempt_public_id: &str,
    ) -> Option<RuntimeAttemptSnapshot> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        inner
            .entries
            .get(&Self::key(invoke_id, occurred_at, attempt_public_id))
            .map(|entry| entry.snapshot.clone())
    }

    pub(crate) fn update_response_capture(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        attempt_public_id: &str,
        capture: RuntimeAttemptResponseCapture,
    ) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        let key = Self::key(invoke_id, occurred_at, attempt_public_id);
        let Some(existing) = inner.entries.get(&key) else {
            return false;
        };
        let old_bytes = existing.bytes;
        let mut updated = existing.snapshot.clone();
        updated.response_raw_path = capture.raw_path;
        updated.response_raw_codec = capture.raw_codec;
        updated.response_raw_size = capture.raw_size;
        updated.response_raw_truncated = capture.raw_truncated;
        updated.response_raw_truncated_reason = capture.raw_truncated_reason;
        updated.response_content_encoding = capture.content_encoding;
        let updated_bytes = Self::entry_memory_bytes(&updated);
        let available = inner
            .bytes
            .saturating_sub(old_bytes)
            .saturating_add(updated_bytes);
        if updated_bytes > PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES
            || available > PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES
        {
            return false;
        }
        inner.bytes = available;
        let Some(entry) = inner.entries.get_mut(&key) else {
            return false;
        };
        entry.snapshot = updated;
        entry.bytes = updated_bytes;
        true
    }

    pub(crate) fn mark_interrupted(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        attempt_public_id: &str,
        finished_at: &str,
    ) -> bool {
        let key = Self::key(invoke_id, occurred_at, attempt_public_id);
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        let Some(entry) = inner.entries.get_mut(&key) else {
            return false;
        };
        entry.snapshot.finished_at = Some(finished_at.to_string());
        entry.snapshot.status = "interrupted".to_string();
        entry.snapshot.phase = Some("interrupted".to_string());
        entry.snapshot.failure_kind = Some("invocation_interrupted".to_string());
        entry.snapshot.error_message =
            Some("proxy request ended before a terminal record was written".to_string());
        entry.retained_at = Instant::now();
        true
    }

    pub(crate) fn discard_for_attempt(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        attempt_public_id: &str,
    ) -> bool {
        let key = Self::key(invoke_id, occurred_at, attempt_public_id);
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let removed = inner.entries.remove(&key);
        if let Some(entry) = removed {
            inner.bytes = inner.bytes.saturating_sub(entry.bytes);
            inner.terminal_keys.insert(key, Instant::now());
            self.evict_expired(&mut inner);
            true
        } else {
            false
        }
    }

    pub(crate) fn take_for_invocation(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Vec<RuntimeAttemptSnapshot> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let keys = inner
            .entries
            .keys()
            .filter(|(candidate_invoke_id, candidate_occurred_at, _)| {
                candidate_invoke_id == invoke_id && candidate_occurred_at == occurred_at
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(keys.len());
        for key in keys {
            inner.terminal_keys.insert(key.clone(), Instant::now());
            if let Some(entry) = inner.entries.remove(&key) {
                inner.bytes = inner.bytes.saturating_sub(entry.bytes);
                snapshots.push(entry.snapshot);
            }
        }
        self.evict_expired(&mut inner);
        snapshots.sort_by_key(|snapshot| {
            (
                snapshot.attempt_index,
                snapshot.distinct_account_index,
                snapshot.same_account_retry_index,
            )
        });
        snapshots
    }

    pub(crate) fn list_for_invocation(
        &self,
        invoke_id: &str,
        occurred_at: Option<&str>,
    ) -> Vec<RuntimeAttemptSnapshot> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        let mut snapshots = inner
            .entries
            .values()
            .filter(|entry| {
                entry.snapshot.invoke_id == invoke_id
                    && occurred_at.is_none_or(|value| entry.snapshot.occurred_at == value)
            })
            .map(|entry| entry.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| {
            (
                snapshot.attempt_index,
                snapshot.distinct_account_index,
                snapshot.same_account_retry_index,
            )
        });
        snapshots
    }

    pub(crate) fn restore(&self, snapshots: impl IntoIterator<Item = RuntimeAttemptSnapshot>) {
        for snapshot in snapshots {
            let key = Self::key(
                &snapshot.invoke_id,
                &snapshot.occurred_at,
                &snapshot.attempt_public_id,
            );
            self.inner
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .terminal_keys
                .remove(&key);
            let _ = self.upsert(snapshot);
        }
    }

    pub(crate) fn health_snapshot(&self) -> RuntimeAttemptStoreHealth {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        self.evict_expired(&mut inner);
        RuntimeAttemptStoreHealth {
            records: inner.entries.len(),
            bytes: inner
                .bytes
                .saturating_add(Self::terminal_fence_memory_bytes(&inner)),
            evicted_records: inner.evicted_records,
            rejected_records: inner.rejected_records,
        }
    }

    fn evict_expired(&self, inner: &mut RuntimeAttemptStoreInner) {
        let now = Instant::now();
        inner.terminal_keys.retain(|_, retained_at| {
            now.duration_since(*retained_at) <= PROXY_RUNTIME_ATTEMPT_STORE_MAX_AGE
        });
        let expired = inner
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.snapshot.finished_at.is_some()
                    && now.duration_since(entry.retained_at) > PROXY_RUNTIME_ATTEMPT_STORE_MAX_AGE)
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in expired {
            if let Some(entry) = inner.entries.remove(&key) {
                inner.bytes = inner.bytes.saturating_sub(entry.bytes);
                inner.evicted_records = inner.evicted_records.saturating_add(1);
            }
        }
        while inner
            .entries
            .len()
            .saturating_add(inner.terminal_keys.len())
            > PROXY_RUNTIME_ATTEMPT_STORE_MAX_RECORDS
            || Self::terminal_fence_memory_bytes(inner) > PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES
        {
            let Some(oldest_key) = inner
                .terminal_keys
                .iter()
                .min_by_key(|(_, retained_at)| **retained_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            inner.terminal_keys.remove(&oldest_key);
        }
    }

    fn evict_to_bound(
        &self,
        inner: &mut RuntimeAttemptStoreInner,
        protected_key: &(String, String, String),
        _terminal_priority: bool,
    ) -> bool {
        while inner
            .entries
            .len()
            .saturating_add(inner.terminal_keys.len())
            > PROXY_RUNTIME_ATTEMPT_STORE_MAX_RECORDS
            || inner
                .bytes
                .saturating_add(Self::terminal_fence_memory_bytes(inner))
                > PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES
        {
            if let Some(oldest_key) = inner
                .terminal_keys
                .iter()
                .min_by_key(|(_, retained_at)| **retained_at)
                .map(|(key, _)| key.clone())
            {
                inner.terminal_keys.remove(&oldest_key);
                continue;
            }
            let Some(oldest_key) = inner
                .entries
                .iter()
                .filter(|(key, entry)| {
                    *key != protected_key && entry.snapshot.finished_at.is_some()
                })
                .min_by_key(|(_, entry)| entry.retained_at)
                .map(|(key, _)| key.clone())
            else {
                return false;
            };
            if let Some(entry) = inner.entries.remove(&oldest_key) {
                inner.bytes = inner.bytes.saturating_sub(entry.bytes);
                inner.evicted_records = inner.evicted_records.saturating_add(1);
            }
        }
        true
    }

    fn entry_memory_bytes(snapshot: &RuntimeAttemptSnapshot) -> usize {
        snapshot
            .estimated_memory_bytes()
            .saturating_add(snapshot.invoke_id.capacity())
            .saturating_add(snapshot.occurred_at.capacity())
            .saturating_add(snapshot.attempt_public_id.capacity())
            .saturating_add(std::mem::size_of::<RuntimeAttemptEntry>())
    }

    fn terminal_fence_memory_bytes(inner: &RuntimeAttemptStoreInner) -> usize {
        inner
            .terminal_keys
            .keys()
            .map(|(invoke_id, occurred_at, attempt_public_id)| {
                std::mem::size_of::<(String, String, String)>()
                    + invoke_id.capacity()
                    + occurred_at.capacity()
                    + attempt_public_id.capacity()
                    + std::mem::size_of::<Instant>()
            })
            .sum()
    }
}

pub(crate) fn merge_runtime_attempt_snapshot(
    previous: &RuntimeAttemptSnapshot,
    incoming: RuntimeAttemptSnapshot,
) -> RuntimeAttemptSnapshot {
    if previous.finished_at.is_some() && incoming.finished_at.is_none() {
        let mut terminal = previous.clone();
        terminal.response_raw_path = incoming.response_raw_path;
        terminal.response_raw_codec = incoming.response_raw_codec;
        terminal.response_raw_size = incoming.response_raw_size;
        terminal.response_raw_truncated = incoming.response_raw_truncated;
        terminal.response_raw_truncated_reason = incoming.response_raw_truncated_reason;
        terminal.response_content_encoding = incoming.response_content_encoding;
        terminal
    } else {
        incoming
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(id: &str, phase: &str) -> RuntimeAttemptSnapshot {
        RuntimeAttemptSnapshot {
            attempt_public_id: id.to_string(),
            invoke_id: "invoke-1".to_string(),
            occurred_at: "2026-10-12 10:00:00".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: None,
            routing_source: Some("pool".to_string()),
            routing_selection_audit_json: None,
            requester_ip: None,
            upstream_base_url_host: Some("example.invalid".to_string()),
            group_name_snapshot: None,
            proxy_binding_key_snapshot: None,
            request_model: Some("gpt-test".to_string()),
            upstream_request_model: None,
            model_mapping_pattern: None,
            request_summary_json: None,
            upstream_account_id: 7,
            upstream_route_key: "route-1".to_string(),
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 0,
            started_at: "2026-10-12 10:00:00".to_string(),
            finished_at: None,
            status: "pending".to_string(),
            phase: Some(phase.to_string()),
            http_status: None,
            downstream_http_status: None,
            failure_kind: None,
            error_message: None,
            downstream_error_message: None,
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            stream_latency_ms: None,
            upstream_request_id: None,
            compact_support_status: None,
            compact_support_reason: None,
            upstream_request_compression_algorithm: None,
            upstream_request_compression_mode: None,
            upstream_request_logical_body_bytes: None,
            upstream_request_transmitted_body_bytes: None,
            upstream_request_header_bytes_approx: None,
            upstream_response_body_bytes: None,
            upstream_response_header_bytes_approx: None,
            response_raw_path: None,
            response_raw_codec: None,
            response_raw_size: None,
            response_raw_truncated: false,
            response_raw_truncated_reason: None,
            response_content_encoding: None,
        }
    }

    #[test]
    fn terminal_extraction_rejects_late_progress_until_restore() {
        let store = RuntimeAttemptStore::default();
        assert!(store.upsert(snapshot("attempt-fenced", "connecting")));
        let terminal = store.take_for_invocation("invoke-1", "2026-10-12 10:00:00");
        assert_eq!(terminal.len(), 1);
        assert!(!store.upsert(snapshot("attempt-fenced", "late_progress")));
        store.restore(terminal);
        assert!(store.upsert(snapshot("attempt-fenced", "recovered_progress")));
    }

    #[test]
    fn latest_snapshot_wins_for_one_attempt() {
        let store = RuntimeAttemptStore::default();
        assert!(store.upsert(snapshot("attempt-1", "connecting")));
        assert!(store.upsert(snapshot("attempt-1", "waiting_first_byte")));

        let snapshots = store.take_for_invocation("invoke-1", "2026-10-12 10:00:00");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].phase.as_deref(), Some("waiting_first_byte"));
        assert_eq!(store.health_snapshot().records, 0);
    }

    #[test]
    fn oversized_snapshot_is_rejected_instead_of_spilling_unbounded_memory() {
        let store = RuntimeAttemptStore::default();
        let mut oversized = snapshot("attempt-oversized", "terminal");
        oversized.request_summary_json = Some("x".repeat(PROXY_RUNTIME_ATTEMPT_STORE_MAX_BYTES));

        assert!(!store.upsert(oversized));
        let health = store.health_snapshot();
        assert_eq!(health.records, 0);
        assert_eq!(health.rejected_records, 1);
    }
}
