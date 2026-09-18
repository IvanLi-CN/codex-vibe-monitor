#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionCursorRow {
    cursor_row_id: i64,
}

pub(crate) fn spawn_long_term_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    spawn_long_term_stats_backfill(
        state.pool.clone(),
        state.config.long_term_stats_hourly_retention_days,
        cancel.clone(),
    );
    tokio::spawn(async move {
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let mut pressure_eligibility_generation = pressure_gate.eligibility_generation();
        let mut pressure_retry_pending = false;
        let mut pressure_retry_at = None;
        let (mut flush_ticker, mut maintenance_ticker, mut repair_ticker, mut daily_verify_ticker) =
            long_term_projection_supervisor_tickers().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = wait_for_long_term_projection_pressure_retry(
                    pressure_gate,
                    pressure_eligibility_generation,
                    pressure_retry_at,
                ), if pressure_retry_pending => {
                    pressure_eligibility_generation = pressure_gate.eligibility_generation();
                    settle_long_term_projection_flush(
                        &state,
                        "pressure_eligible",
                        flush_long_term_projection(&state, "pressure_eligible").await,
                        &mut pressure_retry_pending,
                        &mut pressure_retry_at,
                        true,
                        "long-term projection pressure retry failed",
                    ).await;
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
                        settle_long_term_projection_flush(
                            &state, trigger, flush_long_term_projection(&state, trigger).await,
                            &mut pressure_retry_pending, &mut pressure_retry_at, false,
                            "long-term projection flush failed",
                        ).await;
                    } else {
                        debug!(projection = "long_term", trigger = "terminal_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term projection flush");
                    }
                }
                _ = maintenance_ticker.tick() => {
                    handle_long_term_projection_maintenance_tick(
                        &state, pressure_gate, &mut pressure_eligibility_generation,
                        &mut pressure_retry_pending, &mut pressure_retry_at,
                    ).await;
                }
                _ = repair_ticker.tick() => {
                    match long_term_projection_repair_needed(&state).await {
                        Ok(true) => {
                            pressure_eligibility_generation = pressure_gate.eligibility_generation();
                            settle_long_term_projection_flush(
                                &state, "repair_deadline",
                                flush_long_term_projection(&state, "repair_deadline").await,
                                &mut pressure_retry_pending, &mut pressure_retry_at, false,
                                "long-term projection repair failed",
                            ).await;
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
                    settle_long_term_projection_flush(
                        &state, "daily_verify",
                        flush_long_term_projection(&state, "daily_verify").await,
                        &mut pressure_retry_pending, &mut pressure_retry_at, false,
                        "long-term projection daily verification failed",
                    ).await;
                }
            }
        }
    })
}

async fn handle_long_term_projection_maintenance_tick(
    state: &AppState,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    pressure_eligibility_generation: &mut u64,
    pressure_retry_pending: &mut bool,
    pressure_retry_at: &mut Option<Instant>,
) {
    match long_term_projection_maintenance_needed(
        &state.pool,
        state.config.long_term_stats_hourly_retention_days,
    )
    .await
    {
        Ok(true) => {
            *pressure_eligibility_generation = pressure_gate.eligibility_generation();
            settle_long_term_projection_flush(
                state,
                "maintenance_deadline",
                flush_long_term_projection(state, "maintenance_deadline").await,
                pressure_retry_pending,
                pressure_retry_at,
                false,
                "long-term projection maintenance failed",
            )
            .await;
        }
        Ok(false) => debug!(
            projection = "long_term",
            trigger = "maintenance_deadline",
            flush_outcome = "noop_suppressed",
            "skipping idle long-term projection maintenance"
        ),
        Err(error) => {
            mark_long_term_projection_failure(state, &error).await;
            warn!(error = %error, projection = "long_term", trigger = "maintenance_deadline", "failed to inspect long-term projection maintenance work");
        }
    }
}

async fn long_term_projection_supervisor_tickers() -> (
    tokio::time::Interval,
    tokio::time::Interval,
    tokio::time::Interval,
    tokio::time::Interval,
) {
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
    (
        flush_ticker,
        maintenance_ticker,
        repair_ticker,
        daily_verify_ticker,
    )
}

async fn settle_long_term_projection_flush(
    state: &AppState,
    trigger: &'static str,
    outcome: Result<LongTermProjectionFlushOutcome>,
    pressure_retry_pending: &mut bool,
    pressure_retry_at: &mut Option<Instant>,
    clear_retry_on_error: bool,
    failure_message: &'static str,
) {
    match outcome {
        Ok(LongTermProjectionFlushOutcome::Completed) => {
            *pressure_retry_pending = false;
            *pressure_retry_at = None;
        }
        Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
            *pressure_retry_pending = true;
            *pressure_retry_at = retry_at;
        }
        Err(error) => {
            if clear_retry_on_error {
                *pressure_retry_pending = false;
                *pressure_retry_at = None;
            }
            mark_long_term_projection_failure(state, &error).await;
            warn!(error = %error, projection = "long_term", trigger, "{failure_message}");
        }
    }
}

async fn long_term_projection_terminal_flush_needed(state: &AppState) -> bool {
    let has_persisted_work = state.terminal_projection_hub.has_persisted_work();
    let runtime = state.long_term_projection_runtime.lock().await;
    long_term_projection_terminal_flush_due(has_persisted_work, runtime.state.is_empty())
}

