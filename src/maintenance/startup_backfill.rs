use super::*;
use std::future::Future;

const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: u64 = 16;
const STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS: u64 = 6;
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
    HistoricalRollups,
}

#[derive(Debug, Default)]
struct StartupBackfillScheduler {
    wake_generation: AtomicU64,
    notify: Notify,
    woken_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    next_due: std::sync::Mutex<HashMap<StartupBackfillTask, DateTime<Utc>>>,
    deferred_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    pressure_deferred_tasks: std::sync::Mutex<HashMap<StartupBackfillTask, PressureDeferredTask>>,
    failed_tasks: std::sync::Mutex<HashSet<StartupBackfillTask>>,
    wake_count: AtomicU64,
    due_dispatch_count: AtomicU64,
    noop_suppressed_count: AtomicU64,
    pressure_defer_count: AtomicU64,
    failure_count: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PressureDeferredTask {
    pressure_generation: u64,
    next_eligibility: Option<DateTime<Utc>>,
    selected_for_eligibility: bool,
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
        if self.is_pressure_deferred(task) {
            return;
        }
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
        if self.is_pressure_deferred(task) {
            return;
        }
        self.record_next_due_unchecked(task, due);
    }

    fn record_next_due_unchecked(&self, task: StartupBackfillTask, due: DateTime<Utc>) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.insert(task, due);
        }
    }

    fn record_pressure_eligibility_deadline(
        &self,
        task: StartupBackfillTask,
        pressure_generation: u64,
        due: DateTime<Utc>,
    ) {
        // Keep the generation check and due write ordered under the pressure-defer lock. An
        // older arming pass must not overwrite the next deadline after a newer defer replaces it.
        let recorded = self
            .pressure_deferred_tasks
            .lock()
            .ok()
            .and_then(|tasks| {
                tasks
                    .get(&task)
                    .filter(|entry| {
                        entry.pressure_generation == pressure_generation
                            && entry.next_eligibility == Some(due)
                    })
                    .and_then(|_| {
                        self.next_due.lock().ok().map(|mut next_due| {
                            next_due.insert(task, due);
                        })
                    })
            })
            .is_some();
        if !recorded {
            return;
        }
        // A cooldown can be registered after the supervisor has already consumed the pressure
        // event and selected its idle sleep. Wake it once so the new absolute deadline is armed.
        self.wake_generation.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    fn clear_next_due(&self, task: StartupBackfillTask) {
        if let Ok(mut next_due) = self.next_due.lock() {
            next_due.remove(&task);
        }
    }

    fn clear_pressure_eligibility_deadline(
        &self,
        task: StartupBackfillTask,
        pressure_generation: u64,
    ) {
        // A stale busy defer must not erase a cooldown deadline installed by a newer pressure
        // generation after this task released the scheduler state lock.
        let _cleared = self.pressure_deferred_tasks.lock().ok().and_then(|tasks| {
            tasks
                .get(&task)
                .filter(|entry| {
                    entry.pressure_generation == pressure_generation
                        && entry.next_eligibility.is_none()
                })
                .and_then(|_| {
                    self.next_due.lock().ok().map(|mut next_due| {
                        next_due.remove(&task);
                    })
                })
        });
    }

    fn defer_for_pressure(
        &self,
        task: StartupBackfillTask,
        next_eligibility: Option<DateTime<Utc>>,
        pressure_generation: u64,
    ) -> bool {
        let newly_registered = self
            .pressure_deferred_tasks
            .lock()
            .map(|mut tasks| match tasks.get(&task) {
                Some(entry) if entry.pressure_generation == pressure_generation => false,
                _ => {
                    tasks.insert(
                        task,
                        PressureDeferredTask {
                            pressure_generation,
                            next_eligibility,
                            selected_for_eligibility: false,
                        },
                    );
                    true
                }
            })
            .unwrap_or(false);
        if newly_registered {
            if let Some(next_eligibility) = next_eligibility {
                self.record_pressure_eligibility_deadline(
                    task,
                    pressure_generation,
                    next_eligibility,
                );
            } else {
                self.clear_pressure_eligibility_deadline(task, pressure_generation);
            }
            if let Ok(mut tasks) = self.woken_tasks.lock() {
                tasks.remove(&task);
            }
        }
        newly_registered
    }

    fn defer_for_current_gate(
        &self,
        task: StartupBackfillTask,
        gate: &crate::db_pressure::DbPressureGate,
    ) -> Option<(
        crate::db_pressure::DbPressureDenyReason,
        Option<DateTime<Utc>>,
        bool,
    )> {
        let Some(reason) = gate.background_deny_reason() else {
            // The only busy-slot release may have arrived before this defer could be registered.
            // Re-enqueue once while the gate is open rather than retain a busy defer with no
            // future eligibility event to consume.
            self.wake(task);
            return None;
        };
        let pressure_generation = gate.pressure_generation();
        let next_eligibility = startup_backfill_pressure_eligibility_deadline(gate, reason);
        let newly_registered = self.defer_for_pressure(task, next_eligibility, pressure_generation);
        // The deny recheck and generation read cannot reserve the gate. Once the deferred entry
        // is visible, absorb a concurrently-recorded cooldown before its notification can be
        // consumed by the supervisor without this task present.
        self.arm_pressure_deferred_deadlines(gate);
        let next_eligibility = self
            .pressure_deferred_entry(task)
            .and_then(|entry| entry.next_eligibility);
        Some((reason, next_eligibility, newly_registered))
    }

    fn is_pressure_deferred(&self, task: StartupBackfillTask) -> bool {
        self.pressure_deferred_tasks
            .lock()
            .map(|tasks| tasks.contains_key(&task))
            .unwrap_or(false)
    }

    fn pressure_deferred_entry(&self, task: StartupBackfillTask) -> Option<PressureDeferredTask> {
        self.pressure_deferred_tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(&task).copied())
    }

    fn clear_pressure_deferred_if_matches(
        &self,
        task: StartupBackfillTask,
        entry: Option<PressureDeferredTask>,
    ) {
        let Some(entry) = entry else {
            return;
        };
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock()
            && tasks.get(&task) == Some(&entry)
        {
            tasks.remove(&task);
        }
    }

    fn requeue_pressure_deferred(&self, task: StartupBackfillTask) {
        if let Ok(mut tasks) = self.pressure_deferred_tasks.lock()
            && let Some(entry) = tasks.get_mut(&task)
        {
            entry.selected_for_eligibility = false;
        }
    }

    fn arm_pressure_deferred_deadlines(&self, gate: &crate::db_pressure::DbPressureGate) {
        let Some(next_eligibility) =
            gate.pressure_cooldown_deadline_epoch_ms()
                .and_then(|deadline_ms| {
                    i64::try_from(deadline_ms)
                        .ok()
                        .and_then(DateTime::<Utc>::from_timestamp_millis)
                })
        else {
            return;
        };
        let pressure_generation = gate.pressure_generation();
        let tasks = self
            .pressure_deferred_tasks
            .lock()
            .map(|mut tasks| {
                tasks
                    .iter_mut()
                    .filter_map(|(task, entry)| {
                        let needs_deadline = entry.pressure_generation != pressure_generation
                            || entry.next_eligibility.is_none();
                        if !needs_deadline {
                            return None;
                        }
                        entry.pressure_generation = pressure_generation;
                        entry.next_eligibility = Some(next_eligibility);
                        entry.selected_for_eligibility = false;
                        Some((*task, pressure_generation))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (task, pressure_generation) in tasks {
            self.record_pressure_eligibility_deadline(task, pressure_generation, next_eligibility);
        }
    }

    fn take_pressure_deferred_tasks_if_eligible(
        &self,
        gate: &crate::db_pressure::DbPressureGate,
    ) -> Vec<StartupBackfillTask> {
        if gate.background_deny_reason().is_some() {
            self.arm_pressure_deferred_deadlines(gate);
            return Vec::new();
        }
        let tasks = self
            .pressure_deferred_tasks
            .lock()
            .map(|mut tasks| {
                StartupBackfillTask::ordered_tasks()
                    .iter()
                    .copied()
                    .filter(|task| {
                        let Some(entry) = tasks.get_mut(task) else {
                            return false;
                        };
                        if entry.selected_for_eligibility {
                            return false;
                        }
                        entry.selected_for_eligibility = true;
                        true
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Ok(mut next_due) = self.next_due.lock() {
            for task in &tasks {
                next_due.remove(task);
            }
        }
        tasks
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

fn startup_backfill_pressure_eligibility_deadline(
    gate: &crate::db_pressure::DbPressureGate,
    reason: crate::db_pressure::DbPressureDenyReason,
) -> Option<DateTime<Utc>> {
    match reason {
        crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => gate
            .pressure_cooldown_deadline_epoch_ms()
            .and_then(|deadline_ms| {
                i64::try_from(deadline_ms)
                    .ok()
                    .and_then(DateTime::<Utc>::from_timestamp_millis)
            })
            .or_else(|| {
                let remaining_ms = i64::try_from(remaining_ms.max(1)).unwrap_or(i64::MAX);
                Some(Utc::now() + ChronoDuration::milliseconds(remaining_ms))
            }),
        // Permit release emits an eligibility event. A synthetic timer would redispatch a task
        // while the same slot remains occupied, so busy defers have no due-ticker fallback.
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy => None,
    }
}

fn register_startup_backfill_pressure_defer(
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Option<DateTime<Utc>> {
    let (reason, next_eligibility, newly_registered) =
        STARTUP_BACKFILL_SCHEDULER.defer_for_current_gate(task, gate)?;
    if newly_registered {
        info!(
            task = task.log_label(),
            reason = %reason,
            defer_kind = "pressure_gate",
            defer_reason = %reason,
            next_eligibility = ?next_eligibility,
            wake_reason = "pressure_defer",
            "startup backfill task deferred before SQLite access because database pressure gate is closed"
        );
    }
    next_eligibility
}

fn startup_backfill_pressure_defer_outcome(
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillTaskRunOutcome {
    let next_due = register_startup_backfill_pressure_defer(task, gate).unwrap_or_else(|| {
        Utc::now() + ChronoDuration::seconds(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS as i64)
    });
    STARTUP_BACKFILL_SCHEDULER.requeue_pressure_deferred(task);
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due,
    }
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
    let newly_registered = STARTUP_BACKFILL_SCHEDULER.defer_for_pressure(
        task,
        Some(retry_at),
        gate.pressure_generation(),
    );
    STARTUP_BACKFILL_SCHEDULER.requeue_pressure_deferred(task);
    if newly_registered {
        info!(
            task = task.log_label(),
            defer_kind = "sqlite_pressure_error",
            defer_reason = "sqlite_busy_or_locked",
            next_eligibility = %retry_at,
            wake_reason = "pressure_error_defer",
            "startup backfill task deferred after a SQLite pressure error"
        );
    }
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

pub(crate) async fn load_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    Ok(sqlx::query_as::<_, StartupBackfillProgressRow>(
        r#"
        SELECT
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation
        FROM startup_backfill_progress
        WHERE task_name = ?1
        LIMIT 1
        "#,
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?
    .map(Into::into)
    .unwrap_or_else(|| StartupBackfillProgress::pending(task_name.to_string())))
}

pub(crate) async fn mark_startup_backfill_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation
        )
        VALUES (?1, ?2, NULL, 0, ?3, NULL, 0, 0, ?4, NULL, NULL, 0)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = NULL,
            last_started_at = excluded.last_started_at,
            last_status = excluded.last_status,
            suspension_reason = NULL,
            next_probe_at = NULL
        "#,
    )
    .bind(task_name)
    .bind(cursor_id)
    .bind(&now)
    .bind(STARTUP_BACKFILL_STATUS_RUNNING)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) struct StartupBackfillProgressUpdate<'a> {
    pub(crate) cursor_id: i64,
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) zero_update_streak: u32,
    pub(crate) next_run_after: &'a str,
    pub(crate) status: &'a str,
    pub(crate) suspension_reason: Option<&'a str>,
}

pub(crate) async fn save_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    let finished_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, 0)
        ON CONFLICT(task_name) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_finished_at = excluded.last_finished_at,
            last_scanned = excluded.last_scanned,
            last_updated = excluded.last_updated,
            last_status = excluded.last_status,
            suspension_reason = excluded.suspension_reason,
            next_probe_at = excluded.next_probe_at
        "#,
    )
    .bind(task_name)
    .bind(update.cursor_id)
    .bind(update.next_run_after)
    .bind(i64::from(update.zero_update_streak))
    .bind(&finished_at)
    .bind(update.scanned as i64)
    .bind(update.updated as i64)
    .bind(update.status)
    .bind(update.suspension_reason)
    .bind(if update.suspension_reason.is_some() {
        Some(update.next_run_after)
    } else {
        None
    })
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn wake_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
) -> Result<u64> {
    wake_startup_backfill_tasks_with_pricing_catalog(pool, tasks, None, wake_reason).await
}

pub(crate) async fn wake_startup_backfill_tasks_with_pricing_catalog(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    pricing_catalog: Option<&PricingCatalog>,
    wake_reason: &'static str,
) -> Result<u64> {
    wake_startup_backfill_tasks_with_pricing_catalog_and_gate(
        pool,
        tasks,
        pricing_catalog,
        wake_reason,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
}

pub(crate) async fn wake_startup_backfill_tasks_with_gate(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<u64> {
    wake_startup_backfill_tasks_with_pricing_catalog_and_gate(pool, tasks, None, wake_reason, gate)
        .await
}

async fn wake_startup_backfill_tasks_with_pricing_catalog_and_gate(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    pricing_catalog: Option<&PricingCatalog>,
    wake_reason: &'static str,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<u64> {
    let mut woken = 0;
    let mut proxy_cost_catalog_missing = false;
    for task in tasks {
        // A pressure defer is an in-memory scheduler decision. Do not turn an input event in
        // the same cooldown into a progress write that clears its pending eligibility deadline.
        if STARTUP_BACKFILL_SCHEDULER.is_pressure_deferred(*task) {
            continue;
        }
        // Hold admission across the wake write. A deny remains entirely in memory, and a gate
        // closure after successful admission does not retroactively interrupt the admitted work.
        let _permit = match gate.try_begin_background("startup_backfill_input_wake") {
            Ok(permit) => permit,
            Err(_) => {
                register_startup_backfill_pressure_defer(*task, gate);
                continue;
            }
        };
        if STARTUP_BACKFILL_SCHEDULER.is_pressure_deferred(*task) {
            continue;
        }
        let task_name = match task {
            StartupBackfillTask::ProxyCost => {
                let Some(catalog) = pricing_catalog else {
                    proxy_cost_catalog_missing = true;
                    continue;
                };
                startup_backfill_task_progress_key_for_catalog(*task, catalog)
            }
            _ => task.name().to_string(),
        };
        let outcome = match sqlx::query(
            r#"
            INSERT INTO startup_backfill_progress (
                task_name,
                cursor_id,
                next_run_after,
                zero_update_streak,
                last_started_at,
                last_finished_at,
                last_scanned,
                last_updated,
                last_status,
                suspension_reason,
                next_probe_at,
                wake_generation
            )
            VALUES (?1, 0, NULL, 0, NULL, NULL, 0, 0, ?2, NULL, NULL, 1)
            ON CONFLICT(task_name) DO UPDATE SET
                next_run_after = NULL,
                next_probe_at = NULL,
                suspension_reason = NULL,
                wake_generation = startup_backfill_progress.wake_generation + 1,
                last_status = ?2
            "#,
        )
        .bind(&task_name)
        .bind(STARTUP_BACKFILL_STATUS_IDLE)
        .execute(pool)
        .await
        .with_context(|| {
            format!(
                "wake startup backfill task={} progress_key={} wake_reason={wake_reason}",
                task.name(),
                task_name
            )
        }) {
            Ok(outcome) => outcome,
            Err(err) => {
                if startup_backfill_pressure_error_defer_outcome_if_recorded(*task, gate, &err)
                    .is_some()
                {
                    continue;
                }
                return Err(err);
            }
        };
        woken += outcome.rows_affected();
        STARTUP_BACKFILL_SCHEDULER.wake(*task);
    }
    if woken > 0 {
        info!(
            wake_reason,
            woken,
            task_count = tasks.len(),
            "woke affected startup backfill tasks"
        );
    }
    if proxy_cost_catalog_missing {
        return Err(anyhow!(
            "wake startup backfill task={} requires the runtime pricing catalog",
            StartupBackfillTask::ProxyCost.name()
        ));
    }
    Ok(woken)
}

#[derive(Debug, Default)]
pub(crate) struct StartupBackfillMaintenancePass {
    pub(crate) ran_actionable_task: bool,
    pub(crate) had_failure: bool,
}

pub(crate) async fn defer_startup_backfill_task(
    state: &AppState,
    task: StartupBackfillTask,
    delay: Duration,
    wake_reason: &'static str,
) -> Result<()> {
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_after = Utc::now() + ChronoDuration::from_std(delay).unwrap_or_default();
    let retry_after = format_utc_iso(retry_after);
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: &progress.last_status,
            suspension_reason: progress.suspension_reason.as_deref(),
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    info!(
        task = task.log_label(),
        next_retry_after = %retry_after,
        wake_reason,
        "startup backfill task retry scheduled"
    );
    Ok(())
}

pub(crate) fn coverage_repair_retry_delay(retry_generation: u32) -> Duration {
    let index = retry_generation.saturating_sub(1) as usize;
    Duration::from_secs(
        COVERAGE_REPAIR_RETRY_DELAYS_SECS[index.min(COVERAGE_REPAIR_RETRY_DELAYS_SECS.len() - 1)],
    )
}

pub(crate) async fn defer_startup_backfill_coverage_repair(
    state: &AppState,
) -> Result<DateTime<Utc>> {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_generation = progress.zero_update_streak.saturating_add(1);
    let delay = coverage_repair_retry_delay(retry_generation);
    let retry_after = Utc::now() + ChronoDuration::from_std(delay).unwrap_or_default();
    let retry_after = format_utc_iso(retry_after);
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: retry_generation,
            next_run_after: &retry_after,
            status: &progress.last_status,
            suspension_reason: progress.suspension_reason.as_deref(),
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    let backoff_stage = match delay.as_secs() {
        0..=15 => "15s",
        16..=60 => "1m",
        61..=300 => "5m",
        _ => "15m",
    };
    info!(
        task = task.log_label(),
        next_retry_after = %retry_after,
        retry_generation,
        backoff_stage,
        wake_reason = "coverage_repair_retry",
        "startup backfill coverage repair retry scheduled"
    );
    Ok(retry_at)
}

pub(crate) async fn record_startup_backfill_coverage_repair_progress(
    state: &AppState,
    outcome: ActiveAccountActivityV2RepairOutcome,
) -> Result<DateTime<Utc>> {
    if outcome.repaired_bucket_count == 0 {
        return Ok(Utc::now());
    }

    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_after = format_utc_iso(
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    );
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: 0,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    info!(
        task = task.log_label(),
        coverage_priority_bucket_count = outcome.priority_bucket_count,
        repaired_bucket_count = outcome.repaired_bucket_count,
        next_retry_after = %retry_after,
        retry_generation = 0_u32,
        backoff_stage = "15s",
        wake_reason = "coverage_repair_progress",
        "startup backfill coverage repair progress reset its retry backoff"
    );
    Ok(retry_at)
}

pub(crate) async fn wake_startup_backfill_coverage_repair(
    pool: &Pool<Sqlite>,
    wake_reason: &'static str,
) -> Result<u64> {
    wake_startup_backfill_coverage_repair_with_gate(
        pool,
        wake_reason,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
}

pub(crate) async fn wake_startup_backfill_coverage_repair_with_gate(
    pool: &Pool<Sqlite>,
    wake_reason: &'static str,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<u64> {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    if STARTUP_BACKFILL_SCHEDULER.is_pressure_deferred(task) {
        return Ok(0);
    }
    let _permit = match gate.try_begin_background("startup_backfill_coverage_wake") {
        Ok(permit) => permit,
        Err(_) => {
            register_startup_backfill_pressure_defer(task, gate);
            return Ok(0);
        }
    };
    if STARTUP_BACKFILL_SCHEDULER.is_pressure_deferred(task) {
        return Ok(0);
    }
    let task_name = task.name();
    let progress = match load_startup_backfill_progress(pool, task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            if startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err).is_some()
            {
                return Ok(0);
            }
            return Err(err);
        }
    };
    let deadline_preserved = !progress.is_due(Utc::now())
        && (progress.zero_update_streak > 0 || progress.last_status == STARTUP_BACKFILL_STATUS_OK);
    if deadline_preserved {
        STARTUP_BACKFILL_SCHEDULER.record_next_due(task, startup_backfill_progress_due(&progress));
        info!(
            task = task.log_label(),
            wake_reason,
            deadline_preserved,
            retry_generation = progress.zero_update_streak,
            "kept account activity v2 coverage repair on its active follow-up deadline"
        );
        return Ok(progress.wake_generation);
    }

    let wake_result = sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation
        )
        VALUES (?1, 0, NULL, 0, NULL, NULL, 0, 0, ?2, NULL, NULL, 1)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = CASE
                WHEN startup_backfill_progress.zero_update_streak > 0
                    THEN startup_backfill_progress.next_run_after
                ELSE NULL
            END,
            last_status = CASE
                WHEN startup_backfill_progress.zero_update_streak > 0
                    THEN startup_backfill_progress.last_status
                ELSE ?2
            END,
            wake_generation = startup_backfill_progress.wake_generation + 1
        "#,
    )
    .bind(task_name)
    .bind(STARTUP_BACKFILL_STATUS_IDLE)
    .execute(pool)
    .await;
    if let Err(err) = wake_result {
        let err = anyhow::Error::from(err);
        if startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err).is_some() {
            return Ok(0);
        }
        return Err(err);
    }

    let progress = match load_startup_backfill_progress(pool, task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            if startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err).is_some()
            {
                return Ok(0);
            }
            return Err(err);
        }
    };
    STARTUP_BACKFILL_SCHEDULER.wake(task);
    info!(
        task = task.log_label(),
        wake_reason,
        deadline_preserved,
        retry_generation = progress.zero_update_streak,
        "woke account activity v2 coverage repair"
    );
    Ok(progress.wake_generation)
}

