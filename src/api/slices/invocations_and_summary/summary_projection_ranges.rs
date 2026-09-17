async fn load_summary_projection_rollup_totals_in_range(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
) -> Result<(
    HashMap<(i64, Option<i64>), StatsTotals>,
    HashMap<(i64, Option<i64>), i64>,
)> {
    count_summary_projection_rollup_rows(pool, "invocation_rollup_hourly", range).await?;
    count_summary_projection_rollup_rows(pool, "upstream_account_stats_hourly", range).await?;
    let mut totals = HashMap::<(i64, Option<i64>), StatsTotals>::new();
    let mut non_success_tokens = HashMap::<(i64, Option<i64>), i64>::new();
    let mut rollup_bytes = 0usize;
    load_summary_projection_overall_rollup_rows(pool, range, &mut totals, &mut rollup_bytes)
        .await?;
    load_summary_projection_account_rollup_rows(
        pool,
        range,
        &mut totals,
        &mut non_success_tokens,
        &mut rollup_bytes,
    )
    .await?;
    Ok((totals, non_success_tokens))
}

async fn load_summary_projection_overall_rollup_rows(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
    totals: &mut HashMap<(i64, Option<i64>), StatsTotals>,
    rollup_bytes: &mut usize,
) -> Result<()> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_start_epoch, total_count, success_count, failure_count, total_tokens, \
         total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost, 0 AS non_success_tokens \
         FROM invocation_rollup_hourly",
    );
    if let Some((start, end)) = range {
        query
            .push(" WHERE bucket_start_epoch >= ")
            .push_bind(start)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end);
    }
    let rows = query
        .build_query_as::<SummaryProjectionRollupRow>()
        .fetch_all(pool)
        .await
        .context("summary projection overall rollup hydration failed")?;
    for row in rows {
        *rollup_bytes = rollup_bytes.saturating_add(256);
        if *rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        let bucket_totals = StatsTotals {
            total_count: row.total_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            non_success_cost: row.non_success_cost,
        };
        let key = (row.bucket_start_epoch, None);
        let previous = totals.get(&key).copied().unwrap_or_default();
        totals.insert(key, previous.add(bucket_totals));
    }
    Ok(())
}

async fn load_summary_projection_account_rollup_rows(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
    totals: &mut HashMap<(i64, Option<i64>), StatsTotals>,
    non_success_tokens: &mut HashMap<(i64, Option<i64>), i64>,
    rollup_bytes: &mut usize,
) -> Result<()> {
    #[derive(Debug, FromRow)]
    struct AccountRollupRow {
        upstream_account_id: i64,
        bucket_start_epoch: i64,
        total_count: i64,
        success_count: i64,
        failure_count: i64,
        total_tokens: i64,
        total_cost: f64,
        non_success_cost: f64,
        non_success_tokens: i64,
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT upstream_account_id, bucket_start_epoch, total_count, success_count, \
         failure_count, total_tokens, total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost, \
         COALESCE(activity_v2_non_success_tokens, 0) AS non_success_tokens \
         FROM upstream_account_stats_hourly WHERE upstream_account_id <> 0",
    );
    if let Some((start, end)) = range {
        query
            .push(" AND bucket_start_epoch >= ")
            .push_bind(start)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end);
    }
    let rows = query
        .build_query_as::<AccountRollupRow>()
        .fetch_all(pool)
        .await
        .context("summary projection account rollup hydration failed")?;
    for row in rows {
        *rollup_bytes = rollup_bytes.saturating_add(256);
        if *rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        if row.upstream_account_id > 0 {
            let bucket_totals = StatsTotals {
                total_count: row.total_count,
                success_count: row.success_count,
                failure_count: row.failure_count,
                total_tokens: row.total_tokens,
                total_cost: row.total_cost,
                non_success_cost: row.non_success_cost,
            };
            let key = (row.bucket_start_epoch, Some(row.upstream_account_id));
            let previous = totals.get(&key).copied().unwrap_or_default();
            totals.insert(key, previous.add(bucket_totals));
            non_success_tokens.insert(
                key,
                non_success_tokens
                    .get(&key)
                    .copied()
                    .unwrap_or_default()
                    .saturating_add(row.non_success_tokens),
            );
        }
        *non_success_tokens
            .entry((row.bucket_start_epoch, None))
            .or_default() += row.non_success_tokens;
    }
    Ok(())
}

