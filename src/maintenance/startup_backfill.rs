use super::*;
use crate::observability::contention::WakeDecision;
mod backoff;
mod coverage_repair;
mod maintenance_loop;
mod observability;
mod progress;
mod task_runner;
mod wake_confirmation;
use coverage_repair::run_startup_backfill_coverage_repair_if_due;
#[cfg(test)]
pub(crate) use coverage_repair::run_startup_backfill_coverage_repair_if_due_with_repair;
pub(crate) use observability::StartupBackfillHealthSnapshot;
use observability::{
    StartupBackfillScheduler, WakeObligationAttempt, WakeObligationKey, WakeObligationTicket,
    WakeObservationTicket, confirm_scheduler_wake_observation, observe_wake_attempt,
    record_scheduler_wake_confirmation,
};
#[cfg(test)]
use wake_confirmation::confirm_startup_backfill_wake_obligation;
use wake_confirmation::confirm_startup_backfill_wake_obligations;
pub(crate) use wake_confirmation::{
    StartupBackfillWakeConfirmation, WakeProgressWriteContext,
    confirm_startup_backfill_progress_write,
};

#[cfg(test)]
pub(crate) async fn confirm_startup_backfill_wake_obligations_for_test(pool: &Pool<Sqlite>) {
    wake_confirmation::confirm_startup_backfill_wake_obligations_for_test(pool).await;
}

#[cfg(test)]
pub(crate) use wake_confirmation::confirm_startup_backfill_wake_obligation_for_test;

mod prompt_cache_control;
mod task_identity;
#[cfg(test)]
pub(crate) use backoff::historical_rollup_startup_backfill_run_state;
pub(crate) use backoff::{
    BackfillBatchOutcome, BackfillPartialFailure, StartupBackfillRunState,
    historical_rollup_should_retry_soon, startup_backfill_budget_reached,
    startup_backfill_next_delay, startup_backfill_next_run_after, startup_backfill_query_limit,
    startup_backfill_run_budget, startup_backfill_samples_text, startup_backfill_scan_limit,
};
pub(crate) use maintenance_loop::{
    StartupBackfillMaintenancePass, defer_startup_backfill_coverage_repair,
    record_startup_backfill_coverage_repair_progress,
    run_startup_backfill_maintenance_pass_managed, spawn_startup_backfill_maintenance,
    wake_startup_backfill_coverage_repair,
};
use maintenance_loop::{
    run_startup_backfill_maintenance_pass_with_gate_held, startup_backfill_wake_decision,
};
pub(crate) use progress::{
    StartupBackfillProgress, StartupBackfillProgressUpdate, load_startup_backfill_progress,
    load_startup_backfill_progress_from_pool, mark_startup_backfill_running,
    save_startup_backfill_progress, set_startup_backfill_progress_enabled,
    startup_backfill_task_progress_key, startup_backfill_task_progress_key_for_catalog,
};
pub(crate) type StartupBackfillProgressRow = progress::StartupBackfillProgressRow;
use progress::{
    save_startup_backfill_progress_for_task, startup_backfill_progress_due,
    startup_backfill_progress_pool,
};
#[cfg(test)]
pub(crate) use prompt_cache_control::run_prompt_cache_materialization_with_control_for_test;
#[cfg(test)]
pub(crate) use prompt_cache_control::wake_prompt_cache_materialization_for_test;
#[cfg(test)]
use prompt_cache_control::{
    apply_prompt_cache_control_schedule, prompt_cache_tasks_when_startup_backfill_root_is_skipped,
    requeue_legacy_startup_backfill_tasks, wake_prompt_cache_materialization_with_scheduler,
};
use prompt_cache_control::{
    confirm_prompt_cache_materialization_wake, coordinator_for_prompt_cache_run,
    legacy_startup_backfill_tasks, persist_prompt_cache_materialization_defer,
    prompt_cache_materialization_failed_outcome, prompt_cache_stale_result_outcome,
    run_background_startup_backfill_pass_with_root_state,
    save_prompt_cache_materialization_progress,
};
pub(crate) use prompt_cache_control::{
    set_prompt_cache_materialization_enabled_with_store,
    wake_prompt_cache_materialization_with_store,
};
use task_identity::{audit_task_kind, observation_parent_task_key, should_record_audit};
use task_runner::run_startup_backfill_task_if_due_outcome;
#[cfg(test)]
pub(crate) use task_runner::run_startup_backfill_task_if_due_with_store_for_test;

