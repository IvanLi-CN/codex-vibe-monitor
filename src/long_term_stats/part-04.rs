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
    let daily_verify_date = queue_long_term_projection_daily_verify_if_due(state, trigger).await?;
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
        let baseline = rebuild_long_term_projection_baseline(
            state,
            baseline_cursor,
            state_row.status != LONG_TERM_STATUS_ERROR,
            &control,
        )
        .await?;
        loaded_row_count = loaded_row_count.saturating_add(baseline.loaded_row_count);
        repaired.extend(baseline.repaired);
        cursor = baseline_cursor;
    } else {
        let (events, ready_dates) = load_long_term_projection_events(state, cursor).await?;
        let incremental = apply_long_term_projection_terminal_events(
            state,
            cursor,
            events,
            &ready_dates,
            &control,
        )
        .await?;
        let Some(incremental) = incremental else {
            return Ok(0);
        };
        cursor = incremental.cursor;
        event_count = event_count.saturating_add(incremental.event_count);
        let repair = repair_long_term_projection_terminal_event(
            state,
            trigger,
            incremental.repair_event,
            &control,
        )
        .await?;
        cursor = repair.cursor.unwrap_or(cursor);
        repaired.extend(repair.repaired);
        loaded_row_count = loaded_row_count.saturating_add(repair.loaded_row_count);
        deferred_repair_count = deferred_repair_count.saturating_add(repair.deferred_repair_count);
        deferred_repair_backoff_count =
            deferred_repair_backoff_count.saturating_add(repair.deferred_repair_backoff_count);
    }

    if !repaired.is_empty() {
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let dirty_outcome =
        repair_long_term_projection_dirty_dates(state, trigger, &repaired, &control).await?;
    repaired.extend(dirty_outcome.repaired);
    loaded_row_count = loaded_row_count.saturating_add(dirty_outcome.loaded_row_count);
    deferred_repair_count =
        deferred_repair_count.saturating_add(dirty_outcome.deferred_repair_count);

    finish_long_term_projection_flush(
        state,
        LongTermProjectionFlushCompletion {
            trigger,
            daily_verify_date: daily_verify_date.as_deref(),
            cursor,
            repaired: &repaired,
            event_count,
            deferred_repair_count,
            deferred_repair_backoff_count,
            started,
        },
        &control,
    )
    .await?;
    Ok(loaded_row_count.saturating_add(event_count as u64))
}

async fn queue_long_term_projection_daily_verify_if_due(
    state: &AppState,
    trigger: &'static str,
) -> Result<Option<String>> {
    if trigger == "daily_verify" || long_term_projection_daily_verify_due(&state.pool).await? {
        return queue_long_term_projection_daily_verify(state)
            .await
            .map(Some);
    }
    Ok(None)
}

async fn load_long_term_projection_events(
    state: &AppState,
    cursor: i64,
) -> Result<(Vec<LongTermProjectionEvent>, HashSet<String>)> {
    let identities = load_long_term_account_identities(&state.pool).await?;
    let mut rows = load_long_term_projection_terminal_rows(&state.pool, cursor).await?;
    for row in &mut rows {
        hydrate_long_term_account_identity(row, &identities);
    }
    let events = rows
        .iter()
        .map(build_long_term_projection_event)
        .collect::<Vec<_>>();
    let candidate_dates = events
        .iter()
        .flat_map(|event| event.bucket_dates.iter().cloned())
        .collect::<HashSet<_>>();
    let ready_dates = load_long_term_projection_ready_dates(&state.pool, &candidate_dates).await?;
    Ok((events, ready_dates))
}

struct LongTermProjectionIncrementalOutcomeState {
    cursor: i64,
    event_count: usize,
    repair_event: Option<LongTermProjectionEvent>,
}

struct LongTermProjectionTerminalRepairOutcome {
    cursor: Option<i64>,
    repaired: Vec<String>,
    loaded_row_count: u64,
    deferred_repair_count: usize,
    deferred_repair_backoff_count: usize,
}

