use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use sqlx::{Pool, Sqlite, SqliteConnection};
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::*;

const SQLITE_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const SQLITE_BATCH_MAX_ROWS: usize = 100;
const SQLITE_BATCH_MAX_AGE: Duration = Duration::from_secs(5);
const SQLITE_BATCH_STALE_WARN_AGE: Duration = Duration::from_secs(30);
const SQLITE_BATCH_CHANNEL_CAPACITY: usize = 10_000;

#[derive(Debug, Clone, Copy)]
enum FlushReason {
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
        matches!(self, Self::Barrier | Self::Shutdown)
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
    pub(crate) payload: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedRunningProxySnapshot {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) record: ProxyCaptureRecord,
}

impl BatchedRunningProxySnapshot {
    fn key(&self) -> String {
        format!("{}\n{}", self.invoke_id, self.occurred_at)
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
    AttemptProgress(BatchedAttemptProgress),
    InvocationDerived(BatchedInvocationDerivedWrites),
    RunningProxySnapshot(BatchedRunningProxySnapshot),
    AccountSelectedTouch(BatchedAccountSelectedTouch),
    SystemTaskFinish(BatchedSystemTaskFinish),
}

enum SqliteBatchWriterControl {
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
struct PendingBatch {
    attempt_progress: HashMap<i64, BatchedAttemptProgress>,
    invocation_derived: BTreeMap<i64, BatchedInvocationDerivedWrites>,
    running_proxy_snapshots: HashMap<String, BatchedRunningProxySnapshot>,
    account_selected_touches: HashMap<i64, BatchedAccountSelectedTouch>,
    system_task_finishes: HashMap<i64, BatchedSystemTaskFinish>,
    enqueued_rows: usize,
    coalesced_rows: usize,
    oldest_at: Option<Instant>,
}

impl PendingBatch {
    fn is_empty(&self) -> bool {
        self.attempt_progress.is_empty()
            && self.invocation_derived.is_empty()
            && self.running_proxy_snapshots.is_empty()
            && self.account_selected_touches.is_empty()
            && self.system_task_finishes.is_empty()
    }

    fn logical_rows(&self) -> usize {
        self.attempt_progress.len()
            + self.invocation_derived.len()
            + self.running_proxy_snapshots.len()
            + self.account_selected_touches.len()
            + self.system_task_finishes.len()
    }

    fn age(&self) -> Duration {
        self.oldest_at
            .map(|oldest| oldest.elapsed())
            .unwrap_or_default()
    }

    fn push(&mut self, write: SqliteBatchWrite) {
        let now = Instant::now();
        self.oldest_at.get_or_insert(now);
        self.enqueued_rows += 1;
        match write {
            SqliteBatchWrite::AttemptProgress(progress) => {
                if self
                    .attempt_progress
                    .insert(progress.attempt_id, progress)
                    .is_some()
                {
                    self.coalesced_rows += 1;
                }
            }
            SqliteBatchWrite::InvocationDerived(derived) => {
                if self
                    .invocation_derived
                    .insert(derived.invocation_id, derived)
                    .is_some()
                {
                    self.coalesced_rows += 1;
                }
            }
            SqliteBatchWrite::RunningProxySnapshot(snapshot) => {
                if self
                    .running_proxy_snapshots
                    .insert(snapshot.key(), snapshot)
                    .is_some()
                {
                    self.coalesced_rows += 1;
                }
            }
            SqliteBatchWrite::AccountSelectedTouch(touch) => {
                match self.account_selected_touches.get_mut(&touch.account_id) {
                    Some(existing) => {
                        if existing.selected_at < touch.selected_at {
                            *existing = touch;
                        }
                        self.coalesced_rows += 1;
                    }
                    None => {
                        self.account_selected_touches
                            .insert(touch.account_id, touch);
                    }
                }
            }
            SqliteBatchWrite::SystemTaskFinish(finish) => {
                if self
                    .system_task_finishes
                    .insert(finish.run_id, finish)
                    .is_some()
                {
                    self.coalesced_rows += 1;
                }
            }
        }
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

#[derive(Debug)]
pub(crate) struct SqliteBatchWriter {
    write_sender: mpsc::Sender<SqliteBatchWrite>,
    control_sender: mpsc::Sender<SqliteBatchWriterControl>,
    pending_depth: Arc<AtomicUsize>,
    dropped_writes: Arc<AtomicU64>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    buffered_writes: Option<Arc<std::sync::Mutex<Vec<SqliteBatchWrite>>>>,
}

impl SqliteBatchWriter {
    pub(crate) fn spawn(
        pool: Pool<Sqlite>,
        _shutdown: CancellationToken,
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    ) -> Arc<Self> {
        let (write_sender, write_receiver) = mpsc::channel(SQLITE_BATCH_CHANNEL_CAPACITY);
        let (control_sender, control_receiver) = mpsc::channel(128);
        let pending_depth = Arc::new(AtomicUsize::new(0));
        let dropped_writes = Arc::new(AtomicU64::new(0));
        let cache_for_task = prompt_cache_conversation_cache.clone();
        let handle = tokio::spawn(run_sqlite_batch_writer(
            pool,
            write_receiver,
            control_receiver,
            pending_depth.clone(),
            Some(cache_for_task),
        ));
        Arc::new(Self {
            write_sender,
            control_sender,
            pending_depth,
            dropped_writes,
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(Some(handle)),
            #[cfg(test)]
            buffered_writes: None,
        })
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
            control_sender,
            pending_depth: Arc::new(AtomicUsize::new(0)),
            dropped_writes: Arc::new(AtomicU64::new(0)),
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(None),
            buffered_writes: Some(Arc::new(std::sync::Mutex::new(Vec::new()))),
        })
    }

