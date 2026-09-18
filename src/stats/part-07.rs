pub(crate) async fn load_unmaterialized_invocation_archive_failure_rows(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<Vec<ArchivedInvocationFailureRow>> {
    let archive_rows = load_invocation_archives_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        Some((start, end)),
    )
    .await?;
    let pending_state = load_pending_invocation_archive_failure_state(
        archive_rows,
        start,
        end,
        source_scope,
        exclude_invocation_ids,
    )
    .await?;

    let materialized_bucket_sources =
        load_materialized_rollup_bucket_sources(
            pool,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            &pending_state
                .unmaterialized_rows
                .iter()
                .filter_map(|row| {
                    summary_rollup_bucket_start_epoch(&row.occurred_at)
                        .ok()
                        .map(|bucket_start_epoch| (bucket_start_epoch, row.source.clone()))
                })
                .chain(pending_state.materialized_row_counts.keys().map(
                    |(bucket_start_epoch, source, _, _, _)| (*bucket_start_epoch, source.clone()),
                ))
                .collect::<HashSet<_>>(),
        )
        .await?;
    let relevant_failure_keys = pending_state
        .unmaterialized_rows
        .iter()
        .filter_map(|row| archived_failure_rollup_key(row).ok().flatten())
        .chain(pending_state.materialized_row_counts.keys().cloned())
        .filter(|(bucket_start_epoch, source, _, _, _)| {
            materialized_bucket_sources.contains(&(*bucket_start_epoch, source.clone()))
        })
        .collect::<HashSet<_>>();
    let (completed_archive_row_counts, mut unreadable_materialized_bucket_start_epochs) =
        load_materialized_failure_rollup_row_counts_for_keys(
            pool,
            start,
            end,
            source_scope,
            &relevant_failure_keys,
        )
        .await?;
    let mut known_materialized_row_counts = completed_archive_row_counts;
    for (key, count) in &pending_state.materialized_row_counts {
        *known_materialized_row_counts
            .entry(key.clone())
            .or_default() += *count;
    }
    unreadable_materialized_bucket_start_epochs
        .extend(pending_state.unreadable_materialized_bucket_start_epochs);
    let missing_row_counts = load_missing_failure_rollup_row_counts_for_rows(
        pool,
        &pending_state.unmaterialized_rows,
        &materialized_bucket_sources,
        &known_materialized_row_counts,
        &unreadable_materialized_bucket_start_epochs,
    )
    .await?;
    if missing_row_counts.is_empty() {
        return Ok(Vec::new());
    }

    let mut emitted_row_counts = HashMap::<(i64, String, String, i64, String), usize>::new();
    Ok(pending_state
        .unmaterialized_rows
        .into_iter()
        .filter(|row| {
            let Ok(Some(key)) = archived_failure_rollup_key(row) else {
                return false;
            };
            let Some(missing_count) = missing_row_counts.get(&key) else {
                return false;
            };
            let emitted_count = emitted_row_counts.entry(key).or_default();
            if *emitted_count >= *missing_count {
                return false;
            }
            *emitted_count += 1;
            true
        })
        .collect())
}

