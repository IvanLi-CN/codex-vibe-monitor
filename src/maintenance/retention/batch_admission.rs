use super::*;

#[cfg(test)]
use tokio::sync::Notify;

#[cfg(test)]
#[derive(Default)]
pub(crate) struct BatchCommitProbe {
    pub(crate) committed: Notify,
    pub(crate) resume: Notify,
    pub(crate) force_commit_busy_timeout_zero: std::sync::atomic::AtomicBool,
    pub(crate) commit_receipt: Notify,
    pub(crate) resume_commit_receipt: Notify,
    pub(crate) pause_after_commit_before_record: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct ArchiveArtifactProbe {
    pub(crate) verified: Notify,
    pub(crate) resume: Notify,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct ArchiveIdentityProofProbe {
    pub(crate) started: Notify,
    pub(crate) resume: Notify,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct ArchiveFileVerifyProbe {
    pub(crate) started: Notify,
    pub(crate) resume: Notify,
    pub(crate) paused: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TransactionBeginProbe {
    pub(crate) begun: Notify,
    pub(crate) resume: Notify,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct GateGenerationProbe {
    pub(crate) captured: Notify,
    pub(crate) resume: Notify,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_BATCH_COMMIT: Arc<BatchCommitProbe>;
    pub(crate) static RETENTION_TEST_ARCHIVE_ARTIFACT: Arc<ArchiveArtifactProbe>;
    pub(crate) static RETENTION_TEST_ARCHIVE_IDENTITY_PROOF: Arc<ArchiveIdentityProofProbe>;
    pub(crate) static RETENTION_TEST_ARCHIVE_FILE_VERIFY: Arc<ArchiveFileVerifyProbe>;
    pub(crate) static RETENTION_TEST_TRANSACTION_BEGIN: Arc<TransactionBeginProbe>;
    pub(crate) static RETENTION_TEST_GATE_GENERATION: Arc<GateGenerationProbe>;
}

#[cfg(test)]
pub(super) async fn pause_after_first_chunk(committed_rows: usize) {
    if committed_rows == RETENTION_WRITE_MAX_ROWS
        && let Ok(probe) = RETENTION_TEST_BATCH_COMMIT.try_with(Arc::clone)
    {
        probe.committed.notify_one();
        probe.resume.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn prepare_commit_for_test(connection: &mut SqliteConnection) {
    let Ok(probe) = RETENTION_TEST_BATCH_COMMIT.try_with(Arc::clone) else {
        return;
    };
    if probe
        .force_commit_busy_timeout_zero
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        sqlx::query("PRAGMA busy_timeout=0")
            .execute(connection)
            .await
            .expect("set deterministic commit lock timeout");
    }
}

#[cfg(test)]
pub(super) async fn pause_after_commit_before_record() {
    let Ok(probe) = RETENTION_TEST_BATCH_COMMIT.try_with(Arc::clone) else {
        return;
    };
    if probe
        .pause_after_commit_before_record
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        probe.commit_receipt.notify_one();
        probe.resume_commit_receipt.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn pause_after_archive_artifact_verified() {
    if let Ok(probe) = RETENTION_TEST_ARCHIVE_ARTIFACT.try_with(Arc::clone) {
        probe.verified.notify_one();
        probe.resume.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn pause_before_archive_identity_proof_inflate() {
    if let Ok(probe) = RETENTION_TEST_ARCHIVE_IDENTITY_PROOF.try_with(Arc::clone) {
        probe.started.notify_one();
        probe.resume.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn pause_before_archive_file_verify() {
    if let Ok(probe) = RETENTION_TEST_ARCHIVE_FILE_VERIFY.try_with(Arc::clone) {
        if probe.paused.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        probe.started.notify_one();
        probe.resume.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn pause_after_transaction_begin() {
    if let Ok(probe) = RETENTION_TEST_TRANSACTION_BEGIN.try_with(Arc::clone) {
        probe.begun.notify_one();
        probe.resume.notified().await;
    }
}

#[cfg(test)]
pub(super) async fn pause_after_gate_generation() {
    if let Ok(probe) = RETENTION_TEST_GATE_GENERATION.try_with(Arc::clone) {
        probe.captured.notify_one();
        probe.resume.notified().await;
    }
}

pub(crate) async fn acquire_retention_batch_write_connection(
    pool: &Pool<Sqlite>,
    operation: &'static str,
) -> Result<Option<(sqlx::pool::PoolConnection<Sqlite>, RetentionWriteAdmission)>> {
    acquire_write_connection(pool, operation, true).await
}

pub(crate) async fn acquire_write_connection_with_archive_fence(
    pool: &Pool<Sqlite>,
    operation: &'static str,
    archive_path: &Path,
    expected_sha256: Option<&str>,
) -> Result<
    Option<(
        sqlx::pool::PoolConnection<Sqlite>,
        RetentionWriteAdmission,
        RetentionArchiveFileLock,
    )>,
> {
    acquire_write_connection_with_archive_fence_observed(
        pool,
        operation,
        archive_path,
        expected_sha256,
        || None,
    )
    .await
}

pub(crate) async fn acquire_write_connection_with_archive_fence_observed<F>(
    pool: &Pool<Sqlite>,
    operation: &'static str,
    archive_path: &Path,
    expected_sha256: Option<&str>,
    mut observe_file_verify: F,
) -> Result<
    Option<(
        sqlx::pool::PoolConnection<Sqlite>,
        RetentionWriteAdmission,
        RetentionArchiveFileLock,
    )>,
>
where
    F: FnMut() -> Option<super::observability::RetentionPhaseGuard>,
{
    #[cfg(test)]
    let test_gate = RETENTION_TEST_DB_PRESSURE_GATE.try_with(Arc::clone).ok();
    #[cfg(not(test))]
    let test_gate: Option<Arc<crate::db_pressure::DbPressureGate>> = None;
    let gate = test_gate
        .as_deref()
        .unwrap_or_else(|| crate::db_pressure::global_db_pressure_gate());
    let requested_at = Instant::now();
    let pressure_retry_started = Instant::now();
    let mut pressure_retry_attempts = 0;

    loop {
        // Confirm pool readiness before taking the archive fence. This readiness notification is
        // also the synchronization point used by the admission tests to inject competition.
        let Some(mut connection) = acquire_retention_pool_connection(pool, operation).await? else {
            return Ok(None);
        };
        let release = connection.return_to_pool();
        let release = async {
            if let Some(remaining) = retention_run_remaining_budget() {
                tokio::time::timeout(remaining, release).await.is_ok()
            } else {
                release.await;
                true
            }
        };
        let returned = if let Some(shutdown) = retention_run_shutdown_token() {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    retention_record_defer(operation, "shutdown");
                    return Ok(None);
                }
                returned = release => returned,
            }
        } else {
            release.await
        };
        drop(connection);
        if !returned {
            retention_record_defer(operation, "retention_work_budget");
            return Ok(None);
        }
        #[cfg(test)]
        let _ = RETENTION_TEST_WRITE_CONNECTION_POOL_READY.try_with(|ready| ready.notify_one());

        // A known pressure denial is handled without taking the file fence. The background-busy
        // path waits through the normal admission loop, which keeps the fence out of the wait.
        let gate_generation = gate.eligibility_generation();
        #[cfg(test)]
        pause_after_gate_generation().await;
        if let Some(reason) = gate.background_deny_reason() {
            match reason {
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                    if let Some(defer_reason) = wait_for_pressure_cooldown_retry(
                        gate,
                        operation,
                        pressure_retry_started,
                        &mut pressure_retry_attempts,
                    )
                    .await
                    {
                        retention_record_defer(operation, defer_reason);
                        return Ok(None);
                    }
                    continue;
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                    if !wait_for_background_change(gate, operation, gate_generation).await {
                        return Ok(None);
                    }
                    continue;
                }
            }
        }
        if retention_run_budget_expired()
            || retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled())
        {
            retention_record_defer(
                operation,
                if retention_run_budget_expired() {
                    "retention_work_budget"
                } else {
                    "shutdown"
                },
            );
            return Ok(None);
        }

        // The shared file permit and directory fence cover only artifact verification. If either
        // resource is unavailable, release both before waiting for database admission.
        let file_phase = acquire_file_phase_permit(operation).await?;
        let archive_lock = retention_archive_file_try_lock(archive_path)?;
        #[cfg(test)]
        pause_before_archive_file_verify().await;
        let file_verify_phase = observe_file_verify();
        let actual_sha256 = expected_sha256
            .map(|_| {
                sha256_hex_file_with_deadline(
                    archive_path,
                    retention_archive_file_operation_deadline(),
                )
            })
            .transpose()?;
        let target_exists = if expected_sha256.is_none() {
            match fs::symlink_metadata(archive_path) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            }
        } else {
            false
        };
        drop(file_phase);
        drop(file_verify_phase);
        if expected_sha256
            .zip(actual_sha256)
            .is_some_and(|(expected, actual)| expected != actual)
        {
            bail!("archive artifact changed while awaiting retention admission; retry required");
        }
        if target_exists {
            bail!("archive target exists while awaiting retention admission");
        }
        match try_acquire_write_admission(operation, gate, requested_at).await {
            Ok(mut admission) => {
                if retention_run_budget_expired()
                    || retention_run_shutdown_token()
                        .is_some_and(|shutdown| shutdown.is_cancelled())
                {
                    admission.write_permit.revoke_fairness_admission();
                    drop(admission);
                    drop(archive_lock);
                    retention_record_defer(
                        operation,
                        if retention_run_budget_expired() {
                            "retention_work_budget"
                        } else {
                            "shutdown"
                        },
                    );
                    return Ok(None);
                }
                if let Some(connection) = pool.try_acquire() {
                    return Ok(Some((connection, admission, archive_lock)));
                }
                admission.write_permit.revoke_fairness_admission();
                drop(admission);
                drop(archive_lock);
                continue;
            }
            Err(RetentionWriteAdmissionRejection::GateDenied { reason, generation }) => {
                drop(archive_lock);
                match reason {
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                        if let Some(defer_reason) = wait_for_pressure_cooldown_retry(
                            gate,
                            operation,
                            pressure_retry_started,
                            &mut pressure_retry_attempts,
                        )
                        .await
                        {
                            retention_record_defer(operation, defer_reason);
                            return Ok(None);
                        }
                        continue;
                    }
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                        if !wait_for_background_change(gate, operation, generation).await {
                            return Ok(None);
                        }
                        continue;
                    }
                }
            }
            Err(RetentionWriteAdmissionRejection::CoordinatorBusy) => {
                drop(archive_lock);
                if !wait_for_maintenance_eligibility(operation, requested_at).await {
                    return Ok(None);
                }
            }
            Err(RetentionWriteAdmissionRejection::BudgetExpired) => {
                drop(archive_lock);
                retention_record_defer(operation, "retention_work_budget");
                return Ok(None);
            }
        }
    }
}

enum RetentionWriteAdmissionRejection {
    CoordinatorBusy,
    GateDenied {
        reason: crate::db_pressure::DbPressureDenyReason,
        generation: u64,
    },
    BudgetExpired,
}

async fn try_acquire_write_admission(
    operation: &'static str,
    gate: &crate::db_pressure::DbPressureGate,
    requested_at: Instant,
) -> Result<RetentionWriteAdmission, RetentionWriteAdmissionRejection> {
    if retention_run_budget_expired() {
        return Err(RetentionWriteAdmissionRejection::BudgetExpired);
    }
    let coordinator = retention_write_coordinator_handle();
    let mut write_permit = coordinator
        .try_acquire_maintenance(requested_at, RETENTION_FAIRNESS_INTERVAL)
        .ok_or(RetentionWriteAdmissionRejection::CoordinatorBusy)?;
    if retention_run_budget_expired() {
        drop(write_permit);
        return Err(RetentionWriteAdmissionRejection::BudgetExpired);
    }
    let generation = gate.eligibility_generation();
    match gate.try_begin_background(operation) {
        Ok(pressure_permit) => {
            let coordinator_snapshot = coordinator.snapshot().await;
            crate::task_runtime_observation::mark_managed_maintenance_work_started();
            Ok(RetentionWriteAdmission {
                write_permit,
                _pressure_permit: pressure_permit,
                p1_waiter_count: coordinator_snapshot.p1_waiter_count,
                _write_hold_observation: observability::observe_write_admission(operation),
            })
        }
        Err(reason) => {
            write_permit.revoke_fairness_admission();
            if matches!(
                reason,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
            ) {
                write_permit.suppress_background_eligibility_wakeup();
            }
            drop(write_permit);
            Err(RetentionWriteAdmissionRejection::GateDenied { reason, generation })
        }
    }
}

async fn wait_for_maintenance_eligibility(operation: &'static str, requested_at: Instant) -> bool {
    if retention_run_budget_expired()
        || retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled())
    {
        retention_record_defer(
            operation,
            if retention_run_budget_expired() {
                "retention_work_budget"
            } else {
                "shutdown"
            },
        );
        return false;
    }
    let coordinator = retention_write_coordinator_handle();
    let eligibility =
        coordinator.wait_for_maintenance_eligibility(requested_at, RETENTION_FAIRNESS_INTERVAL);
    let (admission_budget, timeout_reason) = retention_write_admission_budget();
    let eligibility = tokio::time::timeout(admission_budget, eligibility);
    let waited = if let Some(shutdown) = retention_run_shutdown_token() {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                retention_record_defer(operation, "shutdown");
                return false;
            }
            result = eligibility => result.is_ok(),
        }
    } else {
        eligibility.await.is_ok()
    };
    if !waited {
        retention_record_defer(operation, timeout_reason);
    }
    waited
}

async fn wait_for_pressure_cooldown_retry(
    gate: &crate::db_pressure::DbPressureGate,
    operation: &'static str,
    retry_started: Instant,
    retry_attempts: &mut usize,
) -> Option<&'static str> {
    let Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms }) =
        gate.background_deny_reason()
    else {
        return None;
    };
    if *retry_attempts >= RETENTION_PRESSURE_RETRY_MAX_ATTEMPTS
        || retry_started.elapsed() >= RETENTION_PRESSURE_RETRY_BUDGET
    {
        return Some("pressure_cooldown");
    }