const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: u64 = 16;
const STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS: u64 = 6;
const STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT: u64 = 128;
const STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS: u64 = 6;
const COVERAGE_REPAIR_RETRY_DELAYS_SECS: [u64; 4] = [15, 60, 5 * 60, 15 * 60];

pub(crate) async fn defer_startup_backfill_task(
    state: &AppState,
    task: StartupBackfillTask,
    delay: Duration,
    wake_reason: &'static str,
) -> Result<()> {
    maintenance_loop::defer_startup_backfill_task(state, task, delay, wake_reason).await
}

pub(crate) async fn run_startup_persistent_prep_best_effort(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
) -> bool {
    maintenance_loop::run_startup_persistent_prep_best_effort(state, prep_cli).await
}

pub(crate) async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
    zero_update_streak: u32,
    source_unavailable_probe: bool,
) -> Result<(StartupBackfillRunState, String)> {
    task_runner::run_startup_backfill_task(
        state,
        task,
        cursor_id,
        zero_update_streak,
        source_unavailable_probe,
    )
    .await
}

pub(crate) fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    task_runner::startup_backfill_task_enabled(state, task)
}

pub(crate) fn coverage_repair_retry_delay(retry_generation: u32) -> Duration {
    maintenance_loop::coverage_repair_retry_delay(retry_generation)
}

