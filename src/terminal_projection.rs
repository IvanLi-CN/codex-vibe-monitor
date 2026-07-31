use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use tokio::sync::Notify;

use crate::{
    ApiInvocation, AppState, DashboardActivityTerminalDeltaOutcome,
    apply_dashboard_activity_terminal_record, rollback_dashboard_activity_terminal_record,
};

pub(crate) const TERMINAL_PROJECTION_MAX_PENDING_EVENTS: usize = 10_000;
pub(crate) const TERMINAL_PROJECTION_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub(crate) struct TerminalProjectionHealth {
    pub(crate) pending_event_count: usize,
    pub(crate) pending_event_bytes: usize,
    pub(crate) last_persisted_row_id: i64,
    pub(crate) long_term_cursor_row_id: i64,
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
}

#[derive(Debug, Default)]
struct TerminalProjectionHubState {
    pending: VecDeque<TerminalProjectionEvent>,
    pending_bytes: usize,
    last_persisted_row_id: i64,
    long_term_cursor_row_id: i64,
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
        self.register_pending_parts(&record.invoke_id, &record.occurred_at)
    }

    fn register_pending_parts(&self, invoke_id: &str, occurred_at: &str) -> Option<u64> {
        let estimated_bytes = invoke_id.len() + occurred_at.len() + 64;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending.len() >= TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
            state.dirty_last_good = true;
            state.hard_limit_reason = Some("pending_event_count");
            return None;
        }
        if state.pending_bytes.saturating_add(estimated_bytes)
            > TERMINAL_PROJECTION_MAX_PENDING_BYTES
        {
            state.dirty_last_good = true;
            state.hard_limit_reason = Some("pending_event_bytes");
            return None;
        }
        let id = self
            .next_event_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        state.pending_bytes = state.pending_bytes.saturating_add(estimated_bytes);
        state.pending.push_back(TerminalProjectionEvent {
            id,
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            estimated_bytes,
            persisted_row_id: None,
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
        // SQLite ACKs can arrive out of ingress order. Remove every event whose
        // durable row is already behind the long-term cursor instead of allowing
        // one delayed head event to retain an otherwise acknowledged tail.
        let mut retained = VecDeque::with_capacity(state.pending.len());
        while let Some(event) = state.pending.pop_front() {
            if event
                .persisted_row_id
                .is_some_and(|row_id| row_id <= state.long_term_cursor_row_id)
            {
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
            last_persisted_row_id: state.last_persisted_row_id,
            long_term_cursor_row_id: state.long_term_cursor_row_id,
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
}

pub(crate) async fn register_terminal_projection_before_enqueue(
    state: &AppState,
    record: &ApiInvocation,
) -> TerminalProjectionRegistration {
    let dashboard = apply_dashboard_activity_terminal_record(state, record).await;
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
        .terminal_projection_hub
        .discard_pending(registration.event_id);
    rollback_dashboard_activity_terminal_record(
        state,
        record,
        registration.dashboard.terminal_sequence,
    )
    .await;
}
