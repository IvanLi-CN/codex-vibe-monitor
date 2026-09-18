#[derive(Debug, Clone, FromRow)]
pub(crate) struct MaterializedBucketRow {
    bucket_start_epoch: i64,
    source: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ReplayedInvocationArchiveRow {
    file_path: String,
    month_key: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) async fn load_materialized_rollup_bucket_sources(
    pool: &Pool<Sqlite>,
    target: &str,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_sources.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum materialized bucket epoch"))?;
    let max_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum materialized bucket epoch"))?;

    let rows = sqlx::query_as::<_, MaterializedBucketRow>(
        r#"
        SELECT bucket_start_epoch, source
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1
          AND bucket_start_epoch >= ?2
          AND bucket_start_epoch <= ?3
        "#,
    )
    .bind(target)
    .bind(min_bucket_epoch)
    .bind(max_bucket_epoch)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.bucket_start_epoch, row.source))
        .filter(|key| bucket_sources.contains(key))
        .collect())
}

pub(crate) fn shanghai_month_keys_for_bucket_starts(
    bucket_start_epochs: impl IntoIterator<Item = i64>,
) -> HashSet<String> {
    bucket_start_epochs
        .into_iter()
        .filter_map(|bucket_start_epoch| {
            Utc.timestamp_opt(bucket_start_epoch, 0)
                .single()
                .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
        })
        .collect()
}

pub(crate) fn shanghai_month_key_for_bucket_start(bucket_start_epoch: i64) -> Option<String> {
    Utc.timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

pub(crate) fn shanghai_month_bucket_start_epochs(month_key: &str) -> Result<HashSet<i64>> {
    let month_start = NaiveDate::parse_from_str(&format!("{month_key}-01"), "%Y-%m-%d")
        .with_context(|| format!("invalid archive month key: {month_key}"))?;
    let next_month_start = if month_start.month() == 12 {
        NaiveDate::from_ymd_opt(month_start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(month_start.year(), month_start.month() + 1, 1)
    }
    .ok_or_else(|| anyhow!("failed to resolve next month start for archive month {month_key}"))?;
    let month_start_local = month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve local month start for archive month {month_key}")
    })?;
    let next_month_start_local = next_month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve next local month start for archive month {month_key}")
    })?;
    let start_epoch = Shanghai
        .from_local_datetime(&month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();
    let end_epoch_exclusive = Shanghai
        .from_local_datetime(&next_month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize next archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();

    let mut bucket_start_epochs = HashSet::new();
    let mut current_epoch = align_bucket_epoch(start_epoch, 3_600, 0);
    while current_epoch < end_epoch_exclusive {
        bucket_start_epochs.insert(current_epoch);
        current_epoch += 3_600;
    }
    Ok(bucket_start_epochs)
}

pub(crate) fn archive_bucket_start_epochs_from_bounds(
    month_key: Option<&str>,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<HashSet<i64>> {
    if let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at) {
        let coverage_start_epoch = summary_rollup_bucket_start_epoch(coverage_start_at)?;
        let coverage_end_epoch = summary_rollup_bucket_start_epoch(coverage_end_at)?;
        if coverage_end_epoch < coverage_start_epoch {
            return Ok(HashSet::new());
        }

        let mut bucket_start_epochs = HashSet::new();
        let mut current_epoch = coverage_start_epoch;
        while current_epoch <= coverage_end_epoch {
            bucket_start_epochs.insert(current_epoch);
            current_epoch += 3_600;
        }
        return Ok(bucket_start_epochs);
    }

    month_key
        .map(shanghai_month_bucket_start_epochs)
        .transpose()
        .map(|maybe_buckets| maybe_buckets.unwrap_or_default())
}

pub(crate) fn archive_bucket_start_epochs_for_row(
    archive_row: &ArchiveBatchPathRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        archive_row.month_key.as_deref(),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) fn replayed_archive_bucket_start_epochs(
    archive_row: &ReplayedInvocationArchiveRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        Some(archive_row.month_key.as_str()),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) async fn load_replayed_invocation_archives_for_month_keys(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    month_keys: &HashSet<String>,
) -> Result<Vec<ReplayedInvocationArchiveRow>> {
    if month_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut month_keys = month_keys.iter().cloned().collect::<Vec<_>>();
    month_keys.sort();

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT batches.file_path, batches.month_key
             , batches.coverage_start_at, batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
         AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
         AND EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target =
        "#,
    );
    query.push_bind(target);
    query.push(
        r#"
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
         )
         AND batches.month_key IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for month_key in month_keys {
            separated.push_bind(month_key);
        }
    }
    query.push(") ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");

    query
        .build_query_as::<ReplayedInvocationArchiveRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) fn materialized_archive_path_row(
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
) -> ArchiveBatchPathRow {
    ArchiveBatchPathRow {
        file_path,
        month_key: None,
        coverage_start_at,
        coverage_end_at,
        historical_rollups_materialized_at: Some("materialized".to_string()),
        needs_overall: None,
        needs_failures: None,
    }
}

