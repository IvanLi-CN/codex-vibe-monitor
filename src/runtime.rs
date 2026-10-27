use super::*;
use futures_util::FutureExt;
use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;

const STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL: Duration = Duration::from_secs(1);
const STARTUP_HOT_READ_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(30);
const STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX: Duration = Duration::from_secs(300);
const MANAGED_TASK_FINISH_RETRY_ATTEMPTS: usize = 4;
const MANAGED_TASK_FINISH_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(250);
const MANAGED_TASK_MAX_ACTIVE_WORKERS: usize = 4;
const MANAGED_TASK_MAX_ACTIVE_FINALIZERS: usize = 2;
const MANAGED_TASK_MAX_PENDING_FINISHES: usize = 16;
const STARTUP_RECOVERY_BATCH_ROWS: usize = 64;
pub(crate) const STARTUP_RECOVERY_BUSY_TIMEOUT: Duration = Duration::from_millis(200);
pub(crate) const STARTUP_RECOVERY_ADMISSION_BUDGET: Duration = Duration::from_millis(250);
const STARTUP_RECOVERY_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
static STARTUP_RECOVERY_ACTIVE: AtomicBool = AtomicBool::new(false);
static POOL_ATTEMPT_RECOVERY_GATE: std::sync::OnceLock<tokio::sync::RwLock<()>> =
    std::sync::OnceLock::new();

pub(crate) fn pool_attempt_recovery_gate() -> &'static tokio::sync::RwLock<()> {
    POOL_ATTEMPT_RECOVERY_GATE.get_or_init(|| tokio::sync::RwLock::new(()))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StartupRecoveryFence {
    pub(crate) invocation_max_id: i64,
    pub(crate) pool_attempt_max_id: i64,
    pub(crate) captured: bool,
}

impl StartupRecoveryFence {
    fn empty() -> Self {
        Self::default()
    }
}

async fn capture_startup_recovery_fence(
    options: &SqliteConnectOptions,
) -> Result<StartupRecoveryFence> {
    let mut connection = SqliteConnection::connect_with(options).await?;
    let mut transaction = connection.begin().await?;
    let invocation_max_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(transaction.as_mut())
            .await?;
    let pool_attempt_max_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM pool_upstream_request_attempts",
    )
    .fetch_one(transaction.as_mut())
    .await?;
    transaction.commit().await?;
    Ok(StartupRecoveryFence {
        invocation_max_id,
        pool_attempt_max_id,
        captured: true,
    })
}

async fn capture_startup_recovery_fence_bounded(
    options: &SqliteConnectOptions,
) -> StartupRecoveryFence {
    match timeout(
        STARTUP_RECOVERY_ADMISSION_BUDGET,
        capture_startup_recovery_fence(options),
    )
    .await
    {
        Ok(Ok(fence)) => fence,
        Ok(Err(error)) => {
            warn!(error = %error, "startup recovery fence could not be captured before HTTP readiness");
            StartupRecoveryFence::empty()
        }
        Err(_) => {
            warn!(
                budget_ms = STARTUP_RECOVERY_ADMISSION_BUDGET.as_millis() as u64,
                "startup recovery fence capture exceeded its bounded readiness budget"
            );
            StartupRecoveryFence::empty()
        }
    }
}

fn observe_startup_recovery_state(
    state: &AppState,
    component: &'static str,
    pending: usize,
    oldest_age: Duration,
    state_name: &'static str,
) {
    let available = state_name != "unavailable";
    state.observability.gauge(
        "cvm_startup_recovery_data_available",
        &[("component", component)],
        f64::from(available),
    );
    if available {
        state.observability.gauge(
            "cvm_startup_recovery_pending_records",
            &[("component", component)],
            pending as f64,
        );
        state.observability.gauge(
            "cvm_startup_recovery_oldest_age_seconds",
            &[("component", component)],
            oldest_age.as_secs_f64(),
        );
    }
    for candidate in ["running", "complete", "unavailable"] {
        state.observability.gauge(
            "cvm_startup_recovery_last_state",
            &[("component", component), ("state", candidate)],
            if candidate == state_name { 1.0 } else { 0.0 },
        );
    }
}

fn observe_startup_recovery_retry(state: &AppState, component: &'static str, reason: &'static str) {
    state.observability.counter(
        "cvm_startup_recovery_retries_total",
        &[("component", component), ("reason", reason)],
        1,
    );
}

async fn wait_for_startup_recovery_retry(shutdown: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = sleep(delay) => true,
    }
}

enum StartupRecoveryBatch {
    Invocations(Vec<crate::proxy::RecoveredInvocationRow>),
    PoolAttempts(Vec<crate::proxy::RecoveredPoolAttemptRow>),
}

struct StartupRecoveryActiveGuard;

impl StartupRecoveryActiveGuard {
    fn acquire() -> Option<Self> {
        STARTUP_RECOVERY_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(Self)
    }
}

impl Drop for StartupRecoveryActiveGuard {
    fn drop(&mut self) {
        STARTUP_RECOVERY_ACTIVE.store(false, Ordering::Release);
    }
}

async fn wait_for_startup_recovery_guard(
    shutdown: &CancellationToken,
) -> Option<StartupRecoveryActiveGuard> {
    loop {
        if let Some(guard) = StartupRecoveryActiveGuard::acquire() {
            return Some(guard);
        }
        tokio::select! {
            _ = shutdown.cancelled() => return None,
            _ = sleep(Duration::from_millis(25)) => {}
        }
    }
}

async fn load_startup_invocation_ids(
    connection: &mut SqliteConnection,
    cursor: i64,
    fence: i64,
) -> Result<Vec<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM codex_invocations
        WHERE id > ?1
          AND id <= ?2
          AND source = ?3
          AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        ORDER BY id
        LIMIT ?4
        "#,
    )
    .bind(cursor)
    .bind(fence)
    .bind(SOURCE_PROXY)
    .bind(STARTUP_RECOVERY_BATCH_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .context("load startup invocation recovery batch")
}

async fn load_startup_pool_attempt_ids(
    connection: &mut SqliteConnection,
    cursor: i64,
    fence: i64,
) -> Result<Vec<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_request_attempts
        WHERE id > ?1
          AND id <= ?2
          AND status = ?3
          AND finished_at IS NULL
        ORDER BY id
        LIMIT ?4
        "#,
    )
    .bind(cursor)
    .bind(fence)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .bind(STARTUP_RECOVERY_BATCH_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .context("load startup pool attempt recovery batch")
}

async fn load_startup_invocation_pending_stats(
    connection: &mut SqliteConnection,
    cursor: i64,
    fence: i64,
) -> Result<(usize, Duration)> {
    let (pending, oldest_age_seconds): (i64, f64) = sqlx::query_as(
        r#"
        SELECT COUNT(*) AS pending,
               COALESCE((julianday('now') - julianday(MIN(occurred_at))) * 86400.0, 0.0)
        FROM codex_invocations
        WHERE id > ?1
          AND id <= ?2
          AND source = ?3
          AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        "#,
    )
    .bind(cursor)
    .bind(fence)
    .bind(SOURCE_PROXY)
    .fetch_one(&mut *connection)
    .await
    .context("load startup invocation recovery progress")?;
    Ok((
        pending.max(0) as usize,
        Duration::from_secs_f64(oldest_age_seconds.max(0.0)),
    ))
}

async fn load_startup_pool_attempt_pending_stats(
    connection: &mut SqliteConnection,
    cursor: i64,
    fence: i64,
) -> Result<(usize, Duration)> {
    let (pending, oldest_age_seconds): (i64, f64) = sqlx::query_as(
        r#"
        SELECT COUNT(*) AS pending,
               COALESCE((julianday('now') - julianday(MIN(occurred_at))) * 86400.0, 0.0)
        FROM pool_upstream_request_attempts
        WHERE id > ?1
          AND id <= ?2
          AND status = ?3
          AND finished_at IS NULL
        "#,
    )
    .bind(cursor)
    .bind(fence)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .fetch_one(&mut *connection)
    .await
    .context("load startup pool attempt recovery progress")?;
    Ok((
        pending.max(0) as usize,
        Duration::from_secs_f64(oldest_age_seconds.max(0.0)),
    ))
}

async fn run_startup_recovery_batch(
    state: &AppState,
    connection: &mut SqliteConnection,
    component: &'static str,
    ids: &[i64],
) -> Result<StartupRecoveryBatch> {
    if ids.is_empty() {
        return Ok(match component {
            "invocations" => StartupRecoveryBatch::Invocations(Vec::new()),
            "pool_attempts" => StartupRecoveryBatch::PoolAttempts(Vec::new()),
            _ => unreachable!("startup recovery component is fixed"),
        });
    }
    // Pool-attempt recovery shares ownership with the periodic orphan sweep. Acquire the
    // dedicated gate before taking any database permits so a blocked recovery attempt does not
    // hold a background slot or SQLite write admission while it waits.
    let _pool_attempt_recovery_gate = if component == "pool_attempts" {
        Some(
            timeout(
                STARTUP_RECOVERY_ADMISSION_BUDGET,
                pool_attempt_recovery_gate().write(),
            )
            .await
            .map_err(|_| anyhow!("startup pool attempt recovery gate exceeded 250ms"))?,
        )
    } else {
        None
    };
    let background_permit = crate::db_pressure::global_db_pressure_gate()
        .try_begin_background("startup_recovery")
        .map_err(|reason| anyhow!("startup recovery background slot unavailable: {reason}"))?;
    let write_permit = timeout(
        STARTUP_RECOVERY_ADMISSION_BUDGET,
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator().acquire(
            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
        ),
    )
    .await
    .map_err(|_| anyhow!("startup recovery write admission exceeded 250ms"))?;
    let started = Instant::now();
    let recovered = match component {
        "invocations" => crate::proxy::recover_proxy_invocations_with_connection(
            connection,
            crate::proxy::ProxyInvocationRecoveryScope::Ids(ids),
        )
        .await
        .map(StartupRecoveryBatch::Invocations),
        "pool_attempts" => crate::proxy::recover_pool_upstream_request_attempts_with_connection(
            connection,
            crate::proxy::PoolAttemptRecoveryScope::Ids(ids),
        )
        .await
        .map(StartupRecoveryBatch::PoolAttempts),
        _ => unreachable!("startup recovery component is fixed"),
    };
    drop(write_permit);
    drop(background_permit);
    let outcome = if recovered.is_ok() {
        "success"
    } else {
        "error"
    };
    state.observability.duration(
        "cvm_startup_recovery_batch_duration_seconds",
        &[("component", component), ("outcome", outcome)],
        started.elapsed(),
    );
    state.observability.counter(
        "cvm_startup_recovery_batches_total",
        &[("component", component), ("outcome", outcome)],
        1,
    );
    recovered
}

async fn publish_startup_recovery_batch(state: &AppState, batch: &StartupRecoveryBatch) -> usize {
    match batch {
        StartupRecoveryBatch::Invocations(rows) => {
            if let Err(error) = broadcast_recovered_proxy_invocations(state, rows).await {
                warn!(error = %error, "failed to broadcast startup-recovered invocations");
            }
            rows.len()
        }
        StartupRecoveryBatch::PoolAttempts(rows) => {
            clean_up_recovered_pool_routes(state, rows, &[], "startup_recovery").await;
            for invoke_id in rows
                .iter()
                .map(|row| row.invoke_id.as_str())
                .collect::<BTreeSet<_>>()
            {
                if let Err(error) =
                    broadcast_pool_upstream_attempts_snapshot(state, invoke_id).await
                {
                    warn!(
                        invoke_id,
                        error = %error,
                        "failed to broadcast startup-recovered pool attempt snapshot"
                    );
                }
            }
            rows.len()
        }
    }
}

async fn run_startup_recovery(
    state: Arc<AppState>,
    fence: StartupRecoveryFence,
    readiness_at: Instant,
    recovery_connect_options: Arc<SqliteConnectOptions>,
) {
    let Some(active_guard) = wait_for_startup_recovery_guard(&state.shutdown).await else {
        warn!("startup recovery coordinator stopped while waiting for the recovery path");
        return;
    };
    let mut connection = None;
    let mut invocation_cursor = 0_i64;
    let mut pool_attempt_cursor = 0_i64;
    let mut retry_index = 0_usize;
    let mut invocation_done = false;
    let mut pool_attempt_done = false;

    state.observability.gauge(
        "cvm_startup_recovery_readiness_to_recovery_seconds",
        &[],
        readiness_at.elapsed().as_secs_f64(),
    );
    let _ = state.sqlite_batch_writer.start_terminal_journal_recovery();
    if !fence.captured {
        warn!(
            "startup recovery fence was unavailable before HTTP readiness; deferring database row recovery to the next start"
        );
        observe_startup_recovery_state(&state, "invocations", 0, Duration::ZERO, "unavailable");
        observe_startup_recovery_state(&state, "pool_attempts", 0, Duration::ZERO, "unavailable");
        // Only terminal-journal dispatch remains in this path. Release the database recovery
        // owner so the independent runtime orphan sweep can continue while fenced row recovery
        // is deferred to the next start.
        drop(active_guard);
    }

    loop {
        if state.shutdown.is_cancelled() {
            warn!(
                "startup recovery stopped during shutdown; durable obligations remain for the next start"
            );
            return;
        }
        let journal = state.sqlite_batch_writer.terminal_journal_stats();
        if journal.unavailable {
            observe_startup_recovery_state(
                &state,
                "terminal_journal",
                journal.pending_materialization_records,
                Duration::from_millis(journal.oldest_pending_age_ms),
                "unavailable",
            );
            warn!(
                "startup recovery stopped because terminal journal recovery is unavailable; durable obligations remain for the next start"
            );
            return;
        }
        if !fence.captured {
            observe_startup_recovery_state(
                &state,
                "terminal_journal",
                journal.pending_materialization_records,
                Duration::from_millis(journal.oldest_pending_age_ms),
                if journal.unavailable {
                    "unavailable"
                } else if journal.pending_materialization_records == 0 {
                    "complete"
                } else {
                    "running"
                },
            );
            if journal.pending_materialization_records == 0 {
                info!(
                    "startup recovery deferred fenced database rows because the pre-readiness fence was unavailable"
                );
                return;
            }
            if retry_index > 0
                && !wait_for_startup_recovery_retry(
                    &state.shutdown,
                    STARTUP_RECOVERY_RETRY_DELAYS[retry_index],
                )
                .await
            {
                warn!(
                    "startup recovery retry wait cancelled; durable obligations remain for the next start"
                );
                return;
            }
            retry_index = (retry_index + 1).min(STARTUP_RECOVERY_RETRY_DELAYS.len() - 1);
            continue;
        }
        if connection.is_none() {
            match SqliteConnection::connect_with(recovery_connect_options.as_ref()).await {
                Ok(new_connection) => connection = Some(new_connection),
                Err(error) => {
                    observe_startup_recovery_retry(&state, "connection", "connect_failed");
                    warn!(error = %error, "startup recovery connection unavailable");
                    if !wait_for_startup_recovery_retry(
                        &state.shutdown,
                        STARTUP_RECOVERY_RETRY_DELAYS[retry_index],
                    )
                    .await
                    {
                        warn!(
                            "startup recovery connection retry cancelled; durable obligations remain for the next start"
                        );
                        return;
                    }
                    retry_index = (retry_index + 1).min(STARTUP_RECOVERY_RETRY_DELAYS.len() - 1);
                    continue;
                }
            }
        }

        let connection_ref = connection
            .as_mut()
            .expect("startup recovery connection exists");
        let mut progressed = false;
        let mut retry_required = false;
        let mut connection_failed = false;
        if !invocation_done {
            match load_startup_invocation_ids(
                connection_ref,
                invocation_cursor,
                fence.invocation_max_id,
            )
            .await
            {
                Ok(ids) if ids.is_empty() => {
                    invocation_done = true;
                    observe_startup_recovery_state(
                        &state,
                        "invocations",
                        0,
                        Duration::ZERO,
                        "complete",
                    );
                }
                Ok(ids) => {
                    match run_startup_recovery_batch(&state, connection_ref, "invocations", &ids)
                        .await
                    {
                        Ok(batch) => {
                            let recovered = publish_startup_recovery_batch(&state, &batch).await;
                            invocation_cursor = ids.last().copied().unwrap_or(invocation_cursor);
                            progressed = true;
                            retry_index = 0;
                            match load_startup_invocation_pending_stats(
                                connection_ref,
                                invocation_cursor,
                                fence.invocation_max_id,
                            )
                            .await
                            {
                                Ok((pending, oldest_age)) => {
                                    observe_startup_recovery_state(
                                        &state,
                                        "invocations",
                                        pending.saturating_add(ids.len().saturating_sub(recovered)),
                                        oldest_age,
                                        "running",
                                    );
                                }
                                Err(error) => {
                                    retry_required = true;
                                    connection_failed = true;
                                    observe_startup_recovery_retry(
                                        &state,
                                        "invocations",
                                        "pending_stats",
                                    );
                                    crate::db_pressure::global_db_pressure_gate()
                                        .record_error("startup_recovery_invocations", &error);
                                    observe_startup_recovery_state(
                                        &state,
                                        "invocations",
                                        0,
                                        Duration::ZERO,
                                        "unavailable",
                                    );
                                    warn!(
                                        error = %error,
                                        "startup invocation recovery progress query deferred"
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            retry_required = true;
                            observe_startup_recovery_retry(&state, "invocations", "write_or_lock");
                            crate::db_pressure::global_db_pressure_gate()
                                .record_error("startup_recovery_invocations", &error);
                            connection_failed = true;
                            warn!(error = %error, "startup invocation recovery batch deferred");
                        }
                    }
                }
                Err(error) => {
                    retry_required = true;
                    observe_startup_recovery_retry(&state, "invocations", "read_or_lock");
                    connection_failed = true;
                    warn!(error = %error, "startup invocation recovery batch read deferred");
                }
            }
        }
        if !pool_attempt_done && (invocation_done || progressed) {
            match load_startup_pool_attempt_ids(
                connection_ref,
                pool_attempt_cursor,
                fence.pool_attempt_max_id,
            )
            .await
            {
                Ok(ids) if ids.is_empty() => {
                    pool_attempt_done = true;
                    observe_startup_recovery_state(
                        &state,
                        "pool_attempts",
                        0,
                        Duration::ZERO,
                        "complete",
                    );
                }
                Ok(ids) => {
                    match run_startup_recovery_batch(&state, connection_ref, "pool_attempts", &ids)
                        .await
                    {
                        Ok(batch) => {
                            let recovered = publish_startup_recovery_batch(&state, &batch).await;
                            pool_attempt_cursor =
                                ids.last().copied().unwrap_or(pool_attempt_cursor);
                            progressed = true;
                            retry_index = 0;
                            match load_startup_pool_attempt_pending_stats(
                                connection_ref,
                                pool_attempt_cursor,
                                fence.pool_attempt_max_id,
                            )
                            .await
                            {
                                Ok((pending, oldest_age)) => {
                                    observe_startup_recovery_state(
                                        &state,
                                        "pool_attempts",
                                        pending.saturating_add(ids.len().saturating_sub(recovered)),
                                        oldest_age,
                                        "running",
                                    );
                                }
                                Err(error) => {
                                    retry_required = true;
                                    connection_failed = true;
                                    observe_startup_recovery_retry(
                                        &state,
                                        "pool_attempts",
                                        "pending_stats",
                                    );
                                    crate::db_pressure::global_db_pressure_gate()
                                        .record_error("startup_recovery_pool_attempts", &error);
                                    observe_startup_recovery_state(
                                        &state,
                                        "pool_attempts",
                                        0,
                                        Duration::ZERO,
                                        "unavailable",
                                    );
                                    warn!(
                                        error = %error,
                                        "startup pool attempt recovery progress query deferred"
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            retry_required = true;
                            observe_startup_recovery_retry(
                                &state,
                                "pool_attempts",
                                "write_or_lock",
                            );
                            crate::db_pressure::global_db_pressure_gate()
                                .record_error("startup_recovery_pool_attempts", &error);
                            connection_failed = true;
                            warn!(error = %error, "startup pool attempt recovery batch deferred");
                        }
                    }
                }
                Err(error) => {
                    retry_required = true;
                    observe_startup_recovery_retry(&state, "pool_attempts", "read_or_lock");
                    connection_failed = true;
                    warn!(error = %error, "startup pool attempt recovery batch read deferred");
                }
            }
        }
        if connection_failed {
            connection = None;
        }

        let journal = state.sqlite_batch_writer.terminal_journal_stats();
        observe_startup_recovery_state(
            &state,
            "terminal_journal",
            journal.pending_materialization_records,
            Duration::from_millis(journal.oldest_pending_age_ms),
            if journal.unavailable {
                "unavailable"
            } else if journal.pending_materialization_records == 0 {
                "complete"
            } else {
                "running"
            },
        );
        if journal.unavailable {
            warn!(
                "startup recovery stopped because terminal journal recovery became unavailable; durable obligations remain for the next start"
            );
            return;
        }
        if invocation_done
            && pool_attempt_done
            && !journal.unavailable
            && journal.pending_materialization_records == 0
        {
            info!("startup recovery coordinator completed fenced recovery");
            return;
        }
        if retry_required || !progressed {
            if !wait_for_startup_recovery_retry(
                &state.shutdown,
                STARTUP_RECOVERY_RETRY_DELAYS[retry_index],
            )
            .await
            {
                warn!(
                    "startup recovery retry wait cancelled; durable obligations remain for the next start"
                );
                return;
            }
            retry_index = (retry_index + 1).min(STARTUP_RECOVERY_RETRY_DELAYS.len() - 1);
        } else {
            retry_index = 0;
        }
    }
}

pub(crate) fn next_startup_hourly_rollup_p2_preemption_retry(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX)
        .min(STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX)
}

pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration(
    state: Arc<AppState>,
    startup_started_at: Instant,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        None,
        None,
        None,
    )
}

fn publish_http_readiness_and_spawn_recovery(
    state: Arc<AppState>,
    startup_started_at: Instant,
    fence: StartupRecoveryFence,
    recovery_connect_options: Option<Arc<SqliteConnectOptions>>,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        None,
        Some(fence),
        recovery_connect_options,
    )
}

#[cfg(test)]
pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_deadline: Duration,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        summary_deadline,
        None,
        None,
        None,
    )
}

#[cfg(test)]
pub(crate) fn startup_recovery_fence_for_test(
    invocation_max_id: i64,
    pool_attempt_max_id: i64,
) -> StartupRecoveryFence {
    StartupRecoveryFence {
        invocation_max_id,
        pool_attempt_max_id,
        captured: true,
    }
}

#[cfg(test)]
pub(crate) fn publish_http_readiness_and_spawn_recovery_for_test(
    state: Arc<AppState>,
    startup_started_at: Instant,
    fence: StartupRecoveryFence,
    recovery_connect_options: SqliteConnectOptions,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        None,
        Some(fence),
        Some(Arc::new(recovery_connect_options)),
    )
}