pub(crate) async fn run_startup_backfill_maintenance_pass(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> StartupBackfillMaintenancePass {
    maintenance_loop::run_startup_backfill_maintenance_pass(state, cancel, selected_tasks).await
}

pub(crate) async fn run_startup_backfill_maintenance_pass_with_gate(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    maintenance_loop::run_startup_backfill_maintenance_pass_with_gate(
        state,
        cancel,
        selected_tasks,
        gate,
    )
    .await
}

pub(crate) async fn run_pressure_eligible_startup_backfill_tasks(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    gate: &crate::db_pressure::DbPressureGate,
) {
    maintenance_loop::run_pressure_eligible_startup_backfill_tasks(state, cancel, gate).await
}

pub(crate) async fn save_startup_backfill_progress_to_pool(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    progress::save_startup_backfill_progress_to_pool(pool, task_name, update).await
}

pub(crate) async fn set_startup_backfill_task_enabled(
    pool: &Pool<Sqlite>,
    task: StartupBackfillTask,
    enabled: bool,
) -> Result<StartupBackfillProgress> {
    progress::set_startup_backfill_task_enabled(pool, task, enabled).await
}

pub(crate) async fn run_startup_backfill_task_if_due(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> Result<bool> {
    task_runner::run_startup_backfill_task_if_due(state, task).await
}

pub(crate) async fn run_startup_backfill_task_if_due_with_gate(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<bool> {
    task_runner::run_startup_backfill_task_if_due_with_gate(state, task, gate).await
}

pub(crate) async fn persist_startup_backfill_task_failure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    err: &anyhow::Error,
) -> Result<DateTime<Utc>> {
    task_runner::persist_startup_backfill_task_failure(
        state, task, task_name, progress, started_at, err,
    )
    .await
}

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
    PromptCacheConversationsMaterialization,
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

pub(super) fn requeue_startup_backfill_tasks(
    tasks: &[StartupBackfillTask],
    retry_at: DateTime<Utc>,
) {
    STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(tasks, retry_at);
}

fn startup_backfill_tasks_for_retry(
    selected_tasks: Option<&[StartupBackfillTask]>,
    startup_backfill_root_skipped: bool,
) -> Vec<StartupBackfillTask> {
    selected_tasks.map(ToOwned::to_owned).unwrap_or_else(|| {
        if startup_backfill_root_skipped {
            Vec::new()
        } else {
            StartupBackfillTask::ordered_tasks().to_vec()
        }
    })
}
static STARTUP_BACKFILL_SCHEDULER: Lazy<StartupBackfillScheduler> =
    Lazy::new(StartupBackfillScheduler::default);

pub(crate) fn startup_backfill_health_snapshot() -> StartupBackfillHealthSnapshot {
    STARTUP_BACKFILL_SCHEDULER.health_snapshot()
}

#[cfg(test)]
pub(crate) fn record_startup_backfill_task_outcome_for_test(
    task: StartupBackfillTask,
    outcome: StartupBackfillTaskRunOutcome,
) {
    STARTUP_BACKFILL_SCHEDULER.record_task_result_with_actionable(
        task,
        outcome.failed,
        outcome.deferred,
        outcome.actionable,
    );
}

#[cfg(test)]
pub(crate) fn clear_startup_backfill_task_scheduler_for_test(task: StartupBackfillTask) {
    STARTUP_BACKFILL_SCHEDULER.clear_pending(task);
}

fn startup_backfill_wait_duration(next_due: Option<DateTime<Utc>>) -> Duration {
    match next_due {
        Some(deadline) => (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO),
        None => Duration::from_secs(24 * 60 * 60),
    }
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
    ArchiveLockBusy,
    FilePhaseBusy,
    SqliteBusyOrLocked,
    Operation,
}

#[derive(Clone, Copy)]
struct StartupBackfillResourceDeferMetadata {
    reason: &'static str,
    wake_reason: &'static str,
    message: &'static str,
}

impl StartupBackfillFailureKind {
    fn telemetry_reason(self) -> &'static str {
        match self {
            Self::ArchiveLockBusy => "archive_lock_busy",
            Self::FilePhaseBusy => "file_phase_busy",
            Self::SqliteBusyOrLocked => "sqlite_busy_or_locked",
            Self::Operation => "operation_error",
        }
    }
}
fn startup_backfill_failure_kind(err: &anyhow::Error) -> StartupBackfillFailureKind {
    let has_archive_lock_busy_message = err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("archive directory lock busy")
    });
    if has_archive_lock_busy_message {
        return StartupBackfillFailureKind::ArchiveLockBusy;
    }
    let has_file_phase_busy_message = err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("raw file phase is busy")
    });
    if has_file_phase_busy_message {
        return StartupBackfillFailureKind::FilePhaseBusy;
    }
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
    startup_backfill_pressure_defer_outcome_at(&STARTUP_BACKFILL_SCHEDULER, task, reason, retry_at)
}

fn startup_backfill_pressure_defer_outcome_at(
    scheduler: &StartupBackfillScheduler,
    task: StartupBackfillTask,
    reason: crate::db_pressure::DbPressureDenyReason,
    retry_at: DateTime<Utc>,
) -> StartupBackfillTaskRunOutcome {
    if !crate::maintenance::maintenance_execution_is_manual() {
        scheduler.defer_for_pressure(task, retry_at);
    }
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
fn startup_backfill_coordinator_defer_outcome(
    scheduler: &StartupBackfillScheduler,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillTaskRunOutcome {
    let reason = crate::db_pressure::DbPressureDenyReason::BackgroundBusy;
    let retry_at = startup_backfill_pressure_retry_at(gate, reason);
    if !crate::maintenance::maintenance_execution_is_manual() {
        scheduler.defer_for_background_busy(task, retry_at);
    }
    info!(
        task = task.log_label(),
        reason = %reason,
        defer_kind = "coordinator_priority",
        next_eligibility = %retry_at,
        wake_reason = "coordinator_defer",
        "startup backfill task deferred until the coordinator admits background work"
    );
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    }
}

fn startup_backfill_coordinator_defer_outcome_at(
    scheduler: &StartupBackfillScheduler,
    task: StartupBackfillTask,
    reason: crate::db_pressure::DbPressureDenyReason,
    retry_at: DateTime<Utc>,
) -> StartupBackfillTaskRunOutcome {
    if !crate::maintenance::maintenance_execution_is_manual() {
        scheduler.defer_for_background_busy(task, retry_at);
    }
    info!(
        task = task.log_label(),
        reason = %reason,
        defer_kind = "coordinator_priority",
        next_eligibility = %retry_at,
        wake_reason = "coordinator_defer",
        "startup backfill task yielded for coordinator priority"
    );
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    }
}

