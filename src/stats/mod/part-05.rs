pub(crate) async fn query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
    pool: &Pool<Sqlite>,
    rollup_target: &str,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
) -> Result<Vec<UpstreamAccountStatsRollupRecord>> {
    let archive_rows = load_invocation_archives_missing_rollup_target_bounded(
        pool,
        rollup_target,
        range,
        SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES,
    )
    .await?;
    if archive_rows.len() > SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES {
        return Err(anyhow!(
            "summary account archive batch cardinality exceeded bounded budget ({SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES})"
        ));
    }
    let archive_rows =
        if account_archive_target_treats_materialized_batch_as_replayed(rollup_target) {
            archive_rows
                .into_iter()
                .filter(|archive_row| archive_row.historical_rollups_materialized_at.is_none())
                .collect::<Vec<_>>()
        } else {
            archive_rows
        };
    let mut archive_deltas = BTreeMap::<i64, UpstreamAccountStatsDelta>::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "account-stats").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
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
            for row in rows {
                if exclude_invocation_ids.is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
                {
                    continue;
                }
                if !invocation_hourly_source_record_matches_range(&row, range) {
                    continue;
                }
                if row.resolved_upstream_account_id() != Some(upstream_account_id) {
                    continue;
                }
                let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
                let entry = archive_deltas.entry(bucket_start_epoch).or_default();
                add_account_invocation_row_to_stats_delta(entry, &row);
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(archive_deltas
        .into_iter()
        .filter_map(|(bucket_start_epoch, delta)| {
            (delta.total_count > 0).then_some(UpstreamAccountStatsRollupRecord {
                bucket_start_epoch,
                total_count: delta.total_count,
                success_count: delta.success_count,
                failure_count: delta.failure_count,
                in_flight_count: delta.in_flight_count,
                total_tokens: delta.total_tokens,
                input_tokens: delta.input_tokens,
                output_tokens: delta.output_tokens,
                cache_input_tokens: delta.cache_input_tokens,
                reasoning_tokens: delta.reasoning_tokens,
                total_cost: delta.total_cost,
                non_success_cost: delta.non_success_cost,
                total_latency_sample_count: delta.total_latency_sample_count,
                total_latency_sum_ms: delta.total_latency_sum_ms,
                first_byte_sample_count: delta.first_byte_sample_count,
                first_byte_sum_ms: delta.first_byte_sum_ms,
                first_byte_max_ms: delta.first_byte_max_ms,
                first_byte_histogram: encode_approx_histogram(&delta.first_byte_histogram).ok()?,
                first_response_byte_total_sample_count: delta
                    .first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms: delta.first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms: delta.first_response_byte_total_max_ms,
                first_response_byte_total_histogram: encode_approx_histogram(
                    &delta.first_response_byte_total_histogram,
                )
                .ok()?,
                first_token_sample_count: delta.first_token_sample_count,
                first_token_sum_ms: delta.first_token_sum_ms,
                first_token_max_ms: delta.first_token_max_ms,
                first_token_histogram: encode_approx_histogram(&delta.first_token_histogram)
                    .ok()?,
            })
        })
        .collect())
}

pub(crate) async fn query_unmaterialized_upstream_account_archive_totals(
    pool: &Pool<Sqlite>,
    rollup_target: &str,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
) -> Result<StatsTotals> {
    let mut totals = StatsTotals::default();
    for row in query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
        pool,
        rollup_target,
        source_scope,
        range,
        exclude_invocation_ids,
        upstream_account_id,
    )
    .await?
    {
        totals.total_count += row.total_count;
        totals.success_count += row.success_count;
        totals.failure_count += row.failure_count;
        totals.total_tokens += row.total_tokens;
        totals.total_cost += row.total_cost;
        totals.non_success_cost += row.non_success_cost;
    }

    Ok(totals)
}

/// Aggregate unmaterialized account archive rows in one archive pass.  The summary projection
/// uses this for its all-time account snapshots so account cardinality does not multiply archive
/// opens and decompression work.
const SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS: usize = 50_000;
const SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES: usize = 4_096;

