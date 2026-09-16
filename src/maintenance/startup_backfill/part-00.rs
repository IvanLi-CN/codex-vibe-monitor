use super::*;
use std::future::Future;

const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: u64 = 16;
const STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS: u64 = 6;
const STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT: u64 = 128;
const STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS: u64 = 6;
const COVERAGE_REPAIR_RETRY_DELAYS_SECS: [u64; 4] = [15, 60, 5 * 60, 15 * 60];

pub(crate) fn push_backfill_sample(samples: &mut Vec<String>, sample: String) {
    if samples.len() < STARTUP_BACKFILL_LOG_SAMPLE_LIMIT {
        samples.push(sample);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StartupBackfillTask {
    ProxyUsage,
    ProxyCost,
    PromptCacheKey,
    RequestedServiceTier,
    InvocationServiceTier,
    ReasoningEffort,
    FailureClassification,
    PoolAttemptPublicIdLive,
    PoolAttemptPublicIdArchives,
    UpstreamActivityLive,
    UpstreamActivityArchives,
    PoolUpstreamNodeHealthArchives,
    AccountActivityV2Coverage,
    LegacyDetailMirrors,
    HistoricalRollups,
}

#[derive(Debug, Default)]
struct StartupBackfillScheduler {
    wake_generation: AtomicU64,
    notify: Notify,
    woken_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    next_due: std::sync::Mutex<HashMap<StartupBackfillTask, DateTime<Utc>>>,
    deferred_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    pressure_deferred_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    failed_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    wake_count: AtomicU64,
    due_dispatch_count: AtomicU64,
    noop_suppressed_count: AtomicU64,
    pressure_defer_count: AtomicU64,
    failure_count: AtomicU64,
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
    fn wake(&self, task: StartupBackfillTask) {
        if let Ok(mut tasks) = self.woken_tasks.lock() {
            tasks.insert(task);
        }
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.insert(task, Utc::now());
        }
        self.wake_count.fetch_add(1, Ordering::Relaxed);
        self.wake_generation.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    fn generation(&self) -> u64 {
        self.wake_generation.load(Ordering::Acquire)
    }

    fn drain_woken_tasks(&self) -> Vec<StartupBackfillTask> {
        let Ok(mut tasks) = self.woken_tasks.lock() else {
            return Vec::new();
        };
        StartupBackfillTask::ordered_tasks()
            .iter()
            .copied()
            .filter(|task| tasks.remove(task))
            .collect()
    }

    fn drain_due_tasks(&self, now: DateTime<Utc>) -> Vec<StartupBackfillTask> {
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
        self.due_dispatch_count
            .fetch_add(due_tasks.len() as u64, Ordering::Relaxed);
        due_tasks
    }

    fn record_next_due(&self, task: StartupBackfillTask, due: DateTime<Utc>) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.insert(task, due);
        }
    }

    fn clear_next_due(&self, task: StartupBackfillTask) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.remove(&task);
        }
    }

    fn mark_pressure_deferred(&self, task: StartupBackfillTask) {
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock() {
            tasks.insert(task);
        }
    }

    fn defer_for_pressure(&self, task: StartupBackfillTask, retry_at: DateTime<Utc>) {
        self.mark_pressure_deferred(task);
        self.record_next_due(task, retry_at);
    }

    fn take_pressure_deferred_tasks(&self) -> Vec<StartupBackfillTask> {
        let tasks = self
            .pressure_deferred_tasks
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        if let Ok(mut next_due) = self.next_due.lock() {
            for task in &tasks {
                next_due.remove(task);
            }
        }
        StartupBackfillTask::ordered_tasks()
            .iter()
            .copied()
            .filter(|task| tasks.contains(task))
            .collect()
    }

    fn record_task_result(&self, task: StartupBackfillTask, failed: bool, deferred: bool) {
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
            return;
        }

        if deferred {
            self.pressure_defer_count.fetch_add(1, Ordering::Relaxed);
            let has_active_failure = self
                .failed_tasks
                .lock()
                .map(|tasks| tasks.contains(&task))
                .unwrap_or(true);
            if has_active_failure {
                return;
            }
            if let Ok(mut tasks) = self.deferred_tasks.lock() {
                tasks.insert(task);
            }
            if let Ok(mut tasks) = self.failed_tasks.lock() {
                tasks.remove(&task);
            }
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
    }

    fn record_noop_suppressed(&self) {
        self.noop_suppressed_count.fetch_add(1, Ordering::Relaxed);
    }

    fn health_snapshot(&self) -> StartupBackfillHealthSnapshot {
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

    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.next_due
            .lock()
            .ok()
            .and_then(|next_due| next_due.values().min().cloned())
    }

    async fn wait_for_wake(&self, observed_generation: u64) {
        loop {
            let notified = self.notify.notified();
            if self.generation() != observed_generation {
                return;
            }
            notified.await;
        }
    }
}

