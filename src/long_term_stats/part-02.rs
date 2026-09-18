#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LongTermProjectionFlushOutcome {
    Completed,
    DeferredByPressure { retry_at: Option<Instant> },
}

#[derive(Debug)]
struct LongTermProjectionWritePermit {
    _pressure: Option<crate::db_pressure::DbBackgroundPermit>,
    _write: Option<crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit>,
}

#[derive(Debug)]
struct LongTermProjectionWriteControl<'a> {
    shutdown: Option<&'a CancellationToken>,
    gate: Option<&'a crate::db_pressure::DbPressureGate>,
    #[cfg(test)]
    committed_batches: Option<(&'a AtomicUsize, usize)>,
    #[cfg(test)]
    cancel_after_commit: Option<(&'a CancellationToken, &'a AtomicUsize, usize)>,
    #[cfg(test)]
    stop_after_rebuild_chunk: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_refresh_publication: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_completed_integrity_repairs: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_backup_cleanup_marker: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_archive_compatibility_batch: Option<&'a CancellationToken>,
}

impl<'a> LongTermProjectionWriteControl<'a> {
    fn unrestricted() -> Self {
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

    fn background(
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
    fn stopping_after(
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
    fn cancelling_after(
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
    fn stopping_after_rebuild_chunk(
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
    fn stopping_after_refresh_publication(
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
    fn stopping_after_backup_cleanup_marker(
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
    fn stopping_after_archive_compatibility_batch(
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

    fn complete_rebuild_chunk(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_rebuild_chunk {
            shutdown.cancel();
        }
    }

    fn complete_refresh_publication(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_refresh_publication {
            shutdown.cancel();
        }
    }

    #[cfg(test)]
    fn stopping_after_completed_integrity_repairs(
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

    fn complete_integrity_repairs(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_completed_integrity_repairs {
            shutdown.cancel();
        }
    }

    fn complete_backup_cleanup_marker(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_backup_cleanup_marker {
            shutdown.cancel();
        }
    }

    fn complete_archive_compatibility_batch(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_archive_compatibility_batch {
            shutdown.cancel();
        }
    }

    fn check(&self) -> Result<()> {
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

    async fn await_sqlite<T>(
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

    fn try_begin_background(&self) -> Result<Option<crate::db_pressure::DbBackgroundPermit>> {
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

    async fn begin_cursor_initialization<'p>(
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

    async fn begin<'p>(
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

    async fn commit(
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

fn long_term_projection_write_is_deferred(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("long-term projection write deferred by database pressure")
        || message.contains("long-term projection write cancelled")
}

fn long_term_projection_write_is_pressure_deferred(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .contains("long-term projection write deferred by database pressure")
}

async fn ensure_long_term_projection_source_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    if !invocation_table_exists {
        return Ok(());
    }
    let columns = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if columns.contains("status") {
        // Terminal delta scans advance by id. Index the exact terminal predicate so a dense
        // running/pending prefix cannot turn a bounded incremental flush into a table scan.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_terminal_id
            ON codex_invocations (id)
            WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection terminal id index")?;
    }
    if !columns.contains("occurred_at") || !columns.contains("t_total_ms") {
        return Ok(());
    }
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_text_end
        ON codex_invocations (
            CASE
                WHEN instr(occurred_at, 'T') = 0
                  AND t_total_ms IS NOT NULL
                  AND t_total_ms > 0
                THEN julianday(occurred_at) + t_total_ms / 86400000.0
            END
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection text end index")?;
    if columns.contains("status") {
        // RFC3339 rows are exceptional, but preserving them requires an index-backed candidate
        // range before the exact (and deliberately compatibility-safe) epoch predicate runs.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_occurred_at
            ON codex_invocations (occurred_at)
            WHERE instr(occurred_at, 'T') > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 timestamp index")?;
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_duration
            ON codex_invocations (t_total_ms DESC)
            WHERE instr(occurred_at, 'T') > 0
              AND t_total_ms IS NOT NULL
              AND t_total_ms > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 duration index")?;
    }
    Ok(())
}

async fn ensure_long_term_projection_correction_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?;
    if invocation_table_exists == 0 {
        return Ok(());
    }

    // Corrections originate in several write-side workers. A normal terminal finalize moves an
    // in-flight record to a terminal state and is consumed through the projection cursor unless
    // a newer terminal row has already advanced that cursor past it. In that out-of-order case,
    // the cursor cannot revisit the row, so queue the affected date for an exact repair.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_invocation_correction")
        .execute(pool)
        .await?;
    let old_start_date = long_term_rfc3339_shanghai_date_sql("OLD.occurred_at", None);
    let old_end_date = long_term_rfc3339_shanghai_date_sql(
        "OLD.occurred_at",
        Some("MAX(COALESCE(OLD.t_total_ms, 0), 0)"),
    );
    let new_start_date = long_term_rfc3339_shanghai_date_sql("NEW.occurred_at", None);
    let new_end_date = long_term_rfc3339_shanghai_date_sql(
        "NEW.occurred_at",
        Some("MAX(COALESCE(NEW.t_total_ms, 0), 0)"),
    );
    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_invocation_correction
        AFTER UPDATE OF
          source, status, occurred_at, model, payload,
          input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens,
          cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
          t_upstream_ttfb_ms, t_upstream_stream_ms, error_message, failure_kind
        ON codex_invocations
        WHEN NOT (
          (
            LOWER(TRIM(COALESCE(OLD.status, ''))) IN ('running', 'pending')
            OR (
              LOWER(TRIM(COALESCE(OLD.status, ''))) = 'interrupted'
              AND LOWER(TRIM(COALESCE(OLD.failure_kind, ''))) = 'proxy_interrupted'
            )
          )
          AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
          AND NEW.id > COALESCE(
            (
              SELECT cursor_row_id
              FROM long_term_projection_state
              WHERE consumer = 'long_term_v1'
            ),
            0
          )
        )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
            SELECT
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_start_date}
                ELSE date(OLD.occurred_at) END,
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_end_date}
                ELSE date(julianday(OLD.occurred_at) + MAX(COALESCE(OLD.t_total_ms, 0), 0) / 86400000.0) END
            WHERE OLD.occurred_at IS NOT NULL AND TRIM(OLD.occurred_at) <> ''
            UNION ALL
            SELECT
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_start_date}
                ELSE date(NEW.occurred_at) END,
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_end_date}
                ELSE date(julianday(NEW.occurred_at) + MAX(COALESCE(NEW.t_total_ms, 0), 0) / 86400000.0) END
            WHERE NEW.occurred_at IS NOT NULL AND TRIM(NEW.occurred_at) <> ''
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM affected_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'invocation_correction'
          FROM affected_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection correction trigger")?;
    Ok(())
}

async fn ensure_long_term_projection_archive_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    if !long_term_projection_archive_batches_exist(pool).await? {
        return Ok(());
    }

    // Archive writes and rewrites are source changes for durable long-term rollups. Invocation
    // archives provide the terminal facts; attempt archives provide a later account-attribution
    // fallback. Both must invalidate the same target dates. Recreate these triggers so upgrades
    // do not retain a prior definition that only observed invocation archives.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_insert")
        .execute(pool)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_update")
        .execute(pool)
        .await?;

    let (
        new_coverage_start_date,
        new_coverage_end_date,
        old_coverage_start_date,
        old_coverage_end_date,
    ) = long_term_projection_archive_trigger_date_expressions();

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_insert
        AFTER INSERT ON archive_batches
        WHEN NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
          AND NEW.status = 'completed'
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE covered_dates(bucket_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive insert trigger")?;

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_update
        AFTER UPDATE OF dataset, status, file_path, sha256, coverage_start_at, coverage_end_at, historical_rollups_materialized_at ON archive_batches
        WHEN (
              NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            )
            OR (
              OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
            )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE coverage_ranges(start_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
            WHERE NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            UNION ALL
            SELECT
              COALESCE(CASE WHEN instr(OLD.coverage_start_at, 'T') > 0 THEN {old_coverage_start_date} ELSE date(OLD.coverage_start_at) END, date(OLD.month_key || '-01')),
              COALESCE(CASE WHEN instr(OLD.coverage_end_at, 'T') > 0 THEN {old_coverage_end_date} ELSE date(OLD.coverage_end_at) END, date(OLD.month_key || '-01', '+1 month', '-1 day'))
            WHERE OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
          ), covered_dates(bucket_date, end_date) AS (
            SELECT start_date, end_date FROM coverage_ranges
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive update trigger")?;
    Ok(())
}

async fn long_term_projection_archive_batches_exist(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archive_batches')",
    )
    .fetch_one(pool)
    .await?
        != 0)
}

fn long_term_projection_archive_trigger_date_expressions() -> (String, String, String, String) {
    (
        long_term_rfc3339_shanghai_date_sql("NEW.coverage_start_at", None),
        long_term_rfc3339_shanghai_date_sql("NEW.coverage_end_at", None),
        long_term_rfc3339_shanghai_date_sql("OLD.coverage_start_at", None),
        long_term_rfc3339_shanghai_date_sql("OLD.coverage_end_at", None),
    )
}

pub(crate) async fn ensure_long_term_projection_account_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    for trigger in [
        "long_term_projection_account_kind_update",
        "long_term_projection_account_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(pool)
            .await?;
    }
    for (trigger, event, account_id) in [
        (
            "long_term_projection_account_kind_update",
            "AFTER UPDATE OF kind ON pool_upstream_accounts WHEN OLD.kind IS NOT NEW.kind",
            "NEW.id",
        ),
        (
            "long_term_projection_account_delete",
            "AFTER DELETE ON pool_upstream_accounts",
            "OLD.id",
        ),
    ] {
        let occurred_at_start_date = long_term_rfc3339_shanghai_date_sql("inv.occurred_at", None);
        let occurred_at_end_date = long_term_rfc3339_shanghai_date_sql(
            "inv.occurred_at",
            Some("MAX(COALESCE(inv.t_total_ms, 0), 0)"),
        );
        let archive_coverage_start_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_start_at", None);
        let archive_coverage_end_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_end_at", None);
        let statement = format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS {trigger}
            {event}
            BEGIN
              INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
              WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
                SELECT
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_start_date}
                    ELSE date(inv.occurred_at) END,
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_end_date}
                    ELSE date(julianday(inv.occurred_at) + MAX(COALESCE(inv.t_total_ms, 0), 0) / 86400000.0) END
                FROM codex_invocations inv
                WHERE (CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END) = {account_id}
                   OR EXISTS (
                        SELECT 1 FROM pool_upstream_request_attempts attempt
                        WHERE attempt.invoke_id = inv.invoke_id
                          AND attempt.occurred_at = inv.occurred_at
                          AND attempt.upstream_account_id = {account_id}
                      )
                UNION ALL
                SELECT
                  COALESCE(CASE WHEN instr(archive.coverage_start_at, 'T') > 0
                    THEN {archive_coverage_start_date}
                    ELSE date(archive.coverage_start_at) END, date(archive.month_key || '-01')),
                  COALESCE(CASE WHEN instr(archive.coverage_end_at, 'T') > 0
                    THEN {archive_coverage_end_date}
                    ELSE date(archive.coverage_end_at) END, date(archive.month_key || '-01', '+1 month', '-1 day'))
                FROM archive_batches archive
                WHERE archive.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
                  AND archive.status = 'completed'
                UNION ALL
                SELECT date(bucket_date, '+1 day'), end_date
                FROM affected_dates
                WHERE bucket_date < end_date
              )
              SELECT DISTINCT bucket_date, 'account_classification_changed'
              FROM affected_dates
              WHERE bucket_date IS NOT NULL
              ON CONFLICT(bucket_date) DO UPDATE SET
                repair_reason = excluded.repair_reason,
                generation = long_term_projection_dirty_buckets.generation + 1,
                next_attempt_at = NULL,
                updated_at = datetime('now');
            END
            "#,
        );
        sqlx::query(&statement).execute(pool).await?;
    }
    Ok(())
}
