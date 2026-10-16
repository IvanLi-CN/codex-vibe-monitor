use super::*;
use crate::observability::contention::{
    PendingKind, PerformanceGauge, StatsProjection, current_performance, spawn_performance,
};

pub(super) const LONG_TERM_PROJECTION_CONSUMER: &str = "long_term_v1";
pub(super) const LONG_TERM_PROJECTION_FLUSH_INTERVAL: Duration = Duration::from_secs(60);
pub(super) const LONG_TERM_PROJECTION_REPAIR_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub(super) const LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL: Duration =
    Duration::from_secs(24 * 60 * 60);
pub(super) const LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH: i64 = 1;
pub(super) const LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH: i64 = 2_000;
pub(super) const LONG_TERM_PROJECTION_WRITE_BATCH_ROWS: usize = 512;
pub(super) const LONG_TERM_PROJECTION_ADMISSION_WAIT: Duration = Duration::from_millis(250);
pub(super) const LONG_TERM_PROJECTION_TRANSACTION_WAIT: Duration = Duration::from_millis(250);
// An incremental publication persists canonical interval state, rollups, its cursor, and status
// atomically. Keep every mutation in that one short transaction below the shared write limit.
pub(super) const LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS: usize = 2;
pub(super) const LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS: usize =
    LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS;
// A rebuild segment updates canonical state, membership, and a suppression row. Its first
// transaction also updates the bucket marker, so leave one row of headroom.
pub(super) const LONG_TERM_PROJECTION_REBUILD_SEGMENT_ROWS: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 1) / 3;
// Staging more dates than this would make retry bookkeeping needlessly large. Publication is a
// single token transaction; each already-published date is released separately afterward.
pub(super) const LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 2) / 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LongTermProjectionFlushOutcome {
    Completed,
    DeferredByPressure { retry_at: Option<Instant> },
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LongTermProjectionWorkStats {
    pub(super) loaded_row_count: u64,
    pub(super) terminal_event_count: u64,
}

#[derive(Debug)]
pub(super) struct LongTermProjectionWritePermit {
    pub(super) _pressure: Option<crate::db_pressure::DbBackgroundPermit>,
    pub(super) _write: Option<crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit>,
}

#[derive(Debug)]
pub(super) struct LongTermProjectionWriteControl<'a> {
    pub(super) shutdown: Option<&'a CancellationToken>,
    pub(super) gate: Option<&'a crate::db_pressure::DbPressureGate>,
    #[cfg(test)]
    pub(super) committed_batches: Option<(&'a AtomicUsize, usize)>,
    #[cfg(test)]
    pub(super) cancel_after_commit: Option<(&'a CancellationToken, &'a AtomicUsize, usize)>,
    #[cfg(test)]
    pub(super) stop_after_rebuild_chunk: Option<&'a CancellationToken>,
    #[cfg(test)]
    pub(super) stop_after_refresh_publication: Option<&'a CancellationToken>,
    #[cfg(test)]
    pub(super) stop_after_completed_integrity_repairs: Option<&'a CancellationToken>,
    #[cfg(test)]
    pub(super) stop_after_backup_cleanup_marker: Option<&'a CancellationToken>,
    #[cfg(test)]
    pub(super) stop_after_archive_compatibility_batch: Option<&'a CancellationToken>,
}

impl<'a> LongTermProjectionWriteControl<'a> {
    pub(super) fn unrestricted() -> Self {
        Self {
            shutdown: None,
            gate: None,
            #[cfg(test)]
            committed_batches: None,
            #[cfg(test)]
            cancel_after_commit: None,
            #[cfg(test)]
            stop_after_rebuild_chunk: None,
            #[cfg(test)]
            stop_after_refresh_publication: None,
            #[cfg(test)]
            stop_after_completed_integrity_repairs: None,
            #[cfg(test)]
            stop_after_backup_cleanup_marker: None,
            #[cfg(test)]
            stop_after_archive_compatibility_batch: None,
        }
    }

    pub(super) fn background(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            #[cfg(test)]
            committed_batches: None,
            #[cfg(test)]
            cancel_after_commit: None,
            #[cfg(test)]
            stop_after_rebuild_chunk: None,
            #[cfg(test)]
            stop_after_refresh_publication: None,
            #[cfg(test)]
            stop_after_completed_integrity_repairs: None,
            #[cfg(test)]
            stop_after_backup_cleanup_marker: None,
            #[cfg(test)]
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
        committed_batches: &'a AtomicUsize,
        limit: usize,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: Some((committed_batches, limit)),
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn cancelling_after(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
        committed_batches: &'a AtomicUsize,
        limit: usize,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: Some((shutdown, committed_batches, limit)),
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after_rebuild_chunk(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: Some(shutdown),
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after_refresh_publication(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: Some(shutdown),
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after_backup_cleanup_marker(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: Some(shutdown),
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after_archive_compatibility_batch(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: Some(shutdown),
        }
    }

    pub(super) fn complete_rebuild_chunk(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_rebuild_chunk {
            shutdown.cancel();
        }
    }

    pub(super) fn complete_refresh_publication(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_refresh_publication {
            shutdown.cancel();
        }
    }

    #[cfg(test)]
    pub(super) fn stopping_after_completed_integrity_repairs(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: Some(shutdown),
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    pub(super) fn complete_integrity_repairs(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_completed_integrity_repairs {
            shutdown.cancel();
        }
    }

    pub(super) fn complete_backup_cleanup_marker(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_backup_cleanup_marker {
            shutdown.cancel();
        }
    }

    pub(super) fn complete_archive_compatibility_batch(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_archive_compatibility_batch {
            shutdown.cancel();
        }
    }

    pub(super) fn check(&self) -> Result<()> {
        if self.shutdown.is_some_and(CancellationToken::is_cancelled) {
            bail!("long-term projection write cancelled");
        }
        #[cfg(test)]
        if self
            .committed_batches
            .is_some_and(|(count, limit)| count.load(Ordering::Acquire) >= limit)
        {
            bail!("long-term projection write cancelled after committed batch");
        }
        Ok(())
    }

    pub(super) async fn await_sqlite<T>(
        &self,
        operation: impl std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
        p2_coordinator: Option<&crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    ) -> Result<T> {
        self.check()?;
        match (self.shutdown, p2_coordinator) {
            (Some(shutdown), Some(coordinator)) => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                    _ = coordinator.wait_for_p2_preemption() => bail!("long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"),
                    result = operation => Ok(result?),
                }
            }
            (Some(shutdown), None) => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                    result = operation => Ok(result?),
                }
            }
            (None, Some(coordinator)) => {
                tokio::select! {
                    biased;
                    _ = coordinator.wait_for_p2_preemption() => bail!("long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"),
                    result = operation => Ok(result?),
                }
            }
            (None, None) => Ok(operation.await?),
        }
    }

    pub(super) fn try_begin_background(
        &self,
    ) -> Result<Option<crate::db_pressure::DbBackgroundPermit>> {
        self.check()?;
        self.gate
            .map(|gate| {
                gate.try_begin_background("long_term_projection_write")
                    .map_err(|reason| {
                        anyhow!(
                            "long-term projection write deferred by database pressure: {reason}"
                        )
                    })
            })
            .transpose()
    }

    pub(super) async fn begin_cursor_initialization<'p>(
        &self,
        pool: &'p Pool<Sqlite>,
        coordinator: &crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator,
    ) -> Result<sqlx::Transaction<'p, Sqlite>> {
        self.check()?;
        let begin = async {
            match tokio::time::timeout(
                LONG_TERM_PROJECTION_TRANSACTION_WAIT,
                pool.begin_with("BEGIN IMMEDIATE"),
            )
            .await
            {
                Ok(transaction) => Ok(transaction?),
                Err(_) => {
                    if let Some(gate) = self.gate {
                        gate.record_pressure(
                            "long_term_projection_write",
                            "transaction_admission_timeout",
                        );
                    }
                    bail!(
                        "long-term projection write deferred by database pressure: transaction admission timed out"
                    );
                }
            }
        };
        let transaction = (if let Some(shutdown) = self.shutdown {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                _ = coordinator.wait_for_p2_preemption() => bail!("long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"),
                result = begin => result,
            }
        } else {
            tokio::select! {
                biased;
                _ = coordinator.wait_for_p2_preemption() => bail!("long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"),
                result = begin => result,
            }
        })?;
        if coordinator.p2_should_yield() {
            drop(transaction);
            bail!(
                "long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"
            );
        }
        Ok(transaction)
    }

    pub(super) async fn begin<'p>(
        &self,
        pool: &'p Pool<Sqlite>,
    ) -> Result<(
        sqlx::Transaction<'p, Sqlite>,
        Option<LongTermProjectionWritePermit>,
    )> {
        self.check()?;
        let pressure_permit = if let Some(gate) = self.gate {
            let result = if let Some(shutdown) = self.shutdown {
                tokio::select! {
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                    result = gate.begin_background_with_busy_wait(
                        "long_term_projection_write",
                        LONG_TERM_PROJECTION_ADMISSION_WAIT,
                    ) => result,
                }
            } else {
                gate.begin_background_with_busy_wait(
                    "long_term_projection_write",
                    LONG_TERM_PROJECTION_ADMISSION_WAIT,
                )
                .await
            };
            Some(result.map_err(|reason| {
                anyhow!("long-term projection write deferred by database pressure: {reason}")
            })?)
        } else {
            None
        };
        let write_permit = if self.gate.is_some() {
            let coordinator =
                crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
            match coordinator.try_acquire(
                crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
            ) {
                Some(permit) => Some(permit),
                None => {
                    drop(pressure_permit);
                    bail!(
                        "long-term projection write deferred by database pressure: P2 write admission is not available"
                    );
                }
            }
        } else {
            None
        };
        let transaction = if let Some(shutdown) = self.shutdown {
            tokio::select! {
                _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                result = tokio::time::timeout(
                    LONG_TERM_PROJECTION_TRANSACTION_WAIT,
                    pool.begin_with("BEGIN IMMEDIATE"),
                ) => match result {
                    Ok(transaction) => transaction?,
                    Err(_) => {
                        if let Some(gate) = self.gate {
                            gate.record_pressure(
                                "long_term_projection_write",
                                "transaction_admission_timeout",
                            );
                        }
                        bail!("long-term projection write deferred by database pressure: transaction admission timed out");
                    }
                },
            }
        } else {
            pool.begin().await?
        };
        Ok((
            transaction,
            (pressure_permit.is_some() || write_permit.is_some()).then_some(
                LongTermProjectionWritePermit {
                    _pressure: pressure_permit,
                    _write: write_permit,
                },
            ),
        ))
    }

    pub(super) async fn commit(
        &self,
        transaction: sqlx::Transaction<'_, Sqlite>,
        permit: Option<LongTermProjectionWritePermit>,
    ) -> Result<()> {
        if let Some(shutdown) = self.shutdown {
            tokio::select! {
                _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                result = tokio::time::timeout(LONG_TERM_PROJECTION_TRANSACTION_WAIT, transaction.commit()) => match result {
                    Ok(result) => result?,
                    Err(_) => {
                        if let Some(gate) = self.gate {
                            gate.record_pressure(
                                "long_term_projection_write",
                                "transaction_commit_timeout",
                            );
                        }
                        bail!("long-term projection write deferred by database pressure: transaction commit timed out");
                    }
                },
            }
        } else {
            transaction.commit().await?;
        }
        drop(permit);
        #[cfg(test)]
        if let Some((count, _)) = self.committed_batches {
            count.fetch_add(1, Ordering::AcqRel);
        }
        #[cfg(test)]
        if let Some((shutdown, count, limit)) = self.cancel_after_commit
            && count.fetch_add(1, Ordering::AcqRel) + 1 >= limit
        {
            shutdown.cancel();
        }
        Ok(())
    }
}

