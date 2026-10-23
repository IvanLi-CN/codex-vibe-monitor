use super::*;

pub(crate) async fn load_failure_rows_from_archive_pool(
    archive_pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<Vec<ArchivedInvocationFailureRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, occurred_at, source, status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start));
    query
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .build_query_as::<ArchivedInvocationFailureRow>()
        .fetch_all(archive_pool)
        .await
        .map_err(Into::into)
}

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
                .filter(|row| {
                    !exclude_invocation_ids
                        .is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
                })
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

#[expect(
    clippy::too_many_arguments,
    reason = "Archive rollup rebuild keeps pool, archive, scope, deduplication, clearing, target, replacement, and persistence state explicit."
)]
pub(crate) async fn rebuild_invocation_summary_rollups_from_archive_batch(
    pool: &Pool<Sqlite>,
    archive_row: &ArchiveBatchPathRow,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    cleared_rollup_buckets: &mut ClearedSummaryRollupBuckets,
    targets: &[&str],
    replace_existing_rollups: bool,
    persist_seen_ids: bool,
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }

    let archive_path = PathBuf::from(&archive_row.file_path);
    if !archive_path.exists() {
        bail!("{}", MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX);
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
    let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .context("failed to open archive batch")?;

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
        let mut tx = pool.begin().await?;
        if persist_seen_ids {
            persist_invocation_summary_repair_seen_ids(tx.as_mut(), &rows).await?;
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
                delete_invocation_summary_rollup_bucket_tx(
                    tx.as_mut(),
                    key.0,
                    &key.1,
                    &targets_to_clear,
                )
                .await?;
            }
        }
        upsert_invocation_hourly_rollups_tx(tx.as_mut(), &rows, targets).await?;
        tx.commit().await?;
    }

    archive_pool.close().await;
    drop(temp_cleanup);

    let mut tx = pool.begin().await?;
    for target in targets {
        mark_hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }
    if persist_seen_ids {
        // Page writes and replay markers are separate transactions. Clear the durable de-dup
        // state only with the final marker transaction so a restart cannot add a page twice.
        clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
    }
    tx.commit().await?;

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

async fn load_live_invocation_summary_rows_for_cleared_buckets_page(
    tx: &mut SqliteConnection,
    cleared_rollup_buckets: &HashSet<(i64, String)>,
    source_scope: InvocationSourceScope,
    after_id: i64,
    end_at_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, i64, bool)> {
    if cleared_rollup_buckets.is_empty() || end_at_id <= 0 {
        return Ok((Vec::new(), after_id, false));
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
        WHERE id >
        "#,
    );
    query
        .push_bind(after_id)
        .push(" AND id <= ")
        .push_bind(end_at_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(min_bucket_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(max_bucket_end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE + 1);

    let mut source_rows = query
        .build_query_as::<InvocationHourlySourceRecord>()
        .fetch_all(&mut *tx)
        .await?;
    let has_more = source_rows.len() > INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize;
    if has_more {
        source_rows.truncate(INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize);
    }
    let next_cursor = source_rows.last().map(|row| row.id).unwrap_or(after_id);
    let rows = source_rows
        .into_iter()
        .filter(|row| {
            summary_rollup_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_start_epoch| {
                    cleared_rollup_buckets.contains(&(bucket_start_epoch, row.source.clone()))
                })
                .unwrap_or(false)
        })
        .collect();
    Ok((rows, next_cursor, has_more))
}

async fn restore_invocation_summary_live_rows_page(
    pool: &Pool<Sqlite>,
    cleared_rollup_buckets: &ClearedSummaryRollupBuckets,
    end_at_id: i64,
    after_id: i64,
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let (mut rows, next_cursor, has_more) =
        load_live_invocation_summary_rows_for_cleared_buckets_page(
            tx.as_mut(),
            &cleared_rollup_buckets.overall,
            InvocationSourceScope::All,
            after_id,
            end_at_id,
        )
        .await?;
    retain_unseen_invocation_summary_repair_rows(tx.as_mut(), &mut rows).await?;
    persist_invocation_summary_repair_seen_ids(tx.as_mut(), &rows).await?;
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &rows, &INVOCATION_SUMMARY_ROLLUP_TARGETS)
        .await?;
    if has_more {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET,
            next_cursor,
        )
        .await?;
    } else {
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET)
            .execute(tx.as_mut())
            .await?;
        clear_invocation_summary_repair_cleared_buckets(tx.as_mut()).await?;
    }
    tx.commit().await?;
    Ok(has_more)
}

