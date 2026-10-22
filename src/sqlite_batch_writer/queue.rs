use super::*;

pub(crate) const SQLITE_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(20);
pub(crate) const SQLITE_P2_COALESCE_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const SQLITE_BATCH_MAX_ROWS: usize = 32;
pub(crate) const SQLITE_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const SQLITE_BATCH_MAX_AGE: Duration = Duration::from_secs(5);
pub(crate) const SQLITE_BATCH_STALE_WARN_AGE: Duration = Duration::from_secs(30);
pub(crate) const SQLITE_BATCH_CHANNEL_CAPACITY: usize = 10_000;
pub(super) const SQLITE_SHUTDOWN_DRAIN_DEADLINE: Duration = Duration::from_secs(5);
pub(super) const SQLITE_RELIABLE_ATTEMPT_PROGRESS_OVERFLOW_MAX_ROWS: usize = SQLITE_BATCH_MAX_ROWS;
const SQLITE_P1_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];

#[derive(Debug, Default)]
pub(super) struct P1RetryState {
    pub(super) generation: usize,
    pub(super) due_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum P2WakeReason {
    CoalescedDeadline,
    PressureCooldownElapsed,
    BackgroundEligible,
    LockRetry,
}

impl P2WakeReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::CoalescedDeadline => "coalesced_deadline",
            Self::PressureCooldownElapsed => "pressure_cooldown_elapsed",
            Self::BackgroundEligible => "background_eligible",
            Self::LockRetry => "lock_retry",
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct P2ScheduleState {
    pub(super) generation: usize,
    pub(super) due_at: Option<Instant>,
    pub(super) wake_reason: Option<P2WakeReason>,
    pub(super) deferred_since: Option<Instant>,
}

impl P2ScheduleState {
    pub(super) fn arm_if_idle(&mut self, now: Instant) {
        if self.due_at.is_none() && self.wake_reason.is_none() {
            self.due_at = Some(now + SQLITE_P2_COALESCE_INTERVAL);
            self.wake_reason = Some(P2WakeReason::CoalescedDeadline);
            self.deferred_since.get_or_insert(now);
        }
    }

    pub(super) fn ready(&self, now: Instant) -> bool {
        self.due_at.is_some_and(|due_at| now >= due_at)
    }

    pub(super) fn defer_pressure(&mut self, delay: Duration, reason: P2WakeReason) {
        self.due_at = Some(Instant::now() + delay.max(Duration::from_millis(1)));
        self.wake_reason = Some(reason);
        self.deferred_since.get_or_insert_with(Instant::now);
    }

    pub(super) fn defer_until_background_eligible(&mut self) {
        self.due_at = None;
        self.wake_reason = Some(P2WakeReason::BackgroundEligible);
        self.deferred_since.get_or_insert_with(Instant::now);
    }

    pub(super) fn wake_background_eligible(&mut self) {
        self.due_at = Some(Instant::now());
        self.wake_reason = Some(P2WakeReason::BackgroundEligible);
    }

