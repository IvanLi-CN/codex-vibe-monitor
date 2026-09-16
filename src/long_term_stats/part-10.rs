async fn delete_initial_long_term_rollups_for_date(
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

struct LongTermRefreshRollupInput<'a> {
    hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    daily: &'a HashMap<(String, String, String), LongTermBucket>,
    recomputed_dates: &'a HashSet<NaiveDate>,
    retention_start: NaiveDate,
    integrity_repair_failures: &'a [LongTermIntegrityMismatch],
    completed_integrity_repairs: &'a HashSet<NaiveDate>,
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
}

struct LongTermRefreshArchiveMarkers<'a> {
    archive_markers: &'a [(String, String)],
    archive_read_failed: bool,
    failed_archive_paths: &'a HashSet<String>,
    clear_all_attempt_markers: bool,
    failed_archive_ranges: &'a [(String, String)],
    attempt_archive_markers: &'a HashSet<(String, String)>,
}

async fn apply_long_term_refresh_rollups_with_control(
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

async fn persist_long_term_refresh_archive_markers_with_control(
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

async fn persist_long_term_refresh_archive_marker_with_control(
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

async fn delete_long_term_refresh_archive_marker_with_control(
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

async fn delete_long_term_refresh_replay_markers_with_control(
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
struct LongTermAccountIdentity {
    id: i64,
    kind: String,
    display_name: String,
}

async fn load_long_term_account_identities(
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

fn long_term_archive_end_date(raw: &str) -> Option<NaiveDate> {
    parse_long_term_timestamp_ms(raw).and_then(|timestamp| {
        Shanghai
            .timestamp_millis_opt(timestamp)
            .single()
            .map(|value| value.date_naive())
    })
}

fn insert_long_term_date_range(
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

fn hydrate_long_term_account_identity(
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

fn merge_long_term_invocation_row(
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

fn accumulate_long_term_invocation(
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
