use super::*;

pub(super) async fn mark_long_term_stats_backfill_preparing(pool: &Pool<Sqlite>) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    mark_long_term_stats_backfill_preparing_with_control(pool, &control).await
}

pub(super) async fn mark_long_term_stats_backfill_preparing_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    // Preserve error across restart so a persisted integrity queue keeps the next refresh on the
    // verified incremental path. Error without durable rows still transitions to running inside
    // refresh_long_term_stats_once.
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status NOT IN (?3, ?4)",
    )
    .bind(LONG_TERM_STATUS_PREPARING)
    .bind(LONG_TERM_STATE_ID)
    .bind(LONG_TERM_STATUS_READY)
    .bind(LONG_TERM_STATUS_ERROR)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(crate) fn spawn_long_term_stats_backfill(
    pool: Pool<Sqlite>,
    retention_days: u64,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if shutdown.is_cancelled() {
                break;
            }
            let state = match load_long_term_state(&pool).await {
                Ok(state) => state,
                Err(error) => {
                    warn!(error = %error, "failed to inspect long-term initial materialization state");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = ticker.tick() => {}
                    }
                    continue;
                }
            };
            let rollups_exist = match long_term_rollups_exist(&pool).await {
                Ok(exists) => exists,
                Err(error) => {
                    warn!(error = %error, "failed to inspect long-term initial materialization rollups");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = ticker.tick() => {}
                    }
                    continue;
                }
            };
            if !long_term_initial_materialization_needed(
                &state.status,
                state.last_error.as_deref(),
                rollups_exist,
            ) {
                break;
            }
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution("long_term_projection")
            else {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                continue;
            };
            let observation = crate::TaskExecutionObservation::begin(
                "long_term_projection",
                &crate::maintenance_store::task_title_for_observation("long_term_projection"),
                "startup",
                crate::maintenance_store::task_execution_class("long_term_projection"),
                "processing",
            );
            let control = LongTermProjectionWriteControl::background(
                &shutdown,
                crate::db_pressure::global_db_pressure_gate(),
            );
            let status = match mark_long_term_stats_backfill_preparing_with_control(&pool, &control)
                .await
            {
                Err(error) => {
                    warn!(error = %error, "failed to mark long-term initial materialization preparing");
                    "failed"
                }
                Ok(()) if shutdown.is_cancelled() => break,
                Ok(()) => {
                    match refresh_long_term_stats_with_control(&pool, retention_days, &control)
                        .await
                    {
                        Ok(()) => "success",
                        Err(error) if long_term_projection_write_is_deferred(&error) => {
                            debug!(error = %error, "long-term initial materialization deferred by database pressure");
                            "skipped"
                        }
                        Err(error) => {
                            warn!(error = %error, "long-term initial materialization failed");
                            "failed"
                        }
                    }
                }
            };
            if shutdown.is_cancelled() {
                break;
            }
            observation.finish_with_status(status);
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {}
            }
        }
    })
}

pub(crate) async fn refresh_long_term_stats(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    refresh_long_term_stats_with_control(pool, retention_days, &control).await
}

pub(super) async fn refresh_long_term_stats_with_control(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let _guard = LONG_TERM_REFRESH_LOCK.lock().await;
    run_long_term_refresh_with_retry(control.shutdown, || {
        refresh_long_term_stats_once(pool, retention_days, control)
    })
    .await
}

pub(super) async fn persist_long_term_refresh_progress(
    pool: &Pool<Sqlite>,
    processed_rows: i64,
    total_rows: i64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET processed_rows = ?1, total_rows = ?2, updated_at = datetime('now') WHERE id = ?3",
    )
    .bind(processed_rows)
    .bind(total_rows)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(super) async fn run_long_term_refresh_with_retry<T, Operation, OperationFuture>(
    shutdown: Option<&CancellationToken>,
    operation: Operation,
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    run_long_term_refresh_with_retry_delays(
        shutdown,
        operation,
        &LONG_TERM_REFRESH_LOCK_RETRY_DELAYS,
    )
    .await
}

pub(super) async fn run_long_term_refresh_with_retry_delays<T, Operation, OperationFuture>(
    shutdown: Option<&CancellationToken>,
    mut operation: Operation,
    retry_delays: &[Duration],
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        if shutdown.is_some_and(CancellationToken::is_cancelled) {
            bail!("long-term stats refresh cancelled before SQLite lock retry");
        }
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if crate::is_sqlite_lock_error(&error) => {
                warn!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    error = %error,
                    "long-term stats refresh hit a SQLite lock; retrying"
                );
                if let Some(shutdown) = shutdown {
                    tokio::select! {
                        _ = shutdown.cancelled() => bail!("long-term stats refresh cancelled during SQLite lock retry"),
                        _ = sleep(*delay) => {}
                    }
                } else {
                    sleep(*delay).await;
                }
            }
            Err(error) => return Err(error),
        }
    }
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        bail!("long-term stats refresh cancelled before SQLite lock retry");
    }
    operation().await
}

pub(super) fn long_term_refresh_start_state(
    was_ready: bool,
    has_pending_integrity_repairs: bool,
) -> (&'static str, bool) {
    if has_pending_integrity_repairs {
        (LONG_TERM_STATUS_ERROR, false)
    } else if !was_ready {
        (LONG_TERM_STATUS_RUNNING, true)
    } else {
        // Durable daily backups keep the ready read model on the prior complete date until its
        // bounded replacement is fully written and published.
        (LONG_TERM_STATUS_READY, true)
    }
}

pub(super) fn long_term_refresh_pending_marker(
    was_ready: bool,
    starting_status: &str,
) -> Option<&'static str> {
    (!was_ready && starting_status != LONG_TERM_STATUS_ERROR)
        .then_some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
}

