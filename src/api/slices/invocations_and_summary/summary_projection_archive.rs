async fn load_summary_projection_unrepresented_current_archive_coverage(
    pool: &Pool<Sqlite>,
    represented_archive_paths: &HashSet<String>,
    exact_horizon: ExactUtcRange,
) -> Result<(Vec<ExactUtcRange>, Option<DateTime<Utc>>, bool)> {
    // A current selection can only ignore an archive after its complete bounded coverage has
    // been materialized into the resident candidate view. Keep the exclusion bind set small;
    // a larger fully represented set is still safe, but its SQL proof must fail closed rather
    // than exceed SQLite's parameter budget.
    if represented_archive_paths.len() > SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE {
        return Ok((Vec::new(), None, true));
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT month_key, coverage_start_epoch, coverage_end_epoch \
         FROM archive_batches WHERE dataset = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .push(" AND status = ")
        .push_bind(ARCHIVE_STATUS_COMPLETED)
        .push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    if !represented_archive_paths.is_empty() {
        query.push(" AND file_path NOT IN (");
        let mut separated = query.separated(", ");
        for path in represented_archive_paths {
            separated.push_bind(path);
        }
        separated.push_unseparated(")");
    }
    let rows: Vec<(String, Option<i64>, Option<i64>)> =
        query.build_query_as().fetch_all(pool).await?;
    let source_row_count = rows.len();
    let (finite_ranges, latest_coverage_end, has_unknown_coverage, horizon_fallback_count) =
        summarize_unrepresented_archive_coverage(rows, exact_horizon);
    info!(
        stage = "unrepresented_current_archive_coverage_summary",
        source_row_count,
        range_count = finite_ranges.len(),
        horizon_fallback_count,
        has_unknown_coverage,
        "summary projection unrepresented current archive coverage summarized"
    );
    Ok((
        summary_projection_merge_exact_ranges(finite_ranges),
        latest_coverage_end,
        has_unknown_coverage,
    ))
}

fn summarize_unrepresented_archive_coverage(
    rows: Vec<(String, Option<i64>, Option<i64>)>,
    exact_horizon: ExactUtcRange,
) -> (Vec<ExactUtcRange>, Option<DateTime<Utc>>, bool, usize) {
    let mut finite_ranges = Vec::new();
    let mut has_unknown_coverage = false;
    let mut latest_coverage_end = None;
    let mut horizon_fallback_count = 0usize;
    for (month_key, start_epoch, end_epoch) in rows {
        let known_range = match (start_epoch, end_epoch) {
            (Some(start_epoch), Some(end_epoch)) => {
                let start = Utc.timestamp_opt(start_epoch, 0).single();
                let end = Utc.timestamp_opt(end_epoch, 0).single();
                match (start, end) {
                    (Some(start), Some(end)) if start <= end => Some((
                        start,
                        end.checked_add_signed(ChronoDuration::seconds(1))
                            .unwrap_or(end),
                    )),
                    _ => {
                        horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some((start, end)) = known_range {
            latest_coverage_end = latest_coverage_end.max(Some(end));
            let localized = ExactUtcRange {
                start: start.max(exact_horizon.start),
                end: end.min(exact_horizon.end),
            };
            if localized.start < localized.end {
                finite_ranges.push(localized);
            }
            continue;
        }

        // A parseable month key gives a finite conservative range that the request path can
        // compare with its actual Nth current row. A malformed identity remains broad
        // fail-closed because no temporal boundary can be proven.
        let Some(buckets) =
            crate::stats::archive_bucket_start_epochs_from_bounds(Some(&month_key), None, None)
                .ok()
                .filter(|buckets| !buckets.is_empty())
        else {
            has_unknown_coverage = true;
            continue;
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
            has_unknown_coverage = true;
            continue;
        };
        let localized = ExactUtcRange {
            start: start.max(exact_horizon.start),
            end: end.min(exact_horizon.end),
        };
        if localized.start < localized.end {
            finite_ranges.push(localized);
        }
        latest_coverage_end = latest_coverage_end.max(Some(end));
    }
    (
        finite_ranges,
        latest_coverage_end,
        has_unknown_coverage,
        horizon_fallback_count,
    )
}

async fn load_summary_projection_current_archive_admission(
    pool: &Pool<Sqlite>,
    current_start: DateTime<Utc>,
) -> Result<Vec<crate::stats::ArchiveBatchPathRow>> {
    // Current is ordered by recency, not archive creation order. Admit the newest bounded
    // manifest prefix so a large backlog of old materialized batches cannot hide a current
    // candidate that must be hydrated off-request.
    let rows = sqlx::query_as::<_, crate::stats::ArchiveBatchPathRow>(
        "SELECT file_path, month_key, coverage_start_at, coverage_end_at, \
         historical_rollups_materialized_at, NULL AS needs_overall, NULL AS needs_failures \
         FROM archive_batches \
         WHERE dataset = ?1 AND status = ?2 \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
           AND (COALESCE(coverage_start_epoch, CAST(strftime('%s', coverage_start_at) AS INTEGER)) IS NULL \
                OR COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) IS NULL \
                OR COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) >= ?3) \
         ORDER BY COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) DESC, id DESC \
         LIMIT ?4",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(current_start.timestamp())
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES.saturating_add(1)) as i64)
    .fetch_all(pool)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(rows)
}

fn summary_projection_exact_bucket_ranges(buckets: &HashSet<i64>) -> Vec<ExactUtcRange> {
    summary_projection_merge_exact_ranges(
        buckets
            .iter()
            .filter_map(|bucket| {
                let start = Utc.timestamp_opt(*bucket, 0).single()?;
                let end = Utc
                    .timestamp_opt(bucket.saturating_add(3_600), 0)
                    .single()?;
                Some(ExactUtcRange { start, end })
            })
            .collect(),
    )
}

fn summary_projection_ensure_exact_bucket_budget(buckets: &HashSet<i64>) -> Result<()> {
    if buckets.len() > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
        return Err(anyhow!(
            "summary projection exact bucket budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
        ));
    }
    Ok(())
}