async fn repair_long_term_projection_terminal_event(
    state: &AppState,
    trigger: &'static str,
    event: Option<LongTermProjectionEvent>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionTerminalRepairOutcome> {
    let Some(event) = event else {
        return Ok(LongTermProjectionTerminalRepairOutcome {
            cursor: None,
            repaired: Vec::new(),
            loaded_row_count: 0,
            deferred_repair_count: 0,
            deferred_repair_backoff_count: 0,
        });
    };
    let mut dates = event.bucket_dates.into_iter().collect::<Vec<_>>();
    dates.sort();
    if !long_term_projection_allows_expensive_repair(trigger) {
        debug!(projection = "long_term", repair_scope = ?dates, defer_reason = "terminal_hot_path",
            "long-term cursor repair deferred to the bounded repair window");
        defer_long_term_projection_terminal_repair(state, "terminal_hot_path").await;
        return Ok(LongTermProjectionTerminalRepairOutcome {
            cursor: None,
            repaired: Vec::new(),
            loaded_row_count: 0,
            deferred_repair_count: dates.len(),
            deferred_repair_backoff_count: dates.len(),
        });
    }
    ensure_long_term_projection_repairs_with_control(
        &state.pool,
        &dates,
        "interval_baseline",
        control,
    )
    .await?;
    let dirty = load_long_term_projection_dirty_buckets(&state.pool, &dates).await?;
    if long_term_projection_repairs_are_deferred(&state.pool, &dates).await? {
        debug!(projection = "long_term", repair_scope = ?dates, defer_reason = "repair_backoff",
            "long-term cursor repair retained until its retry deadline");
        return Ok(LongTermProjectionTerminalRepairOutcome {
            cursor: None,
            repaired: Vec::new(),
            loaded_row_count: 0,
            deferred_repair_count: dates.len(),
            deferred_repair_backoff_count: dates.len(),
        });
    }
    let Some((rebuilds, loaded_row_count)) =
        build_long_term_projection_repairs(state, &dates, control).await?
    else {
        defer_long_term_projection_repairs_with_control(&state.pool, &dates, control).await?;
        return Ok(LongTermProjectionTerminalRepairOutcome {
            cursor: None,
            repaired: Vec::new(),
            loaded_row_count: 0,
            deferred_repair_count: dates.len(),
            deferred_repair_backoff_count: 0,
        });
    };
    commit_long_term_projection_date_rebuilds_with_control(
        &state.pool,
        &rebuilds,
        Some(event.row_id),
        &dirty,
        false,
        control,
    )
    .await?;
    Ok(LongTermProjectionTerminalRepairOutcome {
        cursor: Some(event.row_id),
        repaired: dates,
        loaded_row_count,
        deferred_repair_count: 0,
        deferred_repair_backoff_count: 0,
    })
}

async fn build_long_term_projection_repairs(
    state: &AppState,
    dates: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<Option<(Vec<LongTermProjectionDateRebuild>, u64)>> {
    let mut rebuilds = Vec::with_capacity(dates.len());
    let mut loaded_row_count = 0_u64;
    for date in dates {
        match build_long_term_projection_date_rebuild(&state.pool, date, control).await {
            Ok(rebuild) => {
                loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
                rebuilds.push(rebuild);
            }
            Err(error) => {
                warn!(error = %error, projection = "long_term", repair_scope = ?dates, retry_after_ms = 300_000_u64,
                    "long-term cursor repair deferred after an unavailable source");
                return Ok(None);
            }
        }
    }
    Ok(Some((rebuilds, loaded_row_count)))
}

async fn apply_long_term_projection_terminal_events(
    state: &AppState,
    cursor: i64,
    events: Vec<LongTermProjectionEvent>,
    ready_dates: &HashSet<String>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<Option<LongTermProjectionIncrementalOutcomeState>> {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut segments = Vec::new();
    let mut state_cursor = cursor;
    let mut event_count = 0_usize;
    let mut batch_count = 0;
    let mut repair_event = None;
    for event in events {
        if long_term_projection_event_needs_rebuild(&event, ready_dates) {
            repair_event = Some(event);
            break;
        }
        let batch_is_full = long_term_projection_incremental_batch_is_full(
            &event,
            &hourly,
            &daily,
            &segments,
            batch_count,
        );
        if batch_is_full {
            if !flush_long_term_projection_incremental_batch(
                state,
                &hourly,
                &daily,
                &segments,
                state_cursor,
                batch_count,
                control,
            )
            .await?
            {
                return Ok(None);
            }
            hourly.clear();
            daily.clear();
            segments.clear();
            batch_count = 0;
        }
        state_cursor = event.row_id;
        event_count = event_count.saturating_add(1);
        batch_count = batch_count.saturating_add(1);
        merge_long_term_projection_buckets(&mut hourly, event.hourly);
        merge_long_term_projection_buckets(&mut daily, event.daily);
        segments.extend(event.segments);
    }
    if batch_count > 0
        && !flush_long_term_projection_incremental_batch(
            state,
            &hourly,
            &daily,
            &segments,
            state_cursor,
            batch_count,
            control,
        )
        .await?
    {
        return Ok(None);
    }
    Ok(Some(LongTermProjectionIncrementalOutcomeState {
        cursor: state_cursor,
        event_count,
        repair_event,
    }))
}

fn long_term_projection_event_needs_rebuild(
    event: &LongTermProjectionEvent,
    ready_dates: &HashSet<String>,
) -> bool {
    if !event.bucket_dates.is_subset(ready_dates)
        || event.hourly.len() + event.daily.len() + event.segments.len()
            > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS
    {
        return true;
    }
    false
}

fn long_term_projection_incremental_batch_is_full(
    event: &LongTermProjectionEvent,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    batch_count: usize,
) -> bool {
    let additional_rows = event
        .hourly
        .keys()
        .filter(|key| !hourly.contains_key(*key))
        .count()
        + event
            .daily
            .keys()
            .filter(|key| !daily.contains_key(*key))
            .count();
    batch_count > 0
        && hourly.len() + daily.len() + additional_rows + segments.len() + event.segments.len()
            > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS
}

async fn flush_long_term_projection_incremental_batch(
    state: &AppState,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    cursor: i64,
    event_count: usize,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &state.pool,
        &state.long_term_projection_runtime,
        LongTermProjectionIncrementalBatch {
            hourly,
            daily,
            segments,
        },
        cursor,
        event_count,
        control,
    )
    .await?;
    if outcome == LongTermProjectionIncrementalOutcome::RebuildRequired {
        defer_long_term_projection_terminal_repair(state, "dirty_publication").await;
        return Ok(false);
    }
    Ok(true)
}

struct LongTermProjectionBaselineOutcome {
    repaired: Vec<String>,
    loaded_row_count: u64,
}

async fn rebuild_long_term_projection_baseline(
    state: &AppState,
    baseline_cursor: i64,
    mark_ready: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionBaselineOutcome> {
    let dates = long_term_projection_open_baseline_dates();
    queue_long_term_projection_repairs_with_control(
        &state.pool,
        &dates,
        "interval_baseline",
        control,
    )
    .await?;
    let dirty = load_long_term_projection_dirty_buckets(&state.pool, &dates).await?;
    let mut rebuilds = Vec::with_capacity(dates.len());
    let mut loaded_row_count = 0_u64;
    for date in &dates {
        let rebuild = build_long_term_projection_date_rebuild(&state.pool, date, control).await?;
        loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
        rebuilds.push(rebuild);
    }
    commit_long_term_projection_date_rebuilds_with_control(
        &state.pool,
        &rebuilds,
        Some(baseline_cursor),
        &dirty,
        mark_ready,
        control,
    )
    .await?;
    Ok(LongTermProjectionBaselineOutcome {
        repaired: dates,
        loaded_row_count,
    })
}

struct LongTermProjectionDirtyRepairOutcome {
    repaired: Vec<String>,
    loaded_row_count: u64,
    deferred_repair_count: usize,
}

async fn repair_long_term_projection_dirty_dates(
    state: &AppState,
    trigger: &'static str,
    already_repaired: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionDirtyRepairOutcome> {
    let dirty_dates = load_long_term_projection_due_dirty_dates(&state.pool, trigger).await?;
    let mut outcome = LongTermProjectionDirtyRepairOutcome {
        repaired: Vec::new(),
        loaded_row_count: 0,
        deferred_repair_count: 0,
    };
    for dirty in dirty_dates {
        if already_repaired.contains(&dirty.bucket_date) {
            continue;
        }
        match rebuild_long_term_projection_dirty_date(state, dirty, control).await? {
            Some((date, row_count)) => {
                outcome.repaired.push(date);
                outcome.loaded_row_count = outcome.loaded_row_count.saturating_add(row_count);
            }
            None => outcome.deferred_repair_count = outcome.deferred_repair_count.saturating_add(1),
        }
    }
    Ok(outcome)
}

async fn load_long_term_projection_due_dirty_dates(
    pool: &Pool<Sqlite>,
    trigger: &'static str,
) -> Result<Vec<LongTermProjectionDirtyBucket>> {
    if !long_term_projection_allows_expensive_repair(trigger) {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, LongTermProjectionDirtyBucket>(
        "SELECT bucket_date, generation FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now') ORDER BY queued_at ASC, bucket_date ASC LIMIT ?1",
    ).bind(LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH).fetch_all(pool).await.map_err(Into::into)
}

async fn rebuild_long_term_projection_dirty_date(
    state: &AppState,
    dirty: LongTermProjectionDirtyBucket,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<Option<(String, u64)>> {
    let date = dirty.bucket_date.clone();
    let rebuild = match build_long_term_projection_date_rebuild(&state.pool, &date, control).await {
        Ok(rebuild) => rebuild,
        Err(error) => {
            defer_long_term_projection_repair_with_control(&state.pool, &date, control).await?;
            warn!(error = %error, projection = "long_term", repair_scope = %date, retry_after_ms = 300_000_u64,
                "long-term projection repair deferred after an unavailable source");
            return Ok(None);
        }
    };
    let row_count = rebuild.source_row_count;
    commit_long_term_projection_date_rebuilds_with_control(
        &state.pool,
        &[rebuild],
        None,
        std::slice::from_ref(&dirty),
        false,
        control,
    )
    .await?;
    invalidate_long_term_projection_interval_cache(state).await;
    Ok(Some((date, row_count)))
}

struct LongTermProjectionFlushCompletion<'a> {
    trigger: &'static str,
    daily_verify_date: Option<&'a str>,
    cursor: i64,
    repaired: &'a [String],
    event_count: usize,
    deferred_repair_count: usize,
    deferred_repair_backoff_count: usize,
    started: Instant,
}

async fn finish_long_term_projection_flush(
    state: &AppState,
    completion: LongTermProjectionFlushCompletion<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    complete_long_term_projection_daily_verify(state, completion.daily_verify_date, control)
        .await?;
    let dirty_bucket_count = long_term_projection_dirty_bucket_count(&state.pool).await?;
    let deferred_backoff = completion
        .deferred_repair_backoff_count
        .max(long_term_projection_deferred_repair_count(&state.pool).await?);
    update_long_term_projection_flush_runtime(
        state,
        completion,
        deferred_backoff,
        dirty_bucket_count,
    )
    .await
}

async fn complete_long_term_projection_daily_verify(
    state: &AppState,
    daily_verify_date: Option<&str>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let Some(daily_verify_date) = daily_verify_date else {
        return Ok(());
    };
    let maintenance_pending = long_term_projection_maintenance_needed(
        &state.pool,
        state.config.long_term_stats_hourly_retention_days,
    )
    .await?;
    complete_long_term_projection_daily_verify_with_control(
        &state.pool,
        daily_verify_date,
        maintenance_pending,
        control,
    )
    .await
}

async fn long_term_projection_dirty_bucket_count(pool: &Pool<Sqlite>) -> Result<usize> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(pool)
            .await?
            .max(0) as usize,
    )
}

async fn long_term_projection_deferred_repair_count(pool: &Pool<Sqlite>) -> Result<usize> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE datetime(next_attempt_at) > datetime('now')",
    ).fetch_one(pool).await?.max(0) as usize)
}