pub(super) async fn refresh_long_term_stats_once(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let refresh_started_at = format_utc_iso(Utc::now());
    let state_snapshot =
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<String>)>(
            "SELECT status, statistics_start_date, integrity_source_start_date, last_error FROM long_term_stats_state WHERE id = ?1",
        )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let was_ready =
        state_snapshot
            .as_ref()
            .is_some_and(|(status, statistics_start_date, _, last_error)| {
                status.as_deref().is_some_and(|status| {
                    matches!(status, LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY)
                    // An error after the final publication still has a durable baseline. Keep
                    // its replacement incremental so a deferred repair retains its retry
                    // backoff. A failed initial refresh retains its explicit pending marker,
                    // even if it had reached a provisional start date before an archive read
                    // failed, so recovery replays every source archive.
                    || (status == LONG_TERM_STATUS_ERROR
                        && statistics_start_date.is_some()
                        && last_error.as_deref()
                            != Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR))
                })
            });
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let retention_start = today - ChronoDuration::days(retention_days.max(366) as i64 - 1);
    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        state_snapshot
            .as_ref()
            .and_then(|(_, statistics_start_date, _, _)| statistics_start_date.as_deref()),
        state_snapshot
            .as_ref()
            .and_then(|(_, _, integrity_source_start_date, _)| {
                integrity_source_start_date.as_deref()
            }),
    );
    let has_pending_integrity_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_stats_repair_queue WHERE stats_date >= ?1)",
    )
    .bind(reconstructable_start.to_string())
    .fetch_one(pool)
    .await?
        != 0;
    let preserves_prior_error = state_snapshot.as_ref().is_some_and(|(status, _, _, _)| {
        status
            .as_deref()
            .is_some_and(|status| status == LONG_TERM_STATUS_ERROR)
    });
    let (starting_status, clear_last_error) = long_term_refresh_start_state(
        was_ready,
        has_pending_integrity_repairs || preserves_prior_error,
    );
    let pending_marker = long_term_refresh_pending_marker(was_ready, starting_status);
    let (mut transaction, permit) = control.begin(pool).await?;
    if let Some(pending_marker) = pending_marker {
        // A bounded replacement may be interrupted after committing only one batch. Persist a
        // distinct marker before those writes so startup and the P2 cursor never mistake a
        // partial durable prefix for a complete baseline.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3",
        )
        .bind(starting_status)
        .bind(pending_marker)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    } else if clear_last_error {
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    } else {
        // Keep known-bad materialized data hidden throughout a repair attempt. The final
        // replacement transaction is the only path that clears the queue and restores ready.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;

    let result = refresh_long_term_stats_inner(
        pool,
        retention_days,
        !was_ready,
        &refresh_started_at,
        control,
    )
    .await;
    if let Err(err) = &result
        && let Ok((mut transaction, permit)) = control.begin(pool).await
    {
        let _ = sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(err.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await;
        let _ = control.commit(transaction, permit).await;
    }
    result
}

pub(super) async fn refresh_long_term_stats_inner(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    initial_materialization: bool,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let previous_state = sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let live_tail_start = (today - ChronoDuration::days(2))
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| db_occurred_at_lower_bound(value.with_timezone(&Utc)));
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let live_upstream_account_id_sql = if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    };
    let legacy_model_keys = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_usage_daily WHERE dimension = 'model' AND series_key NOT LIKE 'model:v2:%')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let ready_state = !initial_materialization && !legacy_model_keys;
    let retention_start = today - ChronoDuration::days(retention_days.max(366) as i64 - 1);
    let integrity_audit_due = ready_state && long_term_integrity_audit_due(pool).await?;
    let mut terminal_proof_reconciliation_incomplete =
        previous_state.as_ref().is_some_and(|state| {
            state.status == LONG_TERM_STATUS_ERROR
                && state.last_error.as_deref() == Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        });
    let mut invalidated_terminal_proof_buckets = Vec::new();
    let mut unavailable_reconciliation_archive_paths = Vec::new();
    // A source-availability failure is retryable as soon as the next refresh runs. Waiting for
    // the hourly audit after a restored archive would leave an otherwise recoverable view in
    // error for up to an hour.
    if ready_state || integrity_audit_due || terminal_proof_reconciliation_incomplete {
        match backfill_invocation_rollup_hourly_from_sources(pool).await {
            Ok(reconciliation) => {
                terminal_proof_reconciliation_incomplete = !reconciliation.source_complete;
                invalidated_terminal_proof_buckets = reconciliation.invalidated_bucket_start_epochs;
                unavailable_reconciliation_archive_paths =
                    reconciliation.unavailable_archive_file_paths;
            }
            Err(error) => {
                terminal_proof_reconciliation_incomplete = true;
                warn!(
                    error = %error,
                    "terminal integrity proof source reconciliation failed; keeping affected buckets untrusted"
                );
            }
        }
    }
    if terminal_proof_reconciliation_incomplete {
        // Persist the availability failure before later source reads. A subsequent error must
        // not restore ready while canonical proofs have already been revoked.
        let (mut transaction, permit) = control.begin(pool).await?;
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .bind(refresh_started_at)
        .execute(&mut *transaction)
        .await?;
        control.commit(transaction, permit).await?;
    }
    if !unavailable_reconciliation_archive_paths.is_empty() {
        clear_long_term_invocation_replay_markers_for_unavailable_sources(
            pool,
            &unavailable_reconciliation_archive_paths,
            control,
        )
        .await?;
    }
    let mut hourly: HashMap<(i64, String, String), LongTermBucket> = HashMap::new();
    let mut daily: HashMap<(String, String, String), LongTermBucket> = HashMap::new();
    let mut statistics_start_date = previous_state
        .as_ref()
        .and_then(|state| state.statistics_start_date.clone());
    let account_identities = load_long_term_account_identities(pool).await?;
    let mut rows = Vec::new();
    let mut row_positions = HashMap::new();
    let mut processed_rows_count = 0_i64;
    if ready_state {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.occurred_at,
            inv.status,
            inv.model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
            {live_upstream_account_id_sql} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            inv.total_tokens,
            inv.output_tokens,
            inv.cost,
            inv.t_total_ms,
            inv.t_req_read_ms,
            inv.t_req_parse_ms,
            inv.t_upstream_connect_ms,
            inv.t_upstream_ttfb_ms,
            inv.t_upstream_stream_ms,
            inv.error_message
        FROM codex_invocations inv
        WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
          AND (
              datetime(inv.occurred_at) >= datetime(?1)
              OR (
                  inv.t_total_ms IS NOT NULL
                  AND inv.t_total_ms > 0
                  AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)
              )
          )
        ORDER BY inv.occurred_at ASC, inv.id ASC
            "#,
            live_upstream_account_id_sql = live_upstream_account_id_sql,
        );
        let mut live_rows = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql)
            .bind(live_tail_start.as_deref())
            .fetch(pool);
        while let Some(row) = live_rows.try_next().await? {
            if row_positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
            }
        }
    } else {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.occurred_at,
            inv.status,
            inv.model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
            {live_upstream_account_id_sql} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            inv.total_tokens,
            inv.output_tokens,
            inv.cost,
            inv.t_total_ms,
            inv.t_req_read_ms,
            inv.t_req_parse_ms,
            inv.t_upstream_connect_ms,
            inv.t_upstream_ttfb_ms,
            inv.t_upstream_stream_ms,
            inv.error_message
        FROM codex_invocations inv
        WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
        ORDER BY inv.occurred_at ASC, inv.id ASC
            "#,
            live_upstream_account_id_sql = live_upstream_account_id_sql,
        );
        let mut live_rows = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql).fetch(pool);
        while let Some(row) = live_rows.try_next().await? {
            if row_positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
                processed_rows_count += 1;
            }
            if processed_rows_count % 256 == 0 {
                // The archive workload has not been enumerated yet during a full rebuild, so
                // keep the total explicitly unknown instead of presenting a false completion
                // ratio to the preparation UI.
                persist_long_term_refresh_progress(pool, processed_rows_count, 0, control).await?;
            }
        }
    }

    let archive_paths = match load_completed_invocation_archive_paths(pool).await {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => Vec::new(),
        Err(error) => return Err(error),
    };
    // Archive manifests update `created_at` when a legacy monthly file is rewritten. Remove
    // stale replay markers before deciding which archive files can be skipped.
    let stale_marker_cleanup = delete_long_term_refresh_replay_markers_with_control(
        pool,
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE rowid IN (
            SELECT replay.rowid
            FROM hourly_rollup_archive_replay replay
            WHERE replay.target = ?1
              AND replay.dataset = 'codex_invocations'
              AND EXISTS (
                  SELECT 1
                  FROM archive_batches batches
                  WHERE batches.dataset = 'codex_invocations'
                    AND batches.file_path = replay.file_path
                    AND datetime(batches.created_at) > datetime(replay.replayed_at)
              )
            LIMIT ?2
        )
        "#,
        &[LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string()],
        control,
    )
    .await;
    if let Err(error) = stale_marker_cleanup
        && !error.to_string().contains("no such table")
    {
        return Err(error);
    }
    let replayed_archive_files = if !ready_state {
        HashSet::new()
    } else {
        match sqlx::query_as::<_, (String, Option<String>)>(
            r#"
        SELECT replay.file_path, replay.source_identity
        FROM hourly_rollup_archive_replay replay
        INNER JOIN archive_batches batches
          ON batches.dataset = 'codex_invocations'
         AND batches.file_path = replay.file_path
         AND batches.sha256 = replay.archive_sha256
        WHERE replay.target = ?1 AND replay.dataset = 'codex_invocations'
        "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows
                .into_iter()
                .filter(|(file_path, source_identity)| {
                    source_identity.as_deref().is_some_and(|source_identity| {
                        long_term_archive_file_identity(file_path).ok().as_deref()
                            == Some(source_identity)
                    })
                })
                .map(|(file_path, _)| file_path)
                .collect::<HashSet<_>>(),
            Err(error) if error.to_string().contains("no such table") => HashSet::new(),
            Err(error) => return Err(error.into()),
        }
    };
    // Replayed invocation archives can still be reopened during a date rebuild, so keep the
    // attempt-account fallback available even when no archive needs first-time materialization.
    let attempt_date_range = if ready_state {
        let mut dates = HashSet::new();
        for row in &rows {
            if let Some(date) =
                parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
                    Shanghai
                        .timestamp_millis_opt(timestamp)
                        .single()
                        .map(|value| value.date_naive())
                })
            {
                dates.insert(date);
            }
        }
        let mut requires_full_attempt_scan = false;
        for path in &archive_paths {
            if replayed_archive_files.contains(path.file_path()) {
                continue;
            }
            if path.coverage_start_at().is_none() || path.coverage_end_at().is_none() {
                requires_full_attempt_scan = true;
            } else {
                insert_long_term_date_range(
                    &mut dates,
                    path.coverage_start_at(),
                    path.coverage_end_at(),
                );
            }
        }
        if requires_full_attempt_scan {
            None
        } else {
            dates
                .iter()
                .min()
                .copied()
                .zip(dates.iter().max().copied())
                .map(|(start, end)| {
                    (
                        start.pred_opt().unwrap_or(start),
                        end.succ_opt().unwrap_or(end),
                    )
                })
        }
    } else {
        None
    };
    let (mut archive_attempt_accounts, mut attempt_archive_markers) =
        if !ready_state || attempt_date_range.is_some() {
            load_long_term_archive_attempt_accounts(pool, attempt_date_range).await?
        } else {
            (HashMap::new(), HashSet::new())
        };
    let mut archive_markers = Vec::new();
    let mut archive_read_failed = false;
    let mut failed_archive_paths = HashSet::new();
    let mut failed_archive_ranges = Vec::new();
    let mut clear_all_attempt_markers = false;
    let mut unreadable_source_start_date: Option<NaiveDate> = None;
    let mut affected_archive_dates = HashSet::new();
    let all_archive_paths = archive_paths.clone();
    for archive_path in archive_paths {
        if replayed_archive_files.contains(archive_path.file_path()) {
            continue;
        }
        let archive_sha256_before_open = match crate::maintenance::sha256_hex_file(
            std::path::Path::new(archive_path.file_path()),
        ) {
            Ok(value) => value,
            Err(error) => {
                archive_read_failed = true;
                failed_archive_paths.insert(archive_path.file_path().to_string());
                let unreadable_start =
                    long_term_unreadable_source_start(&archive_path, retention_start);
                unreadable_source_start_date = Some(
                    unreadable_source_start_date
                        .map_or(unreadable_start, |current| current.min(unreadable_start)),
                );
                match (
                    archive_path
                        .coverage_start_at()
                        .and_then(long_term_archive_end_date),
                    archive_path
                        .coverage_end_at()
                        .and_then(long_term_archive_end_date),
                ) {
                    (Some(start), Some(end)) => {
                        failed_archive_ranges.push((start.to_string(), end.to_string()));
                    }
                    _ => clear_all_attempt_markers = true,
                }
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive identity read failed");
                continue;
            }
        };
        let archive_manifest_sha256 =
            load_long_term_archive_sha256(pool, archive_path.file_path()).await?;
        if !long_term_archive_scan_identity_matches_manifest(
            &archive_sha256_before_open,
            Some(&archive_sha256_before_open),
            archive_manifest_sha256.as_deref(),
        ) {
            archive_read_failed = true;
            failed_archive_paths.insert(archive_path.file_path().to_string());
            let unreadable_start =
                long_term_unreadable_source_start(&archive_path, retention_start);
            unreadable_source_start_date = Some(
                unreadable_source_start_date
                    .map_or(unreadable_start, |current| current.min(unreadable_start)),
            );
            match (
                archive_path
                    .coverage_start_at()
                    .and_then(long_term_archive_end_date),
                archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date),
            ) {
                (Some(start), Some(end)) => {
                    failed_archive_ranges.push((start.to_string(), end.to_string()));
                }
                _ => clear_all_attempt_markers = true,
            }
            warn!(
                file_path = archive_path.file_path(),
                "long-term stats archive does not match its completed manifest; clearing its replay marker for retry"
            );
            continue;
        }
        let Some((archive_pool, cleanup)) = (match open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-stats",
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                archive_read_failed = true;
                failed_archive_paths.insert(archive_path.file_path().to_string());
                let unreadable_start =
                    long_term_unreadable_source_start(&archive_path, retention_start);
                unreadable_source_start_date = Some(
                    unreadable_source_start_date
                        .map_or(unreadable_start, |current| current.min(unreadable_start)),
                );
                match (
                    archive_path
                        .coverage_start_at()
                        .and_then(long_term_archive_end_date),
                    archive_path
                        .coverage_end_at()
                        .and_then(long_term_archive_end_date),
                ) {
                    (Some(start), Some(end)) => {
                        failed_archive_ranges.push((start.to_string(), end.to_string()));
                    }
                    _ => clear_all_attempt_markers = true,
                }
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive read failed");
                None
            }
        }) else {
            archive_read_failed = true;
            failed_archive_paths.insert(archive_path.file_path().to_string());
            let unreadable_start =
                long_term_unreadable_source_start(&archive_path, retention_start);
            unreadable_source_start_date = Some(
                unreadable_source_start_date
                    .map_or(unreadable_start, |current| current.min(unreadable_start)),
            );
            match (
                archive_path
                    .coverage_start_at()
                    .and_then(long_term_archive_end_date),
                archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date),
            ) {
                (Some(start), Some(end)) => {
                    failed_archive_ranges.push((start.to_string(), end.to_string()));
                }
                _ => clear_all_attempt_markers = true,
            }
            continue;
        };
        let archive_rows = match long_term_archive_invocation_query(&archive_pool).await {
            Ok(query) => sqlx::query_as::<_, LongTermInvocationRow>(&query)
                .fetch_all(&archive_pool)
                .await
                .map_err(Into::into),
            Err(error) => Err(error),
        };
        archive_pool.close().await;
        drop(cleanup);
        match archive_rows {
            Ok(archive_rows) => {
                for mut row in archive_rows {
                    hydrate_long_term_archive_attempt_account(&mut row, &archive_attempt_accounts);
                    if let Some(date) =
                        parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
                            Shanghai
                                .timestamp_millis_opt(timestamp)
                                .single()
                                .map(|value| value.date_naive())
                        })
                    {
                        affected_archive_dates.insert(date);
                    }
                    if let Some(index) = row_positions.get(&row.id).copied() {
                        merge_long_term_invocation_row(&mut row, &rows[index]);
                        rows[index] = row;
                    } else {
                        row_positions.insert(row.id, rows.len());
                        rows.push(row);
                        processed_rows_count += 1;
                    }
                }
                if !ready_state {
                    insert_long_term_date_range(
                        &mut affected_archive_dates,
                        archive_path.coverage_start_at(),
                        archive_path.coverage_end_at(),
                    );
                }
                let archive_sha256_after_read = crate::maintenance::sha256_hex_file(
                    std::path::Path::new(archive_path.file_path()),
                );
                let archive_manifest_sha256 =
                    load_long_term_archive_sha256(pool, archive_path.file_path()).await?;
                if long_term_archive_scan_identity_matches_manifest(
                    &archive_sha256_before_open,
                    archive_sha256_after_read.as_deref().ok(),
                    archive_manifest_sha256.as_deref(),
                ) {
                    archive_markers.push((
                        archive_path.file_path().to_string(),
                        archive_sha256_before_open,
                    ));
                } else {
                    archive_read_failed = true;
                    failed_archive_paths.insert(archive_path.file_path().to_string());
                    let unreadable_start =
                        long_term_unreadable_source_start(&archive_path, retention_start);
                    unreadable_source_start_date = Some(
                        unreadable_source_start_date
                            .map_or(unreadable_start, |current| current.min(unreadable_start)),
                    );
                    match (
                        archive_path
                            .coverage_start_at()
                            .and_then(long_term_archive_end_date),
                        archive_path
                            .coverage_end_at()
                            .and_then(long_term_archive_end_date),
                    ) {
                        (Some(start), Some(end)) => {
                            failed_archive_ranges.push((start.to_string(), end.to_string()));
                        }
                        _ => clear_all_attempt_markers = true,
                    }
                    warn!(
                        file_path = archive_path.file_path(),
                        error = ?archive_sha256_after_read.as_ref().err(),
                        "long-term stats archive changed while it was scanned; clearing its replay marker for retry"
                    );
                }
            }
            Err(error) => {
                archive_read_failed = true;
                failed_archive_paths.insert(archive_path.file_path().to_string());
                let unreadable_start =
                    long_term_unreadable_source_start(&archive_path, retention_start);
                unreadable_source_start_date = Some(
                    unreadable_source_start_date
                        .map_or(unreadable_start, |current| current.min(unreadable_start)),
                );
                match (
                    archive_path
                        .coverage_start_at()
                        .and_then(long_term_archive_end_date),
                    archive_path
                        .coverage_end_at()
                        .and_then(long_term_archive_end_date),
                ) {
                    (Some(start), Some(end)) => {
                        failed_archive_ranges.push((start.to_string(), end.to_string()));
                    }
                    _ => clear_all_attempt_markers = true,
                }
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive query failed");
            }
        }
    }
    let total_rows = rows.len() as i64;
    if ready_state {
        persist_long_term_refresh_progress(pool, 0, total_rows, control).await?;
    }
    for (index, row) in rows.iter().enumerate() {
        let mut row = row.clone();
        hydrate_long_term_account_identity(&mut row, &account_identities);
        accumulate_long_term_invocation(&row, &mut hourly, &mut daily, &mut statistics_start_date);
        if ready_state && ((index + 1) % 256 == 0 || index + 1 == rows.len()) {
            persist_long_term_refresh_progress(pool, (index + 1) as i64, total_rows, control)
                .await?;
        }
    }

    // A completed day is audited against the canonical hourly rollup once per hour. The durable
    // source boundary only advances after a source archive is deleted with an exact interval
    // proof; a temporarily unreadable archive instead blocks candidate publication below.
    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        statistics_start_date.as_deref(),
        previous_state
            .as_ref()
            .and_then(|state| state.integrity_source_start_date.as_deref()),
    );
    for mismatch in load_long_term_reconciliation_mismatches(
        pool,
        &invalidated_terminal_proof_buckets,
        reconstructable_start,
    )
    .await?
    {
        warn!(
            stats_date = %mismatch.date,
            reason = %mismatch.reason,
            "canonical terminal proof reconciliation mismatch queued for long-term repair"
        );
        enqueue_long_term_integrity_mismatch(pool, &mismatch, control).await?;
    }
    if ready_state
        && integrity_audit_due
        && let Some(audit_end) = today.pred_opt()
    {
        for mismatch in audit_long_term_integrity(pool, reconstructable_start, audit_end).await? {
            warn!(
                stats_date = %mismatch.date,
                reason = %mismatch.reason,
                "long-term stats integrity mismatch detected"
            );
            enqueue_long_term_integrity_mismatch(pool, &mismatch, control).await?;
        }
        mark_long_term_integrity_audit(pool, control).await?;
    }
    let scheduled_repair_date = next_due_long_term_repair_date(pool, reconstructable_start).await?;

    // A day may be split across live rows and archive parts. Rebuild every date touched by the
    // current live tail from all overlapping source parts before replacing durable buckets.
    let mut recomputed_dates = affected_archive_dates.clone();
    for (date, _, _) in daily.keys() {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            recomputed_dates.insert(date);
        }
    }
    if let Some(date) = scheduled_repair_date {
        recomputed_dates.insert(date);
    }
    let mut integrity_repair_failures = Vec::new();
    let mut completed_integrity_repairs = HashSet::new();
    if let Some(unreadable_source_start) = unreadable_source_start_date {
        if let Some(date) = scheduled_repair_date.filter(|date| *date >= unreadable_source_start) {
            let (candidate_daily, _) = long_term_candidate_integrity(date, &hourly, &daily);
            if let Some(mismatch) = queued_long_term_repair_mismatch(
                pool,
                date,
                candidate_daily,
                "one or more invocation archives are unreadable, so this repair cannot prove complete source coverage",
            )
            .await?
            {
                integrity_repair_failures.push(mismatch);
            }
        }

        // An unreadable archive can contain a request whose wall-time interval reaches any
        // later date. Do not infer a bounded continuation from manifest coverage: preserve
        // existing rows and wait for the source file to become readable again.
        let mut blocked_candidate_dates = recomputed_dates
            .iter()
            .copied()
            .filter(|date| *date >= unreadable_source_start)
            .collect::<HashSet<_>>();
        for (date, _, _) in daily.keys() {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                && date >= unreadable_source_start
            {
                blocked_candidate_dates.insert(date);
            }
        }
        for (bucket_start_epoch, _, _) in hourly.keys() {
            if let Some(date) = long_term_bucket_date(*bucket_start_epoch)
                && date >= unreadable_source_start
            {
                blocked_candidate_dates.insert(date);
            }
        }
        remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_candidate_dates);
        recomputed_dates.retain(|date| !blocked_candidate_dates.contains(date));
    }
    if !ready_state && !recomputed_dates.is_empty() {
        // Initial/full rebuilds have no retained row lower bound, but completed dates still need
        // the same canonical proof before their candidate can be materialized. This also lets a
        // queued repair recover after an earlier failure left the long-term tables empty.
        let mut blocked_recomputed_dates = HashSet::new();
        for date in &recomputed_dates {
            if *date >= today {
                continue;
            }
            let (candidate_daily, candidate_hourly) =
                long_term_candidate_integrity(*date, &hourly, &daily);
            let date_string = date.to_string();
            match load_long_term_integrity_oracle(pool, *date).await? {
                Some(oracle) => {
                    let candidate_is_empty = !daily
                        .keys()
                        .any(|(bucket_date, _, _)| bucket_date == &date_string)
                        && !hourly.keys().any(|(bucket_start_epoch, _, _)| {
                            long_term_bucket_date(*bucket_start_epoch) == Some(*date)
                        });
                    let initial_complete_snapshot_without_hourly_proof = initial_materialization
                        && oracle.hourly.is_empty()
                        && !archive_read_failed
                        && !terminal_proof_reconciliation_incomplete
                        && *date >= reconstructable_start;
                    if initial_complete_snapshot_without_hourly_proof {
                        // An empty canonical table is not a zero-value proof. During the first
                        // complete source scan, however, the snapshot itself is the bootstrap
                        // evidence until hourly canonical rows are generated.
                        if scheduled_repair_date == Some(*date) {
                            completed_integrity_repairs.insert(*date);
                        }
                    } else if scheduled_repair_date == Some(*date) && candidate_is_empty {
                        // A durable repair queue represents an explicitly invalidated date. A
                        // complete empty source is a valid replacement even when the prior
                        // canonical proof still contains the stale pre-repair totals.
                        completed_integrity_repairs.insert(*date);
                    } else if let Some(mismatch) = long_term_integrity_mismatch(
                        oracle.date,
                        oracle.daily,
                        &oracle.hourly,
                        candidate_daily,
                        &candidate_hourly,
                    ) {
                        warn!(
                            stats_date = %mismatch.date,
                            reason = %mismatch.reason,
                            "long-term stats full rebuild cannot prove complete"
                        );
                        blocked_recomputed_dates.insert(*date);
                        integrity_repair_failures.push(mismatch);
                    } else if scheduled_repair_date == Some(*date) {
                        completed_integrity_repairs.insert(*date);
                    }
                }
                None if initial_materialization
                    && !archive_read_failed
                    && !terminal_proof_reconciliation_incomplete
                    && *date >= reconstructable_start =>
                {
                    // The initial pass reads every retained live and archive source before
                    // publishing. That complete snapshot is sufficient bootstrap evidence until
                    // the canonical hourly proof is reconciled; retired source prefixes remain
                    // outside the reconstructable window and cannot take this path.
                    if scheduled_repair_date == Some(*date) {
                        completed_integrity_repairs.insert(*date);
                    }
                }
                None if scheduled_repair_date == Some(*date) => {
                    blocked_recomputed_dates.insert(*date);
                    if let Some(mismatch) = queued_long_term_repair_mismatch(
                        pool,
                        *date,
                        candidate_daily,
                        "canonical hourly integrity evidence is unavailable for the queued repair",
                    )
                    .await?
                    {
                        blocked_recomputed_dates.insert(*date);
                        integrity_repair_failures.push(mismatch);
                    }
                }
                None => {
                    // Historical source rows are not sufficient proof by themselves. This is
                    // especially important after archive cleanup, where an incomplete source
                    // prefix can otherwise look like an empty or smaller completed day.
                    blocked_recomputed_dates.insert(*date);
                    integrity_repair_failures.push(LongTermIntegrityMismatch {
                        date: *date,
                        // There is no canonical expectation to persist yet. Keep the observed
                        // candidate for retry bookkeeping; a future trusted proof replaces it.
                        expected: candidate_daily,
                        observed: candidate_daily,
                        reason: "canonical hourly integrity evidence is unavailable for the full rebuild".to_string(),
                    });
                }
            }
        }
        remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_recomputed_dates);
        recomputed_dates.retain(|date| !blocked_recomputed_dates.contains(date));
    }
    if ready_state && !recomputed_dates.is_empty() {
        if let Some(previous_date) = recomputed_dates
            .iter()
            .min()
            .copied()
            .and_then(|date| date.pred_opt())
        {
            recomputed_dates.insert(previous_date);
        }
        // Replacing a date can alter the preceding date when an invocation crosses midnight.
        // Load every overlapping attempt archive only after that final rebuild range is known;
        // a missing archive is a source-integrity failure, not an empty account mapping.
        if let Some((start, end)) = recomputed_dates
            .iter()
            .min()
            .copied()
            .zip(recomputed_dates.iter().max().copied())
        {
            let (rebuild_attempt_accounts, rebuild_attempt_markers) =
                load_long_term_archive_attempt_accounts(pool, Some((start, end))).await?;
            archive_attempt_accounts.extend(rebuild_attempt_accounts);
            attempt_archive_markers.extend(rebuild_attempt_markers);
        }
        let mut rebuild_rows = rows
            .iter()
            .filter(|row| {
                parse_long_term_timestamp_ms(&row.occurred_at)
                    .and_then(|timestamp| {
                        Shanghai
                            .timestamp_millis_opt(timestamp)
                            .single()
                            .map(|value| recomputed_dates.contains(&value.date_naive()))
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut rebuild_seen_ids = rebuild_rows
            .iter()
            .map(|row| row.id)
            .collect::<HashSet<_>>();
        let min_date = recomputed_dates.iter().min().copied();
        let max_date = recomputed_dates.iter().max().copied();
        if let (Some(min_date), Some(max_date)) = (min_date, max_date) {
            let live_start = min_date.and_hms_opt(0, 0, 0).map(format_naive);
            let live_end = max_date
                .succ_opt()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(format_naive);
            let live_rebuild_sql = format!(
                r#"
                SELECT
                    inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
                    {live_upstream_account_id_sql} AS upstream_account_id,
                    NULL AS upstream_account_kind, NULL AS upstream_account_name,
                    inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms,
                    inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms,
                    inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message
                FROM codex_invocations inv
                WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
                  AND inv.occurred_at < ?2
                  AND (
                      inv.occurred_at >= ?1
                      OR (
                          inv.t_total_ms IS NOT NULL
                          AND inv.t_total_ms > 0
                          AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)
                      )
                  )
                ORDER BY inv.occurred_at ASC, inv.id ASC
                "#,
                live_upstream_account_id_sql = live_upstream_account_id_sql,
            );
            if let (Some(live_start), Some(live_end)) = (live_start, live_end) {
                let live_rebuild_rows =
                    sqlx::query_as::<_, LongTermInvocationRow>(&live_rebuild_sql)
                        .bind(live_start)
                        .bind(live_end)
                        .fetch_all(pool)
                        .await?;
                for row in live_rebuild_rows {
                    if rebuild_seen_ids.insert(row.id) {
                        rebuild_rows.push(row);
                    }
                }
            }
        }
        for archive_path in all_archive_paths {
            let overlaps = match (
                archive_path
                    .coverage_start_at()
                    .and_then(long_term_archive_end_date),
                archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date),
            ) {
                (Some(start), Some(end)) => recomputed_dates
                    .iter()
                    .any(|date| *date >= start && *date <= end),
                _ => true,
            };
            if !overlaps {
                continue;
            }
            let archive_sha256 = load_long_term_archive_sha256(pool, archive_path.file_path())
                .await?
                .context("completed invocation archive has no manifest sha256")?;
            ensure_long_term_archive_source_identity(
                pool,
                "codex_invocations",
                archive_path.file_path(),
                &archive_sha256,
            )
            .await?;
            let Some((archive_pool, cleanup)) =
                open_invocation_archive_batch_pool(&archive_path, "long-term-stats-rebuild")
                    .await?
            else {
                continue;
            };
            let archive_query = long_term_archive_invocation_query(&archive_pool).await?;
            let archive_rows = sqlx::query_as::<_, LongTermInvocationRow>(&archive_query)
                .fetch_all(&archive_pool)
                .await?;
            archive_pool.close().await;
            drop(cleanup);
            ensure_long_term_archive_source_identity(
                pool,
                "codex_invocations",
                archive_path.file_path(),
                &archive_sha256,
            )
            .await?;
            for mut row in archive_rows {
                hydrate_long_term_archive_attempt_account(&mut row, &archive_attempt_accounts);
                if rebuild_seen_ids.insert(row.id) {
                    rebuild_rows.push(row);
                }
            }
        }
        let mut rebuilt_hourly = HashMap::new();
        let mut rebuilt_daily = HashMap::new();
        let mut rebuilt_start = None;
        for mut row in rebuild_rows {
            hydrate_long_term_account_identity(&mut row, &account_identities);
            accumulate_long_term_invocation(
                &row,
                &mut rebuilt_hourly,
                &mut rebuilt_daily,
                &mut rebuilt_start,
            );
        }
        // A retained daily row is durable evidence that older source parts contributed to this
        // date. If the current archive inventory can only reproduce fewer calls, keep the
        // existing date intact instead of replacing it with a partial reconstruction. For
        // completed days, the canonical hourly rollup is the stricter completeness witness.
        let mut blocked_recomputed_dates = HashSet::new();
        for date in &recomputed_dates {
            let date_string = date.to_string();
            let (candidate_daily, candidate_hourly) =
                long_term_candidate_integrity(*date, &rebuilt_hourly, &rebuilt_daily);
            let persisted_calls = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(&date_string)
            .fetch_one(pool)
            .await?;
            if *date < today {
                match load_long_term_integrity_oracle(pool, *date).await? {
                    Some(oracle) => {
                        if let Some(mismatch) = long_term_integrity_mismatch(
                            oracle.date,
                            oracle.daily,
                            &oracle.hourly,
                            candidate_daily,
                            &candidate_hourly,
                        ) {
                            warn!(
                                stats_date = %mismatch.date,
                                reason = %mismatch.reason,
                                "long-term stats rebuild cannot prove complete"
                            );
                            blocked_recomputed_dates.insert(*date);
                            integrity_repair_failures.push(mismatch);
                        } else if scheduled_repair_date == Some(*date) {
                            // Canonical zero totals are valid proof too: replace all dimensions
                            // with an empty candidate rather than leaving a stale nonzero date.
                            completed_integrity_repairs.insert(*date);
                        }
                    }
                    None => {
                        blocked_recomputed_dates.insert(*date);
                        if scheduled_repair_date == Some(*date)
                            && let Some(mismatch) = queued_long_term_repair_mismatch(
                                pool,
                                *date,
                                candidate_daily,
                                "canonical hourly integrity evidence is unavailable for the queued repair",
                            )
                            .await?
                        {
                            integrity_repair_failures.push(mismatch);
                        }
                    }
                }
                continue;
            }
            if persisted_calls > candidate_daily.calls {
                blocked_recomputed_dates.insert(*date);
                if scheduled_repair_date == Some(*date)
                    && let Some(mismatch) = queued_long_term_repair_mismatch(
                        pool,
                        *date,
                        candidate_daily,
                        format!(
                            "candidate calls={} is below retained calls={persisted_calls}; source reconstruction is incomplete",
                            candidate_daily.calls
                        ),
                    )
                    .await?
                {
                    integrity_repair_failures.push(mismatch);
                }
            } else if scheduled_repair_date == Some(*date)
                && let Some(mismatch) = queued_long_term_repair_mismatch(
                    pool,
                    *date,
                    candidate_daily,
                    "canonical hourly integrity evidence is unavailable for the queued repair",
                )
                .await?
            {
                blocked_recomputed_dates.insert(*date);
                integrity_repair_failures.push(mismatch);
            }
        }
        if !blocked_recomputed_dates.is_empty() {
            remove_long_term_candidate_dates(
                &mut rebuilt_hourly,
                &mut rebuilt_daily,
                &blocked_recomputed_dates,
            );
            // The partial live candidate was built before the full-date reconstruction. Drop it
            // as well, otherwise its UPSERT would still overwrite the durable rollup.
            remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_recomputed_dates);
            recomputed_dates.retain(|date| !blocked_recomputed_dates.contains(date));
        }
        hourly.retain(|(bucket_start, _, _), _| {
            Shanghai
                .timestamp_opt(*bucket_start, 0)
                .single()
                .map(|value| !recomputed_dates.contains(&value.date_naive()))
                .unwrap_or(true)
        });
        daily.retain(|(date, _, _), _| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|value| !recomputed_dates.contains(&value))
                .unwrap_or(true)
        });
        hourly.extend(rebuilt_hourly);
        daily.extend(rebuilt_daily);
        if let Some(rebuilt_start) = rebuilt_start
            && statistics_start_date
                .as_deref()
                .is_none_or(|current| rebuilt_start.as_str() < current)
        {
            statistics_start_date = Some(rebuilt_start);
        }
    }

    apply_long_term_refresh_rollups_with_control(
        pool,
        LongTermRefreshRollupInput {
            hourly: &hourly,
            daily: &daily,
            recomputed_dates: &recomputed_dates,
            retention_start,
            integrity_repair_failures: &integrity_repair_failures,
            completed_integrity_repairs: &completed_integrity_repairs,
            reconstructable_start,
            statistics_start_date: statistics_start_date.as_deref(),
            initial_materialization,
            processed_rows_count: if ready_state {
                rows.len() as i64
            } else {
                processed_rows_count
            },
            source_rows_empty: rows.is_empty(),
            archive_read_failed,
            terminal_proof_reconciliation_incomplete,
            archive_markers: &archive_markers,
            failed_archive_paths: &failed_archive_paths,
            clear_all_attempt_markers,
            failed_archive_ranges: &failed_archive_ranges,
            attempt_archive_markers: &attempt_archive_markers,
        },
        control,
    )
    .await
}