pub(super) fn long_term_projection_write_is_deferred(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("long-term projection write deferred by database pressure")
        || message.contains("long-term projection write cancelled")
}

pub(super) fn long_term_projection_write_is_pressure_deferred(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .contains("long-term projection write deferred by database pressure")
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermProjectionCursorRow {
    pub(super) cursor_row_id: i64,
}

pub(crate) fn spawn_long_term_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let backfill_handle = spawn_long_term_stats_backfill(
        state.observability.clone(),
        state.pool.clone(),
        state.config.long_term_stats_hourly_retention_days,
        cancel.clone(),
    );
    spawn_performance(state.observability.clone(), async move {
        let supervisor_handle = spawn_performance(state.observability.clone(), async move {
            let pressure_gate = crate::db_pressure::global_db_pressure_gate();
            let mut pressure_eligibility_generation = pressure_gate.eligibility_generation();
            let mut pressure_retry_pending = false;
            let mut pressure_retry_at = None;
            let mut flush_ticker = interval(LONG_TERM_PROJECTION_FLUSH_INTERVAL);
            flush_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            flush_ticker.tick().await;
            let mut maintenance_ticker = interval(LONG_TERM_PROJECTION_FLUSH_INTERVAL);
            maintenance_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            maintenance_ticker.tick().await;
            let mut repair_ticker = interval(LONG_TERM_PROJECTION_REPAIR_INTERVAL);
            repair_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            repair_ticker.tick().await;
            let mut daily_verify_ticker = interval(LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL);
            daily_verify_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            daily_verify_ticker.tick().await;

            loop {
                if crate::maintenance_store::legacy_worker_should_skip("long_term_projection").await
                {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                    }
                }
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = wait_for_long_term_projection_pressure_retry(
                        pressure_gate,
                        pressure_eligibility_generation,
                        pressure_retry_at,
                    ), if pressure_retry_pending => {
                        pressure_eligibility_generation = pressure_gate.eligibility_generation();
                        match flush_long_term_projection(&state, "pressure_eligible").await {
                            Ok(LongTermProjectionFlushOutcome::Completed) => {
                                pressure_retry_pending = false;
                                pressure_retry_at = None;
                            }
                            Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                pressure_retry_at = retry_at;
                            }
                            Err(error) => {
                                pressure_retry_pending = false;
                                pressure_retry_at = None;
                                mark_long_term_projection_failure(&state, &error).await;
                                warn!(error = %error, projection = "long_term", trigger = "pressure_eligible", "long-term projection pressure retry failed");
                            }
                        }
                    }
                    _ = state.terminal_projection_hub.wait_for_persisted_work() => {
                        debug!(projection = "long_term", trigger = "terminal_p1_ack", "long-term projection marked dirty by terminal persistence");
                    }
                    _ = flush_ticker.tick() => {
                        let trigger = long_term_projection_terminal_flush_needed(&state)
                            .await
                            .then_some("terminal_deadline");
                        if let Some(trigger) = trigger {
                            pressure_eligibility_generation = pressure_gate.eligibility_generation();
                            match flush_long_term_projection(&state, trigger).await {
                                Ok(LongTermProjectionFlushOutcome::Completed) => {
                                    pressure_retry_pending = false;
                                    pressure_retry_at = None;
                                }
                                Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                    pressure_retry_pending = true;
                                    pressure_retry_at = retry_at;
                                }
                                Err(error) => {
                                    mark_long_term_projection_failure(&state, &error).await;
                                    warn!(error = %error, projection = "long_term", trigger, "long-term projection flush failed");
                                }
                            }
                        }
                        else {
                            debug!(projection = "long_term", trigger = "terminal_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term projection flush");
                        }
                    }
                    _ = maintenance_ticker.tick() => {
                        match long_term_projection_maintenance_needed(
                            &state.pool,
                            state.config.long_term_stats_hourly_retention_days,
                        )
                        .await
                        {
                            Ok(true) => {
                                pressure_eligibility_generation = pressure_gate.eligibility_generation();
                                match flush_long_term_projection(&state, "maintenance_deadline").await {
                                    Ok(LongTermProjectionFlushOutcome::Completed) => {
                                        pressure_retry_pending = false;
                                        pressure_retry_at = None;
                                    }
                                    Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                        pressure_retry_pending = true;
                                        pressure_retry_at = retry_at;
                                    }
                                    Err(error) => {
                                        mark_long_term_projection_failure(&state, &error).await;
                                        warn!(error = %error, projection = "long_term", trigger = "maintenance_deadline", "long-term projection maintenance failed");
                                    }
                                }
                            }
                            Ok(false) => {
                                debug!(projection = "long_term", trigger = "maintenance_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term projection maintenance");
                            }
                            Err(error) => {
                                mark_long_term_projection_failure(&state, &error).await;
                                warn!(error = %error, projection = "long_term", trigger = "maintenance_deadline", "failed to inspect long-term projection maintenance work");
                            }
                        }
                    }
                    _ = repair_ticker.tick() => {
                        match long_term_projection_repair_needed(&state).await {
                            Ok(true) => {
                                pressure_eligibility_generation = pressure_gate.eligibility_generation();
                                match flush_long_term_projection(&state, "repair_deadline").await {
                                    Ok(LongTermProjectionFlushOutcome::Completed) => {
                                        pressure_retry_pending = false;
                                        pressure_retry_at = None;
                                    }
                                    Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                        pressure_retry_pending = true;
                                        pressure_retry_at = retry_at;
                                    }
                                    Err(error) => {
                                        mark_long_term_projection_failure(&state, &error).await;
                                        warn!(error = %error, projection = "long_term", trigger = "repair_deadline", "long-term projection repair failed");
                                    }
                                }
                            }
                            Ok(false) => {
                                debug!(projection = "long_term", trigger = "repair_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term repair");
                            }
                            Err(error) => {
                                mark_long_term_projection_failure(&state, &error).await;
                                warn!(error = %error, projection = "long_term", trigger = "repair_deadline", "failed to inspect long-term projection repair work");
                            }
                        }
                    }
                    _ = daily_verify_ticker.tick() => {
                        pressure_eligibility_generation = pressure_gate.eligibility_generation();
                        match flush_long_term_projection(&state, "daily_verify").await {
                            Ok(LongTermProjectionFlushOutcome::Completed) => {
                                pressure_retry_pending = false;
                                pressure_retry_at = None;
                            }
                            Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                pressure_retry_pending = true;
                                pressure_retry_at = retry_at;
                            }
                            Err(error) => {
                                mark_long_term_projection_failure(&state, &error).await;
                                warn!(error = %error, projection = "long_term", trigger = "daily_verify", "long-term projection daily verification failed");
                            }
                        }
                    }
                }
            }
        });
        if let Err(error) = supervisor_handle.await {
            warn!(error = %error, projection = "long_term", "long-term projection supervisor terminated unexpectedly");
        }
        if let Err(error) = backfill_handle.await {
            warn!(error = %error, projection = "long_term", "long-term initial materialization worker terminated unexpectedly");
        }
    })
}