fn summary_projection_exact_range_fits_bucket_budget(range: ExactUtcRange) -> Result<()> {
    let first_bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
    let bucket_count = last_bucket
        .saturating_sub(first_bucket)
        .saturating_div(3_600)
        .saturating_add(1) as usize;
    if bucket_count > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
        return Err(anyhow!(
            "summary projection exact range bucket budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
        ));
    }
    Ok(())
}

fn summary_projection_extend_exact_buckets_for_ranges(
    buckets: &mut HashSet<i64>,
    ranges: impl IntoIterator<Item = ExactUtcRange>,
) -> Result<()> {
    for range in ranges {
        summary_projection_exact_range_fits_bucket_budget(range)?;
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(buckets)?;
            bucket = bucket.saturating_add(3_600);
        }
    }
    Ok(())
}

fn summary_projection_mark_unavailable_archive_ranges(
    buckets: &mut BTreeSet<i64>,
    ranges: impl IntoIterator<Item = ExactUtcRange>,
) -> Result<()> {
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        let first_bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        let bucket_count = last_bucket
            .saturating_sub(first_bucket)
            .saturating_div(3_600)
            .saturating_add(1) as usize;
        if bucket_count > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
            return Err(anyhow!(
                "summary projection unavailable archive range budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
            ));
        }
        let mut bucket = first_bucket;
        while bucket <= last_bucket {
            if !buckets.contains(&bucket) && buckets.len() >= SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
                return Err(anyhow!(
                    "summary projection unavailable archive range budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
                ));
            }
            buckets.insert(bucket);
            bucket = bucket.saturating_add(3_600);
        }
    }
    Ok(())
}

