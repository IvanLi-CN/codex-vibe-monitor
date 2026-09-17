struct SummaryProjectionExactLiveAdmissionInput<'a> {
    pool: &'a Pool<Sqlite>,
    state: &'a AppState,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    archive_start: DateTime<Utc>,
    live_start: DateTime<Utc>,
    live_high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    exact_archive_buckets: &'a HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    previous_historical_live_coverage: Option<&'a SummaryProjectionHistoricalLiveCoverage>,
    previous_unavailable_exact_live_ranges: &'a [ExactUtcRange],
    previous_unavailable_exact_live_account_ranges: &'a HashMap<i64, Vec<ExactUtcRange>>,
    previous_persisted_live_terminal_invoke_ids: &'a HashSet<String>,
    live_preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    current_record_bytes: usize,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
    unavailable_exact_live_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    historical_global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
    known_account_ids: &'a mut HashSet<i64>,
}

struct SummaryProjectionExactLiveAdmission {
    exact_bucket_requirements: HashSet<i64>,
    historical_live_coverage: Option<SummaryProjectionHistoricalLiveCoverage>,
    persisted_live_ids: HashSet<SummarySourceIdentity>,
    exact_record_budget: usize,
}

struct SummaryProjectionHistoricalLiveTailInput<'a> {
    pool: &'a Pool<Sqlite>,
    range: ExactUtcRange,
    live_high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    live_preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
    recent_index_complete: bool,
}

struct SummaryProjectionHistoricalLiveTailMergeInput<'a> {
    admission: SummaryProjectionLiveAdmission,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
}

struct SummaryProjectionExactLiveRangesInput<'a> {
    pool: &'a Pool<Sqlite>,
    live_high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    ranges: Vec<ExactUtcRange>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    live_preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    current_record_bytes: usize,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
    fully_admitted_historical_live_buckets: &'a mut BTreeSet<i64>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a HashSet<i64>,
}

struct SummaryProjectionExactLiveRangeInput<'a> {
    pool: &'a Pool<Sqlite>,
    range: ExactUtcRange,
    live_high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    live_preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    current_record_bytes: usize,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a HashSet<i64>,
}

struct SummaryProjectionHistoricalLiveBuildCoverageInput<'a> {
    pool: &'a Pool<Sqlite>,
    state: &'a AppState,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    range: ExactUtcRange,
    high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    fully_admitted_live_buckets: &'a BTreeSet<i64>,
    previous_coverage: Option<&'a SummaryProjectionHistoricalLiveCoverage>,
    previous_unavailable_global_ranges: &'a [ExactUtcRange],
    previous_unavailable_account_ranges: &'a HashMap<i64, Vec<ExactUtcRange>>,
    previous_terminal_identities: &'a HashSet<String>,
    unavailable_global_buckets: &'a mut BTreeSet<i64>,
    unavailable_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
}