pub(super) async fn long_term_projection_terminal_flush_needed(state: &AppState) -> bool {
    let has_persisted_work = state.terminal_projection_hub.has_persisted_work();
    let runtime = state.long_term_projection_runtime.lock().await;
    long_term_projection_terminal_flush_due(has_persisted_work, runtime.state.is_empty())
}

pub(super) async fn long_term_projection_maintenance_needed(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<bool> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals LIMIT 1)",
    )
    .fetch_one(pool)
    .await?
        != 0
    {
        return Ok(true);
    }

    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let retention_start_ms = retention_start_epoch * 1_000;
    for (statement, value) in [
        (
            "SELECT EXISTS(SELECT 1 FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT 1)",
            retention_start_epoch,
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT 1)",
            retention_start_ms,
        ),
    ] {
        if sqlx::query_scalar::<_, i64>(statement)
            .bind(value)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    for table in [
        "long_term_projection_interval_suppressions",
        "long_term_projection_rebuild_members",
    ] {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT 1)"
        );
        if sqlx::query_scalar::<_, i64>(&statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    for statement in [
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_daily_backups backup
            JOIN long_term_projection_bucket_state state
              ON state.bucket_date = backup.stats_date
             AND state.active_daily_backup_token IS NULL
             AND state.publication_token = 'cleanup:' || backup.rebuild_token
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_daily_backup_claims claim
                WHERE claim.bucket_date = state.bucket_date
                  AND claim.rebuild_token = backup.rebuild_token
            )
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_bucket_state state
            WHERE state.active_daily_backup_token IS NULL
              AND state.publication_token LIKE 'cleanup:%'
              AND NOT EXISTS (
                  SELECT 1
                  FROM long_term_projection_daily_backups backup
                  WHERE backup.rebuild_token = substr(state.publication_token, 9)
              )
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_bucket_state state
            JOIN long_term_projection_date_publications publication
              ON publication.publication_token = state.publication_token
            WHERE publication.published = 1
              AND state.active_daily_backup_token IS NOT NULL
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_date_publications publication
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_bucket_state state
                WHERE state.publication_token = publication.publication_token
            )
            LIMIT 1
        )
        "#,
    ] {
        if sqlx::query_scalar::<_, i64>(statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn long_term_projection_terminal_flush_due(
    has_persisted_work: bool,
    runtime_state_is_empty: bool,
) -> bool {
    // A deferred date rebuild must not suppress the bounded terminal delta pass.
    // The pass can still advance any ready prefix while the repair ticker owns
    // the expensive rebuild deadline.
    has_persisted_work || runtime_state_is_empty
}

pub(super) async fn long_term_projection_repair_needed(state: &AppState) -> Result<bool> {
    let has_persisted_work = state.terminal_projection_hub.has_persisted_work();
    let (runtime_state_is_empty, next_repair_at) = {
        let runtime = state.long_term_projection_runtime.lock().await;
        (runtime.state.is_empty(), runtime.next_repair_at)
    };
    let now = Instant::now();
    if long_term_projection_repair_due(
        has_persisted_work,
        false,
        runtime_state_is_empty,
        next_repair_at,
        now,
    ) {
        return Ok(true);
    }

    // Correction and archive triggers write durable dirty markers without touching the
    // in-memory runtime state. Probe them only on the bounded repair cadence.
    let has_due_dirty_bucket = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now'))",
    )
    .fetch_one(&state.pool)
    .await?
        != 0;
    let daily_verify_due = long_term_projection_daily_verify_due(&state.pool).await?;
    Ok(long_term_projection_repair_due(
        has_persisted_work,
        has_due_dirty_bucket || daily_verify_due,
        runtime_state_is_empty,
        next_repair_at,
        now,
    ))
}

pub(super) fn long_term_projection_repair_due(
    has_persisted_work: bool,
    has_due_dirty_bucket: bool,
    runtime_state_is_empty: bool,
    next_repair_at: Option<Instant>,
    now: Instant,
) -> bool {
    // A scheduled repair owns the expensive rebuild deadline. Persisted terminal
    // deltas still use the bounded terminal pass, but must not pull this rebuild
    // forward before the deadline.
    next_repair_at.map_or(
        has_persisted_work || has_due_dirty_bucket || runtime_state_is_empty,
        |retry_at| retry_at <= now,
    )
}

pub(super) async fn defer_long_term_projection_terminal_repair(
    state: &AppState,
    defer_reason: &'static str,
) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = "dirty_last_good".to_string();
    runtime.last_defer_reason = Some(defer_reason.to_string());
    runtime.next_repair_at = Some(long_term_projection_repair_deadline(
        runtime.next_repair_at,
        Instant::now(),
    ));
}

