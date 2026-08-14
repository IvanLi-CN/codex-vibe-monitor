use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use sqlx::{Pool, Sqlite, SqliteConnection};
use tokio::{
    sync::{Mutex, RwLock, mpsc, oneshot},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, sleep, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::*;
use crate::terminal_journal::{
    TerminalJournal, TerminalJournalAppendOutcome, TerminalJournalDurabilityMode,
    TerminalJournalStats,
};

pub(crate) const SQLITE_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(20);
pub(crate) const SQLITE_P2_COALESCE_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const SQLITE_BATCH_MAX_ROWS: usize = 32;
pub(crate) const SQLITE_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const SQLITE_BATCH_MAX_AGE: Duration = Duration::from_secs(5);
pub(crate) const SQLITE_BATCH_STALE_WARN_AGE: Duration = Duration::from_secs(30);
pub(crate) const SQLITE_BATCH_CHANNEL_CAPACITY: usize = 10_000;
const SQLITE_SHUTDOWN_DRAIN_DEADLINE: Duration = Duration::from_secs(5);
const SQLITE_P1_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];

#[derive(Debug, Default)]
struct P1RetryState {
    generation: usize,
    due_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P2WakeReason {
    CoalescedDeadline,
    PressureCooldownElapsed,
    BackgroundEligible,
    LockRetry,
}

impl P2WakeReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::CoalescedDeadline => "coalesced_deadline",
            Self::PressureCooldownElapsed => "pressure_cooldown_elapsed",
            Self::BackgroundEligible => "background_eligible",
            Self::LockRetry => "lock_retry",
        }
    }
}

#[derive(Debug, Default)]
struct P2ScheduleState {
    generation: usize,
    due_at: Option<Instant>,
    wake_reason: Option<P2WakeReason>,
    deferred_since: Option<Instant>,
}

impl P2ScheduleState {
    fn arm_if_idle(&mut self, now: Instant) {
        if self.due_at.is_none() && self.wake_reason.is_none() {
            self.due_at = Some(now + SQLITE_P2_COALESCE_INTERVAL);
            self.wake_reason = Some(P2WakeReason::CoalescedDeadline);
            self.deferred_since.get_or_insert(now);
        }
    }

    fn ready(&self, now: Instant) -> bool {
        self.due_at.is_some_and(|due_at| now >= due_at)
    }

    fn defer_pressure(&mut self, delay: Duration, reason: P2WakeReason) {
        self.due_at = Some(Instant::now() + delay.max(Duration::from_millis(1)));
        self.wake_reason = Some(reason);
        self.deferred_since.get_or_insert_with(Instant::now);
    }

    fn defer_until_background_eligible(&mut self) {
        self.due_at = None;
        self.wake_reason = Some(P2WakeReason::BackgroundEligible);
        self.deferred_since.get_or_insert_with(Instant::now);
    }

    fn wake_background_eligible(&mut self) {
        self.due_at = Some(Instant::now());
        self.wake_reason = Some(P2WakeReason::BackgroundEligible);
    }

    fn failed(&mut self, transaction_seed: u64) -> Duration {
        let base = SQLITE_P1_RETRY_DELAYS[self.generation.min(SQLITE_P1_RETRY_DELAYS.len() - 1)];
        self.generation = self.generation.saturating_add(1);
        let jitter_ceiling_ms = (base.as_millis() as u64 / 10).max(1);
        let delay =
            base.saturating_add(Duration::from_millis(transaction_seed % jitter_ceiling_ms));
        self.due_at = Some(Instant::now() + delay);
        self.wake_reason = Some(P2WakeReason::LockRetry);
        self.deferred_since.get_or_insert_with(Instant::now);
        delay
    }

    fn succeeded(&mut self) {
        *self = Self::default();
    }

    fn next_attempt_in_ms(&self) -> u64 {
        self.due_at
            .map(|due_at| due_at.saturating_duration_since(Instant::now()).as_millis() as u64)
            .unwrap_or_default()
    }

    fn deferred_age_ms(&self) -> u64 {
        self.deferred_since
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or_default()
    }
}

async fn wait_for_p2_deadline(due_at: Option<Instant>) {
    match due_at {
        Some(due_at) => sleep(due_at.saturating_duration_since(Instant::now())).await,
        None => std::future::pending::<()>().await,
    }
}

fn p2_deadline_ready(
    pending: &PendingBatch,
    p2_schedule: &P2ScheduleState,
    queued_p1_count: &AtomicUsize,
    p1_retry: &P1RetryState,
) -> bool {
    pending.has_p2()
        && p2_schedule.ready(Instant::now())
        && queued_p1_count.load(Ordering::SeqCst) == 0
        && p1_retry.ready(Instant::now())
}

fn is_p1_terminal_write(write: &SqliteBatchWrite) -> bool {
    matches!(write, SqliteBatchWrite::TerminalInvocation(_))
}

fn decrement_queued_p1_count(queued_p1_count: &AtomicUsize) {
    let _ = queued_p1_count.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
        count.checked_sub(1)
    });
}

fn drain_queued_writes_before_dispatch(
    write_receiver: &mut mpsc::Receiver<SqliteBatchWrite>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    p2_schedule: &mut P2ScheduleState,
    max_messages: usize,
    queued_p1_count: &AtomicUsize,
) {
    drain_queued_batch_writes(
        write_receiver,
        pending,
        accounting,
        max_messages,
        queued_p1_count,
    );
    if pending.has_p2() {
        p2_schedule.arm_if_idle(Instant::now());
        accounting.update_p2_schedule(p2_schedule);
    }
}

fn drain_queued_writes_before_p2_dispatch(
    write_receiver: &mut mpsc::Receiver<SqliteBatchWrite>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    p2_schedule: &mut P2ScheduleState,
    queued_p1_count: &AtomicUsize,
) {
    // A snapshot keeps the priority scan bounded even while P2 producers continue writing.
    let queued_messages = write_receiver.len().min(SQLITE_BATCH_MAX_ROWS);
    drain_queued_writes_before_dispatch(
        write_receiver,
        pending,
        accounting,
        p2_schedule,
        queued_messages,
        queued_p1_count,
    );
}

impl P1RetryState {
    fn ready(&self, now: Instant) -> bool {
        self.due_at.is_none_or(|due_at| now >= due_at)
    }

    fn failed(&mut self, transaction_seed: u64) -> Duration {
        let base = SQLITE_P1_RETRY_DELAYS[self.generation.min(SQLITE_P1_RETRY_DELAYS.len() - 1)];
        self.generation = self.generation.saturating_add(1);
        let jitter_ceiling_ms = (base.as_millis() as u64 / 10).max(1);
        let jitter_ms = transaction_seed % jitter_ceiling_ms;
        let delay = base.saturating_add(Duration::from_millis(jitter_ms));
        self.due_at = Some(Instant::now() + delay);
        delay
    }

    fn succeeded(&mut self) {
        self.generation = 0;
        self.due_at = None;
    }
}

fn is_sqlite_lock_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database is busy")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingQueueInvariantViolation {
    pub(crate) operation: String,
    pub(crate) counter: String,
    pub(crate) expected_value: usize,
    pub(crate) actual_value: usize,
    pub(crate) expected_bytes: usize,
    pub(crate) actual_bytes: usize,
    pub(crate) pending_depth: usize,
    pub(crate) pending_bytes: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingQueueAccountingSnapshot {
    pub(crate) state: String,
    pub(crate) pending_depth: usize,
    pub(crate) pending_bytes: usize,
    pub(crate) transfer_bytes: usize,
    pub(crate) retry_count: u64,
    pub(crate) p2_flush_attempt_count: u64,
    pub(crate) p2_pressure_defer_count: u64,
    pub(crate) p2_lock_retry_count: u64,
    pub(crate) p2_next_attempt_in_ms: u64,
    pub(crate) p2_deferred_age_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) p2_wake_reason: Option<String>,
    pub(crate) invariant_violation_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) degraded_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_invariant_violation: Option<PendingQueueInvariantViolation>,
}

#[derive(Debug, Default)]
pub(crate) struct PendingQueueAccounting {
    pending_depth: AtomicUsize,
    pending_bytes: AtomicUsize,
    transfer_bytes: AtomicUsize,
    retry_count: AtomicU64,
    p2_flush_attempt_count: AtomicU64,
    p2_pressure_defer_count: AtomicU64,
    p2_lock_retry_count: AtomicU64,
    p2_next_attempt_in_ms: AtomicU64,
    p2_deferred_age_ms: AtomicU64,
    p2_wake_reason: std::sync::Mutex<Option<String>>,
    invariant_violation_count: AtomicU64,
    last_invariant_violation: std::sync::Mutex<Option<PendingQueueInvariantViolation>>,
}

impl PendingQueueAccounting {
    pub(crate) fn enqueue(&self, bytes: usize) {
        self.add(&self.pending_bytes, bytes, "enqueue", "pending_bytes");
        self.add(&self.pending_depth, 1, "enqueue", "pending_depth");
    }

    pub(crate) fn rollback_enqueue(&self, bytes: usize) {
        self.subtract(
            &self.pending_depth,
            1,
            "sender_failure_rollback",
            "pending_depth",
        );
        self.subtract(
            &self.pending_bytes,
            bytes,
            "sender_failure_rollback",
            "pending_bytes",
        );
    }

    pub(crate) fn replace_batch(
        &self,
        admitted_depth: usize,
        retained_depth: usize,
        admitted_bytes: usize,
        retained_bytes: usize,
    ) {
        self.replace_for(
            "batch_replacement",
            admitted_depth,
            retained_depth,
            admitted_bytes,
            retained_bytes,
        );
    }

    pub(crate) fn transfer_p1_to_p2(&self, bytes: usize) {
        self.add(
            &self.transfer_bytes,
            bytes,
            "p1_to_p2_transfer",
            "transfer_bytes",
        );
    }

    pub(crate) fn retry_deferred(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }

    fn p2_attempted(&self) {
        self.p2_flush_attempt_count.fetch_add(1, Ordering::Relaxed);
    }

    fn p2_pressure_deferred(&self) {
        self.p2_pressure_defer_count.fetch_add(1, Ordering::Relaxed);
    }

    fn p2_lock_retried(&self) {
        self.p2_lock_retry_count.fetch_add(1, Ordering::Relaxed);
    }

    fn update_p2_schedule(&self, schedule: &P2ScheduleState) {
        self.p2_next_attempt_in_ms
            .store(schedule.next_attempt_in_ms(), Ordering::Relaxed);
        self.p2_deferred_age_ms
            .store(schedule.deferred_age_ms(), Ordering::Relaxed);
        if let Ok(mut wake_reason) = self.p2_wake_reason.lock() {
            *wake_reason = schedule
                .wake_reason
                .map(|reason| reason.as_str().to_string());
        }
    }

    pub(crate) fn complete(
        &self,
        submitted_depth: usize,
        retained_depth: usize,
        submitted_bytes: usize,
        retained_bytes: usize,
    ) {
        self.replace_for(
            "completion",
            submitted_depth,
            retained_depth,
            submitted_bytes,
            retained_bytes,
        );
    }

    pub(crate) fn release(&self, depth: usize, bytes: usize) {
        self.subtract(&self.pending_depth, depth, "release", "pending_depth");
        self.subtract(&self.pending_bytes, bytes, "release", "pending_bytes");
    }

    fn clear_after_shutdown(&self) -> (usize, usize) {
        let pending_depth = self.pending_depth.swap(0, Ordering::SeqCst);
        let pending_bytes = self.pending_bytes.swap(0, Ordering::SeqCst);
        (pending_depth, pending_bytes)
    }

    pub(crate) fn snapshot(&self) -> PendingQueueAccountingSnapshot {
        let last_invariant_violation = self
            .last_invariant_violation
            .lock()
            .ok()
            .and_then(|violation| violation.clone());
        let invariant_violation_count = self.invariant_violation_count.load(Ordering::Relaxed);
        let degraded_reason = last_invariant_violation.as_ref().map(|violation| {
            format!(
                "{} {} invariant: expected {}, actual {}",
                violation.operation,
                violation.counter,
                violation.expected_value,
                violation.actual_value
            )
        });
        PendingQueueAccountingSnapshot {
            state: if invariant_violation_count == 0 {
                "healthy".to_string()
            } else {
                "degraded".to_string()
            },
            pending_depth: self.pending_depth.load(Ordering::Relaxed),
            pending_bytes: self.pending_bytes.load(Ordering::Relaxed),
            transfer_bytes: self.transfer_bytes.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            p2_flush_attempt_count: self.p2_flush_attempt_count.load(Ordering::Relaxed),
            p2_pressure_defer_count: self.p2_pressure_defer_count.load(Ordering::Relaxed),
            p2_lock_retry_count: self.p2_lock_retry_count.load(Ordering::Relaxed),
            p2_next_attempt_in_ms: self.p2_next_attempt_in_ms.load(Ordering::Relaxed),
            p2_deferred_age_ms: self.p2_deferred_age_ms.load(Ordering::Relaxed),
            p2_wake_reason: self
                .p2_wake_reason
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            invariant_violation_count,
            degraded_reason,
            last_invariant_violation,
        }
    }

    fn replace_for(
        &self,
        operation: &'static str,
        old_depth: usize,
        new_depth: usize,
        old_bytes: usize,
        new_bytes: usize,
    ) {
        self.subtract(&self.pending_depth, old_depth, operation, "pending_depth");
        self.add(&self.pending_depth, new_depth, operation, "pending_depth");
        self.subtract(&self.pending_bytes, old_bytes, operation, "pending_bytes");
        self.add(&self.pending_bytes, new_bytes, operation, "pending_bytes");
    }