#[cfg(test)]
pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_start_delay: Duration,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        Some(summary_start_delay),
        None,
        None,
    )
}

fn publish_http_readiness_and_spawn_hot_read_hydration_with_options(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_deadline: Duration,
    summary_start_delay: Option<Duration>,
    startup_recovery_fence: Option<StartupRecoveryFence>,
    startup_recovery_connect_options: Option<Arc<SqliteConnectOptions>>,
) -> JoinHandle<()> {
    state.startup_ready.store(true, Ordering::Release);
    let readiness_at = Instant::now();
    info!(
        time_to_health_ms = startup_started_at.elapsed().as_millis() as u64,
        "application readiness reached"
    );

    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let recovery_handle = startup_recovery_fence
            .zip(startup_recovery_connect_options)
            .map(|(fence, recovery_connect_options)| {
                crate::observability::contention::spawn_performance(
                    state.observability.clone(),
                    run_startup_recovery(
                        state.clone(),
                        fence,
                        readiness_at,
                        recovery_connect_options,
                    ),
                )
            });
        let post_readiness_warmups_handle = crate::observability::contention::spawn_performance(
            state.observability.clone(),
            run_post_readiness_runtime_warmups(state.clone()),
        );
        // Spawn the independent system-status worker before taking the Summary priority
        // reservation. A busy database gate must never delay the readiness-side status cache.
        let system_status_handle = crate::observability::contention::spawn_performance(
            state.observability.clone(),
            hydrate_system_status_at_startup(state.clone()),
        );
        let summary_startup_priority =
            crate::db_pressure::global_db_pressure_gate().reserve_priority_background();
        let summary_state = state.clone();
        let summary_handle = crate::observability::contention::spawn_performance(
            summary_state.observability.clone(),
            async move {
                let startup_priority = hydrate_summary_at_startup(
                    summary_state.clone(),
                    summary_deadline,
                    summary_start_delay,
                    summary_startup_priority,
                )
                .await;
                if let Some(startup_priority) = startup_priority {
                    spawn_summary_snapshot_maintenance(summary_state.clone());
                    spawn_summary_coverage_recovery_maintenance(summary_state, startup_priority);
                    true
                } else {
                    false
                }
            },
        );
        let (summary_result, system_status_result, post_readiness_warmups_result) = tokio::join!(
            summary_handle,
            system_status_handle,
            post_readiness_warmups_handle
        );

        match summary_result {
            Ok(true) | Ok(false) => {}
            Err(error) => {
                warn!(
                    ?error,
                    "summary projection startup hydration worker ended unexpectedly"
                );
            }
        }
        if let Err(error) = system_status_result {
            warn!(
                ?error,
                "system status startup hydration worker ended unexpectedly"
            );
        }
        if let Err(error) = post_readiness_warmups_result {
            warn!(
                ?error,
                "post-readiness runtime warm-up worker ended unexpectedly"
            );
        }
        if let Some(recovery_handle) = recovery_handle
            && let Err(error) = recovery_handle.await
        {
            warn!(error = %error, "startup recovery coordinator ended unexpectedly");
        }
    })
}

async fn run_post_readiness_runtime_warmups(state: Arc<AppState>) {
    let shutdown = state.shutdown.clone();
    tokio::select! {
        _ = shutdown.cancelled() => {}
        _ = run_post_readiness_runtime_warmups_inner(state) => {}
    }
}

async fn run_post_readiness_runtime_warmups_inner(state: Arc<AppState>) {
    if let Err(error) = runtime_route_facts(state.as_ref()).await {
        warn!(error = %error, "runtime route facts warm-up failed after readiness");
    }
    warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;
    warm_dashboard_runtime_projection(state.as_ref()).await;
    if let Err(error) = hydrate_raw_capture_circuit(state.as_ref()).await {
        warn!(error = %error, "raw capture circuit hydration failed; keeping capture fail-closed");
    }
    recover_raw_overflow_spools_with_circuit(state.as_ref()).await;
    crate::api::register_dashboard_runtime_projection_handle(
        spawn_dashboard_runtime_projection_reconcile(state),
    );
}

async fn hydrate_summary_at_startup(
    state: Arc<AppState>,
    summary_deadline: Duration,
    summary_start_delay: Option<Duration>,
    mut startup_priority: crate::db_pressure::DbBackgroundPriorityReservation,
) -> Option<crate::db_pressure::DbBackgroundPriorityReservation> {
    if let Some(delay) = summary_start_delay {
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            _ = tokio::time::sleep(delay) => {}
        }
    }

    let mut retry_delay = STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL;
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            result = hydrate_summary_snapshots_with_deadline(state.as_ref(), summary_deadline) => match result {
                Ok(()) => {
                    info!("summary projection startup hydration completed");
                    return Some(startup_priority);
                }
                Err(error) => {
                    // A failed Bootstrap must not reserve the only background slot across its
                    // exponential retry delay. The next cold attempt receives a fresh bounded
                    // reservation when it begins.
                    drop(startup_priority);
                    warn!(?error, "summary projection startup hydration failed; retaining unavailable or last-good response");
                }
            }
        }

        warn!(
            retry_after_secs = retry_delay.as_secs(),
            "summary projection startup hydration deferred behind bounded pressure backoff"
        );
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            _ = tokio::time::sleep(retry_delay) => {}
        }
        retry_delay = retry_delay
            .saturating_mul(2)
            .min(STARTUP_HOT_READ_HYDRATION_RETRY_MAX);
        startup_priority =
            crate::db_pressure::global_db_pressure_gate().reserve_priority_background();
    }
}

async fn hydrate_system_status_at_startup(state: Arc<AppState>) {
    let mut retry_delay = STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL;
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => return,
            result = hydrate_system_status_snapshot(state.as_ref()) => match result {
                Ok(()) => {
                    info!("system status startup hydration completed");
                    return;
                }
                Err(error) => {
                    warn!(?error, "system status startup hydration failed; retaining unavailable or last-good response");
                }
            }
        }

        warn!(
            retry_after_secs = retry_delay.as_secs(),
            "system status startup hydration deferred behind bounded pressure backoff"
        );
        tokio::select! {
            _ = state.shutdown.cancelled() => return,
            _ = tokio::time::sleep(retry_delay) => {}
        }
        retry_delay = retry_delay
            .saturating_mul(2)
            .min(STARTUP_HOT_READ_HYDRATION_RETRY_MAX);
    }
}