pub(super) fn long_term_projection_repair_deadline(
    existing_deadline: Option<Instant>,
    now: Instant,
) -> Instant {
    // Repeated terminal flushes and pressure deferrals may arrive while a
    // targeted repair is pending. Keep the first deadline so they cannot
    // postpone recovery indefinitely.
    existing_deadline.unwrap_or(now + LONG_TERM_PROJECTION_REPAIR_INTERVAL)
}

pub(super) fn long_term_projection_pressure_retry_at(
    gate: &crate::db_pressure::DbPressureGate,
) -> Option<Instant> {
    match gate.background_deny_reason() {
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms }) => {
            Some(Instant::now() + Duration::from_millis(remaining_ms.max(1)))
        }
        Some(crate::db_pressure::DbPressureDenyReason::BackgroundBusy) | None => None,
    }
}

pub(super) async fn wait_for_long_term_projection_pressure_retry(
    gate: &crate::db_pressure::DbPressureGate,
    observed_generation: u64,
    retry_at: Option<Instant>,
) {
    if let Some(retry_at) = retry_at {
        sleep(retry_at.saturating_duration_since(Instant::now())).await;
    } else {
        gate.wait_for_eligibility_change(observed_generation).await;
    }
}

pub(super) fn long_term_projection_allows_expensive_repair(trigger: &str) -> bool {
    matches!(trigger, "repair_deadline" | "daily_verify")
}

pub(super) async fn mark_long_term_projection_failure(state: &AppState, error: &anyhow::Error) {
    let message = error.to_string().to_ascii_lowercase();
    let error_kind =
        if message.contains("database is locked") || message.contains("database is busy") {
            "sqlite_lock"
        } else if message.contains("source coverage incomplete") {
            "source_coverage"
        } else {
            "builder_error"
        };
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = "dirty_last_good".to_string();
    runtime.last_error_kind = Some(error_kind.to_string());
    runtime.next_repair_at = Some(long_term_projection_repair_deadline(
        runtime.next_repair_at,
        Instant::now(),
    ));
}

pub(super) async fn queue_long_term_projection_daily_verify(state: &AppState) -> Result<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );
    queue_long_term_projection_daily_verify_with_control(&state.pool, &today, &control).await
}

pub(super) async fn queue_long_term_projection_daily_verify_with_control(
    pool: &Pool<Sqlite>,
    daily_verify_date: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<String> {
    let pending = sqlx::query_as::<_, (i64, Option<String>)>(
        "SELECT COALESCE(daily_verify_pending, 0), daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_optional(pool)
    .await?
    .unwrap_or((0, None));
    if pending.0 != 0 {
        if let Some(pending_date) = pending.1 {
            return Ok(pending_date);
        }

        // Databases created by the preceding schema revision can retain a pending bit without
        // its bucket date. Recover the existing durable repair before using a new calendar date.
        let recovered_date = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY CASE repair_reason WHEN 'daily_verify' THEN 0 ELSE 1 END, queued_at ASC, bucket_date ASC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
        .unwrap_or_else(|| daily_verify_date.to_string());
        let (mut tx, permit) = control.begin(pool).await?;
        sqlx::query(
            "UPDATE long_term_projection_state SET daily_verify_bucket_date = ?2, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 1 AND daily_verify_bucket_date IS NULL",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(&recovered_date)
        .execute(&mut *tx)
        .await?;
        control.commit(tx, permit).await?;
        return Ok(recovered_date);
    }

    let (mut tx, permit) = control.begin(pool).await?;
    let claimed = sqlx::query(
        "UPDATE long_term_projection_state SET daily_verify_pending = 1, daily_verify_bucket_date = ?2, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 0",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(daily_verify_date)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed != 0 {
        sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'daily_verify') ON CONFLICT(bucket_date) DO UPDATE SET repair_reason = excluded.repair_reason, generation = long_term_projection_dirty_buckets.generation + 1, next_attempt_at = NULL, updated_at = datetime('now')",
        )
        .bind(daily_verify_date)
        .execute(&mut *tx)
        .await?;
    }
    control.commit(tx, permit).await?;
    if claimed != 0 {
        Ok(daily_verify_date.to_string())
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1 AND daily_verify_pending = 1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .fetch_one(pool)
        .await
        .context("daily verification pending bucket disappeared while it was claimed")
    }
}

pub(super) async fn long_term_projection_daily_verify_due(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_state WHERE consumer = ?1 AND (daily_verify_pending = 1 OR last_daily_verify_at IS NULL OR date(last_daily_verify_at, '+8 hours') < date('now', '+8 hours')))",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_one(pool)
    .await?
        != 0)
}