fn summary_projection_admit_paged_boundary_raw_archive(
    admitted_archives: &mut usize,
) -> Result<()> {
    if *admitted_archives >= SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        return Err(anyhow!(
            "summary projection paged boundary raw archive admission exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES})"
        ));
    }
    *admitted_archives += 1;
    Ok(())
}

fn summary_projection_unavailable_bucket_ranges(buckets: BTreeSet<i64>) -> Vec<ExactUtcRange> {
    let mut ranges = Vec::<ExactUtcRange>::new();
    for bucket in buckets {
        let Some(start) = Utc.timestamp_opt(bucket, 0).single() else {
            continue;
        };
        let end = start + ChronoDuration::hours(1);
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

fn summary_projection_split_ranges_by_buckets(
    ranges: impl IntoIterator<Item = ExactUtcRange>,
    selected_buckets: &HashSet<i64>,
) -> (Vec<ExactUtcRange>, Vec<ExactUtcRange>) {
    let mut selected = Vec::new();
    let mut deferred = Vec::new();
    for range in ranges {
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            let bucket_start = Utc
                .timestamp_opt(bucket, 0)
                .single()
                .expect("valid summary archive bucket start");
            let segment = ExactUtcRange {
                start: range.start.max(bucket_start),
                end: range.end.min(bucket_start + ChronoDuration::hours(1)),
            };
            if segment.start < segment.end {
                if selected_buckets.contains(&bucket) {
                    selected.push(segment);
                } else {
                    deferred.push(segment);
                }
            }
            bucket = bucket.saturating_add(3_600);
        }
    }
    (
        summary_projection_merge_exact_ranges(selected),
        summary_projection_merge_exact_ranges(deferred),
    )
}

fn summary_projection_partial_rollup_ranges(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<ExactUtcRange> {
    if start >= end {
        return Vec::new();
    }
    let first_full_hour = Utc
        .timestamp_opt(ceil_hour_epoch(start.timestamp()), 0)
        .single()
        .expect("valid summary partial range start");
    let last_full_hour = Utc
        .timestamp_opt(align_bucket_epoch(end.timestamp(), 3_600, 0), 0)
        .single()
        .expect("valid summary partial range end");
    let mut partial_ranges = Vec::new();
    if start < first_full_hour {
        partial_ranges.push(ExactUtcRange {
            start,
            end: end.min(first_full_hour),
        });
    }
    if first_full_hour <= last_full_hour && last_full_hour < end {
        partial_ranges.push(ExactUtcRange {
            start: last_full_hour,
            end,
        });
    }
    partial_ranges
}

fn summary_projection_mark_unavailable_archive_ranges_by_requirement(
    unavailable_ranges: &mut BTreeSet<i64>,
    unavailable_boundary_ranges: &mut Vec<ExactUtcRange>,
    archive_has_materialized_rollups: bool,
    exact_ranges: &[ExactUtcRange],
    exact_bucket_requirements: &HashSet<i64>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return summary_projection_mark_unavailable_archive_ranges(
            unavailable_ranges,
            exact_ranges.iter().copied(),
        );
    }

    let mut strict_ranges = Vec::new();
    let mut boundary_ranges = Vec::new();
    for range in exact_ranges {
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            let bucket_start = Utc
                .timestamp_opt(bucket, 0)
                .single()
                .expect("valid summary archive bucket start");
            let bucket_end = bucket_start + ChronoDuration::hours(1);
            let segment = ExactUtcRange {
                start: range.start.max(bucket_start),
                end: range.end.min(bucket_end),
            };
            if segment.start < segment.end {
                if exact_bucket_requirements.contains(&bucket) {
                    strict_ranges.push(segment);
                } else {
                    boundary_ranges.push(segment);
                }
            }
            bucket = bucket.saturating_add(3_600);
        }
    }
    summary_projection_mark_unavailable_archive_ranges(unavailable_ranges, strict_ranges)?;
    unavailable_boundary_ranges.extend(boundary_ranges);
    Ok(())
}