async fn load_pending_invocation_archive_failure_state(
    archive_rows: Vec<ArchiveBatchPathRow>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<PendingInvocationArchiveFailureState> {
    let mut pending_state = PendingInvocationArchiveFailureState::default();
    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "failure-breakdown").await?
        else {
            if archive_row.historical_rollups_materialized_at.is_some() {
                pending_state
                    .unreadable_materialized_bucket_start_epochs
                    .extend(archive_bucket_start_epochs_for_row(&archive_row)?);
            }
            continue;
        };
        let batch_rows =
            load_failure_rows_from_archive_pool(&archive_pool, start, end, source_scope)
                .await?
                .into_iter()
                .filter(|row| !exclude_invocation_ids.is_some_and(|ids| ids.contains(&row.id)))
                .collect::<Vec<_>>();
        if archive_row.historical_rollups_materialized_at.is_some() {
            accumulate_failure_rollup_row_counts(
                &mut pending_state.materialized_row_counts,
                batch_rows,
            )?;
        } else {
            pending_state.unmaterialized_rows.extend(batch_rows);
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    Ok(pending_state)
}

pub(crate) async fn load_materialized_failure_rollup_row_counts_for_keys(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    keys: &HashSet<(i64, String, String, i64, String)>,
) -> Result<(
    HashMap<(i64, String, String, i64, String), usize>,
    HashSet<i64>,
)> {
    if keys.is_empty() {
        return Ok((HashMap::new(), HashSet::new()));
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(
        keys.iter()
            .map(|(bucket_start_epoch, _, _, _, _)| *bucket_start_epoch),
    );
    let archive_rows = load_replayed_invocation_archives_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        &month_keys,
    )
    .await?;
    let mut counts = HashMap::<(i64, String, String, i64, String), usize>::new();
    let mut unreadable_bucket_start_epochs = HashSet::new();

    for archive in archive_rows {
        let archive_row = materialized_archive_path_row(
            archive.file_path.clone(),
            archive.coverage_start_at.clone(),
            archive.coverage_end_at.clone(),
        );
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "failure-breakdown").await?
        else {
            unreadable_bucket_start_epochs.extend(replayed_archive_bucket_start_epochs(&archive)?);
            continue;
        };
        let batch_rows =
            load_failure_rows_from_archive_pool(&archive_pool, start, end, source_scope).await?;
        for row in batch_rows {
            let Some(key) = archived_failure_rollup_key(&row)? else {
                continue;
            };
            if !keys.contains(&key) {
                continue;
            }
            *counts.entry(key).or_default() += 1;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((counts, unreadable_bucket_start_epochs))
}

pub(crate) async fn rebuild_invocation_summary_rollups_from_archive_batch(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    cleared_rollup_buckets: &mut ClearedSummaryRollupBuckets,
    targets: &[&str],
    replace_existing_rollups: bool,
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }

    let archive_path = PathBuf::from(&archive_row.file_path);
    if !archive_path.exists() {
        bail!(
            "{}: {}",
            MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX,
            archive_row.file_path
        );
    }

    let mut cursor_id = 0_i64;
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;

    loop {
        let mut rows = load_invocation_hourly_source_rows_after_id(
            &archive_pool,
            cursor_id,
            source_scope,
            BACKFILL_BATCH_SIZE,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        if replace_existing_rollups {
            for row in &rows {
                let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
                let key = (bucket_start_epoch, row.source.clone());
                let targets_to_clear =
                    cleared_rollup_buckets.targets_to_clear_for_bucket(&key, targets);
                if targets_to_clear.is_empty() {
                    continue;
                }
                delete_invocation_summary_rollup_bucket_tx(tx, key.0, &key.1, &targets_to_clear)
                    .await?;
            }
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }

    archive_pool.close().await;
    drop(temp_cleanup);

    for target in targets {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }

    Ok(())
}

pub(crate) fn summary_rollup_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    let occurred_at_utc = parse_to_utc_datetime(occurred_at)
        .ok_or_else(|| anyhow!("failed to parse invocation occurred_at: {occurred_at}"))?;
    Ok(align_bucket_epoch(occurred_at_utc.timestamp(), 3600, 0))
}

pub(crate) async fn delete_invocation_summary_rollup_bucket_tx(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    targets: &[&str],
) -> Result<()> {
    if targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS) {
        sqlx::query(
            "DELETE FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    if targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES) {
        sqlx::query(
            "DELETE FROM invocation_failure_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(crate) async fn load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
    tx: &mut SqliteConnection,
    cleared_rollup_buckets: &HashSet<(i64, String)>,
    source_scope: InvocationSourceScope,
    end_at_id: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    if cleared_rollup_buckets.is_empty() || end_at_id <= 0 {
        return Ok(Vec::new());
    }

    let min_bucket_epoch = cleared_rollup_buckets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum cleared summary rollup bucket epoch"))?;
    let max_bucket_epoch = cleared_rollup_buckets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum cleared summary rollup bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum cleared summary rollup bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum cleared summary rollup bucket epoch"))?;

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id <=
        "#,
    );
    query
        .push_bind(end_at_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(min_bucket_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(max_bucket_end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY id ASC");

    let rows = query
        .build_query_as::<InvocationHourlySourceRecord>()
        .fetch_all(&mut *tx)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            summary_rollup_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_start_epoch| {
                    cleared_rollup_buckets.contains(&(bucket_start_epoch, row.source.clone()))
                })
                .unwrap_or(false)
        })
        .collect())
}

pub(crate) async fn rebuild_invocation_summary_rollups_from_live_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    targets: &[&str],
    start_after_id: i64,
) -> Result<i64> {
    let mut cursor_id = start_after_id;
    loop {
        let mut rows = load_live_invocation_hourly_source_rows_after_id(
            &mut *tx,
            cursor_id,
            source_scope,
            BACKFILL_BATCH_SIZE,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }
    Ok(cursor_id)
}

pub(crate) async fn mark_materialized_invocation_summary_archive_replayed_tx(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
) -> Result<()> {
    for target in INVOCATION_SUMMARY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn hourly_rollup_progress_exists(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1 LIMIT 1",
    )
    .bind(dataset)
    .fetch_optional(executor)
    .await?
    .is_some())
}

pub(crate) async fn invocation_summary_repair_live_cursor_state(
    pool: &Pool<Sqlite>,
) -> Result<(bool, bool, i64, i64)> {
    let repair_marker_done =
        load_hourly_rollup_live_progress(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .await?
            >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE;
    let repair_live_cursor_exists = hourly_rollup_progress_exists(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = if repair_live_cursor_exists {
        load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((
        repair_marker_done,
        repair_live_cursor_exists,
        shared_live_cursor,
        repair_live_cursor,
    ))
}

pub(crate) async fn invocation_summary_repair_live_cursor_state_tx(
    tx: &mut SqliteConnection,
) -> Result<(bool, bool, i64, i64)> {
    let repair_marker_done =
        load_hourly_rollup_live_progress_tx(tx, INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .await?
            >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE;
    let repair_live_cursor_exists = hourly_rollup_progress_exists(
        &mut *tx,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = if repair_live_cursor_exists {
        load_hourly_rollup_live_progress_tx(
            tx,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((
        repair_marker_done,
        repair_live_cursor_exists,
        shared_live_cursor,
        repair_live_cursor,
    ))
}

pub(crate) async fn repair_invocation_summary_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
        invocation_summary_repair_live_cursor_state(pool).await?;
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor >= shared_live_cursor {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
        invocation_summary_repair_live_cursor_state_tx(tx.as_mut()).await?;
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor >= shared_live_cursor {
        tx.rollback().await?;
        return Ok(());
    }
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor < shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    }

    let archive_rows = load_completed_invocation_archive_paths(tx.as_mut()).await?;
    let preserve_materialized_archives = archive_rows.iter().any(|archive_row| {
        archive_row.historical_rollups_materialized_at.is_some()
            && !PathBuf::from(&archive_row.file_path).exists()
    });
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;

    clear_invocation_summary_rollups_if_needed(tx.as_mut(), preserve_materialized_archives).await?;

    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let preserve_materialized_archive =
            archive_row.historical_rollups_materialized_at.is_some()
                && !PathBuf::from(&archive_row.file_path).exists();
        if preserve_materialized_archive {
            mark_materialized_invocation_summary_archive_replayed_tx(tx.as_mut(), archive_row)
                .await?;
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            preserve_materialized_archives,
        )
        .await?;
    }
    let mut restored_live_rows = load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
        tx.as_mut(),
        &cleared_rollup_buckets.overall,
        InvocationSourceScope::All,
        shared_live_cursor,
    )
    .await?;
    restored_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_live_rows,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        )
        .await?;
    }
    let live_cursor_id = rebuild_invocation_summary_rollups_from_live_rows(
        tx.as_mut(),
        InvocationSourceScope::All,
        &mut seen_ids,
        &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        if preserve_materialized_archives {
            shared_live_cursor
        } else {
            0
        },
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        live_cursor_id.max(shared_live_cursor),
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn clear_invocation_summary_rollups_if_needed(
    tx: &mut SqliteConnection,
    preserve_materialized_archives: bool,
) -> Result<()> {
    if preserve_materialized_archives {
        return Ok(());
    }
    sqlx::query("DELETE FROM invocation_rollup_hourly")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM invocation_failure_rollup_hourly")
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub(crate) async fn backfill_missing_invocation_summary_archive_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(pool).await?;
    if archive_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(tx.as_mut()).await?;
    if archive_rows.is_empty() {
        tx.rollback().await?;
        return Ok(());
    }

    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let needs_overall = archive_row.needs_overall.unwrap_or_default() != 0;
        let needs_failures = archive_row.needs_failures.unwrap_or_default() != 0;
        let mut targets = Vec::new();
        if needs_overall {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATIONS);
        }
        if needs_failures {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
        }
        if targets.is_empty() {
            continue;
        }
        let archive_path = PathBuf::from(&archive_row.file_path);
        if archive_row.historical_rollups_materialized_at.is_some() && !archive_path.exists() {
            for target in &targets {
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    target,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_row.file_path,
                )
                .await?;
            }
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &targets,
            true,
        )
        .await?;
    }
    let mut restored_overall_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.overall,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_overall_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_overall_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_overall_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATIONS],
        )
        .await?;
    }
    let mut restored_failure_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.failures,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_failure_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_failure_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_failure_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready(pool: &Pool<Sqlite>) -> Result<()> {
    if load_completed_invocation_archive_paths(pool)
        .await?
        .is_empty()
    {
        return Ok(());
    }

    repair_invocation_summary_rollups(pool).await?;
    backfill_missing_invocation_summary_archive_rollups(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready_best_effort(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    match ensure_invocation_summary_rollups_ready(pool).await {
        Ok(()) => Ok(()),
        Err(err) if is_missing_invocation_summary_archive_error(&err) => {
            warn!(
                error = %err,
                "skipping invocation summary rollup repair because an archive batch file is missing; reusing current rollups for historical range queries"
            );
            Ok(())
        }
        Err(err) if is_unreadable_invocation_summary_archive_error(&err) => {
            warn!(
                error = %err,
                "skipping invocation summary rollup repair because an archive batch is unreadable; reusing current rollups for historical range queries"
            );
            Ok(())
        }
        Err(err) => Err(err),
    }
}