pub(super) fn long_term_projection_open_baseline_dates() -> Vec<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);
    vec![yesterday.to_string(), today.to_string()]
}

pub(super) async fn queue_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    queue_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

pub(super) async fn queue_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if dates.is_empty() {
        return Ok(());
    }
    for batch in dates.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut tx, permit) = control.begin(pool).await?;
        for date in batch {
            sqlx::query(
                "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, ?2) ON CONFLICT(bucket_date) DO UPDATE SET repair_reason = excluded.repair_reason, generation = long_term_projection_dirty_buckets.generation + 1, next_attempt_at = NULL, updated_at = datetime('now')",
            )
            .bind(date)
            .bind(repair_reason)
            .execute(&mut *tx)
            .await?;
        }
        control.commit(tx, permit).await?;
    }
    Ok(())
}

pub(super) async fn ensure_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    ensure_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

pub(super) async fn ensure_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if dates.is_empty() {
        return Ok(());
    }
    for batch in dates.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut tx, permit) = control.begin(pool).await?;
        for date in batch {
            sqlx::query(
                "INSERT OR IGNORE INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, ?2)",
            )
            .bind(date)
            .bind(repair_reason)
            .execute(&mut *tx)
            .await?;
        }
        control.commit(tx, permit).await?;
    }
    Ok(())
}

pub(super) async fn load_long_term_projection_dirty_buckets(
    pool: &Pool<Sqlite>,
    dates: &[String],
) -> Result<Vec<LongTermProjectionDirtyBucket>> {
    if dates.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_date, generation FROM long_term_projection_dirty_buckets WHERE bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(")");
    Ok(builder
        .build_query_as::<LongTermProjectionDirtyBucket>()
        .fetch_all(pool)
        .await?)
}

pub(super) async fn long_term_projection_repairs_are_deferred(
    pool: &Pool<Sqlite>,
    dates: &[String],
) -> Result<bool> {
    if dates.is_empty() {
        return Ok(false);
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE datetime(next_attempt_at) > datetime('now') AND bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated("))");
    Ok(builder.build_query_scalar::<i64>().fetch_one(pool).await? != 0)
}

pub(super) async fn defer_long_term_projection_repair(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repair_with_control(pool, bucket_date, &control).await
}

pub(super) async fn defer_long_term_projection_repair_with_control(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut tx, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_projection_dirty_buckets SET next_attempt_at = datetime('now', '+5 minutes'), updated_at = datetime('now') WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .execute(&mut *tx)
    .await?;
    control.commit(tx, permit).await?;
    Ok(())
}

pub(super) async fn defer_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repairs_with_control(pool, bucket_dates, &control).await
}

pub(super) async fn defer_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for bucket_date in bucket_dates {
        defer_long_term_projection_repair_with_control(pool, bucket_date, control).await?;
    }
    Ok(())
}

pub(super) async fn load_long_term_projection_ready_dates(
    pool: &Pool<Sqlite>,
    dates: &HashSet<String>,
) -> Result<HashSet<String>> {
    if dates.is_empty() {
        return Ok(HashSet::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT state.bucket_date FROM long_term_projection_bucket_state state WHERE state.interval_baseline_ready = 1 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets dirty LEFT JOIN long_term_projection_date_publications publication ON publication.publication_token = state.publication_token WHERE dirty.bucket_date = state.bucket_date AND (publication.published IS NULL OR publication.published = 0 OR state.publication_generation IS NULL OR dirty.generation <> state.publication_generation)) AND state.bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(")");
    Ok(builder
        .build_query_scalar::<String>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

pub(super) async fn load_long_term_projection_terminal_rows(
    pool: &Pool<Sqlite>,
    cursor: i64,
) -> Result<Vec<LongTermInvocationRow>> {
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let upstream_account_sql = if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    };
    let query = format!(
        r#"
        SELECT inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
          {upstream_account_sql} AS upstream_account_id,
          NULL AS upstream_account_kind, NULL AS upstream_account_name,
          inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms,
          inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms,
          inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message
        FROM codex_invocations inv
        WHERE inv.id > ?1
          AND LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
        ORDER BY inv.id ASC
        LIMIT ?2
        "#,
    );
    Ok(sqlx::query_as::<_, LongTermInvocationRow>(&query)
        .bind(cursor)
        .bind(LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH)
        .fetch_all(pool)
        .await?)
}

pub(super) fn build_long_term_projection_event(
    row: &LongTermInvocationRow,
) -> LongTermProjectionEvent {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    accumulate_long_term_invocation(row, &mut hourly, &mut daily, &mut statistics_start);
    let segments = collect_long_term_projection_interval_segments(&hourly, &daily, row.id);
    let mut bucket_dates = daily
        .keys()
        .map(|(bucket_date, _, _)| bucket_date.clone())
        .collect::<HashSet<_>>();
    for segment in &segments {
        bucket_dates.extend(long_term_projection_interval_dates(segment));
    }
    LongTermProjectionEvent {
        row_id: row.id,
        hourly,
        daily,
        segments,
        bucket_dates,
    }
}

pub(super) fn long_term_projection_canonical_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') = 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2"
    )
}

pub(super) fn long_term_projection_crossing_text_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND inv.occurred_at < ?1 AND CASE WHEN instr(inv.occurred_at, 'T') = 0 AND inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 THEN julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 END >= julianday(?1)"
    )
}