pub(crate) async fn run() -> Result<()> {
    RuntimeProjectionMode::reject_removed_legacy_env()?;
    init_tracing();
    let startup_started_at = Instant::now();

    let cli = CliArgs::parse();
    let config = AppConfig::from_sources(&cli)?;
    if let Some(CliCommand::Maintenance(MaintenanceCliArgs {
        command: MaintenanceCommand::PreparePerformanceIndexes(options),
    })) = &cli.command
    {
        selected_owned_maintenance(&cli)?;
        return run_performance_index_preparation_cli(&config, options).await;
    }
    if let Some((task_key, dry_run)) = selected_owned_maintenance(&cli)? {
        return run_owned_maintenance_cli(&config, task_key, dry_run).await;
    }
    let MaintenanceRuntimeRoute::Offline(mut runtime_lock) =
        MaintenanceRuntimeLock::route(&config, false)?
    else {
        bail!("service runtime is already active");
    };
    runtime_lock.publish_role("service:ownership-v1:initializing")?;
    let spawn_background_hourly_rollup_bootstrap =
        should_spawn_background_startup_hourly_rollup_bootstrap(&cli);
    let (backend_ver, frontend_ver) = detect_versions(config.static_dir.as_deref());
    info!(?config, backend_version = %backend_ver, frontend_version = %frontend_ver, "starting codex vibe monitor");

    let database_url = config.database_url();
    ensure_db_directory(&config.database_path)?;
    let recovery_connect_opts =
        build_sqlite_connect_options(&database_url, STARTUP_RECOVERY_BUSY_TIMEOUT)?;
    let recovery_connect_options = Arc::new(
        runtime_lock.sqlite_connect_options(&config.database_path, recovery_connect_opts)?,
    );
    let connect_opts = build_sqlite_connect_options(
        &database_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )?;
    let connect_opts = runtime_lock.sqlite_connect_options(&config.database_path, connect_opts)?;
    let db_connect_started_at = Instant::now();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .context("failed to open sqlite database")?;
    let mut maintenance_pool = None;
    let observability = ObservabilityRuntime::new(config.observability.enabled);
    let outcome = crate::observability::contention::with_performance(observability.clone(), async {
        log_startup_phase("db_connect", db_connect_started_at);

        let schema_started_at = Instant::now();
        ensure_schema(&pool).await?;
        match crate::sqlite_batch_writer::replay_account_selected_touch_outbox(&pool).await {
            Ok(replayed) if replayed > 0 => {
                info!(replayed, "replayed account-selection observations at startup");
            }
            Ok(_) => {}
            Err(error) => {
                warn!(error = %error, "account-selection observation outbox replay deferred");
            }
        }
        let storage_migration = Arc::new(
            StorageMigrationRuntime::load(&config.storage, pool.clone())
                .await
                .context("failed to recover durable storage migration state")?,
        );
        initialize_invocation_identity_cleanup_state(&pool).await?;
        let maintenance_store = match crate::maintenance_store::open_owned(&config, &mut runtime_lock).await {
            Ok(store) => {
                maintenance_pool = Some(store.pool.clone());
                if let Err(error) = store.migrate_legacy_state(&pool).await {
                    warn!(error = %error, "legacy task state migration did not complete; keeping maintenance observation unavailable until the next startup retry");
                } else if let Err(error) = store.apply_initial_task_defaults().await {
                    warn!(error = %error, "initial managed task defaults could not be applied; keeping maintenance observation unavailable until the next startup retry");
                } else {
                    match store.recover_incomplete_runs().await {
                        Ok(recovered_runs) => {
                            if recovered_runs > 0 {
                                warn!(
                                    recovered_runs,
                                    "recovered incomplete managed task runs at startup"
                                );
                            }
                            let task =
                                crate::StartupBackfillTask::PromptCacheConversationsMaterialization;
                            let task_name = task.name();
                            let task_suffix =
                                crate::maintenance_store::managed_startup_backfill_suffix(task_name)
                                    .expect("prompt-cache materialization has a managed task key");
                            let task_key = format!("startup_backfill.{task_suffix}");
                            match store
                                .initialize_prompt_cache_materialization_control(&task_key, task_name)
                                .await
                            {
                                Ok(_) => crate::maintenance_store::set_global(Arc::new(store.clone())),
                                Err(error) => warn!(
                                    error = %error,
                                    "prompt-cache materialization control could not be initialized; maintenance control remains unavailable"
                                ),
                            }
                        }
                        Err(error) => {
                            warn!(error = %error, "incomplete managed task runs could not be recovered; keeping maintenance observation unavailable until the next startup retry");
                        }
                    }
                }
                Some(Arc::new(store))
            }
            Err(error) => {
                warn!(error = %error, path = %config.maintenance_database_path().display(), "maintenance database unavailable; operational observation will be stale");
                None
            }
        };
        log_startup_phase("schema", schema_started_at);
        let startup_prep_held = storage_migration
            .migration_hold_reason("startup_hourly_rollup_bootstrap")
            .await
            .is_some();
        if should_run_blocking_startup_persistent_prep(&cli) && !startup_prep_held {
            let prep_summary = run_startup_persistent_prep_inner_with_store(
                maintenance_store.as_deref(),
                &pool,
                &config,
                &cli,
                true,
            )
            .await?;
            info!(
                migration_hold_deferred = prep_summary.deferred,
                stale_archive_temp_files_removed = prep_summary.stale_archive_temp_files_removed,
                refreshed_manifest_batches = prep_summary.refreshed_manifest_batches,
                refreshed_manifest_account_rows = prep_summary.refreshed_manifest_account_rows,
                missing_manifest_files = prep_summary.missing_manifest_files,
                backfilled_archive_expiries = prep_summary.backfilled_archive_expiries,
                bootstrapped_hourly_rollups = prep_summary.bootstrapped_hourly_rollups,
                pending_manifest_batches = prep_summary.pending_manifest_batches,
                pending_historical_rollup_archive_batches =
                    prep_summary.pending_historical_rollup_archive_batches,
                "startup persistent prep finished"
            );
            if prep_summary.pending_historical_rollup_archive_batches > 0 {
                warn!(
                    pending_historical_rollup_archive_batches =
                        prep_summary.pending_historical_rollup_archive_batches,
                    "legacy archive batches still need historical rollup materialization"
                );
            }
        } else if startup_prep_held {
            info!(
                "startup persistent prep deferred because storage migration holds competing tasks"
            );
        }
        if let Some(command) = &cli.command {
            run_cli_command(&pool, &config, command).await?;
            return Ok(());
        }
        if crate::maintenance_store::global().is_some() {
            runtime_lock.refresh_inode_pair_lock()?;
            runtime_lock.publish_role("service:ownership-v1:ready")?;
        }

        let pricing_catalog = load_pricing_catalog(&pool).await?;
        ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config).await?;
        let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(&pool).await?));
        let forward_proxy_settings = load_forward_proxy_settings(&pool).await?;
        let forward_proxy_runtime = load_forward_proxy_runtime_states(&pool).await?;
        let oauth_installation_seed = oauth_bridge::load_or_init_oauth_installation_seed(&pool).await?;
        let forward_proxy = Arc::new(Mutex::new(ForwardProxyManager::with_algo(
            forward_proxy_settings,
            forward_proxy_runtime,
            config.forward_proxy_algo,
        )));
        let resolved_proxy_raw_dir = config.resolved_proxy_raw_dir();
        fs::create_dir_all(&resolved_proxy_raw_dir).with_context(|| {
            format!(
                "failed to create proxy raw payload directory: {}",
                resolved_proxy_raw_dir.display()
            )
        })?;
        let pricing_catalog = Arc::new(RwLock::new(pricing_catalog));

        let http_clients = HttpClients::build(&config)?;
        let upstream_accounts = Arc::new(UpstreamAccountsRuntime::from_env()?);
        let (tx, _rx) = broadcast::channel(128);
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let proxy_raw_async_semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config)));
        let shutdown = CancellationToken::new();
        let storage_runtime =
            crate::storage::runtime::StorageRuntimeRouter::initialize(
                pool.clone(),
                &config.storage,
                storage_migration.clone(),
                shutdown.clone(),
            )
            .await
            .context("failed to initialize storage runtime router")?;
        let process_started_at_utc = Utc::now();
        observability
            .initialize_traces(config.observability.traces.clone())
            .await;

        observability.start_exporter(&config.observability, shutdown.clone());
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .bind_observability(observability.clone())?;

        let prompt_cache_conversation_cache =
            Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
        prompt_cache_conversation_cache
            .lock()
            .await
            .identity_cache
            .range_manager
            .bind_observability(observability.clone());
        prompt_cache_conversation_cache
            .lock()
            .await
            .identity_cache
            .range_manager
            .start_sizing(&pool, shutdown.clone());
        let proxy_runtime_invocations =
            Arc::new(RuntimeProjectionHub::new(RuntimeProjectionMode::Auto));
        let proxy_runtime_attempts = Arc::new(RuntimeAttemptStore::default());
        let dashboard_network_speed_cache =
            Arc::new(DashboardNetworkSpeedCache::new(process_started_at_utc));
        proxy_runtime_invocations
            .bind_dashboard_network_speed_cache(dashboard_network_speed_cache.clone())?;
        let dashboard_activity_snapshot_cache =
            Arc::new(Mutex::new(DashboardActivitySnapshotCacheState::default()));
        let terminal_projection_hub = Arc::new(TerminalProjectionHub::default());
        let long_term_projection_runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
        let memory_diagnostics = Arc::new(MemoryDiagnosticsRuntime::new());
        let traffic_authority = storage_runtime.traffic_authority();
        let sqlite_batch_writer = SqliteBatchWriter::spawn_with_traffic_authority(
            pool.clone(),
            traffic_authority.clone(),
            shutdown.clone(),
            prompt_cache_conversation_cache.clone(),
            pricing_catalog.clone(),
            &config.database_path,
        )?;
        sqlite_batch_writer.bind_observability(observability.clone());
        sqlite_batch_writer.set_terminal_runtime_store(proxy_runtime_invocations.clone());
        sqlite_batch_writer.set_runtime_attempt_store(proxy_runtime_attempts.clone());
        sqlite_batch_writer
            .set_dashboard_activity_snapshot_cache(dashboard_activity_snapshot_cache.clone());
        sqlite_batch_writer.set_terminal_projection_hub(terminal_projection_hub.clone());
        let pool_account_selection_runtime = Arc::new(PoolAccountSelectionRuntime::default());
        let subscription_hub = Arc::new(SubscriptionHub::new());
        sqlite_batch_writer.set_summary_delta_hub(subscription_hub.clone());
        terminal_projection_hub.set_runtime_mutation_bus(subscription_hub.runtime_mutation_bus());

        let state = Arc::new(AppState {
            system_storage: Arc::new(SystemStorageRuntime::new(&config)),
            storage_migration,
            storage_runtime: Some(storage_runtime.clone()),
            config: config.clone(),
            pool: pool.clone(),
            traffic_authority: traffic_authority.clone(),
            process_started_at_utc,
            observability,
            sqlite_batch_writer,
            pool_account_selection_runtime,
            proxy_runtime_invocations,
            proxy_runtime_attempts,
            dashboard_network_speed_cache,
            oauth_installation_seed,
            hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
            http_clients,
            broadcaster: tx.clone(),
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            subscription_hub: subscription_hub.clone(),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
            dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
            dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(false)),
            shutdown: shutdown.clone(),
            semaphore: semaphore.clone(),
            proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
            proxy_raw_async_semaphore,
            raw_capture_circuit: Arc::new(RawCaptureCircuitBreaker::new(
                config.resolved_proxy_raw_dir(),
            )),
            proxy_model_settings,
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy,
            xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
                config.xray_binary.clone(),
                config.xray_runtime_dir.clone(),
            ))),
            forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog,
            prompt_cache_conversation_cache,
            dashboard_activity_snapshot_cache,
            terminal_projection_hub,
            long_term_projection_runtime,
            memory_diagnostics,
            maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
            system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
            pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
            pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
            pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
            pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
            pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
            pool_group_429_retry_delay_override: None,
            #[cfg(test)]
            fallback_proxy_429_retry_delay_override: None,
            pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
            upstream_accounts,
        });
        storage_runtime.start().await;
        if let Some(store) = crate::maintenance_store::global().cloned() {
            crate::task_timeline::start_recorder(store).await;
        }
        // Listen for shutdown before the readiness-gated hydration loop so an unavailable
        // persistent baseline can be interrupted cleanly without publishing partial HTTP state.
        let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());
        // Only database/schema checks and listener setup remain before readiness. Routing,
        // dashboard, raw-capture, and recovery warm-ups run from the post-readiness coordinator.
        spawn_subscription_broadcast_listener(state.clone());
        spawn_system_raw_payload_metrics_inventory(state.clone(), state.shutdown.clone());
        spawn_memory_diagnostics(state.clone(), state.shutdown.clone());
        spawn_observability_sampler(state.clone());

        run_runtime_until_shutdown(
            state,
            startup_started_at,
            spawn_background_hourly_rollup_bootstrap,
            Some(recovery_connect_options),
            async move {
                let _ = signal_listener.await;
            },
        )
        .await
    })
    .await;
    // The process lifetime lock also covers SQL workers left by startup failures or shutdown.
    pool.close().await;
    if let Some(maintenance_pool) = maintenance_pool {
        maintenance_pool.close().await;
    }
    outcome
}

pub(crate) const POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) async fn warm_pool_routing_runtime_cache_best_effort(state: &AppState) {
    refresh_pool_routing_runtime_cache_best_effort(state, "startup warmup").await;
}

pub(crate) async fn refresh_pool_routing_runtime_cache_best_effort(
    state: &AppState,
    operation: &'static str,
) {
    if let Err(err) = refresh_pool_routing_runtime_cache(state).await {
        warn!(
            error = %err,
            operation,
            "failed to refresh pool routing runtime cache; falling back to lazy pool routing resolution"
        );
    }
}

pub(crate) fn begin_runtime_shutdown_if_requested<F>(
    shutdown_signal: &Shared<F>,
    cancel: &CancellationToken,
) -> bool
where
    F: Future<Output = ()>,
{
    if cancel.is_cancelled() {
        return true;
    }
    if shutdown_signal.clone().now_or_never().is_some() {
        begin_runtime_shutdown(cancel);
        return true;
    }
    false
}

pub(crate) enum StartupStageOutcome<T> {
    SkippedByShutdown,
    Completed { result: T, shutdown_requested: bool },
}

pub(crate) struct TrackedStartupStage<Stage> {
    stage: std::pin::Pin<Box<Stage>>,
    started: bool,
}

impl<Stage> TrackedStartupStage<Stage> {
    fn new(stage: Stage) -> Self {
        Self {
            stage: Box::pin(stage),
            started: false,
        }
    }

    fn has_started(&self) -> bool {
        self.started
    }
}

impl<Stage> TrackedStartupStage<Stage>
where
    Stage: Future,
{
    async fn finish(&mut self) -> Stage::Output {
        self.started = true;
        self.stage.as_mut().await
    }
}

impl<Stage> Future for TrackedStartupStage<Stage>
where
    Stage: Future,
{
    type Output = Stage::Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.started = true;
        this.stage.as_mut().poll(cx)
    }
}

pub(crate) async fn run_startup_stage_until_shutdown<T, Stage, Shutdown>(
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    stage: Stage,
) -> StartupStageOutcome<T>
where
    Stage: Future<Output = T>,
    Shutdown: Future<Output = ()>,
{
    if begin_runtime_shutdown_if_requested(shutdown_signal, cancel) {
        return StartupStageOutcome::SkippedByShutdown;
    }

    let stage = TrackedStartupStage::new(stage);
    tokio::pin!(stage);
    tokio::select! {
        biased;
        _ = shutdown_signal.clone() => {
            begin_runtime_shutdown(cancel);
            if stage.as_ref().get_ref().has_started() {
                StartupStageOutcome::Completed {
                    result: stage.as_mut().get_mut().finish().await,
                    shutdown_requested: true,
                }
            } else {
                StartupStageOutcome::SkippedByShutdown
            }
        }
        result = &mut stage => StartupStageOutcome::Completed {
            shutdown_requested: begin_runtime_shutdown_if_requested(shutdown_signal, cancel),
            result,
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn drain_runtime_after_pending_shutdown(
    state: Arc<AppState>,
    mut shutdown_watcher: JoinHandle<()>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
    startup_hot_read_hydration_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    let shutdown_cancel = state.shutdown.clone();
    tokio::select! {
        _ = shutdown_cancel.cancelled() => {
            shutdown_watcher.abort();
            let _ = shutdown_watcher.await;
        }
        _ = &mut shutdown_watcher => {}
    }
    drain_runtime_after_shutdown(
        state,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        pool_orphan_recovery_handle,
        retention_handle,
        startup_backfill_handle,
        startup_hot_read_hydration_handle,
    )
    .await
}

pub(crate) async fn run_runtime_until_shutdown<F>(
    state: Arc<AppState>,
    startup_started_at: Instant,
    spawn_background_hourly_rollup_bootstrap: bool,
    startup_recovery_connect_options: Option<Arc<SqliteConnectOptions>>,
    shutdown_signal: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let cancel = state.shutdown.clone();
    let runtime_init_started_at = Instant::now();
    let shutdown_signal = shutdown_signal.shared();
    let shutdown_cancel = cancel.clone();
    let shutdown_relay_signal = shutdown_signal.clone();
    let shutdown_watcher = tokio::spawn(async move {
        shutdown_relay_signal.await;
        begin_runtime_shutdown(&shutdown_cancel);
    });
    let poller_handle = None;
    let mut upstream_accounts_handle = None;
    let mut forward_proxy_handle = None;
    let mut pool_orphan_recovery_handle = None;
    let mut retention_handle = None;
    let mut server_handle = None;
    let mut startup_backfill_handle = None;
    let mut startup_hot_read_hydration_handle = None;

    let sync_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        sync_forward_proxy_routes(state.as_ref()),
    )
    .await;
    let sync_shutdown_requested = match sync_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            if let Err(err) = result {
                warn!(error = %err, "failed to initialize forward proxy xray routes at startup");
            }
            shutdown_requested
        }
    };
    log_startup_phase("runtime_init", runtime_init_started_at);
    if sync_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let upstream_accounts_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_upstream_account_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let upstream_accounts_shutdown_requested = match upstream_accounts_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            upstream_accounts_handle = result;
            shutdown_requested
        }
    };
    if upstream_accounts_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let forward_proxy_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        Some(spawn_forward_proxy_maintenance(
            state.clone(),
            cancel.clone(),
        ))
    })
    .await;
    let forward_proxy_shutdown_requested = match forward_proxy_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            forward_proxy_handle = result;
            shutdown_requested
        }
    };
    if forward_proxy_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let retention_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        // The dispatcher is the sole automatic executor of all owned maintenance.
        None
    })
    .await;
    let retention_shutdown_requested = match retention_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            retention_handle = result;
            shutdown_requested
        }
    };
    if retention_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let startup_recovery_fence = match startup_recovery_connect_options.as_deref() {
        Some(recovery_connect_options) => {
            capture_startup_recovery_fence_bounded(recovery_connect_options).await
        }
        None => StartupRecoveryFence::empty(),
    };
    let http_ready_started_at = Instant::now();
    let http_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        spawn_http_server(state.clone()),
    )
    .await;
    let http_shutdown_requested = match http_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            let (_http_addr, handle) = result?;
            server_handle = Some(handle);
            shutdown_requested
        }
    };
    if http_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    startup_hot_read_hydration_handle = Some(publish_http_readiness_and_spawn_recovery(
        state.clone(),
        startup_started_at,
        startup_recovery_fence,
        startup_recovery_connect_options,
    ));
    // The periodic sweep is a runtime recovery path, not part of the fenced startup batch. It
    // remains available after a fence capture failure, while the shared owner gate prevents it
    // from overlapping the startup coordinator when that coordinator is still active.
    pool_orphan_recovery_handle = Some(spawn_pool_orphan_recovery_maintenance(
        state.clone(),
        cancel.clone(),
    ));
    log_startup_phase("http_ready", http_ready_started_at);
    spawn_system_status_snapshot_maintenance(state.clone());
    spawn_system_storage_maintenance(state.clone());
    spawn_invocation_timeline_snapshot_maintenance(state.clone());

    let startup_backfill_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_startup_backfill_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let startup_backfill_shutdown_requested = match startup_backfill_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            startup_backfill_handle = result;
            shutdown_requested
        }
    };
    if startup_backfill_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let managed_task_dispatcher_handle = spawn_managed_task_dispatcher(state.clone());
    let _retention_backlog_observer_handle =
        crate::maintenance::spawn_retention_backlog_observer(state.clone(), cancel.clone());

    let startup_hourly_rollup_bootstrap_handle = spawn_background_hourly_rollup_bootstrap
        .then(|| spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), cancel.clone()));

    tokio::select! {
        biased;
        _ = shutdown_signal => begin_runtime_shutdown(&cancel),
        _ = cancel.cancelled() => {}
    }

    if let Some(startup_hourly_rollup_bootstrap_handle) = startup_hourly_rollup_bootstrap_handle
        && let Err(err) = startup_hourly_rollup_bootstrap_handle.await
    {
        error!(
            ?err,
            "background startup hourly rollup bootstrap task terminated unexpectedly"
        );
    }

    if let Err(error) = managed_task_dispatcher_handle.await {
        warn!(error = %error, "managed task dispatcher task terminated during shutdown");
    }
    drain_runtime_after_pending_shutdown(
        state,
        shutdown_watcher,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        pool_orphan_recovery_handle,
        retention_handle,
        startup_backfill_handle,
        startup_hot_read_hydration_handle,
    )
    .await
}