struct SummaryProjectionExactReplacementInput<'a> {
    archive_has_materialized_rollups: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    exact_range: ExactUtcRange,
    _protected_boundary_buckets: &'a HashSet<i64>,
    exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
}

fn summary_projection_mark_exact_replacement_buckets(
    SummaryProjectionExactReplacementInput {
        archive_has_materialized_rollups,
        replay_coverage,
        exact_range,
        _protected_boundary_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
    }: SummaryProjectionExactReplacementInput<'_>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return Ok(());
    }
    // All-time aggregation consumes whole compact buckets. Any archive which lacks durable
    // replay proof therefore replaces every bucket it intersects, including a partial archive
    // start/end hour. The builder hydrates every archive/live contribution for that bucket
    // before suppressing its compact aggregate.
    let first_bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp().saturating_sub(1), 3_600, 0);
    summary_projection_exact_range_fits_bucket_budget(exact_range)?;
    let mut bucket = first_bucket;
    while bucket <= last_bucket {
        if !replay_coverage.overall {
            exact_global_total_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_global_total_rollup_buckets)?;
        }
        if !replay_coverage.account_stats {
            exact_account_total_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_account_total_rollup_buckets)?;
        }
        if !replay_coverage.usage_breakdown {
            exact_global_usage_rollup_buckets.insert(bucket);
            exact_account_usage_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_global_usage_rollup_buckets)?;
            summary_projection_ensure_exact_bucket_budget(exact_account_usage_rollup_buckets)?;
        }
        bucket = bucket.saturating_add(3_600);
    }
    Ok(())
}

fn summary_projection_mark_account_exact_replacement_buckets(
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    exact_account_total_rollup_buckets: &mut HashSet<i64>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return Ok(());
    }
    // A stale account manifest makes the account compact aggregate unverifiable even when the
    // global replay is complete. The paged raw fallback owns this whole bucket for account
    // totals, while the independently replay-proven account usage rollup remains canonical.
    summary_projection_exact_range_fits_bucket_budget(exact_range)?;
    let mut bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp().saturating_sub(1), 3_600, 0);
    while bucket <= last_bucket {
        exact_account_total_rollup_buckets.insert(bucket);
        summary_projection_ensure_exact_bucket_budget(exact_account_total_rollup_buckets)?;
        bucket = bucket.saturating_add(3_600);
    }
    Ok(())
}

struct SummaryProjectionArchiveRecordsInput<'a> {
    pool: &'a Pool<Sqlite>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &'a HashSet<i64>,
    usage_rollup_cursor: Option<i64>,
    known_account_ids: &'a HashSet<i64>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
}

struct SummaryProjectionArchiveMergeInput<'a> {
    archive_pool: &'a Pool<Sqlite>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    archive_has_materialized_rollups: bool,
    overall_rollup_archive_replayed: Option<bool>,
    account_rollup_archive_replayed: Option<bool>,
    usage_rollup_archive_replayed: Option<bool>,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &'a HashSet<i64>,
    usage_rollup_cursor: Option<i64>,
    known_account_ids: &'a HashSet<i64>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    resident_current_record_bytes: usize,
    snapshot_rows: Option<Vec<SummaryProjectionArchiveRow>>,
}

async fn merge_summary_projection_archive_records(
    input: SummaryProjectionArchiveRecordsInput<'_>,
) -> Result<()> {
    let exact_record_bytes = input
        .records_by_invoke_id
        .values()
        .map(|record| summary_projection_preview_row_bytes(&record.row))
        .sum::<usize>();
    ensure_summary_projection_resident_record_bytes(exact_record_bytes, 0)?;
    let SummaryProjectionArchiveRecordsInput {
        pool,
        persisted_live_ids,
        hourly_rollup_totals,
        hourly_rollup_usage,
        archive_has_materialized_rollups,
        exact_range,
        protected_boundary_buckets,
        usage_rollup_cursor,
        known_account_ids,
        records_by_invoke_id,
        exact_record_budget,
    } = input;
    let mut exact_record_bytes = exact_record_bytes;
    merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
        archive_pool: pool,
        persisted_live_ids,
        hourly_rollup_totals,
        hourly_rollup_usage,
        archive_has_materialized_rollups,
        overall_rollup_archive_replayed: None,
        account_rollup_archive_replayed: None,
        usage_rollup_archive_replayed: None,
        exact_range,
        protected_boundary_buckets,
        usage_rollup_cursor,
        known_account_ids,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes: &mut exact_record_bytes,
        resident_current_record_bytes: 0,
        snapshot_rows: None,
    })
    .await
}