pub(super) async fn load_long_term_projection_live_rfc3339_compatibility(
    pool: &Pool<Sqlite>,
) -> Result<Option<LongTermRfc3339Compatibility>> {
    let terminal_filter = "instr(occurred_at, 'T') > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')";
    let has_rfc3339 = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT 1 FROM codex_invocations WHERE {terminal_filter} LIMIT 1"
    ))
    .fetch_optional(pool)
    .await?;
    if has_rfc3339.is_none() {
        return Ok(None);
    }
    let max_duration_ms = sqlx::query_scalar::<_, f64>(&format!(
        "SELECT t_total_ms FROM codex_invocations WHERE {terminal_filter} AND t_total_ms IS NOT NULL AND t_total_ms > 0 ORDER BY t_total_ms DESC LIMIT 1"
    ))
    .fetch_optional(pool)
    .await?;
    Ok(Some(LongTermRfc3339Compatibility { max_duration_ms }))
}

pub(super) fn long_term_projection_live_rfc3339_query(select: &str) -> String {
    let rfc3339_epoch = long_term_rfc3339_whole_epoch_seconds_sql("inv.occurred_at");
    let rfc3339_reaches_range_start =
        long_term_rfc3339_reaches_epoch_sql("inv.occurred_at", "inv.t_total_ms", "?3");
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') > 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2 AND {rfc3339_epoch} < ?4 AND ({rfc3339_epoch} >= ?3 OR (inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 AND {rfc3339_reaches_range_start}))"
    )
}

pub(super) async fn invalidate_long_term_projection_interval_cache(state: &AppState) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.interval_index.clear();
    runtime.loaded_interval_dates.clear();
}