fn spawn_managed_task_dispatcher(state: Arc<AppState>) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let Some(store) = crate::maintenance_store::global().cloned() else {
            return;
        };
        let mut pending_finishes = VecDeque::new();
        let mut workers = tokio::task::JoinSet::new();
        let mut heavy_waiters = tokio::task::JoinSet::new();
        let mut finalizers = tokio::task::JoinSet::new();
        let heavy_file_gate = Arc::new(Semaphore::new(1));
        loop {
            while finalizers.len() < MANAGED_TASK_MAX_ACTIVE_FINALIZERS {
                let Some(finish) = pending_finishes.pop_front() else {
                    break;
                };
                let store = store.clone();
                finalizers.spawn(async move {
                    finalize_managed_task_with_panic_guard(store, finish).await
                });
            }
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    // A worker owns both the per-task execution lease and any native file
                    // operation permit. Await every worker so cancellation cannot publish a
                    // false idle state while a blocking archive worker is still draining.
                    while let Some(joined) = workers.join_next().await {
                        if let Ok(finish) = joined {
                            queue_managed_task_finish(&mut pending_finishes, finish);
                        }
                    }
                    while let Some(joined) = heavy_waiters.join_next().await {
                        if let Ok(finish) = joined {
                            queue_managed_task_finish(&mut pending_finishes, finish);
                        }
                    }
                    while let Some(joined) = finalizers.join_next().await {
                        if let Ok((finish, Err(error))) = joined {
                            warn!(
                                run_id = finish.run_id,
                                task = %finish.task_key,
                                error = %error,
                                "managed task finalization failed during shutdown"
                            );
                            queue_managed_task_finish(&mut pending_finishes, finish);
                        }
                    }
                    drain_managed_task_finishes_on_shutdown(
                        state.as_ref(),
                        &store,
                        &mut pending_finishes,
                    )
                    .await;
                    return;
                },
                Some(joined) = workers.join_next(), if !workers.is_empty() => {
                    match joined {
                        Ok(finish) => queue_managed_task_finalizer(
                            &mut finalizers,
                            &mut pending_finishes,
                            &store,
                            finish,
                        ),
                        Err(error) => warn!(error = %error, "managed task worker panicked"),
                    }
                },
                Some(joined) = heavy_waiters.join_next(), if !heavy_waiters.is_empty() => {
                    match joined {
                        Ok(finish) => queue_managed_task_finalizer(
                            &mut finalizers,
                            &mut pending_finishes,
                            &store,
                            finish,
                        ),
                        Err(error) => warn!(error = %error, "managed heavy-file task waiter panicked"),
                    }
                },
                Some(joined) = finalizers.join_next(), if !finalizers.is_empty() => {
                    match joined {
                        Ok((_finish, Ok(()))) => {}
                        Ok((finish, Err(error))) => {
                            warn!(run_id=finish.run_id, task=%finish.task_key, error=%error,
                                "managed task dispatcher deferred task-history finalization");
                            queue_managed_task_finish(&mut pending_finishes, finish);
                        }
                        Err(error) => warn!(error = %error, "managed task finalizer panicked"),
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if workers.len() >= MANAGED_TASK_MAX_ACTIVE_WORKERS
                        || pending_finishes
                            .len()
                            .saturating_add(finalizers.len())
                            .saturating_add(workers.len())
                            .saturating_add(heavy_waiters.len())
                            >= MANAGED_TASK_MAX_PENDING_FINISHES
                    {
                        continue;
                    }
                    // Replayed system-task finishes can carry the only durable retry intent for
                    // an interrupted generation. Keep those owner keys fenced while allowing
                    // unrelated managed tasks to continue dispatching.
                    let Some(blocked_task_keys) = state
                        .sqlite_batch_writer
                        .system_task_recovery_blocked_task_keys()
                    else {
                        continue;
                    };
                    let mut blocked_task_keys = blocked_task_keys.into_iter().collect::<Vec<_>>();
                    let held_task_keys = match store.migration_held_managed_task_keys().await {
                        Ok(held_task_keys) => held_task_keys,
                        Err(error) => {
                            warn!(
                                error = %error,
                                "managed task dispatcher could not load migration hold state"
                            );
                            continue;
                        }
                    };
                    blocked_task_keys.extend(held_task_keys);
                    blocked_task_keys.extend(
                        crate::maintenance_store::active_task_execution_task_keys(),
                    );
                    blocked_task_keys.extend(state.storage_migration.held_task_keys().await);
                    blocked_task_keys.sort();
                    blocked_task_keys.dedup();
                    if blocked_task_keys.iter().any(|task_key| {
                        task_key == crate::terminal_journal::UNKNOWN_SYSTEM_TASK_RECOVERY_TASK_KEY
                    }) {
                        // Legacy recovery records do not identify their managed owner. Keep the
                        // dispatcher stopped until replay resolves the run_id to its real task.
                        continue;
                    }
                    let has_dispatch_work = match store
                        .probe_due_or_requested_runs_excluding(&blocked_task_keys)
                        .await
                    {
                        Ok(has_work) => has_work,
                        Err(error) => {
                            warn!(error = %error, "managed task dispatcher failed to probe for work");
                            continue;
                        }
                    };
                    if !has_dispatch_work {
                        continue;
                    }
                    if let Err(error) = store
                        .enqueue_due_runs_excluding(&blocked_task_keys)
                        .await
                    {
                        warn!(error = %error, "managed task dispatcher failed to enqueue scheduled runs");
                    }
                    let claim = match store
                        .claim_requested_run_excluding(&blocked_task_keys)
                        .await
                    {
                        Ok(claim) => claim,
                        Err(error) => {
                            warn!(error = %error, "managed task dispatcher failed to claim a requested run");
                            continue;
                        }
                    };
                    let Some((run_id, task_key, requested_at, trigger_kind)) = claim else {
                        continue;
                    };
                    let migration_hold_context = match store.migration_task_hold_context(&task_key).await {
                        Ok(context) => context,
                        Err(error) => {
                            warn!(
                                task = %task_key,
                                error = %error,
                                "managed task admission could not verify migration hold"
                            );
                            if let Err(requeue_error) = store.requeue_claimed_run(run_id).await {
                                warn!(
                                    run_id,
                                    task = %task_key,
                                    error = %requeue_error,
                                    "managed task admission error could not return the requested run to the queue"
                                );
                            }
                            continue;
                        }
                    };
                    if let Some(migration_hold_context) = migration_hold_context {
                        let requeued = match store
                            .requeue_claimed_run_for_migration_hold(
                                run_id,
                                migration_hold_context.hold_generation,
                                migration_hold_context.checkpoint.as_deref(),
                            )
                            .await
                        {
                            Ok(requeued) => requeued,
                            Err(error) => {
                                warn!(
                                    run_id,
                                    task = %task_key,
                                    error = %error,
                                    "migration-held managed task could not return to the queue"
                                );
                                false
                            }
                        };
                        if !requeued
                            && let Err(error) = store.requeue_claimed_run(run_id).await
                        {
                            warn!(
                                run_id,
                                task = %task_key,
                                error = %error,
                                "migration-held managed task could not return the claimed run to the queue"
                            );
                        }
                        if let Err(error) = store
                            .mark_task_migration_held_at_generation(
                                &task_key,
                                migration_hold_context.hold_generation,
                            )
                            .await
                        {
                            warn!(
                                task = %task_key,
                                error = %error,
                                "migration-held managed task progress could not be published"
                            );
                        }
                        crate::task_timeline::notify_runtime_changed();
                        continue;
                    }
                    crate::task_timeline::notify_runtime_changed();
                    let Some(execution_lease) =
                        crate::maintenance_store::try_acquire_task_execution(&task_key)
                    else {
                        if let Err(error) = store.requeue_claimed_run(run_id).await {
                            warn!(
                                run_id,
                                task = %task_key,
                                error = %error,
                                "managed task lease conflict could not return the requested run to the queue"
                            );
                        }
                        continue;
                    };
                    let initial_stages = (task_key == "retention_archive").then(|| {
                        vec![crate::maintenance_store::TaskStage {
                            name: "archive".to_string(),
                            status: "running".to_string(),
                            completed: None,
                            total: None,
                            elapsed_ms: None,
                            wait_reason: None,
                            checkpoint: Some("admitted".to_string()),
                        }]
                    });
                    if let Err(error) = store
                        .publish_progress(
                            &task_key,
                            None,
                            None,
                            Some("running"),
                            Some("admitted"),
                            None,
                            (task_key == "retention_archive").then_some("expired_invocations"),
                            None,
                            None,
                            initial_stages.as_deref(),
                        )
                        .await
                    {
                        warn!(task = %task_key, error = %error, "managed task progress snapshot could not be published");
                    }
                    let observation = crate::TaskExecutionObservation::begin_for_managed_run(
                        &task_key,
                        &crate::maintenance_store::task_title_for_observation(&task_key),
                        &trigger_kind,
                        crate::maintenance_store::task_execution_class(&task_key),
                        if managed_task_requires_heavy_file_operation(&task_key) {
                            "waiting_resources"
                        } else {
                            "processing"
                        },
                        Some(run_id),
                    );
                    let worker_file_gate =
                        managed_task_requires_heavy_file_operation(&task_key)
                            .then(|| heavy_file_gate.clone());
                    let heavy_file_operation = managed_task_requires_heavy_file_operation(&task_key);
                    let worker = run_managed_task_worker_guarded(
                        state.clone(),
                        task_key,
                        run_id,
                        requested_at,
                        trigger_kind,
                        observation,
                        execution_lease,
                        worker_file_gate,
                        initial_stages,
                    );
                    if heavy_file_operation {
                        // Waiting for the single native-file permit must not consume one of the
                        // worker slots reserved for light maintenance tasks.
                        heavy_waiters.spawn(worker);
                    } else {
                        workers.spawn(worker);
                    }
                }
            }
        }
    })
}

fn managed_task_requires_heavy_file_operation(task_key: &str) -> bool {
    matches!(
        task_key,
        "retention_archive"
            | "pool_upstream_attempt_archive"
            | "forward_proxy_attempt_archive"
            | "invocation_detail_prune"
            | "quota_snapshot_compaction"
            | "raw_orphan_sweep"
            | "raw_compression"
            | "archive_upstream_activity_manifest"
            | "materialize_historical_rollups"
            | "startup_persistent_prep"
            | "startup_backfill"
            | "startup_backfill.pool_attempt_public_id_archives"
            | "startup_backfill.upstream_activity_archives"
            | "startup_backfill.pool_upstream_node_health_archives"
            | "startup_backfill.legacy_detail_mirrors"
            | "startup_backfill.historical_rollups"
            | "verify_archive_storage"
            | "prune_archive_batches"
            | "prune_legacy_archive_batches"
    )
}

fn is_retention_owner_v2_task(task_key: &str) -> bool {
    matches!(
        task_key,
        "retention_archive"
            | "pool_upstream_attempt_archive"
            | "forward_proxy_attempt_archive"
            | "invocation_detail_prune"
            | "quota_snapshot_compaction"
            | "model_route_expiry"
            | "task_run_history_cleanup"
    )
}

fn managed_task_owner_v2_failure_details(
    task_key: &str,
    trigger_kind: &str,
    wait_reason: &str,
    fatal_error: &str,
    committed: Option<i64>,
    raw_files_removed: Option<i64>,
) -> Option<Value> {
    if !is_retention_owner_v2_task(task_key) {
        return None;
    }
    let committed = committed.map(|value| value.max(0));
    let raw_files_removed = raw_files_removed.map(|value| value.max(0));
    let fatal_error = crate::maintenance_store::sanitize_task_detail(fatal_error);
    let mut details = json!({
        "ownershipVersion": 2,
        "ownerScope": task_key,
        "manual": trigger_kind == "manual",
        "result": "failed",
        "globalBacklogTotal": null,
        "total": null,
        "backlogRemaining": null,
        "discovered": null,
        "selected": null,
        "committed": committed,
        "processedCount": committed,
        "completed": committed,
        "selectedScope": {
            "scope": task_key,
            "unit": "unknown",
            "selected": null,
            "committed": committed,
            "batches": null,
            "rawFilesRemoved": raw_files_removed,
            "hasMore": true,
        },
        "waitReason": wait_reason,
        "fatalError": fatal_error,
        "elapsedMs": 0,
    });
    if trigger_kind != "manual" {
        details["scheduleRetryAfterSecs"] = json!(300);
        details["scheduleRetryReason"] = json!(wait_reason);
    }
    Some(details)
}

fn queue_managed_task_finish(
    pending_finishes: &mut VecDeque<ManagedTaskFinish>,
    finish: ManagedTaskFinish,
) {
    pending_finishes.push_back(finish);
}

fn queue_managed_task_finalizer(
    finalizers: &mut tokio::task::JoinSet<(ManagedTaskFinish, Result<()>)>,
    pending_finishes: &mut VecDeque<ManagedTaskFinish>,
    store: &Arc<crate::maintenance_store::MaintenanceStore>,
    finish: ManagedTaskFinish,
) {
    if finalizers.len() >= MANAGED_TASK_MAX_ACTIVE_FINALIZERS {
        pending_finishes.push_back(finish);
        return;
    }
    let store = Arc::clone(store);
    finalizers.spawn(async move { finalize_managed_task_with_panic_guard(store, finish).await });
}

async fn finalize_managed_task_with_panic_guard(
    store: Arc<crate::maintenance_store::MaintenanceStore>,
    finish: ManagedTaskFinish,
) -> (ManagedTaskFinish, Result<()>) {
    let task_key = finish.task_key.clone();
    let result = AssertUnwindSafe(finish_managed_task_and_schedule_retry(&store, &finish))
        .catch_unwind()
        .await;
    let result = match result {
        Ok(result) => result,
        Err(_) => {
            warn!(task = %task_key, run_id = finish.run_id, "managed task finalizer panicked");
            Err(anyhow!("managed task finalizer panicked"))
        }
    };
    (finish, result)
}

fn managed_task_schedule_retry(details: Option<&Value>) -> Option<(i64, String)> {
    let details = details?;
    let seconds = details
        .get("scheduleRetryAfterSecs")
        .and_then(Value::as_i64)?;
    let reason = details
        .get("scheduleRetryReason")
        .and_then(Value::as_str)?
        .to_string();
    Some((seconds, reason))
}

async fn finish_managed_task_and_schedule_retry(
    store: &crate::maintenance_store::MaintenanceStore,
    finish: &ManagedTaskFinish,
) -> Result<()> {
    let retry = managed_task_schedule_retry(finish.details.as_ref());
    finish_managed_task_run_bounded(store, finish, retry.as_ref()).await?;
    Ok(())
}

async fn run_managed_task_worker_guarded(
    state: Arc<AppState>,
    task_key: String,
    run_id: i64,
    requested_at: String,
    trigger_kind: String,
    observation: crate::TaskExecutionObservation,
    execution_lease: crate::maintenance_store::TaskExecutionLease,
    heavy_file_gate: Option<Arc<Semaphore>>,
    initial_stages: Option<Vec<crate::maintenance_store::TaskStage>>,
) -> ManagedTaskFinish {
    let fallback_task_key = task_key.clone();
    let fallback_requested_at = requested_at.clone();
    let fallback_trigger_kind = trigger_kind.clone();
    let fallback_observation = observation.clone();
    let result = AssertUnwindSafe(run_managed_task_worker(
        state,
        task_key,
        run_id,
        requested_at,
        trigger_kind,
        observation,
        execution_lease,
        heavy_file_gate,
        initial_stages,
    ))
    .catch_unwind()
    .await;
    match result {
        Ok(finish) => finish,
        Err(_) => {
            warn!(
                task = %fallback_task_key,
                run_id,
                "managed task worker panicked; finalizing run as failed"
            );
            ManagedTaskFinish {
                run_id,
                task_key: fallback_task_key.clone(),
                status: SystemTaskStatus::Failed,
                finished_at: format_utc_iso_millis(Utc::now()),
                duration_ms: managed_task_elapsed_ms(&fallback_requested_at, Instant::now()),
                summary: Some(format!("{fallback_task_key} 执行 worker 异常")),
                detail: Some(
                    "managed task worker panicked before publishing a terminal result".to_string(),
                ),
                completion: Some("failed".to_string()),
                core_completion: None,
                migration_hold_generation: None,
                details: managed_task_owner_v2_failure_details(
                    &fallback_task_key,
                    &fallback_trigger_kind,
                    "owner_failed",
                    "managed task worker panicked before publishing a terminal result",
                    fallback_observation.processed_work_value(),
                    fallback_observation.raw_files_removed_value(),
                ),
            }
        }
    }
}