async fn long_term_projection_maintenance_needed(
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
    long_term_projection_publication_cleanup_needed(pool).await
}

async fn long_term_projection_publication_cleanup_needed(pool: &Pool<Sqlite>) -> Result<bool> {
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

fn long_term_projection_terminal_flush_due(
    has_persisted_work: bool,
    runtime_state_is_empty: bool,
) -> bool {
    // A deferred date rebuild must not suppress the bounded terminal delta pass.
    // The pass can still advance any ready prefix while the repair ticker owns
    // the expensive rebuild deadline.
    has_persisted_work || runtime_state_is_empty
}

async fn long_term_projection_repair_needed(state: &AppState) -> Result<bool> {
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

fn long_term_projection_repair_due(
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

async fn defer_long_term_projection_terminal_repair(state: &AppState, defer_reason: &'static str) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = "dirty_last_good".to_string();
    runtime.last_defer_reason = Some(defer_reason.to_string());
    runtime.next_repair_at = Some(long_term_projection_repair_deadline(
        runtime.next_repair_at,
        Instant::now(),
    ));
}

fn long_term_projection_repair_deadline(
    existing_deadline: Option<Instant>,
    now: Instant,
) -> Instant {
    // Repeated terminal flushes and pressure deferrals may arrive while a
    // targeted repair is pending. Keep the first deadline so they cannot
    // postpone recovery indefinitely.
    existing_deadline.unwrap_or(now + LONG_TERM_PROJECTION_REPAIR_INTERVAL)
}

fn long_term_projection_pressure_retry_at(
    gate: &crate::db_pressure::DbPressureGate,
) -> Option<Instant> {
    match gate.background_deny_reason() {
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms }) => {
            Some(Instant::now() + Duration::from_millis(remaining_ms.max(1)))
        }
        Some(crate::db_pressure::DbPressureDenyReason::BackgroundBusy) | None => None,
    }
}

async fn wait_for_long_term_projection_pressure_retry(
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

fn long_term_projection_allows_expensive_repair(trigger: &str) -> bool {
    matches!(trigger, "repair_deadline" | "daily_verify")
}

async fn mark_long_term_projection_failure(state: &AppState, error: &anyhow::Error) {
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

async fn queue_long_term_projection_daily_verify(state: &AppState) -> Result<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );
    queue_long_term_projection_daily_verify_with_control(&state.pool, &today, &control).await
}

async fn queue_long_term_projection_daily_verify_with_control(
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

async fn long_term_projection_daily_verify_due(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_state WHERE consumer = ?1 AND (daily_verify_pending = 1 OR last_daily_verify_at IS NULL OR date(last_daily_verify_at, '+8 hours') < date('now', '+8 hours')))",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_one(pool)
    .await?
        != 0)
}

fn long_term_projection_open_baseline_dates() -> Vec<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);
    vec![yesterday.to_string(), today.to_string()]
}

async fn queue_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    queue_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

async fn queue_long_term_projection_repairs_with_control(
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

async fn ensure_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    ensure_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

async fn ensure_long_term_projection_repairs_with_control(
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

async fn load_long_term_projection_dirty_buckets(
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

async fn long_term_projection_repairs_are_deferred(
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

async fn defer_long_term_projection_repair(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repair_with_control(pool, bucket_date, &control).await
}

async fn defer_long_term_projection_repair_with_control(
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

async fn defer_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repairs_with_control(pool, bucket_dates, &control).await
}

async fn defer_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for bucket_date in bucket_dates {
        defer_long_term_projection_repair_with_control(pool, bucket_date, control).await?;
    }
    Ok(())
}

async fn load_long_term_projection_ready_dates(
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

async fn load_long_term_projection_terminal_rows(
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

fn build_long_term_projection_event(row: &LongTermInvocationRow) -> LongTermProjectionEvent {
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

fn long_term_projection_canonical_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') = 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2"
    )
}

fn long_term_projection_crossing_text_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND inv.occurred_at < ?1 AND CASE WHEN instr(inv.occurred_at, 'T') = 0 AND inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 THEN julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 END >= julianday(?1)"
    )
}

async fn load_long_term_projection_live_rfc3339_compatibility(
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

fn long_term_projection_live_rfc3339_query(select: &str) -> String {
    let rfc3339_epoch = long_term_rfc3339_whole_epoch_seconds_sql("inv.occurred_at");
    let rfc3339_reaches_range_start =
        long_term_rfc3339_reaches_epoch_sql("inv.occurred_at", "inv.t_total_ms", "?3");
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') > 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2 AND {rfc3339_epoch} < ?4 AND ({rfc3339_epoch} >= ?3 OR (inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 AND {rfc3339_reaches_range_start}))"
    )
}

async fn invalidate_long_term_projection_interval_cache(state: &AppState) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.interval_index.clear();
    runtime.loaded_interval_dates.clear();
}

async fn flush_long_term_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<LongTermProjectionFlushOutcome> {
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = run_long_term_projection_flush_with_retry(&state.shutdown, || {
        flush_long_term_projection_inner(state, trigger)
    })
    .await;
    let load_row_count = result.as_ref().copied().unwrap_or_default();
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
