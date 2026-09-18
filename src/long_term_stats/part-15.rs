struct LongTermRefreshArchiveScanInput<'a> {
    ready_state: bool,
    retention_start: NaiveDate,
    rows: Vec<LongTermInvocationRow>,
    row_positions: HashMap<i64, usize>,
    processed_rows_count: i64,
    archive_paths: Vec<ArchiveBatchPathRow>,
    replayed_archive_files: &'a HashSet<String>,
    archive_attempt_accounts: &'a HashMap<(String, String), i64>,
}

struct LongTermRefreshArchiveScanContext<'a> {
    pool: &'a Pool<Sqlite>,
    ready_state: bool,
    retention_start: NaiveDate,
    archive_attempt_accounts: &'a HashMap<(String, String), i64>,
    state: &'a mut LongTermRefreshArchiveScanState,
}

async fn scan_long_term_refresh_archives(
    pool: &Pool<Sqlite>,
    input: LongTermRefreshArchiveScanInput<'_>,
) -> Result<LongTermRefreshArchiveScanState> {
    let LongTermRefreshArchiveScanInput {
        ready_state,
        retention_start,
        rows,
        row_positions,
        processed_rows_count,
        archive_paths,
        replayed_archive_files,
        archive_attempt_accounts,
    } = input;
    let mut state = LongTermRefreshArchiveScanState {
        rows,
        row_positions,
        processed_rows_count,
        archive_markers: Vec::new(),
        failed_archive_paths: HashSet::new(),
        failed_archive_ranges: Vec::new(),
        affected_archive_dates: HashSet::new(),
        unreadable_source_start_date: None,
        archive_read_failed: false,
        clear_all_attempt_markers: false,
    };
    let mut context = LongTermRefreshArchiveScanContext {
        pool,
        ready_state,
        retention_start,
        archive_attempt_accounts,
        state: &mut state,
    };
    for archive_path in archive_paths {
        if replayed_archive_files.contains(archive_path.file_path()) {
            continue;
        }
        scan_long_term_refresh_archive(&mut context, archive_path).await?;
    }
    Ok(state)
}

fn mark_long_term_refresh_archive_failure(
    state: &mut LongTermRefreshArchiveScanState,
    archive_path: &ArchiveBatchPathRow,
    retention_start: NaiveDate,
) {
    state.archive_read_failed = true;
    state
        .failed_archive_paths
        .insert(archive_path.file_path().to_string());
    let unreadable_start = long_term_unreadable_source_start(archive_path, retention_start);
    state.unreadable_source_start_date = Some(
        state
            .unreadable_source_start_date
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
            state
                .failed_archive_ranges
                .push((start.to_string(), end.to_string()));
        }
        _ => state.clear_all_attempt_markers = true,
    }
}

fn merge_long_term_archive_scan_rows(
    state: &mut LongTermRefreshArchiveScanState,
    archive_rows: Vec<LongTermInvocationRow>,
    archive_attempt_accounts: &HashMap<(String, String), i64>,
) {
    for mut row in archive_rows {
        hydrate_long_term_archive_attempt_account(&mut row, archive_attempt_accounts);
        if let Some(date) = parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
            Shanghai
                .timestamp_millis_opt(timestamp)
                .single()
                .map(|value| value.date_naive())
        }) {
            state.affected_archive_dates.insert(date);
        }
        if let Some(index) = state.row_positions.get(&row.id).copied() {
            merge_long_term_invocation_row(&mut row, &state.rows[index]);
            state.rows[index] = row;
        } else {
            state.row_positions.insert(row.id, state.rows.len());
            state.rows.push(row);
            state.processed_rows_count += 1;
        }
    }
}

async fn scan_long_term_refresh_archive(
    context: &mut LongTermRefreshArchiveScanContext<'_>,
    archive_path: ArchiveBatchPathRow,
) -> Result<()> {
    let archive_sha256_before_open = match crate::maintenance::sha256_hex_file(
        std::path::Path::new(archive_path.file_path()),
    ) {
        Ok(value) => value,
        Err(error) => {
            mark_long_term_refresh_archive_failure(
                context.state,
                &archive_path,
                context.retention_start,
            );
            warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive identity read failed");
            return Ok(());
        }
    };
    let archive_manifest_sha256 =
        load_long_term_archive_sha256(context.pool, archive_path.file_path()).await?;
    if !long_term_archive_scan_identity_matches_manifest(
        &archive_sha256_before_open,
        Some(&archive_sha256_before_open),
        archive_manifest_sha256.as_deref(),
    ) {
        mark_long_term_refresh_archive_failure(
            context.state,
            &archive_path,
            context.retention_start,
        );
        warn!(
            file_path = archive_path.file_path(),
            "long-term stats archive does not match its completed manifest; clearing its replay marker for retry"
        );
        return Ok(());
    }
    let Some((archive_pool, cleanup)) = (match open_invocation_archive_batch_pool(
        &archive_path,
        "long-term-stats",
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            mark_long_term_refresh_archive_failure(
                context.state,
                &archive_path,
                context.retention_start,
            );
            warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive read failed");
            None
        }
    }) else {
        mark_long_term_refresh_archive_failure(
            context.state,
            &archive_path,
            context.retention_start,
        );
        return Ok(());
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
    finish_long_term_refresh_archive_scan(
        context,
        archive_path,
        archive_sha256_before_open,
        archive_rows,
    )
    .await
}

async fn finish_long_term_refresh_archive_scan(
    context: &mut LongTermRefreshArchiveScanContext<'_>,
    archive_path: ArchiveBatchPathRow,
    archive_sha256_before_open: String,
    archive_rows: Result<Vec<LongTermInvocationRow>>,
) -> Result<()> {
    match archive_rows {
        Ok(archive_rows) => {
            merge_long_term_archive_scan_rows(
                context.state,
                archive_rows,
                context.archive_attempt_accounts,
            );
            if !context.ready_state {
                insert_long_term_date_range(
                    &mut context.state.affected_archive_dates,
                    archive_path.coverage_start_at(),
                    archive_path.coverage_end_at(),
                );
            }
            let archive_sha256_after_read =
                crate::maintenance::sha256_hex_file(std::path::Path::new(archive_path.file_path()));
            let archive_manifest_sha256 =
                load_long_term_archive_sha256(context.pool, archive_path.file_path()).await?;
            if long_term_archive_scan_identity_matches_manifest(
                &archive_sha256_before_open,
                archive_sha256_after_read.as_deref().ok(),
                archive_manifest_sha256.as_deref(),
            ) {
                context.state.archive_markers.push((
                    archive_path.file_path().to_string(),
                    archive_sha256_before_open,
                ));
            } else {
                mark_long_term_refresh_archive_failure(
                    context.state,
                    &archive_path,
                    context.retention_start,
                );
                warn!(
                    file_path = archive_path.file_path(),
                    error = ?archive_sha256_after_read.as_ref().err(),
                    "long-term stats archive changed while it was scanned; clearing its replay marker for retry"
                );
            }
        }
        Err(error) => {
            mark_long_term_refresh_archive_failure(
                context.state,
                &archive_path,
                context.retention_start,
            );
            warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive query failed");
        }
    }
    Ok(())
}