    let mut wait = Duration::from_millis(remaining_ms.saturating_add(50));
    wait = wait.min(RETENTION_PRESSURE_RETRY_BUDGET.saturating_sub(retry_started.elapsed()));
    if let Some(remaining) = retention_run_remaining_budget() {
        wait = wait.min(remaining);
    }
    if wait.is_zero() {
        return Some(if retention_run_budget_expired() {
            "retention_work_budget"
        } else {
            "pressure_cooldown"
        });
    }

    *retry_attempts += 1;
    tracing::debug!(
        operation,
        attempt = *retry_attempts,
        max_attempts = RETENTION_PRESSURE_RETRY_MAX_ATTEMPTS,
        wait_ms = wait.as_millis() as u64,
        "retention admission retry waiting for SQLite pressure cooldown"
    );
    let sleep = tokio::time::sleep(wait);
    if let Some(shutdown) = retention_run_shutdown_token() {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                return Some("shutdown");
            }
            _ = sleep => {}
        }
    } else {
        sleep.await;
    }
    if retention_run_budget_expired() {
        return Some("retention_work_budget");
    }
    None
}

pub(super) async fn acquire_write_connection(
    pool: &Pool<Sqlite>,
    operation: &'static str,
    wait_for_busy: bool,
) -> Result<Option<(sqlx::pool::PoolConnection<Sqlite>, RetentionWriteAdmission)>> {
    #[cfg(test)]
    let test_gate = RETENTION_TEST_DB_PRESSURE_GATE.try_with(Arc::clone).ok();
    #[cfg(not(test))]
    let test_gate: Option<Arc<crate::db_pressure::DbPressureGate>> = None;
    let gate = test_gate
        .as_deref()
        .unwrap_or_else(|| crate::db_pressure::global_db_pressure_gate());
    let pressure_retry_started = Instant::now();
    let mut pressure_retry_attempts = 0;
    loop {
        if let Some(reason) = gate.background_deny_reason() {
            if matches!(
                reason,
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. }
            ) {
                if let Some(defer_reason) = wait_for_pressure_cooldown_retry(
                    gate,
                    operation,
                    pressure_retry_started,
                    &mut pressure_retry_attempts,
                )
                .await
                {
                    retention_record_defer(operation, defer_reason);
                    return Ok(None);
                }
                continue;
            }
            if !wait_for_busy {
                retention_record_defer(operation, reason);
                return Ok(None);
            }
        }
        // Confirm pool readiness with no admission held, then finish SQLx's asynchronous
        // return before queueing for the coordinator. Never retain one resource for another.
        let Some(mut connection) = acquire_retention_pool_connection(pool, operation).await? else {
            return Ok(None);
        };
        let release = connection.return_to_pool();
        let release = async {
            if let Some(remaining) = retention_run_remaining_budget() {
                tokio::time::timeout(remaining, release).await.is_ok()
            } else {
                release.await;
                true
            }
        };
        let returned = if let Some(shutdown) = retention_run_shutdown_token() {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    retention_record_defer(operation, "shutdown");
                    return Ok(None);
                }
                returned = release => returned,
            }
        } else {
            release.await
        };
        drop(connection);
        if !returned {
            retention_record_defer(operation, "retention_work_budget");
            return Ok(None);
        }
        #[cfg(test)]
        let _ = RETENTION_TEST_WRITE_CONNECTION_POOL_READY.try_with(|ready| ready.notify_one());
        let Some(mut admission) = acquire_write_admission(operation, wait_for_busy).await else {
            return Ok(None);
        };
        if retention_run_budget_expired()
            || retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled())
        {
            admission.write_permit.revoke_fairness_admission();
            drop(admission);
            retention_record_defer(
                operation,
                if retention_run_budget_expired() {
                    "retention_work_budget"
                } else {
                    "shutdown"
                },
            );
            return Ok(None);
        }
        if let Some(connection) = pool.try_acquire() {
            return Ok(Some((connection, admission)));
        }
        admission.write_permit.revoke_fairness_admission();
        drop(admission);
        if !wait_for_busy {
            retention_record_defer(operation, "sqlite_pool_wait");
            return Ok(None);
        }
        // A foreground writer won the readiness race. Recheck pool capacity without a
        // writer permit; a transient race must not abandon the prepared batch.
    }
}

