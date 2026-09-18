struct LongTermRefreshCandidateState {
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    recomputed_dates: HashSet<NaiveDate>,
    integrity_repair_failures: Vec<LongTermIntegrityMismatch>,
    completed_integrity_repairs: HashSet<NaiveDate>,
}

struct LongTermRefreshValidationContext<'a> {
    pool: &'a Pool<Sqlite>,
    initial_materialization: bool,
    ready_state: bool,
    today: NaiveDate,
    archive_read_failed: bool,
    terminal_proof_reconciliation_incomplete: bool,
    reconstructable_start: NaiveDate,
    scheduled_repair_date: Option<NaiveDate>,
}

struct LongTermRefreshSourceState {
    rows: Vec<LongTermInvocationRow>,
    processed_rows_count: i64,
    archive_markers: Vec<(String, String)>,
    failed_archive_paths: HashSet<String>,
    failed_archive_ranges: Vec<(String, String)>,
    affected_archive_dates: HashSet<NaiveDate>,
    unreadable_source_start_date: Option<NaiveDate>,
    archive_read_failed: bool,
    clear_all_attempt_markers: bool,
    all_archive_paths: Vec<ArchiveBatchPathRow>,
    archive_attempt_accounts: HashMap<(String, String), i64>,
    attempt_archive_markers: HashSet<(String, String)>,
}

struct LongTermRefreshApplyContext<'a> {
    pool: &'a Pool<Sqlite>,
    candidate: &'a LongTermRefreshCandidateState,
    retention_start: NaiveDate,
    reconstructable_start: NaiveDate,
    statistics_start_date: Option<&'a str>,
    initial_materialization: bool,
    processed_rows_count: i64,
    source_rows_empty: bool,
    archive_read_failed: bool,
    terminal_proof_reconciliation_incomplete: bool,
    archive_markers: &'a [(String, String)],
    failed_archive_paths: &'a HashSet<String>,
    clear_all_attempt_markers: bool,
    failed_archive_ranges: &'a [(String, String)],
    attempt_archive_markers: &'a HashSet<(String, String)>,
    control: &'a LongTermProjectionWriteControl<'a>,
}

struct LongTermRefreshPostSourceInput<'a> {
    pool: &'a Pool<Sqlite>,
    ready_state: bool,
    initial_materialization: bool,
    retention_start: NaiveDate,
    today: NaiveDate,
    reconstructable_start: NaiveDate,
    scheduled_repair_date: Option<NaiveDate>,
    terminal_proof_reconciliation_incomplete: bool,
    archive_read_failed: bool,
    unreadable_source_start_date: Option<NaiveDate>,
    rows: Vec<LongTermInvocationRow>,
    processed_rows_count: i64,
    archive_markers: Vec<(String, String)>,
    failed_archive_paths: HashSet<String>,
    failed_archive_ranges: Vec<(String, String)>,
    clear_all_attempt_markers: bool,
    all_archive_paths: Vec<ArchiveBatchPathRow>,
    archive_attempt_accounts: HashMap<(String, String), i64>,
    attempt_archive_markers: HashSet<(String, String)>,
    account_identities: HashMap<i64, LongTermAccountIdentity>,
    live_upstream_account_id_sql: &'a str,
    statistics_start_date: Option<String>,
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    affected_archive_dates: HashSet<NaiveDate>,
    control: &'a LongTermProjectionWriteControl<'a>,
}