pub(super) async fn flush_long_term_projection_unlocked(
    state: &AppState,
    trigger: &'static str,
) -> Result<LongTermProjectionFlushOutcome> {
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = run_long_term_projection_flush_with_retry(&state.shutdown, || {
        flush_long_term_projection_inner(state, trigger)
    })
    .await;
    let load_row_count = result
        .as_ref()
        .map(|stats| stats.loaded_row_count)
        .unwrap_or_default();
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "long_term_projection_flush",
            memory_baseline,
            load_row_count,
            true,
        )
        .await;
    match result {
        Ok(_) => Ok(LongTermProjectionFlushOutcome::Completed),
        Err(error) if long_term_projection_write_is_pressure_deferred(&error) => {
            let mut runtime = state.long_term_projection_runtime.lock().await;
            runtime.state = "deferred".to_string();
            runtime.last_defer_reason = Some("writer_pressure".to_string());
            debug!(projection = "long_term", trigger, gate_outcome = "deferred", defer_reason = "writer_pressure", error = %error, "long-term projection flush deferred at a bounded write boundary");
            Ok(LongTermProjectionFlushOutcome::DeferredByPressure {
                retry_at: long_term_projection_pressure_retry_at(
                    crate::db_pressure::global_db_pressure_gate(),
                ),
            })
        }
        Err(error) if long_term_projection_write_is_deferred(&error) => Err(error),
        Err(error) => {
            let gate = crate::db_pressure::global_db_pressure_gate();
            if gate.record_error("long_term_projection_write", &error) {
                let mut runtime = state.long_term_projection_runtime.lock().await;
                runtime.state = "deferred".to_string();
                runtime.last_defer_reason = Some("writer_pressure".to_string());
                debug!(projection = "long_term", trigger, gate_outcome = "deferred", defer_reason = "sqlite_pressure", error = %error, "long-term projection flush deferred after a SQLite pressure error");
                Ok(LongTermProjectionFlushOutcome::DeferredByPressure {
                    retry_at: long_term_projection_pressure_retry_at(gate),
                })
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) async fn run_long_term_projection_once(state: &AppState) -> Result<()> {
    flush_long_term_projection(state, "managed_task")
        .await
        .map(|_| ())
}

pub(crate) async fn run_long_term_projection_once_managed(
    state: &AppState,
) -> Result<LongTermProjectionFlushOutcome> {
    flush_long_term_projection_unlocked(state, "managed_task").await
}

pub(super) async fn run_long_term_projection_flush_with_retry<T, Operation, OperationFuture>(
    shutdown: &CancellationToken,
    operation: Operation,
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    run_long_term_projection_flush_with_retry_delays(
        shutdown,
        operation,
        &LONG_TERM_PROJECTION_LOCK_RETRY_DELAYS,
    )
    .await
}

pub(super) async fn run_long_term_projection_flush_with_retry_delays<
    T,
    Operation,
    OperationFuture,
>(
    shutdown: &CancellationToken,
    mut operation: Operation,
    retry_delays: &[Duration],
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if crate::is_sqlite_lock_error(&error) => {
                warn!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    error = %error,
                    "long-term projection flush hit a SQLite lock; retrying"
                );
                tokio::select! {
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled during SQLite lock retry"),
                    _ = sleep(*delay) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
    operation().await
}

pub(super) async fn flush_long_term_projection_inner(
    state: &AppState,
    trigger: &'static str,
) -> Result<LongTermProjectionWorkStats> {
    // The initial refresher and P2 share date backups for crash recovery, but never their live
    // replacement window. Defer P2 while the refresher owns this process-wide maintenance lock;
    // a later terminal wake or ticker retries the bounded work.
    let _refresh_guard = match LONG_TERM_REFRESH_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(LongTermProjectionWorkStats::default()),
    };
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );

    let started = Instant::now();
    if trigger == "maintenance_deadline" {
        return advance_long_term_projection_maintenance(state, &control).await;
    }
    let mut cursor = load_long_term_projection_cursor_with_control(&state.pool, &control).await?;
    let daily_verify_requested =
        trigger == "daily_verify" || long_term_projection_daily_verify_due(&state.pool).await?;
    let daily_verify_date = if daily_verify_requested {
        Some(queue_long_term_projection_daily_verify(state).await?)
    } else {
        None
    };
    let state_row = load_long_term_state(&state.pool).await?;
    let mut baseline_cursor = None;
    let rollups_exist = long_term_rollups_exist(&state.pool).await?;
    if long_term_initial_materialization_needed(
        &state_row.status,
        state_row.last_error.as_deref(),
        rollups_exist,
    ) {
        // The dedicated refresher owns the first full materialization. Running it from this
        // P2 cursor worker bypasses pressure admission and can hold a competing writer lock.
        control.check()?;
        return Ok(LongTermProjectionWorkStats::default());
    } else if rollups_exist
        && (cursor == 0
            || matches!(
                state_row.status.as_str(),
                LONG_TERM_STATUS_RUNNING | LONG_TERM_STATUS_PREPARING | LONG_TERM_STATUS_DISABLED
            ))
    {
        // Existing rollups are a durable baseline after upgrade. Only the two open calendar
        // buckets need interval baselines before new terminal deltas can be merged exactly.
        baseline_cursor = Some(load_long_term_terminal_watermark(&state.pool).await?);
    }

    let mut repaired = Vec::new();
    let mut event_count = 0usize;
    let mut loaded_row_count = 0u64;
    let mut deferred_repair_count = 0usize;
    let mut deferred_repair_backoff_count = 0usize;
    if let Some(baseline_cursor) = baseline_cursor {
        let baseline_dates = long_term_projection_open_baseline_dates();
        queue_long_term_projection_repairs_with_control(
            &state.pool,
            &baseline_dates,
            "interval_baseline",
            &control,
        )
        .await?;
        let baseline_dirty =
            load_long_term_projection_dirty_buckets(&state.pool, &baseline_dates).await?;
        let mut rebuilds = Vec::with_capacity(baseline_dates.len());
        for date in &baseline_dates {
            let rebuild =
                build_long_term_projection_date_rebuild(&state.pool, date, &control).await?;
            loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
            rebuilds.push(rebuild);
        }
        commit_long_term_projection_date_rebuilds_with_control(
            &state.pool,
            &rebuilds,
            Some(baseline_cursor),
            &baseline_dirty,
            state_row.status != LONG_TERM_STATUS_ERROR,
            &control,
        )
        .await?;
        repaired.extend(baseline_dates);
        cursor = baseline_cursor;
    } else {
        let identities = load_long_term_account_identities(&state.pool).await?;
        let mut events = load_long_term_projection_terminal_rows(&state.pool, cursor).await?;
        for row in &mut events {
            hydrate_long_term_account_identity(row, &identities);
        }
        let events = events
            .iter()
            .map(build_long_term_projection_event)
            .collect::<Vec<_>>();
        let candidate_dates = events
            .iter()
            .flat_map(|event| event.bucket_dates.iter().cloned())
            .collect::<HashSet<_>>();
        let ready_dates =
            load_long_term_projection_ready_dates(&state.pool, &candidate_dates).await?;
        let mut hourly = HashMap::new();
        let mut daily = HashMap::new();
        let mut segments = Vec::new();
        let mut direct_cursor = cursor;
        let mut batch_event_count = 0usize;
        let mut repair_event = None;
        for event in events {
            if !event.bucket_dates.is_subset(&ready_dates) {
                repair_event = Some(event);
                break;
            }
            let event_mutation_rows = event.hourly.len() + event.daily.len() + event.segments.len();
            if event_mutation_rows > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS {
                // A single event cannot fit beside its canonical interval, cursor, and state
                // updates. Rebuild all affected dates exactly instead of widening a write lock.
                repair_event = Some(event);
                break;
            }
            let additional_rollup_rows = event
                .hourly
                .keys()
                .filter(|key| !hourly.contains_key(*key))
                .count()
                + event
                    .daily
                    .keys()
                    .filter(|key| !daily.contains_key(*key))
                    .count();
            if batch_event_count > 0
                && hourly.len()
                    + daily.len()
                    + additional_rollup_rows
                    + segments.len()
                    + event.segments.len()
                    > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS
            {
                let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
                    &state.pool,
                    &state.long_term_projection_runtime,
                    LongTermProjectionIncrementalBatch {
                        hourly: &hourly,
                        daily: &daily,
                        segments: &segments,
                    },
                    direct_cursor,
                    batch_event_count,
                    &control,
                )
                .await?;
                if outcome == LongTermProjectionIncrementalOutcome::RebuildRequired {
                    defer_long_term_projection_terminal_repair(state, "dirty_publication").await;
                    return Ok(LongTermProjectionWorkStats::default());
                }
                crate::record_managed_task_discovered_work(
                    &["long_term_projection"],
                    i64::try_from(batch_event_count).unwrap_or(i64::MAX),
                );
                crate::record_managed_task_processed_work(
                    &["long_term_projection"],
                    i64::try_from(batch_event_count).unwrap_or(i64::MAX),
                );
                cursor = direct_cursor;
                hourly.clear();
                daily.clear();
                segments.clear();
                batch_event_count = 0;
            }
            direct_cursor = event.row_id;
            event_count = event_count.saturating_add(1);
            batch_event_count = batch_event_count.saturating_add(1);
            merge_long_term_projection_buckets(&mut hourly, event.hourly);
            merge_long_term_projection_buckets(&mut daily, event.daily);
            segments.extend(event.segments);
        }
        if batch_event_count > 0 {
            let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
                &state.pool,
                &state.long_term_projection_runtime,
                LongTermProjectionIncrementalBatch {
                    hourly: &hourly,
                    daily: &daily,
                    segments: &segments,
                },
                direct_cursor,
                batch_event_count,
                &control,
            )
            .await?;
            if outcome == LongTermProjectionIncrementalOutcome::RebuildRequired {
                defer_long_term_projection_terminal_repair(state, "dirty_publication").await;
                return Ok(LongTermProjectionWorkStats::default());
            }
            crate::record_managed_task_discovered_work(
                &["long_term_projection"],
                i64::try_from(batch_event_count).unwrap_or(i64::MAX),
            );
            crate::record_managed_task_processed_work(
                &["long_term_projection"],
                i64::try_from(batch_event_count).unwrap_or(i64::MAX),
            );
            cursor = direct_cursor;
        }
        if let Some(event) = repair_event {
            let mut repair_dates = event.bucket_dates.into_iter().collect::<Vec<_>>();
            repair_dates.sort();
            if !long_term_projection_allows_expensive_repair(trigger) {
                deferred_repair_count = deferred_repair_count.saturating_add(repair_dates.len());
                deferred_repair_backoff_count =
                    deferred_repair_backoff_count.saturating_add(repair_dates.len());
                debug!(
                    projection = "long_term",
                    repair_scope = ?repair_dates,
                    defer_reason = "terminal_hot_path",
                    "long-term cursor repair deferred to the bounded repair window"
                );
                defer_long_term_projection_terminal_repair(state, "terminal_hot_path").await;
            } else {
                ensure_long_term_projection_repairs_with_control(
                    &state.pool,
                    &repair_dates,
                    "interval_baseline",
                    &control,
                )
                .await?;
                let repair_dirty =
                    load_long_term_projection_dirty_buckets(&state.pool, &repair_dates).await?;
                if long_term_projection_repairs_are_deferred(&state.pool, &repair_dates).await? {
                    deferred_repair_count =
                        deferred_repair_count.saturating_add(repair_dates.len());
                    deferred_repair_backoff_count =
                        deferred_repair_backoff_count.saturating_add(repair_dates.len());
                    debug!(
                        projection = "long_term",
                        repair_scope = ?repair_dates,
                        defer_reason = "repair_backoff",
                        "long-term cursor repair retained until its retry deadline"
                    );
                } else {
                    let mut rebuilds = Vec::with_capacity(repair_dates.len());
                    let mut rebuild_error = None;
                    for date in &repair_dates {
                        match build_long_term_projection_date_rebuild(&state.pool, date, &control)
                            .await
                        {
                            Ok(rebuild) => {
                                loaded_row_count =
                                    loaded_row_count.saturating_add(rebuild.source_row_count);
                                rebuilds.push(rebuild);
                            }
                            Err(error) => {
                                rebuild_error = Some(error);
                                break;
                            }
                        }
                    }
                    if let Some(error) = rebuild_error {
                        defer_long_term_projection_repairs_with_control(
                            &state.pool,
                            &repair_dates,
                            &control,
                        )
                        .await?;
                        deferred_repair_count =
                            deferred_repair_count.saturating_add(repair_dates.len());
                        warn!(
                            error = %error,
                            projection = "long_term",
                            repair_scope = ?repair_dates,
                            retry_after_ms = 300_000_u64,
                            "long-term cursor repair deferred after an unavailable source"
                        );
                    } else {
                        commit_long_term_projection_date_rebuilds_with_control(
                            &state.pool,
                            &rebuilds,
                            Some(event.row_id),
                            &repair_dirty,
                            false,
                            &control,
                        )
                        .await?;
                        repaired.extend(repair_dates);
                        cursor = event.row_id;
                    }
                }
            }
        }
    }

    if !repaired.is_empty() {
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let dirty_dates = if !long_term_projection_allows_expensive_repair(trigger) {
        Vec::new()
    } else {
        sqlx::query_as::<_, LongTermProjectionDirtyBucket>(
            "SELECT bucket_date, generation FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now') ORDER BY queued_at ASC, bucket_date ASC LIMIT ?1",
        )
        .bind(LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH)
        .fetch_all(&state.pool)
        .await?
    };
    for dirty in dirty_dates {
        let date = dirty.bucket_date.clone();
        if repaired.contains(&date) {
            continue;
        }
        let rebuild =
            match build_long_term_projection_date_rebuild(&state.pool, &date, &control).await {
                Ok(rebuild) => {
                    loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
                    rebuild
                }
                Err(error) => {
                    defer_long_term_projection_repair_with_control(&state.pool, &date, &control)
                        .await?;
                    deferred_repair_count = deferred_repair_count.saturating_add(1);
                    warn!(
                        error = %error,
                        projection = "long_term",
                        repair_scope = %date,
                        retry_after_ms = 300_000_u64,
                        "long-term projection repair deferred after an unavailable source"
                    );
                    continue;
                }
            };
        commit_long_term_projection_date_rebuilds_with_control(
            &state.pool,
            &[rebuild],
            None,
            std::slice::from_ref(&dirty),
            false,
            &control,
        )
        .await?;
        repaired.push(date);
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let (retention_pruned_hourly_rows, retention_pruned_interval_rows) = (0, 0);

    if let Some(daily_verify_date) = daily_verify_date {
        let maintenance_pending = long_term_projection_maintenance_needed(
            &state.pool,
            state.config.long_term_stats_hourly_retention_days,
        )
        .await?;
        complete_long_term_projection_daily_verify_with_control(
            &state.pool,
            &daily_verify_date,
            maintenance_pending,
            &control,
        )
        .await?;
    }

    let dirty_bucket_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&state.pool)
            .await?
            .max(0) as usize;
    deferred_repair_backoff_count = deferred_repair_backoff_count.max(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE datetime(next_attempt_at) > datetime('now')",
        )
        .fetch_one(&state.pool)
        .await?
        .max(0) as usize,
    );
    state
        .terminal_projection_hub
        .advance_long_term_cursor(cursor);
    let projection_health = state.terminal_projection_hub.health();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = if deferred_repair_count > 0 {
        "dirty_last_good"
    } else if dirty_bucket_count == 0 {
        "healthy"
    } else {
        "repairing"
    }
    .to_string();
    runtime.cursor_row_id = cursor;
    runtime.dirty_bucket_count = dirty_bucket_count;
    runtime.pending_event_count = projection_health.pending_event_count;
    runtime.last_flush_elapsed_ms = Some(elapsed_ms);
    runtime.last_flush_at = Some(Instant::now());
    runtime.last_repair_scope = (!repaired.is_empty()).then(|| repaired.join(","));
    let terminal_hot_path_deferred =
        !long_term_projection_allows_expensive_repair(trigger) && deferred_repair_count > 0;
    runtime.last_defer_reason = if terminal_hot_path_deferred {
        Some("terminal_hot_path".to_string())
    } else if deferred_repair_backoff_count > 0 {
        Some("repair_backoff".to_string())
    } else if deferred_repair_count > 0 {
        Some("repair_source_unavailable".to_string())
    } else {
        None
    };
    runtime.last_error_kind = (!terminal_hot_path_deferred && deferred_repair_count > 0)
        .then(|| "targeted_repair".to_string());
    runtime.next_repair_at = if deferred_repair_count > 0 {
        Some(long_term_projection_repair_deadline(
            runtime.next_repair_at,
            Instant::now(),
        ))
    } else {
        None
    };
    if let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) {
        metrics.set_performance(
            PerformanceGauge::ProjectionPending(
                StatsProjection::LongTerm,
                PendingKind::DirtyBuckets,
            ),
            dirty_bucket_count as f64,
        );
        metrics.set_performance(
            PerformanceGauge::ProjectionPending(StatsProjection::LongTerm, PendingKind::Rows),
            projection_health.pending_event_count as f64,
        );
        if dirty_bucket_count == 0 && deferred_repair_count == 0 {
            metrics.set_performance(
                PerformanceGauge::ProjectionLastGood(StatsProjection::LongTerm),
                Utc::now().timestamp() as f64,
            );
        }
    }
    let interval_bytes = runtime
        .interval_index
        .iter()
        .map(|(key, union)| {
            key.bucket_key.len()
                + key.dimension.len()
                + key.series_key.len()
                + union.intervals.len() * std::mem::size_of::<(i64, i64)>()
        })
        .sum::<usize>();
    debug!(
        projection = "long_term",
        trigger,
        event_count,
        cursor_lag = projection_health.last_persisted_row_id.saturating_sub(cursor),
        dirty_bucket_count,
        repair_scope = ?runtime.last_repair_scope,
        interval_bytes,
        interval_key_count = runtime.interval_index.len(),
        deferred_repair_count,
        deferred_repair_backoff_count,
        retention_pruned_hourly_rows,
        retention_pruned_interval_rows,
        flush_outcome = "accepted",
        elapsed_ms,
        "long-term projection flush completed"
    );
    drop(runtime);
    if event_count == 0 {
        crate::record_managed_task_discovered_work(&["long_term_projection"], 0);
        crate::record_managed_task_processed_work(&["long_term_projection"], 0);
    }
    Ok(LongTermProjectionWorkStats {
        loaded_row_count,
        terminal_event_count: event_count as u64,
    })
}
