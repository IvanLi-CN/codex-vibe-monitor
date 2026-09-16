async fn commit_long_term_projection_date_rebuild_chunk_with_control(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    publication: LongTermProjectionRebuildPublication<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    debug_assert!(rebuilds.len() <= LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES);
    let chunk_repaired_start_date = rebuilds
        .iter()
        .filter(|rebuild| !rebuild.daily.is_empty())
        .map(|rebuild| rebuild.bucket_date.as_str())
        .min()
        .map(str::to_string);
    let repaired_start_date = publication
        .repaired_start_date
        .map(str::to_string)
        .or(chunk_repaired_start_date);
    let repaired_nonempty = repaired_start_date.is_some();

    let mut rebuild_tokens = Vec::with_capacity(rebuilds.len());
    for rebuild in rebuilds {
        let token = format!("long-term-date:{}", rebuild.bucket_date);
        ensure_long_term_projection_daily_backup(pool, rebuild, &token, control).await?;
        if let Some(publication_token) = publication.publication_token {
            let publication_generation = publication
                .clear_dirty_buckets
                .iter()
                .find(|dirty| dirty.bucket_date == rebuild.bucket_date)
                .map(|dirty| dirty.generation);
            stage_long_term_projection_date_publication(
                pool,
                &rebuild.bucket_date,
                &token,
                publication_token,
                publication_generation,
                control,
            )
            .await?;
        }
        clear_long_term_projection_rebuild_members(pool, &token, control).await?;
        rebuild_tokens.push(token.clone());
        if rebuild.interval_segments.is_empty() {
            let (mut transaction, permit) = control.begin(pool).await?;
            sqlx::query(
                "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 0) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, updated_at = datetime('now')",
            )
            .bind(&rebuild.bucket_date)
            .execute(&mut *transaction)
            .await?;
            control.commit(transaction, permit).await?;
        }
        for (batch_index, batch) in rebuild
            .interval_segments
            .chunks(LONG_TERM_PROJECTION_REBUILD_SEGMENT_ROWS)
            .enumerate()
        {
            let (mut transaction, permit) = control.begin(pool).await?;
            if batch_index == 0 {
                sqlx::query(
                    "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 0) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, updated_at = datetime('now')",
                )
                .bind(&rebuild.bucket_date)
                .execute(&mut *transaction)
                .await?;
            }
            for segment in batch {
                sqlx::query(
                    "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(invocation_row_id) DO UPDATE SET model_series_key = excluded.model_series_key, upstream_series_key = excluded.upstream_series_key, interval_start_ms = excluded.interval_start_ms, interval_end_ms = excluded.interval_end_ms",
                )
                .bind(segment.invocation_row_id)
                .bind(&segment.model_series_key)
                .bind(&segment.upstream_series_key)
                .bind(segment.interval_start_ms)
                .bind(segment.interval_end_ms)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT OR IGNORE INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES (?1, ?2)",
                )
                .bind(&token)
                .bind(segment.invocation_row_id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "DELETE FROM long_term_projection_interval_suppressions WHERE invocation_row_id = ?1 AND bucket_date = ?2",
                )
                .bind(segment.invocation_row_id)
                .bind(&rebuild.bucket_date)
                .execute(&mut *transaction)
                .await?;
            }
            control.commit(transaction, permit).await?;
        }

        loop {
            let (mut transaction, permit) = control.begin(pool).await?;
            let suppressed = sqlx::query(
                r#"
                INSERT OR IGNORE INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date)
                SELECT candidate.invocation_row_id, ?1
                FROM (
                    SELECT state.invocation_row_id
                    FROM long_term_projection_interval_state state
                    WHERE state.interval_start_ms < ?2
                      AND state.interval_end_ms > ?3
                    UNION
                    SELECT legacy.invocation_row_id
                    FROM long_term_projection_intervals legacy
                    WHERE legacy.bucket_date = ?1
                ) candidate
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_rebuild_members member
                    WHERE member.rebuild_token = ?4
                      AND member.invocation_row_id = candidate.invocation_row_id
                )
                  AND NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_interval_suppressions suppressed
                    WHERE suppressed.invocation_row_id = candidate.invocation_row_id
                      AND suppressed.bucket_date = ?1
                  )
                LIMIT ?5
                "#,
            )
            .bind(&rebuild.bucket_date)
            .bind(rebuild.end_epoch * 1_000)
            .bind(rebuild.start_epoch * 1_000)
            .bind(&token)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            control.commit(transaction, permit).await?;
            if suppressed == 0 {
                break;
            }
        }
    }

    for rebuild in rebuilds {
        replace_long_term_projection_date_rollups(pool, rebuild, control).await?;
    }

    for token in &rebuild_tokens {
        clear_long_term_projection_rebuild_members(pool, token, control).await?;
    }

    if !publication.publish_state {
        return Ok(());
    }

    // Publishing the token, cursor, and status is a single small transaction. Cleanup only
    // removes already-published indirection, so cancellation cannot expose a staged prefix.
    let (mut transaction, permit) = control.begin(pool).await?;
    if publication.mark_ready {
        let initial_marker = sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_optional(&mut *transaction)
        .await?
        .flatten();
        if initial_marker.as_deref() == Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR) {
            bail!(
                "long-term projection baseline cannot publish over an incomplete initial materialization"
            );
        }
    }
    if let Some(publication_token) = publication.publication_token {
        sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1) ON CONFLICT(publication_token) DO UPDATE SET published = 1, updated_at = datetime('now')",
        )
        .bind(publication_token)
        .execute(&mut *transaction)
        .await?;
    } else {
        let published_dirty = publication
            .clear_dirty_buckets
            .iter()
            .filter(|dirty| {
                rebuilds
                    .iter()
                    .any(|rebuild| rebuild.bucket_date == dirty.bucket_date)
            })
            .collect::<Vec<_>>();
        if !published_dirty.is_empty() {
            let mut query = QueryBuilder::<Sqlite>::new(
                "DELETE FROM long_term_projection_dirty_buckets WHERE ",
            );
            for (index, dirty) in published_dirty.iter().enumerate() {
                if index > 0 {
                    query.push(" OR ");
                }
                query
                    .push("(bucket_date = ")
                    .push_bind(&dirty.bucket_date)
                    .push(" AND generation = ")
                    .push_bind(dirty.generation)
                    .push(")");
            }
            query.build().execute(&mut *transaction).await?;
        }
    }
    if publication.publication_token.is_none() && !rebuild_tokens.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE ",
        );
        for (index, (rebuild, token)) in rebuilds.iter().zip(&rebuild_tokens).enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(&rebuild.bucket_date)
                .push(" AND active_daily_backup_token = ")
                .push_bind(token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE ",
        );
        for (index, (rebuild, token)) in rebuilds.iter().zip(&rebuild_tokens).enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(&rebuild.bucket_date)
                .push(" AND rebuild_token = ")
                .push_bind(token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
    }
    if let Some(cursor) = publication.next_cursor {
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = MAX(long_term_projection_state.cursor_row_id, excluded.cursor_row_id), last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(cursor)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 OR (?2 AND ?3 AND status = ?4) THEN ?5 ELSE status END, statistics_start_date = CASE WHEN ?6 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?6 < statistics_start_date THEN ?6 ELSE statistics_start_date END, last_error = CASE WHEN ?1 OR (?2 AND ?3 AND status = ?4) THEN NULL ELSE last_error END, updated_at = datetime('now') WHERE id = ?7",
    )
    .bind(publication.mark_ready)
    .bind(repaired_nonempty)
    .bind(publication.publish_state)
    .bind(LONG_TERM_STATUS_EMPTY)
    .bind(LONG_TERM_STATUS_READY)
    .bind(repaired_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if publication.publication_token.is_none() {
        let _ = finish_long_term_projection_backup_cleanup(pool, control).await?;
    }
    Ok(())
}