async fn continue_long_term_refresh_after_sources(
    input: LongTermRefreshPostSourceInput<'_>,
) -> Result<()> {
    let LongTermRefreshPostSourceInput {
        pool,
        ready_state,
        initial_materialization,
        retention_start,
        today,
        reconstructable_start,
        scheduled_repair_date,
        terminal_proof_reconciliation_incomplete,
        archive_read_failed,
        unreadable_source_start_date,
        rows,
        processed_rows_count,
        archive_markers,
        failed_archive_paths,
        failed_archive_ranges,
        clear_all_attempt_markers,
        all_archive_paths,
        mut archive_attempt_accounts,
        mut attempt_archive_markers,
        account_identities,
        live_upstream_account_id_sql,
        mut statistics_start_date,
        hourly,
        daily,
        affected_archive_dates,
        control,
    } = input;
    let mut candidate = LongTermRefreshCandidateState {
        hourly,
        daily,
        recomputed_dates: affected_archive_dates,
        integrity_repair_failures: Vec::new(),
        completed_integrity_repairs: HashSet::new(),
    };
    let validation_context = LongTermRefreshValidationContext {
        pool,
        initial_materialization,
        ready_state,
        today,
        archive_read_failed,
        terminal_proof_reconciliation_incomplete,
        reconstructable_start,
        scheduled_repair_date,
    };
    prepare_long_term_refresh_candidates(
        &validation_context,
        &mut candidate,
        unreadable_source_start_date,
    )
    .await?;
    let mut rebuild_context = LongTermRefreshRebuildContext {
        pool,
        ready_state,
        all_archive_paths,
        rows: &rows,
        live_upstream_account_id_sql,
        account_identities: &account_identities,
        today,
        scheduled_repair_date,
        archive_attempt_accounts: &mut archive_attempt_accounts,
        attempt_archive_markers: &mut attempt_archive_markers,
    };
    rebuild_long_term_refresh_candidates(
        &mut rebuild_context,
        &mut candidate,
        &mut statistics_start_date,
    )
    .await?;
    drop(rebuild_context);
    apply_long_term_refresh_candidates(LongTermRefreshApplyContext {
        pool,
        candidate: &candidate,
        retention_start,
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
        control,
    })
    .await
}

async fn apply_long_term_refresh_candidates(
    context: LongTermRefreshApplyContext<'_>,
) -> Result<()> {
    apply_long_term_refresh_rollups_with_control(
        context.pool,
        LongTermRefreshRollupInput {
            hourly: &context.candidate.hourly,
            daily: &context.candidate.daily,
            recomputed_dates: &context.candidate.recomputed_dates,
            retention_start: context.retention_start,
            integrity_repair_failures: &context.candidate.integrity_repair_failures,
            completed_integrity_repairs: &context.candidate.completed_integrity_repairs,
            reconstructable_start: context.reconstructable_start,
            statistics_start_date: context.statistics_start_date,
            initial_materialization: context.initial_materialization,
            processed_rows_count: context.processed_rows_count,
            source_rows_empty: context.source_rows_empty,
            archive_read_failed: context.archive_read_failed,
            terminal_proof_reconciliation_incomplete: context
                .terminal_proof_reconciliation_incomplete,
            archive_markers: context.archive_markers,
            failed_archive_paths: context.failed_archive_paths,
            clear_all_attempt_markers: context.clear_all_attempt_markers,
            failed_archive_ranges: context.failed_archive_ranges,
            attempt_archive_markers: context.attempt_archive_markers,
        },
        context.control,
    )
    .await
}

async fn load_long_term_refresh_sources(
    pool: &Pool<Sqlite>,
    ready_state: bool,
    retention_start: NaiveDate,
    live_tail_start: Option<&str>,
    live_upstream_account_id_sql: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermRefreshSourceState> {
    let LongTermRefreshLiveRows {
        rows,
        positions: row_positions,
        processed_rows_count,
    } = load_long_term_refresh_live_rows(
        pool,
        ready_state,
        live_tail_start,
        live_upstream_account_id_sql,
        control,
    )
    .await?;
    let archive_paths = load_long_term_refresh_archive_paths(pool).await?;
    clear_stale_long_term_refresh_replay_markers(pool, control).await?;
    let replayed_archive_files =
        load_long_term_refresh_replayed_archives(pool, ready_state).await?;
    let attempt_date_range = long_term_refresh_attempt_date_range(
        ready_state,
        &rows,
        &archive_paths,
        &replayed_archive_files,
    );
    let (archive_attempt_accounts, attempt_archive_markers) =
        if !ready_state || attempt_date_range.is_some() {
            load_long_term_archive_attempt_accounts(pool, attempt_date_range).await?
        } else {
            (HashMap::new(), HashSet::new())
        };
    let all_archive_paths = archive_paths.clone();
    let LongTermRefreshArchiveScanState {
        rows,
        row_positions: _,
        processed_rows_count,
        archive_markers,
        failed_archive_paths,
        failed_archive_ranges,
        affected_archive_dates,
        unreadable_source_start_date,
        archive_read_failed,
        clear_all_attempt_markers,
    } = scan_long_term_refresh_archives(
        pool,
        LongTermRefreshArchiveScanInput {
            ready_state,
            retention_start,
            rows,
            row_positions,
            processed_rows_count,
            archive_paths,
            replayed_archive_files: &replayed_archive_files,
            archive_attempt_accounts: &archive_attempt_accounts,
        },
    )
    .await?;
    Ok(LongTermRefreshSourceState {
        rows,
        processed_rows_count,
        archive_markers,
        failed_archive_paths,
        failed_archive_ranges,
        affected_archive_dates,
        unreadable_source_start_date,
        archive_read_failed,
        clear_all_attempt_markers,
        all_archive_paths,
        archive_attempt_accounts,
        attempt_archive_markers,
    })
}

struct LongTermRefreshAggregationContext<'a> {
    pool: &'a Pool<Sqlite>,
    rows: &'a [LongTermInvocationRow],
    account_identities: &'a HashMap<i64, LongTermAccountIdentity>,
    ready_state: bool,
    hourly: &'a mut HashMap<(i64, String, String), LongTermBucket>,
    daily: &'a mut HashMap<(String, String, String), LongTermBucket>,
    statistics_start_date: &'a mut Option<String>,
    control: &'a LongTermProjectionWriteControl<'a>,
}

