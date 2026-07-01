use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use sqlx::{Pool, Sqlite};
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
const SQLITE_BATCH_CHANNEL_CAPACITY: usize = 10_000;

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
    SystemTaskFinish(BatchedSystemTaskFinish),
}

enum SqliteBatchWriterMessage {
    Write(SqliteBatchWrite),
    FlushNow(oneshot::Sender<Result<(), String>>),
    Shutdown(oneshot::Sender<Result<(), String>>),
}

#[derive(Debug, Default)]
struct PendingBatch {
    attempt_progress: HashMap<i64, BatchedAttemptProgress>,
    invocation_derived: BTreeMap<i64, BatchedInvocationDerivedWrites>,
    system_task_finishes: HashMap<i64, BatchedSystemTaskFinish>,
    enqueued_rows: usize,
    coalesced_rows: usize,
    oldest_at: Option<Instant>,
}

impl PendingBatch {
    fn is_empty(&self) -> bool {
        self.attempt_progress.is_empty()
            && self.invocation_derived.is_empty()
            && self.system_task_finishes.is_empty()
    }

    fn logical_rows(&self) -> usize {
        self.attempt_progress.len()
            + self.invocation_derived.len()
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
    sender: mpsc::Sender<SqliteBatchWriterMessage>,
    pending_depth: Arc<AtomicUsize>,
    dropped_writes: Arc<AtomicU64>,
    handle: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    buffered_writes: Option<Arc<std::sync::Mutex<Vec<SqliteBatchWrite>>>>,
}

impl SqliteBatchWriter {
    pub(crate) fn spawn(pool: Pool<Sqlite>, _shutdown: CancellationToken) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(SQLITE_BATCH_CHANNEL_CAPACITY);
        let pending_depth = Arc::new(AtomicUsize::new(0));
        let dropped_writes = Arc::new(AtomicU64::new(0));
        let handle = tokio::spawn(run_sqlite_batch_writer(
            pool,
            receiver,
            pending_depth.clone(),
        ));
        Arc::new(Self {
            sender,
            pending_depth,
            dropped_writes,
            handle: Mutex::new(Some(handle)),
            #[cfg(test)]
            buffered_writes: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test() -> Arc<Self> {
        let (sender, _receiver) = mpsc::channel(1);
        Arc::new(Self {
            sender,
            pending_depth: Arc::new(AtomicUsize::new(0)),
            dropped_writes: Arc::new(AtomicU64::new(0)),
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
        match self.sender.try_send(SqliteBatchWriterMessage::Write(write)) {
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
        if let Err(err) = self
            .sender
            .try_send(SqliteBatchWriterMessage::FlushNow(sender))
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
        if let Err(err) = self
            .sender
            .send(SqliteBatchWriterMessage::Shutdown(sender))
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
        flush_pending_batch_inner(pool, &batch).await
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
        flush_pending_batch_inner(pool, &batch)
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
            Self::flush_for_test(pool, writes).await;
        }
    }
}

async fn run_sqlite_batch_writer(
    pool: Pool<Sqlite>,
    mut receiver: mpsc::Receiver<SqliteBatchWriterMessage>,
    pending_depth: Arc<AtomicUsize>,
) {
    let mut ticker = interval(SQLITE_BATCH_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending = PendingBatch::default();

    loop {
        tokio::select! {
            biased;
            maybe_message = receiver.recv() => {
                let Some(message) = maybe_message else {
                    let _ = flush_pending_batch(&pool, pending.take(), true).await;
                    return;
                };
                match message {
                    SqliteBatchWriterMessage::Write(write) => {
                        pending_depth.fetch_sub(1, Ordering::Relaxed);
                        pending.push(write);
                        if pending.logical_rows() >= SQLITE_BATCH_MAX_ROWS {
                            if let Some(retained) = flush_pending_batch(&pool, pending.take(), false).await {
                                pending = retained;
                            }
                        }
                    }
                    SqliteBatchWriterMessage::FlushNow(sender) => {
                        let result = match flush_pending_batch(&pool, pending.take(), true).await {
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
                        let _ = sender.send(result);
                    }
                    SqliteBatchWriterMessage::Shutdown(sender) => {
                        let result = match flush_pending_batch(&pool, pending.take(), true).await {
                            Some(retained) => {
                                Err(format!(
                                    "sqlite batch writer retained {} logical rows after shutdown flush",
                                    retained.logical_rows()
                                ))
                            }
                            None => Ok(()),
                        };
                        let _ = sender.send(result);
                        return;
                    }
                }
            }
            _ = ticker.tick() => {
                if !pending.is_empty() {
                    let force_flush = pending.age() >= SQLITE_BATCH_MAX_AGE;
                    if force_flush {
                        warn!(
                            logical_rows = pending.logical_rows(),
                            enqueued_rows = pending.enqueued_rows,
                            coalesced_rows = pending.coalesced_rows,
                            oldest_age_ms = pending.age().as_millis() as u64,
                            "sqlite batch writer pending derived writes reached max age; forcing flush"
                        );
                    }
                    if let Some(retained) =
                        flush_pending_batch(&pool, pending.take(), force_flush).await
                    {
                        pending = retained;
                    }
                }
            }
        }
    }
}

async fn flush_pending_batch(
    pool: &Pool<Sqlite>,
    batch: PendingBatch,
    force: bool,
) -> Option<PendingBatch> {
    if batch.is_empty() {
        return None;
    }
    let started = Instant::now();
    let enqueued_rows = batch.enqueued_rows;
    let coalesced_rows = batch.coalesced_rows;
    let attempt_count = batch.attempt_progress.len();
    let invocation_count = batch.invocation_derived.len();
    let system_task_count = batch.system_task_finishes.len();
    let system_task_scope = summarize_system_task_batch_scope(&batch);
    let oldest_age_ms = batch.age().as_millis() as u64;

    let permit = if force {
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
                    system_task_count,
                    system_task_scope = %system_task_scope,
                    oldest_age_ms,
                    "sqlite batch writer deferred flush because pressure gate is closed"
                );
                return Some(batch);
            }
        }
    };

    if let Err(err) = flush_pending_batch_inner(pool, &batch).await {
        crate::db_pressure::global_db_pressure_gate().record_error("sqlite_batch_writer", &err);
        warn!(
            error = %err,
            enqueued_rows,
            coalesced_rows,
            attempt_count,
            invocation_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms = started.elapsed().as_millis() as u64,
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
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            "sqlite batch writer flush was slow"
        );
    } else {
        debug!(
            enqueued_rows,
            coalesced_rows,
            attempt_count,
            invocation_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
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

async fn flush_pending_batch_inner(pool: &Pool<Sqlite>, batch: &PendingBatch) -> Result<()> {
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

    if !batch.invocation_derived.is_empty() {
        let invocation_ids = batch.invocation_derived.keys().copied().collect::<Vec<_>>();
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &invocation_ids).await?;
        for derived in batch.invocation_derived.values() {
            touch_invocation_upstream_account_last_activity_tx(
                tx.as_mut(),
                &derived.occurred_at,
                derived.payload.as_deref(),
            )
            .await?;
        }
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
    Ok(())
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
        let writer = SqliteBatchWriter::spawn(pool.clone(), shutdown.clone());

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
    async fn invocation_derived_batch_does_not_advance_live_progress_cursor() {
        let pool = test_pool().await;
        save_hourly_rollup_live_progress_tx(
            pool.acquire().await.expect("acquire").as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            41,
        )
        .await
        .expect("seed live progress");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::InvocationDerived(
                BatchedInvocationDerivedWrites {
                    invocation_id: 100,
                    occurred_at: "2026-07-01 10:00:00".to_string(),
                    payload: None,
                },
            )],
        )
        .await;

        let cursor = load_hourly_rollup_live_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await
            .expect("load live progress");
        assert_eq!(cursor, 41);
    }
}