    fn add(
        &self,
        counter: &AtomicUsize,
        amount: usize,
        operation: &'static str,
        counter_name: &'static str,
    ) {
        if amount == 0 {
            return;
        }
        let previous = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(amount))
            })
            .expect("accounting update always returns a value");
        if previous.checked_add(amount).is_none() {
            self.record_invariant(operation, counter_name, amount, previous);
        }
    }

    fn subtract(
        &self,
        counter: &AtomicUsize,
        amount: usize,
        operation: &'static str,
        counter_name: &'static str,
    ) {
        if amount == 0 {
            return;
        }
        let previous = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(amount))
            })
            .expect("accounting update always returns a value");
        if previous < amount {
            self.record_invariant(operation, counter_name, amount, previous);
        }
    }

    fn record_invariant(
        &self,
        operation: &'static str,
        counter: &'static str,
        expected_value: usize,
        actual_value: usize,
    ) {
        let pending_depth = self.pending_depth.load(Ordering::Relaxed);
        let pending_bytes = self.pending_bytes.load(Ordering::Relaxed);
        let (expected_bytes, actual_bytes) = if counter == "pending_bytes" {
            (expected_value, actual_value)
        } else {
            (pending_bytes, pending_bytes)
        };
        let violation = PendingQueueInvariantViolation {
            operation: operation.to_string(),
            counter: counter.to_string(),
            expected_value,
            actual_value,
            expected_bytes,
            actual_bytes,
            pending_depth,
            pending_bytes,
        };
        let invariant_violation_count = self
            .invariant_violation_count
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if let Ok(mut last) = self.last_invariant_violation.lock() {
            *last = Some(violation.clone());
        }
        warn!(
            accounting_invariant = true,
            operation,
            counter,
            expected_value,
            actual_value,
            pending_depth,
            pending_bytes,
            invariant_violation_count,
            "sqlite pending queue accounting invariant violated"
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FlushReason {
    RowLimit,
    Interval,
    MaxAge,
    Barrier,
    Shutdown,
}

impl FlushReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::RowLimit => "row_limit",
            Self::Interval => "interval",
            Self::MaxAge => "max_age",
            Self::Barrier => "barrier",
            Self::Shutdown => "shutdown",
        }
    }

    fn bypass_pressure_gate(self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedAttemptProgress {
    pub(crate) attempt_id: i64,
    pub(crate) pending_status: &'static str,
    pub(crate) phase: String,
    pub(crate) connect_latency_ms: Option<f64>,
    pub(crate) first_byte_latency_ms: Option<f64>,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedInvocationDerivedWrites {
    pub(crate) invocation_id: i64,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) terminal_overlay_key: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedTerminalInvocationWrite {
    pub(crate) record: ProxyCaptureRecord,
    pub(crate) capture_started: Option<Instant>,
    pub(crate) raw_capture: bool,
    pub(crate) dashboard_terminal_sequence: Option<u64>,
    pub(crate) terminal_projection_event_ids: Vec<u64>,
    // Computed from the already-materialized terminal record before P1 admission. This keeps
    // event-driven repair discovery out of the SQLite transaction and avoids another payload parse.
    pub(crate) startup_backfill_tasks: Vec<StartupBackfillTask>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalEnqueueOutcome {
    pub(crate) enqueued: bool,
    pub(crate) durability_mode: TerminalJournalDurabilityMode,
    pub(crate) journal_sequence: Option<u64>,
    pub(crate) journal_pending_records: usize,
    pub(crate) journal_pending_bytes: u64,
}

impl BatchedTerminalInvocationWrite {
    fn key(&self) -> String {
        format!(
            "{}\n{}\n{}",
            self.record.invoke_id, self.record.occurred_at, self.raw_capture
        )
    }

    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.record.estimated_memory_bytes())
            .saturating_add(
                self.terminal_projection_event_ids
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.startup_backfill_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedAccountSelectedTouch {
    pub(crate) account_id: i64,
    pub(crate) selected_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedSystemTaskFinish {
    pub(crate) run_id: i64,
    pub(crate) task_kind: SystemTaskKind,
    pub(crate) trigger_kind: String,
    pub(crate) status: SystemTaskStatus,
    pub(crate) summary: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) finished_at: String,
    pub(crate) duration_ms: i64,
}

#[derive(Debug)]
pub(crate) enum SqliteBatchWrite {
    TerminalInvocation(BatchedTerminalInvocationWrite),
    AttemptProgress(BatchedAttemptProgress),
    InvocationDerived(BatchedInvocationDerivedWrites),
    AccountSelectedTouch(BatchedAccountSelectedTouch),
    SystemTaskFinish(BatchedSystemTaskFinish),
    #[cfg(test)]
    StartupBackfillWake(StartupBackfillTask),
}

pub(crate) enum SqliteBatchWriterControl {
    FlushNow {
        queued_depth_snapshot: usize,
        responder: oneshot::Sender<Result<(), String>>,
    },
    Shutdown {
        queued_depth_snapshot: usize,
        responder: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct PendingBatch {
    terminal_invocations: BTreeMap<String, BatchedTerminalInvocationWrite>,
    attempt_progress: HashMap<i64, BatchedAttemptProgress>,
    invocation_derived: BTreeMap<i64, BatchedInvocationDerivedWrites>,
    account_selected_touches: HashMap<i64, BatchedAccountSelectedTouch>,
    system_task_finishes: HashMap<i64, BatchedSystemTaskFinish>,
    startup_backfill_wake_tasks: Vec<StartupBackfillTask>,
    enqueued_rows: usize,
    coalesced_rows: usize,
    estimated_bytes: usize,
    terminal_estimated_bytes: usize,
    oldest_at: Option<Instant>,
    retained_for_retry: bool,
}

impl PendingBatch {
    fn add_estimate(&mut self, bytes: usize, is_terminal: bool) {
        self.estimated_bytes = self.estimated_bytes.saturating_add(bytes);
        if is_terminal {
            self.terminal_estimated_bytes = self.terminal_estimated_bytes.saturating_add(bytes);
        }
    }

    fn replace_estimate(&mut self, old_bytes: usize, new_bytes: usize, is_terminal: bool) {
        self.estimated_bytes = self
            .estimated_bytes
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if is_terminal {
            self.terminal_estimated_bytes = self
                .terminal_estimated_bytes
                .saturating_sub(old_bytes)
                .saturating_add(new_bytes);
        }
    }

    fn recalculate_estimates(&mut self) {
        self.estimated_bytes = self
            .terminal_invocations
            .values()
            .map(BatchedTerminalInvocationWrite::estimated_memory_bytes)
            .sum::<usize>()
            .saturating_add(
                self.attempt_progress
                    .values()
                    .map(estimated_attempt_progress_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.invocation_derived
                    .values()
                    .map(estimated_invocation_derived_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.account_selected_touches
                    .values()
                    .map(estimated_account_selected_touch_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.system_task_finishes
                    .values()
                    .map(estimated_system_task_finish_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.startup_backfill_wake_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            );
        self.terminal_estimated_bytes = self
            .terminal_invocations
            .values()
            .map(BatchedTerminalInvocationWrite::estimated_memory_bytes)
            .sum();
    }

    fn is_empty(&self) -> bool {
        self.terminal_invocations.is_empty()
            && self.attempt_progress.is_empty()
            && self.invocation_derived.is_empty()
            && self.account_selected_touches.is_empty()
            && self.system_task_finishes.is_empty()
            && self.startup_backfill_wake_tasks.is_empty()
    }

    fn has_p2(&self) -> bool {
        !self.attempt_progress.is_empty()
            || !self.invocation_derived.is_empty()
            || !self.account_selected_touches.is_empty()
            || !self.system_task_finishes.is_empty()
            || !self.startup_backfill_wake_tasks.is_empty()
    }

    fn logical_rows(&self) -> usize {
        self.terminal_invocations.len()
            + self.attempt_progress.len()
            + self.invocation_derived.len()
            + self.account_selected_touches.len()
            + self.system_task_finishes.len()
            + usize::from(!self.startup_backfill_wake_tasks.is_empty())
    }

    fn age(&self) -> Duration {
        self.oldest_at
            .map(|oldest| oldest.elapsed())
            .unwrap_or_default()
    }

    fn push(&mut self, write: SqliteBatchWrite) -> usize {
        let now = Instant::now();
        let write_bytes = write.estimated_memory_bytes();
        self.oldest_at.get_or_insert(now);
        self.enqueued_rows += 1;
        match write {
            SqliteBatchWrite::TerminalInvocation(terminal) => {
                let key = terminal.key();
                let estimate_change = match self.terminal_invocations.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(terminal);
                        (0, write_bytes)
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let old_bytes = entry.get().estimated_memory_bytes();
                        let preserved_sequence = terminal
                            .dashboard_terminal_sequence
                            .or(entry.get().dashboard_terminal_sequence);
                        let mut terminal = terminal;
                        terminal.dashboard_terminal_sequence = preserved_sequence;
                        terminal
                            .terminal_projection_event_ids
                            .extend(entry.get().terminal_projection_event_ids.iter().copied());
                        terminal.terminal_projection_event_ids.sort_unstable();
                        terminal.terminal_projection_event_ids.dedup();
                        for task in entry.get().startup_backfill_tasks.iter().copied() {
                            if !terminal.startup_backfill_tasks.contains(&task) {
                                terminal.startup_backfill_tasks.push(task);
                            }
                        }
                        let new_bytes = terminal.estimated_memory_bytes();
                        entry.insert(terminal);
                        self.coalesced_rows += 1;
                        (old_bytes, new_bytes)
                    }
                };
                if estimate_change.0 == 0 {
                    self.add_estimate(estimate_change.1, true);
                } else {
                    self.replace_estimate(estimate_change.0, estimate_change.1, true);
                }
                estimate_change.0
            }
            SqliteBatchWrite::AttemptProgress(progress) => {
                let old = self.attempt_progress.insert(progress.attempt_id, progress);
                if let Some(old) = old {
                    let old_bytes = estimated_attempt_progress_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            SqliteBatchWrite::InvocationDerived(derived) => {
                let old = self
                    .invocation_derived
                    .insert(derived.invocation_id, derived);
                if let Some(old) = old {
                    let old_bytes = estimated_invocation_derived_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            SqliteBatchWrite::AccountSelectedTouch(touch) => {
                match self.account_selected_touches.entry(touch.account_id) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(touch);
                        self.add_estimate(write_bytes, false);
                        0
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let existing = entry.get_mut();
                        let old_bytes = estimated_account_selected_touch_memory_bytes(existing);
                        if existing.selected_at < touch.selected_at {
                            *existing = touch;
                            self.replace_estimate(old_bytes, write_bytes, false);
                            self.coalesced_rows += 1;
                            old_bytes
                        } else {
                            self.coalesced_rows += 1;
                            write_bytes
                        }
                    }
                }
            }
            SqliteBatchWrite::SystemTaskFinish(finish) => {
                let old = self.system_task_finishes.insert(finish.run_id, finish);
                if let Some(old) = old {
                    let old_bytes = estimated_system_task_finish_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            #[cfg(test)]
            SqliteBatchWrite::StartupBackfillWake(task) => {
                if self.startup_backfill_wake_tasks.contains(&task) {
                    self.coalesced_rows += 1;
                    write_bytes
                } else {
                    self.startup_backfill_wake_tasks.push(task);
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
        }
    }

    fn push_accounted(&mut self, write: SqliteBatchWrite, accounting: &PendingQueueAccounting) {
        let accounted_depth_before = self.logical_rows().saturating_add(1);
        let accounted_before = self
            .estimated_memory_bytes()
            .saturating_add(write.estimated_memory_bytes());
        self.push(write);
        accounting.replace_batch(
            accounted_depth_before,
            self.logical_rows(),
            accounted_before,
            self.estimated_memory_bytes(),
        );
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    fn take_p1_terminals(&mut self) -> Self {
        let terminal_invocations = std::mem::take(&mut self.terminal_invocations);
        if terminal_invocations.is_empty() {
            return Self::default();
        }
        let estimated_bytes = self.terminal_estimated_bytes;
        self.estimated_bytes = self.estimated_bytes.saturating_sub(estimated_bytes);
        self.terminal_estimated_bytes = self
            .terminal_estimated_bytes
            .saturating_sub(estimated_bytes);
        if !self.has_p2() {
            self.oldest_at = None;
        }
        Self {
            enqueued_rows: terminal_invocations.len(),
            estimated_bytes,
            terminal_estimated_bytes: estimated_bytes,
            terminal_invocations,
            oldest_at: self.oldest_at,
            ..Self::default()
        }
    }

    fn take_p1_terminal_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
        if self.terminal_invocations.is_empty() || max_rows == 0 || max_bytes == 0 {
            return Self::default();
        }
        let mut selected_keys = Vec::new();
        let mut selected_bytes = 0_usize;
        for (key, terminal) in &self.terminal_invocations {
            let bytes = terminal.estimated_memory_bytes();
            if selected_keys.len() >= max_rows {
                break;
            }
            if !selected_keys.is_empty() && selected_bytes.saturating_add(bytes) > max_bytes {
                break;
            }
            selected_keys.push(key.clone());
            selected_bytes = selected_bytes.saturating_add(bytes);
            if bytes > max_bytes {
                break;
            }
        }
        let terminal_invocations = selected_keys
            .into_iter()
            .filter_map(|key| {
                self.terminal_invocations
                    .remove(&key)
                    .map(|write| (key, write))
            })
            .collect::<BTreeMap<_, _>>();
        if terminal_invocations.is_empty() {
            return Self::default();
        }
        self.recalculate_estimates();
        let mut chunk = Self {
            terminal_invocations,
            oldest_at: self.oldest_at,
            retained_for_retry: self.retained_for_retry,
            ..Self::default()
        };
        chunk.recalculate_estimates();
        chunk.enqueued_rows = chunk.logical_rows();
        self.enqueued_rows = self.logical_rows();
        if self.is_empty() {
            self.oldest_at = None;
        }
        chunk
    }

    fn take_p2(&mut self) -> Self {
        let mut p2 = std::mem::take(self);
        let terminal_invocations = std::mem::take(&mut p2.terminal_invocations);
        let terminal_estimated_bytes = p2.terminal_estimated_bytes;
        p2.estimated_bytes = p2.estimated_bytes.saturating_sub(terminal_estimated_bytes);
        p2.terminal_estimated_bytes = 0;

        self.terminal_invocations = terminal_invocations;
        self.estimated_bytes = terminal_estimated_bytes;
        self.terminal_estimated_bytes = terminal_estimated_bytes;
        self.enqueued_rows = self.terminal_invocations.len();
        self.oldest_at = p2.oldest_at;
        p2
    }

    fn take_p2_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
        if !self.has_p2() || max_rows == 0 || max_bytes == 0 {
            return Self::default();
        }

        let oldest_at = self.oldest_at;
        let mut chunk = Self {
            oldest_at,
            retained_for_retry: self.retained_for_retry,
            ..Self::default()
        };
        let mut selected_rows = 0_usize;
        let mut selected_bytes = 0_usize;
        let mut should_take = |bytes: usize| {
            if selected_rows >= max_rows {
                return false;
            }
            // A single logical write can exceed the batch budget. Keep it
            // isolated rather than combining it with any other P2 write;
            // the caller emits an explicit oversized-batch diagnostic.
            if selected_rows == 0 && bytes > max_bytes {
                selected_rows = 1;
                selected_bytes = bytes;
                return true;
            }
            if selected_rows > 0 && selected_bytes.saturating_add(bytes) > max_bytes {
                return false;
            }
            selected_rows += 1;
            selected_bytes = selected_bytes.saturating_add(bytes);
            true
        };

        let attempt_progress = std::mem::take(&mut self.attempt_progress);
        for (key, progress) in attempt_progress {
            let bytes = estimated_attempt_progress_memory_bytes(&progress);
            if should_take(bytes) {
                chunk.attempt_progress.insert(key, progress);
            } else {
                self.attempt_progress.insert(key, progress);
            }
        }
        let invocation_derived = std::mem::take(&mut self.invocation_derived);
        for (key, derived) in invocation_derived {
            let bytes = estimated_invocation_derived_memory_bytes(&derived);
            if should_take(bytes) {
                chunk.invocation_derived.insert(key, derived);
            } else {
                self.invocation_derived.insert(key, derived);
            }
        }
        let account_selected_touches = std::mem::take(&mut self.account_selected_touches);
        for (key, touch) in account_selected_touches {
            let bytes = estimated_account_selected_touch_memory_bytes(&touch);
            if should_take(bytes) {
                chunk.account_selected_touches.insert(key, touch);
            } else {
                self.account_selected_touches.insert(key, touch);
            }
        }
        let system_task_finishes = std::mem::take(&mut self.system_task_finishes);
        for (key, finish) in system_task_finishes {
            let bytes = estimated_system_task_finish_memory_bytes(&finish);
            if should_take(bytes) {
                chunk.system_task_finishes.insert(key, finish);
            } else {
                self.system_task_finishes.insert(key, finish);
            }
        }
        if !self.startup_backfill_wake_tasks.is_empty()
            && should_take(
                self.startup_backfill_wake_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            )
        {
            chunk.startup_backfill_wake_tasks =
                std::mem::take(&mut self.startup_backfill_wake_tasks);
        }

        self.recalculate_estimates();
        chunk.recalculate_estimates();
        chunk.enqueued_rows = chunk.logical_rows();
        self.enqueued_rows = self.logical_rows();
        chunk.coalesced_rows = 0;
        if self.is_empty() {
            self.oldest_at = None;
        }
        chunk
    }

    fn add_startup_backfill_wake_tasks(&mut self, tasks: &[StartupBackfillTask]) {
        for task in tasks {
            if !self.startup_backfill_wake_tasks.contains(task) {
                self.startup_backfill_wake_tasks.push(*task);
            }
        }
        self.recalculate_estimates();
    }

    fn merge_p2(&mut self, mut other: Self) {
        self.attempt_progress.extend(other.attempt_progress.drain());
        self.invocation_derived.extend(other.invocation_derived);
        self.account_selected_touches
            .extend(other.account_selected_touches.drain());
        self.system_task_finishes
            .extend(other.system_task_finishes.drain());
        self.add_startup_backfill_wake_tasks(&other.startup_backfill_wake_tasks);
        self.enqueued_rows = self.enqueued_rows.saturating_add(other.enqueued_rows);
        self.coalesced_rows = self.coalesced_rows.saturating_add(other.coalesced_rows);
        self.recalculate_estimates();
        self.oldest_at = match (self.oldest_at, other.oldest_at) {
            (Some(current), Some(other)) => Some(current.min(other)),
            (current, other) => current.or(other),
        };
    }

    fn merge_all(&mut self, mut other: Self) {
        let terminal_invocations = std::mem::take(&mut other.terminal_invocations);
        let terminal_count = terminal_invocations.len();
        for terminal in terminal_invocations.into_values() {
            self.push(SqliteBatchWrite::TerminalInvocation(terminal));
        }
        other.enqueued_rows = other.enqueued_rows.saturating_sub(terminal_count);
        self.merge_p2(other);
    }

    #[cfg(test)]
    fn into_writes(self) -> Vec<SqliteBatchWrite> {
        let mut writes = Vec::with_capacity(self.logical_rows());
        writes.extend(
            self.terminal_invocations
                .into_values()
                .map(SqliteBatchWrite::TerminalInvocation),
        );
        writes.extend(
            self.attempt_progress
                .into_values()
                .map(SqliteBatchWrite::AttemptProgress),
        );
        writes.extend(
            self.invocation_derived
                .into_values()
                .map(SqliteBatchWrite::InvocationDerived),
        );
        writes.extend(
            self.account_selected_touches
                .into_values()
                .map(SqliteBatchWrite::AccountSelectedTouch),
        );
        writes.extend(
            self.system_task_finishes
                .into_values()
                .map(SqliteBatchWrite::SystemTaskFinish),
        );
        writes.extend(
            self.startup_backfill_wake_tasks
                .into_iter()
                .map(SqliteBatchWrite::StartupBackfillWake),
        );
        writes
    }
}

#[derive(Debug)]
pub(crate) struct RetainedBatch {
    batch: PendingBatch,
    failed: bool,
    p2_retryable_failure: bool,
    p2_lock_failure: bool,
    p2_defer: Option<P2DeferReason>,
}

impl RetainedBatch {
    fn new(mut batch: PendingBatch, failed: bool) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed,
            p2_retryable_failure: false,
            p2_lock_failure: false,
            p2_defer: None,
        }
    }

    fn p2_deferred(mut batch: PendingBatch, reason: P2DeferReason) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed: false,
            p2_retryable_failure: false,
            p2_lock_failure: false,
            p2_defer: Some(reason),
        }
    }

    fn p2_failed(mut batch: PendingBatch, retryable_failure: bool, lock_failure: bool) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed: true,
            p2_retryable_failure: retryable_failure,
            p2_lock_failure: lock_failure,
            p2_defer: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P2DeferReason {
    PressureCooldown(u64),
    BackgroundBusy { observed_generation: u64 },
}

fn estimated_option_string_bytes(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}

impl SqliteBatchWrite {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        match self {
            Self::TerminalInvocation(terminal) => terminal.estimated_memory_bytes(),
            Self::AttemptProgress(progress) => estimated_attempt_progress_memory_bytes(progress),
            Self::InvocationDerived(derived) => estimated_invocation_derived_memory_bytes(derived),
            Self::AccountSelectedTouch(touch) => {
                estimated_account_selected_touch_memory_bytes(touch)
            }
            Self::SystemTaskFinish(finish) => estimated_system_task_finish_memory_bytes(finish),
            #[cfg(test)]
            Self::StartupBackfillWake(_) => std::mem::size_of::<StartupBackfillTask>(),
        }
    }
}

fn estimated_attempt_progress_memory_bytes(progress: &BatchedAttemptProgress) -> usize {
    std::mem::size_of::<BatchedAttemptProgress>()
        .saturating_add(progress.phase.capacity())
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_status,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_reason,
        ))
}

fn estimated_invocation_derived_memory_bytes(derived: &BatchedInvocationDerivedWrites) -> usize {
    std::mem::size_of::<BatchedInvocationDerivedWrites>()
        .saturating_add(derived.occurred_at.capacity())
        .saturating_add(
            derived
                .terminal_overlay_key
                .as_ref()
                .map_or(0, |(invoke_id, occurred_at)| {
                    invoke_id.capacity().saturating_add(occurred_at.capacity())
                }),
        )
}

fn estimated_account_selected_touch_memory_bytes(touch: &BatchedAccountSelectedTouch) -> usize {
    std::mem::size_of::<BatchedAccountSelectedTouch>().saturating_add(touch.selected_at.capacity())
}

fn estimated_system_task_finish_memory_bytes(finish: &BatchedSystemTaskFinish) -> usize {
    std::mem::size_of::<BatchedSystemTaskFinish>()
        .saturating_add(finish.trigger_kind.capacity())
        .saturating_add(estimated_option_string_bytes(&finish.summary))
        .saturating_add(estimated_option_string_bytes(&finish.detail))
        .saturating_add(finish.finished_at.capacity())
}

impl PendingBatch {
    fn estimated_memory_bytes(&self) -> usize {
        self.estimated_bytes
    }
}

#[derive(Debug)]
pub(crate) struct SqliteBatchWriter {
    write_sender: mpsc::Sender<SqliteBatchWrite>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
    control_sender: mpsc::Sender<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    dropped_writes: Arc<AtomicU64>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache:
        Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: std::path::PathBuf,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    #[cfg(test)]
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    journal_sync_shutdown: CancellationToken,
    journal_sync_handle: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    buffered_writes: Option<Arc<std::sync::Mutex<Vec<SqliteBatchWrite>>>>,
    #[cfg(test)]
    auto_flush_terminal_for_test: std::sync::atomic::AtomicBool,
}

impl SqliteBatchWriter {
    pub(crate) fn spawn(
        pool: Pool<Sqlite>,
        shutdown: CancellationToken,
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        pricing_catalog: Arc<RwLock<PricingCatalog>>,
        database_path: &Path,
    ) -> Arc<Self> {
        let (write_sender, write_receiver) = mpsc::channel(SQLITE_BATCH_CHANNEL_CAPACITY);
        let queued_p1_count = Arc::new(AtomicUsize::new(0));
        let p1_priority_gate = Arc::new(std::sync::Mutex::new(()));
        let (control_sender, control_receiver) = mpsc::channel(128);
        let accounting = Arc::new(PendingQueueAccounting::default());
        let dropped_writes = Arc::new(AtomicU64::new(0));
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_journal = match TerminalJournal::open(database_path) {
            Ok(journal) => Some(journal),
            Err(err) => {
                warn!(error = %err, path = %database_path.display(), "terminal journal unavailable; using memory durability fallback");
                None
            }
        };
        let replay_writes = terminal_journal
            .as_ref()
            .map(|journal| journal.stats().replay_count)
            .unwrap_or_default();
        let mut terminal_journal = terminal_journal;
        if let Some(journal) = terminal_journal.as_mut() {
            journal.queue_replay_for_dispatch(SQLITE_BATCH_MAX_ROWS);
        }
        let terminal_journal = Arc::new(std::sync::Mutex::new(terminal_journal));
        queued_p1_count.store(replay_writes, Ordering::SeqCst);
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));
        let journal_sync_shutdown = shutdown.child_token();
        #[cfg(not(test))]
        let journal_sync_handle = Some(tokio::spawn(run_terminal_journal_sync(
            terminal_journal.clone(),
            journal_sync_shutdown.clone(),
        )));
        #[cfg(test)]
        let journal_sync_handle = None;
        let cache_for_task = prompt_cache_conversation_cache.clone();
        let handle = tokio::spawn(run_sqlite_batch_writer(
            pool,
            database_path.to_path_buf(),
            write_receiver,
            control_receiver,
            accounting.clone(),
            Some(cache_for_task),
            Some(pricing_catalog),
            terminal_runtime_store.clone(),
            dashboard_activity_snapshot_cache.clone(),
            terminal_projection_hub.clone(),
            dashboard_reconcile_gate.clone(),
            terminal_journal.clone(),
            queued_p1_count.clone(),
            p1_priority_gate.clone(),
        ));
        let writer = Arc::new(Self {
            write_sender,
            queued_p1_count,
            p1_priority_gate,
            control_sender,
            accounting,
            dropped_writes,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            terminal_projection_hub,
            terminal_journal,
            database_path: database_path.to_path_buf(),
            dashboard_reconcile_gate,
            #[cfg(test)]
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(Some(handle)),
            journal_sync_shutdown,
            journal_sync_handle: Mutex::new(journal_sync_handle),
            #[cfg(test)]
            buffered_writes: None,
            #[cfg(test)]
            auto_flush_terminal_for_test: std::sync::atomic::AtomicBool::new(true),
        });
        if replay_writes > 0 {
            warn!(
                replay_count = replay_writes,
                "requeued terminal journal records during startup"
            );
        }
        writer
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test() -> Arc<Self> {
        Self::spawn_for_test_with_prompt_cache(Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )))
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test_with_prompt_cache(
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    ) -> Arc<Self> {
        let (write_sender, _write_receiver) = mpsc::channel(1);
        let (control_sender, _control_receiver) = mpsc::channel(1);
        Arc::new(Self {
            write_sender,
            queued_p1_count: Arc::new(AtomicUsize::new(0)),
            p1_priority_gate: Arc::new(std::sync::Mutex::new(())),
            control_sender,
            accounting: Arc::new(PendingQueueAccounting::default()),
            dropped_writes: Arc::new(AtomicU64::new(0)),
            terminal_runtime_store: Arc::new(std::sync::Mutex::new(None)),
            dashboard_activity_snapshot_cache: Arc::new(std::sync::Mutex::new(None)),
            terminal_projection_hub: Arc::new(std::sync::Mutex::new(None)),
            terminal_journal: Arc::new(std::sync::Mutex::new(None)),
            database_path: std::path::PathBuf::from("test-sqlite-batch-writer.db"),
            dashboard_reconcile_gate: Arc::new(Mutex::new(())),
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(None),
            journal_sync_shutdown: CancellationToken::new(),
            journal_sync_handle: Mutex::new(None),
            buffered_writes: Some(Arc::new(std::sync::Mutex::new(Vec::new()))),
            auto_flush_terminal_for_test: std::sync::atomic::AtomicBool::new(true),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_auto_flush_terminal_for_test(&self, enabled: bool) {
        self.auto_flush_terminal_for_test
            .store(enabled, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn auto_flush_terminal_for_test(&self) -> bool {
        self.auto_flush_terminal_for_test.load(Ordering::Relaxed)
    }

    pub(crate) fn set_terminal_runtime_store(
        &self,
        runtime_store: Arc<ProxyRuntimeInvocationStore>,
    ) {
        if let Ok(mut guard) = self.terminal_runtime_store.lock() {
            *guard = Some(runtime_store);
        }
    }

    pub(crate) fn set_dashboard_activity_snapshot_cache(
        &self,
        cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    ) {
        if let Ok(mut guard) = self.dashboard_activity_snapshot_cache.lock() {
            *guard = Some(cache);
        }
    }

    pub(crate) fn set_terminal_projection_hub(&self, hub: Arc<TerminalProjectionHub>) {
        if let Ok(mut guard) = self.terminal_projection_hub.lock() {
            *guard = Some(hub);
        }
    }

    pub(crate) fn dashboard_reconcile_gate(&self) -> Arc<Mutex<()>> {
        self.dashboard_reconcile_gate.clone()
    }

    pub(crate) fn enqueue(&self, write: SqliteBatchWrite) -> bool {
        let estimated_bytes = write.estimated_memory_bytes();
        let is_p1 = is_p1_terminal_write(&write);
        let _p1_priority_guard = is_p1.then(|| {
            self.p1_priority_gate
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            match buffered_writes.lock() {
                Ok(mut guard) => {
                    guard.push(write);
                    self.accounting.enqueue(estimated_bytes);
                    return true;
                }
                Err(err) => {
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        error = %err,
                        dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                        "sqlite batch writer test buffer poisoned; dropped derived write"
                    );
                    return false;
                }
            }
        }

        self.accounting.enqueue(estimated_bytes);
        if is_p1 {
            self.queued_p1_count.fetch_add(1, Ordering::SeqCst);
        }
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(err) => {
                if is_p1 {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                self.accounting.rollback_enqueue(estimated_bytes);
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!(
                    error = %err,
                    queue_depth = self.accounting.snapshot().pending_depth,
                    dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                    "sqlite batch writer queue full; dropped derived write"
                );
                false
            }
        }
    }

    pub(crate) fn enqueue_terminal(
        &self,
        terminal: BatchedTerminalInvocationWrite,
    ) -> TerminalEnqueueOutcome {
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            let enqueued = self.enqueue(SqliteBatchWrite::TerminalInvocation(terminal));
            return TerminalEnqueueOutcome {
                enqueued,
                durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
                journal_sequence: None,
                journal_pending_records: 0,
                journal_pending_bytes: 0,
            };
        }

        let _p1_priority_guard = self
            .p1_priority_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let recovery_terminal = terminal.clone();
        let journal = self
            .terminal_journal
            .lock()
            .ok()
            .and_then(|mut journal| {
                journal.as_mut().map(|journal| {
                    journal.append(
                        &terminal.record,
                        terminal.raw_capture,
                        terminal.capture_started,
                    )
                })
            })
            .unwrap_or(TerminalJournalAppendOutcome {
                durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
                sequence: None,
                pending_records: 0,
                pending_bytes: 0,
            });
        let enqueued = self.enqueue_terminal_write(
            SqliteBatchWrite::TerminalInvocation(terminal),
            journal.durability_mode,
        );
        if matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        ) {
            let terminals = [&recovery_terminal];
            let mut recovery_persisted = false;
            if let Ok(mut guard) = self.terminal_journal.lock()
                && let Some(journal) = guard.as_mut()
            {
                match journal.quarantine_shutdown_batch(
                    &terminals,
                    &[],
                    "terminal journal memory-overflow recovery",
                ) {
                    Ok(()) => {
                        journal.remember_shutdown_recovery(&terminals);
                        recovery_persisted = true;
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            invoke_id = %recovery_terminal.record.invoke_id,
                            occurred_at = %recovery_terminal.record.occurred_at,
                            "terminal memory-overflow recovery sink failed"
                        );
                        if TerminalJournal::quarantine_shutdown_batch_at_database_path(
                            &self.database_path,
                            &terminals,
                            &[],
                            "terminal journal memory-overflow recovery",
                        )
                        .is_ok()
                        {
                            journal.remember_shutdown_recovery(&terminals);
                            recovery_persisted = true;
                        }
                    }
                }
            }
            if !recovery_persisted
                && let Err(err) = TerminalJournal::quarantine_shutdown_batch_at_database_path(
                    &self.database_path,
                    &terminals,
                    &[],
                    "terminal journal memory-overflow recovery",
                )
            {
                warn!(
                    error = %err,
                    invoke_id = %recovery_terminal.record.invoke_id,
                    occurred_at = %recovery_terminal.record.occurred_at,
                    "terminal memory-overflow recovery sink failed"
                );
            }
        }
        TerminalEnqueueOutcome {
            enqueued,
            durability_mode: journal.durability_mode,
            journal_sequence: journal.sequence,
            journal_pending_records: journal.pending_records,
            journal_pending_bytes: journal.pending_bytes,
        }
    }

    fn enqueue_terminal_write(
        &self,
        write: SqliteBatchWrite,
        durability_mode: TerminalJournalDurabilityMode,
    ) -> bool {
        let estimated_bytes = write.estimated_memory_bytes();
        let is_p1 = is_p1_terminal_write(&write);
        self.accounting.enqueue(estimated_bytes);
        if is_p1 {
            self.queued_p1_count.fetch_add(1, Ordering::SeqCst);
        }
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(write)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                let deferred = matches!(durability_mode, TerminalJournalDurabilityMode::Journal)
                    && self
                        .terminal_journal
                        .lock()
                        .ok()
                        .and_then(|mut journal| {
                            journal.as_mut().map(|journal| match write {
                                SqliteBatchWrite::TerminalInvocation(terminal) => {
                                    journal.defer_write(terminal)
                                }
                                _ => false,
                            })
                        })
                        .unwrap_or(false);
                if is_p1 && !deferred {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                if !deferred {
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        "terminal journal-backed deferred queue could not accept sqlite batch write"
                    );
                }
                deferred
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                if is_p1 {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!("terminal journal-backed retry could not reach closed sqlite batch writer");
                false
            }
        }
    }

    pub(crate) fn terminal_journal_stats(&self) -> TerminalJournalStats {
        self.terminal_journal
            .lock()
            .ok()
            .and_then(|journal| journal.as_ref().map(TerminalJournal::stats))
            .unwrap_or_default()
    }

    pub(crate) fn telemetry_snapshot(&self) -> (usize, usize, u64) {
        let accounting = self.accounting.snapshot();
        (
            accounting.pending_depth,
            accounting.pending_bytes,
            self.dropped_writes.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn accounting_snapshot(&self) -> PendingQueueAccountingSnapshot {
        self.accounting.snapshot()
    }

    pub(crate) async fn flush_now(&self, _pool: &Pool<Sqlite>) -> Result<()> {
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            self.flush_buffered_for_test(_pool).await;
            return Ok(());
        }

        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self
            .write_sender
            .max_capacity()
            .saturating_sub(self.write_sender.capacity());
        if let Err(err) = self
            .control_sender
            .try_send(SqliteBatchWriterControl::FlushNow {
                queued_depth_snapshot,
                responder: sender,
            })
        {
            self.dropped_writes.fetch_add(1, Ordering::Relaxed);
            warn!(
                error = %err,
                queue_depth = self.accounting.snapshot().pending_depth,
                dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                "sqlite batch writer flush barrier could not be queued"
            );
            return Err(anyhow::anyhow!(
                "sqlite batch writer flush barrier could not be queued"
            ));
        }
        match receiver.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow::anyhow!(err)),
            Err(err) => Err(anyhow::anyhow!(
                "sqlite batch writer flush barrier dropped: {err}"
            )),
        }
    }

    pub(crate) async fn shutdown_and_drain(&self) {
        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            let retained = buffered_writes
                .lock()
                .map(|guard| guard.len())
                .unwrap_or_default();
            if retained > 0 {
                warn!(
                    retained,
                    "sqlite batch writer test buffer was not explicitly flushed before shutdown"
                );
            }
            return;
        }

        let Some(handle) = self.handle.lock().await.take() else {
            return;
        };
        let shutdown_deadline = tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self
            .write_sender
            .max_capacity()
            .saturating_sub(self.write_sender.capacity());
        let barrier_sent = match timeout_at(
            shutdown_deadline,
            self.control_sender
                .send(SqliteBatchWriterControl::Shutdown {
                    queued_depth_snapshot,
                    responder: sender,
                }),
        )
        .await
        {
            Ok(Ok(())) => true,
            Ok(Err(err)) => {
                warn!(error = %err, "sqlite batch writer shutdown barrier could not be queued");
                false
            }
            Err(_) => {
                warn!("sqlite batch writer shutdown barrier queue timed out");
                false
            }
        };
        if barrier_sent {
            match timeout_at(shutdown_deadline, receiver).await {
                Ok(Ok(Err(err))) => {
                    warn!(error = %err, "sqlite batch writer shutdown drain failed");
                }
                Ok(Err(err)) => {
                    warn!(error = %err, "sqlite batch writer shutdown responder dropped");
                }
                Err(_) => {
                    warn!("sqlite batch writer shutdown drain timed out");
                }
                Ok(Ok(Ok(()))) => {}
            }
        }
        let mut handle = handle;
        let worker_deadline = tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
        let worker_timed_out = match timeout_at(worker_deadline, &mut handle).await {
            Ok(Ok(())) => false,
            Ok(Err(err)) => {
                warn!(error = %err, "sqlite batch writer task failed during shutdown");
                false
            }
            Err(_) => {
                warn!(
                    "sqlite batch writer task exceeded shutdown deadline; retaining worker ownership for recovery"
                );
                true
            }
        };
        if worker_timed_out {
            // Do not abort a worker that still owns an in-flight batch. The worker's own
            // bounded shutdown path will quarantine that batch; keeping the handle lets a
            // later shutdown attempt observe completion instead of losing memory-overflow P1.
            *self.handle.lock().await = Some(handle);
            return;
        }
        let (abandoned_depth, abandoned_bytes) = self.accounting.clear_after_shutdown();
        self.queued_p1_count.store(0, Ordering::SeqCst);
        if abandoned_depth > 0 || abandoned_bytes > 0 {
            warn!(
                abandoned_depth,
                abandoned_bytes,
                "sqlite batch writer cleared accounting for shutdown-owned queued work; journal/P2 recovery remains authoritative"
            );
        }
        self.journal_sync_shutdown.cancel();
        if let Some(mut handle) = self.journal_sync_handle.lock().await.take() {
            match timeout_at(
                tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE,
                &mut handle,
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    warn!(error = %err, "terminal journal sync task failed during shutdown")
                }
                Err(_) => {
                    warn!("terminal journal sync task exceeded shutdown deadline; aborting");
                    handle.abort();
                    let _ = handle.await;
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn stats_snapshot(&self) -> (usize, u64) {
        (
            self.accounting.snapshot().pending_depth,
            self.dropped_writes.load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_test(pool: &Pool<Sqlite>, writes: Vec<SqliteBatchWrite>) {
        Self::flush_for_test_with_pricing_catalog(pool, None, writes).await;
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_test_with_pricing_catalog(
        pool: &Pool<Sqlite>,
        pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
        writes: Vec<SqliteBatchWrite>,
    ) {
        let mut batch = PendingBatch::default();
        for write in writes {
            batch.push(write);
        }
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));
        let deferred = flush_pending_batch_inner(
            pool,
            &batch,
            pricing_catalog.as_ref(),
            None,
            &terminal_runtime_store,
            &dashboard_activity_snapshot_cache,
            &terminal_projection_hub,
            &dashboard_reconcile_gate,
        )
        .await
        .expect("flush pending sqlite batch writes");
        if !deferred.is_empty() {
            flush_pending_batch_inner(
                pool,
                &deferred,
                pricing_catalog.as_ref(),
                None,
                &terminal_runtime_store,
                &dashboard_activity_snapshot_cache,
                &terminal_projection_hub,
                &dashboard_reconcile_gate,
            )
            .await
            .expect("flush deferred pending sqlite batch writes");
        }
    }

    #[cfg(test)]
    pub(crate) async fn flush_buffered_for_test(&self, pool: &Pool<Sqlite>) {
        let writes = self
            .buffered_writes
            .as_ref()
            .and_then(|buffered_writes| {
                buffered_writes
                    .lock()
                    .ok()
                    .map(|mut guard| guard.drain(..).collect::<Vec<_>>())
            })
            .unwrap_or_default();

        if !writes.is_empty() {
            let submitted_count = writes.len();
            let submitted_bytes = writes
                .iter()
                .map(SqliteBatchWrite::estimated_memory_bytes)
                .sum::<usize>();
            let mut batch = PendingBatch::default();
            for write in writes {
                batch.push(write);
            }
            let deferred = flush_pending_batch_inner(
                pool,
                &batch,
                None,
                self.prompt_cache_conversation_cache.as_ref(),
                &self.terminal_runtime_store,
                &self.dashboard_activity_snapshot_cache,
                &self.terminal_projection_hub,
                &self.dashboard_reconcile_gate,
            )
            .await
            .expect("flush buffered sqlite batch writes for test");
            let deferred_writes = deferred.into_writes();
            let retained_count = deferred_writes.len();
            let retained_bytes = deferred_writes
                .iter()
                .map(SqliteBatchWrite::estimated_memory_bytes)
                .sum::<usize>();
            self.accounting.transfer_p1_to_p2(retained_bytes);
            let retained = self
                .buffered_writes
                .as_ref()
                .and_then(|buffered_writes| buffered_writes.lock().ok())
                .map(|mut guard| {
                    guard.extend(deferred_writes);
                    (retained_count, retained_bytes)
                })
                .unwrap_or_default();
            self.accounting
                .complete(submitted_count, retained.0, submitted_bytes, retained.1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_sqlite_batch_writer(
    pool: Pool<Sqlite>,
    database_path: std::path::PathBuf,
    mut write_receiver: mpsc::Receiver<SqliteBatchWrite>,
    mut control_receiver: mpsc::Receiver<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
) {
    let mut ticker = interval(SQLITE_BATCH_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut deferred_ticker = interval(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL);
    deferred_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending = PendingBatch::default();
    let mut control_closed = false;
    let mut p1_retry = P1RetryState::default();
    let mut p2_schedule = P2ScheduleState::default();
    let mut p2_eligibility_generation =
        crate::db_pressure::global_db_pressure_gate().eligibility_generation();
    let mut transaction_sequence = 0_u64;

    loop {
        drain_queued_writes_before_dispatch(
            &mut write_receiver,
            &mut pending,
            &accounting,
            &mut p2_schedule,
            SQLITE_BATCH_MAX_ROWS,
            &queued_p1_count,
        );
        tokio::select! {
            biased;
            _ = crate::db_pressure::global_db_pressure_gate()
                .wait_for_eligibility_change(p2_eligibility_generation),
                if pending.has_p2()
                    && p2_schedule.wake_reason == Some(P2WakeReason::BackgroundEligible) =>
            {
                p2_eligibility_generation = crate::db_pressure::global_db_pressure_gate()
                    .eligibility_generation();
                p2_schedule.wake_background_eligible();
                accounting.update_p2_schedule(&p2_schedule);
            }
            maybe_control = control_receiver.recv(), if !control_closed => {
                if let Some(control) = maybe_control {
                    match control {
                    SqliteBatchWriterControl::FlushNow {
                        queued_depth_snapshot,
                        responder,
                    } => {
                        let mut remaining_queued = queued_depth_snapshot;
                        let mut result = Ok(());
                        loop {
                            let drained = drain_queued_batch_writes(
                                &mut write_receiver,
                                &mut pending,
                                &accounting,
                                remaining_queued.min(SQLITE_BATCH_MAX_ROWS),
                                &queued_p1_count,
                            );
                            remaining_queued = remaining_queued.saturating_sub(drained);
                            drain_terminal_journal_deferred_writes(
                                &terminal_journal,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            if pending.is_empty() {
                                if remaining_queued == 0 || drained == 0 {
                                    break;
                                }
                                continue;
                            }
                            let flush_batch = take_next_bounded_batch(&mut pending);
                            if flush_batch.is_empty() {
                                break;
                            }
                            if flush_batch.terminal_invocations.is_empty()
                                && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                            {
                                warn!(
                                    flush_priority = "P2",
                                    batch_rows = flush_batch.logical_rows(),
                                    batch_bytes = flush_batch.estimated_memory_bytes(),
                                    max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                                    "isolating oversized single P2 write"
                                );
                            }
                            let Some(retained) = flush_pending_batch_accounted(
                                &accounting,
                                &pool,
                                pricing_catalog.as_ref(),
                                flush_batch,
                                FlushReason::Barrier,
                                prompt_cache_conversation_cache.as_ref(),
                                &terminal_runtime_store,
                                &dashboard_activity_snapshot_cache,
                                &terminal_projection_hub,
                                &dashboard_reconcile_gate,
                                &terminal_journal,
                            )
                            .await
                            else {
                                p2_schedule.succeeded();
                                accounting.update_p2_schedule(&p2_schedule);
                                if pending.is_empty() && remaining_queued == 0 {
                                    break;
                                }
                                continue;
                            };
                            let logical_rows = retained.batch.logical_rows();
                            let failed = retained.failed;
                            if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                let delay = p1_retry.failed(transaction_sequence);
                                warn!(
                                    write_class = "p1_terminal",
                                    retry_generation = p1_retry.generation as u64,
                                    next_retry_delay_ms = delay.as_millis() as u64,
                                    "scheduled retained P1 batch after forced flush failure"
                                );
                            } else {
                                p1_retry.succeeded();
                            }
                            match retained.p2_defer {
                                Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                    p2_schedule.defer_pressure(
                                        Duration::from_millis(remaining_ms),
                                        P2WakeReason::PressureCooldownElapsed,
                                    );
                                }
                                Some(P2DeferReason::BackgroundBusy {
                                    observed_generation,
                                }) => {
                                    p2_eligibility_generation = observed_generation;
                                    p2_schedule.defer_until_background_eligible();
                                }
                                None if retained.failed
                                    && retained.p2_retryable_failure
                                    && retained.batch.has_p2()
                                    && retained.batch.terminal_invocations.is_empty() =>
                                {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    p2_schedule.failed(transaction_sequence);
                                    if retained.p2_lock_failure {
                                        accounting.p2_lock_retried();
                                    }
                                }
                                None if retained.batch.has_p2() => {
                                    p2_schedule.arm_if_idle(Instant::now());
                                }
                                None => p2_schedule.succeeded(),
                            }
                            accounting.update_p2_schedule(&p2_schedule);
                            let p2_deferred = retained.p2_defer.is_some();
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            pending = retained_batch;
                            if failed {
                                result = Err(format!(
                                    "sqlite batch writer retained {logical_rows} logical rows after forced flush"
                                ));
                                break;
                            }
                            if p2_deferred {
                                break;
                            }
                            if pending.is_empty() && remaining_queued == 0 {
                                break;
                            }
                        }
                        let _ = responder.send(result);
                    }
                    SqliteBatchWriterControl::Shutdown { responder, .. } => {
                        write_receiver.close();
                        let mut result = Ok(());
                        let shutdown_deadline = Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
                        loop {
                            if Instant::now() >= shutdown_deadline {
                                let drained = drain_queued_batch_writes(
                                    &mut write_receiver,
                                    &mut pending,
                                    &accounting,
                                    SQLITE_BATCH_CHANNEL_CAPACITY,
                                    &queued_p1_count,
                                );
                                drain_terminal_journal_deferred_writes(
                                    &terminal_journal,
                                    &mut pending,
                                    &accounting,
                                    SQLITE_BATCH_CHANNEL_CAPACITY,
                                    &queued_p1_count,
                                );
                                let abandoned = std::mem::take(&mut pending);
                                let _ = release_shutdown_pending_batch(
                                    &accounting,
                                    &terminal_journal,
                                    &database_path,
                                    &abandoned,
                                    "shutdown drain deadline exceeded",
                                )
                                .map_err(|err| {
                                    result = Err(format!(
                                        "sqlite batch writer shutdown quarantine failed: {err:#}"
                                    ));
                                });
                                warn!(
                                    drained,
                                    "sqlite batch writer bounded shutdown drain abandoned remaining queued work"
                                );
                                break;
                            }
                            let drained = drain_queued_batch_writes(
                                &mut write_receiver,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            drain_terminal_journal_deferred_writes(
                                &terminal_journal,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            if pending.is_empty() {
                                if drained == 0 {
                                    break;
                                }
                                continue;
                            }
                            let flush_batch = take_next_bounded_batch(&mut pending);
                            let flush_rows = flush_batch.logical_rows();
                            let flush_bytes = flush_batch.estimated_memory_bytes();
                            let shutdown_quarantine = shutdown_recovery_batch(&flush_batch);
                            let retained = match timeout_at(
                                shutdown_deadline.into(),
                                flush_pending_batch_accounted(
                                    &accounting,
                                    &pool,
                                    pricing_catalog.as_ref(),
                                    flush_batch,
                                    FlushReason::Shutdown,
                                    prompt_cache_conversation_cache.as_ref(),
                                    &terminal_runtime_store,
                                    &dashboard_activity_snapshot_cache,
                                    &terminal_projection_hub,
                                    &dashboard_reconcile_gate,
                                    &terminal_journal,
                                ),
                            )
                            .await
                            {
                                Ok(Some(retained)) => retained,
                                Ok(None) => continue,
                                Err(_) => {
                                    if let Some(quarantine) = shutdown_quarantine.as_ref()
                                        && let Err(err) = quarantine_shutdown_batch(
                                            &terminal_journal,
                                            &database_path,
                                            quarantine,
                                            "shutdown flush deadline exceeded",
                                        )
                                    {
                                        warn!(error = %err, "shutdown system-task quarantine failed after flush timeout");
                                    }
                                    accounting.release(flush_rows, flush_bytes);
                                    result = Err(format!(
                                        "sqlite batch writer shutdown flush exceeded deadline with {flush_rows} rows"
                                    ));
                                    break;
                                }
                            };
                            let logical_rows = retained.batch.logical_rows();
                            let failed = retained.failed;
                            let retained_rows_before_merge = retained.batch.logical_rows();
                            let retained_bytes_before_merge = retained.batch.estimated_memory_bytes();
                            let pending_rows_before_merge = pending.logical_rows();
                            let pending_bytes_before_merge = pending.estimated_memory_bytes();
                            let p2_defer_reason = retained.p2_defer;
                            let p2_deferred = p2_defer_reason.is_some();
                            let p2_retryable_failure = retained.p2_retryable_failure;
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            let merged_rows = retained_batch.logical_rows();
                            let merged_bytes = retained_batch.estimated_memory_bytes();
                            accounting.replace_batch(
                                retained_rows_before_merge + pending_rows_before_merge,
                                merged_rows,
                                retained_bytes_before_merge + pending_bytes_before_merge,
                                merged_bytes,
                            );
                            if failed {
                                let retry_delay = if !retained_batch.terminal_invocations.is_empty() {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    Some(p1_retry.failed(transaction_sequence))
                                } else if p2_retryable_failure {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    Some(p2_schedule.failed(transaction_sequence))
                                } else {
                                    None
                                };
                                if retry_delay.is_some() && Instant::now() < shutdown_deadline {
                                    pending = retained_batch;
                                    let delay = retry_delay
                                        .unwrap_or(Duration::from_millis(1))
                                        .min(shutdown_deadline.saturating_duration_since(Instant::now()));
                                    sleep(delay).await;
                                    continue;
                                }
                                result = Err(format!(
                                    "sqlite batch writer retained {logical_rows} logical rows after shutdown flush"
                                ));
                                accounting.release(merged_rows, merged_bytes);
                                break;
                            }
                            if !retained_batch.terminal_invocations.is_empty() {
                                pending = retained_batch;
                                continue;
                            }
                            if p2_deferred {
                                pending = retained_batch;
                                let delay = match p2_defer_reason {
                                    Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                        Duration::from_millis(remaining_ms.max(1))
                                    }
                                    Some(P2DeferReason::BackgroundBusy { .. }) | None => {
                                        Duration::from_millis(250)
                                    }
                                };
                                sleep(delay.min(
                                    shutdown_deadline.saturating_duration_since(Instant::now()),
                                ))
                                .await;
                                continue;
                            }
                            if !retained_batch.is_empty() {
                                pending = retained_batch;
                                continue;
                            }
                            warn!(
                                logical_rows,
                                "sqlite batch writer completed shutdown drain"
                            );
                        }
                        if !pending.is_empty() {
                            let abandoned = std::mem::take(&mut pending);
                            if let Err(err) = release_shutdown_pending_batch(
                                &accounting,
                                &terminal_journal,
                                &database_path,
                                &abandoned,
                                "shutdown drain stopped after flush timeout",
                            ) {
                                result = Err(format!(
                                    "sqlite batch writer shutdown quarantine failed: {err:#}"
                                ));
                            }
                        }
                        let _ = responder.send(result);
                        return;
                    }
                    }
                } else {
                    control_closed = true;
                }
            }
            _ = wait_for_p2_deadline(p2_schedule.due_at),
                if p2_deadline_ready(&pending, &p2_schedule, &queued_p1_count, &p1_retry) =>
            {
                let priority_guard = p1_priority_gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                drain_terminal_journal_deferred_writes(
                    &terminal_journal,
                    &mut pending,
                    &accounting,
                    SQLITE_BATCH_MAX_ROWS,
                    &queued_p1_count,
                );
                drain_queued_writes_before_p2_dispatch(
                    &mut write_receiver,
                    &mut pending,
                    &accounting,
                    &mut p2_schedule,
                    &queued_p1_count,
                );
                if queued_p1_count.load(Ordering::SeqCst) != 0 {
                    drop(priority_guard);
                    p2_schedule.arm_if_idle(Instant::now());
                    accounting.update_p2_schedule(&p2_schedule);
                    continue;
                }
                let now = Instant::now();
                let submitted_p1 = !pending.terminal_invocations.is_empty() && p1_retry.ready(now);
                let flush_batch = if submitted_p1 {
                    pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                } else {
                    pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                };
                drop(priority_guard);
                if flush_batch.terminal_invocations.is_empty()
                    && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                {
                    warn!(
                        flush_priority = "P2",
                        batch_rows = flush_batch.logical_rows(),
                        batch_bytes = flush_batch.estimated_memory_bytes(),
                        max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                        "isolating oversized single P2 write"
                    );
                }
                if let Some(retained) =
                    flush_pending_batch_accounted(
                        &accounting,
                        &pool,
                        pricing_catalog.as_ref(),
                        flush_batch,
                        FlushReason::Interval,
                        prompt_cache_conversation_cache.as_ref(),
                        &terminal_runtime_store,
                        &dashboard_activity_snapshot_cache,
                        &terminal_projection_hub,
                        &dashboard_reconcile_gate,
                        &terminal_journal,
                    )
                    .await
                {
                    if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                        transaction_sequence = transaction_sequence.saturating_add(1);
                        let delay = p1_retry.failed(transaction_sequence);
                        warn!(
                            write_class = "p1_terminal",
                            retry_generation = p1_retry.generation as u64,
                            next_retry_delay_ms = delay.as_millis() as u64,
                            "scheduled retained P1 batch with exponential backoff"
                        );
                    } else if submitted_p1 {
                        p1_retry.succeeded();
                    }
                    match retained.p2_defer {
                        Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                            p2_schedule.defer_pressure(
                                Duration::from_millis(remaining_ms),
                                P2WakeReason::PressureCooldownElapsed,
                            );
                        }
                        Some(P2DeferReason::BackgroundBusy {
                            observed_generation,
                        }) => {
                            p2_eligibility_generation = observed_generation;
                            p2_schedule.defer_until_background_eligible();
                        }
                        None if retained.failed
                            && retained.p2_retryable_failure
                            && retained.batch.has_p2()
                            && retained.batch.terminal_invocations.is_empty() => {
                            transaction_sequence = transaction_sequence.saturating_add(1);
                            let delay = p2_schedule.failed(transaction_sequence);
                            if retained.p2_lock_failure {
                                accounting.p2_lock_retried();
                            }
                            warn!(
                                write_class = "p2_derived",
                                retry_generation = p2_schedule.generation as u64,
                                next_retry_delay_ms = delay.as_millis() as u64,
                                "scheduled retained P2 batch with exponential backoff"
                            );
                        }
                        None if retained.batch.has_p2() => {
                            p2_schedule.arm_if_idle(Instant::now());
                        }
                        None => p2_schedule.succeeded(),
                    }
                    if retained.batch.terminal_invocations.is_empty() {
                        pending.merge_p2(retained.batch);
                    } else {
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        pending = retained_batch;
                    }
                    accounting.update_p2_schedule(&p2_schedule);
                } else {
                    if submitted_p1 {
                        p1_retry.succeeded();
                    }
                    p2_schedule.succeeded();
                    accounting.update_p2_schedule(&p2_schedule);
                }
            }
            _ = deferred_ticker.tick() => {
                let deferred_capacity = SQLITE_BATCH_MAX_ROWS.saturating_sub(pending.logical_rows());
                drain_terminal_journal_deferred_writes(
                    &terminal_journal,
                    &mut pending,
                    &accounting,
                    deferred_capacity,
                    &queued_p1_count,
                );
            }
            maybe_write = write_receiver.recv() => {
                let Some(write) = maybe_write else {
                    let shutdown_deadline = Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
                    loop {
                        if Instant::now() >= shutdown_deadline {
                            let abandoned = std::mem::take(&mut pending);
                            let _ = release_shutdown_pending_batch(
                                &accounting,
                                &terminal_journal,
                                &database_path,
                                &abandoned,
                                "receiver shutdown drain deadline exceeded",
                            )
                            .map_err(|err| {
                                warn!(error = %err, "receiver shutdown quarantine failed");
                            });
                            break;
                        }
                        drain_terminal_journal_deferred_writes(
                            &terminal_journal,
                            &mut pending,
                            &accounting,
                            SQLITE_BATCH_MAX_ROWS,
                            &queued_p1_count,
                        );
                        if pending.is_empty() {
                            break;
                        }
                        let flush_batch = take_next_bounded_batch(&mut pending);
                        let flush_rows = flush_batch.logical_rows();
                        let flush_bytes = flush_batch.estimated_memory_bytes();
                        let shutdown_quarantine = shutdown_recovery_batch(&flush_batch);
                        let retained = match timeout_at(
                            shutdown_deadline.into(),
                            flush_pending_batch_accounted(
                                &accounting,
                                &pool,
                                pricing_catalog.as_ref(),
                                flush_batch,
                                FlushReason::Shutdown,
                                prompt_cache_conversation_cache.as_ref(),
                                &terminal_runtime_store,
                                &dashboard_activity_snapshot_cache,
                                &terminal_projection_hub,
                                &dashboard_reconcile_gate,
                                &terminal_journal,
                            ),
                        )
                        .await
                        {
                            Ok(Some(retained)) => retained,
                            Ok(None) => continue,
                            Err(_) => {
                                if let Some(quarantine) = shutdown_quarantine.as_ref()
                                    && let Err(err) = quarantine_shutdown_batch(
                                        &terminal_journal,
                                        &database_path,
                                        quarantine,
                                        "receiver shutdown flush deadline exceeded",
                                    )
                                {
                                    warn!(error = %err, "receiver shutdown system-task quarantine failed after flush timeout");
                                }
                                accounting.release(flush_rows, flush_bytes);
                                warn!(
                                    flush_rows,
                                    "sqlite batch writer receiver shutdown flush exceeded deadline"
                                );
                                break;
                            }
                        };
                        let retained_rows_before_merge = retained.batch.logical_rows();
                        let retained_bytes_before_merge = retained.batch.estimated_memory_bytes();
                        let pending_rows_before_merge = pending.logical_rows();
                        let pending_bytes_before_merge = pending.estimated_memory_bytes();
                        let p2_defer_reason = retained.p2_defer;
                        let p2_deferred = p2_defer_reason.is_some();
                        let p2_retryable_failure = retained.p2_retryable_failure;
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        let merged_rows = retained_batch.logical_rows();
                        let merged_bytes = retained_batch.estimated_memory_bytes();
                        accounting.replace_batch(
                            retained_rows_before_merge + pending_rows_before_merge,
                            merged_rows,
                            retained_bytes_before_merge + pending_bytes_before_merge,
                            merged_bytes,
                        );
                        if retained.failed {
                            let retry_delay = if !retained_batch.terminal_invocations.is_empty() {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                Some(p1_retry.failed(transaction_sequence))
                            } else if p2_retryable_failure {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                Some(p2_schedule.failed(transaction_sequence))
                            } else {
                                None
                            };
                            if retry_delay.is_some() && Instant::now() < shutdown_deadline {
                                pending = retained_batch;
                                let delay = retry_delay
                                    .unwrap_or(Duration::from_millis(1))
                                    .min(shutdown_deadline.saturating_duration_since(Instant::now()));
                                sleep(delay).await;
                                continue;
                            }
                            accounting.release(merged_rows, merged_bytes);
                            warn!(
                                retained_rows = retained_batch.logical_rows(),
                                retained_bytes = retained_batch.estimated_memory_bytes(),
                                "sqlite batch writer released failed retained memory accounting after receiver shutdown"
                            );
                            break;
                        }
                        if !retained_batch.terminal_invocations.is_empty() {
                            pending = retained_batch;
                            continue;
                        }
                        if p2_deferred {
                            pending = retained_batch;
                            let delay = match p2_defer_reason {
                                Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                    Duration::from_millis(remaining_ms.max(1))
                                }
                                Some(P2DeferReason::BackgroundBusy { .. }) | None => {
                                    Duration::from_millis(250)
                                }
                            };
                            sleep(delay.min(
                                shutdown_deadline.saturating_duration_since(Instant::now()),
                            ))
                            .await;
                            continue;
                        }
                        if !retained_batch.is_empty() {
                            pending = retained_batch;
                            continue;
                        }
                        break;
                    }
                    if !pending.is_empty() {
                        let abandoned = std::mem::take(&mut pending);
                        if let Err(err) = release_shutdown_pending_batch(
                            &accounting,
                            &terminal_journal,
                            &database_path,
                            &abandoned,
                            "receiver shutdown drain stopped after flush timeout",
                        ) {
                            warn!(error = %err, "receiver shutdown quarantine failed after flush timeout");
                        }
                    }
                    return;
                };
                if is_p1_terminal_write(&write) {
                    decrement_queued_p1_count(&queued_p1_count);
                }
                pending.push_accounted(write, &accounting);
                if pending.has_p2() {
                    p2_schedule.arm_if_idle(Instant::now());
                    accounting.update_p2_schedule(&p2_schedule);
                }
                if (pending.logical_rows() >= SQLITE_BATCH_MAX_ROWS
                    || pending.estimated_memory_bytes() >= SQLITE_BATCH_MAX_BYTES)
                    && (pending.terminal_invocations.is_empty() || p1_retry.ready(Instant::now()))
                    && (!pending.terminal_invocations.is_empty()
                        || p2_schedule.ready(Instant::now()))
                {
                    let priority_guard = p1_priority_gate
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let p2_ready = p2_schedule.ready(Instant::now())
                        && queued_p1_count.load(Ordering::SeqCst) == 0;
                    let flush_batch = if !pending.terminal_invocations.is_empty() {
                        pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                    } else if p2_ready {
                        pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                    } else {
                        drop(priority_guard);
                        continue;
                    };
                    drop(priority_guard);
                    if flush_batch.terminal_invocations.is_empty()
                        && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                    {
                        warn!(
                            flush_priority = "P2",
                            batch_rows = flush_batch.logical_rows(),
                            batch_bytes = flush_batch.estimated_memory_bytes(),
                            max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                            "isolating oversized single P2 write"
                        );
                    }
                    let submitted_p1 = !flush_batch.terminal_invocations.is_empty();
                    let submitted_p2 = flush_batch.has_p2();
                    if let Some(retained) =
                        flush_pending_batch_accounted(
                            &accounting,
                            &pool,
                            pricing_catalog.as_ref(),
                            flush_batch,
                            FlushReason::RowLimit,
                            prompt_cache_conversation_cache.as_ref(),
                            &terminal_runtime_store,
                            &dashboard_activity_snapshot_cache,
                            &terminal_projection_hub,
                            &dashboard_reconcile_gate,
                            &terminal_journal,
                        )
                        .await
                    {
                        if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                            transaction_sequence = transaction_sequence.saturating_add(1);
                            let delay = p1_retry.failed(transaction_sequence);
                            warn!(
                                write_class = "p1_terminal",
                                retry_generation = p1_retry.generation as u64,
                                next_retry_delay_ms = delay.as_millis() as u64,
                                "scheduled retained P1 batch with exponential backoff"
                            );
                        } else {
                            p1_retry.succeeded();
                        }
                        match retained.p2_defer {
                            Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                p2_schedule.defer_pressure(
                                    Duration::from_millis(remaining_ms),
                                    P2WakeReason::PressureCooldownElapsed,
                                );
                            }
                            Some(P2DeferReason::BackgroundBusy {
                                observed_generation,
                            }) => {
                                p2_eligibility_generation = observed_generation;
                                p2_schedule.defer_until_background_eligible();
                            }
                            None if retained.failed
                                && retained.p2_retryable_failure
                                && retained.batch.has_p2()
                                && retained.batch.terminal_invocations.is_empty() => {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                p2_schedule.failed(transaction_sequence);
                                if retained.p2_lock_failure {
                                    accounting.p2_lock_retried();
                                }
                            }
                            None if retained.batch.has_p2() => {
                                p2_schedule.arm_if_idle(Instant::now());
                            }
                            None => p2_schedule.succeeded(),
                        }
                        accounting.update_p2_schedule(&p2_schedule);
                        if retained.batch.terminal_invocations.is_empty() {
                            pending.merge_p2(retained.batch);
                        } else {
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            pending = retained_batch;
                        }
                    } else {
                        if submitted_p1 {
                            p1_retry.succeeded();
                        }
                        if submitted_p2 {
                            p2_schedule.succeeded();
                            accounting.update_p2_schedule(&p2_schedule);
                        }
                    }
                }
            }
            _ = ticker.tick() => {
                if pending.terminal_invocations.is_empty() || !p1_retry.ready(Instant::now()) {
                    continue;
                }
                let flush_reason = if pending.age() >= SQLITE_BATCH_MAX_AGE {
                    if pending.age() >= SQLITE_BATCH_STALE_WARN_AGE {
                        warn!(
                            logical_rows = pending.logical_rows(),
                            enqueued_rows = pending.enqueued_rows,
                            coalesced_rows = pending.coalesced_rows,
                            oldest_age_ms = pending.age().as_millis() as u64,
                            flush_reason = FlushReason::MaxAge.as_str(),
                            "sqlite batch writer pending terminal writes are stale under database pressure"
                        );
                    }
                    FlushReason::MaxAge
                } else {
                    FlushReason::Interval
                };
                let flush_batch = pending
                    .take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES);
                if let Some(retained) =
                    flush_pending_batch_accounted(
                        &accounting,
                        &pool,
                        pricing_catalog.as_ref(),
                        flush_batch,
                        flush_reason,
                        prompt_cache_conversation_cache.as_ref(),
                        &terminal_runtime_store,
                        &dashboard_activity_snapshot_cache,
                        &terminal_projection_hub,
                        &dashboard_reconcile_gate,
                        &terminal_journal,
                    )
                    .await
                {
                    if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                        transaction_sequence = transaction_sequence.saturating_add(1);
                        let delay = p1_retry.failed(transaction_sequence);
                        warn!(
                            write_class = "p1_terminal",
                            retry_generation = p1_retry.generation as u64,
                            next_retry_delay_ms = delay.as_millis() as u64,
                            "scheduled retained P1 batch with exponential backoff"
                        );
                    } else {
                        p1_retry.succeeded();
                    }
                    match retained.p2_defer {
                        Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                            p2_schedule.defer_pressure(
                                Duration::from_millis(remaining_ms),
                                P2WakeReason::PressureCooldownElapsed,
                            );
                        }
                        Some(P2DeferReason::BackgroundBusy {
                            observed_generation,
                        }) => {
                            p2_eligibility_generation = observed_generation;
                            p2_schedule.defer_until_background_eligible();
                        }
                        None if retained.batch.has_p2() => {
                            p2_schedule.arm_if_idle(Instant::now());
                        }
                        None => p2_schedule.succeeded(),
                    }
                    if retained.batch.terminal_invocations.is_empty() {
                        pending.merge_p2(retained.batch);
                    } else {
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        pending = retained_batch;
                    }
                    accounting.update_p2_schedule(&p2_schedule);
                } else {
                    p1_retry.succeeded();
                }
            }
        }
    }
}

#[cfg(not(test))]
async fn run_terminal_journal_sync(
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    shutdown: CancellationToken,
) {
    let mut ticker = interval(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                    && let Err(err) = journal.force_sync()
                {
                    warn!(error = %err, "terminal journal final group commit failed during shutdown");
                }
                return;
            }
            _ = ticker.tick() => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                    && let Some(group_commit_elapsed_ms) = journal.sync_if_due()
                {
                    let stats = journal.stats();
                    debug!(
                        group_commit_elapsed_ms,
                        journal_pending_records = stats.pending_records,
                        journal_pending_bytes = stats.pending_bytes,
                        journal_segment_count = stats.segment_count,
                        "terminal journal group commit completed"
                    );
                }
            }
        }
    }
}

fn drain_terminal_journal_deferred_writes(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    max_writes: usize,
    queued_p1_count: &AtomicUsize,
) {
    if max_writes == 0 {
        return;
    }
    if let Ok(mut guard) = terminal_journal.lock()
        && let Some(journal) = guard.as_mut()
    {
        let deferred_capacity = max_writes.saturating_sub(journal.deferred_write_count());
        journal.queue_replay_for_dispatch(deferred_capacity);
        for terminal in journal.take_deferred_writes(max_writes) {
            decrement_queued_p1_count(queued_p1_count);
            let write = SqliteBatchWrite::TerminalInvocation(terminal);
            accounting.enqueue(write.estimated_memory_bytes());
            accounting.retry_deferred();
            pending.push_accounted(write, accounting);
        }
    }
}

pub(crate) fn drain_queued_batch_writes(
    write_receiver: &mut mpsc::Receiver<SqliteBatchWrite>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    max_messages: usize,
    queued_p1_count: &AtomicUsize,
) -> usize {
    let mut drained = 0_usize;
    for _ in 0..max_messages {
        match write_receiver.try_recv() {
            Ok(write) => {
                if is_p1_terminal_write(&write) {
                    decrement_queued_p1_count(queued_p1_count);
                }
                pending.push_accounted(write, accounting);
                drained = drained.saturating_add(1);
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    drained
}

fn take_next_bounded_batch(pending: &mut PendingBatch) -> PendingBatch {
    if !pending.terminal_invocations.is_empty() {
        pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    } else {
        pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The accounting wrapper mirrors the single-writer ownership boundaries."
)]
async fn flush_pending_batch_accounted(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    batch: PendingBatch,
    reason: FlushReason,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
) -> Option<RetainedBatch> {
    let was_retained_retry = batch.retained_for_retry;
    let submitted_depth = batch.logical_rows();
    let submitted_bytes = batch.estimated_memory_bytes();
    let result = flush_pending_batch(
        accounting,
        pool,
        pricing_catalog,
        batch,
        reason,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        terminal_projection_hub,
        dashboard_reconcile_gate,
        terminal_journal,
    )
    .await;
    let discard_non_retryable_p2 = result.as_ref().is_some_and(|retained| {
        retained.failed
            && !retained.p2_retryable_failure
            && retained.batch.terminal_invocations.is_empty()
    });
    if discard_non_retryable_p2 {
        let discarded_overlay_count = result
            .as_ref()
            .map(|retained| {
                cleanup_discarded_p2_runtime_overlays(&retained.batch, terminal_runtime_store)
            })
            .unwrap_or_default();
        warn!(
            flush_priority = "P2",
            submitted_depth,
            submitted_bytes,
            discarded_overlay_count,
            "discarded non-retryable P2 batch after deterministic failure; durable source remains authoritative"
        );
    }
    if was_retained_retry && result.as_ref().is_some_and(|retained| retained.failed) {
        accounting.retry_deferred();
    }
    let retained_bytes = result
        .as_ref()
        .filter(|_| !discard_non_retryable_p2)
        .map(|retained| retained.batch.estimated_memory_bytes())
        .unwrap_or_default();
    let retained_depth = result
        .as_ref()
        .filter(|_| !discard_non_retryable_p2)
        .map(|retained| retained.batch.logical_rows())
        .unwrap_or_default();
    accounting.complete(
        submitted_depth,
        retained_depth,
        submitted_bytes,
        retained_bytes,
    );
    if discard_non_retryable_p2 {
        None
    } else {
        result
    }
}

fn cleanup_discarded_p2_runtime_overlays(
    batch: &PendingBatch,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
) -> usize {
    let Some(runtime_store) = terminal_runtime_store
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    else {
        return 0;
    };
    batch
        .invocation_derived
        .values()
        .filter_map(|derived| derived.terminal_overlay_key.as_ref())
        .filter(|(invoke_id, occurred_at)| {
            runtime_store.remove_persisted_terminal_overlay(invoke_id, occurred_at)
        })
        .count()
}

fn quarantine_system_task_batch(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    batch: &PendingBatch,
    error: &anyhow::Error,
) -> Result<usize> {
    let mut journal = terminal_journal
        .lock()
        .map_err(|_| anyhow::anyhow!("terminal journal lock poisoned"))?;
    let journal = journal
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("terminal journal unavailable for quarantine"))?;
    let error = format!("{error:#}");
    let finishes = batch.system_task_finishes.values().collect::<Vec<_>>();
    journal.quarantine_system_task_finishes(&finishes, &error)?;
    Ok(batch.system_task_finishes.len())
}

fn shutdown_recovery_batch(batch: &PendingBatch) -> Option<PendingBatch> {
    if batch.terminal_invocations.is_empty() && batch.system_task_finishes.is_empty() {
        return None;
    }
    let mut quarantine = PendingBatch {
        terminal_invocations: batch.terminal_invocations.clone(),
        system_task_finishes: batch.system_task_finishes.clone(),
        ..PendingBatch::default()
    };
    quarantine.recalculate_estimates();
    quarantine.enqueued_rows = quarantine.logical_rows();
    Some(quarantine)
}

fn quarantine_shutdown_batch(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: &std::path::Path,
    batch: &PendingBatch,
    error: &str,
) -> Result<()> {
    if batch.terminal_invocations.is_empty() && batch.system_task_finishes.is_empty() {
        return Ok(());
    }
    let error = error.to_string();
    let terminals = batch.terminal_invocations.values().collect::<Vec<_>>();
    let finishes = batch.system_task_finishes.values().collect::<Vec<_>>();
    let mut errors = Vec::new();
    let mut recovery_persisted = false;
    match terminal_journal.lock() {
        Ok(mut guard) => match guard.as_mut() {
            Some(journal) => {
                if let Err(err) = journal.quarantine_terminals(&terminals, &error) {
                    errors.push(format!("terminal quarantine failed: {err:#}"));
                }
                if let Err(err) = journal.quarantine_system_task_finishes(&finishes, &error) {
                    errors.push(format!("system-task quarantine failed: {err:#}"));
                }
                if !errors.is_empty() {
                    match journal.quarantine_shutdown_batch(&terminals, &finishes, &error) {
                        Ok(()) => {
                            journal.remember_shutdown_recovery(&terminals);
                            recovery_persisted = true;
                        }
                        Err(err) => errors.push(format!("shutdown recovery failed: {err:#}")),
                    }
                }
            }
            None => errors.push("terminal journal unavailable for shutdown quarantine".to_string()),
        },
        Err(_) => errors.push("terminal journal lock poisoned".to_string()),
    }
    if recovery_persisted {
        warn!(errors = ?errors, "shutdown quarantine used the journal recovery sink");
        return Ok(());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        match TerminalJournal::quarantine_shutdown_batch_at_database_path(
            database_path,
            &terminals,
            &finishes,
            &error,
        ) {
            Ok(()) => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                {
                    journal.remember_shutdown_recovery(&terminals);
                }
                warn!(
                    errors = ?errors,
                    "shutdown quarantine used the independent recovery sink"
                );
                Ok(())
            }
            Err(fallback_error) => Err(anyhow!(
                "{}; independent recovery sink failed: {fallback_error:#}",
                errors.join("; ")
            )),
        }
    }
}

fn release_shutdown_pending_batch(
    accounting: &PendingQueueAccounting,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: &std::path::Path,
    batch: &PendingBatch,
    reason: &str,
) -> Result<()> {
    let quarantine_error =
        quarantine_shutdown_batch(terminal_journal, database_path, batch, reason).err();
    accounting.release(batch.logical_rows(), batch.estimated_memory_bytes());
    quarantine_error.map_or(Ok(()), Err)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
pub(crate) async fn flush_pending_batch(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    mut batch: PendingBatch,
    reason: FlushReason,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
) -> Option<RetainedBatch> {
    if batch.is_empty() {
        return None;
    }
    let started = Instant::now();
    let enqueued_rows = batch.enqueued_rows;
    let coalesced_rows = batch.coalesced_rows;
    let terminal_invocation_count = batch.terminal_invocations.len();
    let attempt_count = batch.attempt_progress.len();
    let invocation_count = batch.invocation_derived.len();
    let account_touch_count = batch.account_selected_touches.len();
    let system_task_count = batch.system_task_finishes.len();
    let system_task_scope = summarize_system_task_batch_scope(&batch);
    let oldest_age_ms = batch.age().as_millis() as u64;

    let flush_reason = reason.as_str();
    let p2_pending_before_p1 = batch.has_p2();
    let p1_batch = batch.take_p1_terminals();
    if !p1_batch.is_empty() {
        let transaction_id = format!("p1-{}", started.elapsed().as_nanos());
        let permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
            .await;
        let lock_wait_ms = permit.lock_wait().as_millis() as u64;
        let execute_started = Instant::now();
        let initial_result = flush_pending_batch_inner(
            pool,
            &p1_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            terminal_projection_hub,
            dashboard_reconcile_gate,
        )
        .await;
        let mut poison_record_count = 0_usize;
        let p1_result = match initial_result {
            Err(err) if !is_sqlite_lock_error(&err) => {
                let mut deferred = PendingBatch::default();
                let mut isolation_error = None;
                for terminal in p1_batch.terminal_invocations.values() {
                    let mut singleton = PendingBatch::default();
                    singleton.push(SqliteBatchWrite::TerminalInvocation(terminal.clone()));
                    match flush_pending_batch_inner(
                        pool,
                        &singleton,
                        pricing_catalog,
                        prompt_cache_conversation_cache,
                        terminal_runtime_store,
                        dashboard_activity_snapshot_cache,
                        terminal_projection_hub,
                        dashboard_reconcile_gate,
                    )
                    .await
                    {
                        Ok(singleton_deferred) => deferred.merge_p2(singleton_deferred),
                        Err(singleton_err) if !is_sqlite_lock_error(&singleton_err) => {
                            let quarantine_result = terminal_journal
                                .lock()
                                .ok()
                                .and_then(|mut journal| {
                                    journal.as_mut().map(|journal| {
                                        journal.quarantine(terminal, &format!("{singleton_err:#}"))
                                    })
                                })
                                .unwrap_or_else(|| {
                                    Err(anyhow!("terminal journal unavailable for quarantine"))
                                });
                            if let Err(quarantine_err) = quarantine_result {
                                isolation_error = Some(quarantine_err);
                                break;
                            }
                            poison_record_count = poison_record_count.saturating_add(1);
                            warn!(
                                invoke_id = %terminal.record.invoke_id,
                                occurred_at = %terminal.record.occurred_at,
                                error = %singleton_err,
                                poison_record_count,
                                "quarantined deterministic P1 terminal record"
                            );
                        }
                        Err(singleton_err) => {
                            isolation_error = Some(singleton_err);
                            break;
                        }
                    }
                }
                match isolation_error {
                    Some(error) => Err(error),
                    None => Ok(deferred),
                }
            }
            result => result,
        };
        match p1_result {
            Ok(deferred) => {
                debug!(
                    write_class = permit.write_class(),
                    transaction_id,
                    batch_rows = p1_batch.logical_rows(),
                    batch_bytes = p1_batch.estimated_memory_bytes(),
                    lock_wait_ms,
                    execute_ms = execute_started.elapsed().as_millis() as u64,
                    poison_record_count,
                    "proxy sqlite coordinated P1 batch committed"
                );
                accounting.transfer_p1_to_p2(deferred.estimated_memory_bytes());
                if let Ok(mut journal) = terminal_journal.lock()
                    && let Some(journal) = journal.as_mut()
                {
                    for terminal in p1_batch.terminal_invocations.values() {
                        journal.acknowledge(
                            &terminal.record.invoke_id,
                            &terminal.record.occurred_at,
                            terminal.raw_capture,
                        );
                    }
                }
                batch.merge_p2(deferred);
            }
            Err(err) => {
                crate::db_pressure::global_db_pressure_gate()
                    .record_error("sqlite_batch_writer_p1", &err);
                warn!(
                    error = %err,
                    flush_priority = "P1",
                    terminal_invocation_count,
                    oldest_age_ms,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    flush_reason,
                    write_class = permit.write_class(),
                    transaction_id,
                    batch_rows = p1_batch.logical_rows(),
                    batch_bytes = p1_batch.estimated_memory_bytes(),
                    lock_wait_ms,
                    execute_ms = execute_started.elapsed().as_millis() as u64,
                    "sqlite batch writer P1 terminal flush failed"
                );
                batch.terminal_invocations = p1_batch.terminal_invocations;
                batch.recalculate_estimates();
                return Some(RetainedBatch::new(batch, true));
            }
        }
    }

    if batch.is_empty() {
        return None;
    }
    if !p2_pending_before_p1 && !reason.bypass_pressure_gate() {
        return Some(RetainedBatch::new(batch, false));
    }
    let observed_eligibility_generation =
        crate::db_pressure::global_db_pressure_gate().eligibility_generation();
    let Some(mut write_permit) =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    else {
        accounting.p2_pressure_deferred();
        return Some(RetainedBatch::p2_deferred(
            batch,
            P2DeferReason::BackgroundBusy {
                observed_generation: observed_eligibility_generation,
            },
        ));
    };

    let permit = if reason.bypass_pressure_gate() {
        None
    } else {
        match crate::db_pressure::global_db_pressure_gate()
            .try_begin_background("sqlite_batch_writer_p2")
        {
            Ok(permit) => Some(permit),
            Err(deny_reason) => {
                if matches!(
                    deny_reason,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                ) {
                    write_permit.suppress_background_eligibility_wakeup();
                }
                drop(write_permit);
                accounting.p2_pressure_deferred();
                debug!(
                    deny_reason = %deny_reason,
                    flush_priority = "P2",
                    p2_deferred_count = batch.logical_rows(),
                    "sqlite batch writer deferred P2 flush because pressure gate is closed"
                );
                let reason = match deny_reason {
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => {
                        P2DeferReason::PressureCooldown(remaining_ms)
                    }
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                        P2DeferReason::BackgroundBusy {
                            observed_generation: observed_eligibility_generation,
                        }
                    }
                };
                return Some(RetainedBatch::p2_deferred(batch, reason));
            }
        }
    };
    accounting.p2_attempted();

    // A deterministic failure in a derived write must not retain unrelated
    // system-task completions forever. Flush those completions separately so
    // their final state is durable even when another P2 write is discarded.
    let system_task_batch = if !batch.system_task_finishes.is_empty()
        && batch.logical_rows() > batch.system_task_finishes.len()
    {
        let mut system_task_batch = PendingBatch {
            oldest_at: batch.oldest_at,
            ..PendingBatch::default()
        };
        system_task_batch.system_task_finishes = std::mem::take(&mut batch.system_task_finishes);
        system_task_batch.recalculate_estimates();
        system_task_batch.enqueued_rows = system_task_batch.logical_rows();
        batch.recalculate_estimates();
        Some(system_task_batch)
    } else {
        None
    };

    let mut deferred_batch = PendingBatch::default();
    let mut system_task_failure = None;
    let mut system_task_lock_failure = false;
    if let Some(system_task_batch) = system_task_batch.as_ref() {
        match flush_pending_batch_inner(
            pool,
            system_task_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            terminal_projection_hub,
            dashboard_reconcile_gate,
        )
        .await
        {
            Ok(system_task_deferred) => deferred_batch.merge_p2(system_task_deferred),
            Err(err) => {
                crate::db_pressure::global_db_pressure_gate()
                    .record_error("sqlite_batch_writer_p2", &err);
                warn!(
                    error = %err,
                    flush_priority = "P2",
                    p2_deferred_count = system_task_batch.logical_rows(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    flush_reason,
                    system_task_scope = %summarize_system_task_batch_scope(system_task_batch),
                    "sqlite batch writer P2 system-task flush failed"
                );
                system_task_lock_failure = is_sqlite_lock_error(&err);
                system_task_failure = Some(err);
            }
        }
    }
    let system_task_retryable_failure = system_task_failure
        .as_ref()
        .is_some_and(crate::db_pressure::is_db_pressure_error);

    match flush_pending_batch_inner(
        pool,
        &batch,
        pricing_catalog,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        terminal_projection_hub,
        dashboard_reconcile_gate,
    )
    .await
    {
        Ok(main_deferred) => {
            deferred_batch.merge_p2(main_deferred);
            if let Some(system_task_error) = system_task_failure.as_ref() {
                if !system_task_retryable_failure {
                    let system_task_batch_ref = system_task_batch
                        .as_ref()
                        .expect("system task failure must retain its isolated batch");
                    let quarantine_result = quarantine_system_task_batch(
                        terminal_journal,
                        system_task_batch_ref,
                        system_task_error,
                    );
                    if let Err(quarantine_error) = quarantine_result {
                        warn!(
                            error = %quarantine_error,
                            flush_priority = "P2",
                            system_task_scope = %summarize_system_task_batch_scope(
                                system_task_batch_ref
                            ),
                            "system-task quarantine failed; retaining completion for retry"
                        );
                        let mut retry_batch = system_task_batch
                            .expect("system task failure must retain its isolated batch");
                        retry_batch.merge_p2(deferred_batch);
                        drop(permit);
                        return Some(RetainedBatch::p2_failed(retry_batch, true, false));
                    }
                    warn!(
                        flush_priority = "P2",
                        system_task_scope = %summarize_system_task_batch_scope(system_task_batch_ref),
                        "quarantined deterministic system-task completion after failed finalization"
                    );
                    drop(permit);
                    return if deferred_batch.is_empty() {
                        None
                    } else {
                        Some(RetainedBatch::new(deferred_batch, false))
                    };
                }
                let mut retry_batch =
                    system_task_batch.expect("system task failure must retain its isolated batch");
                retry_batch.merge_p2(deferred_batch);
                drop(permit);
                return Some(RetainedBatch::p2_failed(
                    retry_batch,
                    system_task_retryable_failure,
                    system_task_lock_failure,
                ));
            }
        }
        Err(err) => {
            crate::db_pressure::global_db_pressure_gate()
                .record_error("sqlite_batch_writer_p2", &err);
            warn!(
                error = %err,
                flush_priority = "P2",
                p2_deferred_count = batch.logical_rows(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                flush_reason,
                "sqlite batch writer P2 flush failed"
            );
            drop(permit);
            if !crate::db_pressure::is_db_pressure_error(&err)
                && let Some(system_task_error) = system_task_failure.as_ref()
            {
                cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
                let retry_batch =
                    system_task_batch.expect("system task failure must retain its isolated batch");
                if !system_task_retryable_failure
                    && quarantine_system_task_batch(
                        terminal_journal,
                        &retry_batch,
                        system_task_error,
                    )
                    .is_ok()
                {
                    return None;
                }
                return Some(RetainedBatch::p2_failed(
                    retry_batch,
                    system_task_retryable_failure,
                    system_task_lock_failure,
                ));
            }
            if let Some(system_task_batch) = system_task_batch {
                if let Some(system_task_error) = system_task_failure.as_ref() {
                    if !system_task_retryable_failure
                        && quarantine_system_task_batch(
                            terminal_journal,
                            &system_task_batch,
                            system_task_error,
                        )
                        .is_ok()
                    {
                        return Some(RetainedBatch::p2_failed(
                            batch,
                            crate::db_pressure::is_db_pressure_error(&err),
                            is_sqlite_lock_error(&err),
                        ));
                    }
                    let mut retry_batch = system_task_batch;
                    retry_batch.merge_p2(batch);
                    return Some(RetainedBatch::p2_failed(
                        retry_batch,
                        system_task_retryable_failure
                            || crate::db_pressure::is_db_pressure_error(&err),
                        system_task_lock_failure || is_sqlite_lock_error(&err),
                    ));
                }
                if !crate::db_pressure::is_db_pressure_error(&err) {
                    cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
                    return None;
                }
                return Some(RetainedBatch::p2_failed(
                    batch,
                    true,
                    is_sqlite_lock_error(&err),
                ));
            }
            let retryable_failure = crate::db_pressure::is_db_pressure_error(&err);
            return Some(RetainedBatch::p2_failed(
                batch,
                retryable_failure,
                is_sqlite_lock_error(&err),
            ));
        }
    }
    drop(write_permit);
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        warn!(
            enqueued_rows,
            coalesced_rows,
            terminal_invocation_count,
            attempt_count,
            invocation_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            flush_reason,
            "sqlite batch writer flush was slow"
        );
    } else {
        debug!(
            enqueued_rows,
            coalesced_rows,
            terminal_invocation_count,
            attempt_count,
            invocation_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            flush_reason,
            "sqlite batch writer flushed derived writes"
        );
    }
    if deferred_batch.is_empty() {
        None
    } else {
        Some(RetainedBatch::new(deferred_batch, false))
    }
}

pub(crate) fn summarize_system_task_batch_scope(batch: &PendingBatch) -> String {
    let mut values = batch
        .system_task_finishes
        .values()
        .take(3)
        .map(|finish| {
            format!(
                "{}:{}:{}",
                finish.task_kind.as_str(),
                finish.trigger_kind,
                finish.status.as_str()
            )
        })
        .collect::<Vec<_>>();
    if batch.system_task_finishes.len() > values.len() {
        values.push(format!(
            "+{}",
            batch.system_task_finishes.len() - values.len()
        ));
    }
    values.join(",")
}

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
pub(crate) async fn flush_pending_batch_inner(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
) -> Result<PendingBatch> {
    let mut deferred_batch = PendingBatch::default();
    let mut should_invalidate_prompt_cache_conversations = false;
    let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
    let mut persisted_terminals = Vec::with_capacity(batch.terminal_invocations.len());
    if !batch.terminal_invocations.is_empty() {
        let mut terminal_tx = pool.begin().await?;
        for terminal in batch.terminal_invocations.values() {
            let persisted = if terminal.raw_capture {
                let capture_started = terminal.capture_started.unwrap_or_else(Instant::now);
                persist_proxy_capture_record_tx(
                    terminal_tx.as_mut(),
                    capture_started,
                    terminal.record.clone(),
                    false,
                )
                .await
                .with_context(|| "flush terminal raw proxy invocation")?
            } else {
                persist_proxy_capture_runtime_record_tx(
                    terminal_tx.as_mut(),
                    terminal.record.clone(),
                    false,
                )
                .await
                .with_context(|| "flush terminal runtime proxy invocation")?
            };
            let derived_identity = if let Some(persisted) = persisted.as_ref() {
                if persisted
                    .prompt_cache_key
                    .as_deref()
                    .is_some_and(|key| !key.trim().is_empty())
                {
                    should_invalidate_prompt_cache_conversations = true;
                }
                Some((persisted.id, persisted.occurred_at.clone()))
            } else {
                let identity = load_persisted_invocation_identity_tx(
                    terminal_tx.as_mut(),
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                )
                .await?;
                identity.map(|row| (row.id, terminal.record.occurred_at.clone()))
            };
            let (invocation_id, occurred_at) = derived_identity.ok_or_else(|| {
            anyhow!(
                "terminal write completed without a persisted identity: invoke_id={} occurred_at={}",
                terminal.record.invoke_id,
                terminal.record.occurred_at
            )
        })?;
            let payload_metadata =
                persisted
                    .as_ref()
                    .map(|record| crate::TerminalPayloadMetadata {
                        prompt_cache_key: record.prompt_cache_key.clone(),
                        upstream_account_id: record.upstream_account_id,
                    });
            persisted_terminals.push((terminal, invocation_id, occurred_at, payload_metadata));
        }
        terminal_tx.commit().await?;
    }

    for (terminal, invocation_id, occurred_at, payload_metadata) in persisted_terminals {
        deferred_batch.add_startup_backfill_wake_tasks(&terminal.startup_backfill_tasks);
        let dashboard_cache = dashboard_activity_snapshot_cache
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(cache) = dashboard_cache {
            acknowledge_dashboard_activity_terminal_record(
                &cache,
                &terminal.record.invoke_id,
                &terminal.record.occurred_at,
                invocation_id,
                terminal.dashboard_terminal_sequence,
            )
            .await;
        }
        if let Some(hub) = terminal_projection_hub
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
        {
            if terminal.terminal_projection_event_ids.is_empty() {
                hub.acknowledge_persisted(
                    None,
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                    invocation_id,
                );
            } else {
                for event_id in &terminal.terminal_projection_event_ids {
                    hub.acknowledge_persisted(
                        Some(*event_id),
                        &terminal.record.invoke_id,
                        &terminal.record.occurred_at,
                        invocation_id,
                    );
                }
            }
        }
        // The persistence helper already materialized this payload into the returned invocation.
        // Reuse those fields instead of parsing the same terminal payload again on the P2 path.
        let payload_metadata = payload_metadata.unwrap_or_else(|| {
            crate::terminal_payload_metadata(terminal.record.payload.as_deref())
        });
        if payload_metadata
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
        {
            should_invalidate_prompt_cache_conversations = true;
        }
        deferred_batch.push(SqliteBatchWrite::InvocationDerived(
            BatchedInvocationDerivedWrites {
                invocation_id,
                occurred_at,
                upstream_account_id: payload_metadata.upstream_account_id,
                terminal_overlay_key: Some((
                    terminal.record.invoke_id.clone(),
                    terminal.record.occurred_at.clone(),
                )),
            },
        ));
    }

    if !batch.startup_backfill_wake_tasks.is_empty() {
        let pricing_catalog = if let Some(pricing_catalog) = pricing_catalog {
            Some(pricing_catalog.read().await.clone())
        } else {
            None
        };
        let wake_tasks = batch
            .startup_backfill_wake_tasks
            .iter()
            .copied()
            .filter(|task| {
                let available = pricing_catalog.is_some()
                    || !matches!(task, StartupBackfillTask::ProxyCost);
                if !available {
                    warn!(
                        task = task.name(),
                        "skipping startup backfill wake because its runtime pricing catalog is unavailable"
                    );
                }
                available
            })
            .collect::<Vec<_>>();
        wake_startup_backfill_tasks_with_pricing_catalog(
            pool,
            &wake_tasks,
            pricing_catalog.as_ref(),
            "terminal_payload_repair_input",
        )
        .await?;
    }

    if batch.attempt_progress.is_empty()
        && batch.invocation_derived.is_empty()
        && batch.account_selected_touches.is_empty()
        && batch.system_task_finishes.is_empty()
    {
        if should_invalidate_prompt_cache_conversations
            && let Some(cache) = prompt_cache_conversation_cache
        {
            invalidate_prompt_cache_conversations_cache(cache).await;
        }
        return Ok(deferred_batch);
    }

    let mut tx = pool.begin().await?;

    for progress in batch.attempt_progress.values() {
        sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                phase = ?2,
                connect_latency_ms = CASE
                    WHEN ?4 IS NULL THEN connect_latency_ms
                    WHEN connect_latency_ms IS NULL OR connect_latency_ms < ?4 THEN ?4
                    ELSE connect_latency_ms
                END,
                first_byte_latency_ms = CASE
                    WHEN ?5 IS NULL THEN first_byte_latency_ms
                    WHEN first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5 THEN ?5
                    ELSE first_byte_latency_ms
                END,
                compact_support_status = COALESCE(?6, compact_support_status),
                compact_support_reason = COALESCE(?7, compact_support_reason)
            WHERE id = ?1
              AND status = ?3
              AND finished_at IS NULL
              AND (
                    COALESCE(phase, '') <> ?2
                    OR (?4 IS NOT NULL AND (connect_latency_ms IS NULL OR connect_latency_ms < ?4))
                    OR (?5 IS NOT NULL AND (first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5))
                    OR (?6 IS NOT NULL AND COALESCE(compact_support_status, '') <> ?6)
                    OR (?7 IS NOT NULL AND COALESCE(compact_support_reason, '') <> ?7)
                  )
            "#,
        )
        .bind(progress.attempt_id)
        .bind(&progress.phase)
        .bind(progress.pending_status)
        .bind(progress.connect_latency_ms)
        .bind(progress.first_byte_latency_ms)
        .bind(progress.compact_support_status.as_deref())
        .bind(progress.compact_support_reason.as_deref())
        .execute(tx.as_mut())
        .await?;
    }

    let mut terminal_overlay_keys = Vec::new();
    if !batch.invocation_derived.is_empty() {
        let target_invocation_id = batch
            .invocation_derived
            .keys()
            .next_back()
            .copied()
            .unwrap_or_default();
        let live_rollup_cursor_before =
            load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        replay_live_invocation_hourly_rollups_until_tx(tx.as_mut(), target_invocation_id).await?;
        let live_rollup_cursor_after =
            load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        let skipped_terminal_ids = batch
            .invocation_derived
            .keys()
            .filter(|invocation_id| **invocation_id <= live_rollup_cursor_before)
            .copied()
            .collect::<Vec<_>>();
        if !skipped_terminal_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &skipped_terminal_ids)
                .await?;
        }
        for derived in batch.invocation_derived.values() {
            if derived.invocation_id > live_rollup_cursor_after {
                deferred_batch.push(SqliteBatchWrite::InvocationDerived(derived.clone()));
                continue;
            }
            if let Some(key) = derived.terminal_overlay_key.clone() {
                terminal_overlay_keys.push(key);
            }
            crate::touch_upstream_account_last_activity_tx(
                tx.as_mut(),
                &derived.occurred_at,
                derived.upstream_account_id,
            )
            .await?;
        }
    }

    for touch in batch.account_selected_touches.values() {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = CASE
                    WHEN last_selected_at IS NULL OR last_selected_at < ?2 THEN ?2
                    ELSE last_selected_at
                END,
                updated_at = CASE
                    WHEN updated_at IS NULL OR updated_at < ?2 THEN ?2
                    ELSE updated_at
                END
            WHERE id = ?1
            "#,
        )
        .bind(touch.account_id)
        .bind(&touch.selected_at)
        .execute(tx.as_mut())
        .await?;
    }

    for finish in batch.system_task_finishes.values() {
        sqlx::query(
            r#"
            UPDATE system_task_runs
            SET status = ?1,
                summary = COALESCE(?2, summary),
                detail = ?3,
                finished_at = ?4,
                duration_ms = ?5
            WHERE id = ?6
            "#,
        )
        .bind(finish.status.as_str())
        .bind(finish.summary.as_deref())
        .bind(finish.detail.as_deref())
        .bind(&finish.finished_at)
        .bind(finish.duration_ms)
        .bind(finish.run_id)
        .execute(tx.as_mut())
        .await?;
    }

    tx.commit().await?;

    if !terminal_overlay_keys.is_empty()
        && let Some(runtime_store) = terminal_runtime_store
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
    {
        for (invoke_id, occurred_at) in terminal_overlay_keys {
            let removed = runtime_store.remove_persisted_terminal_overlay(&invoke_id, &occurred_at);
            debug!(
                invoke_id = %invoke_id,
                occurred_at = %occurred_at,
                terminal_runtime_overlay_removed_after_derived_flush = removed,
                "removed persisted terminal record from memory runtime overlay after derived writes"
            );
        }
    }

    if should_invalidate_prompt_cache_conversations
        && let Some(cache) = prompt_cache_conversation_cache
    {
        invalidate_prompt_cache_conversations_cache(cache).await;
    }
    Ok(deferred_batch)
}

