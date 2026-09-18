struct LongTermRefreshSetup {
    previous_state: Option<LongTermStateRow>,
    today: NaiveDate,
    live_tail_start: Option<String>,
    live_upstream_account_id_sql: &'static str,
    ready_state: bool,
    retention_start: NaiveDate,
    integrity_audit_due: bool,
    terminal_proof_reconciliation_incomplete: bool,
    invalidated_terminal_proof_buckets: Vec<i64>,
    account_identities: HashMap<i64, LongTermAccountIdentity>,
}

struct LongTermRefreshLiveRows {
    rows: Vec<LongTermInvocationRow>,
    positions: HashMap<i64, usize>,
    processed_rows_count: i64,
}

struct LongTermRefreshArchiveScanState {
    rows: Vec<LongTermInvocationRow>,
    row_positions: HashMap<i64, usize>,
    processed_rows_count: i64,
    archive_markers: Vec<(String, String)>,
    failed_archive_paths: HashSet<String>,
    failed_archive_ranges: Vec<(String, String)>,
    affected_archive_dates: HashSet<NaiveDate>,
    unreadable_source_start_date: Option<NaiveDate>,
    archive_read_failed: bool,
    clear_all_attempt_markers: bool,
}

async fn refresh_long_term_stats_inner(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    initial_materialization: bool,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let setup = prepare_long_term_refresh(
        pool,
        retention_days,
        initial_materialization,
        refresh_started_at,
        control,
    )
    .await?;
    let LongTermRefreshSetup {
        previous_state,
        today,
        live_tail_start,
        live_upstream_account_id_sql,
        ready_state,
        retention_start,
        integrity_audit_due,
        terminal_proof_reconciliation_incomplete,
        invalidated_terminal_proof_buckets,
        account_identities,
    } = setup;
    let mut hourly: HashMap<(i64, String, String), LongTermBucket> = HashMap::new();
    let mut daily: HashMap<(String, String, String), LongTermBucket> = HashMap::new();
    let mut statistics_start_date = previous_state
        .as_ref()
        .and_then(|state| state.statistics_start_date.clone());
    let LongTermRefreshSourceState {
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
    } = load_long_term_refresh_sources(
        pool,
        ready_state,
        retention_start,
        live_tail_start.as_deref(),
        live_upstream_account_id_sql,
        control,
    )
    .await?;
    aggregate_long_term_refresh_rows(LongTermRefreshAggregationContext {
        pool,
        rows: &rows,
        account_identities: &account_identities,
        ready_state,
        hourly: &mut hourly,
        daily: &mut daily,
        statistics_start_date: &mut statistics_start_date,
        control,
    })
    .await?;

    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        statistics_start_date.as_deref(),
        previous_state
            .as_ref()
            .and_then(|state| state.integrity_source_start_date.as_deref()),
    );
    let scheduled_repair_date = reconcile_long_term_refresh_sources(
        pool,
        ready_state,
        integrity_audit_due,
        today,
        reconstructable_start,
        &invalidated_terminal_proof_buckets,
        control,
    )
    .await?;

    continue_long_term_refresh_after_sources(LongTermRefreshPostSourceInput {
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
        archive_attempt_accounts,
        attempt_archive_markers,
        account_identities,
        live_upstream_account_id_sql,
        statistics_start_date,
        hourly,
        daily,
        affected_archive_dates,
        control,
    })
    .await
}

fn long_term_refresh_attempt_date_range(
    ready_state: bool,
    rows: &[LongTermInvocationRow],
    archive_paths: &[ArchiveBatchPathRow],
    replayed_archive_files: &HashSet<String>,
) -> Option<(NaiveDate, NaiveDate)> {
    if !ready_state {
        return None;
    }
    let mut dates = rows
        .iter()
        .filter_map(|row| {
            parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
                Shanghai
                    .timestamp_millis_opt(timestamp)
                    .single()
                    .map(|value| value.date_naive())
            })
        })
        .collect::<HashSet<_>>();
    for path in archive_paths {
        if replayed_archive_files.contains(path.file_path()) {
            continue;
        }
        if path.coverage_start_at().is_none() || path.coverage_end_at().is_none() {
            return None;
        }
        insert_long_term_date_range(&mut dates, path.coverage_start_at(), path.coverage_end_at());
    }
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

async fn load_long_term_refresh_replayed_archives(
    pool: &Pool<Sqlite>,
    ready_state: bool,
) -> Result<HashSet<String>> {
    if !ready_state {
        return Ok(HashSet::new());
    }
    let rows = match sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT replay.file_path, replay.source_identity FROM hourly_rollup_archive_replay replay INNER JOIN archive_batches batches ON batches.dataset = 'codex_invocations' AND batches.file_path = replay.file_path AND batches.sha256 = replay.archive_sha256 WHERE replay.target = ?1 AND replay.dataset = 'codex_invocations'",
    ).bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET).fetch_all(pool).await {
        Ok(rows) => rows,
        Err(error) if error.to_string().contains("no such table") => return Ok(HashSet::new()),
        Err(error) => return Err(error.into()),
    };
    Ok(rows
        .into_iter()
        .filter(|(file_path, identity)| {
            identity.as_deref().is_some_and(|identity| {
                long_term_archive_file_identity(file_path).ok().as_deref() == Some(identity)
            })
        })
        .map(|(file_path, _)| file_path)
        .collect())
}