#[derive(Debug, Default)]
pub(crate) struct PendingInvocationArchiveOverallState {
    unmaterialized: BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    materialized: BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
    unreadable_unmaterialized_paths: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct PendingProxyPerfArchiveState {
    unmaterialized: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    materialized: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct PendingInvocationArchiveFailureState {
    unmaterialized_rows: Vec<ArchivedInvocationFailureRow>,
    materialized_row_counts: HashMap<(i64, String, String, i64, String), usize>,
    unreadable_materialized_bucket_start_epochs: HashSet<i64>,
}

pub(crate) fn merge_invocation_hourly_rollup_delta(
    target: &mut InvocationHourlyRollupDelta,
    delta: &InvocationHourlyRollupDelta,
) -> Result<()> {
    target.total_count += delta.total_count;
    target.success_count += delta.success_count;
    target.failure_count += delta.failure_count;
    target.terminal_count += delta.terminal_count;
    target.terminal_tokens += delta.terminal_tokens;
    target.terminal_cost += delta.terminal_cost;
    target.total_tokens += delta.total_tokens;
    target.input_tokens += delta.input_tokens;
    target.output_tokens += delta.output_tokens;
    target.cache_input_tokens += delta.cache_input_tokens;
    target.reasoning_tokens += delta.reasoning_tokens;
    target.total_cost += delta.total_cost;
    target.non_success_cost += delta.non_success_cost;
    target.total_latency_sample_count += delta.total_latency_sample_count;
    target.total_latency_sum_ms += delta.total_latency_sum_ms;
    target.first_byte_sample_count += delta.first_byte_sample_count;
    target.first_byte_sum_ms += delta.first_byte_sum_ms;
    target.first_byte_max_ms = target.first_byte_max_ms.max(delta.first_byte_max_ms);
    if target.first_byte_histogram.is_empty() {
        target.first_byte_histogram = delta.first_byte_histogram.clone();
    } else if !delta.first_byte_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_byte_histogram,
            &delta.first_byte_histogram,
        )?;
    }
    target.first_response_byte_total_sample_count += delta.first_response_byte_total_sample_count;
    target.first_response_byte_total_sum_ms += delta.first_response_byte_total_sum_ms;
    target.first_response_byte_total_max_ms = target
        .first_response_byte_total_max_ms
        .max(delta.first_response_byte_total_max_ms);
    if target.first_response_byte_total_histogram.is_empty() {
        target.first_response_byte_total_histogram =
            delta.first_response_byte_total_histogram.clone();
    } else if !delta.first_response_byte_total_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_response_byte_total_histogram,
            &delta.first_response_byte_total_histogram,
        )?;
    }
    target.first_token_sample_count += delta.first_token_sample_count;
    target.first_token_sum_ms += delta.first_token_sum_ms;
    target.first_token_max_ms = target.first_token_max_ms.max(delta.first_token_max_ms);
    if target.first_token_histogram.is_empty() {
        target.first_token_histogram = delta.first_token_histogram.clone();
    } else if !delta.first_token_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_token_histogram,
            &delta.first_token_histogram,
        )?;
    }
    Ok(())
}