async fn admit_summary_projection_exact_live_sources(
    input: SummaryProjectionExactLiveAdmissionInput<'_>,
) -> Result<SummaryProjectionExactLiveAdmission> {
    let exact_bucket_requirements = prepare_summary_projection_exact_live_bucket_requirements(
        SummaryProjectionExactLiveBucketRequirementsInput {
            exact_archive_buckets: input.exact_archive_buckets,
            protected_boundary_buckets: input.protected_boundary_buckets,
            exact_global_total_rollup_buckets: input.exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets: input.exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets: input.exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets: input.exact_account_usage_rollup_buckets,
            records_by_invoke_id: input.records_by_invoke_id,
        },
    );
    let exact_live_ranges =
        summary_projection_exact_live_ranges(input.exact_archive_buckets, input.live_start);
    let historical_live_range = ExactUtcRange {
        start: input.archive_start,
        end: input.live_start,
    };
    let mut fully_admitted_historical_live_buckets =
        admit_summary_projection_historical_live_tail(SummaryProjectionHistoricalLiveTailInput {
            pool: input.pool,
            range: historical_live_range,
            live_high_watermark_id: input.live_high_watermark_id,
            rollup_live_cursor: input.rollup_live_cursor,
            account_rollup_live_cursor: input.account_rollup_live_cursor,
            hourly_rollup_totals: input.hourly_rollup_totals,
            hourly_rollup_usage: input.hourly_rollup_usage,
            live_preview_cache: input.live_preview_cache,
            records_by_invoke_id: input.records_by_invoke_id,
            exact_record_bytes: input.exact_record_bytes,
            unavailable_exact_live_buckets: input.unavailable_exact_live_buckets,
            recent_index_complete: input.recent_index_complete,
        })
        .await?;
    admit_summary_projection_exact_live_ranges(SummaryProjectionExactLiveRangesInput {
        pool: input.pool,
        live_high_watermark_id: input.live_high_watermark_id,
        rollup_live_cursor: input.rollup_live_cursor,
        account_rollup_live_cursor: input.account_rollup_live_cursor,
        hourly_rollup_totals: input.hourly_rollup_totals,
        hourly_rollup_usage: input.hourly_rollup_usage,
        live_preview_cache: input.live_preview_cache,
        records_by_invoke_id: input.records_by_invoke_id,
        exact_record_bytes: input.exact_record_bytes,
        current_record_bytes: input.current_record_bytes,
        ranges: exact_live_ranges,
        recent_index_overflow_at: input.recent_index_overflow_at,
        unavailable_exact_live_buckets: input.unavailable_exact_live_buckets,
        fully_admitted_historical_live_buckets: &mut fully_admitted_historical_live_buckets,
        exact_global_total_rollup_buckets: input.exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets: input.exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets: input.exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets: input.exact_account_usage_rollup_buckets,
    })
    .await?;
    let historical_live_coverage = load_summary_projection_historical_live_coverage(
        SummaryProjectionHistoricalLiveBuildCoverageInput {
            pool: input.pool,
            state: input.state,
            mode: input.mode,
            build_started_at: input.build_started_at,
            range: historical_live_range,
            high_watermark_id: input.live_high_watermark_id,
            rollup_live_cursor: input.rollup_live_cursor,
            account_rollup_live_cursor: input.account_rollup_live_cursor,
            hourly_rollup_totals: input.hourly_rollup_totals,
            hourly_rollup_usage: input.hourly_rollup_usage,
            fully_admitted_live_buckets: &fully_admitted_historical_live_buckets,
            previous_coverage: input.previous_historical_live_coverage,
            previous_unavailable_global_ranges: input.previous_unavailable_exact_live_ranges,
            previous_unavailable_account_ranges: input
                .previous_unavailable_exact_live_account_ranges,
            previous_terminal_identities: input.previous_persisted_live_terminal_invoke_ids,
            unavailable_global_buckets: input.unavailable_exact_live_buckets,
            unavailable_account_buckets: input.unavailable_exact_live_account_buckets,
            global_covered_terminal_invoke_ids: input.historical_global_covered_terminal_invoke_ids,
        },
    )
    .await?;
    let (persisted_live_ids, exact_record_budget) =
        finalize_summary_projection_exact_live_admission(
            input.records_by_invoke_id,
            input.known_account_ids,
        )?;
    Ok(SummaryProjectionExactLiveAdmission {
        exact_bucket_requirements,
        historical_live_coverage,
        persisted_live_ids,
        exact_record_budget,
    })
}

fn summary_projection_exact_live_ranges(
    exact_archive_buckets: &HashSet<i64>,
    live_start: DateTime<Utc>,
) -> Vec<ExactUtcRange> {
    summary_projection_exact_bucket_ranges(exact_archive_buckets)
        .into_iter()
        .filter_map(|range| {
            (range.start < live_start).then_some(ExactUtcRange {
                start: range.start,
                end: range.end.min(live_start),
            })
        })
        .collect()
}

struct SummaryProjectionExactLiveBucketRequirementsInput<'a> {
    exact_archive_buckets: &'a HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a HashSet<i64>,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
}