async fn clear_stale_long_term_refresh_replay_markers(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let cleanup = delete_long_term_refresh_replay_markers_with_control(
        pool,
        "DELETE FROM hourly_rollup_archive_replay WHERE rowid IN (SELECT replay.rowid FROM hourly_rollup_archive_replay replay WHERE replay.target = ?1 AND replay.dataset = 'codex_invocations' AND EXISTS (SELECT 1 FROM archive_batches batches WHERE batches.dataset = 'codex_invocations' AND batches.file_path = replay.file_path AND datetime(batches.created_at) > datetime(replay.replayed_at)) LIMIT ?2)",
        &[LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string()],
        control,
    ).await;
    match cleanup {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("no such table") => Ok(()),
        Err(error) => Err(error),
    }
}

async fn load_long_term_refresh_archive_paths(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    match load_completed_invocation_archive_paths(pool).await {
        Ok(paths) => Ok(paths),
        Err(error) if error.to_string().contains("no such table") => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

async fn prepare_long_term_refresh(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    initial_materialization: bool,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermRefreshSetup> {
    let previous_state = sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    ).bind(LONG_TERM_STATE_ID).fetch_optional(pool).await?;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    ).fetch_one(pool).await? != 0;
    let legacy_model_keys = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_usage_daily WHERE dimension = 'model' AND series_key NOT LIKE 'model:v2:%')",
    ).fetch_one(pool).await? != 0;
    let ready_state = !initial_materialization && !legacy_model_keys;
    let integrity_audit_due = ready_state && long_term_integrity_audit_due(pool).await?;
    let reconciliation = reconcile_long_term_terminal_proof(
        pool,
        &previous_state,
        ready_state,
        integrity_audit_due,
        refresh_started_at,
        control,
    )
    .await?;
    Ok(LongTermRefreshSetup {
        previous_state,
        today,
        live_tail_start: long_term_refresh_live_tail_start(today),
        live_upstream_account_id_sql: long_term_live_upstream_account_id_sql(has_attempt_table),
        ready_state,
        retention_start: today - ChronoDuration::days(retention_days.max(366) as i64 - 1),
        integrity_audit_due,
        terminal_proof_reconciliation_incomplete: reconciliation.0,
        invalidated_terminal_proof_buckets: reconciliation.1,
        account_identities: load_long_term_account_identities(pool).await?,
    })
}

async fn load_long_term_refresh_live_rows(
    pool: &Pool<Sqlite>,
    ready_state: bool,
    live_tail_start: Option<&str>,
    upstream_account_id_sql: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermRefreshLiveRows> {
    let live_sql = long_term_refresh_live_rows_sql(upstream_account_id_sql, ready_state);
    let mut rows = Vec::new();
    let mut positions = HashMap::new();
    let mut processed_rows_count = 0;
    if ready_state {
        let mut result = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql)
            .bind(live_tail_start)
            .fetch(pool);
        while let Some(row) = result.try_next().await? {
            if positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
            }
        }
    } else {
        let mut result = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql).fetch(pool);
        while let Some(row) = result.try_next().await? {
            if positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
                processed_rows_count += 1;
            }
            if processed_rows_count % 256 == 0 {
                persist_long_term_refresh_progress(pool, processed_rows_count, 0, control).await?;
            }
        }
    }
    Ok(LongTermRefreshLiveRows {
        rows,
        positions,
        processed_rows_count,
    })
}

fn long_term_refresh_live_rows_sql(upstream_account_id_sql: &str, ready_state: bool) -> String {
    let tail_predicate = ready_state.then_some(
        " AND (datetime(inv.occurred_at) >= datetime(?1) OR (inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)))",
    ).unwrap_or_default();
    format!(
        "SELECT inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model, CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort, {upstream_account_id_sql} AS upstream_account_id, NULL AS upstream_account_kind, NULL AS upstream_account_name, inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms, inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms, inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message FROM codex_invocations inv WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending'){tail_predicate} ORDER BY inv.occurred_at ASC, inv.id ASC"
    )
}

fn long_term_live_upstream_account_id_sql(has_attempt_table: bool) -> &'static str {
    if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    }
}

async fn reconcile_long_term_terminal_proof(
    pool: &Pool<Sqlite>,
    previous_state: &Option<LongTermStateRow>,
    ready_state: bool,
    integrity_audit_due: bool,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<(bool, Vec<i64>)> {
    let was_incomplete = previous_state.as_ref().is_some_and(|state| {
        state.status == LONG_TERM_STATUS_ERROR
            && state.last_error.as_deref() == Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
    });
    let (incomplete, invalidated, unavailable_paths) = if ready_state
        || integrity_audit_due
        || was_incomplete
    {
        match backfill_invocation_rollup_hourly_from_sources(pool).await {
            Ok(result) => (
                !result.source_complete,
                result.invalidated_bucket_start_epochs,
                result.unavailable_archive_file_paths,
            ),
            Err(error) => {
                warn!(error = %error, "terminal integrity proof source reconciliation failed; keeping affected buckets untrusted");
                (true, Vec::new(), Vec::new())
            }
        }
    } else {
        (was_incomplete, Vec::new(), Vec::new())
    };
    if incomplete {
        persist_long_term_terminal_proof_unavailable(pool, refresh_started_at, control).await?;
    }
    if !unavailable_paths.is_empty() {
        clear_long_term_invocation_replay_markers_for_unavailable_sources(
            pool,
            &unavailable_paths,
            control,
        )
        .await?;
    }
    Ok((incomplete, invalidated))
}

async fn persist_long_term_terminal_proof_unavailable(
    pool: &Pool<Sqlite>,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query("UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))")
        .bind(LONG_TERM_STATUS_ERROR).bind(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        .bind(LONG_TERM_STATE_ID).bind(LONG_TERM_STATUS_PREPARING).bind(refresh_started_at)
        .execute(&mut *transaction).await?;
    control.commit(transaction, permit).await
}