async fn aggregate_long_term_refresh_rows(
    context: LongTermRefreshAggregationContext<'_>,
) -> Result<()> {
    let total_rows = context.rows.len() as i64;
    if context.ready_state {
        persist_long_term_refresh_progress(context.pool, 0, total_rows, context.control).await?;
    }
    for (index, source_row) in context.rows.iter().enumerate() {
        let mut row = source_row.clone();
        hydrate_long_term_account_identity(&mut row, context.account_identities);
        accumulate_long_term_invocation(
            &row,
            context.hourly,
            context.daily,
            context.statistics_start_date,
        );
        if context.ready_state && ((index + 1) % 256 == 0 || index + 1 == context.rows.len()) {
            persist_long_term_refresh_progress(
                context.pool,
                (index + 1) as i64,
                total_rows,
                context.control,
            )
            .await?;
        }
    }
    Ok(())
}

async fn reconcile_long_term_refresh_sources(
    pool: &Pool<Sqlite>,
    ready_state: bool,
    integrity_audit_due: bool,
    today: NaiveDate,
    reconstructable_start: NaiveDate,
    invalidated_terminal_proof_buckets: &[i64],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<Option<NaiveDate>> {
    for mismatch in load_long_term_reconciliation_mismatches(
        pool,
        invalidated_terminal_proof_buckets,
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
    next_due_long_term_repair_date(pool, reconstructable_start).await
}

async fn queue_unreadable_long_term_refresh_repairs(
    context: &LongTermRefreshValidationContext<'_>,
    state: &mut LongTermRefreshCandidateState,
    unreadable_source_start: NaiveDate,
) -> Result<()> {
    if let Some(date) = context
        .scheduled_repair_date
        .filter(|date| *date >= unreadable_source_start)
    {
        let (candidate_daily, _) = long_term_candidate_integrity(date, &state.hourly, &state.daily);
        if let Some(mismatch) = queued_long_term_repair_mismatch(
            context.pool,
            date,
            candidate_daily,
            "one or more invocation archives are unreadable, so this repair cannot prove complete source coverage",
        )
        .await?
        {
            state.integrity_repair_failures.push(mismatch);
        }
    }
    let mut blocked_candidate_dates = state
        .recomputed_dates
        .iter()
        .copied()
        .filter(|date| *date >= unreadable_source_start)
        .collect::<HashSet<_>>();
    for (date, _, _) in state.daily.keys() {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            && date >= unreadable_source_start
        {
            blocked_candidate_dates.insert(date);
        }
    }
    for (bucket_start_epoch, _, _) in state.hourly.keys() {
        if let Some(date) = long_term_bucket_date(*bucket_start_epoch)
            && date >= unreadable_source_start
        {
            blocked_candidate_dates.insert(date);
        }
    }
    remove_long_term_candidate_dates(
        &mut state.hourly,
        &mut state.daily,
        &blocked_candidate_dates,
    );
    state
        .recomputed_dates
        .retain(|date| !blocked_candidate_dates.contains(date));
    Ok(())
}

async fn validate_initial_long_term_refresh_date(
    context: &LongTermRefreshValidationContext<'_>,
    state: &mut LongTermRefreshCandidateState,
    date: NaiveDate,
) -> Result<bool> {
    let (candidate_daily, candidate_hourly) =
        long_term_candidate_integrity(date, &state.hourly, &state.daily);
    let date_string = date.to_string();
    match load_long_term_integrity_oracle(context.pool, date).await? {
        Some(oracle) => {
            let candidate_is_empty = !state
                .daily
                .keys()
                .any(|(bucket_date, _, _)| bucket_date == &date_string)
                && !state.hourly.keys().any(|(bucket_start_epoch, _, _)| {
                    long_term_bucket_date(*bucket_start_epoch) == Some(date)
                });
            let initial_complete_snapshot_without_hourly_proof = context.initial_materialization
                && oracle.hourly.is_empty()
                && !context.archive_read_failed
                && !context.terminal_proof_reconciliation_incomplete
                && date >= context.reconstructable_start;
            if initial_complete_snapshot_without_hourly_proof {
                if context.scheduled_repair_date == Some(date) {
                    state.completed_integrity_repairs.insert(date);
                }
                return Ok(false);
            }
            if context.scheduled_repair_date == Some(date) && candidate_is_empty {
                state.completed_integrity_repairs.insert(date);
                return Ok(false);
            }
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
                    "long-term stats full rebuild cannot prove complete"
                );
                state.integrity_repair_failures.push(mismatch);
                return Ok(true);
            }
            if context.scheduled_repair_date == Some(date) {
                state.completed_integrity_repairs.insert(date);
            }
            Ok(false)
        }
        None if context.initial_materialization
            && !context.archive_read_failed
            && !context.terminal_proof_reconciliation_incomplete
            && date >= context.reconstructable_start =>
        {
            if context.scheduled_repair_date == Some(date) {
                state.completed_integrity_repairs.insert(date);
            }
            Ok(false)
        }
        None if context.scheduled_repair_date == Some(date) => {
            if let Some(mismatch) = queued_long_term_repair_mismatch(
                context.pool,
                date,
                candidate_daily,
                "canonical hourly integrity evidence is unavailable for the queued repair",
            )
            .await?
            {
                state.integrity_repair_failures.push(mismatch);
            }
            Ok(true)
        }
        None => {
            state
                .integrity_repair_failures
                .push(LongTermIntegrityMismatch {
                    date,
                    expected: candidate_daily,
                    observed: candidate_daily,
                    reason:
                        "canonical hourly integrity evidence is unavailable for the full rebuild"
                            .to_string(),
                });
            Ok(true)
        }
    }
}