static STARTUP_BACKFILL_SCHEDULER: Lazy<StartupBackfillScheduler> =
    Lazy::new(StartupBackfillScheduler::default);

pub(crate) fn startup_backfill_health_snapshot() -> StartupBackfillHealthSnapshot {
    STARTUP_BACKFILL_SCHEDULER.health_snapshot()
}

fn startup_backfill_wait_duration(next_due: Option<DateTime<Utc>>) -> Duration {
    match next_due {
        Some(deadline) => (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO),
        None => Duration::from_secs(24 * 60 * 60),
    }
}

fn startup_backfill_progress_due(progress: &StartupBackfillProgress) -> DateTime<Utc> {
    progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .unwrap_or_else(Utc::now)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StartupBackfillTaskRunOutcome {
    actionable: bool,
    failed: bool,
    deferred: bool,
    completed: bool,
    next_due: DateTime<Utc>,
}

#[cfg(test)]
impl StartupBackfillTaskRunOutcome {
    pub(crate) fn is_pressure_deferred(self) -> bool {
        !self.actionable && !self.failed && self.deferred && self.completed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupBackfillFailureKind {
    SqliteBusyOrLocked,
    Operation,
}

impl StartupBackfillFailureKind {
    fn telemetry_reason(self) -> &'static str {
        match self {
            Self::SqliteBusyOrLocked => "sqlite_busy_or_locked",
            Self::Operation => "operation_error",
        }
    }
}

fn startup_backfill_failure_kind(err: &anyhow::Error) -> StartupBackfillFailureKind {
    let has_busy_message = err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("database is busy")
    });
    if crate::is_sqlite_lock_error(err) || has_busy_message {
        StartupBackfillFailureKind::SqliteBusyOrLocked
    } else {
        StartupBackfillFailureKind::Operation
    }
}

fn record_startup_backfill_pressure_error(
    gate: &crate::db_pressure::DbPressureGate,
    err: &anyhow::Error,
) -> bool {
    if gate.record_error("startup_backfill", err) {
        return true;
    }
    if startup_backfill_failure_kind(err) == StartupBackfillFailureKind::SqliteBusyOrLocked {
        gate.record_pressure("startup_backfill", "sqlite_busy_or_locked");
        return true;
    }
    false
}

fn startup_backfill_pressure_retry_at(
    gate: &crate::db_pressure::DbPressureGate,
    reason: crate::db_pressure::DbPressureDenyReason,
) -> DateTime<Utc> {
    match reason {
        crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => gate
            .pressure_cooldown_deadline_epoch_ms()
            .and_then(|deadline_ms| {
                i64::try_from(deadline_ms)
                    .ok()
                    .and_then(DateTime::<Utc>::from_timestamp_millis)
            })
            .unwrap_or_else(|| {
                let remaining_ms = i64::try_from(remaining_ms.max(1)).unwrap_or(i64::MAX);
                Utc::now() + ChronoDuration::milliseconds(remaining_ms)
            }),
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
            Utc::now() + ChronoDuration::seconds(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS as i64)
        }
    }
}

fn startup_backfill_pressure_defer_outcome(
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    reason: crate::db_pressure::DbPressureDenyReason,
) -> StartupBackfillTaskRunOutcome {
    let retry_at = startup_backfill_pressure_retry_at(gate, reason);
    startup_backfill_pressure_defer_outcome_at(task, reason, retry_at)
}

fn startup_backfill_pressure_defer_outcome_at(
    task: StartupBackfillTask,
    reason: crate::db_pressure::DbPressureDenyReason,
    retry_at: DateTime<Utc>,
) -> StartupBackfillTaskRunOutcome {
    STARTUP_BACKFILL_SCHEDULER.defer_for_pressure(task, retry_at);
    info!(
        task = task.log_label(),
        reason = %reason,
        defer_kind = "pressure_gate",
        defer_reason = %reason,
        next_eligibility = %retry_at,
        wake_reason = "pressure_defer",
        "startup backfill task deferred before SQLite access because database pressure gate is closed"
    );
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    }
}