async fn run_managed_task_worker(
    state: Arc<AppState>,
    task_key: String,
    run_id: i64,
    requested_at: String,
    trigger_kind: String,
    observation: crate::TaskExecutionObservation,
    _execution_lease: crate::maintenance_store::TaskExecutionLease,
    heavy_file_gate: Option<Arc<Semaphore>>,
    initial_stages: Option<Vec<crate::maintenance_store::TaskStage>>,
) -> ManagedTaskFinish {
    let _heavy_file_permit = match heavy_file_gate {
        Some(gate) => Some(gate.acquire_owned().await.expect("managed file gate open")),
        None => None,
    };
    let store = crate::maintenance_store::global()
        .cloned()
        .expect("maintenance store available while managed task worker runs");
    let execution_started_at = Instant::now();
    let migration_hold_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut migration_hold_generation = None;
    let task_key_for_future = task_key.clone();
    let mut task_future = Box::pin(crate::maintenance::with_migration_hold_signal(
        Arc::clone(&migration_hold_flag),
        crate::maintenance::with_managed_task_key(
            &task_key_for_future,
            run_managed_task_once_with_observation(
                &state,
                &task_key_for_future,
                run_id,
                &observation,
            ),
        ),
    ));
    let result = tokio::select! {
        biased;
        result = &mut task_future => result,
        _ = wait_for_managed_task_migration_hold(&store, &task_key) => {
            migration_hold_flag.store(true, std::sync::atomic::Ordering::Release);
            (&mut task_future).await
        }
    };
    let result = match result {
        Ok(mut execution) => {
            let migration_hold_observed =
                migration_hold_flag.load(std::sync::atomic::Ordering::Acquire);
            let (migration_hold_context, migration_hold_check_failed) =
                match store.migration_task_hold_context(&task_key).await {
                    Ok(context) => (context, false),
                    Err(error) => {
                        warn!(
                            task = %task_key,
                            error = %error,
                            "managed task completion could not verify migration hold"
                        );
                        (None, true)
                    }
                };
            if let Some(migration_hold_context) = migration_hold_context {
                migration_hold_generation = Some(migration_hold_context.hold_generation);
                apply_migration_hold_to_execution(
                    &mut execution,
                    migration_hold_context.checkpoint,
                );
                if let Err(error) = store
                    .mark_task_migration_held_at_generation(
                        &task_key,
                        migration_hold_context.hold_generation,
                    )
                    .await
                {
                    warn!(
                        task = %task_key,
                        error = %error,
                        "migration-held task progress could not be published after batch"
                    );
                }
            } else if migration_hold_check_failed {
                apply_migration_hold_to_execution(&mut execution, None);
            } else if migration_hold_observed {
                debug!(
                    task = %task_key,
                    "managed task hold was released before completion; retaining normal completion"
                );
            }
            Ok(execution)
        }
        Err(error) => Err(error),
    };
    let (status, summary, detail, completion, core_completion, details) = match result {
        Ok(execution) => {
            let status = match execution.completion.as_deref() {
                Some("partial") | Some("deferred") => SystemTaskStatus::Skipped,
                Some("failed") => SystemTaskStatus::Failed,
                _ => SystemTaskStatus::Success,
            };
            (
                status,
                Some(execution.summary),
                execution.detail,
                execution.completion,
                execution.core_completion,
                execution.details,
            )
        }
        Err(error) => {
            let error_message = crate::maintenance_store::sanitize_task_detail(&error.to_string());
            if trigger_kind != "manual"
                && matches!(
                    task_key.as_str(),
                    "invocation_identity_cleanup" | "raw_orphan_sweep"
                )
                && let Err(retry_error) = store
                    .record_owned_retry(&task_key, is_retention_write_deferred(&error))
                    .await
            {
                warn!(error=%retry_error, task=%task_key, "maintenance safety retry could not be recorded");
            }
            (
                SystemTaskStatus::Failed,
                Some(format!("{task_key} 执行失败")),
                Some(error_message.clone()),
                None,
                None,
                managed_task_owner_v2_failure_details(
                    &task_key,
                    &trigger_kind,
                    "owner_failed",
                    &error_message,
                    observation.processed_work_value(),
                    observation.raw_files_removed_value(),
                ),
            )
        }
    };
    let workload_reason = details
        .as_ref()
        .and_then(|details| details.get("waitReason"))
        .and_then(Value::as_str)
        .or(detail.as_deref());
    observation.finish_with_status_and_reason(
        if state.shutdown.is_cancelled() {
            "interrupted"
        } else {
            status.as_str()
        },
        workload_reason,
    );
    let duration_ms = managed_task_elapsed_ms(&requested_at, execution_started_at);
    let task_dimension = managed_task_metric_dimension(&task_key);
    let task_outcome = match completion.as_deref() {
        Some("partial") => "partial",
        Some("deferred") => "deferred",
        Some("failed") if status == SystemTaskStatus::Failed => "error",
        _ if status == SystemTaskStatus::Success => "success",
        _ if status == SystemTaskStatus::Failed => "error",
        _ => "deferred",
    };
    state.observability.task_completed(
        task_dimension,
        task_outcome,
        Duration::from_millis(duration_ms.max(0) as u64),
    );
    let finish = ManagedTaskFinish {
        run_id,
        task_key,
        status,
        finished_at: format_utc_iso_millis(Utc::now()),
        duration_ms,
        summary,
        detail,
        completion,
        core_completion,
        migration_hold_generation,
        details,
    };
    let progress_total = finish
        .details
        .as_ref()
        .and_then(|details| {
            if is_retention_owner_v2_task(&finish.task_key) {
                details
                    .get("selectedScope")
                    .and_then(|scope| scope.get("selected"))
            } else {
                details
                    .get("globalBacklogTotal")
                    .or_else(|| details.get("total"))
            }
        })
        .and_then(Value::as_i64);
    let progress_completed = finish
        .details
        .as_ref()
        .and_then(|details| {
            details
                .get("committed")
                .or_else(|| details.get("completed"))
        })
        .and_then(Value::as_i64);
    let progress_wait_reason = finish
        .details
        .as_ref()
        .and_then(|details| details.get("waitReason"))
        .and_then(Value::as_str);
    let progress_unit = finish
        .details
        .as_ref()
        .and_then(|details| details.get("selectedScope"))
        .and_then(|scope| scope.get("unit"))
        .and_then(Value::as_str);
    let progress_source_scope = finish
        .details
        .as_ref()
        .and_then(|details| details.get("selectedScope"))
        .and_then(|scope| scope.get("scope"))
        .and_then(Value::as_str);
    let final_stages = (finish.task_key == "retention_archive").then(|| {
        let archive_status = match finish.completion.as_deref() {
            Some("complete") | Some("completed") => "complete",
            Some("no-work") => "no-work",
            Some("partial") => "partial",
            Some("deferred") => "deferred",
            _ if finish.status == SystemTaskStatus::Failed => "failed",
            _ => "complete",
        };
        vec![crate::maintenance_store::TaskStage {
            name: "archive".to_string(),
            status: archive_status.to_string(),
            completed: progress_completed,
            total: progress_total,
            elapsed_ms: finish
                .details
                .as_ref()
                .and_then(|details| details.get("elapsedMs"))
                .and_then(Value::as_i64),
            wait_reason: progress_wait_reason.map(str::to_string),
            checkpoint: Some("finished".to_string()),
        }]
    });
    let completion_phase =
        finish
            .completion
            .as_deref()
            .unwrap_or(if status == SystemTaskStatus::Failed {
                "failed"
            } else {
                "complete"
            });
    if progress_wait_reason == Some(crate::maintenance_store::MIGRATION_HOLD_REASON)
        && let Some(hold_generation) = finish.migration_hold_generation
    {
        if let Err(error) = store
            .mark_task_migration_held_at_generation(&finish.task_key, hold_generation)
            .await
        {
            warn!(
                task = %finish.task_key,
                error = %error,
                "migration-held managed task final progress snapshot could not be published"
            );
        }
    } else if progress_wait_reason == Some(crate::maintenance_store::MIGRATION_HOLD_REASON) {
        debug!(
            task = %finish.task_key,
            "skipping migration-held final progress snapshot without a current hold generation"
        );
    } else if let Err(error) = store
        .publish_progress(
            &finish.task_key,
            progress_total,
            progress_completed,
            Some(completion_phase),
            Some("finished"),
            progress_unit,
            progress_source_scope,
            progress_wait_reason.or((status == SystemTaskStatus::Failed).then_some("fatal_error")),
            None,
            final_stages.as_deref().or(initial_stages.as_deref()),
        )
        .await
    {
        warn!(task = %finish.task_key, error = %error, "managed task final progress snapshot could not be published");
    }
    finish
}

