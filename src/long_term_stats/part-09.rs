async fn refresh_long_term_stats_inner(
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