pub(crate) async fn query_unmaterialized_upstream_account_archive_totals_by_account(
    pool: &Pool<Sqlite>,
    rollup_target: &str,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<HashMap<i64, StatsTotals>> {
    let archive_rows = load_invocation_archives_missing_rollup_target_bounded(
        pool,
        rollup_target,
        range,
        SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES,
    )
    .await?;
    // Missing account replay for a materialized archive is repaired by SummaryProjection's
    // bounded exact bucket replacement. Adding the whole archive here on top of its compact
    // account rollup can double count a partially replayed prefix, so this aggregate is reserved
    // for archives with no materialized historical baseline at all.
    let archive_rows = archive_rows
        .into_iter()
        .filter(|archive_row| archive_row.historical_rollups_materialized_at.is_none())
        .collect::<Vec<_>>();
    if archive_rows.len() > SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES {
        return Err(anyhow!(
            "summary account archive batch cardinality exceeded bounded budget ({SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES})"
        ));
    }
    let archive_paths = archive_rows
        .iter()
        .map(|row| row.file_path.clone())
        .collect::<Vec<_>>();
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }
    let mut row_count_query = QueryBuilder::<Sqlite>::new(
        "SELECT file_path, row_count FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' AND file_path IN (",
    );
    {
        let mut separated = row_count_query.separated(", ");
        for path in &archive_paths {
            separated.push_bind(path);
        }
    }
    row_count_query.push(")");
    let row_counts = row_count_query
        .build_query_as::<(String, i64)>()
        .fetch_all(pool)
        .await
        .context("summary account archive row-count hydration failed")?
        .into_iter()
        .collect::<HashMap<_, _>>();
    if row_counts
        .values()
        .any(|row_count| *row_count > SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS as i64)
    {
        return Err(anyhow!(
            "summary account archive exact rows exceeded bounded budget ({SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS})"
        ));
    }
    let mut totals_by_account = HashMap::<i64, StatsTotals>::new();
    let mut scanned_rows = 0usize;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "summary-account-stats").await?
        else {
            return Err(anyhow!(
                "summary account archive is unavailable: {}",
                archive_row.file_path
            ));
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                source_scope,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            scanned_rows = scanned_rows.saturating_add(rows.len());
            if scanned_rows > SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS {
                return Err(anyhow!(
                    "summary account archive exact rows exceeded bounded budget ({SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS})"
                ));
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            for row in rows {
                if exclude_invocation_ids.is_some_and(|ids| ids.contains(&row.id))
                    || !invocation_hourly_source_record_matches_range(&row, range)
                {
                    continue;
                }
                let Some(account_id) = row.resolved_upstream_account_id().filter(|id| *id > 0)
                else {
                    continue;
                };
                let classification = resolve_failure_classification(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                );
                let entry = totals_by_account.entry(account_id).or_default();
                entry.total_count += 1;
                if crate::api::prompt_invocation_status_is_success_like(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                ) && classification.failure_class == FailureClass::None
                {
                    entry.success_count += 1;
                } else if crate::api::prompt_invocation_status_counts_toward_terminal_totals(
                    row.status.as_deref(),
                ) && classification.failure_class != FailureClass::None
                {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
                if invocation_counts_toward_non_success_usage(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                ) {
                    entry.non_success_cost += row.cost.unwrap_or_default();
                }
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(totals_by_account)
}

pub(crate) async fn query_unmaterialized_upstream_account_archive_non_success_usage(
    pool: &Pool<Sqlite>,
    rollup_target: &str,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
) -> Result<(f64, i64)> {
    let archive_rows =
        load_invocation_archives_missing_effective_rollup_target(pool, rollup_target, range)
            .await?;
    let mut total_cost = 0.0_f64;
    let mut total_tokens = 0_i64;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "account-stats").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
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
            for row in rows {
                if exclude_invocation_ids.is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
                {
                    continue;
                }
                if !invocation_hourly_source_record_matches_range(&row, range) {
                    continue;
                }
                if row.resolved_upstream_account_id() != Some(upstream_account_id) {
                    continue;
                }
                if !invocation_row_counts_toward_non_success_usage(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                ) {
                    continue;
                }
                total_cost += row.cost.unwrap_or_default();
                total_tokens += row.total_tokens.unwrap_or_default();
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((total_cost, total_tokens))
}

pub(crate) async fn query_completed_upstream_account_archive_non_success_usage(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
) -> Result<(f64, i64)> {
    let archive_rows = load_completed_invocation_archive_paths_in_range(pool, range).await?;
    let mut total_cost = 0.0_f64;
    let mut total_tokens = 0_i64;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "account-stats").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
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
            for row in rows {
                if exclude_invocation_ids.is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
                {
                    continue;
                }
                if !invocation_hourly_source_record_matches_range(&row, range) {
                    continue;
                }
                if row.resolved_upstream_account_id() != Some(upstream_account_id) {
                    continue;
                }
                if !invocation_row_counts_toward_non_success_usage(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                ) {
                    continue;
                }
                total_cost += row.cost.unwrap_or_default();
                total_tokens += row.total_tokens.unwrap_or_default();
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((total_cost, total_tokens))
}

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