async fn release_long_term_projection_date_publication(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    publication_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for rebuild in rebuilds {
        let rebuild_token = format!("long-term-date:{}", rebuild.bucket_date);
        let publication_generation = clear_dirty_buckets
            .iter()
            .find(|dirty| dirty.bucket_date == rebuild.bucket_date)
            .map(|dirty| dirty.generation);
        release_long_term_projection_publication_member(
            pool,
            &rebuild.bucket_date,
            &rebuild_token,
            publication_generation,
            publication_token,
            control,
        )
        .await?;
    }
    let _ = prune_long_term_projection_publications(pool, control).await?;
    Ok(())
}

async fn release_long_term_projection_publication_member(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    publication_generation: Option<i64>,
    publication_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let has_newer_dirty = if let Some(publication_generation) = publication_generation {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1 AND generation <> ?2)",
        )
        .bind(bucket_date)
        .bind(publication_generation)
        .fetch_one(&mut *transaction)
        .await?
            != 0
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1)",
        )
        .bind(bucket_date)
        .fetch_one(&mut *transaction)
        .await?
            != 0
    };
    if has_newer_dirty {
        control.commit(transaction, permit).await?;
        return Ok(false);
    }
    if let Some(publication_generation) = publication_generation {
        sqlx::query(
            "DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1 AND generation = ?2",
        )
        .bind(bucket_date)
        .bind(publication_generation)
        .execute(&mut *transaction)
        .await?;
    }
    let released = sqlx::query(
        "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE bucket_date = ?1 AND active_daily_backup_token = ?2 AND publication_token = ?3",
    )
    .bind(bucket_date)
    .bind(rebuild_token)
    .bind(publication_token)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if released != 0 {
        sqlx::query(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1 AND rebuild_token = ?2",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;
    if released != 0 {
        control.complete_backup_cleanup_marker();
        control.check()?;
    }
    Ok(released != 0)
}

async fn finish_long_term_projection_publication_cleanup(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    if finish_long_term_projection_backup_cleanup(pool, control).await? {
        return Ok(true);
    }
    let member = sqlx::query_as::<_, LongTermProjectionPublicationMember>(
        "SELECT state.bucket_date, state.active_daily_backup_token AS rebuild_token, state.publication_token, state.publication_generation FROM long_term_projection_bucket_state state JOIN long_term_projection_date_publications publication ON publication.publication_token = state.publication_token WHERE publication.published = 1 AND state.active_daily_backup_token IS NOT NULL ORDER BY state.updated_at ASC, state.bucket_date ASC LIMIT ?1",
    )
    .bind(1_i64)
    .fetch_optional(pool)
    .await?;
    if let Some(member) = member {
        let _released = release_long_term_projection_publication_member(
            pool,
            &member.bucket_date,
            &member.rebuild_token,
            member.publication_generation,
            &member.publication_token,
            control,
        )
        .await?;
        return Ok(true);
    }
    prune_long_term_projection_publications(pool, control).await
}

async fn prune_long_term_projection_publications(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        r#"
            DELETE FROM long_term_projection_date_publications
            WHERE rowid IN (
                SELECT publication.rowid
                FROM long_term_projection_date_publications publication
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_bucket_state state
                    WHERE state.publication_token = publication.publication_token
                )
                ORDER BY publication.updated_at ASC, publication.publication_token ASC
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(deleted != 0)
}

async fn rebuild_long_term_projection_date(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    let rebuild = build_long_term_projection_date_rebuild(pool, bucket_date, &control).await?;
    commit_long_term_projection_date_rebuilds(pool, &[rebuild], None, &[], false).await
}

pub(crate) async fn bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    // Earlier schemas had no terminal proof or retirement boundary. The existing canonical
    // history may therefore predate every source that this version can verify. Keep it outside
    // the reconstructable window instead of treating source absence as proof of an empty day.
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = COALESCE(integrity_source_start_date, ?1),
            integrity_source_pending_start_date = NULL,
            updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(today)
    .bind(LONG_TERM_STATE_ID)
    .execute(pool)
    .await
    .context("failed to bootstrap legacy long-term integrity source boundary")?;
    Ok(())
}