fn prepare_summary_projection_exact_live_bucket_requirements(
    input: SummaryProjectionExactLiveBucketRequirementsInput<'_>,
) -> HashSet<i64> {
    let SummaryProjectionExactLiveBucketRequirementsInput {
        exact_archive_buckets,
        protected_boundary_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
        records_by_invoke_id,
    } = input;
    let mut requirements = exact_archive_buckets
        .difference(protected_boundary_buckets)
        .copied()
        .collect::<HashSet<_>>();
    requirements.extend(exact_global_total_rollup_buckets.iter().copied());
    requirements.extend(exact_account_total_rollup_buckets.iter().copied());
    requirements.extend(exact_global_usage_rollup_buckets.iter().copied());
    requirements.extend(exact_account_usage_rollup_buckets.iter().copied());
    for record in records_by_invoke_id.values_mut() {
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        record.global_rollup_covered &= !exact_global_total_rollup_buckets.contains(&bucket);
        record.account_rollup_covered &= !exact_account_total_rollup_buckets.contains(&bucket);
        record.usage_global_rollup_covered &= !exact_global_usage_rollup_buckets.contains(&bucket);
        record.usage_account_rollup_covered &=
            !exact_account_usage_rollup_buckets.contains(&bucket);
    }
    requirements
}

fn finalize_summary_projection_exact_live_admission(
    records_by_invoke_id: &HashMap<String, SummaryProjectionRecord>,
    known_account_ids: &mut HashSet<i64>,
) -> Result<(HashSet<SummarySourceIdentity>, usize)> {
    known_account_ids.extend(
        records_by_invoke_id
            .values()
            .filter_map(|record| record.row.upstream_account_id)
            .filter(|account_id| *account_id > 0),
    );
    if records_by_invoke_id.len() > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection exact archive/live replacement exceeded bounded record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    let persisted_live_ids = records_by_invoke_id
        .values()
        .filter(|record| record.is_persisted_live_record)
        .map(summary_projection_record_source_identity)
        .collect::<HashSet<_>>();
    Ok((persisted_live_ids, records_by_invoke_id.len()))
}

async fn admit_summary_projection_historical_live_tail(
    input: SummaryProjectionHistoricalLiveTailInput<'_>,
) -> Result<BTreeSet<i64>> {
    let SummaryProjectionHistoricalLiveTailInput {
        pool,
        range,
        live_high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        live_preview_cache,
        records_by_invoke_id,
        exact_record_bytes,
        unavailable_exact_live_buckets,
        recent_index_complete,
    } = input;
    if range.start >= range.end
        || rollup_live_cursor >= live_high_watermark_id
        || !recent_index_complete
    {
        return Ok(BTreeSet::new());
    }
    let remaining = summary_projection_exact_record_limit()
        .saturating_sub(records_by_invoke_id.len())
        .saturating_add(1);
    let tail_admission =
        query_summary_projection_live_rows_with_budget(SummaryProjectionLiveRowsQuery {
            pool,
            source_scope: InvocationSourceScope::All,
            range,
            high_watermark_id: live_high_watermark_id,
            min_id_exclusive: Some(rollup_live_cursor),
            upstream_account_id: None,
            limit: remaining,
            in_progress_only: false,
            preview_cache: live_preview_cache,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "summary_projection",
                builder: "historical_live_tail",
                purpose: "summary_projection_historical_live_tail",
            },
        })
        .await?;
    merge_summary_projection_historical_live_tail(SummaryProjectionHistoricalLiveTailMergeInput {
        admission: tail_admission,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        records_by_invoke_id,
        exact_record_bytes,
        unavailable_exact_live_buckets,
    })
}