fn startup_backfill_defer_outcome_at(
    scheduler: &StartupBackfillScheduler,
    task: StartupBackfillTask,
    reason: crate::db_pressure::DbPressureDenyReason,
    retry_at: DateTime<Utc>,
) -> StartupBackfillTaskRunOutcome {
    if task == StartupBackfillTask::PromptCacheConversationsMaterialization
        && reason == crate::db_pressure::DbPressureDenyReason::BackgroundBusy
    {
        startup_backfill_coordinator_defer_outcome_at(scheduler, task, reason, retry_at)
    } else {
        startup_backfill_pressure_defer_outcome_at(scheduler, task, reason, retry_at)
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
    save_startup_backfill_progress_for_task(
        &state.pool,
        task,
        task_name,
        progress.wake_generation,
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
    Ok(startup_backfill_defer_outcome_at(
        &STARTUP_BACKFILL_SCHEDULER,
        task,
        reason,
        retry_at,
    ))
}

async fn persist_startup_backfill_resource_defer(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    metadata: StartupBackfillResourceDeferMetadata,
) -> Result<StartupBackfillTaskRunOutcome> {
    let retry_at =
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64);
    let retry_after = format_utc_iso(retry_at);
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .await;
    save_startup_backfill_progress_for_task(
        &state.pool,
        task,
        task_name,
        progress.wake_generation,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: Some(metadata.reason),
        },
    )
    .await?;
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    info!(
        task = task.log_label(),
        task_name,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        next_run_after = %retry_after,
        defer_reason = metadata.reason,
        wake_reason = metadata.wake_reason,
        metadata.message,
    );
    Ok(StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    })
}

async fn persist_startup_backfill_archive_lock_defer(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
) -> Result<StartupBackfillTaskRunOutcome> {
    persist_startup_backfill_resource_defer(
        state,
        task,
        task_name,
        progress,
        started_at,
        StartupBackfillResourceDeferMetadata {
            reason: "archive_lock_busy",
            wake_reason: "archive_lock_retry",
            message: "startup backfill task deferred because an archive directory lock was busy",
        },
    )
    .await
}

async fn persist_startup_backfill_file_phase_defer(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
) -> Result<StartupBackfillTaskRunOutcome> {
    persist_startup_backfill_resource_defer(
        state,
        task,
        task_name,
        progress,
        started_at,
        StartupBackfillResourceDeferMetadata {
            reason: "file_phase_busy",
            wake_reason: "file_phase_retry",
            message: "startup backfill task deferred because the shared file phase was busy",
        },
    )
    .await
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
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ordered_tasks()
            .iter()
            .copied()
            .find(|task| task.name() == name)
    }

    pub(crate) fn from_managed_key(name: &str) -> Option<Self> {
        Self::ordered_tasks().iter().copied().find(|task| {
            crate::maintenance_store::managed_startup_backfill_suffix(task.name()) == Some(name)
        })
    }

    pub(crate) fn ordered_tasks() -> &'static [Self] {
        &[
            Self::ProxyUsage,
            Self::PromptCacheKey,
            Self::PromptCacheConversationsMaterialization,
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
            Self::PromptCacheConversationsMaterialization => {
                STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION
            }
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
            Self::PromptCacheConversationsMaterialization => {
                "prompt-cache conversation materialization"
            }
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
    if record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|prompt_cache_key| !prompt_cache_key.trim().is_empty())
    {
        tasks.push(StartupBackfillTask::PromptCacheConversationsMaterialization);
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

pub(crate) async fn wake_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
) -> Result<u64> {
    wake_startup_backfill_tasks_with_pricing_catalog(pool, tasks, None, wake_reason).await
}

