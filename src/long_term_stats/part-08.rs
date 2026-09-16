pub(crate) fn spawn_long_term_stats_backfill(
    pool: Pool<Sqlite>,
    retention_days: u64,
    shutdown: CancellationToken,
) {
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
            let control = LongTermProjectionWriteControl::background(
                &shutdown,
                crate::db_pressure::global_db_pressure_gate(),
            );
            if let Err(error) =
                mark_long_term_stats_backfill_preparing_with_control(&pool, &control).await
            {
                warn!(error = %error, "failed to mark long-term initial materialization preparing");
            } else {
                if shutdown.is_cancelled() {
                    break;
                }
                if let Err(error) =
                    refresh_long_term_stats_with_control(&pool, retention_days, &control).await
                {
                    if long_term_projection_write_is_deferred(&error) {
                        debug!(error = %error, "long-term initial materialization deferred by database pressure");
                    } else {
                        warn!(error = %error, "long-term initial materialization failed");
                    }
                }
            }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {}
            }
        }
    });
}

pub(crate) async fn refresh_long_term_stats(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    refresh_long_term_stats_with_control(pool, retention_days, &control).await
}

async fn refresh_long_term_stats_with_control(
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

async fn persist_long_term_refresh_progress(
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

async fn run_long_term_refresh_with_retry<T, Operation, OperationFuture>(
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

async fn run_long_term_refresh_with_retry_delays<T, Operation, OperationFuture>(
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

fn long_term_refresh_start_state(
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

fn long_term_refresh_pending_marker(
    was_ready: bool,
    starting_status: &str,
) -> Option<&'static str> {
    (!was_ready && starting_status != LONG_TERM_STATUS_ERROR)
        .then_some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
}

async fn refresh_long_term_stats_once(
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