fn merge_summary_projection_historical_live_tail(
    input: SummaryProjectionHistoricalLiveTailMergeInput<'_>,
) -> Result<BTreeSet<i64>> {
    let SummaryProjectionHistoricalLiveTailMergeInput {
        admission,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        records_by_invoke_id,
        exact_record_bytes,
        unavailable_exact_live_buckets,
    } = input;
    let mut complete = admission.overflow.is_none() && admission.gaps.is_empty();
    for candidate in admission.gaps.into_iter().chain(admission.overflow) {
        if let Some(bucket) = summary_projection_live_candidate_bucket(&candidate) {
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_exact_live_buckets,
                [summary_projection_exact_hour_range(bucket)],
            )?;
        }
        complete = false;
    }
    let mut buckets = BTreeSet::new();
    for row in admission.rows {
        let Some((_record_key, record)) = summary_projection_live_record_from_preview(
            row,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
        ) else {
            complete = false;
            continue;
        };
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        let replaced_bytes = summary_projection_record_bytes_for_identity(
            records_by_invoke_id,
            record.row.id,
            &record.row.invoke_id,
            &record.row.occurred_at,
        );
        let next_record_bytes = exact_record_bytes
            .saturating_sub(replaced_bytes)
            .saturating_add(summary_projection_preview_row_bytes(&record.row));
        if let Err(error) = ensure_summary_projection_resident_record_bytes(next_record_bytes, 0) {
            if !summary_projection_resident_record_budget_exceeded(&error) {
                return Err(error);
            }
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_exact_live_buckets,
                [summary_projection_exact_hour_range(bucket)],
            )?;
            complete = false;
            continue;
        }
        *exact_record_bytes = next_record_bytes;
        let record_key = summary_projection_record_insert_key(records_by_invoke_id, &record);
        records_by_invoke_id.insert(record_key, record);
        buckets.insert(bucket);
    }
    if complete {
        Ok(buckets)
    } else {
        Ok(BTreeSet::new())
    }
}

fn summary_projection_exact_hour_range(bucket: i64) -> ExactUtcRange {
    ExactUtcRange {
        start: Utc
            .timestamp_opt(bucket, 0)
            .single()
            .expect("valid summary projection hour bucket start"),
        end: Utc
            .timestamp_opt(bucket.saturating_add(3_600), 0)
            .single()
            .expect("valid summary projection hour bucket end"),
    }
}

async fn admit_summary_projection_exact_live_ranges(
    input: SummaryProjectionExactLiveRangesInput<'_>,
) -> Result<()> {
    let SummaryProjectionExactLiveRangesInput {
        pool,
        live_high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        ranges,
        hourly_rollup_totals,
        hourly_rollup_usage,
        live_preview_cache,
        records_by_invoke_id,
        exact_record_bytes,
        current_record_bytes,
        recent_index_overflow_at,
        unavailable_exact_live_buckets,
        fully_admitted_historical_live_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
    } = input;
    for range in ranges {
        if recent_index_overflow_at.is_some_and(|overflow_at| range.start <= overflow_at) {
            continue;
        }
        if admit_summary_projection_exact_live_range(SummaryProjectionExactLiveRangeInput {
            pool,
            range,
            live_high_watermark_id,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
            live_preview_cache,
            records_by_invoke_id,
            exact_record_bytes,
            current_record_bytes,
            unavailable_exact_live_buckets,
            exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets,
        })
        .await?
        {
            let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
            let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
            while bucket <= last_bucket {
                fully_admitted_historical_live_buckets.insert(bucket);
                bucket = bucket.saturating_add(3_600);
            }
        }
    }
    Ok(())
}