pub(super) async fn acquire_write_admission(
    operation: &'static str,
    wait_for_busy: bool,
) -> Option<RetentionWriteAdmission> {
    #[cfg(test)]
    let test_gate = RETENTION_TEST_DB_PRESSURE_GATE.try_with(Arc::clone).ok();
    #[cfg(not(test))]
    let test_gate: Option<Arc<crate::db_pressure::DbPressureGate>> = None;
    let gate = test_gate
        .as_deref()
        .unwrap_or_else(|| crate::db_pressure::global_db_pressure_gate());
    let nested_pressure_slot = super::retention_db_pressure_slot_held();
    let pressure_retry_started = Instant::now();
    let mut pressure_retry_attempts = 0;
    loop {
        let generation = gate.eligibility_generation();
        if let Some(reason) = gate.background_deny_reason() {
            match reason {
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                    if let Some(defer_reason) = wait_for_pressure_cooldown_retry(
                        gate,
                        operation,
                        pressure_retry_started,
                        &mut pressure_retry_attempts,
                    )
                    .await
                    {
                        retention_record_defer(operation, defer_reason);
                        return None;
                    }
                    continue;
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                    if nested_pressure_slot =>
                {
                    // The enclosing recovery turn already owns the sole pressure slot. It still
                    // needs a fresh SQLite coordinator permit for this write, but must not request
                    // a second process-wide pressure slot and self-defer.
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                    if wait_for_busy
                        && wait_for_background_change(gate, operation, generation).await
                    {
                        continue;
                    }
                    retention_record_defer(operation, reason);
                    return None;
                }
            }
        }
        if retention_run_budget_expired() {
            retention_record_defer(operation, "retention_work_budget");
            return None;
        }
        let (mut write_permit, coordinator_snapshot) =
            acquire_retention_write_coordinator(operation).await?;
        if retention_run_budget_expired() {
            drop(write_permit);
            retention_record_defer(operation, "retention_work_budget");
            return None;
        }
        let try_generation = gate.eligibility_generation();
        let pressure_permit = if nested_pressure_slot {
            if matches!(
                gate.background_deny_reason(),
                Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. })
            ) {
                write_permit.revoke_fairness_admission();
                drop(write_permit);
                retention_record_defer(operation, "pressure_cooldown");
                return None;
            }
            crate::task_timeline::clear_background_denial(operation);
            Ok(crate::db_pressure::DbBackgroundPermit::nested())
        } else {
            gate.try_begin_background(operation)
        };
        match pressure_permit {
            Ok(pressure_permit) => {
                crate::task_runtime_observation::mark_managed_maintenance_work_started();
                return Some(RetentionWriteAdmission {
                    write_permit,
                    _pressure_permit: pressure_permit,
                    p1_waiter_count: coordinator_snapshot.p1_waiter_count,
                    _write_hold_observation: observability::observe_write_admission(operation),
                });
            }
            Err(reason) => {
                write_permit.revoke_fairness_admission();
                if matches!(
                    reason,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                ) {
                    // No SQLite work ran. Our own permit release must not wake the
                    // old-generation wait while a recovery reservation still blocks us.
                    write_permit.suppress_background_eligibility_wakeup();
                }
                drop(write_permit);
                match reason {
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                        if let Some(defer_reason) = wait_for_pressure_cooldown_retry(
                            gate,
                            operation,
                            pressure_retry_started,
                            &mut pressure_retry_attempts,
                        )
                        .await
                        {
                            retention_record_defer(operation, defer_reason);
                            return None;
                        }
                        continue;
                    }
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                        if wait_for_busy
                            && wait_for_background_change(gate, operation, try_generation).await
                        {
                            continue;
                        }
                    }
                }
                retention_record_defer(operation, reason);
                return None;
            }
        }
    }
}