async fn load_summary_projection_rollup_usage(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<(i64, Option<i64>), UsageBreakdownResponse>> {
    load_summary_projection_rollup_usage_in_range(pool, None).await
}

async fn load_summary_projection_rollup_usage_in_range(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
) -> Result<HashMap<(i64, Option<i64>), UsageBreakdownResponse>> {
    let mut connection = pool.acquire().await?;
    count_summary_projection_rollup_rows(
        &mut *connection,
        "upstream_account_usage_breakdown_hourly",
        range,
    )
    .await?;
    let (start_epoch, end_epoch) = range.unwrap_or((i64::MIN, i64::MAX));
    let rows = query_upstream_account_usage_breakdown_hourly_rollup_range_bounded_tx(
        &mut connection,
        start_epoch,
        end_epoch,
        InvocationSourceScope::All,
        None,
        SUMMARY_PROJECTION_MAX_ROLLUP_BYTES,
    )
    .await
    .map_err(|error| anyhow!("summary projection usage rollup hydration failed: {error:?}"))?;

    let mut accumulators = HashMap::<(i64, Option<i64>), UsageBreakdownAccumulator>::new();
    let mut rollup_bytes = 0usize;
    for row in rows {
        rollup_bytes = rollup_bytes.saturating_add(
            256usize
                .saturating_add(row.model.len())
                .saturating_add(row.reasoning_effort.as_ref().map_or(0, String::len)),
        );
        if rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection usage rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        let bucket_start_epoch = row.bucket_start_epoch;
        let upstream_account_id = row.upstream_account_id;
        let aggregate = UpstreamAccountUsageBreakdownAggregateRow {
            upstream_account_id,
            model: row.model,
            reasoning_effort: row.reasoning_effort,
            request_count: row.request_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            cache_write_tokens: row.cache_write_tokens,
            cache_read_tokens: row.cache_read_tokens,
            output_tokens: row.output_tokens,
            cost_input: row.cost_input,
            cost_cache_write: row.cost_cache_write,
            cost_cache_read: row.cost_cache_read,
            cost_output: row.cost_output,
            cost_reasoning: row.cost_reasoning,
            cost_unknown: row.cost_unknown,
            has_cost: row.has_cost,
            performance_total_tokens: row.performance_total_tokens,
            performance_stream_output_tokens: row.performance_stream_output_tokens,
            performance_stream_duration_ms: row.performance_stream_duration_ms,
            performance_response_sample_count: row.performance_response_sample_count,
            performance_response_sum_ms: row.performance_response_sum_ms,
            performance_first_byte_sample_count: row.performance_first_byte_sample_count,
            performance_first_byte_sum_ms: row.performance_first_byte_sum_ms,
            performance_first_token_sample_count: row.performance_first_token_sample_count,
            performance_first_token_sum_ms: row.performance_first_token_sum_ms,
            performance_usage_duration_sample_count: row.performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms: row.performance_usage_duration_sum_ms,
        };
        // The rollup table stores account-scoped rows, including a NULL account row for
        // account-less invocations.  The global projection must include every row exactly once;
        // a NULL row is already the global key and must not be inserted twice.
        accumulators
            .entry((bucket_start_epoch, None))
            .or_default()
            .add_aggregate_row(&aggregate);
        if let Some(account_id) = upstream_account_id {
            accumulators
                .entry((bucket_start_epoch, Some(account_id)))
                .or_default()
                .add_aggregate_row(&aggregate);
        }
    }
    Ok(accumulators
        .into_iter()
        .map(|(key, accumulator)| (key, accumulator.into_response()))
        .collect())
}

fn summary_projection_archive_column(column: &str, available: bool, fallback: &str) -> String {
    if available {
        format!("{column} AS {column}")
    } else {
        format!("{fallback} AS {column}")
    }
}

fn summary_projection_archive_overlap_range(
    archive: &crate::stats::ArchiveBatchPathRow,
    requested_range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let coverage_start = archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)
        .unwrap_or(requested_range.start);
    let coverage_end =
        summary_projection_archive_coverage_end_exclusive(archive).unwrap_or(requested_range.end);
    let start = coverage_start.max(requested_range.start);
    let end = coverage_end.min(requested_range.end);
    (start < end).then_some(ExactUtcRange { start, end })
}