pub(super) async fn delete_initial_long_term_rollups_for_date(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
        return Ok(());
    };
    for statement in [
        "DELETE FROM long_term_usage_daily WHERE rowid IN (SELECT rowid FROM long_term_usage_daily WHERE stats_date = ?1 LIMIT ?2)",
        "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2 LIMIT ?3)",
    ] {
        loop {
            let (mut transaction, permit) = control.begin(pool).await?;
            let mut query = sqlx::query(statement);
            if statement.contains("bucket_start_epoch") {
                query = query
                    .bind(start_epoch)
                    .bind(end_epoch)
                    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64);
            } else {
                query = query
                    .bind(date.to_string())
                    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64);
            }
            let deleted = query.execute(&mut *transaction).await?.rows_affected();
            control.commit(transaction, permit).await?;
            if deleted == 0 {
                break;
            }
        }
    }
    Ok(())
}

pub(super) struct LongTermRefreshRollupInput<'a> {
    pub(super) hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    pub(super) daily: &'a HashMap<(String, String, String), LongTermBucket>,
    pub(super) recomputed_dates: &'a HashSet<NaiveDate>,
    pub(super) retention_start: NaiveDate,
    pub(super) integrity_repair_failures: &'a [LongTermIntegrityMismatch],
    pub(super) completed_integrity_repairs: &'a HashSet<NaiveDate>,
    pub(super) reconstructable_start: NaiveDate,
    pub(super) statistics_start_date: Option<&'a str>,
    pub(super) initial_materialization: bool,
    pub(super) processed_rows_count: i64,
    pub(super) source_rows_empty: bool,
    pub(super) archive_read_failed: bool,
    pub(super) terminal_proof_reconciliation_incomplete: bool,
    pub(super) archive_markers: &'a [(String, String)],
    pub(super) failed_archive_paths: &'a HashSet<String>,
    pub(super) clear_all_attempt_markers: bool,
    pub(super) failed_archive_ranges: &'a [(String, String)],
    pub(super) attempt_archive_markers: &'a HashSet<(String, String)>,
}