pub(crate) async fn rebuild_invocation_summary_rollups_from_live_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    targets: &[&str],
    start_after_id: i64,
    persist_seen_ids: bool,
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
        if persist_seen_ids {
            persist_invocation_summary_repair_seen_ids(tx, &rows).await?;
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }
    Ok(cursor_id)
}

async fn mark_materialized_invocation_summary_archive_replayed_tx(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
) -> Result<bool> {
    let can_repair_markers = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM archive_batches AS batches
            WHERE batches.dataset = ?1
              AND batches.file_path = ?2
              AND batches.status = ?3
              AND batches.historical_rollups_materialized_at IS NOT NULL
              AND batches.sha256 IS NOT NULL
              AND TRIM(batches.sha256) <> ''
              AND EXISTS(
                  SELECT 1
                  FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = ?4
                    AND replay.dataset = batches.dataset
                    AND replay.file_path = batches.file_path
                    AND replay.archive_sha256 = batches.sha256
              )
              AND NOT EXISTS(
                  SELECT 1
                  FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target IN (?4, ?5)
                    AND replay.dataset = batches.dataset
                    AND replay.file_path = batches.file_path
                    AND (
                        replay.archive_sha256 IS NULL
                        OR TRIM(replay.archive_sha256) = ''
                        OR replay.archive_sha256 <> batches.sha256
                    )
              )
        )
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_row.file_path())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    if !can_repair_markers {
        return Ok(false);
    }

    for target in INVOCATION_SUMMARY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            archive_row.file_path(),
        )
        .await?;
    }
    Ok(true)
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
    repair_invocation_summary_rollups_with_mode(pool, false).await?;
    Ok(())
}