async fn validate_initial_long_term_refresh_candidates(
    context: &LongTermRefreshValidationContext<'_>,
    state: &mut LongTermRefreshCandidateState,
) -> Result<()> {
    if !context.ready_state && !state.recomputed_dates.is_empty() {
        let mut blocked_recomputed_dates = HashSet::new();
        for date in state.recomputed_dates.clone() {
            if date >= context.today {
                continue;
            }
            if validate_initial_long_term_refresh_date(context, state, date).await? {
                blocked_recomputed_dates.insert(date);
            }
        }
        remove_long_term_candidate_dates(
            &mut state.hourly,
            &mut state.daily,
            &blocked_recomputed_dates,
        );
        state
            .recomputed_dates
            .retain(|date| !blocked_recomputed_dates.contains(date));
    }
    Ok(())
}

async fn prepare_long_term_refresh_candidates(
    context: &LongTermRefreshValidationContext<'_>,
    state: &mut LongTermRefreshCandidateState,
    unreadable_source_start_date: Option<NaiveDate>,
) -> Result<()> {
    for (date, _, _) in state.daily.keys() {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            state.recomputed_dates.insert(date);
        }
    }
    if let Some(date) = context.scheduled_repair_date {
        state.recomputed_dates.insert(date);
    }
    if let Some(unreadable_source_start) = unreadable_source_start_date {
        queue_unreadable_long_term_refresh_repairs(context, state, unreadable_source_start).await?;
    }
    validate_initial_long_term_refresh_candidates(context, state).await
}

