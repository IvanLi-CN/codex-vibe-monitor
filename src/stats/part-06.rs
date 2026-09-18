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
    let archive_deltas = collect_upstream_account_archive_hourly_rollup_deltas(
        archive_rows,
        source_scope,
        range,
        exclude_invocation_ids,
        upstream_account_id,
    )
    .await?;
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

async fn collect_upstream_account_archive_hourly_rollup_deltas(
    archive_rows: Vec<ArchiveBatchPathRow>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
) -> Result<BTreeMap<i64, UpstreamAccountStatsDelta>> {
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
            accumulate_upstream_account_archive_rows(
                rows,
                range,
                exclude_invocation_ids,
                upstream_account_id,
                &mut archive_deltas,
            )?;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    Ok(archive_deltas)
}

fn accumulate_upstream_account_archive_rows(
    rows: Vec<InvocationHourlySourceRecord>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    upstream_account_id: i64,
    archive_deltas: &mut BTreeMap<i64, UpstreamAccountStatsDelta>,
) -> Result<()> {
    for row in rows {
        if exclude_invocation_ids.is_some_and(|ids| ids.contains(&row.id))
            || !invocation_hourly_source_record_matches_range(&row, range)
            || row.resolved_upstream_account_id() != Some(upstream_account_id)
        {
            continue;
        }
        let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
        add_account_invocation_row_to_stats_delta(
            archive_deltas.entry(bucket_start_epoch).or_default(),
            &row,
        );
    }
    Ok(())
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
    let totals_by_account = collect_unmaterialized_upstream_account_totals_by_account(
        archive_rows,
        source_scope,
        range,
        exclude_invocation_ids,
    )
    .await?;
    Ok(totals_by_account)
}

async fn collect_unmaterialized_upstream_account_totals_by_account(
    archive_rows: Vec<ArchiveBatchPathRow>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<HashMap<i64, StatsTotals>> {
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
                add_unmaterialized_upstream_account_row(
                    &mut totals_by_account,
                    row,
                    range,
                    exclude_invocation_ids,
                );
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(totals_by_account)
}

fn add_unmaterialized_upstream_account_row(
    totals_by_account: &mut HashMap<i64, StatsTotals>,
    row: InvocationHourlySourceRecord,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) {
    if exclude_invocation_ids.is_some_and(|ids| ids.contains(&row.id))
        || !invocation_hourly_source_record_matches_range(&row, range)
    {
        return;
    }
    let Some(account_id) = row.resolved_upstream_account_id().filter(|id| *id > 0) else {
        return;
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