pub(super) struct LongTermRefreshArchiveMarkers<'a> {
    pub(super) archive_markers: &'a [(String, String)],
    pub(super) archive_read_failed: bool,
    pub(super) failed_archive_paths: &'a HashSet<String>,
    pub(super) clear_all_attempt_markers: bool,
    pub(super) failed_archive_ranges: &'a [(String, String)],
    pub(super) attempt_archive_markers: &'a HashSet<(String, String)>,
}

pub(super) async fn apply_long_term_refresh_rollups_with_control(
    pool: &Pool<Sqlite>,
    input: LongTermRefreshRollupInput<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let LongTermRefreshRollupInput {
        hourly,
        daily,
        recomputed_dates,
        retention_start,
        integrity_repair_failures,
        completed_integrity_repairs,
        reconstructable_start,
        statistics_start_date,
        initial_materialization,
        processed_rows_count,
        source_rows_empty,
        archive_read_failed,
        terminal_proof_reconciliation_incomplete,
        archive_markers,
        failed_archive_paths,
        clear_all_attempt_markers,
        failed_archive_ranges,
        attempt_archive_markers,
    } = input;
    let has_persisted_daily_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0;
    // A pending initial materialization has no publishable baseline. If its only partial prefix
    // is retried against a successfully empty source, clear it before reporting `empty` rather
    // than promoting stale rows to a completed initial snapshot.
    let clears_pending_empty_initial_materialization = initial_materialization
        && source_rows_empty
        && !archive_read_failed
        && !terminal_proof_reconciliation_incomplete;
    let mut refresh_backups = Vec::with_capacity(recomputed_dates.len());
    for date in recomputed_dates {
        let bucket_date = date.to_string();
        // P2 and the initial refresher intentionally share the durable owner for a date. If a
        // P2 snapshot won the race just before the initial marker was persisted, the refresher
        // can finish and release that same last-good backup instead of being stranded by it.
        let rebuild_token = format!("long-term-date:{bucket_date}");
        // The full refresher also writes in bounded transactions. Keep the same durable daily
        // snapshot used by a P2 date rebuild so pressure or shutdown cannot leave a deleted
        // prefix as the only recoverable state.
        ensure_long_term_projection_daily_backup_for_date(
            pool,
            &bucket_date,
            &rebuild_token,
            false,
            control,
        )
        .await?;
        refresh_backups.push((bucket_date, rebuild_token));
    }
    if recomputed_dates.is_empty()
        && (!has_persisted_daily_rows || clears_pending_empty_initial_materialization)
    {
        for table in ["long_term_usage_hourly", "long_term_usage_daily"] {
            loop {
                let (mut transaction, permit) = control.begin(pool).await?;
                let deleted = sqlx::query(&format!(
                    "DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} LIMIT ?1)"
                ))
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .execute(&mut *transaction)
                .await?
                .rows_affected();
                control.commit(transaction, permit).await?;
                if deleted == 0 {
                    break;
                }
            }
        }
    } else {
        for date in recomputed_dates {
            delete_initial_long_term_rollups_for_date(pool, *date, control).await?;
        }
    }

    let retention_start_epoch = retention_start
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .unwrap_or(i64::MIN);
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT ?2)",
        )
        .bind(retention_start_epoch)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            break;
        }
    }

    let hourly = hourly
        .values()
        .filter(|bucket| {
            Shanghai
                .timestamp_opt(bucket.bucket_start_epoch, 0)
                .single()
                .is_some_and(|value| value.date_naive() >= retention_start)
        })
        .collect::<Vec<_>>();
    for batch in hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_hourly(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let daily = daily.values().collect::<Vec<_>>();
    for batch in daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_daily(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let mut retry_dates = HashSet::new();
    for mismatch in integrity_repair_failures
        .iter()
        .filter(|mismatch| retry_dates.insert(mismatch.date))
    {
        let (mut transaction, permit) = control.begin(pool).await?;
        schedule_long_term_repair_retry(&mut transaction, mismatch).await?;
        control.commit(transaction, permit).await?;
    }
    // The planner selects one due repair date per refresh. Keep this final publication write
    // bounded even if a future caller accidentally changes that scheduling contract.
    if completed_integrity_repairs.len() > 1 {
        bail!("long-term refresh may complete at most one integrity repair per publication");
    }
    // A completed repair is not durable until its candidate has been published. Account for it
    // while choosing the public state, but retain its queue entry until the final publication
    // transaction so cancellation cannot strand an unpublished backup without a retry path.
    let completed_integrity_repair_dates = completed_integrity_repairs
        .iter()
        .filter(|date| **date >= reconstructable_start)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut pending_integrity_repairs_query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date >= ",
    );
    pending_integrity_repairs_query.push_bind(reconstructable_start.to_string());
    if !completed_integrity_repair_dates.is_empty() {
        pending_integrity_repairs_query.push(" AND stats_date NOT IN (");
        let mut dates = pending_integrity_repairs_query.separated(", ");
        for date in &completed_integrity_repair_dates {
            dates.push_bind(date);
        }
        dates.push_unseparated(")");
    }
    let pending_integrity_repairs = pending_integrity_repairs_query
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await?;
    let status = if pending_integrity_repairs > 0
        || archive_read_failed
        || terminal_proof_reconciliation_incomplete
    {
        LONG_TERM_STATUS_ERROR
    } else if source_rows_empty
        && daily.is_empty()
        && (!has_persisted_daily_rows || clears_pending_empty_initial_materialization)
    {
        LONG_TERM_STATUS_EMPTY
    } else {
        LONG_TERM_STATUS_READY
    };
    let last_error = if status == LONG_TERM_STATUS_ERROR && initial_materialization {
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR.to_string())
    } else if terminal_proof_reconciliation_incomplete {
        Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR.to_string())
    } else if pending_integrity_repairs > 0 {
        Some(format!(
            "long-term integrity repair pending for {pending_integrity_repairs} date(s)"
        ))
    } else if archive_read_failed {
        Some("one or more invocation archives could not be materialized".to_string())
    } else {
        None
    };
    control.complete_integrity_repairs();
    control.check()?;
    // Stage every replacement behind one publication token. The final transaction below flips
    // this token and the public state together, so cancellation cannot expose a mixed refresh.
    let refresh_publication_token =
        (!refresh_backups.is_empty()).then(next_long_term_projection_publication_token);
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        for (bucket_date, rebuild_token) in &refresh_backups {
            stage_long_term_projection_date_publication(
                pool,
                bucket_date,
                rebuild_token,
                publication_token,
                None,
                control,
            )
            .await?;
        }
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1) ON CONFLICT(publication_token) DO UPDATE SET published = 1, updated_at = datetime('now')",
        )
        .bind(publication_token)
        .execute(&mut *transaction)
        .await?;
    }
    for date in completed_integrity_repairs {
        sqlx::query("DELETE FROM long_term_stats_repair_queue WHERE stats_date = ?1")
            .bind(date.to_string())
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, processed_rows = ?3, total_rows = ?3, last_error = ?4, updated_at = datetime('now') WHERE id = ?5",
    )
    .bind(status)
    .bind(statistics_start_date)
    .bind(processed_rows_count)
    .bind(last_error)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    for date in completed_integrity_repairs {
        info!(stats_date = %date, "long-term stats integrity repair completed");
    }
    control.complete_refresh_publication();
    control.check()?;
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        for (bucket_date, rebuild_token) in &refresh_backups {
            let _released = release_long_term_projection_publication_member(
                pool,
                bucket_date,
                rebuild_token,
                None,
                publication_token,
                control,
            )
            .await?;
        }
        prune_long_term_projection_publications(pool, control).await?;
    }
    // A retry may safely rebuild an already-published date. It cannot safely skip an archive
    // while its replay marker exists but the replacement is still behind publication cleanup.
    persist_long_term_refresh_archive_markers_with_control(
        pool,
        LongTermRefreshArchiveMarkers {
            archive_markers,
            archive_read_failed,
            failed_archive_paths,
            clear_all_attempt_markers,
            failed_archive_ranges,
            attempt_archive_markers,
        },
        control,
    )
    .await?;
    Ok(())
}