async fn invocation_summary_repair_archive_cursor_state(
    pool: &Pool<Sqlite>,
) -> Result<(bool, i64)> {
    let exists = hourly_rollup_progress_exists(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
    )
    .await?;
    let cursor = if exists {
        load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((exists, cursor))
}

async fn invocation_summary_repair_archive_cursor_state_tx(
    tx: &mut SqliteConnection,
) -> Result<(bool, i64)> {
    let exists = hourly_rollup_progress_exists(
        &mut *tx,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
    )
    .await?;
    let cursor = if exists {
        load_hourly_rollup_live_progress_tx(
            tx,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((exists, cursor))
}

async fn invocation_summary_repair_archive_row_cursor_state(
    pool: &Pool<Sqlite>,
) -> Result<(bool, i64)> {
    let exists = hourly_rollup_progress_exists(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET,
    )
    .await?;
    let cursor = if exists {
        load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((exists, cursor))
}

async fn invocation_summary_repair_started(pool: &Pool<Sqlite>) -> Result<bool> {
    hourly_rollup_progress_exists(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET).await
}

async fn summary_rollup_repair_manifest_matches(
    tx: &mut SqliteConnection,
    archive_id: i64,
    archive_row: &ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM archive_batches
            WHERE id = ?1
              AND file_path = ?2
              AND status = 'completed'
              AND sha256 = ?3
        )
        "#,
    )
    .bind(archive_id)
    .bind(archive_row.file_path())
    .bind(expected_sha256)
    .fetch_one(&mut *tx)
    .await?
        != 0)
}

async fn repair_invocation_summary_rollups_with_mode(
    pool: &Pool<Sqlite>,
    force_rebuild: bool,
) -> Result<bool> {
    if force_rebuild {
        return repair_invocation_summary_rollups_force_page(pool).await;
    }
    let (archive_cursor_exists, _) = invocation_summary_repair_archive_cursor_state(pool).await?;
    if archive_cursor_exists {
        return Ok(false);
    }
    let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
        invocation_summary_repair_live_cursor_state(pool).await?;
    let repair_complete =
        repair_marker_done && repair_live_cursor_exists && repair_live_cursor >= shared_live_cursor;
    if !force_rebuild && repair_complete {
        return Ok(true);
    }
    let (
        archive_rows,
        missing_materialized_bucket_epochs,
        missing_materialized_archive_has_unknown_scope,
        shared_live_cursor,
        repair_live_cursor,
    ) = {
        let mut tx = pool.begin().await?;
        let (archive_cursor_exists, _) =
            invocation_summary_repair_archive_cursor_state_tx(tx.as_mut()).await?;
        if archive_cursor_exists {
            tx.rollback().await?;
            return Ok(false);
        }
        let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
            invocation_summary_repair_live_cursor_state_tx(tx.as_mut()).await?;
        let repair_complete = repair_marker_done
            && repair_live_cursor_exists
            && repair_live_cursor >= shared_live_cursor;
        if !force_rebuild && repair_complete {
            tx.rollback().await?;
            return Ok(true);
        }
        if repair_marker_done
            && repair_live_cursor_exists
            && repair_live_cursor < shared_live_cursor
        {
            save_hourly_rollup_live_progress_tx(
                tx.as_mut(),
                INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
                shared_live_cursor,
            )
            .await?;
            tx.commit().await?;
            return Ok(true);
        }
        let archive_rows = load_completed_invocation_archive_paths(tx.as_mut()).await?;
        let mut missing_materialized_bucket_epochs = HashSet::new();
        let mut missing_materialized_archive_has_unknown_scope = false;
        for archive_row in &archive_rows {
            if archive_row.historical_rollups_materialized_at.is_none()
                || PathBuf::from(archive_row.file_path()).exists()
            {
                continue;
            }
            let bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
            if bucket_epochs.is_empty() {
                missing_materialized_archive_has_unknown_scope = true;
            } else {
                missing_materialized_bucket_epochs.extend(bucket_epochs);
            }
        }
        let shared_live_cursor =
            load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        tx.commit().await?;
        (
            archive_rows,
            missing_materialized_bucket_epochs,
            missing_materialized_archive_has_unknown_scope,
            shared_live_cursor,
            repair_live_cursor,
        )
    };
    let (_, _, missing_unmaterialized_archive_exists, _) =
        preflight_invocation_summary_repair_archives(pool).await?;
    if missing_unmaterialized_archive_exists {
        bail!("{}", MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX);
    }
    let preserve_materialized_archives = !missing_materialized_bucket_epochs.is_empty()
        || missing_materialized_archive_has_unknown_scope;
    let live_rebuild_start_id = if preserve_materialized_archives {
        shared_live_cursor.max(repair_live_cursor)
    } else {
        0
    };

    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    {
        let mut tx = pool.begin().await?;
        if preserve_materialized_archives {
            if !missing_materialized_archive_has_unknown_scope {
                cleared_rollup_buckets = clear_invocation_summary_rollups_outside_buckets(
                    tx.as_mut(),
                    &missing_materialized_bucket_epochs,
                )
                .await?;
                clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
            }
        } else {
            sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
                .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET)
                .execute(tx.as_mut())
                .await?;
            clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
            sqlx::query("DELETE FROM invocation_rollup_hourly")
                .execute(tx.as_mut())
                .await?;
            sqlx::query("DELETE FROM invocation_failure_rollup_hourly")
                .execute(tx.as_mut())
                .await?;
        }
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
            .execute(tx.as_mut())
            .await?;
        tx.commit().await?;
    }

    let mut seen_ids = HashSet::new();
    let mut repair_incomplete = false;
    for archive_row in &archive_rows {
        let materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
        let archive_path = PathBuf::from(archive_row.file_path());
        if materialized_archive && !archive_path.exists() {
            let mut tx = pool.begin().await?;
            if !mark_materialized_invocation_summary_archive_replayed_tx(tx.as_mut(), archive_row)
                .await?
            {
                repair_incomplete = true;
            }
            tx.commit().await?;
            continue;
        }
        let archive_bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
        let overlaps_missing_materialized_archive = missing_materialized_archive_has_unknown_scope
            || archive_bucket_epochs
                .iter()
                .any(|bucket| missing_materialized_bucket_epochs.contains(bucket));
        if preserve_materialized_archives && overlaps_missing_materialized_archive {
            // Replacing a shared bucket would erase the contribution of a missing materialized
            // archive. Leave both archives for a later pass with a complete source set.
            repair_incomplete = true;
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            pool,
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            true,
            true,
        )
        .await?;
    }

    let mut restored_cursor = 0_i64;
    loop {
        if cleared_rollup_buckets.overall.is_empty() {
            break;
        }
        let mut tx = pool.begin().await?;
        let (mut restored_live_rows, next_cursor, has_more) =
            load_live_invocation_summary_rows_for_cleared_buckets_page(
                tx.as_mut(),
                &cleared_rollup_buckets.overall,
                InvocationSourceScope::All,
                restored_cursor,
                live_rebuild_start_id,
            )
            .await?;
        restored_live_rows.retain(|row| seen_ids.insert(row.id));
        if !restored_live_rows.is_empty() {
            upsert_invocation_hourly_rollups_tx(
                tx.as_mut(),
                &restored_live_rows,
                &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            )
            .await?;
        }
        tx.commit().await?;
        if !has_more {
            break;
        }
        restored_cursor = next_cursor;
    }

    let mut live_cursor_id = live_rebuild_start_id;
    loop {
        let mut tx = pool.begin().await?;
        let mut live_rows = load_live_invocation_hourly_source_rows_after_id(
            tx.as_mut(),
            live_cursor_id,
            InvocationSourceScope::All,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE.saturating_add(1),
        )
        .await?;
        let live_has_more = live_rows.len() > INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize;
        if live_has_more {
            live_rows.truncate(INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize);
        }
        live_cursor_id = live_rows.last().map(|row| row.id).unwrap_or(live_cursor_id);
        live_rows.retain(|row| seen_ids.insert(row.id));
        if !live_rows.is_empty() {
            upsert_invocation_hourly_rollups_tx(
                tx.as_mut(),
                &live_rows,
                &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            )
            .await?;
        }
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
            live_cursor_id.max(shared_live_cursor),
        )
        .await?;
        tx.commit().await?;
        if !live_has_more {
            break;
        }
    }

    let mut tx = pool.begin().await?;
    if repair_incomplete {
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .execute(tx.as_mut())
            .await?;
    } else {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(!repair_incomplete)
}