fn startup_backfill_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
    // The dedicated coverage task owns the active-window planner and its retry
    // deadline. Generic backfill refreshes must never re-enter that planner.
    HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
}

async fn run_startup_backfill_coverage_repair_if_due(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<StartupBackfillTaskRunOutcome> {
    run_startup_backfill_coverage_repair_if_due_with_repair(state, gate, || {
        repair_active_account_activity_v2_coverage(&state.pool)
    })
    .await
}

pub(crate) async fn run_startup_backfill_coverage_repair_if_due_with_repair<Repair, RepairFuture>(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
    repair: Repair,
) -> Result<StartupBackfillTaskRunOutcome>
where
    Repair: FnOnce() -> RepairFuture,
    RepairFuture: Future<Output = Result<ActiveAccountActivityV2RepairOutcome>>,
{
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    // Coverage repair owns this one permit. The hourly-rollup convenience wrapper also acquires
    // the global gate, so calling it while the startup path holds a permit would always defer in
    // production. Keep admission before progress access, then call the underlying repair once.
    let pressure_defer = STARTUP_BACKFILL_SCHEDULER.pressure_deferred_entry(task);
    let _permit = match gate.try_begin_background("startup_backfill_account_activity_v2_coverage") {
        Ok(permit) => permit,
        Err(_) => return Ok(startup_backfill_pressure_defer_outcome(task, gate)),
    };
    STARTUP_BACKFILL_SCHEDULER.clear_pressure_deferred_if_matches(task, pressure_defer);

    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                return Ok(outcome);
            }
            return Err(err);
        }
    };
    let now = Utc::now();
    if !progress.is_due(now) {
        debug!(
            task = task.log_label(),
            task_name = %progress.task_name,
            next_run_after = progress.next_run_after.as_deref().unwrap_or("-"),
            last_status = %progress.last_status,
            last_started_at = progress.last_started_at.as_deref().unwrap_or("-"),
            last_finished_at = progress.last_finished_at.as_deref().unwrap_or("-"),
            last_scanned = progress.last_scanned,
            last_updated = progress.last_updated,
            "startup backfill task is not due"
        );
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: false,
            next_due: startup_backfill_progress_due(&progress),
        });
    }

    let repair_outcome = {
        let _guard = state.hourly_rollup_sync_lock.lock().await;
        repair().await
    };
    let repair_outcome = match repair_outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            warn!(
                task = task.log_label(),
                error = %err,
                wake_reason = "active_window_coverage_check",
                "startup backfill account activity v2 coverage repair failed"
            );
            let next_due = match defer_startup_backfill_coverage_repair(state.as_ref()).await {
                Ok(next_due) => next_due,
                Err(persist_err) => {
                    // A retry-progress lock is still a pressure defer, even though the durable
                    // retry deadline could not be written. Prefer that error so two locks close
                    // the gate once, then leave re-dispatch to the in-memory eligibility wake.
                    if let Some(outcome) = startup_backfill_pressure_error_defer_outcome_if_recorded(
                        task,
                        gate,
                        &persist_err,
                    ) {
                        return Ok(outcome);
                    }
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(persist_err);
                }
            };
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                return Ok(outcome);
            }
            return Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: true,
                deferred: false,
                completed: true,
                next_due,
            });
        }
    };

    match repair_outcome {
        outcome if outcome.repaired_bucket_count > 0 => {
            let next_due =
                match record_startup_backfill_coverage_repair_progress(state.as_ref(), outcome)
                    .await
                {
                    Ok(next_due) => next_due,
                    Err(err) => {
                        if let Some(outcome) =
                            startup_backfill_pressure_error_defer_outcome_if_recorded(
                                task, gate, &err,
                            )
                        {
                            return Ok(outcome);
                        }
                        return Err(err);
                    }
                };
            Ok(StartupBackfillTaskRunOutcome {
                actionable: true,
                failed: false,
                deferred: false,
                completed: true,
                next_due,
            })
        }
        outcome if outcome.priority_bucket_count > 0 => {
            let next_due = match defer_startup_backfill_coverage_repair(state.as_ref()).await {
                Ok(next_due) => next_due,
                Err(err) => {
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(err);
                }
            };
            Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: true,
                completed: true,
                next_due,
            })
        }
        _ => {
            let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
            let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
                Ok(progress) => progress,
                Err(err) => {
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(err);
                }
            };
            let next_retry_after = format_utc_iso(
                Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_IDLE_INTERVAL_SECS as i64),
            );
            let save_result = save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: progress.cursor_id,
                    scanned: progress.last_scanned,
                    updated: progress.last_updated,
                    zero_update_streak: 0,
                    next_run_after: &next_retry_after,
                    status: STARTUP_BACKFILL_STATUS_IDLE,
                    suspension_reason: None,
                },
            )
            .await;
            if let Err(err) = save_result {
                if let Some(outcome) =
                    startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                {
                    return Ok(outcome);
                }
                return Err(err);
            }
            let next_due = parse_to_utc_datetime(&next_retry_after).unwrap_or_else(Utc::now);
            STARTUP_BACKFILL_SCHEDULER.record_next_due(task, next_due);
            debug!(
                task = task.log_label(),
                next_retry_after = %next_retry_after,
                "account activity v2 coverage repair is idle"
            );
            Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: false,
                completed: true,
                next_due,
            })
        }
    }
}

