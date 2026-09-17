#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryProjectionOverflowedBoundaryCoverage {
    high_watermark_id: i64,
    unknown_coverage_ranges: Vec<ExactUtcRange>,
}

async fn summary_projection_overflowed_boundary_manifest_coverage(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
) -> Result<Option<SummaryProjectionOverflowedBoundaryCoverage>> {
    // The overflow path pages manifest metadata even when some partitions lack durable replay.
    // Bounded gaps are localized by the exact-range admission below. A legacy manifest without
    // bounds is localized from its immutable `month_key`; malformed keys retain the requested
    // horizon as the conservative fail-closed fallback.
    let Some(high_watermark_id) =
        summary_projection_completed_manifest_high_watermark(pool).await?
    else {
        return Ok(None);
    };
    let month_keys = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT batches.month_key FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1 \
           AND (batches.coverage_start_at IS NULL OR batches.coverage_end_at IS NULL) \
         ORDER BY batches.month_key \
         LIMIT ?2",
    )
    .bind(high_watermark_id)
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection overflowed boundary manifest coverage hydration failed")?;
    let mut unknown_coverage_ranges = Vec::new();
    let mut horizon_fallback_count = 0usize;
    for month_key in month_keys {
        let buckets = match crate::stats::archive_bucket_start_epochs_from_bounds(
            Some(&month_key),
            None,
            None,
        ) {
            Ok(buckets) if !buckets.is_empty() => buckets,
            // Without a parseable partition key, no finite source boundary exists. Preserve the
            // established fail-closed behavior, limited to the supported request horizon.
            Ok(_) | Err(_) => {
                horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                unknown_coverage_ranges.push(exact_horizon);
                continue;
            }
        };
        let start_epoch = buckets
            .iter()
            .min()
            .copied()
            .expect("non-empty archive buckets");
        let end_epoch = buckets
            .iter()
            .max()
            .copied()
            .expect("non-empty archive buckets")
            + 3_600;
        let Some(start) = Utc.timestamp_opt(start_epoch, 0).single() else {
            horizon_fallback_count = horizon_fallback_count.saturating_add(1);
            unknown_coverage_ranges.push(exact_horizon);
            continue;
        };
        let Some(end) = Utc.timestamp_opt(end_epoch, 0).single() else {
            horizon_fallback_count = horizon_fallback_count.saturating_add(1);
            unknown_coverage_ranges.push(exact_horizon);
            continue;
        };
        let localized = ExactUtcRange {
            start: start.max(exact_horizon.start),
            end: end.min(exact_horizon.end),
        };
        if localized.start < localized.end {
            unknown_coverage_ranges.push(localized);
        }
    }
    let unknown_coverage_ranges = summary_projection_merge_exact_ranges(unknown_coverage_ranges);
    info!(
        stage = "overflowed_boundary_manifest_coverage_summary",
        unknown_range_count = unknown_coverage_ranges.len(),
        horizon_fallback_count,
        "summary projection overflowed boundary manifest coverage summarized"
    );
    Ok(Some(SummaryProjectionOverflowedBoundaryCoverage {
        high_watermark_id,
        unknown_coverage_ranges,
    }))
}

#[derive(Debug, Default)]
struct SummaryProjectionOverflowedBoundaryUnprovenRanges {
    global: Vec<ExactUtcRange>,
    account: Vec<ExactUtcRange>,
    broad_fail_closed: bool,
}