pub(crate) fn merge_invocation_hourly_rollup_delta_map(
    target: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    source: &BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
) -> Result<()> {
    for (key, delta) in source {
        let entry = target
            .entry(key.clone())
            .or_insert_with(|| InvocationHourlyRollupDelta {
                first_byte_histogram: empty_approx_histogram(),
                first_response_byte_total_histogram: empty_approx_histogram(),
                first_token_histogram: empty_approx_histogram(),
                ..InvocationHourlyRollupDelta::default()
            });
        merge_invocation_hourly_rollup_delta(entry, delta)?;
    }
    Ok(())
}

pub(crate) fn merge_proxy_perf_stage_hourly_delta(
    target: &mut ProxyPerfStageHourlyDelta,
    delta: &ProxyPerfStageHourlyDelta,
) -> Result<()> {
    target.sample_count += delta.sample_count;
    target.sum_ms += delta.sum_ms;
    target.max_ms = target.max_ms.max(delta.max_ms);
    if target.histogram.is_empty() {
        target.histogram = delta.histogram.clone();
    } else if !delta.histogram.is_empty() {
        merge_approx_histogram_into(&mut target.histogram, &delta.histogram)?;
    }
    Ok(())
}

pub(crate) fn merge_proxy_perf_stage_hourly_delta_map(
    target: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    source: &BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
) -> Result<()> {
    for (key, delta) in source {
        let entry = target
            .entry(key.clone())
            .or_insert_with(|| ProxyPerfStageHourlyDelta {
                histogram: empty_approx_histogram(),
                ..ProxyPerfStageHourlyDelta::default()
            });
        merge_proxy_perf_stage_hourly_delta(entry, delta)?;
    }
    Ok(())
}

pub(crate) fn accumulate_failure_rollup_row_counts(
    counts: &mut HashMap<(i64, String, String, i64, String), usize>,
    rows: impl IntoIterator<Item = ArchivedInvocationFailureRow>,
) -> Result<()> {
    for row in rows {
        let Some(key) = archived_failure_rollup_key(&row)? else {
            continue;
        };
        *counts.entry(key).or_default() += 1;
    }
    Ok(())
}