struct SummaryProjectionSnapshotManifest {
    archive_batch_id: i64,
    manifest_sha256: String,
    manifest_row_count: i64,
    manifest_start: Option<DateTime<Utc>>,
    manifest_end: Option<DateTime<Utc>>,
}

/// Load a verified V2 Snapshot page from the primary SQLite database. This is an off-request
/// recovery source used only when the immutable archive file is unavailable; the HTTP/SSE path
/// never calls it.
async fn load_summary_projection_snapshot_records(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
    exact_ranges: &[ExactUtcRange],
) -> Result<Option<Vec<SummaryProjectionArchiveRow>>> {
    let Some(manifest) = load_summary_projection_snapshot_manifest(pool, archive_file_path).await?
    else {
        return Ok(None);
    };
    let pages = sqlx::query(
        "SELECT page_index, coverage_start, coverage_end, row_count, payload, payload_bytes, \
                snapshot_sha256, format_version \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
    )
    .bind(manifest.archive_batch_id)
    .bind(&manifest.manifest_sha256)
    .fetch_all(pool)
    .await
    .context("summary archive Snapshot page lookup failed")?;
    if pages.is_empty() {
        return Ok(None);
    }
    let Some(records) = validate_summary_projection_snapshot_pages(
        pages,
        manifest.manifest_row_count,
        manifest.manifest_start,
        manifest.manifest_end,
    ) else {
        return Ok(None);
    };
    Ok(Some(summary_projection_snapshot_rows(
        records,
        exact_ranges,
    )))
}

async fn load_summary_projection_snapshot_manifest(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
) -> Result<Option<SummaryProjectionSnapshotManifest>> {
    let Some((archive_batch_id, manifest_sha256, manifest_row_count, manifest_start, manifest_end)) =
        sqlx::query_as::<_, (i64, String, i64, Option<String>, Option<String>)>(
            "SELECT id, sha256, row_count, coverage_start_at, coverage_end_at \
             FROM archive_batches WHERE dataset = 'codex_invocations' \
             AND status = 'completed' \
             AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
             AND file_path = ?1 ORDER BY id DESC LIMIT 1",
        )
        .bind(archive_file_path)
        .fetch_optional(pool)
        .await
        .context("summary archive Snapshot identity lookup failed")?
    else {
        return Ok(None);
    };
    if manifest_sha256.trim().is_empty() || manifest_row_count < 0 {
        return Ok(None);
    }
    Ok(Some(SummaryProjectionSnapshotManifest {
        archive_batch_id,
        manifest_sha256,
        manifest_row_count,
        manifest_start: manifest_start.and_then(|value| parse_snapshot_coverage_at(&value)),
        manifest_end: manifest_end.and_then(|value| parse_snapshot_coverage_at(&value)),
    }))
}