    pub(crate) fn enqueue(&self, write: SqliteBatchWrite) -> bool {
        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            match buffered_writes.lock() {
                Ok(mut guard) => {
                    guard.push(write);
                    self.pending_depth.fetch_add(1, Ordering::Relaxed);
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

        self.pending_depth.fetch_add(1, Ordering::Relaxed);
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(err) => {
                self.pending_depth.fetch_sub(1, Ordering::Relaxed);
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!(
                    error = %err,
                    queue_depth = self.pending_depth.load(Ordering::Relaxed),
                    dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                    "sqlite batch writer queue full; dropped derived write"
                );
                false
            }
        }
    }

    pub(crate) async fn flush_now(&self, _pool: &Pool<Sqlite>) -> Result<()> {
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            self.flush_buffered_for_test(_pool).await;
            return Ok(());
        }

        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self.pending_depth.load(Ordering::Relaxed);
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
                queue_depth = self.pending_depth.load(Ordering::Relaxed),
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
        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self.pending_depth.load(Ordering::Relaxed);
        if let Err(err) = self
            .control_sender
            .send(SqliteBatchWriterControl::Shutdown {
                queued_depth_snapshot,
                responder: sender,
            })
            .await
        {
            warn!(error = %err, "sqlite batch writer shutdown barrier could not be queued");
        } else if let Ok(Err(err)) = receiver.await {
            warn!(error = %err, "sqlite batch writer shutdown drain failed");
        }
        if let Err(err) = handle.await {
            warn!(error = %err, "sqlite batch writer task failed during shutdown");
        }
    }

    pub(crate) async fn flush_invocation_derived_inline(
        pool: &Pool<Sqlite>,
        derived: BatchedInvocationDerivedWrites,
    ) -> Result<()> {
        let mut batch = PendingBatch::default();
        batch.push(SqliteBatchWrite::InvocationDerived(derived));
        flush_pending_batch_inner(pool, &batch, None).await
    }

    pub(crate) async fn flush_running_proxy_snapshot_inline(
        &self,
        pool: &Pool<Sqlite>,
        snapshot: BatchedRunningProxySnapshot,
    ) -> Result<()> {
        let mut batch = PendingBatch::default();
        batch.push(SqliteBatchWrite::RunningProxySnapshot(snapshot));
        flush_pending_batch_inner(pool, &batch, self.prompt_cache_conversation_cache.as_ref()).await
    }

    pub(crate) async fn flush_account_selected_touch_inline(
        pool: &Pool<Sqlite>,
        touch: BatchedAccountSelectedTouch,
    ) -> Result<()> {
        let mut batch = PendingBatch::default();
        batch.push(SqliteBatchWrite::AccountSelectedTouch(touch));
        flush_pending_batch_inner(pool, &batch, None).await
    }