const MAX_WAKE_OBLIGATION_PROGRESS_KEY_BYTES: usize = 512;
const MAX_WAKE_OBLIGATION_CATALOG_MODELS: usize = 16_384;

fn reject_startup_backfill_notification(
    task: StartupBackfillTask,
    error: anyhow::Error,
) -> anyhow::Error {
    if STARTUP_BACKFILL_SCHEDULER.wake_notification_rejection_should_warn(task, Utc::now()) {
        warn!(
            task = task.log_label(),
            error = %error,
            "startup backfill wake notification rejected"
        );
    }
    error
}

pub(crate) fn notify_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    pricing_catalog: Option<&PricingCatalog>,
    wake_reason: &'static str,
) -> Result<()> {
    // The producer is intentionally memory-only. The pool remains in the signature so terminal
    // callers can use one fixed notification boundary without giving this path database access.
    let _ = pool;
    if tasks.contains(&StartupBackfillTask::ProxyCost) && pricing_catalog.is_none() {
        return Err(reject_startup_backfill_notification(
            StartupBackfillTask::ProxyCost,
            anyhow!(
                "startup backfill proxy cost notification requires the runtime pricing catalog"
            ),
        ));
    }
    let proxy_cost_progress_key = if tasks.contains(&StartupBackfillTask::ProxyCost) {
        let Some(catalog) = pricing_catalog else {
            return Err(reject_startup_backfill_notification(
                StartupBackfillTask::ProxyCost,
                anyhow!(
                    "startup backfill proxy cost notification requires the runtime pricing catalog"
                ),
            ));
        };
        let max_version_bytes = MAX_WAKE_OBLIGATION_PROGRESS_KEY_BYTES
            .saturating_sub(StartupBackfillTask::ProxyCost.name().len() + 2 + 16);
        if catalog.version.len() > max_version_bytes {
            return Err(reject_startup_backfill_notification(
                StartupBackfillTask::ProxyCost,
                anyhow!(
                    "startup backfill proxy cost catalog version exceeds the bounded notification key"
                ),
            ));
        }
        if catalog.models.len() > MAX_WAKE_OBLIGATION_CATALOG_MODELS {
            return Err(reject_startup_backfill_notification(
                StartupBackfillTask::ProxyCost,
                anyhow!(
                    "startup backfill proxy cost catalog exceeds the bounded notification model count"
                ),
            ));
        }
        let progress_key =
            startup_backfill_task_progress_key_for_catalog(StartupBackfillTask::ProxyCost, catalog);
        if progress_key.len() > MAX_WAKE_OBLIGATION_PROGRESS_KEY_BYTES {
            return Err(reject_startup_backfill_notification(
                StartupBackfillTask::ProxyCost,
                anyhow!(
                    "startup backfill proxy cost progress identity exceeds the bounded notification key"
                ),
            ));
        }
        Some(progress_key)
    } else {
        None
    };

    let decision = startup_backfill_wake_decision(wake_reason);
    for task in tasks.iter().copied() {
        let control_snapshot = (task
            == StartupBackfillTask::PromptCacheConversationsMaterialization)
            .then(|| {
                crate::maintenance_store::global()
                    .and_then(|store| store.prompt_cache_materialization_control.snapshot())
            })
            .flatten();
        let control_generation = control_snapshot.map(|snapshot| snapshot.generation);
        let progress_key = (task == StartupBackfillTask::ProxyCost)
            .then(|| proxy_cost_progress_key.clone())
            .flatten();
        let obligation_key = WakeObligationKey {
            task,
            control_generation,
            progress_key,
        };
        if matches!(decision, WakeDecision::Disabled)
            || control_snapshot.is_some_and(|snapshot| !snapshot.enabled)
        {
            if STARTUP_BACKFILL_SCHEDULER.is_stale_wake_obligation_key(&obligation_key) {
                continue;
            }
            let decision = if control_snapshot.is_some_and(|snapshot| !snapshot.enabled) {
                WakeDecision::Disabled
            } else {
                decision
            };
            STARTUP_BACKFILL_SCHEDULER.record_and_cancel_wake_obligation(
                task,
                decision,
                control_generation,
            );
            continue;
        }
        if let Err(error) =
            STARTUP_BACKFILL_SCHEDULER.record_and_accept_wake_obligation(obligation_key, decision)
        {
            return Err(reject_startup_backfill_notification(task, error));
        }
    }
    Ok(())
}