pub(crate) async fn replay_live_invocation_hourly_rollups_until_tx(
    tx: &mut SqliteConnection,
    target_invocation_id: i64,
) -> Result<u64> {
    let cursor = load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    if cursor >= target_invocation_id {
        return Ok(0);
    }
    replay_live_invocation_hourly_rollups_tx(tx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p1_retry_backoff_is_bounded_and_new_work_does_not_reset_deadline() {
        let mut retry = P1RetryState::default();
        let expected = [250_u128, 500, 1_000, 2_000, 5_000, 5_000];
        for (generation, expected_ms) in expected.into_iter().enumerate() {
            let delay = retry.failed(0);
            assert_eq!(delay.as_millis(), expected_ms, "generation {generation}");
            let due_at = retry.due_at.expect("failed attempt sets deadline");
            assert!(!retry.ready(Instant::now()));
            assert_eq!(
                retry.due_at,
                Some(due_at),
                "enqueue does not mutate retry state"
            );
        }
        retry.succeeded();
        assert!(retry.ready(Instant::now()));
        assert_eq!(retry.generation, 0);
    }

    #[test]
    fn p2_schedule_coalesces_and_separates_pressure_from_lock_retries() {
        let accounting = PendingQueueAccounting::default();
        let mut schedule = P2ScheduleState::default();
        let now = Instant::now();
        schedule.arm_if_idle(now);
        let initial_due = schedule.due_at;
        schedule.arm_if_idle(now + Duration::from_millis(100));
        assert_eq!(
            schedule.due_at, initial_due,
            "new work must not extend deadline"
        );

        accounting.p2_pressure_deferred();
        schedule.defer_pressure(
            Duration::from_secs(30),
            P2WakeReason::PressureCooldownElapsed,
        );
        accounting.update_p2_schedule(&schedule);
        let pressure = accounting.snapshot();
        assert_eq!(pressure.retry_count, 0);
        assert_eq!(pressure.p2_pressure_defer_count, 1);
        assert!(pressure.p2_next_attempt_in_ms >= 29_000);

        let delay = schedule.failed(0);
        accounting.p2_lock_retried();
        accounting.retry_deferred();
        accounting.update_p2_schedule(&schedule);
        let locked = accounting.snapshot();
        assert_eq!(delay, Duration::from_millis(250));
        assert_eq!(locked.retry_count, 1);
        assert_eq!(locked.p2_lock_retry_count, 1);
        assert_eq!(locked.p2_wake_reason.as_deref(), Some("lock_retry"));
    }

    #[tokio::test]
    async fn p2_pressure_defer_waits_for_its_deadline_without_a_20ms_retry() {
        let mut schedule = P2ScheduleState::default();
        schedule.defer_pressure(
            Duration::from_millis(80),
            P2WakeReason::PressureCooldownElapsed,
        );

        assert!(
            tokio::time::timeout(
                Duration::from_millis(40),
                wait_for_p2_deadline(schedule.due_at),
            )
            .await
            .is_err(),
            "P2 pressure defer must not wake on the 20ms P1 ticker"
        );
        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_p2_deadline(schedule.due_at),
        )
        .await
        .expect("P2 pressure deadline should eventually wake");
    }

    #[test]
    fn queued_p1_write_is_drained_before_ready_p2_deadline() {
        let accounting = PendingQueueAccounting::default();
        let mut pending = PendingBatch::default();
        pending.push(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_998,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        ));
        let mut schedule = P2ScheduleState::default();
        schedule.wake_background_eligible();
        let queued_p1_count = AtomicUsize::new(0);
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .try_send(SqliteBatchWrite::TerminalInvocation(
                terminal_write_for_coalescing("queued-p1-priority", None),
            ))
            .expect("queue P1 terminal");
        drain_queued_writes_before_dispatch(
            &mut receiver,
            &mut pending,
            &accounting,
            &mut schedule,
            SQLITE_BATCH_MAX_ROWS,
            &queued_p1_count,
        );

        assert!(
            receiver.is_empty(),
            "queued P1 must be classified before checking the P2 deadline"
        );
        assert_eq!(pending.terminal_invocations.len(), 1);
        assert!(p2_deadline_ready(
            &pending,
            &schedule,
            &queued_p1_count,
            &P1RetryState::default()
        ));
    }

    #[test]
    fn queued_p2_does_not_block_ready_p2_deadline() {
        let accounting = PendingQueueAccounting::default();
        let mut pending = PendingBatch::default();
        pending.push(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_997,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        ));
        let mut schedule = P2ScheduleState::default();
        schedule.wake_background_eligible();
        let queued_p1_count = AtomicUsize::new(0);
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .try_send(SqliteBatchWrite::AccountSelectedTouch(
                BatchedAccountSelectedTouch {
                    account_id: 999_996,
                    selected_at: "2026-08-10T12:00:01Z".to_string(),
                },
            ))
            .expect("queue P2 write");

        drain_queued_writes_before_dispatch(
            &mut receiver,
            &mut pending,
            &accounting,
            &mut schedule,
            SQLITE_BATCH_MAX_ROWS,
            &queued_p1_count,
        );

        assert!(receiver.is_empty());
        assert!(p2_deadline_ready(
            &pending,
            &schedule,
            &queued_p1_count,
            &P1RetryState::default()
        ));
    }

    #[test]
    fn p1_retry_backoff_blocks_p2_deadline() {
        let mut pending = PendingBatch::default();
        pending.push(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_995,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        ));
        let mut schedule = P2ScheduleState::default();
        schedule.wake_background_eligible();
        let queued_p1_count = AtomicUsize::new(0);
        let mut p1_retry = P1RetryState::default();
        p1_retry.failed(0);

        assert!(!p2_deadline_ready(
            &pending,
            &schedule,
            &queued_p1_count,
            &p1_retry
        ));
    }

    #[tokio::test]
    async fn p2_eligibility_wake_resumes_a_deferred_flush() {
        let mut schedule = P2ScheduleState::default();
        schedule.arm_if_idle(Instant::now());
        schedule.defer_until_background_eligible();

        assert!(!schedule.ready(Instant::now() + Duration::from_secs(30)));
        assert_eq!(schedule.next_attempt_in_ms(), 0);
        assert_eq!(schedule.wake_reason, Some(P2WakeReason::BackgroundEligible));

        schedule.arm_if_idle(Instant::now());
        assert!(schedule.due_at.is_none(), "new P2 work must only coalesce");

        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let permit = gate
            .try_begin_background("test_p2_eligibility_wake")
            .expect("acquire background permit");
        let observed_generation = gate.eligibility_generation();
        let wait_for_eligibility = gate.wait_for_eligibility_change(observed_generation);
        drop(permit);
        tokio::time::timeout(Duration::from_millis(100), wait_for_eligibility)
            .await
            .expect("releasing background capacity should notify P2 eligibility waiters");

        schedule.wake_background_eligible();
        assert!(schedule.ready(Instant::now()));
        tokio::time::timeout(
            Duration::from_millis(20),
            wait_for_p2_deadline(schedule.due_at),
        )
        .await
        .expect("eligibility wake should make P2 dispatch immediately");
    }

    #[test]
    fn pending_queue_accounting_clamps_underflow_and_degrades_health() {
        let accounting = PendingQueueAccounting::default();

        accounting.complete(0, 0, 64, 0);

        let snapshot = accounting.snapshot();
        assert_eq!(snapshot.pending_depth, 0);
        assert_eq!(snapshot.pending_bytes, 0);
        assert_eq!(snapshot.state, "degraded");
        assert_eq!(snapshot.invariant_violation_count, 1);
        let violation = snapshot
            .last_invariant_violation
            .expect("underflow should retain actionable telemetry");
        assert_eq!(violation.operation, "completion");
        assert_eq!(violation.expected_bytes, 64);
        assert_eq!(violation.actual_bytes, 0);
    }

    #[test]
    fn pending_queue_accounting_preserves_retry_transfer_and_retained_bytes() {
        let accounting = PendingQueueAccounting::default();

        accounting.enqueue(120);
        accounting.retry_deferred();
        accounting.replace_batch(1, 1, 120, 96);
        accounting.transfer_p1_to_p2(40);
        accounting.complete(1, 1, 96, 40);

        let snapshot = accounting.snapshot();
        assert_eq!(snapshot.pending_depth, 1);
        assert_eq!(snapshot.pending_bytes, 40);
        assert_eq!(snapshot.transfer_bytes, 40);
        assert_eq!(snapshot.retry_count, 1);
        assert_eq!(snapshot.state, "healthy");
        assert!(snapshot.last_invariant_violation.is_none());

        accounting.complete(1, 0, 40, 0);
        let completed = accounting.snapshot();
        assert_eq!(completed.pending_depth, 0);
        assert_eq!(completed.pending_bytes, 0);
        assert_eq!(completed.state, "healthy");
    }

    #[test]
    fn pending_queue_accounting_rolls_back_failed_sender_admission() {
        let accounting = PendingQueueAccounting::default();

        accounting.enqueue(72);
        accounting.rollback_enqueue(72);

        let snapshot = accounting.snapshot();
        assert_eq!(snapshot.pending_depth, 0);
        assert_eq!(snapshot.pending_bytes, 0);
        assert_eq!(snapshot.state, "healthy");
        assert_eq!(snapshot.invariant_violation_count, 0);
    }
    use axum::http::StatusCode;
    use sqlx::SqlitePool;

    fn attempt_trace(invoke_id: &str) -> PoolUpstreamAttemptTraceContext {
        PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: "2026-07-01 10:00:00".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some(format!("{invoke_id}-sticky")),
            requester_ip: Some("192.168.31.6".to_string()),
            upstream_base_url_host: None,
            request_model: None,
        }
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect sqlite memory pool");
        ensure_schema(&pool).await.expect("ensure schema");
        pool
    }

    #[tokio::test]
    async fn normal_p2_schedule_becomes_ready_after_its_deadline() {
        let mut schedule = P2ScheduleState::default();
        schedule.arm_if_idle(Instant::now());
        tokio::time::timeout(
            SQLITE_P2_COALESCE_INTERVAL + Duration::from_millis(100),
            wait_for_p2_deadline(schedule.due_at),
        )
        .await
        .expect("normal P2 coalescing deadline should elapse");

        assert!(
            schedule.ready(Instant::now()),
            "normal P2 schedule should be eligible after its fixed deadline"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn p1_queued_after_a_deep_p2_backlog_runs_first() {
        const ACCOUNT_ID: i64 = 999_998;
        const P1_INVOKE_ID: &str = "deep-deferred-p2-p1-priority";

        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
            )
            VALUES (999998, 'api_key', 'codex', 'Priority Test', 'active', 1, NULL, '2026-08-10T12:00:00Z', '2026-08-10T12:00:00Z')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed P2 account");
        for offset in 1..=SQLITE_BATCH_MAX_ROWS {
            let account_id = ACCOUNT_ID + offset as i64;
            sqlx::query(
                r#"
                INSERT INTO pool_upstream_accounts (
                    id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
                )
                VALUES (?1, 'api_key', 'codex', 'Priority Test', 'active', 1, NULL, '2026-08-10T12:00:00Z', '2026-08-10T12:00:00Z')
                "#,
            )
            .bind(account_id)
            .execute(&pool)
            .await
            .expect("seed deep P2 account");
        }
        sqlx::query("CREATE TABLE batch_writer_dispatch_order (write_class TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create dispatch order table");
        sqlx::query(
            r#"
            CREATE TRIGGER batch_writer_p1_dispatch_order
            AFTER INSERT ON codex_invocations
            WHEN NEW.invoke_id = 'deep-deferred-p2-p1-priority'
            BEGIN
                INSERT INTO batch_writer_dispatch_order (write_class) VALUES ('p1');
            END
            "#,
        )
        .execute(&pool)
        .await
        .expect("create P1 dispatch order trigger");
        sqlx::query(
            r#"
            CREATE TRIGGER batch_writer_p2_dispatch_order
            AFTER UPDATE OF last_selected_at ON pool_upstream_accounts
            WHEN NEW.id BETWEEN 999998 AND 1000030
            BEGIN
                INSERT INTO batch_writer_dispatch_order (write_class) VALUES ('p2');
            END
            "#,
        )
        .execute(&pool)
        .await
        .expect("create P2 dispatch order trigger");

        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            CancellationToken::new(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );

        assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: ACCOUNT_ID,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        )));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if writer.accounting_snapshot().p2_wake_reason.as_deref()
                    == Some("coalesced_deadline")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("initial P2 batch should arm its fixed coalescing deadline");

        for offset in 0..=SQLITE_BATCH_MAX_ROWS {
            assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
                BatchedAccountSelectedTouch {
                    account_id: ACCOUNT_ID + offset as i64,
                    selected_at: format!("2026-08-10T12:01:{offset:02}Z"),
                },
            )));
        }
        assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
            terminal_write_for_coalescing(P1_INVOKE_ID, None),
        )));

        // Keep the single-threaded writer asleep until its fixed P2 deadline is overdue.
        std::thread::sleep(SQLITE_P2_COALESCE_INTERVAL + Duration::from_millis(10));
        let order = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let order = sqlx::query_scalar::<_, String>(
                    "SELECT write_class FROM batch_writer_dispatch_order ORDER BY rowid",
                )
                .fetch_all(&pool)
                .await
                .expect("load writer dispatch order");
                if order.len() >= 2 {
                    return order;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
        writer.shutdown_and_drain().await;

        let order = order.expect("writer should dispatch both P1 and P2 work");
        assert_eq!(
            order.first().map(String::as_str),
            Some("p1"),
            "the writer must classify the full queued snapshot before dispatching overdue P2 work"
        );
        assert!(
            order.iter().skip(1).all(|class| class == "p2"),
            "all remaining writes should be bounded P2 chunks"
        );
    }

    async fn pending_attempt(pool: &SqlitePool, invoke_id: &str) -> PendingPoolAttemptRecord {
        let trace = attempt_trace(invoke_id);
        let pending = begin_pool_upstream_request_attempt(
            pool,
            &trace,
            101,
            "route-primary",
            1,
            1,
            1,
            "2026-07-01 10:00:00",
        )
        .await;
        assert!(
            pending.attempt_id.is_some(),
            "pending attempt should be inserted synchronously"
        );
        pending
    }

    fn terminal_write_for_coalescing(
        invoke_id: &str,
        dashboard_terminal_sequence: Option<u64>,
    ) -> BatchedTerminalInvocationWrite {
        let request_info = RequestCaptureInfo::default();
        BatchedTerminalInvocationWrite {
            record: build_running_proxy_capture_record(
                invoke_id,
                "2026-07-01 10:00:00",
                ProxyCaptureTarget::Responses,
                &request_info,
                None,
                None,
                None,
                false,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                0.0,
                0.0,
                0.0,
                0.0,
            ),
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        }
    }

    #[test]
    fn terminal_batch_coalescing_preserves_the_persistence_ack_sequence() {
        let mut batch = PendingBatch::default();
        let accounting = PendingQueueAccounting::default();
        let mut first = terminal_write_for_coalescing("coalesced-terminal", Some(7));
        first.terminal_projection_event_ids.extend(0..64);
        first
            .startup_backfill_tasks
            .push(StartupBackfillTask::ProxyUsage);
        let first = SqliteBatchWrite::TerminalInvocation(first);
        accounting.enqueue(first.estimated_memory_bytes());
        batch.push_accounted(first, &accounting);
        let mut second = terminal_write_for_coalescing("coalesced-terminal", None);
        second.terminal_projection_event_ids.extend(64..128);
        second
            .startup_backfill_tasks
            .push(StartupBackfillTask::ReasoningEffort);
        let second = SqliteBatchWrite::TerminalInvocation(second);
        accounting.enqueue(second.estimated_memory_bytes());
        batch.push_accounted(second, &accounting);

        let terminal = batch
            .terminal_invocations
            .values()
            .next()
            .expect("coalesced terminal");
        assert_eq!(terminal.dashboard_terminal_sequence, Some(7));
        assert_eq!(
            terminal.terminal_projection_event_ids,
            (0..128).collect::<Vec<_>>()
        );
        assert_eq!(
            terminal.startup_backfill_tasks,
            vec![
                StartupBackfillTask::ReasoningEffort,
                StartupBackfillTask::ProxyUsage,
            ]
        );
        assert_eq!(batch.coalesced_rows, 1);
        assert_eq!(
            batch.estimated_memory_bytes(),
            terminal.estimated_memory_bytes()
        );
        assert_eq!(
            accounting.snapshot().pending_bytes,
            batch.estimated_memory_bytes()
        );
        assert_eq!(accounting.snapshot().pending_depth, batch.logical_rows());
    }

    #[tokio::test]
    async fn p1_terminal_defers_backfill_wake_to_the_coordinated_p2_batch() {
        let pool = test_pool().await;
        let task = StartupBackfillTask::ReasoningEffort;
        let record = crate::tests::test_proxy_capture_record(
            "batch-terminal-backfill-wake",
            "2026-08-09 12:00:00",
        );
        let mut batch = PendingBatch::default();
        batch.push(SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: vec![task],
                record,
            },
        ));
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));

        let deferred = flush_pending_batch_inner(
            &pool,
            &batch,
            None,
            None,
            &terminal_runtime_store,
            &dashboard_activity_snapshot_cache,
            &terminal_projection_hub,
            &dashboard_reconcile_gate,
        )
        .await
        .expect("flush terminal P1 batch");

        let direct_wake_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
        )
        .bind(task.name())
        .fetch_one(&pool)
        .await
        .expect("count direct P1 backfill wakes");
        assert_eq!(
            direct_wake_count, 0,
            "P1 persistence must not directly write startup backfill progress"
        );
        assert_eq!(deferred.startup_backfill_wake_tasks, vec![task]);

        flush_pending_batch_inner(
            &pool,
            &deferred,
            None,
            None,
            &terminal_runtime_store,
            &dashboard_activity_snapshot_cache,
            &terminal_projection_hub,
            &dashboard_reconcile_gate,
        )
        .await
        .expect("flush coordinated P2 backfill wake");

        let progress = load_startup_backfill_progress(&pool, task.name())
            .await
            .expect("load terminal-woken backfill progress");
        assert_eq!(progress.wake_generation, 1);
        assert!(progress.is_due(Utc::now()));
        assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
    }

    #[tokio::test]
    async fn persisted_terminal_wakes_versioned_proxy_cost_backfill_after_p1_commit() {
        let pool = test_pool().await;
        let task = StartupBackfillTask::ProxyCost;
        let persisted_catalog = crate::pricing::load_pricing_catalog(&pool)
            .await
            .expect("load persisted pricing catalog for terminal wake");
        let persisted_task_name =
            startup_backfill_task_progress_key_for_catalog(task, &persisted_catalog);
        let mut runtime_catalog = persisted_catalog;
        runtime_catalog.version = "runtime-terminal-wake".to_string();
        let task_name = startup_backfill_task_progress_key_for_catalog(task, &runtime_catalog);
        assert_ne!(task_name, persisted_task_name);
        let pricing_catalog = Arc::new(RwLock::new(runtime_catalog));
        let record = crate::tests::test_proxy_capture_record(
            "batch-terminal-proxy-cost-backfill-wake",
            "2026-08-09 12:00:00",
        );

        SqliteBatchWriter::flush_for_test_with_pricing_catalog(
            &pool,
            Some(pricing_catalog),
            vec![SqliteBatchWrite::TerminalInvocation(
                BatchedTerminalInvocationWrite {
                    capture_started: None,
                    raw_capture: false,
                    dashboard_terminal_sequence: None,
                    terminal_projection_event_ids: Vec::new(),
                    startup_backfill_tasks: vec![task],
                    record,
                },
            )],
        )
        .await;

        let wake_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let progress = load_startup_backfill_progress(&pool, &task_name)
                .await
                .expect("load terminal-woken ProxyCost backfill progress");
            if progress.wake_generation > 0 {
                assert!(progress.is_due(Utc::now()));
                assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
                break;
            }
            assert!(
                Instant::now() < wake_deadline,
                "terminal P1 commit did not wake the versioned ProxyCost startup backfill task"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let bare_task_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
        )
        .bind(task.name())
        .fetch_one(&pool)
        .await
        .expect("count bare ProxyCost startup backfill progress records");
        assert_eq!(bare_task_count, 0);

        let persisted_task_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
        )
        .bind(&persisted_task_name)
        .fetch_one(&pool)
        .await
        .expect("count persisted-catalog ProxyCost progress records");
        assert_eq!(persisted_task_count, 0);
    }

    #[tokio::test]
    async fn missing_proxy_cost_catalog_does_not_block_other_backfill_wakes() {
        let pool = test_pool().await;
        let unaffected_task = StartupBackfillTask::ReasoningEffort;

        let error = wake_startup_backfill_tasks(
            &pool,
            &[StartupBackfillTask::ProxyCost, unaffected_task],
            "test_missing_proxy_cost_catalog",
        )
        .await
        .expect_err("ProxyCost wake without a catalog should be reported");
        assert!(
            error
                .to_string()
                .contains("requires the runtime pricing catalog"),
            "unexpected missing catalog error: {error:#}"
        );

        let progress = load_startup_backfill_progress(&pool, unaffected_task.name())
            .await
            .expect("load unaffected task progress after missing ProxyCost catalog");
        assert_eq!(progress.wake_generation, 1);
        assert!(progress.is_due(Utc::now()));
    }

    #[tokio::test]
    async fn p1_terminal_batch_rolls_back_the_committed_prefix_on_poison_record() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            CREATE TRIGGER reject_poison_terminal
            BEFORE INSERT ON codex_invocations
            WHEN NEW.invoke_id = 'poison-terminal'
            BEGIN
                SELECT RAISE(ABORT, 'poison terminal');
            END
            "#,
        )
        .execute(&pool)
        .await
        .expect("install poison trigger");

        let mut batch = PendingBatch::default();
        batch.push(SqliteBatchWrite::TerminalInvocation(
            terminal_write_for_coalescing("valid-terminal", Some(1)),
        ));
        batch.push(SqliteBatchWrite::TerminalInvocation(
            terminal_write_for_coalescing("poison-terminal", Some(2)),
        ));
        let runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_cache = Arc::new(std::sync::Mutex::new(None));
        let projection_hub = Arc::new(std::sync::Mutex::new(None));
        let reconcile_gate = Arc::new(Mutex::new(()));

        let error = flush_pending_batch_inner(
            &pool,
            &batch,
            None,
            None,
            &runtime_store,
            &dashboard_cache,
            &projection_hub,
            &reconcile_gate,
        )
        .await
        .expect_err("poison record aborts the complete P1 transaction");
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("poison terminal")),
            "unexpected error chain: {error:#}"
        );

        let persisted = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id IN ('valid-terminal', 'poison-terminal')",
        )
        .fetch_one(&pool)
        .await
        .expect("count terminal rows");
        assert_eq!(persisted, 0, "P1 transaction must not commit a prefix");
    }

    #[tokio::test]
    async fn failed_p1_flush_retains_the_full_accounted_batch() {
        let pool = test_pool().await;
        pool.close().await;

        let accounting = PendingQueueAccounting::default();
        let write = SqliteBatchWrite::TerminalInvocation(terminal_write_for_coalescing(
            "failed-p1-accounting",
            Some(9),
        ));
        accounting.enqueue(write.estimated_memory_bytes());
        let mut batch = PendingBatch::default();
        batch.push_accounted(write, &accounting);
        let submitted_bytes = batch.estimated_memory_bytes();

        let retained = flush_pending_batch_accounted(
            &accounting,
            &pool,
            None,
            batch,
            FlushReason::Barrier,
            None,
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(Mutex::new(())),
            &Arc::new(std::sync::Mutex::new(None)),
        )
        .await
        .expect("failed P1 flush should retain its batch");

        assert!(retained.failed);
        assert_eq!(retained.batch.estimated_memory_bytes(), submitted_bytes);
        let snapshot = accounting.snapshot();
        assert_eq!(snapshot.pending_depth, retained.batch.logical_rows());
        assert_eq!(snapshot.pending_bytes, submitted_bytes);
        assert_eq!(snapshot.retry_count, 0);
        assert_eq!(snapshot.state, "healthy");

        let retried = flush_pending_batch_accounted(
            &accounting,
            &pool,
            None,
            retained.batch,
            FlushReason::Interval,
            None,
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(std::sync::Mutex::new(None)),
            &Arc::new(Mutex::new(())),
            &Arc::new(std::sync::Mutex::new(None)),
        )
        .await
        .expect("failed retained batch should remain available after retry");
        assert!(retried.failed);
        assert_eq!(accounting.snapshot().retry_count, 1);
    }

    #[tokio::test]
    async fn attempt_progress_batch_coalesces_by_attempt_id() {
        let pool = test_pool().await;
        let pending = pending_attempt(&pool, "batch-progress-coalesce").await;
        let attempt_id = pending.attempt_id.expect("attempt id");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![
                SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                    attempt_id,
                    pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                    phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST.to_string(),
                    connect_latency_ms: Some(12.0),
                    first_byte_latency_ms: None,
                    compact_support_status: None,
                    compact_support_reason: None,
                }),
                SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                    attempt_id,
                    pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                    phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
                    connect_latency_ms: Some(18.0),
                    first_byte_latency_ms: Some(33.0),
                    compact_support_status: Some("supported".to_string()),
                    compact_support_reason: Some("cached_probe".to_string()),
                }),
            ],
        )
        .await;

        let row = sqlx::query_as::<
            _,
            (
                String,
                Option<f64>,
                Option<f64>,
                Option<String>,
                Option<String>,
            ),
        >(
            r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms, compact_support_status, compact_support_reason
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced attempt");

        assert_eq!(
            row.0,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE
        );
        assert_eq!(row.1, Some(18.0));
        assert_eq!(row.2, Some(33.0));
        assert_eq!(row.3.as_deref(), Some("supported"));
        assert_eq!(row.4.as_deref(), Some("cached_probe"));
    }

    #[tokio::test]
    async fn attempt_progress_batch_does_not_overwrite_terminal_finalize() {
        let pool = test_pool().await;
        let pending = pending_attempt(&pool, "batch-progress-terminal-cover").await;
        let attempt_id = pending.attempt_id.expect("attempt id");

        finalize_pool_upstream_request_attempt(
            &pool,
            &pending,
            "2026-07-01 10:00:05",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            Some(StatusCode::OK),
            None,
            None,
            None,
            None,
            Some(42.0),
            Some(16.0),
            Some(188.0),
            Some("req_terminal"),
            None,
            None,
        )
        .await
        .expect("finalize attempt synchronously");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string(),
                connect_latency_ms: Some(99.0),
                first_byte_latency_ms: Some(99.0),
                compact_support_status: Some("stale".to_string()),
                compact_support_reason: Some("should_not_apply".to_string()),
            })],
        )
        .await;

        let row = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<i64>,
                Option<f64>,
                Option<f64>,
                Option<f64>,
                Option<String>,
                Option<String>,
            ),
        >(
            r#"
            SELECT
                status,
                phase,
                http_status,
                connect_latency_ms,
                first_byte_latency_ms,
                stream_latency_ms,
                upstream_request_id,
                compact_support_status
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load finalized attempt");

        assert_eq!(row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
        assert_eq!(
            row.1.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
        );
        assert_eq!(row.2, Some(200));
        assert_eq!(row.3, Some(42.0));
        assert_eq!(row.4, Some(16.0));
        assert_eq!(row.5, Some(188.0));
        assert_eq!(row.6.as_deref(), Some("req_terminal"));
        assert_eq!(row.7, None);
    }

    #[tokio::test]
    async fn shutdown_drains_pending_batch_writes() {
        let pool = test_pool().await;
        let pending = pending_attempt(&pool, "batch-progress-shutdown-drain").await;
        let attempt_id = pending.attempt_id.expect("attempt id");
        let shutdown = CancellationToken::new();
        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            shutdown.clone(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );

        assert!(
            writer.enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
                connect_latency_ms: Some(21.0),
                first_byte_latency_ms: Some(34.0),
                compact_support_status: None,
                compact_support_reason: None,
            }))
        );

        shutdown.cancel();
        writer.shutdown_and_drain().await;

        let row = sqlx::query_as::<_, (Option<String>, Option<f64>, Option<f64>)>(
            r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load drained attempt progress");

        assert_eq!(
            row.0.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
        assert_eq!(row.1, Some(21.0));
        assert_eq!(row.2, Some(34.0));
        assert_eq!(writer.stats_snapshot(), (0, 0));
        let accounting = writer.accounting_snapshot();
        assert_eq!(accounting.pending_depth, 0);
        assert_eq!(accounting.pending_bytes, 0);
        assert_eq!(accounting.state, "healthy");
    }

    #[tokio::test]
    async fn flush_now_applies_pending_writes_through_control_path() {
        let pool = test_pool().await;
        let pending = pending_attempt(&pool, "batch-progress-flush-now").await;
        let attempt_id = pending.attempt_id.expect("attempt id");
        let shutdown = CancellationToken::new();
        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            shutdown.clone(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );

        assert!(
            writer.enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
                connect_latency_ms: Some(23.0),
                first_byte_latency_ms: Some(37.0),
                compact_support_status: None,
                compact_support_reason: None,
            }))
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), writer.flush_now(&pool))
            .await
            .expect("flush_now should not be starved by normal write traffic")
            .expect("flush pending write");

        let row = sqlx::query_as::<_, (Option<String>, Option<f64>, Option<f64>)>(
            r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load flushed attempt progress");

        assert_eq!(
            row.0.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
        assert_eq!(row.1, Some(23.0));
        assert_eq!(row.2, Some(37.0));
        writer.shutdown_and_drain().await;
    }

    #[tokio::test]
    async fn flush_now_resets_completed_p2_schedule() {
        let pool = test_pool().await;
        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            CancellationToken::new(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );
        assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_995,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        )));

        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if writer.accounting_snapshot().p2_wake_reason.as_deref()
                    == Some("coalesced_deadline")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("writer should arm the P2 coalescing deadline before FlushNow");

        writer
            .flush_now(&pool)
            .await
            .expect("FlushNow should complete the pending P2 batch");
        let reset = writer.accounting_snapshot();
        assert_eq!(reset.pending_depth, 0);
        assert_eq!(reset.p2_next_attempt_in_ms, 0);
        assert_eq!(reset.p2_wake_reason, None);

        assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_994,
                selected_at: "2026-08-10T12:00:01Z".to_string(),
            },
        )));
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                let snapshot = writer.accounting_snapshot();
                if snapshot.p2_wake_reason.as_deref() == Some("coalesced_deadline") {
                    assert!(snapshot.p2_next_attempt_in_ms >= 200);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("new P2 work should receive a full coalescing interval");
        writer.shutdown_and_drain().await;
    }

    #[tokio::test]
    async fn queued_p2_work_does_not_starve_its_scheduled_flush() {
        let pool = test_pool().await;
        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            CancellationToken::new(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );
        assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_993,
                selected_at: "2026-08-10T12:00:00Z".to_string(),
            },
        )));
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if writer.accounting_snapshot().p2_wake_reason.as_deref()
                    == Some("coalesced_deadline")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("writer should arm the P2 coalescing deadline");

        let producer_active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let producer = {
            let producer_active = producer_active.clone();
            let producer_writer = writer.clone();
            tokio::spawn(async move {
                while producer_active.load(std::sync::atomic::Ordering::Acquire) {
                    for _ in 0..1024 {
                        let _ = producer_writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
                            BatchedAccountSelectedTouch {
                                account_id: 999_992,
                                selected_at: "2026-08-10T12:00:01Z".to_string(),
                            },
                        ));
                    }
                    tokio::task::yield_now().await;
                }
            })
        };

        let resumed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if writer.accounting_snapshot().p2_flush_attempt_count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();
        producer_active.store(false, std::sync::atomic::Ordering::Release);
        producer.abort();
        let _ = producer.await;
        writer.shutdown_and_drain().await;
        assert!(
            resumed,
            "queued P2 writes must not prevent the scheduled P2 flush"
        );
    }

    #[tokio::test]
    async fn system_task_finish_batch_marks_running_task_terminal() {
        let pool = test_pool().await;
        let handle = begin_system_task_run(
            &pool,
            SystemTaskKind::StartupBackfill,
            "test",
            Some("started".to_string()),
        )
        .await
        .expect("begin system task");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::SystemTaskFinish(
                BatchedSystemTaskFinish {
                    run_id: handle.id,
                    task_kind: handle.task_kind,
                    trigger_kind: handle.trigger_kind.clone(),
                    status: SystemTaskStatus::Success,
                    summary: Some("completed".to_string()),
                    detail: None,
                    finished_at: "2026-07-01T10:00:05Z".to_string(),
                    duration_ms: 125,
                },
            )],
        )
        .await;

        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<i64>)>(
            r#"
            SELECT status, summary, finished_at, duration_ms
            FROM system_task_runs
            WHERE id = ?1
            "#,
        )
        .bind(handle.id)
        .fetch_one(&pool)
        .await
        .expect("load finished system task run");

        assert_eq!(row.0, SystemTaskStatus::Success.as_str());
        assert_eq!(row.1.as_deref(), Some("completed"));
        assert_eq!(row.2.as_deref(), Some("2026-07-01T10:00:05Z"));
        assert_eq!(row.3, Some(125));
    }

    #[tokio::test]
    async fn invocation_derived_batch_advances_live_progress_cursor_with_replay() {
        let pool = test_pool().await;
        save_hourly_rollup_live_progress_tx(
            pool.acquire().await.expect("acquire").as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            0,
        )
        .await
        .expect("seed live progress");

        let row_count = BACKFILL_BATCH_SIZE + 5;
        for index in 0..row_count {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id,
                    occurred_at,
                    source,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_tokens,
                    cost,
                    status,
                    raw_response,
                    detail_level
                )
                VALUES (?1, ?2, 'proxy', 1, 2, 0, 3, 0.01, 'success', '', 'full')
                "#,
            )
            .bind(format!("batch-derived-{index}"))
            .bind(format!("2026-07-01 10:{:02}:00", index % 60))
            .execute(&pool)
            .await
            .expect("seed invocation");
        }

        let max_id = sqlx::query_scalar::<_, i64>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("load max invocation id");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::InvocationDerived(
                BatchedInvocationDerivedWrites {
                    invocation_id: max_id,
                    occurred_at: "2026-07-01 10:00:00".to_string(),
                    upstream_account_id: None,
                    terminal_overlay_key: None,
                },
            )],
        )
        .await;

        let cursor = load_hourly_rollup_live_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await
            .expect("load live progress");
        assert_eq!(cursor, max_id);
    }

    #[tokio::test]
    async fn terminal_invocation_batch_persists_and_updates_rollups() {
        let pool = test_pool().await;
        save_hourly_rollup_live_progress_tx(
            pool.acquire().await.expect("acquire").as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            0,
        )
        .await
        .expect("seed live progress");

        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let mut record = build_running_proxy_capture_record(
            "batch-terminal-invocation",
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("192.0.2.44"),
            Some("sticky-a"),
            Some("pck-a"),
            true,
            Some(99),
            Some("Primary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(1),
            Some(1),
            None,
            None,
            3.0,
            4.0,
            5.0,
            6.0,
        );
        record.status = "success".to_string();
        record.usage.input_tokens = Some(2);
        record.usage.output_tokens = Some(3);
        record.usage.total_tokens = Some(5);
        record.cost = Some(0.02);

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::TerminalInvocation(
                BatchedTerminalInvocationWrite {
                    capture_started: None,
                    raw_capture: false,
                    dashboard_terminal_sequence: None,
                    terminal_projection_event_ids: Vec::new(),
                    startup_backfill_tasks: Vec::new(),
                    record,
                },
            )],
        )
        .await;

        let row = sqlx::query_as::<_, (String, i64, i64, Option<i64>)>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END
            FROM codex_invocations
            WHERE invoke_id = 'batch-terminal-invocation'
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load terminal invocation");

        assert_eq!(row.0, "success");
        assert_eq!(row.1, 2);
        assert_eq!(row.2, 3);
        assert_eq!(row.3, Some(99));

        let rollup = sqlx::query_as::<_, (i64, i64, i64)>(
            r#"
            SELECT
                COALESCE(SUM(total_count), 0),
                COALESCE(SUM(success_count), 0),
                COALESCE(SUM(total_tokens), 0)
            FROM invocation_rollup_hourly
            WHERE source = 'proxy'
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load invocation rollup");

        assert_eq!(rollup.0, 1);
        assert_eq!(rollup.1, 1);
        assert_eq!(rollup.2, 5);
    }

    #[tokio::test]
    async fn flush_now_treats_deferred_terminal_derived_writes_as_success() {
        let pool = test_pool().await;
        save_hourly_rollup_live_progress_tx(
            pool.acquire().await.expect("acquire").as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            0,
        )
        .await
        .expect("seed live progress");

        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let mut record = build_running_proxy_capture_record(
            "batch-terminal-flush-now-deferred-derived",
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("192.0.2.44"),
            Some("sticky-a"),
            Some("pck-a"),
            true,
            Some(99),
            Some("Primary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(1),
            Some(1),
            None,
            None,
            3.0,
            4.0,
            5.0,
            6.0,
        );
        record.status = "success".to_string();
        record.usage.input_tokens = Some(2);
        record.usage.output_tokens = Some(3);
        record.usage.total_tokens = Some(5);

        let runtime_store = Arc::new(ProxyRuntimeInvocationStore::default());
        let runtime_record = api_invocation_from_runtime_record(&record);
        runtime_store.upsert_terminal(runtime_record);
        assert_eq!(runtime_store.snapshot().len(), 1);

        let writer = SqliteBatchWriter::spawn_for_test();
        writer.set_terminal_runtime_store(runtime_store.clone());
        assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: Vec::new(),
                record,
            },
        )));

        writer
            .flush_now(&pool)
            .await
            .expect("P1 terminal flush should not fail because P2 derived work was deferred");
        let retained = writer.accounting_snapshot();
        assert_eq!(retained.state, "healthy");
        assert!(retained.pending_bytes > 0);
        assert!(retained.pending_depth > 0);
        assert!(retained.transfer_bytes > 0);

        let persisted_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'batch-terminal-flush-now-deferred-derived' AND status = 'success'",
        )
        .fetch_one(&pool)
        .await
        .expect("count terminal invocation after forced flush");
        assert_eq!(persisted_count, 1);

        let rollup_before_second_flush = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = 'proxy'",
        )
        .fetch_one(&pool)
        .await
        .expect("load rollup before deferred flush");
        assert_eq!(rollup_before_second_flush, 0);
        assert_eq!(
            runtime_store.snapshot().len(),
            1,
            "terminal overlay should remain until deferred derived writes flush"
        );

        writer
            .flush_now(&pool)
            .await
            .expect("deferred P2 derived write should flush later");

        let rollup_after_second_flush = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = 'proxy'",
        )
        .fetch_one(&pool)
        .await
        .expect("load rollup after deferred flush");
        assert_eq!(rollup_after_second_flush, 1);
        assert!(
            runtime_store.snapshot().is_empty(),
            "terminal overlay should be removed after derived writes flush"
        );
        let completed = writer.accounting_snapshot();
        assert_eq!(completed.pending_depth, 0);
        assert_eq!(completed.pending_bytes, 0);
        assert_eq!(completed.state, "healthy");

        writer.shutdown_and_drain().await;
    }

    #[tokio::test]
    async fn flush_now_schedules_retained_p2_for_coordinator_eligibility() {
        let pool = test_pool().await;
        let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
        let initial = coordinator.snapshot().await;
        assert!(initial.active_write_class.is_none());
        assert_eq!(initial.p1_waiter_count, 0);
        assert_eq!(initial.interactive_waiter_count, 0);
        assert_eq!(initial.p2_waiter_count, 0);

        let active_p2 = coordinator
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
            .await;
        let interactive_coordinator = coordinator.clone();
        let interactive_waiter = tokio::spawn(async move {
            interactive_coordinator
                .acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy,
                )
                .await
        });
        let waiter_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if coordinator.snapshot().await.interactive_waiter_count == 1 {
                break;
            }
            assert!(
                Instant::now() < waiter_deadline,
                "interactive coordinator waiter did not register"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            CancellationToken::new(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );
        assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                record: crate::tests::test_proxy_capture_record(
                    "flush-now-coordinator-p2",
                    "2026-08-10 12:00:00",
                ),
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: Vec::new(),
            },
        )));

        let flush_writer = writer.clone();
        let flush_pool = pool.clone();
        let flush_task = tokio::spawn(async move { flush_writer.flush_now(&flush_pool).await });
        let p1_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if coordinator.snapshot().await.p1_waiter_count == 1 {
                break;
            }
            assert!(
                Instant::now() < p1_deadline,
                "P1 coordinator waiter did not register"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        drop(active_p2);

        tokio::time::timeout(Duration::from_secs(2), flush_task)
            .await
            .expect("flush_now should finish after the P1 coordinator permit is released")
            .expect("flush_now task should complete")
            .expect("deferred P2 should not fail the flush_now barrier");
        let scheduled = writer.accounting_snapshot();
        assert_eq!(
            scheduled.p2_wake_reason.as_deref(),
            Some("background_eligible"),
            "retained P2 must wait on coordinator eligibility"
        );
        assert!(scheduled.pending_depth > 0);

        let interactive_permit = tokio::time::timeout(Duration::from_secs(2), interactive_waiter)
            .await
            .expect("interactive waiter should be admitted after P1 commits")
            .expect("interactive waiter task should complete");
        drop(interactive_permit);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = writer.accounting_snapshot();
                if snapshot.pending_depth == 0 && snapshot.p2_flush_attempt_count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("eligibility notification should resume the retained P2 flush");
        writer.shutdown_and_drain().await;
    }

    #[test]
    fn barrier_drain_includes_deferred_journal_terminals() {
        let root =
            std::env::temp_dir().join(format!("batch-writer-deferred-{}", nanoid::nanoid!()));
        std::fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut journal = TerminalJournal::open(&database_path).expect("open terminal journal");
        let record = crate::tests::test_proxy_capture_record(
            "batch-writer-deferred-terminal",
            "2026-07-29T00:00:00Z",
        );
        assert!(journal.defer_write(BatchedTerminalInvocationWrite {
            record,
            capture_started: None,
            raw_capture: true,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        }));
        let journal = Arc::new(std::sync::Mutex::new(Some(journal)));
        let mut pending = PendingBatch::default();

        let accounting = PendingQueueAccounting::default();
        let queued_p1_count = AtomicUsize::new(0);
        drain_terminal_journal_deferred_writes(
            &journal,
            &mut pending,
            &accounting,
            usize::MAX,
            &queued_p1_count,
        );

        assert_eq!(pending.terminal_invocations.len(), 1);
        let snapshot = accounting.snapshot();
        assert_eq!(snapshot.pending_depth, 1);
        assert_eq!(snapshot.pending_bytes, pending.estimated_memory_bytes());
        assert_eq!(snapshot.retry_count, 1);
        assert_eq!(snapshot.state, "healthy");
        assert!(
            journal
                .lock()
                .expect("lock terminal journal")
                .as_mut()
                .expect("terminal journal available")
                .take_deferred_writes(usize::MAX)
                .is_empty()
        );
        std::fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[tokio::test]
    async fn shutdown_drain_flushes_terminal_invocations() {
        let pool = test_pool().await;
        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let mut record = build_running_proxy_capture_record(
            "batch-terminal-shutdown-drain",
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("192.0.2.44"),
            Some("sticky-a"),
            Some("pck-a"),
            true,
            Some(42),
            Some("Primary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(1),
            Some(1),
            None,
            None,
            3.0,
            4.0,
            5.0,
            6.0,
        );
        record.status = "success".to_string();

        let writer = SqliteBatchWriter::spawn(
            pool.clone(),
            CancellationToken::new(),
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
            Arc::new(RwLock::new(PricingCatalog::default())),
            &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
        );
        assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: Vec::new(),
                record,
            },
        )));

        writer.shutdown_and_drain().await;

        let persisted_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'batch-terminal-shutdown-drain' AND status = 'success'",
        )
        .fetch_one(&pool)
        .await
        .expect("count drained terminal invocation");
        assert_eq!(persisted_count, 1);
    }

    #[tokio::test]
    async fn account_selected_touch_batch_coalesces_by_account_id() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
            )
            VALUES (77, 'api_key', 'codex', 'Primary', 'active', 1, NULL, '2026-07-01T09:59:00Z', '2026-07-01T09:59:00Z')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed account");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![
                SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                    account_id: 77,
                    selected_at: "2026-07-01T10:00:00Z".to_string(),
                }),
                SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                    account_id: 77,
                    selected_at: "2026-07-01T10:00:05Z".to_string(),
                }),
                SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                    account_id: 77,
                    selected_at: "2026-07-01T10:00:02Z".to_string(),
                }),
            ],
        )
        .await;

        let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT last_selected_at, updated_at FROM pool_upstream_accounts WHERE id = 77",
        )
        .fetch_one(&pool)
        .await
        .expect("load selected account");

        assert_eq!(row.0.as_deref(), Some("2026-07-01T10:00:05Z"));
        assert_eq!(row.1.as_deref(), Some("2026-07-01T10:00:05Z"));
    }
}