/// Return finite global and account ranges whose compact/V2 authority is not yet complete when
/// the boundary manifest cardinality is over the Bootstrap admission bound.  The optional
/// in-memory rollup maps let Bootstrap distinguish a missing compact key from a merely old
/// manifest without opening any archive source; the overlay rebuild uses metadata-only mode.
type SummaryProjectionBoundaryUnprovenRow = (
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn load_summary_projection_boundary_unproven_rows(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
) -> Result<Vec<SummaryProjectionBoundaryUnprovenRow>> {
    sqlx::query_as::<_, SummaryProjectionBoundaryUnprovenRow>(
        "SELECT month_key, file_path, \
                COALESCE(coverage_start_epoch, CAST(strftime('%s', coverage_start_at) AS INTEGER)), \
                COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)), \
                coverage_start_at, coverage_end_at, \
                historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1 \
           AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
           AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
           AND NOT EXISTS ( \
               SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
               WHERE proof.archive_batch_id = batches.id \
                 AND batches.sha256 IS NOT NULL \
                 AND TRIM(batches.sha256) <> '' \
                 AND proof.manifest_sha256 = batches.sha256 \
           ) \
         LIMIT ?4",
    )
    .bind(high_watermark_id)
    .bind(exact_horizon.start.timestamp())
    .bind(exact_horizon.end.timestamp())
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection overflowed boundary unproven-range hydration failed")
}

fn summary_projection_boundary_row_range(
    month_key: &str,
    coverage_start_epoch: Option<i64>,
    coverage_end_epoch: Option<i64>,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    exact_horizon: ExactUtcRange,
) -> (ExactUtcRange, bool) {
    let range = match (
        coverage_start_at.and_then(crate::stats::parse_to_utc_datetime),
        coverage_end_at.and_then(crate::stats::parse_to_utc_datetime),
    ) {
        (Some(start), Some(end)) if start <= end => ExactUtcRange {
            start,
            end: if start == end {
                end.checked_add_signed(ChronoDuration::seconds(1))
                    .unwrap_or(end)
            } else {
                end
            },
        },
        _ => match (coverage_start_epoch, coverage_end_epoch) {
            (Some(start_epoch), Some(end_epoch)) => match (
                Utc.timestamp_opt(start_epoch, 0).single(),
                Utc.timestamp_opt(end_epoch, 0).single(),
            ) {
                (Some(start), Some(end)) if start <= end => ExactUtcRange {
                    start,
                    end: end
                        .checked_add_signed(ChronoDuration::seconds(1))
                        .unwrap_or(end),
                },
                _ => return (exact_horizon, true),
            },
            _ => {
                let Some(buckets) = crate::stats::archive_bucket_start_epochs_from_bounds(
                    Some(month_key),
                    None,
                    None,
                )
                .ok()
                .filter(|buckets| !buckets.is_empty()) else {
                    return (exact_horizon, true);
                };
                let start_epoch = *buckets.iter().min().expect("non-empty archive buckets");
                let end_epoch = buckets
                    .iter()
                    .max()
                    .copied()
                    .unwrap_or(start_epoch)
                    .saturating_add(3_600);
                let (Some(start), Some(end)) = (
                    Utc.timestamp_opt(start_epoch, 0).single(),
                    Utc.timestamp_opt(end_epoch, 0).single(),
                ) else {
                    return (exact_horizon, true);
                };
                ExactUtcRange { start, end }
            }
        },
    };
    (
        ExactUtcRange {
            start: range.start.max(exact_horizon.start),
            end: range.end.min(exact_horizon.end),
        },
        false,
    )
}

struct SummaryProjectionBoundaryScopeCoverage<'a> {
    replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    account_manifest: &'a HashMap<String, HashSet<i64>>,
    rollup_totals: Option<&'a HashMap<(i64, Option<i64>), StatsTotals>>,
    rollup_usage: Option<&'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>>,
}