pub(super) async fn persist_long_term_refresh_archive_markers_with_control(
    pool: &Pool<Sqlite>,
    markers: LongTermRefreshArchiveMarkers<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let LongTermRefreshArchiveMarkers {
        archive_markers,
        archive_read_failed,
        failed_archive_paths,
        clear_all_attempt_markers,
        failed_archive_ranges,
        attempt_archive_markers,
    } = markers;
    for (file_path, archive_sha256) in archive_markers {
        persist_long_term_refresh_archive_marker_with_control(
            pool,
            "codex_invocations",
            file_path,
            archive_sha256,
            control,
        )
        .await?;
    }

    if archive_read_failed {
        for failed_archive_path in failed_archive_paths {
            let (mut transaction, permit) = control.begin(pool).await?;
            sqlx::query(
                "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .bind(failed_archive_path)
            .execute(&mut *transaction)
            .await?;
            control.commit(transaction, permit).await?;
        }
        if clear_all_attempt_markers {
            delete_long_term_refresh_replay_markers_with_control(
                pool,
                "DELETE FROM hourly_rollup_archive_replay WHERE rowid IN (SELECT rowid FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' LIMIT ?2)",
                &[LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string()],
                control,
            )
            .await?;
        } else {
            for (failed_start, failed_end) in failed_archive_ranges {
                delete_long_term_refresh_replay_markers_with_control(
                    pool,
                    r#"
                    DELETE FROM hourly_rollup_archive_replay
                    WHERE rowid IN (
                        SELECT replay.rowid
                        FROM hourly_rollup_archive_replay replay
                        WHERE replay.target = ?1
                          AND replay.dataset = 'pool_upstream_request_attempts'
                          AND EXISTS (
                              SELECT 1
                              FROM archive_batches attempts
                              WHERE attempts.dataset = 'pool_upstream_request_attempts'
                                AND attempts.file_path = replay.file_path
                                AND (attempts.coverage_end_at IS NULL OR attempts.coverage_end_at >= ?2)
                                AND (attempts.coverage_start_at IS NULL OR attempts.coverage_start_at <= ?3)
                          )
                        LIMIT ?4
                    )
                    "#,
                    &[
                        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string(),
                        failed_start.clone(),
                        failed_end.clone(),
                    ],
                    control,
                )
                .await?;
            }
        }
    } else {
        for (file_path, archive_sha256) in attempt_archive_markers {
            persist_long_term_refresh_archive_marker_with_control(
                pool,
                "pool_upstream_request_attempts",
                file_path,
                archive_sha256,
                control,
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn persist_long_term_refresh_archive_marker_with_control(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    archive_sha256: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let result = sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256, source_identity, replayed_at)
        SELECT ?1, ?2, ?3, ?4, NULL, strftime('%Y-%m-%d %H:%M:%f', 'now')
        WHERE EXISTS (
            SELECT 1
            FROM archive_batches
            WHERE dataset = ?2
              AND status = 'completed'
              AND file_path = ?3
              AND sha256 = ?4
        )
        ON CONFLICT(target, dataset, file_path) DO UPDATE SET
            archive_sha256 = excluded.archive_sha256,
            source_identity = NULL,
            replayed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
        "#,
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .bind(archive_sha256)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if result.rows_affected() != 1 {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive manifest changed before its replay marker could be persisted: {file_path}"
        );
    }
    let source_identity_before = match long_term_archive_file_identity(file_path) {
        Ok(source_identity) => source_identity,
        Err(error) => {
            delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
                .await?;
            return Err(error.context(format!(
                "long-term archive source changed before its replay marker could be persisted: {file_path}"
            )));
        }
    };
    if let Err(error) =
        ensure_long_term_archive_source_identity(pool, dataset, file_path, archive_sha256).await
    {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        return Err(error.context(format!(
            "long-term archive source changed before its replay marker could be persisted: {file_path}"
        )));
    }
    let source_identity_after = match long_term_archive_file_identity(file_path) {
        Ok(source_identity) => source_identity,
        Err(error) => {
            delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
                .await?;
            return Err(error.context(format!(
                "long-term archive source changed before its replay marker could be persisted: {file_path}"
            )));
        }
    };
    if source_identity_before != source_identity_after {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive source changed before its replay marker could be persisted: {file_path}"
        );
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    let result = sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET source_identity = ?1,
            replayed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
        WHERE target = ?2
          AND dataset = ?3
          AND file_path = ?4
          AND archive_sha256 = ?5
          AND EXISTS (
              SELECT 1
              FROM archive_batches
              WHERE dataset = ?3
                AND status = 'completed'
                AND file_path = ?4
                AND sha256 = ?5
          )
        "#,
    )
    .bind(&source_identity_after)
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .bind(archive_sha256)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if result.rows_affected() != 1 {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive manifest changed before its replay marker could be finalized: {file_path}"
        );
    }
    Ok(())
}

pub(super) async fn delete_long_term_refresh_archive_marker_with_control(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(super) async fn delete_long_term_refresh_replay_markers_with_control(
    pool: &Pool<Sqlite>,
    sql: &str,
    bindings: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let mut query = sqlx::query(sql);
        for binding in bindings {
            query = query.bind(binding);
        }
        let deleted = query
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermAccountIdentity {
    pub(super) id: i64,
    pub(super) kind: String,
    pub(super) display_name: String,
}

pub(super) async fn load_long_term_account_identities(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<i64, LongTermAccountIdentity>> {
    let result = sqlx::query_as::<_, LongTermAccountIdentity>(
        "SELECT id, kind, display_name FROM pool_upstream_accounts",
    )
    .fetch_all(pool)
    .await;
    match result {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|identity| (identity.id, identity))
            .collect()),
        Err(error) if error.to_string().contains("no such table") => Ok(HashMap::new()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn long_term_archive_end_date(raw: &str) -> Option<NaiveDate> {
    parse_long_term_timestamp_ms(raw).and_then(|timestamp| {
        Shanghai
            .timestamp_millis_opt(timestamp)
            .single()
            .map(|value| value.date_naive())
    })
}

pub(super) fn insert_long_term_date_range(
    dates: &mut HashSet<NaiveDate>,
    start: Option<&str>,
    end: Option<&str>,
) {
    let (Some(start), Some(end)) = (
        start.and_then(long_term_archive_end_date),
        end.and_then(long_term_archive_end_date),
    ) else {
        return;
    };
    let mut date = start;
    while date <= end {
        dates.insert(date);
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
}

pub(super) fn hydrate_long_term_account_identity(
    row: &mut LongTermInvocationRow,
    account_identities: &HashMap<i64, LongTermAccountIdentity>,
) {
    if row.upstream_account_kind.is_none()
        && let Some(account_id) = row.upstream_account_id
        && let Some(identity) = account_identities.get(&account_id)
    {
        row.upstream_account_kind = Some(identity.kind.clone());
        row.upstream_account_name = Some(identity.display_name.clone());
    }
}

pub(super) fn merge_long_term_invocation_row(
    preferred: &mut LongTermInvocationRow,
    fallback: &LongTermInvocationRow,
) {
    if preferred.invoke_id.is_none() {
        preferred.invoke_id = fallback.invoke_id.clone();
    }
    if preferred.occurred_at.is_empty() {
        preferred.occurred_at.clone_from(&fallback.occurred_at);
    }
    if preferred.status.is_none() {
        preferred.status = fallback.status.clone();
    }
    if preferred.model.is_none() {
        preferred.model = fallback.model.clone();
    }
    if preferred.request_model.is_none() {
        preferred.request_model = fallback.request_model.clone();
    }
    if preferred.response_model.is_none() {
        preferred.response_model = fallback.response_model.clone();
    }
    if preferred.reasoning_effort.is_none() {
        preferred.reasoning_effort = fallback.reasoning_effort.clone();
    }
    if preferred.upstream_account_id.is_none() {
        preferred.upstream_account_id = fallback.upstream_account_id;
    }
    if preferred.upstream_account_kind.is_none() {
        preferred.upstream_account_kind = fallback.upstream_account_kind.clone();
    }
    if preferred.upstream_account_name.is_none() {
        preferred.upstream_account_name = fallback.upstream_account_name.clone();
    }
    if preferred.total_tokens.is_none() {
        preferred.total_tokens = fallback.total_tokens;
    }
    if preferred.output_tokens.is_none() {
        preferred.output_tokens = fallback.output_tokens;
    }
    if preferred.cost.is_none() {
        preferred.cost = fallback.cost;
    }
    if preferred.t_total_ms.is_none() {
        preferred.t_total_ms = fallback.t_total_ms;
    }
    if preferred.t_req_read_ms.is_none() {
        preferred.t_req_read_ms = fallback.t_req_read_ms;
    }
    if preferred.t_req_parse_ms.is_none() {
        preferred.t_req_parse_ms = fallback.t_req_parse_ms;
    }
    if preferred.t_upstream_connect_ms.is_none() {
        preferred.t_upstream_connect_ms = fallback.t_upstream_connect_ms;
    }
    if preferred.t_upstream_ttfb_ms.is_none() {
        preferred.t_upstream_ttfb_ms = fallback.t_upstream_ttfb_ms;
    }
    if preferred.t_upstream_stream_ms.is_none() {
        preferred.t_upstream_stream_ms = fallback.t_upstream_stream_ms;
    }
    if preferred.error_message.is_none() {
        preferred.error_message = fallback.error_message.clone();
    }
}

pub(super) fn accumulate_long_term_invocation(
    row: &LongTermInvocationRow,
    hourly: &mut HashMap<(i64, String, String), LongTermBucket>,
    daily: &mut HashMap<(String, String, String), LongTermBucket>,
    statistics_start_date: &mut Option<String>,
) {
    let Some(start) = parse_long_term_timestamp(&row.occurred_at) else {
        return;
    };
    let start_ms = start.epoch_ms;
    let Some(local_date) = Shanghai
        .timestamp_millis_opt(start_ms)
        .single()
        .map(|value| value.date_naive())
    else {
        return;
    };
    let date_string = local_date.to_string();
    if statistics_start_date
        .as_deref()
        .is_none_or(|current| date_string.as_str() < current)
    {
        *statistics_start_date = Some(date_string.clone());
    }
    let model = normalize_long_term_model(row);
    let reasoning = normalize_long_term_reasoning(row.reasoning_effort.as_deref());
    let upstream = normalize_long_term_upstream(row);
    let dimensions = [
        (
            "overall",
            "overall".to_string(),
            "全部".to_string(),
            String::new(),
        ),
        (
            "model",
            long_term_model_series_key(&model, &reasoning),
            model,
            reasoning,
        ),
        ("upstream", upstream.0, upstream.1, String::new()),
    ];
    let interval_end_ms = row
        .t_total_ms
        .filter(|value| value.is_finite() && *value > 0.0)
        .and_then(|value| long_term_interval_end_ms(start, value));
    for (dimension, series_key, display_name, reasoning_effort) in dimensions {
        let interval = if is_success_status(row.status.as_deref(), row.error_message.as_deref()) {
            interval_end_ms.map(|end_ms| (start_ms, end_ms))
        } else {
            None
        };
        add_long_term_row(
            hourly,
            dimension,
            &series_key,
            &display_name,
            &reasoning_effort,
            start_ms,
            row,
            interval,
        );
        add_long_term_daily_row(
            daily,
            dimension,
            &series_key,
            &display_name,
            &reasoning_effort,
            &date_string,
            row,
            interval,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
pub(super) fn add_long_term_row(
    buckets: &mut HashMap<(i64, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    start_ms: i64,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let hour_start_ms = start_ms.div_euclid(LONG_TERM_HOUR_MS) * LONG_TERM_HOUR_MS;
    let original_interval = interval;
    let interval = interval.and_then(|(start, end)| {
        let hour_end = hour_start_ms + LONG_TERM_HOUR_MS;
        let clipped_start = start.max(hour_start_ms);
        let clipped_end = end.min(hour_end);
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    let key = (
        hour_start_ms / 1000,
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: hour_start_ms / 1000,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: None,
        accumulator: LongTermAccumulator::default(),
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let mut next_hour_start = hour_start_ms + LONG_TERM_HOUR_MS;
    while next_hour_start < interval_end {
        let segment_start = next_hour_start.max(interval_start);
        let segment_end = (next_hour_start + LONG_TERM_HOUR_MS).min(interval_end);
        if segment_end > segment_start {
            let key = (
                next_hour_start / 1000,
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: next_hour_start / 1000,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: None,
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        next_hour_start += LONG_TERM_HOUR_MS;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
pub(super) fn add_long_term_daily_row(
    buckets: &mut HashMap<(String, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    date: &str,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let original_interval = interval;
    let key = (
        date.to_string(),
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: 0,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: Some(date.to_string()),
        accumulator: LongTermAccumulator::default(),
    });
    let interval = interval.and_then(|(start, end)| {
        let day_start = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())?;
        let day_end = day_start.checked_add_signed(ChronoDuration::days(1))?;
        let clipped_start = start.max(day_start.timestamp_millis());
        let clipped_end = end.min(day_end.timestamp_millis());
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let Some(mut next_date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|value| value.succ_opt())
    else {
        return;
    };
    loop {
        let Some(day_start) = next_date
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())
        else {
            return;
        };
        if day_start.timestamp_millis() >= interval_end {
            break;
        }
        let day_end = day_start
            .checked_add_signed(ChronoDuration::days(1))
            .map(|value| value.timestamp_millis())
            .unwrap_or(interval_end);
        let segment_start = interval_start.max(day_start.timestamp_millis());
        let segment_end = interval_end.min(day_end);
        if segment_end > segment_start {
            let date_string = next_date.to_string();
            let key = (
                date_string.clone(),
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: 0,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: Some(date_string),
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        if segment_end >= interval_end {
            break;
        }
        let Some(next) = next_date.succ_opt() else {
            break;
        };
        next_date = next;
    }
}