async fn admit_summary_projection_exact_live_range(
    input: SummaryProjectionExactLiveRangeInput<'_>,
) -> Result<bool> {
    let SummaryProjectionExactLiveRangeInput {
        pool,
        range,
        live_high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        live_preview_cache,
        records_by_invoke_id,
        exact_record_bytes,
        current_record_bytes,
        unavailable_exact_live_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
    } = input;
    let remaining = summary_projection_exact_record_limit()
        .saturating_sub(records_by_invoke_id.len())
        .saturating_add(1);
    let exact_live_admission =
        query_summary_projection_live_rows_with_budget(SummaryProjectionLiveRowsQuery {
            pool,
            source_scope: InvocationSourceScope::All,
            range,
            high_watermark_id: live_high_watermark_id,
            min_id_exclusive: None,
            upstream_account_id: None,
            limit: remaining,
            in_progress_only: false,
            preview_cache: live_preview_cache,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "summary_projection",
                builder: "exact_replacement_bucket",
                purpose: "summary_projection_archive_live_replacement",
            },
        })
        .await?;
    if exact_live_admission.overflow.is_some() || !exact_live_admission.gaps.is_empty() {
        summary_projection_mark_unavailable_archive_ranges(
            unavailable_exact_live_buckets,
            [range],
        )?;
        return Ok(false);
    }
    let mut range_fully_admitted = true;
    for row in exact_live_admission.rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            range_fully_admitted = false;
            break;
        };
        let replacement_bytes = summary_projection_preview_row_bytes(&row);
        let replaced_bytes = summary_projection_record_bytes_for_identity(
            records_by_invoke_id,
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        );
        let next_record_bytes = exact_record_bytes
            .saturating_sub(replaced_bytes)
            .saturating_add(replacement_bytes);
        if let Err(error) =
            ensure_summary_projection_resident_record_bytes(next_record_bytes, current_record_bytes)
        {
            if !summary_projection_resident_record_budget_exceeded(&error) {
                return Err(error);
            }
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_exact_live_buckets,
                [range],
            )?;
            range_fully_admitted = false;
            break;
        }
        *exact_record_bytes = next_record_bytes;
        let record = summary_projection_exact_live_record(SummaryProjectionExactLiveRecordInput {
            row,
            occurred_at,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
            exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets,
        });
        let record_key = summary_projection_record_insert_key(records_by_invoke_id, &record);
        records_by_invoke_id.insert(record_key, record);
    }
    Ok(range_fully_admitted)
}

struct SummaryProjectionExactLiveRecordInput<'a> {
    row: UpstreamAccountInvocationPreviewRow,
    occurred_at: DateTime<Utc>,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a HashSet<i64>,
}

fn summary_projection_exact_live_record(
    input: SummaryProjectionExactLiveRecordInput<'_>,
) -> SummaryProjectionRecord {
    let SummaryProjectionExactLiveRecordInput {
        row,
        occurred_at,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
    } = input;
    let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
    SummaryProjectionRecord {
        global_rollup_covered: row.id <= rollup_live_cursor
            && hourly_rollup_totals.contains_key(&(bucket, None))
            && !exact_global_total_rollup_buckets.contains(&bucket),
        account_rollup_covered: account_rollup_live_cursor.is_some_and(|cursor| row.id <= cursor)
            && hourly_rollup_totals.contains_key(&(bucket, row.upstream_account_id))
            && !exact_account_total_rollup_buckets.contains(&bucket),
        usage_global_rollup_covered: row.id <= rollup_live_cursor
            && hourly_rollup_usage.contains_key(&(bucket, None))
            && !exact_global_usage_rollup_buckets.contains(&bucket),
        usage_account_rollup_covered: account_rollup_live_cursor
            .is_some_and(|cursor| row.id <= cursor)
            && hourly_rollup_usage.contains_key(&(bucket, row.upstream_account_id))
            && !exact_account_usage_rollup_buckets.contains(&bucket),
        is_persisted_live_record: true,
        is_archive_record: false,
        archive_has_materialized_rollups: false,
        account_archive_totals_fallback_included: false,
        row,
        occurred_at,
    }
}

async fn load_summary_projection_historical_live_coverage(
    input: SummaryProjectionHistoricalLiveBuildCoverageInput<'_>,
) -> Result<Option<SummaryProjectionHistoricalLiveCoverage>> {
    if input.range.start >= input.range.end {
        return Ok(None);
    }
    if input.mode.requires_full_historical_live_coverage() {
        load_full_summary_projection_historical_live_coverage(input).await
    } else {
        reuse_summary_projection_historical_live_coverage_for_build(input).await
    }
}