pub(crate) async fn run_startup_backfill_maintenance_pass(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> StartupBackfillMaintenancePass {
    run_startup_backfill_maintenance_pass_with_gate(
        state,
        cancel,
        selected_tasks,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
}

pub(crate) async fn run_startup_backfill_maintenance_pass_with_gate(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    run_startup_backfill_maintenance_pass_with_gate_inner(state, cancel, selected_tasks, gate).await
}

async fn run_startup_backfill_maintenance_pass_with_gate_inner(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    let mut had_failure = false;
    let mut ran_actionable_task = false;
    let mut had_deferred_task = false;
    let tasks = match selected_tasks {
        Some(tasks) => tasks,
        None => StartupBackfillTask::ordered_tasks(),
    };
    for task in tasks {
        if cancel.is_cancelled() {
            info!(
                task = task.log_label(),
                "startup backfill maintenance stopped at a task boundary because shutdown is in progress"
            );
            break;
        }
        if *task == StartupBackfillTask::ProxyUsage && !state.config.proxy_usage_backfill_on_startup
        {
            debug!(
                task = task.log_label(),
                "startup backfill task is disabled by config"
            );
            STARTUP_BACKFILL_SCHEDULER.clear_next_due(*task);
            continue;
        }
        match run_startup_backfill_task_if_due_outcome(&state, *task, gate).await {
            Ok(outcome) => {
                if !outcome.deferred {
                    STARTUP_BACKFILL_SCHEDULER.record_next_due(*task, outcome.next_due);
                }
                ran_actionable_task |= outcome.actionable;
                had_failure |= outcome.failed;
                had_deferred_task |= outcome.deferred;
                if outcome.completed {
                    STARTUP_BACKFILL_SCHEDULER.record_task_result(
                        *task,
                        outcome.failed,
                        outcome.deferred,
                    );
                }
            }
            Err(err) => {
                had_failure = true;
                STARTUP_BACKFILL_SCHEDULER.record_task_result(*task, true, false);
                STARTUP_BACKFILL_SCHEDULER.record_next_due(
                    *task,
                    Utc::now()
                        + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
                );
                warn!(task = task.log_label(), error = %err, "startup backfill supervisor pass failed");
            }
        }
    }

    if ran_actionable_task {
        refresh_hourly_rollups_for_read_surfaces_best_effort(
            &state.pool,
            state.hourly_rollup_sync_lock.as_ref(),
            "startup backfill maintenance pass",
            startup_backfill_hourly_rollup_refresh_scope(),
        )
        .await;
        if !cancel.is_cancelled() {
            let _guard = state.hourly_rollup_sync_lock.lock().await;
            let live_start_epoch =
                shanghai_retention_cutoff(state.config.invocation_max_days).timestamp();
            if let Err(err) =
                maintain_parallel_work_rollups(&state.pool, Some(live_start_epoch)).await
            {
                had_failure = true;
                crate::db_pressure::global_db_pressure_gate()
                    .record_error("parallel_work_rollup_maintenance", &err);
                warn!(error = %err, "parallel-work rollup maintenance pass failed");
            }
        }
    }

    if ran_actionable_task || had_failure {
        let task_run = begin_system_task_run(
            &state.pool,
            SystemTaskKind::StartupBackfill,
            "event_or_due",
            Some("startup backfill maintenance changed data or failed".to_string()),
        )
        .await
        .ok();
        if let Some(run) = task_run.as_ref() {
            finish_system_task_run_batched(
                state.as_ref(),
                run,
                if had_failure {
                    SystemTaskStatus::Failed
                } else {
                    SystemTaskStatus::Success
                },
                Some(if had_failure {
                    "startup backfill maintenance pass completed with failures".to_string()
                } else {
                    "startup backfill maintenance pass completed".to_string()
                }),
                None,
            )
            .await;
        }
    } else if !had_deferred_task {
        // Idle passes deliberately do not write system_task_runs, avoiding an audit workload
        // that would itself wake persistence maintenance.
        STARTUP_BACKFILL_SCHEDULER.record_noop_suppressed();
    }

    StartupBackfillMaintenancePass {
        ran_actionable_task,
        had_failure,
    }
}

pub(crate) fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    match task {
        StartupBackfillTask::ProxyUsage => state.config.proxy_usage_backfill_on_startup,
        _ => true,
    }
}

pub(crate) async fn run_startup_backfill_task_if_due(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> Result<bool> {
    run_startup_backfill_task_if_due_outcome(
        state,
        task,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
    .map(|outcome| outcome.actionable)
}

pub(crate) async fn run_startup_backfill_task_if_due_with_gate(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<bool> {
    run_startup_backfill_task_if_due_outcome(state, task, gate)
        .await
        .map(|outcome| outcome.actionable)
}

async fn run_startup_backfill_task_if_due_outcome(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<StartupBackfillTaskRunOutcome> {
    if !startup_backfill_task_enabled(state.as_ref(), task) {
        debug!(
            task = task.log_label(),
            "startup backfill task is disabled by config"
        );
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: true,
            next_due: Utc::now() + ChronoDuration::days(1),
        });
    }

    if task == StartupBackfillTask::AccountActivityV2Coverage {
        return run_startup_backfill_coverage_repair_if_due(state, gate).await;
    }

    // This admission is deliberately before progress lookup: a pressure defer must remain a
    // scheduler-only decision, not turn into a SQLite read, progress write, or task-run audit.
    let pressure_defer = STARTUP_BACKFILL_SCHEDULER.pressure_deferred_entry(task);
    let _permit = match gate.try_begin_background("startup_backfill") {
        Ok(permit) => permit,
        Err(_) => return Ok(startup_backfill_pressure_defer_outcome(task, gate)),
    };
    STARTUP_BACKFILL_SCHEDULER.clear_pressure_deferred_if_matches(task, pressure_defer);

    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                return Ok(outcome);
            }
            return Err(err);
        }
    };
    let now = Utc::now();
    if !progress.is_due(now) {
        debug!(
            task = task.log_label(),
            task_name = %progress.task_name,
            next_run_after = progress.next_run_after.as_deref().unwrap_or("-"),
            last_status = %progress.last_status,
            last_started_at = progress.last_started_at.as_deref().unwrap_or("-"),
            last_finished_at = progress.last_finished_at.as_deref().unwrap_or("-"),
            last_scanned = progress.last_scanned,
            last_updated = progress.last_updated,
            "startup backfill task is not due"
        );
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: false,
            next_due: startup_backfill_progress_due(&progress),
        });
    }

    if let Err(err) =
        mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id).await
    {
        if let Some(outcome) =
            startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
        {
            return Ok(outcome);
        }
        return Err(err);
    }

    let started_at = Instant::now();
    let outcome = match run_startup_backfill_task(
        state,
        task,
        progress.cursor_id,
        progress.zero_update_streak,
        progress.last_status == STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
    )
    .await
    {
        Ok((run, detail)) => {
            let zero_update_streak = if run.updated == 0 {
                progress.zero_update_streak.saturating_add(1)
            } else {
                0
            };
            let next_cursor_id = if task == StartupBackfillTask::HistoricalRollups {
                run.next_cursor_id
            } else {
                run.next_cursor_id.max(progress.cursor_id)
            };
            let next_run_after = startup_backfill_next_run_after(&run, zero_update_streak);
            if let Err(err) = save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: next_cursor_id,
                    scanned: run.scanned,
                    updated: run.updated,
                    zero_update_streak,
                    next_run_after: &next_run_after,
                    status: if run.source_unavailable {
                        STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
                    } else {
                        STARTUP_BACKFILL_STATUS_OK
                    },
                    suspension_reason: run.source_unavailable.then_some("source_unavailable"),
                },
            )
            .await
            {
                if let Some(outcome) =
                    startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                {
                    return Ok(outcome);
                }
                return Err(err);
            }
            info!(
                task = task.log_label(),
                task_name = %task_name,
                scanned = run.scanned,
                updated = run.updated,
                cursor_id = next_cursor_id,
                hit_scan_limit = run.hit_scan_limit,
                zero_update_streak,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                next_run_after = %next_run_after,
                actionable_backlog_count = u64::from(run.hit_scan_limit),
                blocked_backlog_count = u64::from(run.source_unavailable),
                suspension_reason = if run.source_unavailable { "source_unavailable" } else { "none" },
                probe_budget_exhausted = false,
                backoff_stage = match startup_backfill_next_delay(&run, zero_update_streak).as_secs() {
                    0..=15 => "15s",
                    16..=60 => "1m",
                    61..=300 => "5m",
                    301..=900 => "15m",
                    _ => "idle",
                },
                wake_reason = if run.updated > 0 { "progress" } else if run.source_unavailable { "daily_probe" } else if run.hit_scan_limit { "actionable_backlog" } else { "scheduled" },
                detail = %detail,
                samples = %startup_backfill_samples_text(&run.samples),
                "startup backfill pass finished"
            );
            StartupBackfillTaskRunOutcome {
                actionable: startup_backfill_run_is_actionable(&run),
                failed: false,
                deferred: false,
                completed: true,
                next_due: parse_to_utc_datetime(&next_run_after).unwrap_or_else(Utc::now),
            }
        }
        Err(err) => {
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                return Ok(outcome);
            }
            let next_due = match persist_startup_backfill_task_failure(
                state, task, &task_name, &progress, started_at, &err,
            )
            .await
            {
                Ok(next_due) => next_due,
                Err(persist_err) => {
                    if let Some(outcome) = startup_backfill_pressure_error_defer_outcome_if_recorded(
                        task,
                        gate,
                        &persist_err,
                    ) {
                        return Ok(outcome);
                    }
                    return Err(persist_err);
                }
            };
            StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: true,
                deferred: false,
                completed: true,
                next_due,
            }
        }
    };

    Ok(outcome)
}