pub(crate) async fn wake_startup_backfill_tasks_with_pricing_catalog(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    pricing_catalog: Option<&PricingCatalog>,
    wake_reason: &'static str,
) -> Result<u64> {
    let mut woken = 0;
    let decision = startup_backfill_wake_decision(wake_reason);
    if tasks.contains(&StartupBackfillTask::PromptCacheConversationsMaterialization)
        && let Some(store) = crate::maintenance_store::global()
    {
        woken += wake_prompt_cache_materialization_with_store(store, wake_reason).await?;
    }
    let generic_tasks = tasks
        .iter()
        .copied()
        .filter(|task| *task != StartupBackfillTask::PromptCacheConversationsMaterialization)
        .collect::<Vec<_>>();
    if generic_tasks.is_empty() {
        return Ok(woken);
    }
    let Some(pool) = startup_backfill_progress_pool(pool) else {
        return Ok(woken);
    };
    let mut proxy_cost_catalog_missing = false;
    for task in &generic_tasks {
        let task_name = match task {
            StartupBackfillTask::ProxyCost => {
                let Some(catalog) = pricing_catalog else {
                    proxy_cost_catalog_missing = true;
                    let _ = STARTUP_BACKFILL_SCHEDULER.record_notification(*task, decision);
                    continue;
                };
                startup_backfill_task_progress_key_for_catalog(*task, catalog)
            }
            _ => task.name().to_string(),
        };
        let Some(managed_suffix) =
            crate::maintenance_store::managed_startup_backfill_suffix(task.name())
        else {
            let _ = STARTUP_BACKFILL_SCHEDULER.record_notification(*task, decision);
            continue;
        };
        let managed_task_key = format!("startup_backfill.{managed_suffix}");
        let managed_tasks_present = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='managed_tasks')",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(0)
            != 0;
        let managed_enabled = if managed_tasks_present {
            sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
                .bind(&managed_task_key)
                .fetch_optional(pool)
                .await
                .with_context(|| {
                    format!("failed to read managed task control for {managed_task_key}")
                })?
        } else {
            None
        };
        let task_disabled = managed_tasks_present && !managed_enabled.unwrap_or(false);
        let ticket = STARTUP_BACKFILL_SCHEDULER.record_notification(
            *task,
            if task_disabled {
                WakeDecision::Disabled
            } else {
                decision
            },
        );
        if task_disabled {
            continue;
        }
        let rows_affected = observe_wake_attempt(*task, async {
            let outcome = crate::maintenance_store::observe_store_write(
                crate::observability::contention::StoreOperation::Wake,
                async {
                    let result = sqlx::query(
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
                    WHERE startup_backfill_progress.enabled != 0
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
                    })?;
                    Ok::<_, anyhow::Error>(result)
                },
            )
            .await
            ?;
            let rows_affected = outcome.rows_affected();
            Ok((rows_affected, rows_affected > 0))
        })
        .await?;
        woken += rows_affected;
        // A completed no-op settles only this observation. It must not discard
        // queued business work or a retry deadline established by another wake.
        if let Some(ticket) = ticket {
            confirm_scheduler_wake_observation(&STARTUP_BACKFILL_SCHEDULER, ticket);
        }
        if rows_affected > 0 {
            STARTUP_BACKFILL_SCHEDULER.wake(*task);
        }
    }
    if !tasks.is_empty() {
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

#[cfg(test)]
mod startup_backfill_tests;

#[cfg(test)]
#[path = "../tests/lightweight/retention_maintenance_controls.rs"]
mod retention_maintenance_controls;

#[cfg(test)]
#[path = "../tests/lightweight/backfill_observability.rs"]
mod backfill_observability;