    #[cfg(test)]
    pub(crate) fn stats_snapshot(&self) -> (usize, u64) {
        (
            self.pending_depth.load(Ordering::Relaxed),
            self.dropped_writes.load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_test(pool: &Pool<Sqlite>, writes: Vec<SqliteBatchWrite>) {
        let mut batch = PendingBatch::default();
        for write in writes {
            batch.push(write);
        }
        flush_pending_batch_inner(pool, &batch, None)
            .await
            .expect("flush pending sqlite batch writes");
    }

    #[cfg(test)]
    pub(crate) async fn flush_buffered_for_test(&self, pool: &Pool<Sqlite>) {
        let writes = self
            .buffered_writes
            .as_ref()
            .and_then(|buffered_writes| {
                buffered_writes.lock().ok().map(|mut guard| {
                    let writes = guard.drain(..).collect::<Vec<_>>();
                    self.pending_depth
                        .fetch_sub(writes.len(), Ordering::Relaxed);
                    writes
                })
            })
            .unwrap_or_default();

        if !writes.is_empty() {
            let mut batch = PendingBatch::default();
            for write in writes {
                batch.push(write);
            }
            flush_pending_batch_inner(pool, &batch, self.prompt_cache_conversation_cache.as_ref())
                .await
                .expect("flush buffered sqlite batch writes for test");
        }
    }
}

async fn run_sqlite_batch_writer(
    pool: Pool<Sqlite>,
    mut write_receiver: mpsc::Receiver<SqliteBatchWrite>,
    mut control_receiver: mpsc::Receiver<SqliteBatchWriterControl>,
    pending_depth: Arc<AtomicUsize>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
) {
    let mut ticker = interval(SQLITE_BATCH_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending = PendingBatch::default();
    let mut control_closed = false;

    loop {
        tokio::select! {
            biased;
            maybe_control = control_receiver.recv(), if !control_closed => {
                if let Some(control) = maybe_control {
                    match control {
                    SqliteBatchWriterControl::FlushNow {
                        queued_depth_snapshot,
                        responder,
                    } => {
                        drain_queued_batch_writes(
                            &mut write_receiver,
                            &mut pending,
                            &pending_depth,
                            queued_depth_snapshot,
                        );
                        let result = match flush_pending_batch(
                            &pool,
                            pending.take(),
                            FlushReason::Barrier,
                            prompt_cache_conversation_cache.as_ref(),
                        )
                        .await
                        {
                            Some(retained) => {
                                let message = format!(
                                    "sqlite batch writer retained {} logical rows after forced flush",
                                    retained.logical_rows()
                                );
                                pending = retained;
                                Err(message)
                            }
                            None => Ok(()),
                        };
                        let _ = responder.send(result);
                    }
                    SqliteBatchWriterControl::Shutdown {
                        queued_depth_snapshot,
                        responder,
                    } => {
                        drain_queued_batch_writes(
                            &mut write_receiver,
                            &mut pending,
                            &pending_depth,
                            queued_depth_snapshot,
                        );
                        let result = match flush_pending_batch(
                            &pool,
                            pending.take(),
                            FlushReason::Shutdown,
                            prompt_cache_conversation_cache.as_ref(),
                        )
                        .await
                        {
                            Some(retained) => {
                                Err(format!(
                                    "sqlite batch writer retained {} logical rows after shutdown flush",
                                    retained.logical_rows()
                                ))
                            }
                            None => Ok(()),
                        };
                        let _ = responder.send(result);
                        return;
                    }
                    }
                } else {
                    control_closed = true;
                }
            }
            maybe_write = write_receiver.recv() => {
                let Some(write) = maybe_write else {
                    let _ =
                        flush_pending_batch(
                            &pool,
                            pending.take(),
                            FlushReason::Shutdown,
                            prompt_cache_conversation_cache.as_ref(),
                        )
                        .await;
                    return;
                };
                pending_depth.fetch_sub(1, Ordering::Relaxed);
                pending.push(write);
                if pending.logical_rows() >= SQLITE_BATCH_MAX_ROWS {
                    if let Some(retained) =
                        flush_pending_batch(
                            &pool,
                            pending.take(),
                            FlushReason::RowLimit,
                            prompt_cache_conversation_cache.as_ref(),
                        )
                        .await
                    {
                        pending = retained;
                    }
                }
            }
            _ = ticker.tick() => {
                if !pending.is_empty() {
                    let flush_reason = if pending.age() >= SQLITE_BATCH_MAX_AGE {
                        if pending.age() >= SQLITE_BATCH_STALE_WARN_AGE {
                            warn!(
                                logical_rows = pending.logical_rows(),
                                enqueued_rows = pending.enqueued_rows,
                                coalesced_rows = pending.coalesced_rows,
                                oldest_age_ms = pending.age().as_millis() as u64,
                                flush_reason = FlushReason::MaxAge.as_str(),
                                "sqlite batch writer pending derived writes are stale under database pressure"
                            );
                        } else {
                            debug!(
                                logical_rows = pending.logical_rows(),
                                enqueued_rows = pending.enqueued_rows,
                                coalesced_rows = pending.coalesced_rows,
                                oldest_age_ms = pending.age().as_millis() as u64,
                                flush_reason = FlushReason::MaxAge.as_str(),
                                "sqlite batch writer pending derived writes reached max age"
                            );
                        }
                        FlushReason::MaxAge
                    } else {
                        FlushReason::Interval
                    };
                    if let Some(retained) =
                        flush_pending_batch(
                            &pool,
                            pending.take(),
                            flush_reason,
                            prompt_cache_conversation_cache.as_ref(),
                        )
                        .await
                    {
                        pending = retained;
                    }
                }
            }
        }
    }
}

fn drain_queued_batch_writes(
    write_receiver: &mut mpsc::Receiver<SqliteBatchWrite>,
    pending: &mut PendingBatch,
    pending_depth: &Arc<AtomicUsize>,
    max_messages: usize,
) {
    for _ in 0..max_messages {
        match write_receiver.try_recv() {
            Ok(write) => {
                pending_depth.fetch_sub(1, Ordering::Relaxed);
                pending.push(write);
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
}

async fn flush_pending_batch(
    pool: &Pool<Sqlite>,
    batch: PendingBatch,
    reason: FlushReason,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Option<PendingBatch> {
    if batch.is_empty() {
        return None;
    }
    let started = Instant::now();
    let enqueued_rows = batch.enqueued_rows;
    let coalesced_rows = batch.coalesced_rows;
    let attempt_count = batch.attempt_progress.len();
    let invocation_count = batch.invocation_derived.len();
    let running_snapshot_count = batch.running_proxy_snapshots.len();
    let account_touch_count = batch.account_selected_touches.len();
    let system_task_count = batch.system_task_finishes.len();
    let system_task_scope = summarize_system_task_batch_scope(&batch);
    let oldest_age_ms = batch.age().as_millis() as u64;

    let flush_reason = reason.as_str();
    let permit = if reason.bypass_pressure_gate() {
        None
    } else {
        match crate::db_pressure::global_db_pressure_gate()
            .try_begin_background("sqlite_batch_writer")
        {
            Ok(permit) => Some(permit),
            Err(reason) => {
                debug!(
                    deny_reason = %reason,
                    enqueued_rows,
                    coalesced_rows,
                    attempt_count,
                    invocation_count,
                    running_snapshot_count,
                    account_touch_count,
                    system_task_count,
                    system_task_scope = %system_task_scope,
                    oldest_age_ms,
                    flush_reason,
                    "sqlite batch writer deferred flush because pressure gate is closed"
                );
                return Some(batch);
            }
        }
    };

    if let Err(err) = flush_pending_batch_inner(pool, &batch, prompt_cache_conversation_cache).await
    {
        crate::db_pressure::global_db_pressure_gate().record_error("sqlite_batch_writer", &err);
        warn!(
            error = %err,
            enqueued_rows,
            coalesced_rows,
            attempt_count,
            invocation_count,
            running_snapshot_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms = started.elapsed().as_millis() as u64,
            flush_reason,
            "sqlite batch writer flush failed"
        );
        drop(permit);
        return Some(batch);
    }
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        warn!(
            enqueued_rows,
            coalesced_rows,
            attempt_count,
            invocation_count,
            running_snapshot_count,
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
            attempt_count,
            invocation_count,
            running_snapshot_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            flush_reason,
            "sqlite batch writer flushed derived writes"
        );
    }
    None
}

fn summarize_system_task_batch_scope(batch: &PendingBatch) -> String {
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

async fn flush_pending_batch_inner(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let mut should_invalidate_prompt_cache_conversations = false;

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
            touch_invocation_upstream_account_last_activity_tx(
                tx.as_mut(),
                &derived.occurred_at,
                derived.payload.as_deref(),
            )
            .await?;
        }
    }

    for snapshot in batch.running_proxy_snapshots.values() {
        let rows_affected =
            insert_running_proxy_snapshot_placeholder_tx(tx.as_mut(), &snapshot.record).await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &snapshot.record.occurred_at,
            snapshot.record.payload.as_deref(),
        )
        .await?;
        if rows_affected > 0
            && prompt_cache_key_from_payload(snapshot.record.payload.as_deref())
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
        {
            should_invalidate_prompt_cache_conversations = true;
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
    if should_invalidate_prompt_cache_conversations
        && let Some(cache) = prompt_cache_conversation_cache
    {
        invalidate_prompt_cache_conversations_cache(cache).await;
    }
    Ok(())
}

async fn replay_live_invocation_hourly_rollups_until_tx(
    tx: &mut SqliteConnection,
    target_invocation_id: i64,
) -> Result<u64> {
    let mut total_updated = 0;
    loop {
        let cursor =
            load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
        if cursor >= target_invocation_id {
            return Ok(total_updated);
        }
        let updated = replay_live_invocation_hourly_rollups_tx(tx).await?;
        total_updated += updated;
        if updated == 0 {
            return Ok(total_updated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use sqlx::SqlitePool;

    fn attempt_trace(invoke_id: &str) -> PoolUpstreamAttemptTraceContext {
        PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: "2026-07-01 10:00:00".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some(format!("{invoke_id}-sticky")),
            requester_ip: Some("192.168.31.6".to_string()),
        }
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect sqlite memory pool");
        ensure_schema(&pool).await.expect("ensure schema");
        pool
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
                    payload: None,
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
    async fn invocation_derived_batch_corrects_terminal_placeholder_rollup_after_cursor_passed() {
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
        let running = build_running_proxy_capture_record(
            "batch-running-placeholder-rollup-correction",
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

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::RunningProxySnapshot(
                BatchedRunningProxySnapshot {
                    invoke_id: running.invoke_id.clone(),
                    occurred_at: running.occurred_at.clone(),
                    record: running,
                },
            )],
        )
        .await;

        let running_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
                .bind("batch-running-placeholder-rollup-correction")
                .fetch_one(&pool)
                .await
                .expect("load running placeholder id");

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
            VALUES ('batch-terminal-after-placeholder', '2026-07-01 10:01:00', 'proxy', 1, 2, 0, 3, 0.01, 'success', '', 'full')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert later terminal invocation");

        let later_terminal_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
                .bind("batch-terminal-after-placeholder")
                .fetch_one(&pool)
                .await
                .expect("load later terminal id");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::InvocationDerived(
                BatchedInvocationDerivedWrites {
                    invocation_id: later_terminal_id,
                    occurred_at: "2026-07-01 10:01:00".to_string(),
                    payload: None,
                },
            )],
        )
        .await;

        let cursor = load_hourly_rollup_live_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await
            .expect("load live progress after later terminal");
        assert_eq!(cursor, later_terminal_id);

        sqlx::query(
            r#"
            UPDATE codex_invocations
            SET status = 'success',
                input_tokens = 2,
                output_tokens = 3,
                cache_input_tokens = 0,
                total_tokens = 5,
                cost = 0.02,
                raw_response = '',
                detail_level = 'full'
            WHERE id = ?1
            "#,
        )
        .bind(running_id)
        .execute(&pool)
        .await
        .expect("terminalize earlier placeholder");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::InvocationDerived(
                BatchedInvocationDerivedWrites {
                    invocation_id: running_id,
                    occurred_at: "2026-07-01 10:00:00".to_string(),
                    payload: None,
                },
            )],
        )
        .await;

        let row = sqlx::query_as::<_, (i64, i64, i64)>(
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
        .expect("load corrected invocation rollup");

        assert_eq!(row.0, 2);
        assert_eq!(row.1, 2);
        assert_eq!(row.2, 8);
    }

    #[tokio::test]
    async fn running_proxy_snapshot_batch_inserts_placeholder_once() {
        let pool = test_pool().await;
        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let first = build_running_proxy_capture_record(
            "batch-running-placeholder",
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
        let second = build_running_proxy_capture_record(
            "batch-running-placeholder",
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("192.0.2.44"),
            Some("sticky-a"),
            Some("pck-a"),
            true,
            Some(43),
            Some("Secondary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(2),
            Some(2),
            None,
            None,
            3.0,
            4.0,
            8.0,
            9.0,
        );

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![
                SqliteBatchWrite::RunningProxySnapshot(BatchedRunningProxySnapshot {
                    invoke_id: first.invoke_id.clone(),
                    occurred_at: first.occurred_at.clone(),
                    record: first,
                }),
                SqliteBatchWrite::RunningProxySnapshot(BatchedRunningProxySnapshot {
                    invoke_id: second.invoke_id.clone(),
                    occurred_at: second.occurred_at.clone(),
                    record: second,
                }),
            ],
        )
        .await;

        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM codex_invocations
            WHERE invoke_id = 'batch-running-placeholder'
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("count running placeholder");
        let row = sqlx::query_as::<_, (String, Option<i64>)>(
            r#"
            SELECT status, CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END
            FROM codex_invocations
            WHERE invoke_id = 'batch-running-placeholder'
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load running placeholder");

        assert_eq!(count, 1);
        assert_eq!(row.0, "running");
        assert_eq!(row.1, Some(43));
    }

    #[tokio::test]
    async fn running_proxy_snapshot_batch_refreshes_existing_running_placeholder() {
        let pool = test_pool().await;
        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let first = build_running_proxy_capture_record(
            "batch-running-placeholder-refresh",
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
        let second = build_running_proxy_capture_record(
            "batch-running-placeholder-refresh",
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("192.0.2.44"),
            Some("sticky-a"),
            Some("pck-a"),
            true,
            Some(99),
            Some("Secondary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(3),
            Some(2),
            None,
            None,
            3.0,
            4.0,
            12.0,
            13.0,
        );

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::RunningProxySnapshot(
                BatchedRunningProxySnapshot {
                    invoke_id: first.invoke_id.clone(),
                    occurred_at: first.occurred_at.clone(),
                    record: first,
                },
            )],
        )
        .await;
        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::RunningProxySnapshot(
                BatchedRunningProxySnapshot {
                    invoke_id: second.invoke_id.clone(),
                    occurred_at: second.occurred_at.clone(),
                    record: second,
                },
            )],
        )
        .await;

        let row = sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<f64>)>(
            r#"
            SELECT
                status,
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END,
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.poolAttemptCount') END,
                t_upstream_ttfb_ms
            FROM codex_invocations
            WHERE invoke_id = 'batch-running-placeholder-refresh'
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load refreshed running placeholder");

        assert_eq!(row.0, "running");
        assert_eq!(row.1, Some(99));
        assert_eq!(row.2, Some(3));
        assert_eq!(row.3, Some(13.0));
    }

    #[tokio::test]
    async fn running_proxy_snapshot_inline_fallback_inserts_placeholder() {
        let pool = test_pool().await;
        let request_info = RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        let record = build_running_proxy_capture_record(
            "batch-running-inline-fallback",
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

        let writer = SqliteBatchWriter::spawn_for_test();
        writer
            .flush_running_proxy_snapshot_inline(
                &pool,
                BatchedRunningProxySnapshot {
                    invoke_id: record.invoke_id.clone(),
                    occurred_at: record.occurred_at.clone(),
                    record,
                },
            )
            .await
            .expect("flush running snapshot fallback");

        let row = sqlx::query_as::<_, (String, Option<i64>)>(
            r#"
            SELECT status, CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END
            FROM codex_invocations
            WHERE invoke_id = 'batch-running-inline-fallback'
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load inline fallback running placeholder");

        assert_eq!(row.0, "running");
        assert_eq!(row.1, Some(42));
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

    #[tokio::test]
    async fn account_selected_touch_inline_fallback_updates_existing_timestamp() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
            )
            VALUES (78, 'api_key', 'codex', 'Secondary', 'active', 1, '2026-07-01T09:00:00Z', '2026-07-01T08:59:00Z', '2026-07-01T09:00:00Z')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed account");

        SqliteBatchWriter::flush_account_selected_touch_inline(
            &pool,
            BatchedAccountSelectedTouch {
                account_id: 78,
                selected_at: "2026-07-01T10:00:00Z".to_string(),
            },
        )
        .await
        .expect("flush account selected fallback");

        let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT last_selected_at, updated_at FROM pool_upstream_accounts WHERE id = 78",
        )
        .fetch_one(&pool)
        .await
        .expect("load selected account");

        assert_eq!(row.0.as_deref(), Some("2026-07-01T10:00:00Z"));
        assert_eq!(row.1.as_deref(), Some("2026-07-01T10:00:00Z"));
    }
}