pub(crate) async fn load_materialized_invocation_archive_hourly_rollup_deltas_for_bucket_sources(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<(
    BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    HashSet<i64>,
)> {
    if bucket_sources.is_empty() {
        return Ok((BTreeMap::new(), HashSet::new()));
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(
        bucket_sources
            .iter()
            .map(|(bucket_start_epoch, _)| *bucket_start_epoch),
    );
    let archive_rows = load_replayed_invocation_archives_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &month_keys,
    )
    .await?;
    let mut materialized = BTreeMap::<(i64, String), InvocationHourlyRollupDelta>::new();
    let mut unreadable_bucket_start_epochs = HashSet::new();

    for archive in archive_rows {
        let archive_row = materialized_archive_path_row(
            archive.file_path.clone(),
            archive.coverage_start_at.clone(),
            archive.coverage_end_at.clone(),
        );
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
        else {
            unreadable_bucket_start_epochs.extend(replayed_archive_bucket_start_epochs(&archive)?);
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
            let filtered_rows = rows
                .into_iter()
                .filter(|row| invocation_hourly_source_record_matches_range(row, range))
                .filter_map(|row| {
                    summary_rollup_bucket_start_epoch(&row.occurred_at)
                        .ok()
                        .map(|bucket_start_epoch| ((bucket_start_epoch, row.source.clone()), row))
                })
                .filter(|(key, _)| bucket_sources.contains(key))
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            if filtered_rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut materialized, &filtered_rows)?;
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((materialized, unreadable_bucket_start_epochs))
}

pub(crate) fn subtract_approx_histogram_counts(
    archive_histogram: &[i64],
    materialized_histogram: &[i64],
) -> ApproxHistogramCounts {
    let expected_len = archive_histogram.len().max(materialized_histogram.len());
    let mut delta = archive_histogram.to_vec();
    delta.resize(expected_len, 0);
    for (idx, slot) in delta.iter_mut().enumerate() {
        let materialized = materialized_histogram.get(idx).copied().unwrap_or_default();
        *slot = slot.saturating_sub(materialized).max(0);
    }
    delta
}

pub(crate) fn subtract_nonnegative_i64(archive_value: i64, materialized_value: i64) -> i64 {
    archive_value.saturating_sub(materialized_value).max(0)
}

pub(crate) fn subtract_nonnegative_f64(archive_value: f64, materialized_value: f64) -> f64 {
    (archive_value - materialized_value).max(0.0)
}

#[derive(Clone, Copy)]
struct InvocationHourlyRollupColumnSql {
    input_tokens: &'static str,
    output_tokens: &'static str,
    cache_input_tokens: &'static str,
    reasoning_tokens: &'static str,
    non_success_cost: &'static str,
    total_latency_sample_count: &'static str,
    total_latency_sum_ms: &'static str,
    first_token_sample_count: &'static str,
    first_token_sum_ms: &'static str,
    first_token_max_ms: &'static str,
    first_token_histogram: &'static str,
}

async fn optional_invocation_rollup_column_sql(
    pool: &Pool<Sqlite>,
    column: &str,
    present: &'static str,
    absent: &'static str,
) -> Result<&'static str> {
    Ok(
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", column).await? {
            present
        } else {
            absent
        },
    )
}

async fn invocation_hourly_rollup_column_sql(
    pool: &Pool<Sqlite>,
) -> Result<InvocationHourlyRollupColumnSql> {
    Ok(InvocationHourlyRollupColumnSql {
        input_tokens: optional_invocation_rollup_column_sql(
            pool,
            "input_tokens",
            "COALESCE(input_tokens, 0) AS input_tokens",
            "0 AS input_tokens",
        )
        .await?,
        output_tokens: optional_invocation_rollup_column_sql(
            pool,
            "output_tokens",
            "COALESCE(output_tokens, 0) AS output_tokens",
            "0 AS output_tokens",
        )
        .await?,
        cache_input_tokens: optional_invocation_rollup_column_sql(
            pool,
            "cache_input_tokens",
            "COALESCE(cache_input_tokens, 0) AS cache_input_tokens",
            "0 AS cache_input_tokens",
        )
        .await?,
        reasoning_tokens: optional_invocation_rollup_column_sql(
            pool,
            "reasoning_tokens",
            "COALESCE(reasoning_tokens, 0) AS reasoning_tokens",
            "0 AS reasoning_tokens",
        )
        .await?,
        non_success_cost: optional_invocation_rollup_column_sql(
            pool,
            "non_success_cost",
            "COALESCE(non_success_cost, 0.0) AS non_success_cost",
            "0.0 AS non_success_cost",
        )
        .await?,
        total_latency_sample_count: optional_invocation_rollup_column_sql(
            pool,
            "total_latency_sample_count",
            "COALESCE(total_latency_sample_count, 0) AS total_latency_sample_count",
            "0 AS total_latency_sample_count",
        )
        .await?,
        total_latency_sum_ms: optional_invocation_rollup_column_sql(
            pool,
            "total_latency_sum_ms",
            "COALESCE(total_latency_sum_ms, 0.0) AS total_latency_sum_ms",
            "0.0 AS total_latency_sum_ms",
        )
        .await?,
        first_token_sample_count: optional_invocation_rollup_column_sql(
            pool,
            "first_token_sample_count",
            "COALESCE(first_token_sample_count, 0) AS first_token_sample_count",
            "0 AS first_token_sample_count",
        )
        .await?,
        first_token_sum_ms: optional_invocation_rollup_column_sql(
            pool,
            "first_token_sum_ms",
            "COALESCE(first_token_sum_ms, 0.0) AS first_token_sum_ms",
            "0.0 AS first_token_sum_ms",
        )
        .await?,
        first_token_max_ms: optional_invocation_rollup_column_sql(
            pool,
            "first_token_max_ms",
            "COALESCE(first_token_max_ms, 0.0) AS first_token_max_ms",
            "0.0 AS first_token_max_ms",
        )
        .await?,
        first_token_histogram: optional_invocation_rollup_column_sql(
            pool,
            "first_token_histogram",
            "COALESCE(first_token_histogram, '[]') AS first_token_histogram",
            "'[]' AS first_token_histogram",
        )
        .await?,
    })
}