fn summary_projection_boundary_scope_gaps(
    localized: ExactUtcRange,
    file_path: &str,
    historical_rollups_materialized_at: Option<&str>,
    upstream_activity_manifest_refreshed_at: Option<&str>,
    coverage: &SummaryProjectionBoundaryScopeCoverage<'_>,
) -> (bool, bool) {
    let replay = coverage
        .replay_coverage
        .get(file_path)
        .copied()
        .unwrap_or_default();
    let mut global_gap = !replay.overall || !replay.usage_breakdown;
    let mut account_gap = upstream_activity_manifest_refreshed_at.is_none()
        || !replay.account_stats
        || !replay.usage_breakdown;
    if coverage.rollup_totals.is_none() || coverage.rollup_usage.is_none() {
        global_gap |= historical_rollups_materialized_at.is_none();
    }
    if let (Some(rollup_totals), Some(rollup_usage)) =
        (coverage.rollup_totals, coverage.rollup_usage)
    {
        let mut bucket = align_bucket_epoch(localized.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(localized.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            global_gap |= !rollup_totals.contains_key(&(bucket, None))
                || !rollup_usage.contains_key(&(bucket, None));
            if let Some(account_ids) = coverage.account_manifest.get(file_path) {
                account_gap |= account_ids.iter().any(|account_id| {
                    !rollup_totals.contains_key(&(bucket, Some(*account_id)))
                        || !rollup_usage.contains_key(&(bucket, Some(*account_id)))
                });
            }
            if global_gap && account_gap {
                break;
            }
            bucket = bucket.saturating_add(3_600);
        }
    }
    (global_gap, account_gap)
}

struct SummaryProjectionBoundaryRangeSummary {
    global: Vec<ExactUtcRange>,
    account: Vec<ExactUtcRange>,
    horizon_fallback_count: usize,
    broad_fail_closed: bool,
}

fn summarize_summary_projection_boundary_rows(
    exact_horizon: ExactUtcRange,
    rows: &[SummaryProjectionBoundaryUnprovenRow],
    coverage: &SummaryProjectionBoundaryScopeCoverage<'_>,
) -> SummaryProjectionBoundaryRangeSummary {
    let mut global = Vec::new();
    let mut account = Vec::new();
    let mut horizon_fallback_count = 0usize;
    let mut broad_fail_closed = false;
    for (
        month_key,
        file_path,
        coverage_start_epoch,
        coverage_end_epoch,
        coverage_start_at,
        coverage_end_at,
        historical_rollups_materialized_at,
        upstream_activity_manifest_refreshed_at,
    ) in rows
    {
        let (range, fallback) = summary_projection_boundary_row_range(
            month_key,
            *coverage_start_epoch,
            *coverage_end_epoch,
            coverage_start_at.as_deref(),
            coverage_end_at.as_deref(),
            exact_horizon,
        );
        if fallback {
            horizon_fallback_count = horizon_fallback_count.saturating_add(1);
            global.push(exact_horizon);
            account.push(exact_horizon);
            broad_fail_closed = true;
            continue;
        }
        if range.start >= range.end {
            continue;
        }
        let (global_gap, account_gap) = summary_projection_boundary_scope_gaps(
            range,
            file_path,
            historical_rollups_materialized_at.as_deref(),
            upstream_activity_manifest_refreshed_at.as_deref(),
            coverage,
        );
        if global_gap {
            global.push(range);
            broad_fail_closed = true;
        }
        if account_gap {
            account.push(range);
        }
    }
    SummaryProjectionBoundaryRangeSummary {
        global: summary_projection_merge_exact_ranges(global),
        account: summary_projection_merge_exact_ranges(account),
        horizon_fallback_count,
        broad_fail_closed,
    }
}

async fn load_summary_projection_overflowed_scope_flags(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
) -> Result<(bool, bool)> {
    let (global, account) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT \
             EXISTS(SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
               AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
                   WHERE proof.archive_batch_id = batches.id \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND proof.manifest_sha256 = batches.sha256 \
               ) \
               AND (batches.historical_rollups_materialized_at IS NULL \
                    OR NOT EXISTS (SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                                   WHERE replay.target = ?4 AND replay.dataset = batches.dataset \
                                     AND replay.file_path = batches.file_path \
                                     AND replay.archive_sha256 = batches.sha256) \
                    OR NOT EXISTS (SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                                   WHERE replay.target = ?5 AND replay.dataset = batches.dataset \
                                     AND replay.file_path = batches.file_path \
                                     AND replay.archive_sha256 = batches.sha256))), \
             EXISTS(SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
               AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
                   WHERE proof.archive_batch_id = batches.id \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND proof.manifest_sha256 = batches.sha256 \
               ) \
               AND (batches.upstream_activity_manifest_refreshed_at IS NULL \
                    OR NOT EXISTS (SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                                   WHERE replay.target = ?6 AND replay.dataset = batches.dataset \
                                     AND replay.file_path = batches.file_path \
                                     AND replay.archive_sha256 = batches.sha256) \
                    OR NOT EXISTS (SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                                   WHERE replay.target = ?5 AND replay.dataset = batches.dataset \
                                     AND replay.file_path = batches.file_path \
                                     AND replay.archive_sha256 = batches.sha256)))",
    )
    .bind(high_watermark_id)
    .bind(exact_horizon.start.timestamp())
    .bind(exact_horizon.end.timestamp())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .fetch_one(pool)
    .await
    .context("summary projection overflowed boundary unproven aggregate lookup failed")?;
    Ok((global != 0, account != 0))
}