fn summary_projection_archive_coverage_end_exclusive(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Option<DateTime<Utc>> {
    // Archive manifests record their final row inclusively, while every exact projection range
    // is half-open. Archive rows use second-granularity timestamps, so advance the endpoint by
    // one second before issuing the `< end` archive query.
    archive
        .coverage_end_at()
        .and_then(parse_to_utc_datetime)
        .and_then(|end| end.checked_add_signed(ChronoDuration::seconds(1)))
}

fn summary_projection_archive_is_fully_within_exact_horizon(
    archive: &crate::stats::ArchiveBatchPathRow,
    exact_horizon: ExactUtcRange,
) -> bool {
    let Some(start) = archive.coverage_start_at().and_then(parse_to_utc_datetime) else {
        return false;
    };
    let Some(end) = summary_projection_archive_coverage_end_exclusive(archive) else {
        return false;
    };
    start >= exact_horizon.start && end <= exact_horizon.end
}

fn summary_projection_archive_is_fully_represented_for_current(
    archive: &crate::stats::ArchiveBatchPathRow,
    exact_horizon: ExactUtcRange,
) -> bool {
    let Some(start) = archive.coverage_start_at().and_then(parse_to_utc_datetime) else {
        return false;
    };
    let Some(end) = summary_projection_archive_coverage_end_exclusive(archive) else {
        return false;
    };
    start >= exact_horizon.start && end <= exact_horizon.end
}

fn summary_projection_range_is_fully_within_exact_horizon(
    range: ExactUtcRange,
    exact_horizon: ExactUtcRange,
) -> bool {
    range.start >= exact_horizon.start && range.end <= exact_horizon.end
}

fn summary_projection_archive_has_exact_all_time_source(
    archive: &crate::stats::ArchiveBatchPathRow,
    actual_coverage_ranges: &HashMap<String, ExactUtcRange>,
    exact_horizon: ExactUtcRange,
) -> bool {
    summary_projection_archive_is_fully_within_exact_horizon(archive, exact_horizon)
        || actual_coverage_ranges
            .get(archive.file_path())
            .is_some_and(|range| {
                summary_projection_range_is_fully_within_exact_horizon(*range, exact_horizon)
            })
}

fn summary_projection_archive_coverage_range(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Option<ExactUtcRange> {
    let start = archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)?;
    let end = summary_projection_archive_coverage_end_exclusive(archive)?;
    (start < end).then_some(ExactUtcRange { start, end })
}

fn summary_projection_archive_raw_admission_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    summary_projection_resident_record_budget_exceeded(error)
        || message.contains("archive byte budget exceeded before preview fetch")
        // The shared archive reader names this bounded admission error after its
        // raw/unmaterialized mode. Materialized archives use the same reader for exact rolling
        // repairs and newest-N candidates, where it must make only that selection unavailable.
        || message.contains("unmaterialized archive exact-record budget")
        || message.contains("paged boundary raw archive admission exceeded")
        || (message.contains("exact-record budget")
            && message.contains("unmaterialized archive data"))
}

fn summary_projection_all_time_materialized_scope_coverage(
    archives: &[crate::stats::ArchiveBatchPathRow],
    replay_coverage: &HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    archive_account_manifest_refreshed_paths: &HashSet<String>,
    archive_account_ids_by_file: &HashMap<String, HashSet<i64>>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool)> {
    let mut global_covered = true;
    let mut accounts_covered = true;

    for archive in archives {
        if !archive.has_materialized_historical_rollups() {
            continue;
        }
        let replay = summary_projection_effective_replay_coverage(
            archive,
            replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        let Some(range) = summary_projection_archive_coverage_range(archive) else {
            // Legacy manifests lack a finite durable coverage range. Materialization alone does
            // not prove the compact aggregate was derived from this immutable archive, so every
            // scope still requires an identity-matching replay marker.
            global_covered &= replay.overall;
            accounts_covered &= replay.account_stats
                && archive_account_manifest_refreshed_paths.contains(archive.file_path());
            continue;
        };
        if summary_projection_exact_range_fits_bucket_budget(range).is_err() {
            return Ok((false, false));
        }
        // A missing manifest cannot prove that the archive contains no account-scoped rows.
        // Serving its global compact aggregate to an account request would silently omit those
        // rows when the account rollup is absent. The refresh marker additionally proves this
        // otherwise bounded list was completed, rather than an interrupted prefix.
        let account_ids = archive_account_ids_by_file.get(archive.file_path());
        if account_ids.is_none() {
            accounts_covered = false;
        }
        accounts_covered &= replay.account_stats
            && archive_account_manifest_refreshed_paths.contains(archive.file_path());
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            // An aggregate row alone can represent an interrupted replay prefix. Its durable
            // replay marker is the completion proof, so require both before replacing the
            // exact archive source outside the bounded raw horizon.
            global_covered &= replay.overall && hourly_rollup_totals.contains_key(&(bucket, None));
            if let Some(account_ids) = account_ids.filter(|account_ids| !account_ids.is_empty()) {
                accounts_covered &= account_ids.iter().all(|account_id| {
                    hourly_rollup_totals.contains_key(&(bucket, Some(*account_id)))
                });
            }
            if !global_covered && !accounts_covered {
                return Ok((false, false));
            }
            bucket = bucket.saturating_add(3_600);
        }
    }

    Ok((global_covered, accounts_covered))
}