pub(crate) async fn persist_startup_backfill_task_failure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    err: &anyhow::Error,
) -> Result<DateTime<Utc>> {
    let failure_kind = startup_backfill_failure_kind(err);
    let retry_after = format_utc_iso(
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    );
    save_startup_backfill_progress(
        &state.pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: 0,
            updated: 0,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await?;
    warn!(
        task = task.log_label(),
        task_name,
        cursor_id = progress.cursor_id,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        next_run_after = %retry_after,
        failure_kind = failure_kind.telemetry_reason(),
        retry_kind = "bounded_operation_backoff",
        error = %err,
        "startup backfill pass failed"
    );
    Ok(parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now))
}

fn startup_backfill_run_is_actionable(run: &StartupBackfillRunState) -> bool {
    run.updated > 0 || (run.hit_scan_limit && !run.force_idle)
}

pub(crate) async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
    _zero_update_streak: u32,
    source_unavailable_probe: bool,
) -> Result<(StartupBackfillRunState, String)> {
    let scan_limit = startup_backfill_scan_limit(source_unavailable_probe);
    let max_elapsed = Some(startup_backfill_run_budget(source_unavailable_probe));
    let raw_path_fallback_root = state.config.database_path.parent();
    match task {
        StartupBackfillTask::ProxyUsage => {
            let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_usage_tokens_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_without_usage={} skipped_decode_error={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_without_usage,
                outcome.summary.skipped_decode_error
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await.clone();
            let attempt_version = pricing_backfill_attempt_version(&catalog);
            let requested_tier_price_version =
                proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
            let response_tier_price_version =
                proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
            let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(
                &state.pool,
                &attempt_version,
                &requested_tier_price_version,
                &response_tier_price_version,
            )
            .await?;
            let outcome = backfill_proxy_missing_costs_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                &catalog,
                &attempt_version,
                &requested_tier_price_version,
                &response_tier_price_version,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_unpriced_model={}",
                outcome.summary.skipped_unpriced_model
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::PromptCacheKey => {
            let outcome = backfill_proxy_prompt_cache_keys_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_key={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_key
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::RequestedServiceTier => {
            let outcome = backfill_proxy_requested_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::InvocationServiceTier => {
            let outcome = backfill_invocation_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file, outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ReasoningEffort => {
            let outcome = backfill_proxy_reasoning_efforts_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_effort={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_effort
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::FailureClassification => {
            let outcome = backfill_failure_classification_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                "failure classification recalculated".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdLive => {
            let outcome = backfill_pool_upstream_request_attempt_public_ids_from_cursor(
                &state.pool,
                cursor_id,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                "attempt_public_id live rows".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdArchives => {
            let outcome =
                backfill_pool_upstream_request_attempt_archive_public_ids_from_batch_cursor(
                    &state.pool,
                    cursor_id,
                    Some(1),
                    max_elapsed,
                )
                .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned_batches,
                    updated: outcome.summary.updated_rows,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                format!(
                    "updated_batches={} updated_rows={}",
                    outcome.summary.updated_batches, outcome.summary.updated_rows
                ),
            ))
        }
        StartupBackfillTask::UpstreamActivityLive => {
            let updated_accounts =
                backfill_upstream_account_last_activity_from_live_invocations(&state.pool).await?;
            let pending_accounts =
                count_upstream_accounts_missing_live_last_activity(&state.pool).await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: 0,
                    updated: updated_accounts,
                    hit_scan_limit: false,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            let summary = backfill_upstream_account_last_activity_from_archives(
                &state.pool,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let pending_accounts =
                count_upstream_accounts_missing_last_activity(&state.pool).await?;
            let force_idle = summary.waiting_for_manifest_backfill
                || (pending_accounts > 0 && !summary.hit_budget && summary.updated_accounts == 0);
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_batches,
                    updated: summary.updated_accounts,
                    hit_scan_limit: pending_accounts > 0 && summary.hit_budget,
                    retry_soon: false,
                    force_idle,
                    source_unavailable: pending_accounts > 0 && force_idle,
                    samples: Vec::new(),
                },
                format!(
                    "pending_accounts={pending_accounts} waiting_for_manifest_backfill={}",
                    summary.waiting_for_manifest_backfill
                ),
            ))
        }
        StartupBackfillTask::PoolUpstreamNodeHealthArchives => {
            let _guard = state.hourly_rollup_sync_lock.lock().await;
            let cache_summary =
                backfill_pool_upstream_node_health_archives(&state.pool, Some(1), max_elapsed)
                    .await?;
            let hourly_summary = backfill_pool_upstream_node_health_hourly_archives(
                &state.pool,
                Some(1),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: cache_summary.scanned_batches + hourly_summary.scanned_batches,
                    updated: cache_summary.cached_rows + hourly_summary.materialized_rows,
                    hit_scan_limit: cache_summary.hit_budget || hourly_summary.hit_budget,
                    retry_soon: false,
                    force_idle: cache_summary.pending_batches == 0
                        && hourly_summary.pending_batches == 0,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!(
                    "pending_cache_batches={} materialized_cache_batches={} pending_hourly_batches={} materialized_hourly_batches={}",
                    cache_summary.pending_batches,
                    cache_summary.materialized_batches,
                    hourly_summary.pending_batches,
                    hourly_summary.materialized_batches,
                ),
            ))
        }
        StartupBackfillTask::AccountActivityV2Coverage => Err(anyhow!(
            "account activity v2 coverage repair must use its dedicated scheduler path"
        )),
        StartupBackfillTask::HistoricalRollups => {
            let window = materialize_historical_rollups_startup_window(
                &state.pool,
                cursor_id,
                Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS),
            )
            .await?;
            let summary = window.summary;
            let updated = window.changed_path_count as u64;
            info!(
                task = StartupBackfillTask::HistoricalRollups.log_label(),
                candidate_limit = 32_u64,
                batch_limit = STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT,
                elapsed_budget_ms =
                    Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS).as_millis() as u64,
                candidate_count = window.candidate_count,
                inspected_path_count = window.inspected_path_count,
                changed_path_count = window.changed_path_count,
                wrapped = window.wrapped,
                blocked_archive_batches = summary.blocked_archive_batches,
                "startup historical rollup keyset pass completed"
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: window.next_cursor_id,
                    scanned: summary.scanned_archive_batches as u64,
                    updated,
                    hit_scan_limit: updated > 0
                        && (window.candidate_count >= 32
                            || summary.scanned_archive_batches
                                >= STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT as usize),
                    retry_soon: historical_rollup_should_retry_soon(
                        window.hit_budget,
                        window.candidate_count,
                    ),
                    force_idle: window.candidate_count == 0,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!(
                    "candidate_count={} inspected_path_count={} changed_path_count={} hit_budget={} wrapped={} next_cursor_id={} materialized_archive_batches={} blocked_archive_batches={}",
                    window.candidate_count,
                    window.inspected_path_count,
                    window.changed_path_count,
                    window.hit_budget,
                    window.wrapped,
                    window.next_cursor_id,
                    summary.materialized_archive_batches,
                    summary.blocked_archive_batches,
                ),
            ))
        }
    }
}

pub(crate) async fn run_startup_persistent_prep_best_effort(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
) -> bool {
    if !should_run_startup_persistent_prep(prep_cli) {
        return true;
    }

    // The individual manifest and expiry writers acquire the maintenance permit themselves.
    // Holding the global background slot here would make those nested admissions reject every
    // startup pass before they can do any work.
    let defer_generation = super::retention::retention_defer_generation();
    match run_startup_persistent_prep_inner(&state.pool, &state.config, prep_cli, false).await {
        Ok(summary) => {
            let deferred = super::retention::retention_defer_generation() != defer_generation;
            info!(
                refreshed_manifest_batches = summary.refreshed_manifest_batches,
                refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                missing_manifest_files = summary.missing_manifest_files,
                backfilled_archive_expiries = summary.backfilled_archive_expiries,
                bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                pending_manifest_batches = summary.pending_manifest_batches,
                deferred,
                "startup background prep finished"
            );
            !deferred && summary.pending_manifest_batches == 0
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("startup_persistent_prep", &err);
            warn!(error = %err, retry_soon = pressure_error, "startup background prep failed");
            !pressure_error
        }
    }
}

pub(crate) async fn run_pressure_eligible_startup_backfill_tasks(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    gate: &crate::db_pressure::DbPressureGate,
) {
    if cancel.is_cancelled() {
        return;
    }
    let tasks = STARTUP_BACKFILL_SCHEDULER.take_pressure_deferred_tasks_if_eligible(gate);
    if tasks.is_empty() {
        return;
    }
    debug!(
        task_count = tasks.len(),
        "pressure eligibility changed; dispatching deferred startup backfill tasks"
    );
    run_startup_backfill_maintenance_pass_with_gate_inner(state, cancel, Some(&tasks), gate).await;
}

pub(crate) fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        let prep_cli = CliArgs::default();
        let mut startup_prep_pending =
            !run_startup_persistent_prep_best_effort(&state, &prep_cli).await;
        let mut startup_prep_retry_at = startup_prep_pending
            .then(|| Instant::now() + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;
        // Register before either P2 supervisor is scheduled so long-term pruning cannot
        // reclaim a terminal event ahead of the minute projection consumer.
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        spawn_long_term_projection_supervisor(state.clone(), cancel.clone());
        spawn_timeseries_minute_projection_supervisor(state.clone(), cancel.clone());

        let mut observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();

        loop {
            let gate = crate::db_pressure::global_db_pressure_gate();
            // Re-check before registering the next notification waiter. A permit release that
            // raced with the previous pass has already advanced the gate generation, so this
            // consumes its deferred task instead of treating that generation as the baseline.
            run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, gate).await;
            let observed_pressure_eligibility = gate.eligibility_generation();
            // Close the release-between-probe-and-snapshot race. A release before this pass is
            // consumed here; a release after it changes the generation used by the waiter below.
            run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, gate).await;
            let next_due = STARTUP_BACKFILL_SCHEDULER.next_due();
            let mut wait_for = startup_backfill_wait_duration(next_due);
            if let Some(retry_at) = startup_prep_retry_at {
                wait_for = wait_for.min(retry_at.saturating_duration_since(Instant::now()));
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = STARTUP_BACKFILL_SCHEDULER.wait_for_wake(observed_generation) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    let tasks = STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if !tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(state.clone(), &cancel, Some(&tasks)).await;
                    }
                }
                _ = gate.wait_for_eligibility_change(observed_pressure_eligibility) => {
                    run_pressure_eligible_startup_backfill_tasks(
                        state.clone(),
                        &cancel,
                        gate,
                    ).await;
                }
                _ = sleep(wait_for) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if startup_prep_pending
                        && startup_prep_retry_at.is_none_or(|retry_at| retry_at <= Instant::now())
                    {
                        startup_prep_pending =
                            !run_startup_persistent_prep_best_effort(&state, &prep_cli).await;
                        startup_prep_retry_at = startup_prep_pending.then(|| {
                            Instant::now()
                                + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
                        });
                    }
                    let due_tasks = STARTUP_BACKFILL_SCHEDULER.drain_due_tasks(Utc::now());
                    if !due_tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(
                            state.clone(),
                            &cancel,
                            Some(&due_tasks),
                        )
                        .await;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod startup_backfill_tests {
    use super::*;

    #[test]
    fn scheduler_health_tracks_wakes_due_work_and_active_outcomes() {
        let scheduler = StartupBackfillScheduler::default();
        scheduler.wake(StartupBackfillTask::HistoricalRollups);
        scheduler.record_noop_suppressed();

        let woken = scheduler.health_snapshot();
        assert_eq!(woken.state, "healthy");
        assert_eq!(woken.wake_count, 1);
        assert_eq!(woken.woken_task_count, 1);

        assert_eq!(
            scheduler.drain_due_tasks(Utc::now()),
            vec![StartupBackfillTask::HistoricalRollups]
        );
        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, true);
        let deferred = scheduler.health_snapshot();
        assert_eq!(deferred.state, "deferred");
        assert_eq!(deferred.due_dispatch_count, 1);
        assert_eq!(deferred.pressure_defer_count, 1);
        assert_eq!(deferred.noop_suppressed_count, 1);

        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);
        assert_eq!(scheduler.health_snapshot().state, "degraded");

        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, false);
        let recovered = scheduler.health_snapshot();
        assert_eq!(recovered.state, "healthy");
        assert_eq!(recovered.failure_count, 1);
        assert_eq!(recovered.failed_task_count, 0);
    }

    #[test]
    fn pressure_defer_uses_the_gate_absolute_deadline() {
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
        gate.record_pressure("test", "forced");
        let expected_deadline = gate
            .pressure_cooldown_deadline_epoch_ms()
            .expect("active pressure cooldown deadline");

        let retry_at = startup_backfill_pressure_eligibility_deadline(
            &gate,
            crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 },
        )
        .expect("pressure cooldown has a durable eligibility deadline");

        assert_eq!(retry_at.timestamp_millis() as u64, expected_deadline);
    }

    #[test]
    fn pressure_defer_deduplicates_the_same_generation_deadline_and_wake() {
        let scheduler = StartupBackfillScheduler::default();
        let task = StartupBackfillTask::ReasoningEffort;
        let deadline = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_750)
            .expect("valid fixed pressure deadline");

        assert!(scheduler.defer_for_pressure(task, Some(deadline), 41));
        assert!(
            !scheduler.defer_for_pressure(task, Some(deadline + ChronoDuration::minutes(1)), 41),
            "the same pressure generation must preserve its first eligibility deadline"
        );
        assert_eq!(
            scheduler.generation(),
            1,
            "one pressure generation must schedule exactly one supervisor wake"
        );
        scheduler.wake(task);
        assert!(
            scheduler.drain_woken_tasks().is_empty(),
            "an input wake during the pending cooldown must not redispatch the task"
        );
        assert!(
            scheduler
                .drain_due_tasks(deadline - ChronoDuration::milliseconds(1))
                .is_empty()
        );
        let waiting = scheduler.health_snapshot();
        assert_eq!(waiting.wake_count, 0);
        assert_eq!(waiting.due_dispatch_count, 0);
        assert_eq!(waiting.pressure_defer_count, 0);
        assert_eq!(waiting.scheduled_task_count, 1);

        assert_eq!(scheduler.drain_due_tasks(deadline), vec![task]);
        scheduler.record_task_result(task, false, true);
        assert!(scheduler.drain_due_tasks(deadline).is_empty());
        let deferred = scheduler.health_snapshot();
        assert_eq!(deferred.wake_count, 0);
        assert_eq!(deferred.due_dispatch_count, 1);
        assert_eq!(deferred.pressure_defer_count, 1);
        assert_eq!(deferred.scheduled_task_count, 0);
        assert_eq!(deferred.deferred_task_count, 1);
    }

    #[test]
    fn stale_busy_defer_cannot_clear_a_newer_pressure_deadline() {
        let scheduler = StartupBackfillScheduler::default();
        let task = StartupBackfillTask::ReasoningEffort;
        let deadline = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_750)
            .expect("valid fixed pressure deadline");

        assert!(scheduler.defer_for_pressure(task, None, 41));
        assert!(scheduler.defer_for_pressure(task, Some(deadline), 42));

        scheduler.clear_pressure_eligibility_deadline(task, 41);
        assert_eq!(
            scheduler.next_due(),
            Some(deadline),
            "a stale busy defer must not remove a newer pressure generation deadline"
        );
    }

    #[tokio::test]
    async fn pressure_deadline_wakes_an_idle_scheduler() {
        let scheduler = Arc::new(StartupBackfillScheduler::default());
        let task = StartupBackfillTask::ReasoningEffort;
        let observed_generation = scheduler.generation();
        let waiting_scheduler = scheduler.clone();
        let waiter = tokio::spawn(async move {
            waiting_scheduler.wait_for_wake(observed_generation).await;
        });
        tokio::task::yield_now().await;

        let deadline = Utc::now() + ChronoDuration::minutes(5);
        assert!(scheduler.defer_for_pressure(task, Some(deadline), 73));
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("a new pressure deadline must wake an idle supervisor")
            .expect("scheduler waiter should not panic");
        assert_eq!(scheduler.next_due(), Some(deadline));
    }

    #[tokio::test]
    async fn pressure_eligibility_notification_keeps_a_closed_cooldown_deferred() {
        let scheduler = StartupBackfillScheduler::default();
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(60),
        ));
        gate.record_pressure("test", "forced");
        let observed_eligibility = gate.eligibility_generation();
        let task = StartupBackfillTask::ReasoningEffort;
        let deadline = Utc::now() + ChronoDuration::minutes(5);

        assert!(scheduler.defer_for_pressure(task, Some(deadline), gate.pressure_generation()));
        scheduler.record_task_result(task, false, true);

        assert!(
            scheduler.drain_due_tasks(Utc::now()).is_empty(),
            "the in-memory fallback deadline must not be due yet"
        );

        gate.notify_background_eligibility();
        tokio::time::timeout(
            Duration::from_secs(1),
            gate.wait_for_eligibility_change(observed_eligibility),
        )
        .await
        .expect("explicit eligibility notification must be observed");
        assert!(
            scheduler
                .take_pressure_deferred_tasks_if_eligible(&gate)
                .is_empty(),
            "an eligibility notification must not consume a task while the cooldown remains closed"
        );
        assert_eq!(scheduler.next_due(), Some(deadline));
    }

    #[test]
    fn background_busy_defer_waits_only_for_a_gate_eligibility_event() {
        let scheduler = StartupBackfillScheduler::default();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
        let task = StartupBackfillTask::ReasoningEffort;

        assert!(scheduler.defer_for_pressure(task, None, gate.pressure_generation()));
        assert_eq!(scheduler.next_due(), None);

        scheduler.record_task_result(task, false, false);
        assert_eq!(
            scheduler.take_pressure_deferred_tasks_if_eligible(&gate),
            vec![task],
            "an older successful pass must not discard a wake registered while it was running"
        );
        assert!(
            scheduler
                .take_pressure_deferred_tasks_if_eligible(&gate)
                .is_empty(),
            "repeated eligibility notifications must not select the same pending task twice"
        );

        scheduler.requeue_pressure_deferred(task);
        assert_eq!(
            scheduler.take_pressure_deferred_tasks_if_eligible(&gate),
            vec![task],
            "a gate denial after selection must leave one recoverable eligibility wake"
        );
    }

    #[test]
    fn background_busy_defer_clears_stale_wakes_and_requeues_after_a_prior_release() {
        let scheduler = StartupBackfillScheduler::default();
        let stale_task = StartupBackfillTask::ReasoningEffort;
        let released_task = StartupBackfillTask::PoolAttemptPublicIdArchives;
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));

        scheduler.wake(stale_task);
        assert!(scheduler.defer_for_pressure(stale_task, None, gate.pressure_generation()));
        assert!(scheduler.drain_woken_tasks().is_empty());
        assert_eq!(scheduler.next_due(), None);

        let held = gate
            .try_begin_background("test_busy_release_before_defer")
            .expect("occupy the only background slot");
        assert!(matches!(
            gate.try_begin_background("test_busy_defer"),
            Err(crate::db_pressure::DbPressureDenyReason::BackgroundBusy)
        ));
        drop(held);

        assert!(
            scheduler
                .defer_for_current_gate(released_task, &gate)
                .is_none(),
            "an already-open gate must schedule the task instead of retaining a busy defer"
        );
        assert_eq!(scheduler.drain_woken_tasks(), vec![released_task]);
        assert_eq!(scheduler.drain_due_tasks(Utc::now()), vec![released_task]);
    }

    #[test]
    fn pressure_cooldown_upgrades_busy_defer_and_preserves_eligibility_deadline() {
        let scheduler = StartupBackfillScheduler::default();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
        let task = StartupBackfillTask::ReasoningEffort;

        assert!(scheduler.defer_for_pressure(task, None, gate.pressure_generation()));
        gate.record_pressure("test", "forced");
        assert!(
            scheduler
                .take_pressure_deferred_tasks_if_eligible(&gate)
                .is_empty(),
            "a newly closed cooldown must keep the deferred task out of SQLite"
        );
        let expected_deadline = gate
            .pressure_cooldown_deadline_epoch_ms()
            .and_then(|deadline_ms| {
                i64::try_from(deadline_ms)
                    .ok()
                    .and_then(DateTime::<Utc>::from_timestamp_millis)
            })
            .expect("pressure cooldown has an absolute eligibility deadline");
        assert_eq!(scheduler.next_due(), Some(expected_deadline));

        scheduler.record_next_due(task, Utc::now());
        assert_eq!(
            scheduler.next_due(),
            Some(expected_deadline),
            "an older successful task result must not overwrite a pending pressure deadline"
        );
    }

    #[test]
    fn sqlite_busy_and_locked_are_actual_backfill_failures() {
        for error in [
            anyhow::anyhow!("database is busy"),
            anyhow::anyhow!("database table is locked"),
        ] {
            assert_eq!(
                startup_backfill_failure_kind(&error),
                StartupBackfillFailureKind::SqliteBusyOrLocked
            );
        }
        assert_eq!(
            startup_backfill_failure_kind(&anyhow::anyhow!("source unavailable")),
            StartupBackfillFailureKind::Operation
        );

        let scheduler = StartupBackfillScheduler::default();
        scheduler.record_task_result(StartupBackfillTask::ReasoningEffort, true, false);
        let health = scheduler.health_snapshot();
        assert_eq!(health.state, "degraded");
        assert_eq!(health.failure_count, 1);
        assert_eq!(health.pressure_defer_count, 0);
        assert_eq!(health.failed_task_count, 1);
        assert_eq!(health.deferred_task_count, 0);
    }

    #[test]
    fn coverage_repair_health_is_independent_from_historical_rollups() {
        let scheduler = StartupBackfillScheduler::default();
        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);

        scheduler.record_task_result(StartupBackfillTask::AccountActivityV2Coverage, false, true);

        let health = scheduler.health_snapshot();
        assert_eq!(health.state, "degraded");
        assert_eq!(health.failed_task_count, 1);
        assert_eq!(health.pressure_defer_count, 1);
    }

    #[test]
    fn coverage_repair_does_not_repeat_its_planner_in_the_following_hourly_refresh() {
        assert_eq!(
            startup_backfill_hourly_rollup_refresh_scope(),
            HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
        );
    }

    #[test]
    fn actionable_no_progress_backoff_caps_at_fifteen_minutes() {
        let run = StartupBackfillRunState {
            scanned: 2,
            updated: 0,
            hit_scan_limit: true,
            ..StartupBackfillRunState::default()
        };
        assert_eq!(
            startup_backfill_next_delay(&run, 1),
            Duration::from_secs(15)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 2),
            Duration::from_secs(60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 3),
            Duration::from_secs(5 * 60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 4),
            Duration::from_secs(15 * 60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 99),
            Duration::from_secs(15 * 60)
        );
    }

    #[test]
    fn historical_rollup_cursor_advance_after_budget_exhaustion_retries_without_a_task_run() {
        let run = StartupBackfillRunState {
            next_cursor_id: 12,
            scanned: 1,
            retry_soon: true,
            ..StartupBackfillRunState::default()
        };

        assert_eq!(
            startup_backfill_next_delay(&run, 1),
            Duration::from_secs(15)
        );
        assert!(!startup_backfill_run_is_actionable(&run));
    }

    #[test]
    fn historical_rollup_budget_retry_stays_short_after_cursor_wrap() {
        let retry_soon = historical_rollup_should_retry_soon(true, 1);
        assert!(retry_soon);
        assert!(historical_rollup_should_retry_soon(true, 32));
        assert!(!historical_rollup_should_retry_soon(true, 0));
        assert!(!historical_rollup_should_retry_soon(false, 1));

        let run = StartupBackfillRunState {
            retry_soon,
            ..StartupBackfillRunState::default()
        };
        assert_eq!(
            startup_backfill_next_delay(&run, 0),
            Duration::from_secs(15)
        );
        assert!(!startup_backfill_run_is_actionable(&run));
    }

    #[test]
    fn overdue_backfill_deadline_runs_without_an_idle_sleep() {
        assert_eq!(
            startup_backfill_wait_duration(Some(Utc::now() - ChronoDuration::seconds(1))),
            Duration::ZERO
        );
    }

    #[test]
    fn scheduler_drains_only_tasks_with_an_expired_deadline() {
        let scheduler = StartupBackfillScheduler::default();
        let future_due = Utc::now() + ChronoDuration::hours(1);
        scheduler.record_next_due(
            StartupBackfillTask::HistoricalRollups,
            Utc::now() - ChronoDuration::seconds(1),
        );
        scheduler.record_next_due(StartupBackfillTask::ReasoningEffort, future_due);

        assert_eq!(
            scheduler.drain_due_tasks(Utc::now()),
            vec![StartupBackfillTask::HistoricalRollups]
        );
        assert_eq!(scheduler.next_due(), Some(future_due));
    }

    #[test]
    fn only_progress_or_non_idle_backlog_triggers_rollup_refresh() {
        assert!(startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                updated: 1,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                hit_scan_limit: true,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(!startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                scanned: 1,
                force_idle: true,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(!startup_backfill_run_is_actionable(
            &StartupBackfillRunState::default()
        ));
    }

    #[test]
    fn terminal_payload_input_wakes_only_missing_field_repairs() {
        let mut record = crate::tests::test_proxy_capture_record(
            "startup-backfill-terminal-wake",
            "2026-08-09 12:00:00",
        );
        record.usage.total_tokens = None;
        record.cost = None;
        record.payload = Some("{}".to_string());
        record.req_raw.path = Some("/tmp/request.raw".to_string());
        record.resp_raw.path = Some("/tmp/response.raw".to_string());

        let tasks =
            startup_backfill_tasks_for_terminal(&api_invocation_from_runtime_record(&record));

        assert_eq!(
            tasks,
            vec![
                StartupBackfillTask::ProxyUsage,
                StartupBackfillTask::PromptCacheKey,
                StartupBackfillTask::RequestedServiceTier,
                StartupBackfillTask::ReasoningEffort,
                StartupBackfillTask::InvocationServiceTier,
            ]
        );

        let complete =
            api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
                "startup-backfill-terminal-complete",
                "2026-08-09 12:01:00",
            ));
        assert!(startup_backfill_tasks_for_terminal(&complete).is_empty());
    }

    #[test]
    fn source_unavailable_probe_uses_one_shared_budget() {
        assert_eq!(startup_backfill_scan_limit(true), 100);
        assert_eq!(startup_backfill_run_budget(true), Duration::from_secs(2));
        assert_eq!(
            startup_backfill_scan_limit(false),
            STARTUP_BACKFILL_SCAN_LIMIT
        );
        assert_eq!(
            startup_backfill_run_budget(false),
            Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)
        );
    }

    #[test]
    fn historical_rollup_backfill_run_state_backs_off_when_only_blocked_archives_remain() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 2,
            legacy_archive_pending: 1,
            pending_usage_breakdown_batches: 1,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(7, 0, &before, &after, &summary, 1, 1);

        assert_eq!(run.next_cursor_id, 8);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 0);
        assert!(!run.hit_scan_limit);
        assert!(run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_stays_active_while_catching_up() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 3,
            pending_usage_breakdown_batches: 3,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = HistoricalRollupBackfillSnapshot {
            pending_buckets: 4,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Warn,
        };
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            materialized_archive_batches: 1,
            materialized_invocation_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(11, 0, &before, &after, &summary, 3, 2);

        assert_eq!(run.next_cursor_id, 12);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 4);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_stays_active_when_partial_scan_found_only_blocked_work()
    {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 3,
            pending_usage_breakdown_batches: 3,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(5, 0, &before, &after, &summary, 3, 3);

        assert_eq!(run.next_cursor_id, 6);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 0);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_does_not_back_off_when_only_blocked_archive_was_after_skip()
     {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 2,
            skipped_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(9, 0, &before, &after, &summary, 2, 2);

        assert_eq!(run.next_cursor_id, 10);
        assert_eq!(run.scanned, 2);
        assert_eq!(run.updated, 0);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_backs_off_after_blocked_cycle_across_multiple_passes() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 2,
            skipped_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(9, 1, &before, &after, &summary, 2, 2);

        assert_eq!(run.next_cursor_id, 10);
        assert_eq!(run.scanned, 2);
        assert_eq!(run.updated, 0);
        assert!(!run.hit_scan_limit);
        assert!(run.force_idle);
    }
}