struct LongTermRefreshRebuildContext<'a> {
    pool: &'a Pool<Sqlite>,
    ready_state: bool,
    all_archive_paths: Vec<ArchiveBatchPathRow>,
    rows: &'a [LongTermInvocationRow],
    live_upstream_account_id_sql: &'a str,
    account_identities: &'a HashMap<i64, LongTermAccountIdentity>,
    today: NaiveDate,
    scheduled_repair_date: Option<NaiveDate>,
    archive_attempt_accounts: &'a mut HashMap<(String, String), i64>,
    attempt_archive_markers: &'a mut HashSet<(String, String)>,
}

async fn load_long_term_refresh_rebuild_live_rows(
    context: &mut LongTermRefreshRebuildContext<'_>,
    recomputed_dates: &mut HashSet<NaiveDate>,
) -> Result<(Vec<LongTermInvocationRow>, HashSet<i64>)> {
    let previous_date = recomputed_dates
        .iter()
        .min()
        .copied()
        .and_then(|date| date.pred_opt());
    if let Some(previous_date) = previous_date {
        recomputed_dates.insert(previous_date);
    }
    if let Some((start, end)) = recomputed_dates
        .iter()
        .min()
        .copied()
        .zip(recomputed_dates.iter().max().copied())
    {
        let (accounts, markers) =
            load_long_term_archive_attempt_accounts(context.pool, Some((start, end))).await?;
        context.archive_attempt_accounts.extend(accounts);
        context.attempt_archive_markers.extend(markers);
    }
    let mut rebuild_rows = context
        .rows
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
            "SELECT inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort, {live_upstream_account_id_sql} AS upstream_account_id, NULL AS upstream_account_kind, NULL AS upstream_account_name, inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms, inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms, inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message FROM codex_invocations inv WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND inv.occurred_at < ?2 AND (inv.occurred_at >= ?1 OR (inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1))) ORDER BY inv.occurred_at ASC, inv.id ASC",
            live_upstream_account_id_sql = context.live_upstream_account_id_sql,
        );
        if let (Some(live_start), Some(live_end)) = (live_start, live_end) {
            for row in sqlx::query_as::<_, LongTermInvocationRow>(&live_rebuild_sql)
                .bind(live_start)
                .bind(live_end)
                .fetch_all(context.pool)
                .await?
            {
                if rebuild_seen_ids.insert(row.id) {
                    rebuild_rows.push(row);
                }
            }
        }
    }
    Ok((rebuild_rows, rebuild_seen_ids))
}

