async fn load_long_term_projection_rows_for_date(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
    control: &LongTermProjectionWriteControl<'_>,
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
    let select = format!(
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
        "#,
    );
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let canonical_query = long_term_projection_canonical_query(&select);
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&canonical_query)
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(pool)
        .await?;
    let crossing_text_query = long_term_projection_crossing_text_query(&select);
    rows.extend(
        sqlx::query_as::<_, LongTermInvocationRow>(&crossing_text_query)
            .bind(&start_text)
            .fetch_all(pool)
            .await?,
    );
    if let Some(rfc3339_compatibility) =
        load_long_term_projection_live_rfc3339_compatibility(pool).await?
    {
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        let rfc3339_query = long_term_projection_live_rfc3339_query(&select);
        rows.extend(
            sqlx::query_as::<_, LongTermInvocationRow>(&rfc3339_query)
                .bind(rfc3339_lower)
                .bind(rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(pool)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut row_positions = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id, index))
        .collect::<HashMap<_, _>>();
    let attempt_start = date.pred_opt().unwrap_or(date);
    let (attempt_accounts, _) =
        load_long_term_archive_attempt_accounts(pool, Some((attempt_start, date))).await?;
    let archive_paths = match load_completed_invocation_archive_paths(pool).await {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => Vec::new(),
        Err(error) => return Err(error),
    };
    for archive_path in archive_paths {
        let overlaps = match (
            archive_path
                .coverage_start_at()
                .and_then(long_term_archive_end_date),
            archive_path
                .coverage_end_at()
                .and_then(long_term_archive_end_date),
        ) {
            (Some(path_start), Some(path_end)) => path_start <= date && path_end >= attempt_start,
            // Older manifests without coverage must be inspected rather than treated as absent.
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
        let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-projection-targeted-repair",
        )
        .await?
        else {
            anyhow::bail!(
                "long-term projection source coverage incomplete for {date}: archive {} is unavailable",
                archive_path.file_path()
            );
        };
        let archive_fingerprint = match long_term_archive_pool_fingerprint(&archive_pool).await {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                archive_pool.close().await;
                drop(cleanup);
                return Err(error);
            }
        };
        let archive_rows = async {
            let archive_query = long_term_archive_invocation_query_for_range(&archive_pool).await?;
            let compatibility = load_or_inspect_long_term_archive_compatibility(
                pool,
                &archive_pool,
                archive_path.file_path(),
                &archive_fingerprint,
                &archive_query.parts,
                control,
            )
            .await?;
            load_long_term_archive_invocation_rows_for_range(
                &archive_pool,
                &archive_query,
                compatibility,
                start,
                end,
            )
            .await
        }
        .await;
        archive_pool.close().await;
        drop(cleanup);
        let archive_rows = archive_rows?;
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        for mut row in archive_rows {
            if row.upstream_account_id.is_none()
                && let Some(invoke_id) = row.invoke_id.as_ref()
                && let Some(account_id) =
                    attempt_accounts.get(&(invoke_id.clone(), row.occurred_at.clone()))
            {
                row.upstream_account_id = Some(*account_id);
            }
            if long_term_projection_row_affects_date(&row, &date.to_string()) {
                if let Some(index) = row_positions.get(&row.id).copied() {
                    // Retention can leave a pruned live row with the same id as the archive row.
                    // Preserve the archive's richer fields while using retained columns as fallback.
                    merge_long_term_invocation_row(&mut row, &rows[index]);
                    rows[index] = row;
                } else {
                    row_positions.insert(row.id, rows.len());
                    rows.push(row);
                }
            }
        }
    }
    Ok(rows)
}

