use super::*;

#[cfg(test)]
#[derive(Default)]
pub(crate) struct BatchCommitProbe {
    pub(crate) committed: Notify,
    pub(crate) resume: Notify,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_BATCH_COMMIT: Arc<BatchCommitProbe>;
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

pub(crate) async fn acquire_retention_batch_write_connection(
    pool: &Pool<Sqlite>,
    operation: &'static str,
) -> Result<Option<(sqlx::pool::PoolConnection<Sqlite>, RetentionWriteAdmission)>> {
    acquire_write_connection(pool, operation, true).await
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
    let wait_for_busy = wait_for_busy && retention_run_remaining_budget().is_some();
    loop {
        if let Some(reason) = gate.background_deny_reason()
            && (!wait_for_busy
                || matches!(
                    reason,
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. }
                ))
        {
            retention_record_defer(operation, reason);
            return Ok(None);
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
    let wait_for_busy = wait_for_busy && retention_run_remaining_budget().is_some();
    loop {
        let generation = gate.eligibility_generation();
        if let Some(reason) = gate.background_deny_reason() {
            if wait_for_busy
                && matches!(
                    reason,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                )
            {
                if wait_for_background_change(gate, operation, generation).await {
                    continue;
                }
            } else {
                retention_record_defer(operation, reason);
            }
            return None;
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
        match gate.try_begin_background(operation) {
            Ok(pressure_permit) => {
                crate::task_runtime_observation::mark_managed_maintenance_work_started();
                return Some(RetentionWriteAdmission {
                    write_permit,
                    _pressure_permit: pressure_permit,
                    p1_waiter_count: coordinator_snapshot.p1_waiter_count,
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
                if wait_for_busy
                    && matches!(
                        reason,
                        crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                    )
                {
                    if wait_for_background_change(gate, operation, generation).await {
                        continue;
                    }
                } else {
                    retention_record_defer(operation, reason);
                }
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
    let Some(remaining) = retention_run_remaining_budget() else {
        return false;
    };
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
        retention_record_defer(operation, "retention_work_budget");
        return false;
    }
    true
}