async fn load_full_summary_projection_historical_live_coverage(
    input: SummaryProjectionHistoricalLiveBuildCoverageInput<'_>,
) -> Result<Option<SummaryProjectionHistoricalLiveCoverage>> {
    let SummaryProjectionHistoricalLiveBuildCoverageInput {
        pool,
        state,
        mode,
        build_started_at,
        range,
        high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        fully_admitted_live_buckets,
        previous_coverage: _,
        previous_unavailable_global_ranges: _,
        previous_unavailable_account_ranges: _,
        previous_terminal_identities: _,
        unavailable_global_buckets,
        unavailable_account_buckets,
        global_covered_terminal_invoke_ids,
    } = input;
    info!(
        ?mode,
        stage = "historical_live_coverage",
        "summary projection build stage started"
    );
    #[cfg(test)]
    pause_summary_projection_test_interleave(
        pool,
        mode,
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
    )
    .await?;
    let pending_terminal_identities = state
        .subscription_hub
        .summary_terminal_overlay_identities()
        .await;
    mark_summary_projection_uncovered_historical_live_ranges(
        SummaryProjectionHistoricalLiveCoverageInput {
            pool,
            range,
            high_watermark_id,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
            fully_admitted_live_buckets,
            pending_terminal_identities: &pending_terminal_identities,
            unavailable_global_buckets,
            unavailable_account_buckets,
            global_covered_terminal_invoke_ids,
        },
    )
    .await?;
    let reconciliation_required = summary_projection_historical_live_reconciliation_required(
        range,
        unavailable_global_buckets,
        unavailable_account_buckets,
    );
    info!(
        ?mode,
        stage = "historical_live_coverage",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(Some(SummaryProjectionHistoricalLiveCoverage {
        range,
        high_watermark_id,
        reconciliation_required,
    }))
}

async fn reuse_summary_projection_historical_live_coverage_for_build(
    input: SummaryProjectionHistoricalLiveBuildCoverageInput<'_>,
) -> Result<Option<SummaryProjectionHistoricalLiveCoverage>> {
    let SummaryProjectionHistoricalLiveBuildCoverageInput {
        pool,
        state,
        mode,
        build_started_at,
        range,
        high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        fully_admitted_live_buckets,
        previous_coverage,
        previous_unavailable_global_ranges,
        previous_unavailable_account_ranges,
        previous_terminal_identities,
        unavailable_global_buckets,
        unavailable_account_buckets,
        global_covered_terminal_invoke_ids,
    } = input;
    let pending_terminal_identities = state
        .subscription_hub
        .summary_terminal_overlay_identities()
        .await;
    let mut coverage = reuse_summary_projection_historical_live_coverage(
        SummaryProjectionHistoricalLiveReuseInput {
            pool,
            previous_coverage,
            range,
            high_watermark_id,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
            fully_admitted_live_buckets,
            previous_unavailable_global_ranges,
            previous_unavailable_account_ranges,
            previous_terminal_identities,
            pending_terminal_identities: &pending_terminal_identities,
            unavailable_global_buckets,
            unavailable_account_buckets,
            global_covered_terminal_invoke_ids,
        },
    )
    .await?;
    coverage.reconciliation_required |= summary_projection_historical_live_reconciliation_required(
        range,
        unavailable_global_buckets,
        unavailable_account_buckets,
    );
    info!(
        ?mode,
        stage = "historical_live_coverage_reuse",
        reconciliation_required = coverage.reconciliation_required,
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection historical live coverage reused"
    );
    Ok(Some(coverage))
}

fn summary_projection_historical_live_reconciliation_required(
    range: ExactUtcRange,
    unavailable_global_buckets: &BTreeSet<i64>,
    unavailable_account_buckets: &HashMap<i64, BTreeSet<i64>>,
) -> bool {
    unavailable_global_buckets
        .iter()
        .any(|bucket| summary_projection_bucket_intersects_range(*bucket, range))
        || unavailable_account_buckets.values().any(|buckets| {
            buckets
                .iter()
                .any(|bucket| summary_projection_bucket_intersects_range(*bucket, range))
        })
}