async fn load_summary_projection_archive_coverage_range(
    pool: &Pool<Sqlite>,
) -> Result<Option<ExactUtcRange>> {
    let (first, last) = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT MIN(occurred_at), MAX(occurred_at) FROM codex_invocations",
    )
    .fetch_one(pool)
    .await?;
    let (Some(first), Some(last)) = (first.as_deref(), last.as_deref()) else {
        return Ok(None);
    };
    let start = parse_to_utc_datetime(first)
        .ok_or_else(|| anyhow!("summary projection archive coverage start is invalid"))?;
    let end = parse_to_utc_datetime(last)
        .and_then(|last| last.checked_add_signed(ChronoDuration::seconds(1)))
        .ok_or_else(|| anyhow!("summary projection archive coverage end is invalid"))?;
    (start < end)
        .then_some(ExactUtcRange { start, end })
        .ok_or_else(|| anyhow!("summary projection archive coverage range is empty"))
        .map(Some)
}

fn summary_projection_archive_has_coverage_bounds(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> bool {
    archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)
        .is_some()
        && archive
            .coverage_end_at()
            .and_then(parse_to_utc_datetime)
            .is_some()
}

fn summary_projection_effective_replay_coverage(
    _archive: &crate::stats::ArchiveBatchPathRow,
    coverage: SummaryProjectionArchiveReplayCoverage,
) -> SummaryProjectionArchiveReplayCoverage {
    coverage
}

fn summary_projection_archive_exact_ranges(
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    summary_projection_archive_exact_ranges_with_coverage(
        archive_has_materialized_rollups,
        SummaryProjectionArchiveReplayFlags::default(),
        exact_range,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
    )
}

#[derive(Debug, Default, Clone, Copy)]
struct SummaryProjectionArchiveReplayFlags {
    overall: Option<bool>,
    account: Option<bool>,
    usage: Option<bool>,
}

fn summary_projection_archive_exact_ranges_with_coverage(
    archive_has_materialized_rollups: bool,
    replay_flags: SummaryProjectionArchiveReplayFlags,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    if !archive_has_materialized_rollups {
        return vec![exact_range];
    }

    let first_bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp(), 3_600, 0);
    let first_full_bucket = ceil_hour_epoch(exact_range.start.timestamp());
    let mut exact_buckets = protected_boundary_buckets.clone();
    let mut bucket = first_bucket;
    while bucket <= last_bucket {
        let bucket_end = bucket.saturating_add(3_600);
        let is_full_bucket =
            bucket >= first_full_bucket && bucket_end <= exact_range.end.timestamp();
        if is_full_bucket && !exact_buckets.contains(&bucket) {
            let global_covered = hourly_rollup_totals.contains_key(&(bucket, None))
                && replay_flags.overall.is_none_or(|replayed| replayed);
            let global_usage_covered = hourly_rollup_usage.contains_key(&(bucket, None))
                && replay_flags.usage.is_none_or(|replayed| replayed);
            // Durable archive replay markers prove each response dimension consumed this whole
            // archive. Without them, a present rollup key is not enough: it may be stale or
            // partial, and requiring every durable account would also turn inactive accounts
            // into a full-archive exact read on every refresh.
            let accounts_covered = match (replay_flags.account, replay_flags.usage) {
                (Some(account_replayed), Some(usage_replayed)) => {
                    account_replayed && usage_replayed
                }
                _ => {
                    known_account_ids.iter().all(|account_id| {
                        hourly_rollup_totals.contains_key(&(bucket, Some(*account_id)))
                            && hourly_rollup_usage.contains_key(&(bucket, Some(*account_id)))
                    }) && !known_account_ids.is_empty()
                }
            };
            if !global_covered || !global_usage_covered || !accounts_covered {
                exact_buckets.insert(bucket);
            }
        }
        bucket = bucket.saturating_add(3_600);
    }

    let mut sorted_buckets = exact_buckets.into_iter().collect::<Vec<_>>();
    sorted_buckets.sort_unstable();
    let mut ranges = Vec::<ExactUtcRange>::new();
    for bucket in sorted_buckets {
        let start = Utc.timestamp_opt(bucket, 0).single();
        let end = Utc.timestamp_opt(bucket.saturating_add(3_600), 0).single();
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let start = start.max(exact_range.start);
        let end = end.min(exact_range.end);
        if start >= end {
            continue;
        }
        if let Some(previous) = ranges.last_mut()
            && previous.end >= start
        {
            previous.end = previous.end.max(end);
        } else {
            ranges.push(ExactUtcRange { start, end });
        }
    }
    ranges
}

fn summary_projection_merge_exact_ranges(mut ranges: Vec<ExactUtcRange>) -> Vec<ExactUtcRange> {
    ranges.sort_by_key(|range| range.start);
    let mut merged = Vec::<ExactUtcRange>::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && previous.end >= range.start
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}