async fn summarize_overflowed_boundary_unproven_ranges(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
    rollup_totals: Option<&HashMap<(i64, Option<i64>), StatsTotals>>,
    rollup_usage: Option<&HashMap<(i64, Option<i64>), UsageBreakdownResponse>>,
    rows: &[SummaryProjectionBoundaryUnprovenRow],
) -> Result<SummaryProjectionOverflowedBoundaryUnprovenRanges> {
    let paths = rows
        .iter()
        .map(|(_, file_path, _, _, _, _, _, _)| file_path.clone())
        .collect::<Vec<_>>();
    let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
    let account_manifest = load_summary_projection_archive_manifest_accounts(pool, &paths)
        .await?
        .into_iter()
        .fold(
            HashMap::<String, HashSet<i64>>::new(),
            |mut by_file, (path, id)| {
                by_file.entry(path).or_default().insert(id);
                by_file
            },
        );
    let coverage = SummaryProjectionBoundaryScopeCoverage {
        replay_coverage: &replay_coverage,
        account_manifest: &account_manifest,
        rollup_totals,
        rollup_usage,
    };
    let summary = summarize_summary_projection_boundary_rows(exact_horizon, rows, &coverage);
    let (unproven_manifest_exists, unproven_account_manifest_exists) =
        load_summary_projection_overflowed_scope_flags(pool, exact_horizon, high_watermark_id)
            .await?;
    let global = summary.global;
    let mut account = summary.account;
    let broad_fail_closed = summary.broad_fail_closed || unproven_manifest_exists;
    if unproven_account_manifest_exists {
        account.push(exact_horizon);
    }
    info!(
        stage = "overflowed_boundary_unproven_range_budget",
        source_row_count = rows.len(),
        "summary projection metadata scan exceeded its bounded archive budget; retaining a bounded unavailable range"
    );
    Ok(SummaryProjectionOverflowedBoundaryUnprovenRanges {
        global,
        account,
        broad_fail_closed,
    })
}

async fn summary_projection_overflowed_boundary_unproven_ranges_scoped(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
    rollup_totals: Option<&HashMap<(i64, Option<i64>), StatsTotals>>,
    rollup_usage: Option<&HashMap<(i64, Option<i64>), UsageBreakdownResponse>>,
) -> Result<SummaryProjectionOverflowedBoundaryUnprovenRanges> {
    let rows =
        load_summary_projection_boundary_unproven_rows(pool, exact_horizon, high_watermark_id)
            .await?;

    if rows.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        return summarize_overflowed_boundary_unproven_ranges(
            pool,
            exact_horizon,
            high_watermark_id,
            rollup_totals,
            rollup_usage,
            &rows,
        )
        .await;
    }

    let paths = rows
        .iter()
        .map(|(_, file_path, _, _, _, _, _, _)| file_path.clone())
        .collect::<Vec<_>>();
    let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
    let account_manifest = load_summary_projection_archive_manifest_accounts(pool, &paths)
        .await?
        .into_iter()
        .fold(
            HashMap::<String, HashSet<i64>>::new(),
            |mut by_file, (path, id)| {
                by_file.entry(path).or_default().insert(id);
                by_file
            },
        );
    let coverage = SummaryProjectionBoundaryScopeCoverage {
        replay_coverage: &replay_coverage,
        account_manifest: &account_manifest,
        rollup_totals,
        rollup_usage,
    };
    let summary = summarize_summary_projection_boundary_rows(exact_horizon, &rows, &coverage);
    info!(
        stage = "overflowed_boundary_unproven_range_summary",
        source_row_count = rows.len(),
        horizon_fallback_count = summary.horizon_fallback_count,
        global_range_count = summary.global.len(),
        account_range_count = summary.account.len(),
        "summary projection overflowed boundary ranges summarized"
    );
    Ok(SummaryProjectionOverflowedBoundaryUnprovenRanges {
        global: summary.global,
        account: summary.account,
        broad_fail_closed: false,
    })
}