fn validate_summary_projection_snapshot_pages(
    pages: Vec<sqlx::sqlite::SqliteRow>,
    manifest_row_count: i64,
    manifest_start: Option<DateTime<Utc>>,
    manifest_end: Option<DateTime<Utc>>,
) -> Option<Vec<SummaryArchiveSnapshotV2Record>> {
    let mut records = Vec::new();
    let mut total_rows = 0_i64;
    let mut first_page_start = None;
    let mut last_page_end = None;
    let mut previous_page_end = None;
    let mut previous_record_key = None;
    let mut seen_ids = HashSet::new();
    for (expected_page, page) in pages.into_iter().enumerate() {
        if page.get::<i64, _>("page_index") != i64::try_from(expected_page).unwrap_or(-1)
            || page.get::<i64, _>("format_version") != SUMMARY_ARCHIVE_SNAPSHOT_V2
        {
            return None;
        }
        let payload = page.get::<Vec<u8>, _>("payload");
        let mut hasher = sha2::Sha256::new();
        hasher.update(&payload);
        if page.get::<String, _>("snapshot_sha256") != format!("{:x}", hasher.finalize())
            || page.get::<i64, _>("payload_bytes") != i64::try_from(payload.len()).unwrap_or(-1)
        {
            return None;
        }
        let decoded = decode_summary_archive_snapshot_v2_payload(&payload).ok()?;
        if i64::try_from(decoded.len()).unwrap_or(-1) != page.get::<i64, _>("row_count") {
            return None;
        }
        let coverage_start = parse_snapshot_coverage_at(page.get("coverage_start"))?;
        let coverage_end = parse_snapshot_coverage_at(page.get("coverage_end"))?;
        if coverage_start > coverage_end
            || previous_page_end.is_some_and(|previous_end| coverage_start < previous_end)
        {
            return None;
        }
        first_page_start.get_or_insert(coverage_start);
        last_page_end = Some(coverage_end);
        previous_page_end = Some(coverage_end);
        total_rows = total_rows.saturating_add(page.get::<i64, _>("row_count"));
        for record in &decoded {
            let occurred_at = parse_snapshot_coverage_at(&record.occurred_at)?;
            if occurred_at < coverage_start
                || occurred_at > coverage_end
                || previous_record_key.is_some_and(|previous| (occurred_at, record.id) < previous)
                || !seen_ids.insert(record.id)
            {
                return None;
            }
            previous_record_key = Some((occurred_at, record.id));
        }
        records.extend(decoded);
    }
    if total_rows != manifest_row_count
        || manifest_start.is_some_and(|start| first_page_start != Some(start))
        || manifest_end.is_some_and(|end| last_page_end != Some(end))
    {
        return None;
    }
    Some(records)
}

fn summary_projection_snapshot_rows(
    records: Vec<SummaryArchiveSnapshotV2Record>,
    exact_ranges: &[ExactUtcRange],
) -> Vec<SummaryProjectionArchiveRow> {
    records
        .into_iter()
        .filter(|record| {
            parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
                exact_ranges
                    .iter()
                    .any(|range| occurred_at >= range.start && occurred_at < range.end)
            })
        })
        .map(|record| SummaryProjectionArchiveRow {
            id: record.id,
            invoke_id: record.invoke_id,
            occurred_at: record.occurred_at,
            source: record.source,
            model: record.model,
            response_model: record.response_model,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_input_tokens: record.cache_input_tokens,
            reasoning_tokens: record.reasoning_tokens,
            reasoning_effort: record.reasoning_effort,
            total_tokens: record.total_tokens,
            cost: record.cost,
            cost_input: record.cost_input,
            cost_cache_write: record.cost_cache_write,
            cost_cache_read: record.cost_cache_read,
            cost_output: record.cost_output,
            cost_reasoning: record.cost_reasoning,
            status: record.status,
            error_message: record.error_message,
            failure_kind: record.failure_kind,
            failure_class: record.failure_class,
            upstream_account_id: record.upstream_account_id,
        })
        .collect()
}