async fn update_long_term_projection_flush_runtime(
    state: &AppState,
    completion: LongTermProjectionFlushCompletion<'_>,
    deferred_repair_backoff_count: usize,
    dirty_bucket_count: usize,
) -> Result<()> {
    state
        .terminal_projection_hub
        .advance_long_term_cursor(completion.cursor);
    let projection_health = state.terminal_projection_hub.health();
    let elapsed_ms = completion.started.elapsed().as_millis() as u64;
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state =
        long_term_projection_runtime_state(completion.deferred_repair_count, dirty_bucket_count)
            .to_string();
    runtime.cursor_row_id = completion.cursor;
    runtime.dirty_bucket_count = dirty_bucket_count;
    runtime.pending_event_count = projection_health.pending_event_count;
    runtime.last_flush_elapsed_ms = Some(elapsed_ms);
    runtime.last_flush_at = Some(Instant::now());
    runtime.last_repair_scope =
        (!completion.repaired.is_empty()).then(|| completion.repaired.join(","));
    let terminal_hot_path = !long_term_projection_allows_expensive_repair(completion.trigger)
        && completion.deferred_repair_count > 0;
    runtime.last_defer_reason = long_term_projection_defer_reason(
        terminal_hot_path,
        completion.deferred_repair_count,
        deferred_repair_backoff_count,
    );
    runtime.last_error_kind = (!terminal_hot_path && completion.deferred_repair_count > 0)
        .then(|| "targeted_repair".to_string());
    runtime.next_repair_at = (completion.deferred_repair_count > 0)
        .then(|| long_term_projection_repair_deadline(runtime.next_repair_at, Instant::now()));
    log_long_term_projection_flush(
        &completion,
        dirty_bucket_count,
        deferred_repair_backoff_count,
        elapsed_ms,
        &projection_health,
        &runtime,
    );
    Ok(())
}