async fn wait_for_managed_task_migration_hold(
    store: &crate::maintenance_store::MaintenanceStore,
    task_key: &str,
) {
    loop {
        let held = match store.migration_task_is_held(task_key).await {
            Ok(held) => held,
            Err(error) => {
                warn!(
                    task = %task_key,
                    error = %error,
                    "managed task migration hold watcher failed"
                );
                true
            }
        };
        if held {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_legacy_task_migration_hold(task_key: &'static str) {
    loop {
        if crate::maintenance_store::migration_task_is_held(task_key).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ManagedTaskFinish {
    run_id: i64,
    task_key: String,
    status: SystemTaskStatus,
    finished_at: String,
    duration_ms: i64,
    summary: Option<String>,
    detail: Option<String>,
    completion: Option<String>,
    core_completion: Option<String>,
    migration_hold_generation: Option<u64>,
    details: Option<Value>,
}

async fn finish_managed_task_run_bounded(
    store: &crate::maintenance_store::MaintenanceStore,
    finish: &ManagedTaskFinish,
    retry: Option<&(i64, String)>,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 0..=MANAGED_TASK_FINISH_RETRY_ATTEMPTS {
        let finalization = async {
            match retry {
                Some((seconds, reason)) => {
                    store
                        .finish_run_with_observation_and_retry(
                            finish.run_id,
                            finish.status.as_str(),
                            &finish.finished_at,
                            finish.duration_ms,
                            finish.summary.as_deref(),
                            finish.detail.as_deref(),
                            finish.completion.as_deref(),
                            finish.core_completion.as_deref(),
                            finish.details.as_ref(),
                            *seconds,
                            reason,
                        )
                        .await
                }
                None => {
                    store
                        .finish_run_with_observation(
                            finish.run_id,
                            finish.status.as_str(),
                            &finish.finished_at,
                            finish.duration_ms,
                            finish.summary.as_deref(),
                            finish.detail.as_deref(),
                            finish.completion.as_deref(),
                            finish.core_completion.as_deref(),
                            finish.details.as_ref(),
                        )
                        .await
                }
            }
        };
        match tokio::time::timeout(MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT, finalization).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => last_error = Some(error),
            Err(error) => {
                last_error = Some(anyhow!(
                    "managed task finish timed out after {} ms: {error}",
                    MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT.as_millis()
                ));
            }
        }
        if attempt < MANAGED_TASK_FINISH_RETRY_ATTEMPTS {
            tokio::time::sleep(MANAGED_TASK_FINISH_RETRY_INTERVAL).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("managed task finish failed without an error")))
}

async fn drain_managed_task_finishes_on_shutdown(
    state: &AppState,
    store: &crate::maintenance_store::MaintenanceStore,
    pending_finishes: &mut VecDeque<ManagedTaskFinish>,
) {
    while let Some(finish) = pending_finishes.pop_front() {
        if let Err(error) = finish_managed_task_and_schedule_retry(store, &finish).await {
            // Replay resolves the maintenance row by run_id; task_kind is legacy journal metadata.
            let recovery = BatchedSystemTaskFinish {
                run_id: finish.run_id,
                task_key: finish.task_key.clone(),
                task_kind: SystemTaskKind::StartupBackfill,
                trigger_kind: "managed_dispatcher".to_string(),
                status: finish.status,
                summary: finish.summary.clone(),
                detail: finish.detail.clone(),
                finished_at: finish.finished_at.clone(),
                duration_ms: finish.duration_ms,
                completion: finish.completion.clone(),
                core_completion: finish.core_completion.clone(),
                details: finish.details.clone(),
            };
            let quarantined = state.sqlite_batch_writer.quarantine_system_task_finish(
                &recovery,
                "managed task finish could not be finalized before shutdown",
            );
            warn!(
                run_id = finish.run_id,
                task = %finish.task_key,
                error = %error,
                quarantined,
                "managed task dispatcher could not finalize a pending run before shutdown"
            );
        }
    }
}

fn managed_task_metric_dimension(task_key: &str) -> &'static str {
    match task_key {
        "retention_archive" => "retention_archive",
        "pool_upstream_attempt_archive" => "pool_upstream_attempt_archive",
        "forward_proxy_attempt_archive" => "forward_proxy_attempt_archive",
        "invocation_detail_prune" => "invocation_detail_prune",
        "quota_snapshot_compaction" => "quota_snapshot_compaction",
        "model_route_expiry" => "model_route_expiry",
        "task_run_history_cleanup" => "task_run_history_cleanup",
        "invocation_identity_cleanup" => "invocation_identity_cleanup",
        "raw_orphan_sweep" => "raw_orphan_sweep",
        "upstream_account_maintenance" => "upstream_account_maintenance",
        "forward_proxy_subscription_refresh" => "forward_proxy_subscription_refresh",
        "pool_orphan_recovery" => "pool_orphan_recovery",
        "startup_hourly_rollup_bootstrap" => "startup_hourly_rollup_bootstrap",
        "system_status_snapshot" => "system_status_snapshot",
        "invocation_timeline_snapshot" => "invocation_timeline_snapshot",
        "summary_snapshot" => "summary_snapshot",
        "summary_coverage_recovery" => "summary_coverage_recovery",
        "dashboard_runtime_projection_reconcile" => "dashboard_runtime_projection_reconcile",
        "long_term_projection" => "long_term_projection",
        "timeseries_minute_projection" => "timeseries_minute_projection",
        "raw_payload_metrics_inventory" => "raw_payload_metrics_inventory",
        "prompt_cache_materialization" => "prompt_cache_materialization",
        "startup_backfill" => "startup_backfill",
        "raw_compression" => "raw_compression",
        "archive_upstream_activity_manifest" => "archive_upstream_activity_manifest",
        "materialize_historical_rollups" => "materialize_historical_rollups",
        "verify_archive_storage" => "verify_archive_storage",
        "prune_archive_batches" => "prune_archive_batches",
        "prune_legacy_archive_batches" => "prune_legacy_archive_batches",
        key if key.starts_with("startup_backfill.") => "startup_backfill_child",
        _ => "unknown_managed_task",
    }
}

struct ManagedTaskExecution {
    summary: String,
    detail: Option<String>,
    completion: Option<String>,
    core_completion: Option<String>,
    details: Option<Value>,
}

fn managed_task_elapsed_ms(requested_at: &str, fallback_started_at: Instant) -> i64 {
    chrono::DateTime::parse_from_rfc3339(requested_at)
        .ok()
        .map(|started| {
            (Utc::now() - started.with_timezone(&Utc))
                .num_milliseconds()
                .max(0)
        })
        .unwrap_or_else(|| {
            fallback_started_at
                .elapsed()
                .as_millis()
                .min(i64::MAX as u128) as i64
        })
}

impl ManagedTaskExecution {
    fn simple(summary: String) -> Self {
        Self {
            summary,
            detail: None,
            completion: None,
            core_completion: None,
            details: None,
        }
    }
}

fn apply_migration_hold_to_execution(
    execution: &mut ManagedTaskExecution,
    checkpoint: Option<String>,
) {
    execution.summary = if execution.summary.is_empty() {
        "任务已完成当前有界批次，因 migration_hold 延后".to_string()
    } else {
        format!(
            "{}；当前批次完成后因 migration_hold 延后",
            execution.summary
        )
    };
    execution.detail = Some("等待迁移释放后从任务 checkpoint 继续".to_string());
    execution.completion = Some("deferred".to_string());
    execution.core_completion = None;
    let details = execution.details.get_or_insert_with(|| json!({}));
    if !details.is_object() {
        *details = json!({});
    }
    details["waitReason"] = json!(crate::maintenance_store::MIGRATION_HOLD_REASON);
    details["migrationCheckpoint"] = checkpoint.map(Value::String).unwrap_or(Value::Null);
    details["scheduleRetryAfterSecs"] = json!(crate::maintenance_store::MIGRATION_HOLD_RETRY_SECS);
    details["scheduleRetryReason"] = json!(crate::maintenance_store::MIGRATION_HOLD_REASON);
}

fn migration_hold_execution(checkpoint: Option<String>) -> ManagedTaskExecution {
    let mut execution = ManagedTaskExecution::simple(String::new());
    apply_migration_hold_to_execution(&mut execution, checkpoint);
    execution
}

async fn run_managed_task_once_with_observation(
    state: &Arc<AppState>,
    task_key: &str,
    run_id: i64,
    observation: &crate::TaskExecutionObservation,
) -> Result<ManagedTaskExecution> {
    if let Some(reason) = state
        .storage_migration
        .migration_hold_reason(task_key)
        .await
    {
        return Ok(ManagedTaskExecution {
            summary: format!("{task_key} 已因迁移准入暂停"),
            detail: Some("migration hold remains durable until migration resumes".to_string()),
            completion: Some("deferred".to_string()),
            core_completion: None,
            details: Some(json!({"waitReason": reason})),
        });
    }
    let store = crate::maintenance_store::global().context("maintenance store unavailable")?;
    let migration_hold_active = match store.migration_task_is_held(task_key).await {
        Ok(active) => active,
        Err(error) => {
            warn!(
                task = task_key,
                error = %error,
                "managed task execution could not verify migration hold"
            );
            true
        }
    };
    if migration_hold_active {
        return Ok(migration_hold_execution(
            store
                .migration_task_hold_checkpoint(task_key)
                .await
                .ok()
                .flatten(),
        ));
    }
    let (trigger, details): (String, Option<String>) =
        sqlx::query_as("SELECT trigger_kind,details FROM managed_task_runs WHERE id=?")
            .bind(run_id)
            .fetch_one(&store.pool)
            .await?;
    let details: Value = details
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or(Value::Null);
    if task_key == "prepare_performance_indexes" {
        let request: PerformanceIndexPreparationRequest = if details.is_null() {
            PerformanceIndexPreparationRequest::default()
        } else {
            serde_json::from_value(details)
                .context("invalid performance index preparation request")?
        };
        anyhow::ensure!(
            trigger == "manual",
            "index preparation requires a manual request"
        );
        let report = crate::observability::contention::with_performance(
            state.observability.clone(),
            prepare_performance_indexes(
                state.pool.connect_options().as_ref().clone(),
                request,
                crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
                state.shutdown.clone(),
            ),
        )
        .await?;
        return Ok(ManagedTaskExecution {
            summary: "性能索引准备结束".to_string(),
            detail: Some(report.outcome.to_string()),
            completion: Some(
                if report.is_complete() {
                    "complete"
                } else {
                    "deferred"
                }
                .to_string(),
            ),
            core_completion: None,
            details: Some(serde_json::to_value(report)?),
        });
    }
    let options = MaintenanceExecutionOptions {
        manual: trigger == "manual",
        admitted: true,
        dry_run: if trigger == "manual" {
            details
                .get("dryRun")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        } else {
            matches!(
                task_key,
                "retention_archive"
                    | "pool_upstream_attempt_archive"
                    | "forward_proxy_attempt_archive"
                    | "invocation_detail_prune"
                    | "quota_snapshot_compaction"
                    | "model_route_expiry"
                    | "task_run_history_cleanup"
                    | "invocation_identity_cleanup"
                    | "raw_orphan_sweep"
            ) && state.config.retention_dry_run
        },
    };
    if crate::maintenance_store::is_retention_maintenance_task(task_key) {
        sqlx::query("UPDATE managed_task_runs SET details=? WHERE id=?")
            .bind(
                json!({
                    "ownershipVersion": if matches!(task_key,
                        "retention_archive"
                            | "pool_upstream_attempt_archive"
                            | "forward_proxy_attempt_archive"
                            | "invocation_detail_prune"
                            | "quota_snapshot_compaction"
                            | "model_route_expiry"
                            | "task_run_history_cleanup"
                    ) { 2 } else { 1 },
                    "ownerScope": task_key,
                    "dryRun": options.dry_run,
                    "manual": options.manual
                })
                .to_string(),
            )
            .bind(run_id)
            .execute(&store.pool)
            .await?;
    }
    let mut execution = crate::observability::contention::with_performance(
        state.observability.clone(),
        with_maintenance_execution_options(
            options,
            crate::with_managed_task_observation(
                observation.clone(),
                Box::pin(run_managed_task_once_with_scoped_observation(
                    state,
                    task_key,
                    run_id,
                    observation,
                )),
            ),
        ),
    )
    .await?;
    if crate::maintenance_store::is_retention_maintenance_task(task_key) {
        let details = execution.details.get_or_insert_with(|| json!({}));
        if details.get("ownershipVersion").is_none() {
            details["ownershipVersion"] = json!(if matches!(
                task_key,
                "retention_archive"
                    | "pool_upstream_attempt_archive"
                    | "forward_proxy_attempt_archive"
                    | "invocation_detail_prune"
                    | "quota_snapshot_compaction"
                    | "model_route_expiry"
                    | "task_run_history_cleanup"
            ) {
                2
            } else {
                1
            });
        }
        details["ownerScope"] = json!(task_key);
        details["dryRun"] = json!(options.dry_run);
        details["manual"] = json!(options.manual);
    }
    Ok(execution)
}

async fn run_managed_task_once_with_scoped_observation(
    state: &Arc<AppState>,
    task_key: &str,
    run_id: i64,
    observation: &crate::TaskExecutionObservation,
) -> Result<ManagedTaskExecution> {
    if matches!(
        task_key,
        "retention_archive"
            | "pool_upstream_attempt_archive"
            | "forward_proxy_attempt_archive"
            | "invocation_detail_prune"
            | "quota_snapshot_compaction"
            | "model_route_expiry"
            | "task_run_history_cleanup"
            | "invocation_identity_cleanup"
            | "raw_orphan_sweep"
    ) {
        let options = maintenance_execution_options();
        let raw_traversal = managed_raw_traversal();
        let mut execution = Box::pin(execute_owned_maintenance(
            OwnedMaintenanceContext {
                pool: &state.pool,
                config: &state.config,
                storage_migration: Some(state.storage_migration.as_ref()),
                cache: &state.prompt_cache_conversation_cache,
                circuit: state.raw_capture_circuit.clone(),
                shutdown: &state.shutdown,
                raw_traversal,
            },
            task_key,
            options,
            Some(observation.clone()),
        ))
        .await?;
        if matches!(task_key, "invocation_identity_cleanup" | "raw_orphan_sweep")
            && !options.manual
            && !options.dry_run
            && let Some(store) = crate::maintenance_store::global()
        {
            if task_key == "raw_orphan_sweep"
                && (execution.completion == "deferred"
                    || execution
                        .details
                        .get("failures")
                        .and_then(Value::as_u64)
                        .is_some_and(|count| count > 0))
            {
                store
                    .record_owned_retry_delay(
                        task_key,
                        execution.next_work_secs.unwrap_or(300),
                        "raw_safety_retry",
                    )
                    .await?;
            } else if execution.completion == "deferred" {
                store.record_owned_retry(task_key, true).await?;
            } else if execution
                .details
                .get("failures")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0)
            {
                store.record_owned_retry(task_key, false).await?;
            } else {
                store.clear_owned_retry(task_key).await?;
            }
        }
        if !options.manual
            && !options.dry_run
            && let Some(retry) = execution.next_work_secs
        {
            let retry_reason = if execution.completion == "failed" {
                "owner_failed".to_string()
            } else {
                execution
                    .details
                    .get("waitReason")
                    .and_then(Value::as_str)
                    .filter(|reason| {
                        matches!(
                            *reason,
                            "sqlite_pressure"
                                | "retention_write_admission"
                                | "task_execution_active"
                        )
                    })
                    .unwrap_or("work_remaining")
                    .to_string()
            };
            execution.details["scheduleRetryAfterSecs"] = json!(retry);
            execution.details["scheduleRetryReason"] = json!(retry_reason);
        }
        return Ok(ManagedTaskExecution {
            summary: execution.summary,
            detail: None,
            completion: Some(execution.completion.to_string()),
            core_completion: execution
                .details
                .get("coreCompletion")
                .and_then(Value::as_str)
                .map(str::to_string),
            details: Some(execution.details),
        });
    }
    if task_key == "long_term_projection" {
        return Ok(
            match run_long_term_projection_once_managed(state.as_ref()).await? {
                crate::long_term_stats::LongTermProjectionFlushOutcome::Completed => {
                    ManagedTaskExecution::simple("长期统计投影刷新完成".to_string())
                }
                crate::long_term_stats::LongTermProjectionFlushOutcome::DeferredByPressure {
                    retry_at,
                } => ManagedTaskExecution {
                    summary: "长期统计投影已延后".to_string(),
                    detail: Some("写入压力下保留已提交进度".to_string()),
                    completion: Some("deferred".to_string()),
                    core_completion: None,
                    details: Some(json!({
                        "waitReason": "writer_pressure",
                        "retryAt": retry_at.map(|at| format!("{:?}", at)),
                    })),
                },
            },
        );
    }
    if task_key == "timeseries_minute_projection" {
        return Ok(
            match crate::api::flush_timeseries_minute_projection_managed(
                state.as_ref(),
                "managed_task",
                run_id,
            )
            .await
            .map_err(|error| anyhow!("分钟时序投影刷新失败: {:?}", error))?
            {
                crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed => {
                    ManagedTaskExecution::simple("分钟时序投影刷新完成".to_string())
                }
                crate::api::TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred) => {
                    ManagedTaskExecution {
                        summary: "分钟时序投影已延后".to_string(),
                        detail: Some("写入压力下保留已提交进度".to_string()),
                        completion: Some("deferred".to_string()),
                        core_completion: None,
                        details: Some(json!({
                            "waitReason": "writer_pressure",
                            "retryAfterMs": deferred.retry_after.map(|delay| delay.as_millis() as u64),
                        })),
                    }
                }
                crate::api::TimeseriesMinuteProjectionFlushOutcome::Cancelled => {
                    ManagedTaskExecution {
                        summary: "分钟时序投影已取消".to_string(),
                        detail: Some("本次执行未完成".to_string()),
                        completion: Some("deferred".to_string()),
                        core_completion: None,
                        details: Some(json!({"waitReason": "cancelled"})),
                    }
                }
            },
        );
    }
    if task_key == "summary_coverage_recovery" {
        let next_turn =
            crate::api::SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await?;
        return Ok(match next_turn {
            crate::api::SummaryCoverageRecoveryNextTurn::Deferred => ManagedTaskExecution {
                summary: "汇总覆盖恢复已延后".to_string(),
                detail: Some("数据库压力门控暂未允许恢复".to_string()),
                completion: Some("deferred".to_string()),
                core_completion: None,
                details: Some(json!({"waitReason": "writer_pressure"})),
            },
            _ => ManagedTaskExecution::simple("汇总覆盖恢复完成".to_string()),
        });
    }
    if matches!(
        task_key,
        "startup_backfill" | "prompt_cache_materialization"
    ) || task_key.starts_with("startup_backfill.")
    {
        let selected_tasks = if task_key == "prompt_cache_materialization" {
            Some(vec![
                crate::StartupBackfillTask::PromptCacheConversationsMaterialization,
            ])
        } else if let Some(name) = task_key.strip_prefix("startup_backfill.") {
            Some(vec![managed_startup_backfill_task(name)?])
        } else {
            None
        };
        let pass = crate::run_startup_backfill_maintenance_pass_managed(
            state.clone(),
            &state.shutdown,
            selected_tasks.as_deref(),
            (task_key == "prompt_cache_materialization").then_some("prompt_cache_materialization"),
            Some(run_id),
        )
        .await;
        if pass.had_failure {
            bail!(
                pass.detail
                    .unwrap_or_else(|| format!("{task_key} 执行失败"))
            );
        }
        return Ok(if pass.deferred {
            ManagedTaskExecution {
                summary: format!("{task_key} 已延后"),
                detail: pass.detail,
                completion: Some("deferred".to_string()),
                core_completion: None,
                details: Some(json!({"waitReason": "background_busy"})),
            }
        } else if task_key == "startup_backfill" && !pass.ran_actionable_task {
            ManagedTaskExecution::simple("启动回填没有可处理项".to_string())
        } else {
            ManagedTaskExecution::simple(format!("{task_key} 处理完成"))
        });
    }
    if task_key == "startup_hourly_rollup_bootstrap" {
        crate::maintenance::bootstrap_hourly_rollups_for_runtime_startup_with_work(
            &state.pool,
            Some(state.config.invocation_max_days),
        )
        .await?;
        if crate::maintenance::usage_breakdown_repair_is_pending(&state.pool).await? {
            return Ok(ManagedTaskExecution {
                summary: "启动时小时汇总补齐已延后".to_string(),
                detail: Some("保留归档 usage breakdown 修复游标，等待下一次运行继续".to_string()),
                completion: Some("deferred".to_string()),
                core_completion: None,
                details: Some(json!({
                    "waitReason": "archive_usage_breakdown_repair",
                })),
            });
        }
        return Ok(ManagedTaskExecution::simple(
            "启动时小时汇总补齐完成".to_string(),
        ));
    }
    match task_key {
        "pool_orphan_recovery" => {
            let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref()).await?;
            observation
                .set_processed_work(i64::try_from(outcome.recovered_attempts).unwrap_or(i64::MAX));
            return Ok(ManagedTaskExecution::simple(format!(
                "恢复连接池尝试 {} 条，调用 {} 条",
                outcome.recovered_attempts, outcome.recovered_invocations
            )));
        }
        "raw_compression" => {
            let summary = compress_cold_proxy_raw_payloads(
                &state.pool,
                &state.config,
                state.config.database_path.parent(),
                false,
            )
            .await?;
            observation
                .set_processed_work(i64::try_from(summary.files_compressed).unwrap_or(i64::MAX));
            return Ok(ManagedTaskExecution::simple(format!(
                "原始载荷压缩完成：{summary:?}"
            )));
        }
        "archive_upstream_activity_manifest" => {
            let summary =
                refresh_archive_upstream_activity_manifest(&state.pool, &state.config, false)
                    .await?;
            observation
                .set_processed_work(i64::try_from(summary.refreshed_batches).unwrap_or(i64::MAX));
            return Ok(ManagedTaskExecution::simple(format!(
                "上游活动归档清单完成：{summary:?}"
            )));
        }
        "materialize_historical_rollups" => {
            let summary = materialize_historical_rollups(&state.pool, &state.config, false).await?;
            observation.set_processed_work(
                i64::try_from(summary.materialized_archive_batches).unwrap_or(i64::MAX),
            );
            return Ok(ManagedTaskExecution::simple(format!(
                "历史汇总物化完成：{summary:?}"
            )));
        }
        "verify_archive_storage" => {
            let summary = verify_archive_storage(&state.pool, &state.config).await?;
            observation
                .set_processed_work(i64::try_from(summary.manifest_rows).unwrap_or(i64::MAX));
            return Ok(ManagedTaskExecution::simple(format!(
                "归档存储校验完成：{summary:?}"
            )));
        }
        "prune_archive_batches" => {
            let summary = prune_archive_batches(&state.pool, &state.config, false).await?;
            let deleted = summary
                .expired_archive_batches_deleted
                .saturating_add(summary.legacy_archive_batches_deleted);
            observation.set_processed_work(i64::try_from(deleted).unwrap_or(i64::MAX));
            return Ok(ManagedTaskExecution::simple(format!(
                "归档批次清理完成：{summary:?}"
            )));
        }
        "prune_legacy_archive_batches" => {
            let summary = prune_legacy_archive_batches(&state.pool, &state.config, false).await?;
            observation.set_processed_work(
                i64::try_from(summary.deleted_archive_batches).unwrap_or(i64::MAX),
            );
            return Ok(ManagedTaskExecution::simple(format!(
                "旧归档批次清理完成：{summary:?}"
            )));
        }
        _ => {}
    }
    run_managed_task_once(state, task_key, run_id)
        .await
        .map(ManagedTaskExecution::simple)
}

async fn run_managed_task_once(
    state: &Arc<AppState>,
    task_key: &str,
    run_id: i64,
) -> Result<String> {
    match task_key {
        "retention_archive"
        | "pool_upstream_attempt_archive"
        | "forward_proxy_attempt_archive"
        | "invocation_detail_prune"
        | "quota_snapshot_compaction"
        | "model_route_expiry"
        | "task_run_history_cleanup" => {
            let summary = run_retention_owner_task(
                &state.pool,
                &state.config,
                task_key,
                false,
                &state.shutdown,
                Some(state.storage_migration.as_ref()),
            )
            .await?;
            Ok(format!(
                "{} 完成：提交 {} {}",
                task_key, summary.committed, summary.unit
            ))
        }
        "forward_proxy_subscription_refresh" => {
            refresh_forward_proxy_subscriptions(state.clone(), false, None).await?;
            Ok("正向代理订阅刷新完成".to_string())
        }
        "summary_snapshot" => {
            let journal_before = state
                .subscription_hub
                .summary_delta_journal_unabsorbed_deltas()
                .await;
            crate::api::refresh_summary_snapshots(state.as_ref()).await?;
            let contribution_count = i64::try_from(
                state
                    .subscription_hub
                    .summary_delta_journal_absorbed_deltas_count(&journal_before)
                    .await,
            )
            .unwrap_or(i64::MAX);
            crate::record_managed_task_discovered_work(&["summary_snapshot"], contribution_count);
            crate::record_managed_task_processed_work(&["summary_snapshot"], contribution_count);
            Ok("汇总快照刷新完成".to_string())
        }
        "summary_coverage_recovery" => {
            crate::api::SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await?;
            Ok("汇总覆盖恢复完成".to_string())
        }
        "pool_orphan_recovery" => {
            let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref()).await?;
            Ok(format!(
                "恢复连接池尝试 {} 条，调用 {} 条",
                outcome.recovered_attempts, outcome.recovered_invocations
            ))
        }
        "upstream_account_maintenance" => {
            run_upstream_account_maintenance_once(state.clone()).await?;
            Ok("上游账号维护完成".to_string())
        }
        "system_status_snapshot" => {
            hydrate_system_status_snapshot(state.as_ref()).await?;
            Ok("系统状态快照刷新完成".to_string())
        }
        "invocation_timeline_snapshot" => {
            cleanup_timeline_snapshot_rows_once(&state.pool)
                .await
                .map_err(|_| anyhow!("调用时间线快照清理失败"))?;
            Ok("调用时间线快照清理完成".to_string())
        }
        "dashboard_runtime_projection_reconcile" => {
            let result = reconcile_dashboard_runtime_projection_once(state.as_ref())
                .await
                .map_err(|_| anyhow!("仪表盘运行投影校对失败"))?;
            let reconciled_records = i64::from(result.changed);
            crate::record_managed_task_discovered_work(
                &["dashboard_runtime_projection_reconcile"],
                reconciled_records,
            );
            crate::record_managed_task_processed_work(
                &["dashboard_runtime_projection_reconcile"],
                reconciled_records,
            );
            Ok("仪表盘运行投影校对完成".to_string())
        }
        "timeseries_minute_projection" => {
            crate::api::flush_timeseries_minute_projection_managed(
                state.as_ref(),
                "managed_task",
                run_id,
            )
            .await
            .map_err(|_| anyhow!("分钟时序投影刷新失败"))?;
            Ok("分钟时序投影刷新完成".to_string())
        }
        "raw_payload_metrics_inventory" => {
            let reset =
                resume_retention_raw_payload_metrics_inventory_reset(state.as_ref()).await?;
            if crate::maintenance::migration_hold_boundary_reached().await {
                return Ok("原始载荷指标盘点已在 reset 批次后因 migration_hold 延后".to_string());
            }
            refresh_system_raw_payload_metrics_inventory(state.as_ref()).await?;
            Ok(if reset {
                "原始载荷指标盘点已推进".to_string()
            } else {
                "原始载荷指标盘点无需处理".to_string()
            })
        }
        "prompt_cache_materialization" => {
            let task = crate::StartupBackfillTask::PromptCacheConversationsMaterialization;
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                Some(&[task]),
                Some("prompt_cache_materialization"),
                Some(run_id),
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| "Prompt 缓存物化失败".to_string())
                );
            }
            Ok("Prompt 缓存物化完成".to_string())
        }
        "startup_hourly_rollup_bootstrap" => {
            bootstrap_hourly_rollups_for_runtime_startup_with_work(
                &state.pool,
                Some(state.config.invocation_max_days),
            )
            .await?;
            Ok("启动时小时汇总补齐完成".to_string())
        }
        "raw_compression" => {
            let summary = compress_cold_proxy_raw_payloads(
                &state.pool,
                &state.config,
                state.config.database_path.parent(),
                false,
            )
            .await?;
            Ok(format!("原始载荷压缩完成：{summary:?}"))
        }
        "archive_upstream_activity_manifest" => {
            let summary =
                refresh_archive_upstream_activity_manifest(&state.pool, &state.config, false)
                    .await?;
            Ok(format!("上游活动归档清单完成：{summary:?}"))
        }
        "materialize_historical_rollups" => {
            let summary = materialize_historical_rollups(&state.pool, &state.config, false).await?;
            Ok(format!("历史汇总物化完成：{summary:?}"))
        }
        "verify_archive_storage" => {
            let summary = verify_archive_storage(&state.pool, &state.config).await?;
            Ok(format!("归档存储校验完成：{summary:?}"))
        }
        "prune_archive_batches" => {
            let summary = prune_archive_batches(&state.pool, &state.config, false).await?;
            Ok(format!("归档批次清理完成：{summary:?}"))
        }
        "prune_legacy_archive_batches" => {
            let summary = prune_legacy_archive_batches(&state.pool, &state.config, false).await?;
            Ok(format!("旧归档批次清理完成：{summary:?}"))
        }
        "startup_backfill" => {
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                None,
                None,
                Some(run_id),
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| "启动回填存在失败".to_string())
                );
            }
            Ok(if pass.ran_actionable_task {
                "启动回填处理完成".to_string()
            } else {
                "启动回填没有可处理项".to_string()
            })
        }
        key if key.starts_with("startup_backfill.") => {
            let name = key.trim_start_matches("startup_backfill.");
            let task = managed_startup_backfill_task(name)?;
            let catalog = state.pricing_catalog.read().await.clone();
            crate::wake_startup_backfill_tasks_with_pricing_catalog(
                &state.pool,
                &[task],
                Some(&catalog),
                "manual_run",
            )
            .await?;
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                Some(&[task]),
                None,
                Some(run_id),
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| format!("启动回填子任务失败: {name}"))
                );
            }
            Ok(format!("启动回填子任务 {name} 处理完成"))
        }
        _ => bail!("任务暂不支持立即运行: {task_key}"),
    }
}