async fn preflight_invocation_summary_repair_archives(
    pool: &Pool<Sqlite>,
) -> Result<(HashSet<i64>, bool, bool, bool)> {
    let mut missing_materialized_bucket_epochs = HashSet::new();
    let mut missing_materialized_archive_has_unknown_scope = false;
    let mut missing_unmaterialized_archive_exists = false;
    let mut missing_materialized_archive_exists = false;
    let mut after_archive_id = 0_i64;

    loop {
        let page_rows = load_completed_invocation_summary_repair_page(
            pool,
            after_archive_id,
            SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT,
        )
        .await?;
        if page_rows.is_empty() {
            break;
        }
        for (_, expected_sha256, archive_row) in &page_rows {
            let archive_path = PathBuf::from(archive_row.file_path());
            if !archive_path.exists() {
                if archive_row.historical_rollups_materialized_at.is_none() {
                    missing_unmaterialized_archive_exists = true;
                    continue;
                }
                missing_materialized_archive_exists = true;
                let bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
                if bucket_epochs.is_empty() {
                    missing_materialized_archive_has_unknown_scope = true;
                } else {
                    missing_materialized_bucket_epochs.extend(bucket_epochs);
                }
                continue;
            }
            // Validate every bounded source page before any destructive rebuild. The archive
            // reader keeps only the current page in memory and never retains the full source.
            validate_invocation_summary_repair_archive(archive_row, expected_sha256).await?;
        }
        after_archive_id = page_rows
            .last()
            .map(|(archive_id, _, _)| *archive_id)
            .unwrap_or(after_archive_id);
        if page_rows.len() <= SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT {
            break;
        }
    }

    Ok((
        missing_materialized_bucket_epochs,
        missing_materialized_archive_has_unknown_scope,
        missing_unmaterialized_archive_exists,
        missing_materialized_archive_exists,
    ))
}