async fn merge_summary_projection_archive_records_with_coverage(
    input: SummaryProjectionArchiveMergeInput<'_>,
) -> Result<()> {
    let SummaryProjectionArchiveMergeInput {
        archive_pool,
        persisted_live_ids,
        hourly_rollup_totals,
        hourly_rollup_usage,
        archive_has_materialized_rollups,
        overall_rollup_archive_replayed,
        account_rollup_archive_replayed,
        usage_rollup_archive_replayed,
        exact_range,
        protected_boundary_buckets,
        usage_rollup_cursor,
        known_account_ids,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes,
        resident_current_record_bytes,
        snapshot_rows,
    } = input;
    let columns = load_summary_projection_archive_columns(archive_pool).await?;
    let exact_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive_has_materialized_rollups,
        SummaryProjectionArchiveReplayFlags {
            overall: overall_rollup_archive_replayed,
            account: account_rollup_archive_replayed,
            usage: usage_rollup_archive_replayed,
        },
        exact_range,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
    );
    let rows = load_summary_projection_archive_rows(SummaryProjectionArchiveRowsInput {
        archive_pool,
        columns: &columns,
        exact_ranges: &exact_ranges,
        archive_has_materialized_rollups,
        resident_current_record_bytes,
        snapshot_rows,
    })
    .await?;
    let full_rollup_start = ceil_hour_epoch(exact_range.start.timestamp());
    let full_rollup_end = align_bucket_epoch(exact_range.end.timestamp(), 3_600, 0);
    merge_summary_projection_archive_rows(SummaryProjectionArchiveRowIterationInput {
        rows,
        persisted_live_ids,
        archive_has_materialized_rollups,
        overall_rollup_archive_replayed,
        account_rollup_archive_replayed,
        usage_rollup_archive_replayed,
        full_rollup_start,
        full_rollup_end,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        usage_rollup_cursor,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes,
        resident_current_record_bytes,
    })
}

struct SummaryProjectionArchiveRowIterationInput<'a> {
    rows: Vec<SummaryProjectionArchiveRow>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    archive_has_materialized_rollups: bool,
    overall_rollup_archive_replayed: Option<bool>,
    account_rollup_archive_replayed: Option<bool>,
    usage_rollup_archive_replayed: Option<bool>,
    full_rollup_start: i64,
    full_rollup_end: i64,
    protected_boundary_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    usage_rollup_cursor: Option<i64>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    resident_current_record_bytes: usize,
}

fn merge_summary_projection_archive_rows(
    input: SummaryProjectionArchiveRowIterationInput<'_>,
) -> Result<()> {
    let SummaryProjectionArchiveRowIterationInput {
        rows,
        persisted_live_ids,
        archive_has_materialized_rollups,
        overall_rollup_archive_replayed,
        account_rollup_archive_replayed,
        usage_rollup_archive_replayed,
        full_rollup_start,
        full_rollup_end,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        usage_rollup_cursor,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes,
        resident_current_record_bytes,
    } = input;
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        // Persisted/live rows are the richer, authoritative copy.
        if persisted_live_ids.contains(&SummarySourceIdentity {
            row_id: row.id,
            invoke_id: row.invoke_id.clone(),
            occurred_at: row.occurred_at.clone(),
        }) || summary_projection_records_contains_identity(
            records_by_invoke_id,
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        ) {
            continue;
        }
        let coverage =
            summary_projection_archive_row_coverage(SummaryProjectionArchiveRowCoverageInput {
                row: &row,
                archive_has_materialized_rollups,
                overall_rollup_archive_replayed,
                account_rollup_archive_replayed,
                usage_rollup_archive_replayed,
                bucket,
                full_rollup_start,
                full_rollup_end,
                protected_boundary_buckets,
                hourly_rollup_totals,
                hourly_rollup_usage,
                usage_rollup_cursor,
            });
        if coverage.bucket_is_full_rollup
            && coverage.global_rollup_covered
            && coverage.account_rollup_covered
            && coverage.usage_global_rollup_covered
            && coverage.usage_account_rollup_covered
        {
            continue;
        }
        merge_summary_projection_archive_row(SummaryProjectionArchiveRowMergeInput {
            row,
            occurred_at,
            coverage,
            archive_has_materialized_rollups,
            records_by_invoke_id,
            exact_record_budget,
            exact_record_bytes,
            resident_current_record_bytes,
        })?;
    }
    Ok(())
}