async fn summary_projection_overflowed_all_time_manifests_have_complete_rollups(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    let Some(high_watermark_id) =
        summary_projection_completed_manifest_high_watermark(pool).await?
    else {
        return Ok(None);
    };
    let incomplete = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS( \
             SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND ( \
                    batches.coverage_start_at IS NULL \
                    OR batches.coverage_end_at IS NULL \
                    OR batches.historical_rollups_materialized_at IS NULL \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?2 \
                    ) \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?3 \
                    ) \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?4 \
                    ) \
               ) \
         )",
    )
    .bind(high_watermark_id)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .context("summary projection overflowed all-time manifest coverage hydration failed")?;
    Ok((incomplete == 0).then_some(high_watermark_id))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SummaryProjectionBoundaryManifestPageRow {
    id: i64,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    historical_rollups_materialized_at: Option<String>,
    upstream_activity_manifest_refreshed_at: Option<String>,
}

struct SummaryProjectionBoundaryManifestPage {
    archives: Vec<crate::stats::ArchiveBatchPathRow>,
    account_manifest_refreshed_paths: HashSet<String>,
    next_after_id: Option<i64>,
}

async fn load_summary_projection_boundary_manifest_page(
    pool: &Pool<Sqlite>,
    range: Option<ExactUtcRange>,
    after_id: Option<i64>,
    high_watermark_id: i64,
) -> Result<SummaryProjectionBoundaryManifestPage> {
    load_summary_projection_boundary_manifest_page_with_limit(
        pool,
        range,
        after_id,
        high_watermark_id,
        SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE,
    )
    .await
}

async fn load_summary_projection_boundary_manifest_page_with_limit(
    pool: &Pool<Sqlite>,
    range: Option<ExactUtcRange>,
    after_id: Option<i64>,
    high_watermark_id: i64,
    page_size: usize,
) -> Result<SummaryProjectionBoundaryManifestPage> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, file_path, coverage_start_at, coverage_end_at, \
                historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at \
         FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    );
    if let Some(after_id) = after_id {
        query.push(" AND id > ").push_bind(after_id);
    }
    query.push(" AND id <= ").push_bind(high_watermark_id);
    if let Some(range) = range {
        query
            .push(" AND coverage_end_at >= ")
            .push_bind(db_occurred_at_upper_bound(range.start))
            .push(" AND coverage_start_at < ")
            .push_bind(crate::stats::db_occurred_at_lower_bound(range.end));
    }
    let rows = query
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind(page_size as i64)
        .build_query_as::<SummaryProjectionBoundaryManifestPageRow>()
        .fetch_all(pool)
        .await
        .context("summary projection boundary manifest page hydration failed")?;
    let next_after_id = rows.last().map(|row| row.id);
    let account_manifest_refreshed_paths = rows
        .iter()
        .filter(|row| row.upstream_activity_manifest_refreshed_at.is_some())
        .map(|row| row.file_path.clone())
        .collect();
    let archives = rows
        .into_iter()
        .map(|row| {
            crate::stats::ArchiveBatchPathRow::with_coverage_and_historical_rollups(
                row.file_path,
                row.coverage_start_at,
                row.coverage_end_at,
                row.historical_rollups_materialized_at,
            )
        })
        .collect();
    Ok(SummaryProjectionBoundaryManifestPage {
        archives,
        account_manifest_refreshed_paths,
        next_after_id,
    })
}