async fn repair_invocation_summary_rollups_force_page(pool: &Pool<Sqlite>) -> Result<bool> {
    match repair_invocation_summary_rollups_force_page_inner(pool).await {
        Ok(result) => Ok(result),
        Err(err) => {
            if invocation_summary_repair_backup_exists(pool)
                .await
                .unwrap_or_default()
                && let Err(restore_err) = restore_invocation_summary_repair_backup(pool).await
            {
                return Err(err.context(format!(
                    "summary rollup repair failed and restoring the last-good rollups also failed: {restore_err}"
                )));
            }
            Err(err)
        }
    }
}

async fn repair_invocation_summary_rollups_force_page_inner(pool: &Pool<Sqlite>) -> Result<bool> {
    let (
        missing_materialized_bucket_epochs,
        missing_materialized_archive_has_unknown_scope,
        missing_unmaterialized_archive_exists,
        missing_materialized_archive_exists,
    ) = preflight_invocation_summary_repair_archives(pool).await?;
    if missing_unmaterialized_archive_exists {
        // Preserve the last-good rollups until every unmaterialized source is readable. A force
        // pass must never clear them and then discover a missing source during archive I/O.
        if invocation_summary_repair_backup_exists(pool).await? {
            restore_invocation_summary_repair_backup(pool).await?;
        }
        let mut tx = pool.begin().await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(false);
    }
    let (archive_cursor_exists, mut archive_cursor) =
        invocation_summary_repair_archive_cursor_state(pool).await?;
    let (mut archive_row_cursor_exists, mut archive_row_cursor) =
        invocation_summary_repair_archive_row_cursor_state(pool).await?;
    let incomplete_exists =
        hourly_rollup_progress_exists(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
            .await?;
    let repair_marker_mismatch = summary_rollup_backfill_requires_full_repair(pool).await?;
    let started = invocation_summary_repair_started(pool).await?;
    let restart = archive_cursor_exists
        && !missing_materialized_archive_exists
        && (incomplete_exists || repair_marker_mismatch);
    if restart {
        archive_cursor = 0;
        archive_row_cursor_exists = false;
        archive_row_cursor = 0;
    }
    let first_page = !started || restart;
    if first_page && invocation_summary_repair_backup_exists(pool).await? {
        restore_invocation_summary_repair_backup(pool).await?;
        return Box::pin(repair_invocation_summary_rollups_force_page_inner(pool)).await;
    }
    let preserve_materialized_archives = !missing_materialized_bucket_epochs.is_empty()
        || missing_materialized_archive_has_unknown_scope;
    let shared_live_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let live_start_id = if preserve_materialized_archives {
        shared_live_cursor.max(repair_live_cursor)
    } else {
        repair_live_cursor
    };
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    if !first_page {
        let mut tx = pool.begin().await?;
        cleared_rollup_buckets =
            load_invocation_summary_repair_cleared_buckets(tx.as_mut()).await?;
        tx.rollback().await?;
    }
    if first_page {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET)
            .execute(tx.as_mut())
            .await?;
        clear_invocation_summary_repair_cleared_buckets(tx.as_mut()).await?;
        create_invocation_summary_repair_backup_tx(tx.as_mut()).await?;
        if preserve_materialized_archives {
            if !missing_materialized_archive_has_unknown_scope {
                cleared_rollup_buckets = clear_invocation_summary_rollups_outside_buckets(
                    tx.as_mut(),
                    &missing_materialized_bucket_epochs,
                )
                .await?;
                persist_invocation_summary_repair_cleared_buckets(
                    tx.as_mut(),
                    &cleared_rollup_buckets,
                )
                .await?;
                clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
            }
        } else {
            sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
                .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET)
                .execute(tx.as_mut())
                .await?;
            clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
            sqlx::query("DELETE FROM invocation_rollup_hourly")
                .execute(tx.as_mut())
                .await?;
            sqlx::query("DELETE FROM invocation_failure_rollup_hourly")
                .execute(tx.as_mut())
                .await?;
        }
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        if !cleared_rollup_buckets.overall.is_empty() {
            save_hourly_rollup_live_progress_tx(
                tx.as_mut(),
                INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET,
                live_start_id,
            )
            .await?;
        }
        tx.commit().await?;
        archive_cursor = 0;
        archive_row_cursor_exists = false;
        archive_row_cursor = 0;
    }

    if !cleared_rollup_buckets.overall.is_empty() {
        let restored_live_end = load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET,
        )
        .await?
        .max(live_start_id);
        let restored_live_cursor = load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET,
        )
        .await?;
        if restore_invocation_summary_live_rows_page(
            pool,
            &cleared_rollup_buckets,
            restored_live_end,
            restored_live_cursor,
        )
        .await?
        {
            return Ok(false);
        }
    }

    let mut repair_incomplete = if first_page { false } else { incomplete_exists };
    loop {
        let page_rows = load_completed_invocation_summary_repair_page(
            pool,
            archive_cursor,
            SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT,
        )
        .await?;

        let skippable_prefix_len = page_rows
            .iter()
            .take(SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT)
            .take_while(|(_, _, archive_row)| {
                let materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
                let archive_path = PathBuf::from(archive_row.file_path());
                let archive_bucket_epochs =
                    archive_bucket_start_epochs_for_row(archive_row).unwrap_or_default();
                (materialized_archive && !archive_path.exists())
                    || (preserve_materialized_archives
                        && (missing_materialized_archive_has_unknown_scope
                            || archive_bucket_epochs
                                .iter()
                                .any(|bucket| missing_materialized_bucket_epochs.contains(bucket))))
            })
            .count();
        if skippable_prefix_len > 0 {
            let mut tx = pool.begin().await?;
            let mut skipped_archive_is_incomplete = false;
            for (_, _, archive_row) in page_rows.iter().take(skippable_prefix_len) {
                let materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
                let archive_path = PathBuf::from(archive_row.file_path());
                if materialized_archive
                    && !archive_path.exists()
                    && !mark_materialized_invocation_summary_archive_replayed_tx(
                        tx.as_mut(),
                        archive_row,
                    )
                    .await?
                {
                    skipped_archive_is_incomplete = true;
                }
                if preserve_materialized_archives {
                    skipped_archive_is_incomplete = true;
                }
            }
            if skipped_archive_is_incomplete {
                repair_incomplete = true;
                save_hourly_rollup_live_progress_tx(
                    tx.as_mut(),
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET,
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
                )
                .await?;
            }
            let last_archive_id = page_rows
                .get(skippable_prefix_len - 1)
                .map(|(archive_id, _, _)| *archive_id)
                .expect("skippable archive prefix is non-empty");
            save_hourly_rollup_live_progress_tx(
                tx.as_mut(),
                INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
                last_archive_id,
            )
            .await?;
            sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
                .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
                .execute(tx.as_mut())
                .await?;
            tx.commit().await?;
            archive_cursor = last_archive_id;
            archive_row_cursor_exists = false;
            archive_row_cursor = 0;
            if page_rows.len() > SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT {
                return Ok(false);
            }
            continue;
        }

        if let Some((archive_id, expected_sha256, archive_row)) = page_rows.first() {
            let materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
            let archive_path = PathBuf::from(archive_row.file_path());
            let archive_bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
            let overlaps_missing_materialized_archive =
                missing_materialized_archive_has_unknown_scope
                    || archive_bucket_epochs
                        .iter()
                        .any(|bucket| missing_materialized_bucket_epochs.contains(bucket));
            if materialized_archive && !archive_path.exists()
                || preserve_materialized_archives && overlaps_missing_materialized_archive
            {
                let mut tx = pool.begin().await?;
                let mut skipped_archive_is_incomplete = false;
                if materialized_archive
                    && !archive_path.exists()
                    && !mark_materialized_invocation_summary_archive_replayed_tx(
                        tx.as_mut(),
                        archive_row,
                    )
                    .await?
                {
                    skipped_archive_is_incomplete = true;
                }
                if preserve_materialized_archives && overlaps_missing_materialized_archive {
                    skipped_archive_is_incomplete = true;
                }
                if skipped_archive_is_incomplete {
                    repair_incomplete = true;
                    save_hourly_rollup_live_progress_tx(
                        tx.as_mut(),
                        INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET,
                        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
                    )
                    .await?;
                }
                save_hourly_rollup_live_progress_tx(
                    tx.as_mut(),
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
                    *archive_id,
                )
                .await?;
                sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
                    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
                    .execute(tx.as_mut())
                    .await?;
                tx.commit().await?;
                archive_cursor = *archive_id;
                archive_row_cursor_exists = false;
                archive_row_cursor = 0;
                continue;
            }

            let (rows, next_row_cursor, archive_finished) =
                load_invocation_summary_repair_archive_chunk(
                    archive_row,
                    expected_sha256,
                    if archive_row_cursor_exists {
                        archive_row_cursor
                    } else {
                        0
                    },
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE,
                )
                .await?;
            let mut tx = pool.begin().await?;
            if !summary_rollup_repair_manifest_matches(
                tx.as_mut(),
                *archive_id,
                archive_row,
                expected_sha256,
            )
            .await?
            {
                tx.rollback().await?;
                bail!(
                    "summary rollup repair archive manifest changed before commit (archive_ref={})",
                    archive_sha_log_ref(expected_sha256)
                );
            }
            let mut rows = rows;
            retain_unseen_invocation_summary_repair_rows(tx.as_mut(), &mut rows).await?;
            persist_invocation_summary_repair_seen_ids(tx.as_mut(), &rows).await?;
            upsert_invocation_hourly_rollups_tx(
                tx.as_mut(),
                &rows,
                &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            )
            .await?;
            if archive_finished {
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    HOURLY_ROLLUP_TARGET_INVOCATIONS,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    archive_row.file_path(),
                )
                .await?;
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    archive_row.file_path(),
                )
                .await?;
                save_hourly_rollup_live_progress_tx(
                    tx.as_mut(),
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET,
                    *archive_id,
                )
                .await?;
                sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
                    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
                    .execute(tx.as_mut())
                    .await?;
            } else {
                save_hourly_rollup_live_progress_tx(
                    tx.as_mut(),
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET,
                    next_row_cursor,
                )
                .await?;
            }
            tx.commit().await?;
            if archive_finished {
                archive_cursor = *archive_id;
                archive_row_cursor_exists = false;
                archive_row_cursor = 0;
                continue;
            }
            return Ok(false);
        }
        break;
    }

    let mut live_rows = load_live_invocation_hourly_source_rows_after_id(
        pool,
        live_start_id,
        InvocationSourceScope::All,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE.saturating_add(1),
    )
    .await?;
    let live_has_more = live_rows.len() > INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize;
    if live_has_more {
        live_rows.truncate(INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE as usize);
    }
    let live_cursor_id = live_rows.last().map(|row| row.id).unwrap_or(live_start_id);
    let mut tx = pool.begin().await?;
    retain_unseen_invocation_summary_repair_rows(tx.as_mut(), &mut live_rows).await?;
    persist_invocation_summary_repair_seen_ids(tx.as_mut(), &live_rows).await?;
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &live_rows,
        &INVOCATION_SUMMARY_ROLLUP_TARGETS,
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        live_cursor_id.max(shared_live_cursor),
    )
    .await?;
    if live_has_more {
        tx.commit().await?;
        return Ok(false);
    }
    if repair_incomplete {
        sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2, ?3, ?4)")
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET)
            .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .execute(tx.as_mut())
            .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2, ?3, ?4)")
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET)
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
        .execute(tx.as_mut())
        .await?;
    clear_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
    clear_invocation_summary_repair_cleared_buckets(tx.as_mut()).await?;
    sqlx::query(&format!(
        "DROP TABLE IF EXISTS {}",
        INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_OVERALL_TABLE
    ))
    .execute(tx.as_mut())
    .await?;
    sqlx::query(&format!(
        "DROP TABLE IF EXISTS {}",
        INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_FAILURES_TABLE
    ))
    .execute(tx.as_mut())
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}
async fn summary_rollup_backfill_requires_full_repair(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(SELECT 1 FROM archive_batches AS batches WHERE batches.dataset = 'codex_invocations' AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.sha256 IS NOT NULL AND TRIM(batches.sha256) <> '' AND (EXISTS(SELECT 1 FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target IN (?2, ?3) AND replay.dataset = batches.dataset AND replay.file_path = batches.file_path
              AND TRIM(replay.archive_sha256) <> '' AND replay.archive_sha256 IS NOT batches.sha256)
            OR (batches.historical_rollups_materialized_at IS NOT NULL AND (SELECT COUNT(*) FROM hourly_rollup_archive_replay AS replay
              WHERE replay.target IN (?2, ?3) AND replay.dataset = batches.dataset AND replay.file_path = batches.file_path
                AND replay.archive_sha256 = batches.sha256) < 2)))
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(executor)
    .await?
        != 0)
}
pub(crate) async fn backfill_missing_invocation_summary_archive_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let (archive_cursor_exists, _) = invocation_summary_repair_archive_cursor_state(pool).await?;
    let incomplete_exists =
        hourly_rollup_progress_exists(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
            .await?;
    let force_repair_required = archive_cursor_exists
        || incomplete_exists
        || summary_rollup_backfill_requires_full_repair(pool).await?;
    if force_repair_required && !repair_invocation_summary_rollups_with_mode(pool, true).await? {
        return Ok(());
    }
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(pool).await?;
    if archive_rows.is_empty() {
        return Ok(());
    }
    let mut missing_materialized_bucket_epochs = HashSet::new();
    let mut missing_materialized_archive_has_unknown_scope = false;
    for archive_row in &archive_rows {
        if archive_row.historical_rollups_materialized_at.is_none()
            || PathBuf::from(archive_row.file_path()).exists()
        {
            continue;
        }
        let bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
        if bucket_epochs.is_empty() {
            missing_materialized_archive_has_unknown_scope = true;
        } else {
            missing_materialized_bucket_epochs.extend(bucket_epochs);
        }
    }
    // This query is an archive-count budget, not a complete rollup bucket boundary. Keep the
    // repair additive so a later bounded pass cannot clear contributions from archives replayed
    // by an earlier pass in the same hour/source bucket.
    let mut seen_ids = {
        let mut tx = pool.begin().await?;
        let seen_ids = load_invocation_summary_repair_seen_ids(tx.as_mut()).await?;
        tx.rollback().await?;
        seen_ids
    };
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
        let archive_bucket_epochs = archive_bucket_start_epochs_for_row(archive_row)?;
        let overlaps_missing_materialized_archive = missing_materialized_archive_has_unknown_scope
            || archive_bucket_epochs
                .iter()
                .any(|bucket| missing_materialized_bucket_epochs.contains(bucket));
        if archive_row.historical_rollups_materialized_at.is_some()
            || overlaps_missing_materialized_archive
        {
            // A materialized archive needs the bounded replacement path above to establish both
            // current Summary target markers. Also defer any peer sharing a bucket with a missing
            // materialized archive so additive replay cannot duplicate the preserved aggregate.
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            pool,
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &targets,
            false,
            true,
        )
        .await?;
    }
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