fn long_term_projection_runtime_state(
    deferred_repair_count: usize,
    dirty_bucket_count: usize,
) -> &'static str {
    if deferred_repair_count > 0 {
        "dirty_last_good"
    } else if dirty_bucket_count == 0 {
        "healthy"
    } else {
        "repairing"
    }
}

fn long_term_projection_defer_reason(
    terminal_hot_path: bool,
    deferred_repair_count: usize,
    deferred_repair_backoff_count: usize,
) -> Option<String> {
    if terminal_hot_path {
        Some("terminal_hot_path".to_string())
    } else if deferred_repair_backoff_count > 0 {
        Some("repair_backoff".to_string())
    } else if deferred_repair_count > 0 {
        Some("repair_source_unavailable".to_string())
    } else {
        None
    }
}

fn log_long_term_projection_flush(
    completion: &LongTermProjectionFlushCompletion<'_>,
    dirty_bucket_count: usize,
    deferred_repair_backoff_count: usize,
    elapsed_ms: u64,
    projection_health: &TerminalProjectionHealth,
    runtime: &LongTermProjectionRuntime,
) {
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
    debug!(projection = "long_term", trigger = completion.trigger, event_count = completion.event_count,
        cursor_lag = projection_health.last_persisted_row_id.saturating_sub(completion.cursor), dirty_bucket_count,
        repair_scope = ?runtime.last_repair_scope, interval_bytes, interval_key_count = runtime.interval_index.len(),
        deferred_repair_count = completion.deferred_repair_count, deferred_repair_backoff_count, retention_pruned_hourly_rows = 0,
        retention_pruned_interval_rows = 0, flush_outcome = "accepted", elapsed_ms,
        "long-term projection flush completed");
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
