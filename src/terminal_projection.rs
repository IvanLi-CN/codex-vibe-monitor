use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use tokio::sync::Notify;

use crate::{
    ApiInvocation, AppState, DashboardActivityTerminalDeltaOutcome, SOURCE_PROXY,
    apply_dashboard_activity_terminal_record, ensure_dashboard_activity_live_snapshot_producer,
    rollback_dashboard_activity_terminal_record,
};

pub(crate) const TERMINAL_PROJECTION_MAX_PENDING_EVENTS: usize = 10_000;
pub(crate) const TERMINAL_PROJECTION_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub(crate) struct TerminalProjectionHealth {
    pub(crate) pending_event_count: usize,
    pub(crate) pending_event_bytes: usize,
    pub(crate) timeseries_pending_event_count: usize,
    pub(crate) timeseries_pending_bytes: usize,
    pub(crate) last_persisted_row_id: i64,
    pub(crate) long_term_cursor_row_id: i64,
    pub(crate) timeseries_cursor_row_id: i64,
    pub(crate) timeseries_consumer_active: bool,
    pub(crate) timeseries_coverage_invalidation_pending: bool,
    pub(crate) dirty_last_good: bool,
    pub(crate) hard_limit_reason: Option<&'static str>,
    pub(crate) registered_event_count: u64,
    pub(crate) persisted_ack_count: u64,
    pub(crate) pruned_event_count: u64,
    pub(crate) last_ack_age_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct TerminalProjectionEvent {
    id: u64,
    invoke_id: String,
    occurred_at: String,
    estimated_bytes: usize,
    persisted_row_id: Option<i64>,
    timeseries: Option<TimeseriesTerminalDelta>,
    timeseries_flushed: bool,
    timeseries_warm_coverage: HashSet<TimeseriesProjectionSelection>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TimeseriesTerminalDelta {
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<bool>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct TimeseriesProjectionSelection {
    pub(crate) source_scope: &'static str,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct TimeseriesProjectionSnapshotRecord {
    pub(crate) row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) delta: TimeseriesTerminalDelta,
}

impl From<&ApiInvocation> for TimeseriesTerminalDelta {
    fn from(record: &ApiInvocation) -> Self {
        Self {
            occurred_at: record.occurred_at.clone(),
            source: record.source.clone(),
            upstream_account_id: record.upstream_account_id,
            status: record.status.clone(),
            error_message: record.error_message.clone(),
            failure_kind: record.failure_kind.clone(),
            failure_class: record.failure_class.clone(),
            is_actionable: record.is_actionable,
            total_tokens: record.total_tokens,
            cache_input_tokens: record.cache_input_tokens,
            cost: record.cost,
            t_total_ms: record.t_total_ms,
            t_req_read_ms: record.t_req_read_ms,
            t_req_parse_ms: record.t_req_parse_ms,
            t_upstream_connect_ms: record.t_upstream_connect_ms,
            t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
            first_token_ms: record.first_token_ms,
        }
    }
}

impl TimeseriesTerminalDelta {
    fn estimated_bytes(&self) -> usize {
        self.occurred_at.len()
            + self.source.len()
            + self.status.as_deref().map_or(0, str::len)
            + self.error_message.as_deref().map_or(0, str::len)
            + self.failure_kind.as_deref().map_or(0, str::len)
            + self.failure_class.as_deref().map_or(0, str::len)
            + std::mem::size_of::<Self>()
    }

    fn matches_projection_snapshot(&self, snapshot: &TimeseriesTerminalDelta) -> bool {
        self.occurred_at == snapshot.occurred_at
            && self.status == snapshot.status
            && self.error_message == snapshot.error_message
            && self.failure_kind == snapshot.failure_kind
            && self.failure_class == snapshot.failure_class
            && self.is_actionable == snapshot.is_actionable
            && self.total_tokens == snapshot.total_tokens
            && self.cache_input_tokens == snapshot.cache_input_tokens
            && self.cost == snapshot.cost
            && self.t_total_ms == snapshot.t_total_ms
            && self.t_req_read_ms == snapshot.t_req_read_ms
            && self.t_req_parse_ms == snapshot.t_req_parse_ms
            && self.t_upstream_connect_ms == snapshot.t_upstream_connect_ms
            && self.t_upstream_ttfb_ms == snapshot.t_upstream_ttfb_ms
            && self.first_token_ms == snapshot.first_token_ms
    }
}

#[derive(Debug, Default)]
struct TerminalProjectionHubState {
    pending: VecDeque<TerminalProjectionEvent>,
    pending_bytes: usize,
    last_persisted_row_id: i64,
    long_term_cursor_row_id: i64,
    timeseries_cursor_row_id: i64,
    timeseries_consumer_active: bool,
    timeseries_coverage_invalidation_generation: u64,
    timeseries_coverage_completed_generation: u64,
    dirty_last_good: bool,
    hard_limit_reason: Option<&'static str>,
    registered_event_count: u64,
    persisted_ack_count: u64,
    pruned_event_count: u64,
    last_ack_at: Option<Instant>,
}

#[derive(Debug, Default)]
pub(crate) struct TerminalProjectionHub {
    next_event_id: AtomicU64,
    state: Mutex<TerminalProjectionHubState>,
    persisted_notify: Notify,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TerminalProjectionRegistration {
    pub(crate) event_id: Option<u64>,
    pub(crate) dashboard: DashboardActivityTerminalDeltaOutcome,
}

impl TerminalProjectionHub {
    pub(crate) fn register_pending(
        &self,
        record: &ApiInvocation,
        _dashboard_terminal_sequence: Option<u64>,
    ) -> Option<u64> {
        let timeseries = TimeseriesTerminalDelta::from(record);
        let estimated_bytes =
            record.invoke_id.len() + record.occurred_at.len() + timeseries.estimated_bytes() + 64;
        self.register_pending_parts_with_delta(
            &record.invoke_id,
            &record.occurred_at,
            estimated_bytes,
            Some(timeseries),
        )
    }

    fn register_pending_parts(&self, invoke_id: &str, occurred_at: &str) -> Option<u64> {
        self.register_pending_parts_with_delta(
            invoke_id,
            occurred_at,
            invoke_id.len() + occurred_at.len() + 64,
            None,
        )
    }

    fn register_pending_parts_with_delta(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        estimated_bytes: usize,
        timeseries: Option<TimeseriesTerminalDelta>,
    ) -> Option<u64> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending.len() >= TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
            state.dirty_last_good = true;
            state.hard_limit_reason = Some("pending_event_count");
            if timeseries.is_some() {
                state.timeseries_coverage_invalidation_generation = state
                    .timeseries_coverage_invalidation_generation
                    .saturating_add(1);
            }
            return None;
        }
        if state.pending_bytes.saturating_add(estimated_bytes)
            > TERMINAL_PROJECTION_MAX_PENDING_BYTES
        {
            state.dirty_last_good = true;
            state.hard_limit_reason = Some("pending_event_bytes");
            if timeseries.is_some() {
                state.timeseries_coverage_invalidation_generation = state
                    .timeseries_coverage_invalidation_generation
                    .saturating_add(1);
            }
            return None;
        }
        let id = self
            .next_event_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let timeseries_flushed = timeseries.is_none();
        state.pending_bytes = state.pending_bytes.saturating_add(estimated_bytes);
        state.pending.push_back(TerminalProjectionEvent {
            id,
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            estimated_bytes,
            persisted_row_id: None,
            timeseries,
            timeseries_flushed,
            timeseries_warm_coverage: HashSet::new(),
        });
        state.registered_event_count = state.registered_event_count.saturating_add(1);
        Some(id)
    }

    pub(crate) fn discard_pending(&self, event_id: Option<u64>) {
        let Some(event_id) = event_id else {
            return;
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = state.pending.iter().position(|event| event.id == event_id)
            && let Some(event) = state.pending.remove(index)
        {
            state.pending_bytes = state.pending_bytes.saturating_sub(event.estimated_bytes);
        }
    }

    pub(crate) fn acknowledge_persisted(
        &self,
        event_id: Option<u64>,
        invoke_id: &str,
        occurred_at: &str,
        row_id: i64,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let event_index = event_id
            .and_then(|id| state.pending.iter().position(|event| event.id == id))
            .or_else(|| {
                state.pending.iter().position(|event| {
                    event.invoke_id == invoke_id && event.occurred_at == occurred_at
                })
            });
        let event = event_index.and_then(|index| state.pending.get_mut(index));
        if let Some(event) = event {
            event.persisted_row_id = Some(row_id);
        } else {
            // Journal replay can persist before this process saw the original ingress event.
            // The durable row cursor below makes that recovery path exact without retaining
            // the original payload in memory.
            state.dirty_last_good = true;
        }
        state.last_persisted_row_id = state.last_persisted_row_id.max(row_id);
        state.persisted_ack_count = state.persisted_ack_count.saturating_add(1);
        state.last_ack_at = Some(Instant::now());
        self.persisted_notify.notify_one();
    }

    pub(crate) fn advance_long_term_cursor(&self, cursor_row_id: i64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.long_term_cursor_row_id = state.long_term_cursor_row_id.max(cursor_row_id);
        Self::prune_acknowledged_locked(&mut state);
    }

    pub(crate) fn activate_timeseries_consumer(&self, cursor_row_id: i64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.timeseries_consumer_active = true;
        state.timeseries_cursor_row_id = state.timeseries_cursor_row_id.max(cursor_row_id);
        Self::prune_acknowledged_locked(&mut state);
    }

    pub(crate) fn pending_timeseries_deltas(
        &self,
        limit: usize,
    ) -> Vec<(u64, i64, TimeseriesTerminalDelta)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut deltas = state
            .pending
            .iter()
            .filter_map(|event| {
                let row_id = event.persisted_row_id?;
                (!event.timeseries_flushed)
                    .then(|| {
                        event
                            .timeseries
                            .clone()
                            .map(|delta| (event.id, row_id, delta))
                    })
                    .flatten()
            })
            .collect::<Vec<_>>();
        deltas.sort_by_key(|(event_id, row_id, _)| (*row_id, *event_id));
        deltas.truncate(limit);
        deltas
    }

    pub(crate) fn pending_timeseries_deltas_for_selection(
        &self,
        selection: TimeseriesProjectionSelection,
        limit: usize,
    ) -> Vec<(u64, i64, TimeseriesTerminalDelta)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut deltas = state
            .pending
            .iter()
            .filter_map(|event| {
                let row_id = event.persisted_row_id?;
                let delta = event.timeseries.as_ref()?;
                (!event.timeseries_flushed
                    && !event.timeseries_warm_coverage.contains(&selection)
                    && (selection.source_scope != "proxy_only" || delta.source == SOURCE_PROXY)
                    && selection
                        .upstream_account_id
                        .is_none_or(|account_id| delta.upstream_account_id == Some(account_id)))
                .then(|| (event.id, row_id, delta.clone()))
            })
            .collect::<Vec<_>>();
        deltas.sort_by_key(|(event_id, row_id, _)| (*row_id, *event_id));
        deltas.truncate(limit);
        deltas
    }

    pub(crate) fn mark_timeseries_warm_coverage(
        &self,
        selection: TimeseriesProjectionSelection,
        records: &[TimeseriesProjectionSnapshotRecord],
    ) -> usize {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut covered = 0;
        for event in &mut state.pending {
            let Some(row_id) = event.persisted_row_id else {
                continue;
            };
            let Some(delta) = event.timeseries.as_ref() else {
                continue;
            };
            if event.timeseries_flushed
                || event.timeseries_warm_coverage.contains(&selection)
                || (selection.source_scope == "proxy_only" && delta.source != SOURCE_PROXY)
                || selection
                    .upstream_account_id
                    .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
            {
                continue;
            }
            if records.iter().any(|record| {
                record.row_id == row_id
                    && record.invoke_id == event.invoke_id
                    && record.occurred_at == event.occurred_at
                    && delta.matches_projection_snapshot(&record.delta)
            }) {
                event.timeseries_warm_coverage.insert(selection);
                covered += 1;
            }
        }
        covered
    }

    pub(crate) fn mark_timeseries_deltas_flushed(&self, event_ids: &[u64]) {
        if event_ids.is_empty() {
            return;
        }
        let flushed = event_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for event in &mut state.pending {
            if flushed.contains(&event.id) {
                event.timeseries_flushed = true;
            }
        }
        if let Some(cursor) = state
            .pending
            .iter()
            .filter(|event| flushed.contains(&event.id))
            .filter_map(|event| event.persisted_row_id)
            .max()
        {
            state.timeseries_cursor_row_id = state.timeseries_cursor_row_id.max(cursor);
        }
        Self::prune_acknowledged_locked(&mut state);
    }

    // A rejected terminal event can update a row at or behind the durable cursor. Mark
    // all minute coverage warming before projection reads may trust it again.
    pub(crate) fn timeseries_coverage_invalidation_pending(&self) -> Option<u64> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (state.timeseries_coverage_invalidation_generation
            > state.timeseries_coverage_completed_generation)
            .then_some(state.timeseries_coverage_invalidation_generation)
    }

    pub(crate) fn complete_timeseries_coverage_invalidation(&self, generation: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.timeseries_coverage_invalidation_generation == generation {
            state.timeseries_coverage_completed_generation = generation;
        }
    }

    fn prune_acknowledged_locked(state: &mut TerminalProjectionHubState) {
        // SQLite ACKs can arrive out of ingress order. Remove every event whose
        // durable row is behind every active projection consumer cursor instead
        // of allowing one delayed head event to retain an otherwise acknowledged tail.
        let mut retained = VecDeque::with_capacity(state.pending.len());
        while let Some(event) = state.pending.pop_front() {
            if event.persisted_row_id.is_some_and(|row_id| {
                row_id <= state.long_term_cursor_row_id
                    && (!state.timeseries_consumer_active || event.timeseries_flushed)
            }) {
                state.pending_bytes = state.pending_bytes.saturating_sub(event.estimated_bytes);
                state.pruned_event_count = state.pruned_event_count.saturating_add(1);
            } else {
                retained.push_back(event);
            }
        }
        state.pending = retained;
        if state.pending.len() < TERMINAL_PROJECTION_MAX_PENDING_EVENTS
            && state.pending_bytes < TERMINAL_PROJECTION_MAX_PENDING_BYTES
        {
            state.hard_limit_reason = None;
            state.dirty_last_good = false;
        }
    }

    pub(crate) fn has_persisted_work(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.last_persisted_row_id > state.long_term_cursor_row_id || state.dirty_last_good
    }

    pub(crate) async fn wait_for_persisted_work(&self) {
        self.persisted_notify.notified().await;
    }

    pub(crate) fn health(&self) -> TerminalProjectionHealth {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TerminalProjectionHealth {
            pending_event_count: state.pending.len(),
            pending_event_bytes: state.pending_bytes,
            timeseries_pending_event_count: state
                .pending
                .iter()
                .filter(|event| event.timeseries.is_some() && !event.timeseries_flushed)
                .count(),
            timeseries_pending_bytes: state
                .pending
                .iter()
                .filter(|event| event.timeseries.is_some() && !event.timeseries_flushed)
                .filter_map(|event| event.timeseries.as_ref())
                .map(TimeseriesTerminalDelta::estimated_bytes)
                .sum(),
            last_persisted_row_id: state.last_persisted_row_id,
            long_term_cursor_row_id: state.long_term_cursor_row_id,
            timeseries_cursor_row_id: state.timeseries_cursor_row_id,
            timeseries_consumer_active: state.timeseries_consumer_active,
            timeseries_coverage_invalidation_pending: state
                .timeseries_coverage_invalidation_generation
                > state.timeseries_coverage_completed_generation,
            dirty_last_good: state.dirty_last_good,
            hard_limit_reason: state.hard_limit_reason,
            registered_event_count: state.registered_event_count,
            persisted_ack_count: state.persisted_ack_count,
            pruned_event_count: state.pruned_event_count,
            last_ack_age_ms: state
                .last_ack_at
                .map(|last_ack| last_ack.elapsed().as_millis() as u64),
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn timeseries_delta() -> TimeseriesTerminalDelta {
        TimeseriesTerminalDelta {
            occurred_at: "2026-07-30 10:00:00".to_string(),
            source: "proxy".to_string(),
            upstream_account_id: None,
            status: Some("success".to_string()),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            total_tokens: None,
            cache_input_tokens: None,
            cost: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
        }
    }

    #[test]
    fn long_term_cursor_prunes_out_of_order_persisted_events() {
        let hub = TerminalProjectionHub::default();
        let first = hub.register_pending_parts("first", "2026-07-30 10:00:00");
        let second = hub.register_pending_parts("second", "2026-07-30 10:00:01");

        hub.acknowledge_persisted(second, "second", "2026-07-30 10:00:01", 12);
        hub.advance_long_term_cursor(12);
        assert_eq!(hub.health().pending_event_count, 1);

        hub.acknowledge_persisted(first, "first", "2026-07-30 10:00:00", 11);
        hub.advance_long_term_cursor(12);
        let health = hub.health();
        assert_eq!(health.pending_event_count, 0);
        assert_eq!(health.pruned_event_count, 2);
    }

    #[test]
    fn active_timeseries_consumer_prevents_early_long_term_pruning() {
        let hub = TerminalProjectionHub::default();
        let event = hub
            .register_pending_parts_with_delta(
                "invoke",
                "2026-07-30 10:00:00",
                128,
                Some(timeseries_delta()),
            )
            .expect("event is within the projection hard limit");

        hub.activate_timeseries_consumer(0);
        hub.acknowledge_persisted(Some(event), "invoke", "2026-07-30 10:00:00", 17);
        hub.advance_long_term_cursor(17);
        let health = hub.health();
        assert_eq!(health.pending_event_count, 1);
        assert_eq!(health.timeseries_pending_event_count, 1);
        assert_eq!(
            health.timeseries_pending_bytes,
            timeseries_delta().estimated_bytes()
        );
        assert_eq!(hub.pending_timeseries_deltas(10).len(), 1);

        hub.mark_timeseries_deltas_flushed(&[event]);
        let health = hub.health();
        assert_eq!(health.pending_event_count, 0);
        assert_eq!(health.timeseries_pending_event_count, 0);
        assert_eq!(health.timeseries_pending_bytes, 0);
        assert_eq!(health.timeseries_cursor_row_id, 17);
        assert!(health.timeseries_consumer_active);
        assert!(hub.pending_timeseries_deltas(10).is_empty());
    }

    #[test]
    fn flushing_a_snapshot_does_not_ack_a_later_event_with_the_same_row() {
        let hub = TerminalProjectionHub::default();
        hub.activate_timeseries_consumer(0);
        let first = hub
            .register_pending_parts_with_delta(
                "invoke",
                "2026-07-30 10:00:00",
                128,
                Some(timeseries_delta()),
            )
            .expect("first event is within the projection hard limit");
        hub.acknowledge_persisted(Some(first), "invoke", "2026-07-30 10:00:00", 17);
        hub.advance_long_term_cursor(17);

        let snapshot = hub.pending_timeseries_deltas(10);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, first);

        let second = hub
            .register_pending_parts_with_delta(
                "invoke",
                "2026-07-30 10:00:01",
                128,
                Some(timeseries_delta()),
            )
            .expect("second event is within the projection hard limit");
        hub.acknowledge_persisted(Some(second), "invoke", "2026-07-30 10:00:01", 17);

        hub.mark_timeseries_deltas_flushed(&[first]);
        let pending = hub.pending_timeseries_deltas(10);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, second);
        assert_eq!(pending[0].1, 17);
    }

    #[test]
    fn warm_coverage_skips_only_the_terminal_snapshot_it_contains() {
        let hub = TerminalProjectionHub::default();
        let selection = TimeseriesProjectionSelection {
            source_scope: "all",
            upstream_account_id: None,
        };
        let delta = timeseries_delta();
        let first = hub
            .register_pending_parts_with_delta(
                "invoke",
                &delta.occurred_at,
                128,
                Some(delta.clone()),
            )
            .expect("first event is within the projection hard limit");
        hub.acknowledge_persisted(Some(first), "invoke", &delta.occurred_at, 17);
        assert_eq!(
            hub.mark_timeseries_warm_coverage(
                selection,
                &[TimeseriesProjectionSnapshotRecord {
                    row_id: 17,
                    invoke_id: "invoke".to_string(),
                    occurred_at: delta.occurred_at.clone(),
                    delta: delta.clone(),
                }],
            ),
            1
        );
        assert!(
            hub.pending_timeseries_deltas_for_selection(selection, 10)
                .is_empty()
        );

        let mut updated_delta = delta;
        updated_delta.status = Some("failure".to_string());
        let updated_occurred_at = updated_delta.occurred_at.clone();
        let second = hub
            .register_pending_parts_with_delta(
                "invoke",
                &updated_occurred_at,
                128,
                Some(updated_delta),
            )
            .expect("updated event is within the projection hard limit");
        hub.acknowledge_persisted(Some(second), "invoke", "2026-07-30 10:00:00", 17);
        let pending = hub.pending_timeseries_deltas_for_selection(selection, 10);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, second);
    }

    #[test]
    fn hard_limit_marks_projection_dirty_without_dropping_existing_events() {
        let hub = TerminalProjectionHub::default();
        for index in 0..TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
            assert!(
                hub.register_pending_parts(&format!("invoke-{index}"), "2026-07-30 10:00:00")
                    .is_some()
            );
        }

        assert!(
            hub.register_pending_parts("beyond-limit", "2026-07-30 10:00:01")
                .is_none()
        );
        let health = hub.health();
        assert!(health.dirty_last_good);
        assert_eq!(health.hard_limit_reason, Some("pending_event_count"));
        assert_eq!(
            health.pending_event_count,
            TERMINAL_PROJECTION_MAX_PENDING_EVENTS
        );
    }

    #[test]
    fn rejected_timeseries_delta_invalidates_minute_projection_coverage() {
        let hub = TerminalProjectionHub::default();
        for index in 0..TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
            assert!(
                hub.register_pending_parts(&format!("invoke-{index}"), "2026-07-30 10:00:00")
                    .is_some()
            );
        }

        assert!(
            hub.register_pending_parts_with_delta(
                "terminal-update",
                "2026-07-30 10:00:01",
                128,
                Some(timeseries_delta()),
            )
            .is_none()
        );
        let generation = hub
            .timeseries_coverage_invalidation_pending()
            .expect("rejection must invalidate minute coverage");

        hub.complete_timeseries_coverage_invalidation(generation);
        assert!(hub.timeseries_coverage_invalidation_pending().is_none());
    }
}

pub(crate) async fn register_terminal_projection_before_enqueue(
    state: &AppState,
    record: &ApiInvocation,
) -> TerminalProjectionRegistration {
    let dashboard = apply_dashboard_activity_terminal_record(state, record).await;
    if let Some(delta) = dashboard.terminal_delta.clone() {
        state
            .proxy_runtime_invocations
            .record_dashboard_terminal_delta(delta);
        ensure_dashboard_activity_live_snapshot_producer(state);
    }
    let event_id = state
        .terminal_projection_hub
        .register_pending(record, dashboard.terminal_sequence);
    TerminalProjectionRegistration {
        event_id,
        dashboard,
    }
}

pub(crate) async fn rollback_terminal_projection_before_enqueue(
    state: &AppState,
    record: &ApiInvocation,
    registration: &TerminalProjectionRegistration,
) {
    state
        .proxy_runtime_invocations
        .discard_dashboard_terminal_delta(&record.invoke_id, &record.occurred_at);
    state
        .terminal_projection_hub
        .discard_pending(registration.event_id);
    rollback_dashboard_activity_terminal_record(
        state,
        record,
        registration.dashboard.terminal_sequence,
    )
    .await;
}
