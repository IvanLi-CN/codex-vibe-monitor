use super::StartupBackfillTask;
use crate::observability::contention::{
    PerformanceCounter, PerformanceGauge, WakeDecision, WakeOutcome, WakeState, current_performance,
};
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Notify;

const WAKE_RETRY_BASE_DELAYS_MS: [i64; 5] = [250, 500, 1_000, 2_000, 5_000];
const WAKE_WARNING_INTERVAL_SECS: i64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WakeObligationKey {
    pub(super) task: StartupBackfillTask,
    pub(super) control_generation: Option<u64>,
    pub(super) progress_key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WakeObligationTicket {
    task: StartupBackfillTask,
    generation: u64,
}

#[derive(Clone, Debug)]
pub(super) struct WakeObligationAttempt {
    pub(super) ticket: WakeObligationTicket,
    pub(super) key: WakeObligationKey,
    pub(super) attempt: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WakeObligationFailure {
    pub(super) next_due: DateTime<Utc>,
    pub(super) should_warn: bool,
    pub(super) attempt: u32,
}

#[derive(Clone, Debug)]
struct WakeObligation {
    key: WakeObligationKey,
    next_due: DateTime<Utc>,
    attempt: u32,
    last_warning_at: Option<DateTime<Utc>>,
    observation_ticket: WakeObservationTicket,
    ticket: WakeObligationTicket,
    in_flight: bool,
}

#[derive(Debug, Default)]
pub(super) struct StartupBackfillScheduler {
    wake_generation: AtomicU64,
    observation_generation: AtomicU64,
    obligation_generation: AtomicU64,
    notify: Notify,
    woken_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    pub(super) next_due: std::sync::Mutex<HashMap<StartupBackfillTask, DateTime<Utc>>>,
    // Observation-only obligations awaiting confirmation of the durable wake/control write.
    pub(super) pending_since: std::sync::Mutex<HashMap<StartupBackfillTask, WakeObservation>>,
    wake_obligations: std::sync::Mutex<HashMap<StartupBackfillTask, WakeObligation>>,
    deferred_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    pressure_deferred_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    failed_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    pub(super) wake_count: AtomicU64,
    due_dispatch_count: AtomicU64,
    noop_suppressed_count: AtomicU64,
    pressure_defer_count: AtomicU64,
    failure_count: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WakeObservationTicket {
    task: StartupBackfillTask,
    generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WakeObservation {
    pub(super) ticket: WakeObservationTicket,
    pub(super) since: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupBackfillHealthSnapshot {
    pub(crate) state: String,
    pub(crate) wake_generation: u64,
    pub(crate) wake_count: u64,
    pub(crate) due_dispatch_count: u64,
    pub(crate) noop_suppressed_count: u64,
    pub(crate) pressure_defer_count: u64,
    pub(crate) failure_count: u64,
    pub(crate) woken_task_count: u64,
    pub(crate) scheduled_task_count: u64,
    pub(crate) deferred_task_count: u64,
    pub(crate) failed_task_count: u64,
}

impl StartupBackfillScheduler {
    pub(super) fn record_notification(
        &self,
        task: StartupBackfillTask,
        decision: WakeDecision,
    ) -> Option<WakeObservationTicket> {
        let decision = if matches!(decision, WakeDecision::New)
            && self
                .pending_since
                .lock()
                .map(|pending| pending.contains_key(&task))
                .unwrap_or(true)
        {
            WakeDecision::Merged
        } else {
            decision
        };
        if let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) {
            metrics.count_performance(PerformanceCounter::WakeNotification(task, decision), 1);
        }
        if matches!(decision, WakeDecision::Disabled) {
            self.clear_observation(task);
            self.publish_performance(task);
            return None;
        }

        let ticket = WakeObservationTicket {
            task,
            generation: self
                .observation_generation
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1),
        };
        let observed_at = Utc::now();
        if let Ok(mut pending_since) = self.pending_since.lock() {
            let since = pending_since
                .get(&task)
                .map(|observation| observation.since)
                .unwrap_or(observed_at);
            pending_since.insert(task, WakeObservation { ticket, since });
        }
        self.publish_performance(task);
        Some(ticket)
    }

    pub(super) fn wake(&self, task: StartupBackfillTask) {
        let observed_at = Utc::now();
        if let Ok(mut tasks) = self.woken_tasks.lock() {
            tasks.insert(task);
        }
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.insert(task, observed_at);
        }
        self.wake_count.fetch_add(1, Ordering::Relaxed);
        self.signal();
        self.publish_performance(task);
    }

    pub(super) fn accept_wake_obligation(
        &self,
        key: WakeObligationKey,
        observation_ticket: WakeObservationTicket,
    ) -> crate::Result<WakeObligationTicket> {
        let observed_at = Utc::now();
        let (ticket, should_signal) = {
            let mut obligations = self
                .wake_obligations
                .lock()
                .map_err(|_| anyhow!("startup backfill wake obligation lock poisoned"))?;
            if let Some(existing) = obligations.get_mut(&key.task)
                && existing.key == key
            {
                existing.observation_ticket = observation_ticket;
                (
                    existing.ticket,
                    !existing.in_flight && existing.next_due <= observed_at,
                )
            } else {
                let ticket = WakeObligationTicket {
                    task: key.task,
                    generation: self
                        .obligation_generation
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1),
                };
                obligations.insert(
                    key.task,
                    WakeObligation {
                        key,
                        next_due: observed_at,
                        attempt: 0,
                        last_warning_at: None,
                        observation_ticket,
                        ticket,
                        in_flight: false,
                    },
                );
                (ticket, true)
            }
        };
        if should_signal {
            self.signal();
        }
        Ok(ticket)
    }

    pub(super) fn claim_due_wake_obligations(
        &self,
        now: DateTime<Utc>,
    ) -> Vec<WakeObligationAttempt> {
        let Ok(mut obligations) = self.wake_obligations.lock() else {
            return Vec::new();
        };
        StartupBackfillTask::ordered_tasks()
            .iter()
            .filter_map(|task| {
                let obligation = obligations.get_mut(task)?;
                if obligation.in_flight || obligation.next_due > now {
                    return None;
                }
                obligation.in_flight = true;
                obligation.attempt = obligation.attempt.saturating_add(1);
                Some(WakeObligationAttempt {
                    ticket: obligation.ticket,
                    key: obligation.key.clone(),
                    attempt: obligation.attempt,
                })
            })
            .collect()
    }

    pub(super) fn complete_wake_obligation(
        &self,
        ticket: WakeObligationTicket,
    ) -> Option<WakeObservationTicket> {
        let mut obligations = self.wake_obligations.lock().ok()?;
        let current = obligations.get(&ticket.task)?;
        if current.ticket != ticket {
            return None;
        }
        let observation_ticket = current.observation_ticket;
        obligations.remove(&ticket.task);
        Some(observation_ticket)
    }

    pub(super) fn fail_wake_obligation(
        &self,
        ticket: WakeObligationTicket,
        now: DateTime<Utc>,
    ) -> Option<WakeObligationFailure> {
        let mut obligations = self.wake_obligations.lock().ok()?;
        let obligation = obligations.get_mut(&ticket.task)?;
        if obligation.ticket != ticket {
            return None;
        }
        obligation.in_flight = false;
        let base_delay_ms =
            WAKE_RETRY_BASE_DELAYS_MS[obligation.attempt.saturating_sub(1).min(4) as usize];
        let jitter_max_ms = base_delay_ms / 5;
        let jitter_ms = rand::thread_rng().gen_range(0..=jitter_max_ms);
        let next_due = now + chrono::Duration::milliseconds(base_delay_ms + jitter_ms);
        obligation.next_due = next_due;
        let should_warn = obligation
            .last_warning_at
            .is_none_or(|last| now - last >= chrono::Duration::seconds(WAKE_WARNING_INTERVAL_SECS));
        if should_warn {
            obligation.last_warning_at = Some(now);
        }
        Some(WakeObligationFailure {
            next_due,
            should_warn,
            attempt: obligation.attempt,
        })
    }

    pub(super) fn cancel_wake_obligation(&self, task: StartupBackfillTask) {
        if let Ok(mut obligations) = self.wake_obligations.lock() {
            obligations.remove(&task);
        }
        self.clear_observation(task);
    }

    pub(super) fn pending_wake_obligation_count(&self) -> usize {
        self.wake_obligations
            .lock()
            .map(|obligations| obligations.len())
            .unwrap_or_default()
    }

    pub(super) fn next_wake_obligation_due(&self) -> Option<DateTime<Utc>> {
        self.wake_obligations.lock().ok().and_then(|obligations| {
            obligations
                .values()
                .map(|obligation| obligation.next_due)
                .min()
        })
    }

    fn signal(&self) {
        self.wake_generation.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    pub(super) fn generation(&self) -> u64 {
        self.wake_generation.load(Ordering::Acquire)
    }

    pub(super) fn drain_woken_tasks(&self) -> Vec<StartupBackfillTask> {
        let Ok(mut tasks) = self.woken_tasks.lock() else {
            return Vec::new();
        };
        StartupBackfillTask::ordered_tasks()
            .iter()
            .copied()
            .filter(|task| tasks.remove(task))
            .collect()
    }

    pub(super) fn drain_due_tasks(&self, now: DateTime<Utc>) -> Vec<StartupBackfillTask> {
        let Ok(mut next_due) = self.next_due.lock() else {
            return Vec::new();
        };
        let due_tasks = StartupBackfillTask::ordered_tasks()
            .iter()
            .copied()
            .filter(|task| next_due.get(task).is_some_and(|due| *due <= now))
            .collect::<Vec<_>>();
        for task in &due_tasks {
            next_due.remove(task);
        }
        drop(next_due);
        for task in &due_tasks {
            self.publish_performance(*task);
        }
        self.due_dispatch_count
            .fetch_add(due_tasks.len() as u64, Ordering::Relaxed);
        due_tasks
    }

    pub(super) fn record_next_due(&self, task: StartupBackfillTask, due: DateTime<Utc>) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.insert(task, due);
        }
        self.publish_performance(task);
    }

    pub(super) fn clear_next_due(&self, task: StartupBackfillTask) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.remove(&task);
        }
        self.publish_performance(task);
    }

    pub(super) fn clear_pending(&self, task: StartupBackfillTask) {
        if let Ok(mut tasks) = self.woken_tasks.lock() {
            tasks.remove(&task);
        }
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.remove(&task);
        }
        if let Ok(mut tasks) = self.deferred_tasks.lock() {
            tasks.remove(&task);
        }
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
            tasks.remove(&task);
        }
        if let Ok(mut obligations) = self.wake_obligations.lock() {
            obligations.remove(&task);
        }
        self.clear_observation(task);
        self.publish_performance(task);
        self.notify.notify_waiters();
    }

    fn clear_observation(&self, task: StartupBackfillTask) {
        if let Ok(mut pending_since) = self.pending_since.lock() {
            pending_since.remove(&task);
        }
    }

    pub(super) fn confirm_observation(&self, ticket: WakeObservationTicket) {
        if let Ok(mut pending_since) = self.pending_since.lock()
            && pending_since
                .get(&ticket.task)
                .is_some_and(|observation| observation.ticket == ticket)
        {
            pending_since.remove(&ticket.task);
        }
        self.publish_performance(ticket.task);
    }

    pub(super) fn mark_pressure_deferred(&self, task: StartupBackfillTask) {
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
            tasks.insert(task);
        }
    }

    pub(super) fn defer_for_pressure(&self, task: StartupBackfillTask, retry_at: DateTime<Utc>) {
        self.mark_pressure_deferred(task);
        self.record_next_due(task, retry_at);
    }

    pub(super) fn defer_for_background_busy(
        &self,
        task: StartupBackfillTask,
        retry_at: DateTime<Utc>,
    ) {
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
            tasks.remove(&task);
        }
        self.record_next_due(task, retry_at);
    }

    pub(super) fn requeue_after_dispatch_gap(
        &self,
        tasks: &[StartupBackfillTask],
        retry_at: DateTime<Utc>,
    ) {
        for task in tasks {
            self.defer_for_background_busy(*task, retry_at);
        }
    }

    pub(super) fn take_pressure_deferred_tasks(
        &self,
        now: DateTime<Utc>,
    ) -> Vec<StartupBackfillTask> {
        self.take_pressure_deferred_tasks_matching(now, None)
    }

    pub(super) fn take_pressure_deferred_tasks_matching(
        &self,
        now: DateTime<Utc>,
        allowed_tasks: Option<&[StartupBackfillTask]>,
    ) -> Vec<StartupBackfillTask> {
        let Ok(mut pressure_tasks) = self.pressure_deferred_tasks.lock() else {
            return Vec::new();
        };
        let Ok(mut next_due) = self.next_due.lock() else {
            return Vec::new();
        };
        let tasks = StartupBackfillTask::ordered_tasks()
            .iter()
            .copied()
            .filter(|task| {
                allowed_tasks.is_none_or(|allowed| allowed.contains(task))
                    && pressure_tasks.contains(task)
                    && next_due.get(task).is_some_and(|deadline| *deadline <= now)
            })
            .collect::<Vec<_>>();
        for task in &tasks {
            pressure_tasks.remove(task);
            next_due.remove(task);
        }
        drop(next_due);
        drop(pressure_tasks);
        for task in &tasks {
            self.publish_performance(*task);
        }
        tasks
    }

    pub(super) fn record_task_result(
        &self,
        task: StartupBackfillTask,
        failed: bool,
        deferred: bool,
    ) {
        self.record_task_result_with_actionable(task, failed, deferred, false);
    }

    pub(super) fn record_task_result_with_actionable(
        &self,
        task: StartupBackfillTask,
        failed: bool,
        deferred: bool,
        _actionable: bool,
    ) {
        if failed {
            self.failure_count.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut tasks) = self.failed_tasks.lock() {
                tasks.insert(task);
            }
            if let Ok(mut tasks) = self.deferred_tasks.lock() {
                tasks.remove(&task);
            }
            if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
                tasks.remove(&task);
            }
            self.publish_performance(task);
            return;
        }

        if deferred {
            let pressure_deferred = self
                .pressure_deferred_tasks
                .lock()
                .map(|tasks| tasks.contains(&task))
                .unwrap_or(true);
            if pressure_deferred {
                self.pressure_defer_count.fetch_add(1, Ordering::Relaxed);
            }
            let has_active_failure = self
                .failed_tasks
                .lock()
                .map(|tasks| tasks.contains(&task))
                .unwrap_or(true);
            if has_active_failure {
                self.publish_performance(task);
                return;
            }
            if let Ok(mut tasks) = self.deferred_tasks.lock() {
                tasks.insert(task);
            }
            if let Ok(mut tasks) = self.failed_tasks.lock() {
                tasks.remove(&task);
            }
            self.publish_performance(task);
            return;
        }

        if let Ok(mut tasks) = self.deferred_tasks.lock() {
            tasks.remove(&task);
        }
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
            tasks.remove(&task);
        }
        if let Ok(mut tasks) = self.failed_tasks.lock() {
            tasks.remove(&task);
        }
        self.publish_performance(task);
    }

    pub(super) fn record_noop_suppressed(&self) {
        self.noop_suppressed_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn health_snapshot(&self) -> StartupBackfillHealthSnapshot {
        let woken_task_count = self
            .woken_tasks
            .lock()
            .map(|tasks| tasks.len() as u64)
            .unwrap_or_default();
        let scheduled_task_count = self
            .next_due
            .lock()
            .map(|tasks| tasks.len() as u64)
            .unwrap_or_default();
        let deferred_task_count = self
            .deferred_tasks
            .lock()
            .map(|tasks| tasks.len() as u64)
            .unwrap_or_default();
        let failed_task_count = self
            .failed_tasks
            .lock()
            .map(|tasks| tasks.len() as u64)
            .unwrap_or_default();
        let state = if failed_task_count > 0 {
            "degraded"
        } else if deferred_task_count > 0 {
            "deferred"
        } else {
            "healthy"
        };

        StartupBackfillHealthSnapshot {
            state: state.to_string(),
            wake_generation: self.generation(),
            wake_count: self.wake_count.load(Ordering::Relaxed),
            due_dispatch_count: self.due_dispatch_count.load(Ordering::Relaxed),
            noop_suppressed_count: self.noop_suppressed_count.load(Ordering::Relaxed),
            pressure_defer_count: self.pressure_defer_count.load(Ordering::Relaxed),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            woken_task_count,
            scheduled_task_count,
            deferred_task_count,
            failed_task_count,
        }
    }

    pub(super) fn next_due(&self) -> Option<DateTime<Utc>> {
        self.next_due
            .lock()
            .ok()
            .and_then(|next_due| next_due.values().min().cloned())
    }

    pub(super) fn next_due_for(&self, task: StartupBackfillTask) -> Option<DateTime<Utc>> {
        self.next_due
            .lock()
            .ok()
            .and_then(|next_due| next_due.get(&task).cloned())
    }

    pub(super) fn has_pending_wake(&self, task: StartupBackfillTask) -> bool {
        self.woken_tasks
            .lock()
            .map(|tasks| tasks.contains(&task))
            .unwrap_or(true)
    }

    pub(super) fn has_future_pressure_deadline(
        &self,
        task: StartupBackfillTask,
        now: DateTime<Utc>,
    ) -> bool {
        let pressure_deferred = self
            .pressure_deferred_tasks
            .lock()
            .map(|tasks| tasks.contains(&task))
            .unwrap_or(true);
        pressure_deferred
            && self
                .next_due_for(task)
                .is_some_and(|deadline| deadline > now)
    }

    fn publish_performance(&self, task: StartupBackfillTask) {
        let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) else {
            return;
        };
        let pending_since = self
            .pending_since
            .lock()
            .ok()
            .and_then(|pending| pending.get(&task).map(|observation| observation.since));
        let next_due = self.next_due_for(task);
        metrics.set_performance(
            PerformanceGauge::Wake(task, WakeState::Pending),
            if pending_since.is_some() { 1.0 } else { 0.0 },
        );
        if let Some(oldest) = pending_since {
            metrics.set_performance(
                PerformanceGauge::Wake(task, WakeState::OldestTimestamp),
                oldest.timestamp_millis().max(0) as f64 / 1_000.0,
            );
        } else {
            metrics.set_performance(
                PerformanceGauge::Wake(task, WakeState::OldestTimestamp),
                0.0,
            );
        }
        metrics.set_performance(
            PerformanceGauge::Wake(task, WakeState::NextDueTimestamp),
            next_due
                .map(|due| due.timestamp_millis().max(0) as f64 / 1_000.0)
                .unwrap_or(0.0),
        );
    }

    pub(super) async fn wait_for_wake(&self, observed_generation: u64) {
        loop {
            let notified = self.notify.notified();
            if self.generation() != observed_generation {
                return;
            }
            notified.await;
        }
    }
}