async fn persist_startup_backfill_pressure_defer(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    gate: &crate::db_pressure::DbPressureGate,
    reason: crate::db_pressure::DbPressureDenyReason,
) -> Result<StartupBackfillTaskRunOutcome> {
    let retry_at = startup_backfill_pressure_retry_at(gate, reason);
    let retry_after = format_utc_iso(retry_at);
    // The caller releases its previous P2 permit before entering this helper. Re-admit this short
    // state update after any queued P1/interactive writer completes, so the defer is coordinated
    // without keeping higher-priority work behind the cancelled backfill.
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .await;
    save_startup_backfill_progress(
        &state.pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: None,
        },
    )
    .await
    .inspect_err(|err| {
        record_startup_backfill_pressure_error(gate, err);
    })?;
    Ok(startup_backfill_pressure_defer_outcome_at(
        task, reason, retry_at,
    ))
}

fn startup_backfill_pressure_error_defer_outcome(
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillTaskRunOutcome {
    let retry_at = gate
        .pressure_cooldown_deadline_epoch_ms()
        .and_then(|deadline_ms| {
            i64::try_from(deadline_ms)
                .ok()
                .and_then(DateTime::<Utc>::from_timestamp_millis)
        })
        .unwrap_or_else(|| {
            Utc::now() + ChronoDuration::seconds(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS as i64)
        });
    STARTUP_BACKFILL_SCHEDULER.defer_for_pressure(task, retry_at);
    info!(
        task = task.log_label(),
        defer_kind = "sqlite_pressure_error",
        defer_reason = "sqlite_busy_or_locked",
        next_eligibility = %retry_at,
        wake_reason = "pressure_error_defer",
        "startup backfill task deferred after a SQLite pressure error"
    );
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    }
}

fn startup_backfill_pressure_error_defer_outcome_if_recorded(
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    err: &anyhow::Error,
) -> Option<StartupBackfillTaskRunOutcome> {
    record_startup_backfill_pressure_error(gate, err)
        .then(|| startup_backfill_pressure_error_defer_outcome(task, gate))
}

impl StartupBackfillTask {
    pub(crate) fn ordered_tasks() -> &'static [Self] {
        &[
            Self::ProxyUsage,
            Self::PromptCacheKey,
            Self::RequestedServiceTier,
            Self::InvocationServiceTier,
            Self::ProxyCost,
            Self::ReasoningEffort,
            Self::FailureClassification,
            Self::PoolAttemptPublicIdLive,
            Self::PoolAttemptPublicIdArchives,
            Self::UpstreamActivityLive,
            Self::UpstreamActivityArchives,
            Self::PoolUpstreamNodeHealthArchives,
            Self::AccountActivityV2Coverage,
            Self::LegacyDetailMirrors,
            Self::HistoricalRollups,
        ]
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ProxyUsage => STARTUP_BACKFILL_TASK_PROXY_USAGE,
            Self::ProxyCost => STARTUP_BACKFILL_TASK_PROXY_COST,
            Self::PromptCacheKey => STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            Self::RequestedServiceTier => STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            Self::InvocationServiceTier => STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            Self::ReasoningEffort => STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            Self::FailureClassification => STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            Self::PoolAttemptPublicIdLive => STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_LIVE,
            Self::PoolAttemptPublicIdArchives => {
                STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_ARCHIVES
            }
            Self::UpstreamActivityLive => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            Self::UpstreamActivityArchives => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            Self::PoolUpstreamNodeHealthArchives => {
                STARTUP_BACKFILL_TASK_POOL_UPSTREAM_NODE_HEALTH_ARCHIVES
            }
            Self::AccountActivityV2Coverage => STARTUP_BACKFILL_TASK_ACCOUNT_ACTIVITY_V2_COVERAGE,
            Self::LegacyDetailMirrors => STARTUP_BACKFILL_TASK_LEGACY_DETAIL_MIRRORS,
            Self::HistoricalRollups => STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
        }
    }

    pub(crate) fn log_label(self) -> &'static str {
        match self {
            Self::ProxyUsage => "proxy usage",
            Self::ProxyCost => "proxy cost",
            Self::PromptCacheKey => "proxy prompt cache key",
            Self::RequestedServiceTier => "proxy requested service tier",
            Self::InvocationServiceTier => "invocation service tier",
            Self::ReasoningEffort => "proxy reasoning effort",
            Self::FailureClassification => "invocation failure classification",
            Self::PoolAttemptPublicIdLive => "pool attempt public id live rows",
            Self::PoolAttemptPublicIdArchives => "pool attempt public id archives",
            Self::UpstreamActivityLive => "upstream activity live rows",
            Self::UpstreamActivityArchives => "upstream activity archives",
            Self::PoolUpstreamNodeHealthArchives => "pool upstream node health archives",
            Self::AccountActivityV2Coverage => "account activity v2 coverage repair",
            Self::LegacyDetailMirrors => "legacy invocation detail mirrors",
            Self::HistoricalRollups => "historical rollup materialization",
        }
    }
}