fn managed_startup_backfill_task(name: &str) -> Result<crate::StartupBackfillTask> {
    crate::StartupBackfillTask::from_managed_key(name)
        .ok_or_else(|| anyhow!("未知启动回填子任务: {name}"))
}

pub(crate) fn begin_runtime_shutdown(cancel: &CancellationToken) {
    if !cancel.is_cancelled() {
        info!("shutdown signal received; beginning graceful shutdown");
        cancel.cancel();
    }
}

pub(crate) async fn drain_runtime_after_shutdown(
    state: Arc<AppState>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
    startup_hot_read_hydration_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    if let Some(storage_runtime) = state.storage_runtime.as_ref() {
        storage_runtime.stop().await;
    }
    if let Some(server_handle) = server_handle {
        info!("http server graceful drain started");
        if let Err(err) = server_handle.await {
            error!(?err, "http server terminated unexpectedly");
        }
        info!("http server graceful drain finished");
    }

    if let Some(poller_handle) = poller_handle {
        if let Err(err) = poller_handle.await {
            error!(?err, "poller task terminated unexpectedly");
        }
        info!("scheduler drained");
    }
    if let Some(upstream_accounts_handle) = upstream_accounts_handle
        && let Err(err) = upstream_accounts_handle.await
    {
        error!(
            ?err,
            "upstream account maintenance task terminated unexpectedly"
        );
    }
    state.upstream_accounts.drain_background_tasks().await;
    if let Some(forward_proxy_handle) = forward_proxy_handle
        && let Err(err) = forward_proxy_handle.await
    {
        error!(
            ?err,
            "forward proxy maintenance task terminated unexpectedly"
        );
    }
    if let Some(pool_orphan_recovery_handle) = pool_orphan_recovery_handle
        && let Err(err) = pool_orphan_recovery_handle.await
    {
        error!(
            ?err,
            "pool orphan recovery maintenance task terminated unexpectedly"
        );
    }
    if let Some(retention_handle) = retention_handle
        && let Err(err) = retention_handle.await
    {
        error!(?err, "retention maintenance task terminated unexpectedly");
    }
    if let Some(startup_backfill_handle) = startup_backfill_handle
        && let Err(err) = startup_backfill_handle.await
    {
        error!(
            ?err,
            "startup backfill maintenance task terminated unexpectedly"
        );
    }
    if let Some(startup_hot_read_hydration_handle) = startup_hot_read_hydration_handle
        && let Err(err) = startup_hot_read_hydration_handle.await
    {
        error!(
            ?err,
            "startup hot-read hydration coordinator terminated unexpectedly"
        );
    }

    let dashboard_runtime_projection_handle =
        crate::api::take_dashboard_runtime_projection_handle();
    if let Some(handle) = dashboard_runtime_projection_handle
        && let Err(err) = handle.await
    {
        error!(
            ?err,
            "dashboard runtime projection worker terminated unexpectedly"
        );
    }

    let runtime_shutdown_summary = state.proxy_runtime_invocations.shutdown_summary();
    if runtime_shutdown_summary.running_count > 0 {
        warn!(
            running_snapshot_shutdown_skipped_count = runtime_shutdown_summary.running_count,
            oldest_age_ms = runtime_shutdown_summary.oldest_age_ms,
            "skipping P2 memory running snapshots during graceful shutdown"
        );
    }
    state.sqlite_batch_writer.shutdown_and_drain().await;
    if let Some(route_facts) = state.upstream_accounts.route_facts.lock().await.clone() {
        route_facts.shutdown_and_sync().await;
    }

    let broadcast_handles = {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        std::mem::take(&mut *guard)
    };
    if !broadcast_handles.is_empty() {
        for broadcast_handle in broadcast_handles {
            if let Err(err) = broadcast_handle.await {
                error!(
                    ?err,
                    "summary/quota broadcast worker terminated unexpectedly"
                );
            }
        }
        info!("summary/quota broadcast worker drained");
    }

    state.xray_supervisor.lock().await.shutdown_all().await;
    crate::task_timeline::drain_after_shutdown().await;
    let traces = state.observability.trace_runtime();
    let _ = tokio::task::spawn_blocking(move || traces.shutdown()).await;
    info!("shutdown complete");

    Ok(())
}

pub(crate) fn init_tracing() {
    use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
    let log = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        );
    let sql = ObservabilityConfig::from_env()
        .ok()
        .filter(|config| config.enabled)
        .map(|_| hotpath::sqlx_tracing_layer());
    tracing_subscriber::registry().with(sql).with(log).init();
}

pub(crate) fn log_startup_phase(phase: &'static str, started_at: Instant) {
    info!(
        phase,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "startup phase finished"
    );
}

pub(crate) fn spawn_runtime_startup_hourly_rollup_bootstrap(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        if crate::maintenance_store::legacy_worker_should_skip("startup_hourly_rollup_bootstrap")
            .await
        {
            return;
        }
        let started_at = Instant::now();
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
        // Every task row created by this bootstrap attempt is timestamped after this point.
        // Using the exact lower bound avoids losing a row when SQLite admission takes longer
        // than the old one-second grace window during shutdown.
        let task_start_window = format_utc_iso_millis(Utc::now());
        let initial_p2_preemption_retry =
            Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS);
        let mut p2_preemption_retry = initial_p2_preemption_retry;
        loop {
            if state
                .storage_migration
                .migration_hold_reason("startup_hourly_rollup_bootstrap")
                .await
                .is_some()
            {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                }
            }
            if crate::maintenance_store::legacy_worker_should_skip(
                "startup_hourly_rollup_bootstrap",
            )
            .await
            {
                return;
            }
            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "startup_hourly_rollup_bootstrap",
            ) else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                }
            };
            // Task history is admitted and recorded before waiting for the synchronization lock,
            // but both permits are released immediately so a lock wait cannot occupy the only
            // background pressure slot.
            let mut task_run = loop {
                let pressure_permit = match pressure_gate
                    .try_begin_background("startup_hourly_rollup_bootstrap_task_history")
                {
                    Ok(permit) => permit,
                    Err(reason) => {
                        let retry_after = match reason {
                            crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                                remaining_ms,
                            } => Duration::from_millis(remaining_ms.max(1)),
                            crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                                Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)
                            }
                        };
                        info!(
                            defer_reason = %reason,
                            retry_after_ms = retry_after.as_millis() as u64,
                            "background startup hourly rollup bootstrap task history deferred"
                        );
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(retry_after) => continue,
                        }
                    }
                };
                let Some(write_permit) = coordinator.try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                ) else {
                    drop(pressure_permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(
                            STARTUP_HOURLY_ROLLUP_TASK_HISTORY_COORDINATOR_RETRY_INTERVAL,
                        ) => continue,
                    }
                };
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        drop(write_permit);
                        drop(pressure_permit);
                        finish_orphaned_startup_hourly_rollup_bootstrap_task(
                            state.as_ref(),
                            &cancel,
                            &task_start_window,
                        ).await;
                        return;
                    }
                    result = begin_runtime_startup_hourly_rollup_task(
                        state.as_ref(),
                        SystemTaskKind::HourlyRollupBootstrap,
                        "startup",
                        Some("background hourly rollup bootstrap started".to_string()),
                    ) => result,
                };
                drop(write_permit);
                drop(pressure_permit);
                match result {
                    Ok(task_run) => break task_run,
                    Err(err) => {
                        warn!(
                            error = %err,
                            "failed to record background startup hourly rollup bootstrap start; retrying"
                        );
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(Duration::from_secs(
                                BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                            )) => {}
                        }
                    }
                }
            };
            // The task-history admission is released before waiting for this lock. The actual
            // rollup work reacquires both gates only after the lock is owned.
            let rollup_guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    info!(
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "background startup hourly rollup bootstrap cancelled before acquiring its synchronization lock"
                    );
                    finish_runtime_startup_hourly_rollup_bootstrap_task(
                        state.as_ref(),
                        &cancel,
                        Some(&task_run),
                        SystemTaskStatus::Skipped,
                        "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                        None,
                    ).await;
                    return;
                }
                guard = state.hourly_rollup_sync_lock.lock() => guard,
            };
            let (pressure_permit, write_permit) = loop {
                let pressure_permit = match pressure_gate
                    .try_begin_background("startup_hourly_rollup_bootstrap")
                {
                    Ok(permit) => permit,
                    Err(reason) => {
                        let retry_after = match reason {
                            crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                                remaining_ms,
                            } => Duration::from_millis(remaining_ms.max(1)),
                            crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                                Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)
                            }
                        };
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => {
                                drop(rollup_guard);
                                finish_runtime_startup_hourly_rollup_bootstrap_task(
                                    state.as_ref(),
                                    &cancel,
                                    Some(&task_run),
                                    SystemTaskStatus::Skipped,
                                    "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                    None,
                                ).await;
                                return;
                            }
                            _ = tokio::time::sleep(retry_after) => continue,
                        }
                    }
                };
                let Some(write_permit) = coordinator.try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                ) else {
                    drop(pressure_permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            drop(rollup_guard);
                            finish_runtime_startup_hourly_rollup_bootstrap_task(
                                state.as_ref(),
                                &cancel,
                                Some(&task_run),
                                SystemTaskStatus::Skipped,
                                "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                None,
                            ).await;
                            return;
                        }
                        _ = tokio::time::sleep(Duration::from_secs(
                            BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                        )) => continue,
                    }
                };
                break (pressure_permit, write_permit);
            };

            task_run.observation =
                Some(crate::TaskExecutionObservation::begin_for_system_task_run(
                    SystemTaskKind::HourlyRollupBootstrap.as_str(),
                    "startup",
                    "processing",
                    Some(task_run.id),
                ));

            let hourly_rollups_started_at = Instant::now();
            let migration_hold_flag = Arc::new(AtomicBool::new(false));
            let hourly_rollups = crate::with_managed_task_observation(
                task_run
                    .observation
                    .clone()
                    .expect("hourly rollup observation is initialized before work"),
                async {
                    let mut hourly_rollups_future =
                        Box::pin(crate::maintenance::with_migration_hold_signal(
                            Arc::clone(&migration_hold_flag),
                            crate::maintenance::with_managed_task_key(
                                "startup_hourly_rollup_bootstrap",
                                bootstrap_hourly_rollups_for_runtime_startup_with_work(
                                    &state.pool,
                                    Some(state.config.invocation_max_days),
                                ),
                            ),
                        ));
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => None,
                        _ = coordinator.wait_for_p2_preemption() => None,
                        result = &mut hourly_rollups_future => Some(result),
                        _ = wait_for_legacy_task_migration_hold(
                            "startup_hourly_rollup_bootstrap",
                        ) => {
                            migration_hold_flag.store(true, Ordering::Release);
                            Some((&mut hourly_rollups_future).await)
                        }
                    }
                },
            )
            .await;
            let Some(hourly_rollups) = hourly_rollups else {
                drop(write_permit);
                drop(pressure_permit);
                drop(rollup_guard);
                info!(
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap cancelled during hourly rollup repair"
                );
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled during hourly rollup repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            };
            let migration_hold_active = migration_hold_flag.load(Ordering::Acquire)
                || crate::maintenance_store::migration_task_is_held(
                    "startup_hourly_rollup_bootstrap",
                )
                .await;
            if migration_hold_active {
                drop(write_permit);
                drop(pressure_permit);
                drop(rollup_guard);
                finish_runtime_startup_hourly_rollup_migration_hold(
                    state.as_ref(),
                    &cancel,
                    &task_run,
                )
                .await;
                return;
            }
            if let Err(err) = hourly_rollups.as_ref() {
                drop(write_permit);
                drop(pressure_permit);
                drop(rollup_guard);
                pressure_gate.record_error("startup_hourly_rollup_bootstrap", err);
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Failed,
                    "background hourly rollup bootstrap failed; existing rollups remain available",
                    Some(err.to_string()),
                )
                .await;
                warn!(
                    error = %err,
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap failed; keeping existing rollups"
                );
                return;
            }
            drop(write_permit);
            drop(pressure_permit);
            drop(rollup_guard);
            let usage_breakdown_repair_pending = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                _ = coordinator.wait_for_p2_preemption() => None,
                result = crate::maintenance::usage_breakdown_repair_is_pending(&state.pool) => Some(result),
            };
            let Some(usage_breakdown_repair_pending) = usage_breakdown_repair_pending else {
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled while inspecting pending archive usage breakdown repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            };
            let usage_breakdown_repair_pending = match usage_breakdown_repair_pending {
                Ok(pending) => pending,
                Err(err) => {
                    pressure_gate.record_error("startup_hourly_rollup_bootstrap", &err);
                    finish_runtime_startup_hourly_rollup_bootstrap_task(
                        state.as_ref(),
                        &cancel,
                        Some(&task_run),
                        SystemTaskStatus::Failed,
                        "background hourly rollup bootstrap could not inspect pending repair state",
                        Some(err.to_string()),
                    )
                    .await;
                    warn!(
                        error = %err,
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "background startup hourly rollup bootstrap could not inspect pending repair state"
                    );
                    return;
                }
            };
            if crate::maintenance_store::migration_task_is_held("startup_hourly_rollup_bootstrap")
                .await
            {
                finish_runtime_startup_hourly_rollup_migration_hold(
                    state.as_ref(),
                    &cancel,
                    &task_run,
                )
                .await;
                return;
            }
            if usage_breakdown_repair_pending {
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap deferred with pending archive usage breakdown repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            }
            let rollup_guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    finish_runtime_startup_hourly_rollup_bootstrap_task(
                        state.as_ref(),
                        &cancel,
                        Some(&task_run),
                        SystemTaskStatus::Skipped,
                        "background hourly rollup bootstrap cancelled before reacquiring its synchronization lock",
                        None,
                    ).await;
                    return;
                }
                guard = state.hourly_rollup_sync_lock.lock() => guard,
            };
            let (pressure_permit, write_permit) = loop {
                let pressure_permit = match pressure_gate
                    .try_begin_background("startup_hourly_rollup_bootstrap")
                {
                    Ok(permit) => permit,
                    Err(reason) => {
                        let retry_after = match reason {
                            crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                                remaining_ms,
                            } => Duration::from_millis(remaining_ms.max(1)),
                            crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                                Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)
                            }
                        };
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => {
                                drop(rollup_guard);
                                finish_runtime_startup_hourly_rollup_bootstrap_task(
                                    state.as_ref(),
                                    &cancel,
                                    Some(&task_run),
                                    SystemTaskStatus::Skipped,
                                    "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                    None,
                                ).await;
                                return;
                            }
                            _ = tokio::time::sleep(retry_after) => continue,
                        }
                    }
                };
                let Some(write_permit) = coordinator.try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                ) else {
                    drop(pressure_permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            drop(rollup_guard);
                            finish_runtime_startup_hourly_rollup_bootstrap_task(
                                state.as_ref(),
                                &cancel,
                                Some(&task_run),
                                SystemTaskStatus::Skipped,
                                "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                None,
                            ).await;
                            return;
                        }
                        _ = tokio::time::sleep(Duration::from_secs(
                            BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                        )) => continue,
                    }
                };
                break (pressure_permit, write_permit);
            };
            if let Ok(work_count) = hourly_rollups.as_ref()
                && let Some(observation) = task_run.observation.as_ref()
            {
                let work_count = i64::try_from(*work_count).unwrap_or(i64::MAX);
                observation.set_discovered_work(
                    work_count,
                    format_utc_iso_millis(Utc::now()),
                    "run-window".to_string(),
                );
                observation.set_processed_work(work_count);
            }
            let hourly_rollups_elapsed_ms = hourly_rollups_started_at.elapsed().as_millis() as u64;
            info!(
                elapsed_ms = hourly_rollups_elapsed_ms,
                "background startup hourly rollup bootstrap completed hourly rollup repair"
            );

            let summary_rollups_started_at = Instant::now();
            let summary_rollups = {
                let mut summary_rollups_future =
                    Box::pin(crate::maintenance::with_migration_hold_signal(
                        Arc::clone(&migration_hold_flag),
                        crate::maintenance::with_managed_task_key(
                            "startup_hourly_rollup_bootstrap",
                            ensure_invocation_summary_rollups_ready_best_effort(&state.pool),
                        ),
                    ));
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    _ = coordinator.wait_for_p2_preemption() => None,
                    result = &mut summary_rollups_future => Some(result),
                    _ = wait_for_legacy_task_migration_hold(
                        "startup_hourly_rollup_bootstrap",
                    ) => {
                        migration_hold_flag.store(true, Ordering::Release);
                        Some((&mut summary_rollups_future).await)
                    }
                }
            };
            drop(rollup_guard);
            let migration_hold_active = migration_hold_flag.load(Ordering::Acquire)
                || crate::maintenance_store::migration_task_is_held(
                    "startup_hourly_rollup_bootstrap",
                )
                .await;
            if migration_hold_active {
                drop(write_permit);
                drop(pressure_permit);
                finish_runtime_startup_hourly_rollup_migration_hold(
                    state.as_ref(),
                    &cancel,
                    &task_run,
                )
                .await;
                return;
            }
            let Some(summary_rollups) = summary_rollups else {
                drop(write_permit);
                drop(pressure_permit);
                info!(
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap cancelled during summary rollup repair"
                );
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled during summary rollup repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            };
            if let Err(err) = summary_rollups {
                drop(write_permit);
                drop(pressure_permit);
                pressure_gate.record_error("startup_hourly_rollup_bootstrap", &err);
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Failed,
                    "background hourly rollup bootstrap failed; existing rollups remain available",
                    Some(err.to_string()),
                )
                .await;
                warn!(
                    error = %err,
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap failed; keeping existing rollups"
                );
                return;
            }
            drop(write_permit);
            drop(pressure_permit);
            let summary_rollups_elapsed_ms =
                summary_rollups_started_at.elapsed().as_millis() as u64;
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            finish_runtime_startup_hourly_rollup_bootstrap_task(
            state.as_ref(),
            &cancel,
            Some(&task_run),
            SystemTaskStatus::Success,
            &format!(
                "background hourly rollup bootstrap completed: hourly_rollups_ms={hourly_rollups_elapsed_ms} summary_rollups_ms={summary_rollups_elapsed_ms}"
            ),
            None,
        )
        .await;
            info!(
                elapsed_ms,
                hourly_rollups_elapsed_ms,
                summary_rollups_elapsed_ms,
                "background startup hourly rollup bootstrap completed"
            );
            break;
        }
    })
}