async fn wait_for_background_change(
    gate: &crate::db_pressure::DbPressureGate,
    operation: &'static str,
    generation: u64,
) -> bool {
    let (remaining, timeout_reason) = if let Some(remaining) = retention_run_remaining_budget() {
        (remaining, "retention_work_budget")
    } else {
        retention_write_admission_budget()
    };
    if remaining.is_zero() {
        retention_record_defer(operation, timeout_reason);
        return false;
    }
    let eligible = tokio::time::timeout(remaining, gate.wait_for_eligibility_change(generation));
    let result = if let Some(shutdown) = retention_run_shutdown_token() {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                retention_record_defer(operation, "shutdown");
                return false;
            }
            result = eligible => result,
        }
    } else {
        eligible.await
    };
    if result.is_err() {
        retention_record_defer(operation, timeout_reason);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nested_recovery_write_reuses_parent_pressure_slot() {
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(1),
        ));
        let parent = gate
            .try_begin_background("summary_historical_coverage_recovery")
            .expect("recovery owns the background pressure slot");
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();

        let admission = RETENTION_TEST_DB_PRESSURE_GATE
            .scope(
                gate.clone(),
                RETENTION_TEST_WRITE_COORDINATOR.scope(
                    coordinator,
                    RETENTION_DB_PRESSURE_SLOT_HELD.scope(
                        (),
                        acquire_write_admission("summary_snapshot_proof_promotion", false),
                    ),
                ),
            )
            .await;

        assert!(admission.is_some(), "nested write must not self-defer");
        drop(admission);
        assert!(matches!(
            gate.try_begin_background("other_background_work"),
            Err(crate::db_pressure::DbPressureDenyReason::BackgroundBusy)
        ));
        drop(parent);
        assert!(gate.try_begin_background("other_background_work").is_ok());
    }
}