pub(crate) async fn load_materialized_invocation_rollup_record(
    pool: &Pool<Sqlite>,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let columns = invocation_hourly_rollup_column_sql(pool).await?;
    let query = format!(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            {reasoning_tokens},
            total_cost,
            {non_success_cost},
            {total_latency_sample_count},
            {total_latency_sum_ms},
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            {first_token_sample_count},
            {first_token_sum_ms},
            {first_token_max_ms},
            {first_token_histogram}
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch = ?1
          AND source = ?2
        LIMIT 1
        "#,
        input_tokens = columns.input_tokens,
        output_tokens = columns.output_tokens,
        cache_input_tokens = columns.cache_input_tokens,
        reasoning_tokens = columns.reasoning_tokens,
        non_success_cost = columns.non_success_cost,
        total_latency_sample_count = columns.total_latency_sample_count,
        total_latency_sum_ms = columns.total_latency_sum_ms,
        first_token_sample_count = columns.first_token_sample_count,
        first_token_sum_ms = columns.first_token_sum_ms,
        first_token_max_ms = columns.first_token_max_ms,
        first_token_histogram = columns.first_token_histogram,
    );
    sqlx::query_as::<_, InvocationHourlyRollupRecord>(&query)
        .bind(bucket_start_epoch)
        .bind(source)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

fn invocation_hourly_rollup_delta_from_record(
    row: &InvocationHourlyRollupRecord,
) -> InvocationHourlyRollupDelta {
    InvocationHourlyRollupDelta {
        total_count: row.total_count,
        success_count: row.success_count,
        failure_count: row.failure_count,
        total_tokens: row.total_tokens,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_input_tokens: row.cache_input_tokens,
        reasoning_tokens: row.reasoning_tokens,
        total_cost: row.total_cost,
        non_success_cost: row.non_success_cost,
        total_latency_sample_count: row.total_latency_sample_count,
        total_latency_sum_ms: row.total_latency_sum_ms,
        first_byte_sample_count: row.first_byte_sample_count,
        first_byte_sum_ms: row.first_byte_sum_ms,
        first_byte_max_ms: row.first_byte_max_ms,
        first_byte_histogram: decode_approx_histogram(&row.first_byte_histogram),
        first_response_byte_total_sample_count: row.first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms: row.first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: row.first_response_byte_total_max_ms,
        first_response_byte_total_histogram: decode_approx_histogram(
            &row.first_response_byte_total_histogram,
        ),
        first_token_sample_count: row.first_token_sample_count,
        first_token_sum_ms: row.first_token_sum_ms,
        first_token_max_ms: row.first_token_max_ms,
        first_token_histogram: decode_approx_histogram(&row.first_token_histogram),
        ..InvocationHourlyRollupDelta::default()
    }
}

fn subtract_invocation_hourly_rollup_delta(
    archive_delta: &InvocationHourlyRollupDelta,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
) -> InvocationHourlyRollupDelta {
    let (first_byte_histogram, first_response_byte_total_histogram, first_token_histogram) =
        subtract_invocation_hourly_rollup_histograms(archive_delta, materialized_row);
    let row_i64 = |value: fn(&InvocationHourlyRollupRecord) -> i64| {
        materialized_row.map(value).unwrap_or(0).max(0)
    };
    let row_f64 = |value: fn(&InvocationHourlyRollupRecord) -> f64| {
        materialized_row.map(value).unwrap_or(0.0)
    };
    InvocationHourlyRollupDelta {
        total_count: subtract_nonnegative_i64(
            archive_delta.total_count,
            row_i64(|row| row.total_count),
        ),
        success_count: subtract_nonnegative_i64(
            archive_delta.success_count,
            row_i64(|row| row.success_count),
        ),
        failure_count: subtract_nonnegative_i64(
            archive_delta.failure_count,
            row_i64(|row| row.failure_count),
        ),
        total_tokens: subtract_nonnegative_i64(
            archive_delta.total_tokens,
            row_i64(|row| row.total_tokens),
        ),
        input_tokens: subtract_nonnegative_i64(
            archive_delta.input_tokens,
            row_i64(|row| row.input_tokens),
        ),
        output_tokens: subtract_nonnegative_i64(
            archive_delta.output_tokens,
            row_i64(|row| row.output_tokens),
        ),
        cache_input_tokens: subtract_nonnegative_i64(
            archive_delta.cache_input_tokens,
            row_i64(|row| row.cache_input_tokens),
        ),
        reasoning_tokens: subtract_nonnegative_i64(
            archive_delta.reasoning_tokens,
            row_i64(|row| row.reasoning_tokens),
        ),
        total_cost: subtract_nonnegative_f64(
            archive_delta.total_cost,
            row_f64(|row| row.total_cost),
        ),
        non_success_cost: subtract_nonnegative_f64(
            archive_delta.non_success_cost,
            row_f64(|row| row.non_success_cost),
        ),
        total_latency_sample_count: subtract_nonnegative_i64(
            archive_delta.total_latency_sample_count,
            row_i64(|row| row.total_latency_sample_count),
        ),
        total_latency_sum_ms: subtract_nonnegative_f64(
            archive_delta.total_latency_sum_ms,
            row_f64(|row| row.total_latency_sum_ms),
        ),
        first_byte_sample_count: first_byte_histogram.iter().copied().sum(),
        first_byte_sum_ms: subtract_nonnegative_f64(
            archive_delta.first_byte_sum_ms,
            row_f64(|row| row.first_byte_sum_ms),
        ),
        first_byte_max_ms: approx_histogram_percentile_ms(&first_byte_histogram, 1.0)
            .unwrap_or(0.0),
        first_byte_histogram,
        first_response_byte_total_sample_count: first_response_byte_total_histogram
            .iter()
            .copied()
            .sum(),
        first_response_byte_total_sum_ms: subtract_nonnegative_f64(
            archive_delta.first_response_byte_total_sum_ms,
            row_f64(|row| row.first_response_byte_total_sum_ms),
        ),
        first_response_byte_total_max_ms: approx_histogram_percentile_ms(
            &first_response_byte_total_histogram,
            1.0,
        )
        .unwrap_or(0.0),
        first_response_byte_total_histogram,
        first_token_sample_count: first_token_histogram.iter().copied().sum(),
        first_token_sum_ms: subtract_nonnegative_f64(
            archive_delta.first_token_sum_ms,
            row_f64(|row| row.first_token_sum_ms),
        ),
        first_token_max_ms: approx_histogram_percentile_ms(&first_token_histogram, 1.0)
            .unwrap_or(0.0),
        first_token_histogram,
        ..InvocationHourlyRollupDelta::default()
    }
}

fn subtract_invocation_hourly_rollup_histograms(
    archive_delta: &InvocationHourlyRollupDelta,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
) -> (
    ApproxHistogramCounts,
    ApproxHistogramCounts,
    ApproxHistogramCounts,
) {
    let first_byte_materialized = materialized_row
        .map(|row| decode_approx_histogram(&row.first_byte_histogram))
        .unwrap_or_else(empty_approx_histogram);
    let first_response_materialized = materialized_row
        .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
        .unwrap_or_else(empty_approx_histogram);
    let first_token_materialized = materialized_row
        .map(|row| decode_approx_histogram(&row.first_token_histogram))
        .unwrap_or_else(empty_approx_histogram);
    (
        subtract_approx_histogram_counts(
            &archive_delta.first_byte_histogram,
            &first_byte_materialized,
        ),
        subtract_approx_histogram_counts(
            &archive_delta.first_response_byte_total_histogram,
            &first_response_materialized,
        ),
        subtract_approx_histogram_counts(
            &archive_delta.first_token_histogram,
            &first_token_materialized,
        ),
    )
}

fn invocation_hourly_rollup_delta_is_empty(delta: &InvocationHourlyRollupDelta) -> bool {
    delta.total_count <= 0
        && delta.success_count <= 0
        && delta.failure_count <= 0
        && delta.total_tokens <= 0
        && delta.cache_input_tokens <= 0
        && delta.total_cost <= 0.0
        && delta.non_success_cost <= 0.0
        && delta.total_latency_sample_count <= 0
        && delta.first_byte_sample_count <= 0
        && delta.first_response_byte_total_sample_count <= 0
        && delta.first_token_sample_count <= 0
}

fn invocation_hourly_rollup_delta_to_record(
    bucket_start_epoch: i64,
    delta: &InvocationHourlyRollupDelta,
) -> Result<InvocationHourlyRollupRecord> {
    Ok(InvocationHourlyRollupRecord {
        bucket_start_epoch,
        total_count: delta.total_count,
        success_count: delta.success_count,
        failure_count: delta.failure_count,
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
        first_byte_histogram: encode_approx_histogram(&delta.first_byte_histogram)?,
        first_response_byte_total_sample_count: delta.first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms: delta.first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: delta.first_response_byte_total_max_ms,
        first_response_byte_total_histogram: encode_approx_histogram(
            &delta.first_response_byte_total_histogram,
        )?,
        first_token_sample_count: delta.first_token_sample_count,
        first_token_sum_ms: delta.first_token_sum_ms,
        first_token_max_ms: delta.first_token_max_ms,
        first_token_histogram: encode_approx_histogram(&delta.first_token_histogram)?,
    })
}

pub(crate) fn build_invocation_hourly_rollup_delta_record(
    bucket_start_epoch: i64,
    archive_delta: &InvocationHourlyRollupDelta,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let delta = subtract_invocation_hourly_rollup_delta(archive_delta, materialized_row);
    if invocation_hourly_rollup_delta_is_empty(&delta) {
        return Ok(None);
    }
    invocation_hourly_rollup_delta_to_record(bucket_start_epoch, &delta).map(Some)
}

pub(crate) fn build_materialized_pending_invocation_rollup_overlap_record(
    bucket_start_epoch: i64,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
    completed_archive_delta: Option<&InvocationHourlyRollupDelta>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let Some(materialized_row) = materialized_row else {
        return Ok(None);
    };
    let materialized_delta = invocation_hourly_rollup_delta_from_record(materialized_row);
    let completed_record = completed_archive_delta
        .map(|delta| invocation_hourly_rollup_delta_to_record(bucket_start_epoch, delta))
        .transpose()?;
    build_invocation_hourly_rollup_delta_record(
        bucket_start_epoch,
        &materialized_delta,
        completed_record.as_ref(),
    )
}