    pub(super) fn failed(&mut self, transaction_seed: u64) -> Duration {
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

    pub(super) fn succeeded(&mut self) {
        *self = Self::default();
    }

    pub(super) fn next_attempt_in_ms(&self) -> u64 {
        self.due_at
            .map(|due_at| due_at.saturating_duration_since(Instant::now()).as_millis() as u64)
            .unwrap_or_default()
    }

    pub(super) fn deferred_age_ms(&self) -> u64 {
        self.deferred_since
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or_default()
    }
}

pub(super) async fn wait_for_p2_deadline(due_at: Option<Instant>) {
    match due_at {
        Some(due_at) => sleep(due_at.saturating_duration_since(Instant::now())).await,
        None => std::future::pending::<()>().await,
    }
}

pub(super) fn p2_deadline_wait_armed(
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

pub(super) fn is_p1_terminal_write(write: &SqliteBatchWrite) -> bool {
    matches!(write, SqliteBatchWrite::TerminalInvocation(_))
}

pub(super) fn is_attempt_progress_write(write: &SqliteBatchWrite) -> bool {
    matches!(write, SqliteBatchWrite::AttemptProgress(_))
}

pub(super) fn decrement_queued_p1_count(queued_p1_count: &AtomicUsize) {
    let _ = queued_p1_count.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
        count.checked_sub(1)
    });
}

pub(super) fn drain_queued_writes_before_dispatch(
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

pub(super) fn drain_queued_writes_before_p2_dispatch(
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
    pub(super) fn ready(&self, now: Instant) -> bool {
        self.due_at.is_none_or(|due_at| now >= due_at)
    }

    pub(super) fn failed(&mut self, transaction_seed: u64) -> Duration {
        let base = SQLITE_P1_RETRY_DELAYS[self.generation.min(SQLITE_P1_RETRY_DELAYS.len() - 1)];
        self.generation = self.generation.saturating_add(1);
        let jitter_ceiling_ms = (base.as_millis() as u64 / 10).max(1);
        let jitter_ms = transaction_seed % jitter_ceiling_ms;
        let delay = base.saturating_add(Duration::from_millis(jitter_ms));
        self.due_at = Some(Instant::now() + delay);
        delay
    }

    pub(super) fn succeeded(&mut self) {
        self.generation = 0;
        self.due_at = None;
    }
}

pub(super) fn is_batch_writer_sqlite_lock_error(error: &anyhow::Error) -> bool {
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
    pub(crate) p1_ack_sequence: u64,
    pub(crate) p1_ack_duration_ms: u64,
    pub(crate) transfer_bytes: usize,
    pub(crate) retry_count: u64,
    pub(crate) p1_retry_count: u64,
    pub(crate) p2_retry_count: u64,
    pub(crate) p2_flush_attempt_count: u64,
    pub(crate) p2_pressure_defer_count: u64,
    pub(crate) p2_lock_retry_count: u64,
    pub(crate) p2_next_attempt_in_ms: u64,
    pub(crate) p2_deferred_age_ms: u64,
    pub(crate) write_batch_count: u64,
    pub(crate) write_rows: u64,
    pub(crate) write_bytes: u64,
    pub(crate) write_duration_ms: u64,
    pub(crate) attempt_progress_enqueued: u64,
    pub(crate) attempt_progress_coalesced: u64,
    pub(crate) attempt_progress_dropped: u64,
    pub(crate) attempt_progress_deferred: u64,
    pub(crate) attempt_progress_oldest_age_ms: u64,
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
    pub(crate) observability: crate::observability::MetricsSink,
    pub(crate) pending_depth: AtomicUsize,
    pub(crate) observed_p1_depth: AtomicUsize,
    pub(crate) pending_bytes: AtomicUsize,
    pub(crate) p1_ack_sequence: AtomicU64,
    pub(crate) p1_ack_duration_ms: AtomicU64,
    pub(crate) transfer_bytes: AtomicUsize,
    pub(crate) retry_count: AtomicU64,
    pub(crate) p1_retry_count: AtomicU64,
    pub(crate) p2_retry_count: AtomicU64,
    pub(crate) p2_flush_attempt_count: AtomicU64,
    pub(crate) p2_pressure_defer_count: AtomicU64,
    pub(crate) p2_lock_retry_count: AtomicU64,
    pub(crate) p2_next_attempt_in_ms: AtomicU64,
    pub(crate) p2_deferred_age_ms: AtomicU64,
    pub(crate) write_batch_count: AtomicU64,
    pub(crate) write_rows: AtomicU64,
    pub(crate) write_bytes: AtomicU64,
    pub(crate) write_duration_ms: AtomicU64,
    pub(crate) attempt_progress_enqueued: AtomicU64,
    pub(crate) attempt_progress_coalesced: AtomicU64,
    pub(crate) attempt_progress_dropped: AtomicU64,
    pub(crate) attempt_progress_deferred: AtomicU64,
    pub(crate) attempt_progress_oldest_age_ms: AtomicU64,
    pub(crate) p2_wake_reason: std::sync::Mutex<Option<String>>,
    pub(crate) invariant_violation_count: AtomicU64,
    pub(crate) last_invariant_violation: std::sync::Mutex<Option<PendingQueueInvariantViolation>>,
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

pub(super) fn take_next_bounded_batch(pending: &mut PendingBatch) -> PendingBatch {
    if !pending.terminal_invocations.is_empty() {
        pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    } else {
        pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    }
}
