pub(crate) async fn load_pending_invocation_archive_hourly_rollup_deltas(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: Option<usize>,
) -> Result<PendingInvocationArchiveOverallState> {
    let archive_rows = match max_rows {
        Some(max_rows) => {
            load_invocation_archives_missing_rollup_target_bounded(
                pool,
                HOURLY_ROLLUP_TARGET_INVOCATIONS,
                range,
                SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES.min(max_rows.max(1)),
            )
            .await?
        }
        None => {
            load_invocation_archives_missing_rollup_target(
                pool,
                HOURLY_ROLLUP_TARGET_INVOCATIONS,
                range,
            )
            .await?
        }
    };
    if max_rows.is_some() && archive_rows.len() > SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES {
        return Err(anyhow!(
            "summary archive batch budget exceeded ({})",
            SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES
        ));
    }
    if let Some(max_rows) = max_rows
        && range.is_none()
        && !archive_rows.is_empty()
    {
        let mut row_count_query = QueryBuilder::<Sqlite>::new(
            "SELECT COALESCE(SUM(row_count), 0) FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed' AND file_path IN (",
        );
        {
            let mut separated = row_count_query.separated(", ");
            for archive_row in &archive_rows {
                separated.push_bind(archive_row.file_path());
            }
        }
        row_count_query.push(")");
        let row_count = row_count_query
            .build_query_scalar::<i64>()
            .fetch_one(pool)
            .await
            .context("summary archive row-count budget hydration failed")?;
        if row_count > max_rows as i64 {
            return Err(anyhow!(
                "summary archive row budget exceeded ({row_count} > {max_rows})"
            ));
        }
    }
    let mut pending_state = PendingInvocationArchiveOverallState::default();
    let mut scanned_rows = 0usize;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
        else {
            if archive_row.historical_rollups_materialized_at.is_some() {
                pending_state
                    .unreadable_materialized_bucket_start_epochs
                    .extend(archive_bucket_start_epochs_for_row(&archive_row)?);
            } else {
                // Historical read paths preserve their established best-effort behavior. The
                // bounded SummaryProjection variant inspects this state and fails closed before
                // publishing an inexact memory snapshot.
                pending_state
                    .unreadable_unmaterialized_paths
                    .push(archive_row.file_path.clone());
            }
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
            if let Some(max_rows) = max_rows {
                scanned_rows = scanned_rows.saturating_add(rows.len());
                if scanned_rows > max_rows {
                    return Err(anyhow!(
                        "summary archive exact row budget exceeded ({scanned_rows} > {max_rows})"
                    ));
                }
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            let filtered_rows = rows
                .into_iter()
                .filter(|row| {
                    !exclude_invocation_ids
                        .is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
                })
                .filter(|row| invocation_hourly_source_record_matches_range(row, range))
                .collect::<Vec<_>>();
            if filtered_rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(
                if archive_row.historical_rollups_materialized_at.is_some() {
                    &mut pending_state.materialized
                } else {
                    &mut pending_state.unmaterialized
                },
                &filtered_rows,
            )?;
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(pending_state)
}

pub(crate) fn invocation_hourly_source_record_matches_range(
    row: &InvocationHourlySourceRecord,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> bool {
    let Some((start, end)) = range else {
        return true;
    };
    let Some(occurred_at_utc) = parse_to_utc_datetime(&row.occurred_at) else {
        return false;
    };
    occurred_at_utc >= start && occurred_at_utc < end
}

pub(crate) fn archived_failure_rollup_key(
    row: &ArchivedInvocationFailureRow,
) -> Result<Option<(i64, String, String, i64, String)>> {
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    if classification.failure_class == FailureClass::None {
        return Ok(None);
    }
    Ok(Some((
        summary_rollup_bucket_start_epoch(&row.occurred_at)?,
        row.source.clone(),
        classification.failure_class.as_str().to_string(),
        classification.is_actionable as i64,
        categorize_error(row.error_message.as_deref().unwrap_or_default()),
    )))
}

pub(crate) async fn load_missing_failure_rollup_row_counts_for_rows(
    pool: &Pool<Sqlite>,
    rows: &[ArchivedInvocationFailureRow],
    materialized_bucket_sources: &HashSet<(i64, String)>,
    completed_archive_row_counts: &HashMap<(i64, String, String, i64, String), usize>,
    unreadable_materialized_bucket_start_epochs: &HashSet<i64>,
) -> Result<HashMap<(i64, String, String, i64, String), usize>> {
    let mut grouped_counts = HashMap::<(i64, String, String, i64, String), usize>::new();
    for row in rows {
        let Some(key) = archived_failure_rollup_key(row)? else {
            continue;
        };
        *grouped_counts.entry(key).or_default() += 1;
    }
    if grouped_counts.is_empty() {
        return Ok(HashMap::new());
    }

    let mut missing_counts = HashMap::new();
    for (
        (bucket_start_epoch, source, failure_class, is_actionable, error_category),
        archive_count,
    ) in grouped_counts
    {
        let bucket_source = (bucket_start_epoch, source.clone());
        let has_unreadable_materialized_archive =
            unreadable_materialized_bucket_start_epochs.contains(&bucket_start_epoch);
        let materialized_count = sqlx::query_scalar::<_, i64>(
            "SELECT failure_count FROM invocation_failure_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2 AND failure_class = ?3 AND is_actionable = ?4 AND error_category = ?5 LIMIT 1",
        )
        .bind(bucket_start_epoch)
        .bind(&source)
        .bind(&failure_class)
        .bind(is_actionable)
        .bind(&error_category)
        .fetch_optional(pool)
        .await?
        .unwrap_or_default()
        .max(0) as usize;
        let pending_batch_overlap = if has_unreadable_materialized_archive {
            0
        } else if materialized_bucket_sources.contains(&bucket_source) {
            materialized_count.saturating_sub(
                completed_archive_row_counts
                    .get(&(
                        bucket_start_epoch,
                        source.clone(),
                        failure_class.clone(),
                        is_actionable,
                        error_category.clone(),
                    ))
                    .copied()
                    .unwrap_or_default(),
            )
        } else {
            materialized_count
        };
        let missing_count = archive_count.saturating_sub(pending_batch_overlap);
        if missing_count > 0 {
            missing_counts.insert(
                (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                ),
                missing_count,
            );
        }
    }

    Ok(missing_counts)
}

pub(crate) fn add_invocation_hourly_source_record_to_proxy_perf_rollups(
    perf: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    if row.source != SOURCE_PROXY {
        return Ok(());
    }
    let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
    for (stage, value_ms) in [
        ("total", row.t_total_ms),
        ("requestRead", row.t_req_read_ms),
        ("requestParse", row.t_req_parse_ms),
        ("upstreamConnect", row.t_upstream_connect_ms),
        ("upstreamFirstByte", row.t_upstream_ttfb_ms),
        ("upstreamStream", row.t_upstream_stream_ms),
        ("responseParse", row.t_resp_parse_ms),
        ("persistence", row.t_persist_ms),
    ] {
        record_proxy_perf_stage_sample(perf, bucket_start_epoch, stage, value_ms);
    }
    Ok(())
}

pub(crate) async fn load_materialized_proxy_perf_rollups_for_range(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<BTreeMap<(i64, String), ProxyPerfStageHourlyRollupRecord>> {
    Ok(
        query_proxy_perf_stage_hourly_rollup_range(pool, start.timestamp(), end.timestamp())
            .await?
            .into_iter()
            .map(|row| ((row.bucket_start_epoch, row.stage.clone()), row))
            .collect(),
    )
}

pub(crate) fn build_proxy_perf_stage_rollup_delta(
    archive_delta: &ProxyPerfStageHourlyDelta,
    materialized_row: Option<&ProxyPerfStageHourlyRollupRecord>,
) -> Result<Option<ProxyPerfStageHourlyDelta>> {
    let histogram = subtract_approx_histogram_counts(
        &archive_delta.histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let sample_count = histogram.iter().copied().sum::<i64>();
    let sum_ms = subtract_nonnegative_f64(
        archive_delta.sum_ms,
        materialized_row.map(|row| row.sum_ms).unwrap_or(0.0),
    );

    if sample_count <= 0 && sum_ms <= 0.0 {
        return Ok(None);
    }

    Ok(Some(ProxyPerfStageHourlyDelta {
        sample_count,
        sum_ms,
        max_ms: if sample_count > 0 {
            approx_histogram_percentile_ms(&histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        histogram,
    }))
}

pub(crate) fn build_materialized_pending_proxy_perf_overlap(
    materialized_row: Option<&ProxyPerfStageHourlyRollupRecord>,
    completed_archive_delta: Option<&ProxyPerfStageHourlyDelta>,
) -> Option<ProxyPerfStageHourlyRollupRecord> {
    let materialized_row = materialized_row?;
    let materialized_histogram = decode_approx_histogram(&materialized_row.histogram);
    let empty_histogram = empty_approx_histogram();
    let histogram = subtract_approx_histogram_counts(
        &materialized_histogram,
        completed_archive_delta
            .map(|delta| delta.histogram.as_slice())
            .unwrap_or(empty_histogram.as_slice()),
    );
    let sample_count = histogram.iter().copied().sum::<i64>();
    let sum_ms = subtract_nonnegative_f64(
        materialized_row.sum_ms,
        completed_archive_delta
            .map(|delta| delta.sum_ms)
            .unwrap_or(0.0),
    );
    if sample_count <= 0 && sum_ms <= 0.0 {
        return None;
    }

    Some(ProxyPerfStageHourlyRollupRecord {
        bucket_start_epoch: materialized_row.bucket_start_epoch,
        stage: materialized_row.stage.clone(),
        sample_count,
        sum_ms,
        max_ms: if sample_count > 0 {
            approx_histogram_percentile_ms(&histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        histogram: encode_approx_histogram(&histogram).unwrap_or_default(),
    })
}

pub(crate) async fn load_proxy_perf_stage_rollups_by_materialization_state(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<PendingProxyPerfArchiveState> {
    let archive_rows = load_invocation_archives_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        Some((start, end)),
    )
    .await?;
    let mut pending_state = PendingProxyPerfArchiveState::default();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "proxy-perf").await?
        else {
            if archive_row.historical_rollups_materialized_at.is_some() {
                pending_state
                    .unreadable_materialized_bucket_start_epochs
                    .extend(archive_bucket_start_epochs_for_row(&archive_row)?);
            }
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                InvocationSourceScope::ProxyOnly,
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
                if !invocation_hourly_source_record_matches_range(&row, Some((start, end))) {
                    continue;
                }
                add_invocation_hourly_source_record_to_proxy_perf_rollups(
                    if archive_row.historical_rollups_materialized_at.is_some() {
                        &mut pending_state.materialized
                    } else {
                        &mut pending_state.unmaterialized
                    },
                    &row,
                )?;
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(pending_state)
}

pub(crate) async fn load_materialized_proxy_perf_stage_rollups_for_buckets(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<(
    BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    HashSet<i64>,
)> {
    if bucket_start_epochs.is_empty() {
        return Ok((BTreeMap::new(), HashSet::new()));
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(bucket_start_epochs.iter().copied());
    let archive_rows = load_replayed_invocation_archives_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        &month_keys,
    )
    .await?;
    let mut materialized = BTreeMap::<(i64, String), ProxyPerfStageHourlyDelta>::new();
    let mut unreadable_bucket_start_epochs = HashSet::new();

    for archive in archive_rows {
        let archive_row = materialized_archive_path_row(
            archive.file_path.clone(),
            archive.coverage_start_at.clone(),
            archive.coverage_end_at.clone(),
        );
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "proxy-perf").await?
        else {
            unreadable_bucket_start_epochs.extend(replayed_archive_bucket_start_epochs(&archive)?);
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                InvocationSourceScope::ProxyOnly,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            for row in rows {
                if !invocation_hourly_source_record_matches_range(&row, Some((start, end))) {
                    continue;
                }
                let Ok(bucket_start_epoch) = summary_rollup_bucket_start_epoch(&row.occurred_at)
                else {
                    continue;
                };
                if !bucket_start_epochs.contains(&bucket_start_epoch) {
                    continue;
                }
                add_invocation_hourly_source_record_to_proxy_perf_rollups(&mut materialized, &row)?;
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((materialized, unreadable_bucket_start_epochs))
}

pub(crate) async fn query_unmaterialized_proxy_perf_stage_rollups_from_archives(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<BTreeMap<String, ProxyPerfStageHourlyDelta>> {
    let pending_state = load_proxy_perf_stage_rollups_by_materialization_state(
        pool,
        start,
        end,
        exclude_invocation_ids,
    )
    .await?;

    if pending_state.unmaterialized.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut pending_bucket_start_epochs = pending_state
        .unmaterialized
        .keys()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .collect::<HashSet<_>>();
    pending_bucket_start_epochs.extend(
        pending_state
            .materialized
            .keys()
            .map(|(bucket_start_epoch, _)| *bucket_start_epoch),
    );
    let materialized_bucket_sources = load_materialized_rollup_bucket_sources(
        pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        &pending_bucket_start_epochs
            .iter()
            .map(|bucket_start_epoch| (*bucket_start_epoch, SOURCE_PROXY.to_string()))
            .collect::<HashSet<_>>(),
    )
    .await?;
    let (
        materialized_archive_perf_by_bucket_stage,
        mut unreadable_materialized_bucket_start_epochs,
    ) = load_materialized_proxy_perf_stage_rollups_for_buckets(
        pool,
        start,
        end,
        &pending_bucket_start_epochs
            .iter()
            .copied()
            .filter(|bucket_start_epoch| {
                materialized_bucket_sources
                    .contains(&(*bucket_start_epoch, SOURCE_PROXY.to_string()))
            })
            .collect::<HashSet<_>>(),
    )
    .await?;
    let mut known_materialized_archive_perf_by_bucket_stage =
        materialized_archive_perf_by_bucket_stage;
    merge_proxy_perf_stage_hourly_delta_map(
        &mut known_materialized_archive_perf_by_bucket_stage,
        &pending_state.materialized,
    )?;
    unreadable_materialized_bucket_start_epochs
        .extend(pending_state.unreadable_materialized_bucket_start_epochs);
    let materialized_by_bucket_stage =
        load_materialized_proxy_perf_rollups_for_range(pool, start, end).await?;
    let mut by_stage = BTreeMap::new();
    for ((bucket_start_epoch, stage), archive_delta) in pending_state.unmaterialized {
        let materialized_overlap =
            if unreadable_materialized_bucket_start_epochs.contains(&bucket_start_epoch) {
                None
            } else {
                build_materialized_pending_proxy_perf_overlap(
                    materialized_by_bucket_stage.get(&(bucket_start_epoch, stage.clone())),
                    materialized_bucket_sources
                        .contains(&(bucket_start_epoch, SOURCE_PROXY.to_string()))
                        .then(|| {
                            known_materialized_archive_perf_by_bucket_stage
                                .get(&(bucket_start_epoch, stage.clone()))
                        })
                        .flatten(),
                )
            };
        let Some(delta) =
            build_proxy_perf_stage_rollup_delta(&archive_delta, materialized_overlap.as_ref())?
        else {
            continue;
        };
        let entry = by_stage
            .entry(stage)
            .or_insert_with(ProxyPerfStageHourlyDelta::default);
        entry.sample_count += delta.sample_count;
        entry.sum_ms += delta.sum_ms;
        entry.max_ms = entry.max_ms.max(delta.max_ms);
        if entry.histogram.is_empty() {
            entry.histogram = empty_approx_histogram();
        }
        merge_approx_histogram_into(&mut entry.histogram, &delta.histogram)?;
    }

    Ok(by_stage)
}

pub(crate) async fn query_unmaterialized_invocation_archive_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<StatsTotals> {
    let mut totals = StatsTotals::default();
    for row in query_unmaterialized_invocation_archive_hourly_rollup_deltas(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
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

pub(crate) async fn query_unmaterialized_invocation_archive_totals_bounded(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: usize,
) -> Result<StatsTotals> {
    let mut totals = StatsTotals::default();
    for row in query_unmaterialized_invocation_archive_hourly_rollup_deltas_bounded(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        max_rows,
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

pub(crate) async fn query_unmaterialized_invocation_archive_totals_bounded_strict(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: usize,
) -> Result<StatsTotals> {
    let mut totals = StatsTotals::default();
    for row in query_unmaterialized_invocation_archive_hourly_rollup_deltas_bounded_strict(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        max_rows,
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

pub(crate) fn invocation_row_counts_toward_non_success_usage(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> bool {
    if status
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("interrupted"))
    {
        return true;
    }
    let classification = resolve_failure_classification(
        status,
        error_message,
        failure_kind,
        failure_class,
        is_actionable,
    );
    invocation_status_counts_toward_terminal_totals(status)
        && classification.failure_class != FailureClass::None
}

pub(crate) fn invocation_counts_toward_non_success_usage(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> bool {
    invocation_row_counts_toward_non_success_usage(
        status,
        error_message,
        failure_kind,
        failure_class,
        is_actionable,
    )
}

pub(crate) async fn query_unmaterialized_invocation_archive_non_success_usage(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<(f64, i64)> {
    let archive_rows = load_invocation_archives_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        range,
    )
    .await?;
    let mut total_cost = 0.0_f64;
    let mut total_tokens = 0_i64;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
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

pub(crate) async fn query_completed_invocation_archive_non_success_usage(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<(f64, i64)> {
    let archive_rows = load_completed_invocation_archive_paths_in_range(pool, range).await?;
    let mut total_cost = 0.0_f64;
    let mut total_tokens = 0_i64;

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
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

pub(crate) fn add_account_invocation_row_to_stats_delta(
    entry: &mut UpstreamAccountStatsDelta,
    row: &InvocationHourlySourceRecord,
) {
    accumulate_upstream_account_stats_delta(entry, row);
}
