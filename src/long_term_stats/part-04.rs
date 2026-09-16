async fn run_long_term_projection_flush_with_retry<T, Operation, OperationFuture>(
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

async fn run_long_term_projection_flush_with_retry_delays<T, Operation, OperationFuture>(
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

async fn flush_long_term_projection_inner(state: &AppState, trigger: &'static str) -> Result<u64> {
    // The initial refresher and P2 share date backups for crash recovery, but never their live
    // replacement window. Defer P2 while the refresher owns this process-wide maintenance lock;
    // a later terminal wake or ticker retries the bounded work.
    let _refresh_guard = match LONG_TERM_REFRESH_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(0),
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
        return Ok(0);
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
                    return Ok(0);
                }
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
                return Ok(0);
            }
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
    Ok(loaded_row_count.saturating_add(event_count as u64))
}

async fn advance_long_term_projection_maintenance(
    state: &AppState,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<u64> {
    // Maintenance is intentionally a single durable continuation step. Each helper makes at
    // most one 512-row transaction and re-checks cancellation and database pressure before it
    // starts. Letting a deadline exhaust every backlog would recreate the writer starvation
    // this worker is intended to avoid.
    if finish_long_term_projection_publication_cleanup(&state.pool, control).await? {
        return Ok(0);
    }
    if migrate_long_term_projection_legacy_interval_state(&state.pool, control).await? {
        return Ok(0);
    }
    let (hourly, intervals) = prune_long_term_projection_hourly_retention_with_control(
        &state.pool,
        state.config.long_term_stats_hourly_retention_days,
        control,
    )
    .await?;
    Ok(hourly.saturating_add(intervals))
}

async fn long_term_rollups_exist(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0,
    )
}

fn long_term_initial_materialization_needed(
    status: &str,
    last_error: Option<&str>,
    rollups_exist: bool,
) -> bool {
    status == LONG_TERM_STATUS_ERROR
        || last_error == Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        || (!matches!(status, LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY) && !rollups_exist)
}

async fn complete_long_term_projection_daily_verify_with_control(
    pool: &Pool<Sqlite>,
    daily_verify_date: &str,
    maintenance_pending: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if maintenance_pending {
        return Ok(());
    }
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_projection_state SET daily_verify_pending = 0, daily_verify_bucket_date = NULL, last_daily_verify_at = CASE WHEN ?2 = ?3 THEN datetime('now') ELSE last_daily_verify_at END, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 1 AND daily_verify_bucket_date = ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?2)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(daily_verify_date)
    .bind(&today)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

fn long_term_projection_hourly_retention_start_date(retention_days: u64) -> NaiveDate {
    Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(retention_days.max(366) as i64 - 1)
}