async fn load_long_term_refresh_rebuild_archive_rows(
    context: &mut LongTermRefreshRebuildContext<'_>,
    recomputed_dates: &HashSet<NaiveDate>,
    rebuild_rows: &mut Vec<LongTermInvocationRow>,
    rebuild_seen_ids: &mut HashSet<i64>,
) -> Result<()> {
    for archive_path in context.all_archive_paths.iter() {
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
        let archive_sha256 = load_long_term_archive_sha256(context.pool, archive_path.file_path())
            .await?
            .context("completed invocation archive has no manifest sha256")?;
        ensure_long_term_archive_source_identity(
            context.pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        let Some((archive_pool, cleanup)) =
            open_invocation_archive_batch_pool(archive_path, "long-term-stats-rebuild").await?
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
            context.pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        for mut row in archive_rows {
            hydrate_long_term_archive_attempt_account(&mut row, context.archive_attempt_accounts);
            if rebuild_seen_ids.insert(row.id) {
                rebuild_rows.push(row);
            }
        }
    }
    Ok(())
}

async fn load_long_term_refresh_rebuild_rows(
    context: &mut LongTermRefreshRebuildContext<'_>,
    recomputed_dates: &mut HashSet<NaiveDate>,
) -> Result<Vec<LongTermInvocationRow>> {
    let (mut rebuild_rows, mut rebuild_seen_ids) =
        load_long_term_refresh_rebuild_live_rows(context, recomputed_dates).await?;
    load_long_term_refresh_rebuild_archive_rows(
        context,
        recomputed_dates,
        &mut rebuild_rows,
        &mut rebuild_seen_ids,
    )
    .await?;
    Ok(rebuild_rows)
}

async fn validate_long_term_refresh_rebuild_date(
    context: &LongTermRefreshRebuildContext<'_>,
    candidate: &mut LongTermRefreshCandidateState,
    rebuilt_hourly: &HashMap<(i64, String, String), LongTermBucket>,
    rebuilt_daily: &HashMap<(String, String, String), LongTermBucket>,
    date: NaiveDate,
) -> Result<bool> {
    let date_string = date.to_string();
    let (candidate_daily, candidate_hourly) =
        long_term_candidate_integrity(date, rebuilt_hourly, rebuilt_daily);
    let persisted_calls = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(&date_string)
    .fetch_one(context.pool)
    .await?;
    if date < context.today {
        match load_long_term_integrity_oracle(context.pool, date).await? {
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
                    candidate.integrity_repair_failures.push(mismatch);
                    return Ok(true);
                }
                if context.scheduled_repair_date == Some(date) {
                    candidate.completed_integrity_repairs.insert(date);
                }
            }
            None => {
                if context.scheduled_repair_date == Some(date)
                    && let Some(mismatch) = queued_long_term_repair_mismatch(
                        context.pool,
                        date,
                        candidate_daily,
                        "canonical hourly integrity evidence is unavailable for the queued repair",
                    )
                    .await?
                {
                    candidate.integrity_repair_failures.push(mismatch);
                }
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if persisted_calls > candidate_daily.calls {
        if context.scheduled_repair_date == Some(date)
            && let Some(mismatch) = queued_long_term_repair_mismatch(
                context.pool,
                date,
                candidate_daily,
                format!(
                    "candidate calls={} is below retained calls={persisted_calls}; source reconstruction is incomplete",
                    candidate_daily.calls
                ),
            )
            .await?
        {
            candidate.integrity_repair_failures.push(mismatch);
        }
        return Ok(true);
    }
    if context.scheduled_repair_date == Some(date)
        && let Some(mismatch) = queued_long_term_repair_mismatch(
            context.pool,
            date,
            candidate_daily,
            "canonical hourly integrity evidence is unavailable for the queued repair",
        )
        .await?
    {
        candidate.integrity_repair_failures.push(mismatch);
        return Ok(true);
    }
    Ok(false)
}

async fn rebuild_long_term_refresh_candidates(
    context: &mut LongTermRefreshRebuildContext<'_>,
    candidate: &mut LongTermRefreshCandidateState,
    statistics_start_date: &mut Option<String>,
) -> Result<()> {
    if !context.ready_state || candidate.recomputed_dates.is_empty() {
        return Ok(());
    }
    let rebuild_rows =
        load_long_term_refresh_rebuild_rows(context, &mut candidate.recomputed_dates).await?;
    let mut rebuilt_hourly = HashMap::new();
    let mut rebuilt_daily = HashMap::new();
    let mut rebuilt_start = None;
    for mut row in rebuild_rows {
        hydrate_long_term_account_identity(&mut row, context.account_identities);
        accumulate_long_term_invocation(
            &row,
            &mut rebuilt_hourly,
            &mut rebuilt_daily,
            &mut rebuilt_start,
        );
    }
    let mut blocked_recomputed_dates = HashSet::new();
    for date in candidate.recomputed_dates.clone() {
        if validate_long_term_refresh_rebuild_date(
            context,
            candidate,
            &rebuilt_hourly,
            &rebuilt_daily,
            date,
        )
        .await?
        {
            blocked_recomputed_dates.insert(date);
        }
    }
    if !blocked_recomputed_dates.is_empty() {
        remove_long_term_candidate_dates(
            &mut rebuilt_hourly,
            &mut rebuilt_daily,
            &blocked_recomputed_dates,
        );
        remove_long_term_candidate_dates(
            &mut candidate.hourly,
            &mut candidate.daily,
            &blocked_recomputed_dates,
        );
        candidate
            .recomputed_dates
            .retain(|date| !blocked_recomputed_dates.contains(date));
    }
    candidate.hourly.retain(|(bucket_start, _, _), _| {
        Shanghai
            .timestamp_opt(*bucket_start, 0)
            .single()
            .map(|value| !candidate.recomputed_dates.contains(&value.date_naive()))
            .unwrap_or(true)
    });
    candidate.daily.retain(|(date, _, _), _| {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map(|value| !candidate.recomputed_dates.contains(&value))
            .unwrap_or(true)
    });
    candidate.hourly.extend(rebuilt_hourly);
    candidate.daily.extend(rebuilt_daily);
    if let Some(rebuilt_start) = rebuilt_start
        && statistics_start_date
            .as_deref()
            .is_none_or(|current| rebuilt_start.as_str() < current)
    {
        *statistics_start_date = Some(rebuilt_start);
    }
    Ok(())
}