async fn build_long_term_projection_date_rebuild(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionDateRebuild> {
    let date = NaiveDate::parse_from_str(bucket_date, "%Y-%m-%d")?;
    let start = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .context("invalid long-term projection day start")?;
    let end = date
        .succ_opt()
        .and_then(|next| next.and_hms_opt(0, 0, 0))
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .context("invalid long-term projection day end")?;
    let mut rows = load_long_term_projection_rows_for_date(pool, date, start, end, control).await?;
    let identities = load_long_term_account_identities(pool).await?;
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    let mut interval_segments = Vec::new();
    for row in &mut rows {
        hydrate_long_term_account_identity(row, &identities);
        let mut row_hourly = HashMap::new();
        let mut row_daily = HashMap::new();
        accumulate_long_term_invocation(
            row,
            &mut row_hourly,
            &mut row_daily,
            &mut statistics_start,
        );
        interval_segments.extend(collect_long_term_projection_interval_segments(
            &row_hourly,
            &row_daily,
            row.id,
        ));
        merge_long_term_projection_buckets(&mut hourly, row_hourly);
        merge_long_term_projection_buckets(&mut daily, row_daily);
    }
    hourly.retain(|(hour_epoch, _, _), _| {
        Shanghai
            .timestamp_opt(*hour_epoch, 0)
            .single()
            .is_some_and(|timestamp| timestamp.date_naive() == date)
    });
    daily.retain(|(date_key, _, _), _| date_key == bucket_date);
    interval_segments
        .retain(|segment| long_term_projection_interval_dates(segment).contains(bucket_date));
    Ok(LongTermProjectionDateRebuild {
        bucket_date: bucket_date.to_string(),
        start_epoch: start.timestamp(),
        end_epoch: end.timestamp(),
        hourly,
        daily,
        interval_segments,
        source_row_count: rows.len() as u64,
    })
}

async fn commit_long_term_projection_date_rebuilds(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    next_cursor: Option<i64>,
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    mark_ready: bool,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    commit_long_term_projection_date_rebuilds_with_control(
        pool,
        rebuilds,
        next_cursor,
        clear_dirty_buckets,
        mark_ready,
        &control,
    )
    .await
}

async fn clear_long_term_projection_rebuild_members(
    pool: &Pool<Sqlite>,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_projection_rebuild_members WHERE rowid IN (SELECT rowid FROM long_term_projection_rebuild_members WHERE rebuild_token = ?1 LIMIT ?2)",
        )
        .bind(rebuild_token)
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

async fn clear_long_term_projection_daily_backup(
    pool: &Pool<Sqlite>,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let has_backup = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_daily_backups WHERE rebuild_token = ?1)",
        )
        .bind(rebuild_token)
        .fetch_one(pool)
        .await?
            != 0;
        if !has_backup {
            return Ok(());
        }
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_projection_daily_backups WHERE rowid IN (SELECT rowid FROM long_term_projection_daily_backups WHERE rebuild_token = ?1 LIMIT ?2)",
        )
        .bind(rebuild_token)
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