async fn load_summary_projection_archive_manifest_account_sets(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, HashSet<i64>>> {
    let mut account_ids_by_file = archive_paths
        .iter()
        .cloned()
        .map(|path| (path, HashSet::new()))
        .collect::<HashMap<_, _>>();
    for (file_path, account_id) in
        load_summary_projection_archive_manifest_accounts(pool, archive_paths).await?
    {
        if let Some(account_ids) = account_ids_by_file.get_mut(&file_path) {
            account_ids.insert(account_id);
        }
    }
    Ok(account_ids_by_file)
}

fn summary_projection_all_time_manifest_scope_coverage(
    archive: &crate::stats::ArchiveBatchPathRow,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_manifest_complete: bool,
    account_ids: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool)> {
    let Some(range) = summary_projection_archive_coverage_range(archive) else {
        return Ok((false, false));
    };
    if summary_projection_exact_range_fits_bucket_budget(range).is_err() {
        return Ok((false, false));
    }
    // Global totals include usage/cost dimensions. An invocation replay marker alone cannot
    // prove the complete StatsResponse, so keep the archive unavailable until its usage replay
    // (or a verified V2 replacement) is present as well.
    let mut global_covered = replay_coverage.overall && replay_coverage.usage_breakdown;
    // Account-scoped totals expose the same usage/cost dimensions as the global response. An
    // account replay marker without its usage-breakdown proof must therefore remain unavailable.
    let mut accounts_covered = replay_coverage.account_stats
        && replay_coverage.usage_breakdown
        && account_manifest_complete;
    let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
    while bucket <= last_bucket {
        global_covered &= hourly_rollup_totals.contains_key(&(bucket, None));
        accounts_covered &= account_ids
            .iter()
            .all(|account_id| hourly_rollup_totals.contains_key(&(bucket, Some(*account_id))));
        if !global_covered && !accounts_covered {
            break;
        }
        bucket = bucket.saturating_add(3_600);
    }
    Ok((global_covered, accounts_covered))
}

async fn summary_projection_paged_all_time_materialized_scope_coverage(
    pool: &Pool<Sqlite>,
    high_watermark_id: i64,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool, HashSet<i64>)> {
    let mut global_covered = true;
    let mut accounts_covered = true;
    let mut discovered_account_ids = HashSet::new();
    let mut after_id = None;
    loop {
        let page =
            load_summary_projection_boundary_manifest_page(pool, None, after_id, high_watermark_id)
                .await?;
        if page.archives.is_empty() {
            break;
        }
        let paths = page
            .archives
            .iter()
            .map(|archive| archive.file_path().to_string())
            .collect::<Vec<_>>();
        let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
        let account_ids_by_file =
            match load_summary_projection_archive_manifest_account_sets(pool, &paths).await {
                Ok(account_ids_by_file) => account_ids_by_file,
                Err(error)
                    if error.to_string().starts_with(
                        "summary projection archive account manifest exceeded bounded row budget",
                    ) =>
                {
                    accounts_covered = false;
                    paths
                        .iter()
                        .cloned()
                        .map(|path| (path, HashSet::new()))
                        .collect()
                }
                Err(error) => return Err(error),
            };
        for archive in &page.archives {
            let account_ids = account_ids_by_file
                .get(archive.file_path())
                .expect("page account manifest includes every requested archive path");
            if accounts_covered {
                discovered_account_ids.extend(account_ids.iter().copied());
                if discovered_account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
                    accounts_covered = false;
                    discovered_account_ids.clear();
                }
            }
            let replay = replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default();
            let (archive_global_covered, archive_accounts_covered) =
                summary_projection_all_time_manifest_scope_coverage(
                    archive,
                    replay,
                    page.account_manifest_refreshed_paths
                        .contains(archive.file_path()),
                    account_ids,
                    hourly_rollup_totals,
                )?;
            global_covered &= archive_global_covered;
            accounts_covered &= archive_accounts_covered;
        }
        let Some(next_after_id) = page.next_after_id else {
            break;
        };
        after_id = Some(next_after_id);
    }
    Ok((global_covered, accounts_covered, discovered_account_ids))
}