pub(super) async fn observe_wake_attempt<T>(
    task: StartupBackfillTask,
    future: impl std::future::Future<Output = crate::Result<(T, bool)>>,
) -> crate::Result<T> {
    let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) else {
        return future.await.map(|(value, _)| value);
    };
    let mut observation = WakeAttemptObservation {
        metrics,
        task,
        outcome: WakeOutcome::Cancelled,
    };
    let result = future.await;
    observation.outcome = match &result {
        Ok((_, committed)) => {
            if *committed {
                WakeOutcome::Committed
            } else {
                WakeOutcome::Noop
            }
        }
        Err(error) => crate::maintenance_store::store_write_outcome(error),
    };
    result.map(|(value, _)| value)
}

struct WakeAttemptObservation {
    metrics: Arc<crate::observability::ObservabilityRuntime>,
    task: StartupBackfillTask,
    outcome: WakeOutcome,
}

impl Drop for WakeAttemptObservation {
    fn drop(&mut self) {
        self.metrics
            .count_performance(PerformanceCounter::WakeAttempt(self.task, self.outcome), 1);
    }
}

pub(super) fn confirm_scheduler_wake_observation(
    scheduler: &StartupBackfillScheduler,
    ticket: WakeObservationTicket,
) {
    scheduler.confirm_observation(ticket);
}

pub(super) fn record_scheduler_wake_confirmation(
    scheduler: &StartupBackfillScheduler,
    ticket: WakeObservationTicket,
) {
    if let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) {
        metrics.count_performance(
            PerformanceCounter::WakeAttempt(ticket.task, WakeOutcome::Committed),
            1,
        );
    }
    confirm_scheduler_wake_observation(scheduler, ticket);
}