pub(crate) fn startup_backfill_tasks_for_terminal(
    record: &ApiInvocation,
) -> Vec<StartupBackfillTask> {
    let has_request_raw = record.request_raw_path.is_some();
    let has_response_raw = record.response_raw_path.is_some();
    let is_success = record.status.as_deref().is_some_and(|status| {
        matches!(
            status.trim().to_ascii_lowercase().as_str(),
            "success" | "warning_success"
        )
    });
    let mut tasks = Vec::new();

    if is_success && record.total_tokens.is_none() && has_response_raw {
        tasks.push(StartupBackfillTask::ProxyUsage);
    }
    if has_request_raw && record.prompt_cache_key.is_none() {
        tasks.push(StartupBackfillTask::PromptCacheKey);
    }
    if has_request_raw && record.requested_service_tier.is_none() {
        tasks.push(StartupBackfillTask::RequestedServiceTier);
    }
    if has_request_raw && record.reasoning_effort.is_none() {
        tasks.push(StartupBackfillTask::ReasoningEffort);
    }
    if has_response_raw && record.service_tier.is_none() {
        tasks.push(StartupBackfillTask::InvocationServiceTier);
    }
    if !is_success
        && (record.failure_kind.is_none()
            || record.failure_class.is_none()
            || record.is_actionable.is_none())
    {
        tasks.push(StartupBackfillTask::FailureClassification);
    }
    tasks
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct StartupBackfillProgressRow {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: i64,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: i64,
    last_updated: i64,
    last_status: String,
    suspension_reason: Option<String>,
    next_probe_at: Option<String>,
    wake_generation: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupBackfillProgress {
    pub(crate) task_name: String,
    pub(crate) cursor_id: i64,
    pub(crate) next_run_after: Option<String>,
    pub(crate) zero_update_streak: u32,
    pub(crate) last_started_at: Option<String>,
    pub(crate) last_finished_at: Option<String>,
    pub(crate) last_scanned: u64,
    pub(crate) last_updated: u64,
    pub(crate) last_status: String,
    pub(crate) suspension_reason: Option<String>,
    pub(crate) next_probe_at: Option<String>,
    pub(crate) wake_generation: u64,
}

impl StartupBackfillProgress {
    pub(crate) fn pending(task_name: impl Into<String>) -> Self {
        Self {
            task_name: task_name.into(),
            cursor_id: 0,
            next_run_after: None,
            zero_update_streak: 0,
            last_started_at: None,
            last_finished_at: None,
            last_scanned: 0,
            last_updated: 0,
            last_status: STARTUP_BACKFILL_STATUS_IDLE.to_string(),
            suspension_reason: None,
            next_probe_at: None,
            wake_generation: 0,
        }
    }

    pub(crate) fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_run_after
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_none_or(|deadline| deadline <= now)
    }
}

impl From<StartupBackfillProgressRow> for StartupBackfillProgress {
    fn from(value: StartupBackfillProgressRow) -> Self {
        Self {
            task_name: value.task_name,
            cursor_id: value.cursor_id,
            next_run_after: value.next_run_after,
            zero_update_streak: value.zero_update_streak.max(0) as u32,
            last_started_at: value.last_started_at,
            last_finished_at: value.last_finished_at,
            last_scanned: value.last_scanned.max(0) as u64,
            last_updated: value.last_updated.max(0) as u64,
            last_status: value.last_status,
            suspension_reason: value.suspension_reason,
            next_probe_at: value.next_probe_at,
            wake_generation: value.wake_generation.max(0) as u64,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StartupBackfillRunState {
    next_cursor_id: i64,
    scanned: u64,
    updated: u64,
    hit_scan_limit: bool,
    retry_soon: bool,
    force_idle: bool,
    source_unavailable: bool,
    samples: Vec<String>,
}

pub(crate) fn startup_backfill_next_delay(
    run: &StartupBackfillRunState,
    zero_update_streak: u32,
) -> Duration {
    if run.source_unavailable {
        Duration::from_secs(24 * 60 * 60)
    } else if run.force_idle {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else if run.retry_soon {
        Duration::from_secs(15)
    } else if run.updated > 0 {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.hit_scan_limit && run.scanned > 0 {
        Duration::from_secs(match zero_update_streak {
            0 | 1 => 15,
            2 => 60,
            3 => 5 * 60,
            _ => 15 * 60,
        })
    } else if run.hit_scan_limit {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.scanned == 0 || zero_update_streak > 0 {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    }
}

fn historical_rollup_should_retry_soon(hit_budget: bool, candidate_count: usize) -> bool {
    hit_budget && candidate_count > 0
}

pub(crate) fn startup_backfill_next_run_after(
    run: &StartupBackfillRunState,
    zero_update_streak: u32,
) -> String {
    format_utc_iso(
        Utc::now()
            + ChronoDuration::from_std(startup_backfill_next_delay(run, zero_update_streak))
                .unwrap_or_else(|_| {
                    ChronoDuration::seconds(STARTUP_BACKFILL_IDLE_INTERVAL_SECS as i64)
                }),
    )
}

pub(crate) fn historical_rollup_startup_backfill_run_state(
    cursor_id: i64,
    zero_update_streak: u32,
    before: &HistoricalRollupBackfillSnapshot,
    after: &HistoricalRollupBackfillSnapshot,
    summary: &HistoricalRollupMaterializationSummary,
    pending_before: u64,
    pending_after: u64,
) -> StartupBackfillRunState {
    let archive_progress = before
        .legacy_archive_pending
        .saturating_sub(after.legacy_archive_pending);
    let usage_breakdown_progress = before
        .pending_usage_breakdown_batches
        .saturating_sub(after.pending_usage_breakdown_batches);
    let bucket_progress = before.pending_buckets.saturating_sub(after.pending_buckets);
    let selected_backlog_progress = pending_before.saturating_sub(pending_after);
    let attempted_archive_batches = summary
        .scanned_archive_batches
        .saturating_sub(summary.skipped_archive_batches);
    let scanned_all_pending_archives = attempted_archive_batches as u64 >= pending_before;
    let exhausted_blocked_cycle = summary.blocked_archive_batches == attempted_archive_batches
        && selected_backlog_progress == 0
        && bucket_progress == 0
        && zero_update_streak.saturating_add(attempted_archive_batches as u32) as u64
            >= pending_before;
    let permanently_blocked = summary.blocked_archive_batches > 0
        && selected_backlog_progress == 0
        && bucket_progress == 0
        && (scanned_all_pending_archives || exhausted_blocked_cycle);

    StartupBackfillRunState {
        next_cursor_id: cursor_id.saturating_add(attempted_archive_batches as i64),
        scanned: summary.scanned_archive_batches as u64,
        updated: archive_progress
            .max(usage_breakdown_progress)
            .max(selected_backlog_progress)
            .max(bucket_progress),
        hit_scan_limit: pending_after > 0 && !permanently_blocked,
        retry_soon: false,
        force_idle: pending_after == 0 || permanently_blocked,
        source_unavailable: permanently_blocked,
        samples: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BackfillBatchOutcome<T> {
    pub(crate) summary: T,
    pub(crate) next_cursor_id: i64,
    pub(crate) hit_budget: bool,
    pub(crate) samples: Vec<String>,
}

pub(crate) fn startup_backfill_query_limit(scanned: u64, scan_limit: Option<u64>) -> i64 {
    let remaining = scan_limit
        .map(|limit| limit.saturating_sub(scanned))
        .unwrap_or(BACKFILL_BATCH_SIZE as u64);
    remaining.min(BACKFILL_BATCH_SIZE as u64).max(1) as i64
}

pub(crate) fn startup_backfill_budget_reached(
    started_at: Instant,
    scanned: u64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    let hit_scan_limit = scan_limit.is_some_and(|limit| scanned >= limit);
    let hit_elapsed_limit = max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit);
    hit_scan_limit || hit_elapsed_limit
}

pub(crate) fn startup_backfill_samples_text(samples: &[String]) -> String {
    if samples.is_empty() {
        "-".to_string()
    } else {
        samples.join(" | ")
    }
}

fn startup_backfill_scan_limit(source_unavailable_probe: bool) -> u64 {
    if source_unavailable_probe {
        100
    } else {
        STARTUP_BACKFILL_SCAN_LIMIT
    }
}

fn startup_backfill_run_budget(source_unavailable_probe: bool) -> Duration {
    if source_unavailable_probe {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)
    }
}

pub(crate) async fn startup_backfill_task_progress_key(
    state: &AppState,
    task: StartupBackfillTask,
) -> String {
    match task {
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await;
            startup_backfill_task_progress_key_for_catalog(task, &catalog)
        }
        _ => task.name().to_string(),
    }
}

pub(crate) fn startup_backfill_task_progress_key_for_catalog(
    task: StartupBackfillTask,
    catalog: &PricingCatalog,
) -> String {
    match task {
        StartupBackfillTask::ProxyCost => format!(
            "{}:{}",
            task.name(),
            pricing_backfill_attempt_version(catalog)
        ),
        _ => task.name().to_string(),
    }
}