async fn load_summary_projection_durable_account_ids(pool: &Pool<Sqlite>) -> Result<HashSet<i64>> {
    let pool_rows =
        sqlx::query_scalar::<_, i64>("SELECT id FROM pool_upstream_accounts WHERE id > 0 LIMIT ?1")
            .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
            .fetch_all(pool)
            .await
            .context("summary projection account hydration failed")?;
    let rollup_rows = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT upstream_account_id \
         FROM upstream_account_stats_hourly \
         WHERE upstream_account_id > 0 \
         LIMIT ?1",
    )
    .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection historical account hydration failed")?;
    if pool_rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS
        || rollup_rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS
    {
        return Err(anyhow!(
            "summary projection durable account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(pool_rows.into_iter().chain(rollup_rows).collect())
}

async fn restore_summary_projection_persisted_statuses(
    connection: &mut SqliteConnection,
    rows: &mut [UpstreamAccountInvocationPreviewRow],
) -> Result<()> {
    const STATUS_HYDRATION_CHUNK_SIZE: usize = 500;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let mut statuses = HashMap::<i64, Option<String>>::new();
    for chunk in ids.chunks(STATUS_HYDRATION_CHUNK_SIZE) {
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT id, status FROM codex_invocations WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
        }
        query.push(")");
        statuses.extend(
            query
                .build_query_as::<(i64, Option<String>)>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    for row in rows {
        if let Some(status) = statuses.get(&row.id).and_then(|status| status.clone()) {
            row.status = status;
        }
    }
    Ok(())
}

fn summary_projection_live_record_from_preview(
    row: UpstreamAccountInvocationPreviewRow,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
) -> Option<(String, SummaryProjectionRecord)> {
    let occurred_at = parse_to_utc_datetime(&row.occurred_at)?;
    let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
    let global_rollup_covered =
        row.id <= rollup_live_cursor && hourly_rollup_totals.contains_key(&(bucket, None));
    let account_rollup_covered = account_rollup_live_cursor.is_some_and(|cursor| row.id <= cursor)
        && hourly_rollup_totals.contains_key(&(bucket, row.upstream_account_id));
    let usage_global_rollup_covered =
        row.id <= rollup_live_cursor && hourly_rollup_usage.contains_key(&(bucket, None));
    let usage_account_rollup_covered = account_rollup_live_cursor
        .is_some_and(|cursor| row.id <= cursor)
        && hourly_rollup_usage.contains_key(&(bucket, row.upstream_account_id));
    Some((
        row.invoke_id.clone(),
        SummaryProjectionRecord {
            global_rollup_covered,
            account_rollup_covered,
            usage_global_rollup_covered,
            usage_account_rollup_covered,
            is_persisted_live_record: true,
            is_archive_record: false,
            archive_has_materialized_rollups: false,
            account_archive_totals_fallback_included: false,
            row,
            occurred_at,
        },
    ))
}

#[derive(Clone, Default)]
struct PreviousSummaryProjectionAllTime {
    all_time_by_account: HashMap<Option<i64>, StatsResponse>,
    all_time_refreshed_at: Option<Instant>,
    all_time_manifest_admission_blocked_at: Option<Instant>,
    all_time_account_manifest_admission_blocked_at: Option<Instant>,
    all_time_account_refreshed_at: HashMap<i64, Instant>,
    archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    global_all_time_eligible: bool,
    account_all_time_eligible: HashSet<i64>,
    all_time_terminal_coverage_complete: bool,
    global_all_time_coverage_fence: Option<SummaryCoverageFence>,
    account_all_time_coverage_fence: Option<SummaryCoverageFence>,
    all_time_terminal_sequence_watermark: u64,
    all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
    historical_live_coverage: Option<SummaryProjectionHistoricalLiveCoverage>,
    coverage_overlay: Option<SummaryCoverageOverlay>,
    unavailable_exact_live_ranges: Vec<ExactUtcRange>,
    unavailable_exact_live_account_ranges: HashMap<i64, Vec<ExactUtcRange>>,
    unavailable_unmaterialized_archive_ranges: Vec<ExactUtcRange>,
    unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    persisted_live_terminal_invoke_ids: HashSet<String>,
}