async fn begin_runtime_startup_hourly_rollup_task(
    state: &AppState,
    task_kind: SystemTaskKind,
    trigger_kind: &'static str,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    #[cfg(test)]
    {
        crate::api::begin_system_task_run(&state.pool, task_kind, trigger_kind, summary).await
    }
    #[cfg(not(test))]
    {
        crate::api::begin_system_task_run_nonblocking(
            &state.config.database_url(),
            task_kind,
            trigger_kind,
            summary,
        )
        .await
    }
}

pub(crate) async fn finish_orphaned_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    started_at_from: &str,
) {
    #[cfg(not(test))]
    let Some(store) = crate::maintenance_store::global() else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        #[cfg(test)]
        let task_query = sqlx::query_as::<_, (i64, String)>(
            r#"
            SELECT id, trigger_kind
            FROM system_task_runs
            WHERE task_kind = ?1
              AND trigger_kind = 'startup'
              AND status = ?2
              AND summary = 'background hourly rollup bootstrap started'
              AND started_at >= ?3
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind("hourly_rollup_bootstrap")
        .bind(SystemTaskStatus::Running.as_str())
        .bind(started_at_from)
        .fetch_optional(&state.pool);
        #[cfg(not(test))]
        let task_query = sqlx::query_as::<_, (i64, String)>(
            r#"
                SELECT id, trigger_kind
                FROM managed_task_runs
                WHERE task_key = ?1
                  AND trigger_kind = 'startup'
                  AND status = ?2
                  AND summary = 'background hourly rollup bootstrap started'
                  AND started_at >= ?3
                ORDER BY id DESC
                LIMIT 1
                "#,
        )
        .bind(SystemTaskKind::HourlyRollupBootstrap.as_str())
        .bind(SystemTaskStatus::Running.as_str())
        .bind(started_at_from)
        .fetch_optional(&store.pool);
        let task = tokio::time::timeout(Duration::from_millis(50), task_query).await;
        match task {
            Ok(Ok(Some((id, trigger_kind)))) => {
                let task_run = SystemTaskRunHandle {
                    id,
                    task_kind: SystemTaskKind::HourlyRollupBootstrap,
                    trigger_kind,
                    started_at: Instant::now(),
                    observation: None,
                    observed_terminal: Arc::new(AtomicBool::new(false)),
                };
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state,
                    cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                    None,
                )
                .await;
                return;
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(err)) => {
                debug!(error = %err, "failed to inspect for an orphaned startup hourly rollup bootstrap task");
                return;
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(crate) async fn finish_runtime_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: Option<&SystemTaskRunHandle>,
    status: SystemTaskStatus,
    summary: &str,
    detail: Option<String>,
) {
    if let Some(task_run) = task_run {
        let finished = finish_system_task_run_reliably(
            state,
            Some(cancel),
            task_run,
            status,
            Some(summary.to_string()),
            detail,
        )
        .await;
        if !finished {
            warn!(
                task_kind = task_run.task_kind.as_str(),
                trigger_kind = %task_run.trigger_kind,
                timeout_ms = STARTUP_HOURLY_ROLLUP_BOOTSTRAP_CANCELLED_TASK_FINISH_TIMEOUT.as_millis() as u64,
                "failed to durably finalize startup hourly rollup bootstrap task-history after bounded retries"
            );
        }
    }
}

async fn finish_runtime_startup_hourly_rollup_migration_hold(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
) {
    if let Some(store) = crate::maintenance_store::global()
        && let Err(error) = store
            .mark_task_migration_held("startup_hourly_rollup_bootstrap")
            .await
    {
        warn!(
            error = %error,
            "startup hourly rollup migration-held progress could not be published"
        );
    }
    finish_runtime_startup_hourly_rollup_bootstrap_task(
        state,
        cancel,
        Some(task_run),
        SystemTaskStatus::Skipped,
        "background startup hourly rollup bootstrap deferred by migration hold",
        Some(crate::maintenance_store::MIGRATION_HOLD_REASON.to_string()),
    )
    .await;
}

pub(crate) fn spawn_forward_proxy_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let startup_known_subscription_keys = {
            let manager = state.forward_proxy.lock().await;
            snapshot_known_subscription_proxy_keys(&manager)
        };
        if cancel.is_cancelled() {
            info!("forward proxy maintenance skipped because shutdown is already in progress");
            return;
        }
        if crate::maintenance_store::legacy_worker_should_skip("forward_proxy_subscription_refresh")
            .await
        {
            info!("forward proxy legacy worker skipped by managed task control");
            return;
        }
        {
            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "forward_proxy_subscription_refresh",
            ) else {
                return;
            };
            let startup_run = tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                result = begin_system_task_run_admitted(
                    state.as_ref(),
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                    SystemTaskKind::ForwardProxySubscriptionRefresh,
                    "startup",
                    Some("forward proxy subscription refresh started".to_string()),
                    ) => match result {
                    Ok(run) => Some(run),
                    Err(error) => {
                        warn!(%error, "failed to record forward proxy startup refresh");
                        None
                    }
                },
            };
            let observation = crate::TaskExecutionObservation::begin_for_system_task_run(
                "forward_proxy_subscription_refresh",
                "startup",
                "processing",
                startup_run.as_ref().map(|run| run.id),
            );
            let refresh_result = crate::with_managed_task_observation(
                observation.clone(),
                refresh_forward_proxy_subscriptions(
                    state.clone(),
                    true,
                    Some(startup_known_subscription_keys),
                ),
            )
            .await;
            observation.finish_from_result(&refresh_result);
            if let Err(err) = refresh_result {
                let detail = err.to_string();
                if let Some(run) = startup_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(&cancel),
                        run,
                        SystemTaskStatus::Failed,
                        Some("forward proxy startup refresh failed".to_string()),
                        Some(detail),
                    )
                    .await;
                }
                warn!(error = %err, "failed to refresh forward proxy subscriptions at startup");
            } else {
                if let Some(run) = startup_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(&cancel),
                        run,
                        SystemTaskStatus::Success,
                        Some("forward proxy startup refresh completed".to_string()),
                        None,
                    )
                    .await;
                }
            }
        }

        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("forward proxy maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if crate::maintenance_store::legacy_worker_should_skip(
                        "forward_proxy_subscription_refresh",
                    )
                    .await
                    {
                        continue;
                    }
                    let Some(_execution_lease) =
                        crate::maintenance_store::try_acquire_task_execution(
                            "forward_proxy_subscription_refresh",
                        )
                    else {
                        continue;
                    };
                    let task_run = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        result = begin_system_task_run_admitted(
                            state.as_ref(),
                            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                            SystemTaskKind::ForwardProxySubscriptionRefresh,
                            "interval",
                            Some("forward proxy interval refresh started".to_string()),
                        ) => match result {
                            Ok(run) => Some(run),
                            Err(error) => {
                                warn!(%error, "failed to record forward proxy interval refresh");
                                None
                            }
                        },
                    };
                    let observation = crate::TaskExecutionObservation::begin_for_system_task_run(
                        "forward_proxy_subscription_refresh",
                        "interval",
                        "processing",
                        task_run.as_ref().map(|run| run.id),
                    );
                    let refresh_result = crate::with_managed_task_observation(
                        observation.clone(),
                        refresh_forward_proxy_subscriptions(state.clone(), false, None),
                    )
                    .await;
                    observation.finish_from_result(&refresh_result);
                    if let Err(err) = refresh_result {
                        let detail = err.to_string();
                        if let Some(run) = task_run.as_ref() {
                            let _ = finish_system_task_run_reliably(
                                state.as_ref(),
                                Some(&cancel),
                                run,
                                SystemTaskStatus::Failed,
                                Some("forward proxy interval refresh failed".to_string()),
                                Some(detail),
                            )
                            .await;
                        }
                        warn!(error = %err, "failed to refresh forward proxy subscriptions");
                    } else {
                        if let Some(run) = task_run.as_ref() {
                            let _ = finish_system_task_run_reliably(
                                state.as_ref(),
                                Some(&cancel),
                                run,
                                SystemTaskStatus::Success,
                                Some("forward proxy interval refresh completed".to_string()),
                                None,
                            )
                            .await;
                        }
                    }
                    if crate::maintenance_store::migration_task_is_held(
                        "forward_proxy_subscription_refresh.rollups",
                    )
                    .await
                    {
                        debug!(
                            wait_reason = crate::maintenance_store::MIGRATION_HOLD_REASON,
                            "forward proxy dashboard rollups deferred by migration hold"
                        );
                    } else if let Err(err) = flush_dashboard_network_socket_minute_rollups(
                        &state.pool,
                        state.dashboard_network_speed_cache.as_ref(),
                        Utc::now(),
                    )
                    .await
                    {
                        warn!(error = %err, "failed to flush dashboard socket minute rollups");
                    }
                }
            }
        }
    })
}

pub(crate) fn spawn_pool_orphan_recovery_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let mut ticker = interval(POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("pool orphan recovery maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if crate::maintenance_store::legacy_worker_should_skip("pool_orphan_recovery").await {
                        continue;
                    }
                    let Some(_recovery_guard) = wait_for_startup_recovery_guard(&cancel).await else {
                        break;
                    };
                    let Some(_execution_lease) =
                        crate::maintenance_store::try_acquire_task_execution("pool_orphan_recovery")
                    else {
                        continue;
                    };
                    let observation = crate::TaskExecutionObservation::begin(
                        "pool_orphan_recovery",
                        &crate::maintenance_store::task_title_for_observation(
                            "pool_orphan_recovery",
                        ),
                        "interval",
                        crate::maintenance_store::task_execution_class("pool_orphan_recovery"),
                        "processing",
                    );
                    let result = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => None,
                        result = recover_stale_pool_early_phase_orphans_runtime(state.as_ref()) => Some(result),
                    };
                    let Some(result) = result else {
                        info!("pool orphan recovery cancelled during execution");
                        break;
                    };
                    if let Ok(outcome) = result.as_ref() {
                        observation.set_processed_work(
                            i64::try_from(outcome.recovered_attempts).unwrap_or(i64::MAX),
                        );
                    }
                    observation.finish_with_status(if result.is_ok() {
                        "success"
                    } else {
                        "failed"
                    });
                    match result {
                        Ok(outcome) => {
                            if outcome.recovered_attempts > 0 || outcome.recovered_invocations > 0 {
                                warn!(
                                    recovered_attempts = outcome.recovered_attempts,
                                    recovered_invocations = outcome.recovered_invocations,
                                    "runtime pool orphan recovery swept stale early-phase rows"
                                );
                            }
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to recover stale pool early-phase orphans at runtime");
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
pub(crate) mod managed_task_dispatch_tests {
    use std::sync::Arc;

    use super::managed_startup_backfill_task;
    use crate::AppState;

    pub(crate) async fn run_managed_task_once_for_test(
        state: &Arc<AppState>,
        task_key: &str,
    ) -> anyhow::Result<String> {
        super::run_managed_task_once(state, task_key, 0).await
    }

    #[test]
    fn all_registered_startup_backfill_children_resolve_for_run_now() {
        for key in crate::maintenance_store::STARTUP_BACKFILL_TASKS {
            assert!(
                managed_startup_backfill_task(key).is_ok(),
                "registered child task {key} cannot run"
            );
        }
        assert!(managed_startup_backfill_task("unknown_child").is_err());
    }
}

#[cfg(test)]
mod managed_task_lease_conflict_tests {
    use super::*;

    #[test]
    fn owner_failure_details_redact_errors_and_preserve_observed_work() {
        let details = managed_task_owner_v2_failure_details(
            "retention_archive",
            "schedule",
            "owner_failed",
            "api_key=secret-value /private/path",
            Some(7),
            Some(2),
        )
        .expect("retention owner failure details");

        assert!(details["discovered"].is_null());
        assert!(details["selected"].is_null());
        assert_eq!(details["committed"], 7);
        assert!(details["selectedScope"]["selected"].is_null());
        assert_eq!(details["selectedScope"]["committed"], 7);
        assert!(details["selectedScope"]["batches"].is_null());
        assert_eq!(details["selectedScope"]["rawFilesRemoved"], 2);
        assert!(
            !details["fatalError"]
                .as_str()
                .unwrap()
                .contains("secret-value")
        );

        let unknown = managed_task_owner_v2_failure_details(
            "retention_archive",
            "schedule",
            "owner_failed",
            "worker panic",
            None,
            None,
        )
        .expect("unknown retention owner failure details");
        assert!(unknown["discovered"].is_null());
        assert!(unknown["selected"].is_null());
        assert!(unknown["processedCount"].is_null());
        assert!(unknown["selectedScope"]["rawFilesRemoved"].is_null());
    }

    #[test]
    fn archive_backfill_file_tasks_use_heavy_file_gate() {
        for task_key in [
            "startup_backfill.pool_attempt_public_id_archives",
            "startup_backfill.upstream_activity_archives",
            "startup_backfill.pool_upstream_node_health_archives",
            "startup_backfill.legacy_detail_mirrors",
            "startup_backfill.historical_rollups",
        ] {
            assert!(
                managed_task_requires_heavy_file_operation(task_key),
                "archive backfill task {task_key} must use the heavy-file gate"
            );
        }
    }
}

#[cfg(test)]
mod startup_recovery_contract_tests {
    use super::*;

    #[test]
    fn startup_recovery_contract_is_bounded_and_fenced() {
        assert_eq!(STARTUP_RECOVERY_BATCH_ROWS, 64);
        assert_eq!(STARTUP_RECOVERY_BUSY_TIMEOUT, Duration::from_millis(200));
        assert_eq!(
            STARTUP_RECOVERY_ADMISSION_BUDGET,
            Duration::from_millis(250)
        );
        assert_eq!(STARTUP_RECOVERY_RETRY_DELAYS[0], Duration::from_millis(250));
        assert_eq!(
            STARTUP_RECOVERY_RETRY_DELAYS[STARTUP_RECOVERY_RETRY_DELAYS.len() - 1],
            Duration::from_secs(5)
        );
        assert_eq!(
            StartupRecoveryFence {
                invocation_max_id: 12,
                pool_attempt_max_id: 18,
                captured: true,
            },
            StartupRecoveryFence {
                invocation_max_id: 12,
                pool_attempt_max_id: 18,
                captured: true,
            }
        );
    }

    #[test]
    fn startup_recovery_has_one_owner_and_releases_it() {
        let first = StartupRecoveryActiveGuard::acquire();
        assert!(first.is_some());
        assert!(StartupRecoveryActiveGuard::acquire().is_none());
        drop(first);
        let second = StartupRecoveryActiveGuard::acquire();
        assert!(second.is_some());
        drop(second);
    }

    #[tokio::test]
    async fn startup_recovery_retry_wait_honors_shutdown() {
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            wait_for_startup_recovery_retry(&task_shutdown, Duration::from_secs(5)).await
        });
        shutdown.cancel();
        assert!(!task.await.expect("retry wait task should finish"));
    }
}