async fn finish_long_term_projection_backup_cleanup(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        r#"
            DELETE FROM long_term_projection_daily_backups
            WHERE rowid IN (
                SELECT backup.rowid
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
                ORDER BY backup.rowid
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if deleted != 0 {
        control.commit(transaction, permit).await?;
        return Ok(true);
    }

    let cleared = sqlx::query(
        r#"
            UPDATE long_term_projection_bucket_state
            SET publication_token = NULL, updated_at = datetime('now')
            WHERE rowid IN (
                SELECT state.rowid
                FROM long_term_projection_bucket_state state
                WHERE state.active_daily_backup_token IS NULL
                  AND state.publication_token LIKE 'cleanup:%'
                  AND NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_daily_backups backup
                    WHERE backup.rebuild_token = substr(state.publication_token, 9)
                  )
                ORDER BY state.rowid
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(cleared != 0)
}

async fn release_long_term_projection_daily_backups(
    pool: &Pool<Sqlite>,
    backups: &[(String, String)],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for batch in backups.chunks(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES) {
        let (mut transaction, permit) = control.begin(pool).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE ",
        );
        for (index, (bucket_date, rebuild_token)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(bucket_date)
                .push(" AND active_daily_backup_token = ")
                .push_bind(rebuild_token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE ",
        );
        for (index, (bucket_date, rebuild_token)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(bucket_date)
                .push(" AND rebuild_token = ")
                .push_bind(rebuild_token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        control.commit(transaction, permit).await?;
    }
    let _ = finish_long_term_projection_backup_cleanup(pool, control).await?;
    Ok(())
}

async fn ensure_long_term_projection_daily_backup(
    pool: &Pool<Sqlite>,
    rebuild: &LongTermProjectionDateRebuild,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    ensure_long_term_projection_daily_backup_for_date(
        pool,
        &rebuild.bucket_date,
        rebuild_token,
        true,
        control,
    )
    .await
}

async fn stage_long_term_projection_date_publication(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    publication_token: &str,
    publication_generation: Option<i64>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let staged = sqlx::query(
        "UPDATE long_term_projection_bucket_state SET publication_token = ?1, publication_generation = ?2, updated_at = datetime('now') WHERE bucket_date = ?3 AND active_daily_backup_token = ?4",
    )
    .bind(publication_token)
    .bind(publication_generation)
    .bind(bucket_date)
    .bind(rebuild_token)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if staged != 1 {
        bail!("long-term projection date publication lost its backup for {bucket_date}");
    }
    control.commit(transaction, permit).await
}

async fn ensure_long_term_projection_daily_backup_for_date(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    reset_interval_baseline: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let active = sqlx::query_scalar::<_, Option<String>>(
        "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .fetch_optional(&mut *transaction)
    .await?;
    let active = active.flatten();
    if let Some(active) = active.as_deref()
        && active != rebuild_token
    {
        bail!("long-term projection daily backup for {bucket_date} is owned by {active}");
    }
    let owner = sqlx::query_scalar::<_, String>(
        "SELECT rebuild_token FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(owner) = owner.as_deref()
        && owner != rebuild_token
    {
        bail!("long-term projection daily backup for {bucket_date} is owned by {owner}");
    }
    if owner.is_none() {
        sqlx::query(
            "INSERT INTO long_term_projection_daily_backup_claims (bucket_date, rebuild_token) VALUES (?1, ?2)",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;

    // A previous worker may have completed the backup publication before cancellation. Keep
    // that complete snapshot live while the same owner resumes the replacement.
    if active.as_deref() == Some(rebuild_token) {
        return Ok(());
    }

    clear_long_term_projection_daily_backup(pool, rebuild_token, control).await?;
    let daily_row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(bucket_date)
    .fetch_one(pool)
    .await?;
    let mut offset = 0_i64;
    while offset < daily_row_count {
        let (mut transaction, permit) = control.begin(pool).await?;
        let copied = sqlx::query(
            r#"
            INSERT INTO long_term_projection_daily_backups (
                rebuild_token, stats_date, dimension, series_key, display_name, reasoning_effort,
                calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms,
                first_byte_samples, response_sum_ms, response_samples
            )
            SELECT ?1, stats_date, dimension, series_key, display_name, reasoning_effort,
                calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms,
                first_byte_samples, response_sum_ms, response_samples
            FROM long_term_usage_daily
            WHERE stats_date = ?2
            ORDER BY dimension, series_key
            LIMIT ?3 OFFSET ?4
            "#,
        )
        .bind(rebuild_token)
        .bind(bucket_date)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .bind(offset)
        .execute(&mut *transaction)
        .await?
        .rows_affected() as i64;
        control.commit(transaction, permit).await?;
        offset += copied;
    }

    let (mut transaction, permit) = control.begin(pool).await?;
    if reset_interval_baseline {
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready, active_daily_backup_token) VALUES (?1, 0, ?2) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, active_daily_backup_token = excluded.active_daily_backup_token, updated_at = datetime('now')",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, active_daily_backup_token) VALUES (?1, ?2) ON CONFLICT(bucket_date) DO UPDATE SET active_daily_backup_token = excluded.active_daily_backup_token, updated_at = datetime('now')",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await
}

async fn replace_long_term_projection_date_rollups(
    pool: &Pool<Sqlite>,
    rebuild: &LongTermProjectionDateRebuild,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let hourly = rebuild.hourly.values().collect::<Vec<_>>();
    for batch in hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_hourly(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let daily = rebuild.daily.values().collect::<Vec<_>>();
    for batch in daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_daily(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }

    // Upsert fresh rows before removing obsolete keys so a read cannot observe an empty date
    // between bounded transactions. The dirty marker remains until this entire replacement ends.
    let existing_daily = sqlx::query_as::<_, (String, String)>(
        "SELECT dimension, series_key FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(&rebuild.bucket_date)
    .fetch_all(pool)
    .await?;
    let daily_keys = daily
        .iter()
        .map(|bucket| (bucket.dimension.as_str(), bucket.series_key.as_str()))
        .collect::<HashSet<_>>();
    let obsolete_daily = existing_daily
        .into_iter()
        .filter(|(dimension, series_key)| {
            !daily_keys.contains(&(dimension.as_str(), series_key.as_str()))
        })
        .collect::<Vec<_>>();
    for batch in obsolete_daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for (dimension, series_key) in batch {
            sqlx::query(
                "DELETE FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = ?2 AND series_key = ?3",
            )
            .bind(&rebuild.bucket_date)
            .bind(dimension)
            .bind(series_key)
            .execute(&mut *transaction)
            .await?;
        }
        control.commit(transaction, permit).await?;
    }

    let existing_hourly = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT bucket_start_epoch, dimension, series_key FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
    )
    .bind(rebuild.start_epoch)
    .bind(rebuild.end_epoch)
    .fetch_all(pool)
    .await?;
    let hourly_keys = hourly
        .iter()
        .map(|bucket| {
            (
                bucket.bucket_start_epoch,
                bucket.dimension.as_str(),
                bucket.series_key.as_str(),
            )
        })
        .collect::<HashSet<_>>();
    let obsolete_hourly = existing_hourly
        .into_iter()
        .filter(|(bucket_start_epoch, dimension, series_key)| {
            !hourly_keys.contains(&(*bucket_start_epoch, dimension.as_str(), series_key.as_str()))
        })
        .collect::<Vec<_>>();
    for batch in obsolete_hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for (bucket_start_epoch, dimension, series_key) in batch {
            sqlx::query(
                "DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = ?2 AND series_key = ?3",
            )
            .bind(bucket_start_epoch)
            .bind(dimension)
            .bind(series_key)
            .execute(&mut *transaction)
            .await?;
        }
        control.commit(transaction, permit).await?;
    }

    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 1, updated_at = datetime('now')",
    )
    .bind(&rebuild.bucket_date)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn commit_long_term_projection_date_rebuilds_with_control(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    next_cursor: Option<i64>,
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    mark_ready: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if rebuilds.is_empty() {
        return commit_long_term_projection_date_rebuild_chunk_with_control(
            pool,
            rebuilds,
            LongTermProjectionRebuildPublication {
                next_cursor,
                clear_dirty_buckets,
                mark_ready,
                publish_state: true,
                publication_token: None,
                repaired_start_date: None,
            },
            control,
        )
        .await;
    }
    let publication_token = next_long_term_projection_publication_token();
    let repaired_start_date = rebuilds
        .iter()
        .filter(|rebuild| !rebuild.daily.is_empty())
        .map(|rebuild| rebuild.bucket_date.as_str())
        .min()
        .map(str::to_string);
    for (index, rebuild_chunk) in rebuilds
        .chunks(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES)
        .enumerate()
    {
        let last_chunk =
            (index + 1) * LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES >= rebuilds.len();
        let chunk_dates = rebuild_chunk
            .iter()
            .map(|rebuild| rebuild.bucket_date.as_str())
            .collect::<HashSet<_>>();
        let chunk_dirty = clear_dirty_buckets
            .iter()
            .filter(|dirty| chunk_dates.contains(dirty.bucket_date.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        // Staged chunks retain their backup pointer and dirty generation. The final bounded
        // transaction publishes a token with the cursor; readers do not expose a staged prefix.
        commit_long_term_projection_date_rebuild_chunk_with_control(
            pool,
            rebuild_chunk,
            LongTermProjectionRebuildPublication {
                next_cursor: last_chunk.then_some(next_cursor).flatten(),
                clear_dirty_buckets: &chunk_dirty,
                mark_ready: last_chunk && mark_ready,
                publish_state: last_chunk,
                publication_token: Some(&publication_token),
                repaired_start_date: last_chunk
                    .then_some(repaired_start_date.as_deref())
                    .flatten(),
            },
            control,
        )
        .await?;
        if !last_chunk {
            control.complete_rebuild_chunk();
            control.check()?;
        }
    }
    release_long_term_projection_date_publication(
        pool,
        rebuilds,
        clear_dirty_buckets,
        &publication_token,
        control,
    )
    .await?;
    Ok(())
}