async fn prune_long_term_projection_hourly_retention(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<(u64, u64)> {
    let control = LongTermProjectionWriteControl::unrestricted();
    prune_long_term_projection_hourly_retention_with_control(pool, retention_days, &control).await
}

#[derive(Debug, Clone, Copy)]
enum LongTermProjectionRetentionTarget {
    HourlyRollup,
    LegacyInterval,
    CanonicalInterval,
    Suppression,
    RebuildMember,
}

async fn long_term_projection_hourly_retention_target(
    pool: &Pool<Sqlite>,
    retention_start_date: NaiveDate,
    retention_start_epoch: i64,
) -> Result<Option<LongTermProjectionRetentionTarget>> {
    let retention_start_ms = retention_start_epoch * 1_000;
    let candidates = [
        (
            LongTermProjectionRetentionTarget::HourlyRollup,
            "SELECT EXISTS(SELECT 1 FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT 1)",
            retention_start_epoch,
        ),
        (
            LongTermProjectionRetentionTarget::LegacyInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT 1)",
            0,
        ),
        (
            LongTermProjectionRetentionTarget::CanonicalInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT 1)",
            retention_start_ms,
        ),
    ];
    for (target, statement, value) in candidates {
        let mut query = sqlx::query_scalar::<_, i64>(statement);
        if matches!(target, LongTermProjectionRetentionTarget::LegacyInterval) {
            query = query.bind(retention_start_date.to_string());
        } else {
            query = query.bind(value);
        }
        if query.fetch_one(pool).await? != 0 {
            return Ok(Some(target));
        }
    }
    for (target, table) in [
        (
            LongTermProjectionRetentionTarget::Suppression,
            "long_term_projection_interval_suppressions",
        ),
        (
            LongTermProjectionRetentionTarget::RebuildMember,
            "long_term_projection_rebuild_members",
        ),
    ] {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT 1)"
        );
        if sqlx::query_scalar::<_, i64>(&statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

async fn prune_long_term_projection_hourly_retention_with_control(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<(u64, u64)> {
    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let Some(target) = long_term_projection_hourly_retention_target(
        pool,
        retention_start_date,
        retention_start_epoch,
    )
    .await?
    else {
        return Ok((0, 0));
    };

    let (mut tx, permit) = control.begin(pool).await?;
    let deleted = match target {
        LongTermProjectionRetentionTarget::HourlyRollup => {
            sqlx::query(
                "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::LegacyInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_intervals WHERE rowid IN (SELECT rowid FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT ?2)",
            )
            .bind(retention_start_date.to_string())
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::CanonicalInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_state WHERE rowid IN (SELECT rowid FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch * 1_000)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::Suppression => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_suppressions WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_interval_suppressions metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::RebuildMember => {
            sqlx::query(
                "DELETE FROM long_term_projection_rebuild_members WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_rebuild_members metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
    };
    control.commit(tx, permit).await?;
    Ok(match target {
        LongTermProjectionRetentionTarget::HourlyRollup => (deleted, 0),
        LongTermProjectionRetentionTarget::LegacyInterval
        | LongTermProjectionRetentionTarget::CanonicalInterval
        | LongTermProjectionRetentionTarget::Suppression
        | LongTermProjectionRetentionTarget::RebuildMember => (0, deleted),
    })
}

async fn load_long_term_terminal_watermark(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM codex_invocations WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(pool)
    .await?)
}

async fn load_long_term_projection_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    let control = LongTermProjectionWriteControl::unrestricted();
    load_long_term_projection_cursor_with_control(pool, &control).await
}

async fn load_long_term_projection_cursor_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<i64> {
    if let Some(cursor) = control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_optional(pool),
            None,
        )
        .await?
    {
        return Ok(cursor.cursor_row_id);
    }

    let pressure_permit = control.try_begin_background()?;
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let write_permit = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .ok_or_else(|| {
            anyhow!(
                "long-term projection write deferred by database pressure: P2 cursor initialization is not admitted"
            )
        })?;

    // Another process can initialize the durable cursor between the initial read and this
    // short write. Keep that successful race on the read-only path.
    if let Some(cursor) = control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_optional(pool),
            Some(&coordinator),
        )
        .await?
    {
        return Ok(cursor.cursor_row_id);
    }

    let mut tx = control
        .begin_cursor_initialization(pool, &coordinator)
        .await?;
    if coordinator.p2_should_yield() {
        drop(tx);
        bail!(
            "long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"
        );
    }
    sqlx::query(
        "INSERT OR IGNORE INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .execute(&mut *tx)
    .await?;
    control
        .commit(
            tx,
            Some(LongTermProjectionWritePermit {
                _pressure: pressure_permit,
                _write: Some(write_permit),
            }),
        )
        .await?;
    Ok(control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(pool),
            None,
        )
        .await?
        .cursor_row_id)
}

#[derive(Debug)]
struct LongTermProjectionDateRebuild {
    bucket_date: String,
    start_epoch: i64,
    end_epoch: i64,
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    interval_segments: Vec<LongTermProjectionIntervalSegment>,
    source_row_count: u64,
}

#[derive(Debug)]
struct LongTermProjectionRebuildPublication<'a> {
    next_cursor: Option<i64>,
    clear_dirty_buckets: &'a [LongTermProjectionDirtyBucket],
    mark_ready: bool,
    publish_state: bool,
    publication_token: Option<&'a str>,
    repaired_start_date: Option<&'a str>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionDirtyBucket {
    bucket_date: String,
    generation: i64,
}

#[derive(Debug, FromRow)]
struct LongTermProjectionPublicationMember {
    bucket_date: String,
    rebuild_token: String,
    publication_token: String,
    publication_generation: Option<i64>,
}

fn next_long_term_projection_publication_token() -> String {
    let sequence =
        LONG_TERM_PROJECTION_PUBLICATION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    format!(
        "long-term-publication:{}:{sequence}",
        Utc::now().timestamp_micros()
    )
}

fn long_term_projection_row_affects_date(row: &LongTermInvocationRow, bucket_date: &str) -> bool {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    accumulate_long_term_invocation(row, &mut hourly, &mut daily, &mut statistics_start);
    daily.keys().any(|(date, _, _)| date == bucket_date)
}
