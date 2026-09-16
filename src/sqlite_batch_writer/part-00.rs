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

fn p2_deadline_wait_armed(
    pending: &PendingBatch,
    p2_schedule: &P2ScheduleState,
    queued_p1_count: &AtomicUsize,
    p1_retry: &P1RetryState,
) -> bool {
    pending.has_p2()
        && p2_schedule.due_at.is_some()
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