fn long_term_day_epoch_bounds(date: NaiveDate) -> Option<(i64, i64)> {
    let start = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    let end = date
        .succ_opt()?
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    Some((start, end))
}

fn long_term_bucket_date(bucket_start_epoch: i64) -> Option<NaiveDate> {
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|value| value.date_naive())
}

fn long_term_reconstructable_start(
    retention_start: NaiveDate,
    statistics_start_date: Option<&str>,
    integrity_source_start_date: Option<&str>,
) -> NaiveDate {
    let persisted_start = statistics_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    let integrity_source_start = integrity_source_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    retention_start
        .max(persisted_start)
        .max(integrity_source_start)
}

fn long_term_source_safe_start_after_effective_date(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

fn long_term_unreadable_source_start(
    archive_path: &ArchiveBatchPathRow,
    retention_start: NaiveDate,
) -> NaiveDate {
    archive_path
        .coverage_start_at()
        .and_then(long_term_archive_end_date)
        .unwrap_or(retention_start)
}

async fn clear_long_term_invocation_replay_markers_for_unavailable_sources(
    pool: &Pool<Sqlite>,
    file_paths: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let unique_paths = file_paths.iter().collect::<HashSet<_>>();
    for file_path in unique_paths {
        let (mut transaction, permit) = control.begin(pool).await?;
        let cleared = sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(file_path)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if cleared > 0 {
            warn!(
                file_path = %file_path,
                "cleared long-term archive replay marker after source reconciliation could not read the archive"
            );
        }
    }
    Ok(())
}

pub(crate) async fn advance_long_term_integrity_source_start_tx(
    tx: &mut SqliteConnection,
    retiring_archive_batch_id: i64,
    source_safe_start: NaiveDate,
) -> Result<()> {
    let pending_source_start = sqlx::query_scalar::<_, Option<String>>(
        "SELECT integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .map(|value| {
        NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
            anyhow!("pending long-term integrity source boundary is invalid ({value}): {error}")
        })
    })
    .transpose()?;
    let candidate = pending_source_start
        .map(|pending| pending.max(source_safe_start))
        .unwrap_or(source_safe_start);
    let candidate_start = candidate.to_string();
    let retained_source_blocks_boundary = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM archive_batches
            WHERE id <> ?1
              AND dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND status = 'completed'
              AND (coverage_start_at IS NULL OR coverage_start_at < ?2)
        )
        "#,
    )
    .bind(retiring_archive_batch_id)
    .bind(&candidate_start)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    if retained_source_blocks_boundary {
        sqlx::query(
            "UPDATE long_term_stats_state SET integrity_source_pending_start_date = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(candidate_start)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *tx)
        .await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = CASE
            WHEN integrity_source_start_date IS NULL
              OR integrity_source_start_date < ?1 THEN ?1
            ELSE integrity_source_start_date
        END,
        integrity_source_pending_start_date = NULL,
        updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(candidate_start)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn long_term_integrity_source_safe_start_for_archive_cleanup(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    match dataset {
        HOURLY_ROLLUP_DATASET_INVOCATIONS => {
            long_term_invocation_archive_safe_start(pool, file_path).await
        }
        "pool_upstream_request_attempts" => {
            long_term_attempt_archive_safe_start(pool, file_path, coverage_end_at).await
        }
        _ => Ok(None),
    }
}

async fn long_term_invocation_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<Option<NaiveDate>> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path)
        .await?
        .context("completed invocation archive has no manifest sha256")?;
    let rows = load_long_term_source_timing_rows_from_archive(
        pool,
        file_path,
        &archive_sha256,
        "long-term-stats-cleanup-invocation-boundary",
    )
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let latest_effective_date = rows
        .iter()
        .map(|row| {
            long_term_source_effective_date(&row.occurred_at, row.t_total_ms).ok_or_else(|| {
                anyhow!(
                    "invocation archive has an unparseable timestamp for source-boundary verification: {}",
                    row.occurred_at
                )
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .expect("non-empty invocation archive produces one effective date per source row");
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

async fn long_term_attempt_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
    _coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    let archive_sha256 = load_long_term_archive_sha256_for_dataset(
        pool,
        "pool_upstream_request_attempts",
        file_path,
    )
    .await?
    .context("completed attempt archive has no manifest sha256")?;
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
    let Some((archive_pool, cleanup)) = open_pool_upstream_request_attempt_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        "long-term-stats-cleanup-attempt-boundary",
    )
    .await?
    else {
        bail!("attempt archive is unavailable for long-term source-boundary verification");
    };
    let rows = sqlx::query_as::<_, LongTermArchiveAttemptRow>(
        r#"
        SELECT invoke_id, occurred_at, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id IS NOT NULL
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&archive_pool)
    .await;
    archive_pool.close().await;
    drop(cleanup);
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
    let pairs = rows?
        .into_iter()
        .filter_map(|row| {
            row.upstream_account_id
                .map(|_| (row.invoke_id, row.occurred_at))
        })
        .collect::<HashSet<_>>();
    if pairs.is_empty() {
        return Ok(None);
    }

    let (latest_effective_date, unmatched_pairs) =
        long_term_match_attempt_pairs_to_invocation_sources(pool, pairs).await?;
    if !unmatched_pairs.is_empty() {
        bail!(
            "{} attempt archive account mapping(s) have no readable invocation source",
            unmatched_pairs.len()
        );
    }
    let latest_effective_date = latest_effective_date.ok_or_else(|| {
        anyhow!("attempt archive account mappings have no parseable invocation timestamps")
    })?;
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

async fn load_long_term_source_timing_rows_from_archive(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    read_surface: &'static str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
    let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        read_surface,
    )
    .await?
    else {
        bail!("invocation archive is unavailable for long-term source-boundary verification");
    };
    let archive_columns = load_archive_table_columns(&archive_pool, "codex_invocations").await?;
    if !archive_columns.contains("occurred_at") {
        archive_pool.close().await;
        drop(cleanup);
        bail!("invocation archive has no occurred_at column for source-boundary verification");
    }
    let sql = long_term_source_timing_archive_query(&archive_columns);
    let rows = sqlx::query_as::<_, LongTermSourceTimingRow>(&sql)
        .fetch_all(&archive_pool)
        .await;
    archive_pool.close().await;
    drop(cleanup);
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
    Ok(rows?)
}

async fn long_term_match_attempt_pairs_to_invocation_sources(
    pool: &Pool<Sqlite>,
    mut unmatched_pairs: HashSet<(String, String)>,
) -> Result<(Option<NaiveDate>, HashSet<(String, String)>)> {
    let mut latest_effective_date = None;
    let live_rows =
        load_long_term_source_timing_rows_for_pairs(pool, &unmatched_pairs, "t_total_ms").await?;
    record_long_term_matched_attempt_source_rows(
        &mut unmatched_pairs,
        &mut latest_effective_date,
        live_rows,
    )?;

    if unmatched_pairs.is_empty() {
        return Ok((latest_effective_date, unmatched_pairs));
    }
    let archive_paths = load_completed_invocation_archive_paths(pool).await?;
    for archive_path in archive_paths {
        if unmatched_pairs.is_empty() {
            break;
        }
        if !long_term_archive_may_contain_attempt_pairs(&archive_path, &unmatched_pairs) {
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
            "long-term-stats-cleanup-attempt-invocation-match",
        )
        .await?
        else {
            bail!("invocation archive is unavailable while verifying attempt account mappings");
        };
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        if !archive_columns.contains("invoke_id") || !archive_columns.contains("occurred_at") {
            archive_pool.close().await;
            drop(cleanup);
            bail!("invocation archive lacks keys required to verify attempt account mappings");
        }
        let t_total_ms_expression = long_term_legacy_column_expr(&archive_columns, "t_total_ms");
        let rows = load_long_term_source_timing_rows_for_pairs(
            &archive_pool,
            &unmatched_pairs,
            &t_total_ms_expression,
        )
        .await;
        archive_pool.close().await;
        drop(cleanup);
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        record_long_term_matched_attempt_source_rows(
            &mut unmatched_pairs,
            &mut latest_effective_date,
            rows?,
        )?;
    }

    Ok((latest_effective_date, unmatched_pairs))
}
