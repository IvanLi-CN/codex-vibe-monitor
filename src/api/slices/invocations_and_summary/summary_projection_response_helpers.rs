struct SummaryProjectionQuery {
    window: SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    now: DateTime<Utc>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    requested_current_limit: Option<usize>,
}

fn parse_summary_projection_query(
    projection: &SummaryProjection,
    params: &SummaryQuery,
    default_limit: i64,
) -> Result<SummaryProjectionQuery, ApiError> {
    validate_summary_projection_window(params, default_limit)?;
    let rolling_refreshed_at = projection.rolling_refreshed_at();
    let last_good_age_ms =
        rolling_refreshed_at.map(|refreshed_at| refreshed_at.elapsed().as_millis() as u64);
    debug!(
        ?last_good_age_ms,
        revision = projection.revision,
        "serving summary projection from memory"
    );
    let window = parse_summary_window(params, default_limit)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let upstream_account_id = match params.upstream_account_id {
        Some(account_id) if account_id > 0 => Some(account_id),
        Some(_) => return Err(ApiError::bad_request(anyhow!("invalid upstream account"))),
        None => None,
    };
    if matches!(window, SummaryWindow::All)
        && upstream_account_id.is_some()
        && projection
            .all_time_account_manifest_admission_blocked_at
            .is_some()
        && !projection
            .all_time_by_account
            .contains_key(&upstream_account_id)
    {
        return Err(ApiError::unavailable(anyhow!(
            "summary all-time account manifest admission is unavailable"
        )));
    }
    let now = Utc::now();
    let range = summary_window_range(&window, reporting_tz, now)?;
    let requested_current_limit = match window {
        SummaryWindow::Current(limit) => Some(limit.max(0) as usize),
        _ => None,
    };
    Ok(SummaryProjectionQuery {
        window,
        reporting_tz,
        upstream_account_id,
        now,
        range,
        requested_current_limit,
    })
}

fn ensure_summary_projection_archive_sources(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
) -> Result<(), ApiError> {
    let affects_global = match query.range {
        Some((start, end)) => {
            projection
                .unavailable_unmaterialized_archive_ranges
                .iter()
                .any(|unavailable| unavailable.start < end && start < unavailable.end)
                || projection.unavailable_boundary_archive_may_affect_range(start, end)
        }
        None => query
            .requested_current_limit
            .is_some_and(|limit| projection.unavailable_archive_may_affect_global_current(limit)),
    };
    if affects_global {
        let message = if query.range.is_none() {
            "summary projection current source is unavailable"
        } else {
            "summary projection archive source is unavailable for the requested range"
        };
        return Err(ApiError::unavailable(anyhow!(message)));
    }
    let affects_account = query
        .upstream_account_id
        .is_some_and(|account_id| match query.range {
            Some((start, end)) => {
                projection
                    .unavailable_unmaterialized_archive_account_ranges
                    .iter()
                    .any(|unavailable| unavailable.start < end && start < unavailable.end)
                    || projection.unavailable_account_boundary_archive_may_affect_range(start, end)
            }
            None => query.requested_current_limit.is_some_and(|limit| {
                projection.unavailable_archive_may_affect_account_current(account_id, limit)
            }),
        });
    if affects_account {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection account archive source is unavailable for the requested range"
        )));
    }
    if query.range.is_some_and(|(start, end)| {
        !projection.freshness.global_all_time_eligible
            && projection
                .generation_fence
                .completed_manifest_high_watermark_id
                .is_some()
            && projection.coverage_overlay.is_none()
            && end - start >= ChronoDuration::days(30)
    }) {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection historical archive coverage is not yet proven for the requested range"
        )));
    }
    Ok(())
}

fn ensure_summary_projection_current_sources(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
) -> Result<(), ApiError> {
    if query.range.is_none()
        && matches!(query.window, SummaryWindow::Current(_))
        && (projection.current_source_unavailable
            || (query.upstream_account_id.is_none()
                && (projection
                    .current_source_unavailable_from_rank
                    .is_some_and(|rank| {
                        query
                            .requested_current_limit
                            .is_some_and(|limit| limit >= rank)
                    })
                    || query.requested_current_limit.is_some_and(|limit| {
                        projection.current_archive_may_affect_global_current(limit)
                    })))
            || (query.upstream_account_id.is_some()
                && (projection.current_account_source_unavailable
                    || query.upstream_account_id.is_some_and(|account_id| {
                        projection
                            .current_account_source_unavailable_from_rank
                            .get(&account_id)
                            .is_some_and(|rank| {
                                query
                                    .requested_current_limit
                                    .is_some_and(|limit| limit >= *rank)
                            })
                    }))))
    {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection current source is unavailable"
        )));
    }
    if query.range.is_some_and(|(start, end)| {
        projection
            .unavailable_exact_live_ranges
            .iter()
            .any(|unavailable| unavailable.start < end && start < unavailable.end)
    }) {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection persisted-live source is unavailable for the requested range"
        )));
    }
    if let Some(account_id) = query.upstream_account_id
        && query.range.is_some_and(|(start, end)| {
            projection
                .unavailable_exact_live_account_ranges
                .get(&account_id)
                .is_some_and(|ranges| {
                    ranges
                        .iter()
                        .any(|unavailable| unavailable.start < end && start < unavailable.end)
                })
        })
    {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection persisted-live account source is unavailable for the requested range"
        )));
    }
    Ok(())
}

fn ensure_summary_projection_freshness(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
    rolling_delta_is_exact: bool,
) -> Result<(), ApiError> {
    if !rolling_delta_is_exact
        && !matches!(query.window, SummaryWindow::All)
        && projection
            .rolling_refreshed_at()
            .is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE)
    {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection last-good snapshot exceeded the freshness budget"
        )));
    }
    Ok(())
}

fn response_for_summary_all_time_query(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
) -> Result<Option<StatsResponse>, ApiError> {
    if !matches!(query.window, SummaryWindow::All) {
        return Ok(None);
    }
    let account_has_projection_data = query.upstream_account_id.is_some_and(|account_id| {
        projection
            .all_time_account_ids_with_projection_data
            .contains(&account_id)
    });
    let refreshed_at = projection.freshness.all_time_at(
        match query.upstream_account_id {
            None => projection.all_time_refreshed_at,
            Some(_) if !account_has_projection_data => projection.all_time_refreshed_at,
            Some(account_id) => projection
                .all_time_account_refreshed_at
                .get(&account_id)
                .copied(),
        },
        account_has_projection_data
            .then_some(query.upstream_account_id)
            .flatten(),
    );
    if refreshed_at.is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE) {
        return Err(ApiError::unavailable(anyhow!(
            "summary all-time last-good snapshot exceeded the freshness budget"
        )));
    }
    if let Some(response) = projection
        .all_time_by_account
        .get(&query.upstream_account_id)
    {
        return Ok(Some(response.clone()));
    }
    if let Some(account_id) = query.upstream_account_id {
        if !account_has_projection_data && !projection.known_account_ids.contains(&account_id) {
            return Ok(Some(
                projection.empty_all_time_account_response(Some(account_id)),
            ));
        }
        return Err(ApiError::unavailable(anyhow!(
            "summary all-time account snapshot is not hydrated"
        )));
    }
    Ok(None)
}

struct SummaryProjectionCandidateRows<'a> {
    indexes: std::borrow::Cow<'a, [usize]>,
    records: &'a [SummaryProjectionRecord],
    current_limit: usize,
    full_rollup_range: Option<(i64, i64)>,
}

fn summary_projection_candidate_rows<'a>(
    projection: &'a SummaryProjection,
    query: &SummaryProjectionQuery,
) -> Result<SummaryProjectionCandidateRows<'a>, ApiError> {
    let (indexes, records) = match query.range {
        None if matches!(query.window, SummaryWindow::Current(_)) => (
            std::borrow::Cow::Borrowed(
                projection
                    .recent_indexes
                    .get(&query.upstream_account_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ),
            projection.current_records.as_slice(),
        ),
        None => (
            std::borrow::Cow::Owned((0..projection.records.len()).collect::<Vec<_>>()),
            projection.records.as_slice(),
        ),
        Some((start, end)) => (
            std::borrow::Cow::Owned(
                projection
                    .hourly_buckets
                    .range(
                        align_bucket_epoch(start.timestamp(), 3_600, 0)
                            ..=align_bucket_epoch(end.timestamp(), 3_600, 0),
                    )
                    .flat_map(|(_, indexes)| indexes.iter().copied())
                    .collect(),
            ),
            projection.records.as_slice(),
        ),
    };
    let current_limit = query.requested_current_limit.unwrap_or(usize::MAX);
    if query.upstream_account_id.is_some()
        && query.range.is_none()
        && !projection.recent_index_complete
        && indexes.len() < current_limit
    {
        return Err(ApiError::unavailable(anyhow!(
            "summary current account snapshot is outside the bounded recent index"
        )));
    }
    let full_rollup_range = query.range.and_then(|(start, end)| {
        let first_full_hour = ceil_hour_epoch(start.timestamp());
        let end_full_hour = align_bucket_epoch(end.timestamp(), 3_600, 0);
        (first_full_hour < end_full_hour).then_some((first_full_hour, end_full_hour))
    });
    Ok(SummaryProjectionCandidateRows {
        indexes,
        records,
        current_limit,
        full_rollup_range,
    })
}

fn summary_projection_record_matches_query(
    record: &SummaryProjectionRecord,
    query: &SummaryProjectionQuery,
) -> bool {
    query
        .upstream_account_id
        .is_none_or(|account_id| record.row.upstream_account_id == Some(account_id))
        && query
            .range
            .is_none_or(|(start, end)| record.occurred_at >= start && record.occurred_at < end)
}

fn select_summary_projection_total_records<'a>(
    projection: &'a SummaryProjection,
    query: &SummaryProjectionQuery,
    candidates: &'a SummaryProjectionCandidateRows<'a>,
) -> Vec<&'a SummaryProjectionRecord> {
    let exact_buckets = match query.upstream_account_id {
        None => &projection.exact_global_total_rollup_buckets,
        Some(_) => &projection.exact_account_total_rollup_buckets,
    };
    candidates
        .indexes
        .iter()
        .copied()
        .take(candidates.current_limit)
        .map(|index| &candidates.records[index])
        .filter(|record| summary_projection_record_matches_query(record, query))
        .filter(|record| {
            let Some((start, end)) = candidates.full_rollup_range else {
                return true;
            };
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if bucket < start || bucket >= end {
                return true;
            }
            if record.is_archive_record
                && let Some(overlay) = &projection.coverage_overlay
            {
                let owns = match query.upstream_account_id {
                    None => {
                        overlay.global_coverage_buckets.contains(&bucket)
                            && overlay.global_by_bucket.contains_key(&bucket)
                    }
                    Some(account_id) => {
                        overlay.account_coverage_buckets.contains(&bucket)
                            && overlay
                                .account_by_bucket
                                .contains_key(&(bucket, account_id))
                    }
                };
                if owns && !exact_buckets.contains(&bucket) {
                    return false;
                }
            }
            if record.is_persisted_live_record {
                let covered = query
                    .upstream_account_id
                    .map_or(record.global_rollup_covered, |_| {
                        record.account_rollup_covered
                    });
                if !covered {
                    return true;
                }
            }
            !query
                .upstream_account_id
                .map_or(record.global_rollup_covered, |_| {
                    record.account_rollup_covered
                })
        })
        .collect()
}

fn select_summary_projection_usage_records<'a>(
    projection: &'a SummaryProjection,
    query: &SummaryProjectionQuery,
    candidates: &'a SummaryProjectionCandidateRows<'a>,
) -> Vec<&'a SummaryProjectionRecord> {
    let exact_buckets = match query.upstream_account_id {
        None => &projection.exact_global_usage_rollup_buckets,
        Some(_) => &projection.exact_account_usage_rollup_buckets,
    };
    candidates
        .indexes
        .iter()
        .copied()
        .take(candidates.current_limit)
        .map(|index| &candidates.records[index])
        .filter(|record| summary_projection_record_matches_query(record, query))
        .filter(|record| {
            let Some((start, end)) = candidates.full_rollup_range else {
                return true;
            };
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if bucket < start || bucket >= end {
                return true;
            }
            if record.is_archive_record
                && let Some(overlay) = &projection.coverage_overlay
            {
                let owns = match query.upstream_account_id {
                    None => {
                        overlay.global_coverage_buckets.contains(&bucket)
                            && overlay.global_usage_by_bucket.contains_key(&bucket)
                    }
                    Some(account_id) => {
                        overlay.account_coverage_buckets.contains(&bucket)
                            && overlay
                                .account_usage_by_bucket
                                .contains_key(&(bucket, account_id))
                    }
                };
                if owns && !exact_buckets.contains(&bucket) {
                    return false;
                }
            }
            if record.is_persisted_live_record {
                let covered = query
                    .upstream_account_id
                    .map_or(record.usage_global_rollup_covered, |_| {
                        record.usage_account_rollup_covered
                    });
                if !covered {
                    return true;
                }
            }
            let usage_covered = query
                .upstream_account_id
                .map_or(record.usage_global_rollup_covered, |_| {
                    record.usage_account_rollup_covered
                })
                && projection
                    .hourly_rollup_usage
                    .contains_key(&(bucket, query.upstream_account_id));
            !usage_covered
        })
        .collect()
}

fn aggregate_summary_projection_records(
    selected: &[&SummaryProjectionRecord],
    usage_selected: &[&SummaryProjectionRecord],
) -> (StatsTotals, UsageBreakdownAccumulator, i64) {
    let mut totals = StatsTotals::default();
    let mut usage = UsageBreakdownAccumulator::default();
    let mut non_success_tokens = 0_i64;
    for record in selected {
        let row = &record.row;
        let classification = resolve_failure_classification(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        let terminal = !matches!(
            normalized_runtime_text(Some(row.status.as_str())).as_str(),
            "running" | "pending"
        );
        let success = runtime_record_is_success_for_summary_row(row)
            && classification.failure_class == FailureClass::None;
        let failure = terminal && classification.failure_class != FailureClass::None;
        totals.total_count += 1;
        totals.total_cost += row.cost.unwrap_or_default();
        totals.total_tokens += row.total_tokens;
        if success {
            totals.success_count += 1;
        }
        if failure {
            totals.failure_count += 1;
            totals.non_success_cost += row.cost.unwrap_or_default();
            non_success_tokens += row.total_tokens.max(0);
        }
    }
    for record in usage_selected {
        usage.add_row(&record.row);
    }
    (totals, usage, non_success_tokens)
}

struct SummaryProjectionRollupMergeInput<'a> {
    projection: &'a SummaryProjection,
    query: &'a SummaryProjectionQuery,
    full_rollup_range: Option<(i64, i64)>,
    exact_total_buckets: &'a HashSet<i64>,
    exact_usage_buckets: &'a HashSet<i64>,
    totals: &'a mut StatsTotals,
    usage: &'a mut UsageBreakdownAccumulator,
    non_success_tokens: &'a mut i64,
}

fn merge_summary_projection_hourly_rollups(input: SummaryProjectionRollupMergeInput<'_>) {
    let SummaryProjectionRollupMergeInput {
        projection,
        query,
        full_rollup_range,
        exact_total_buckets,
        exact_usage_buckets,
        totals,
        usage,
        non_success_tokens,
    } = input;
    let Some((start, end)) = full_rollup_range else {
        return;
    };
    for ((bucket, account), bucket_totals) in &projection.hourly_rollup_totals {
        if *bucket >= start
            && *bucket < end
            && *account == query.upstream_account_id
            && !exact_total_buckets.contains(bucket)
        {
            *totals = totals.add(*bucket_totals);
            *non_success_tokens += projection
                .hourly_rollup_non_success_tokens
                .get(&(*bucket, *account))
                .copied()
                .unwrap_or_default();
        }
    }
    for ((bucket, account), value) in &projection.hourly_rollup_usage {
        if *bucket >= start
            && *bucket < end
            && *account == query.upstream_account_id
            && !exact_usage_buckets.contains(bucket)
        {
            usage.merge_response(value);
        }
    }
}

fn merge_summary_projection_overlay(input: SummaryProjectionRollupMergeInput<'_>) {
    let SummaryProjectionRollupMergeInput {
        projection,
        query,
        full_rollup_range,
        exact_total_buckets,
        exact_usage_buckets,
        totals,
        usage,
        non_success_tokens,
    } = input;
    let Some((start, end)) = full_rollup_range else {
        return;
    };
    let Some(overlay) = &projection.coverage_overlay else {
        return;
    };
    match query.upstream_account_id {
        None => {
            for (bucket, value) in &overlay.global_by_bucket {
                if *bucket >= start
                    && *bucket < end
                    && overlay.global_coverage_buckets.contains(bucket)
                    && !exact_total_buckets.contains(bucket)
                {
                    *totals = totals.add(*value);
                    *non_success_tokens += overlay
                        .global_non_success_tokens_by_bucket
                        .get(bucket)
                        .copied()
                        .unwrap_or_default();
                }
            }
            for (bucket, value) in &overlay.global_usage_by_bucket {
                if *bucket >= start
                    && *bucket < end
                    && overlay.global_coverage_buckets.contains(bucket)
                    && !exact_usage_buckets.contains(bucket)
                {
                    usage.merge_response(value);
                }
            }
        }
        Some(account_id) => {
            for ((bucket, candidate), value) in &overlay.account_by_bucket {
                if *candidate == account_id
                    && *bucket >= start
                    && *bucket < end
                    && overlay.account_coverage_buckets.contains(bucket)
                    && !exact_total_buckets.contains(bucket)
                {
                    *totals = totals.add(*value);
                    *non_success_tokens += overlay
                        .account_non_success_tokens_by_bucket
                        .get(&(*bucket, account_id))
                        .copied()
                        .unwrap_or_default();
                }
            }
            for ((bucket, candidate), value) in &overlay.account_usage_by_bucket {
                if *candidate == account_id
                    && *bucket >= start
                    && *bucket < end
                    && overlay.account_coverage_buckets.contains(bucket)
                    && !exact_usage_buckets.contains(bucket)
                {
                    usage.merge_response(value);
                }
            }
        }
    }
}

fn merge_summary_projection_boundary_records(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
    full_rollup_range: Option<(i64, i64)>,
    totals: &mut StatsTotals,
    usage: &mut UsageBreakdownAccumulator,
    non_success_tokens: &mut i64,
) {
    let Some((range_start, range_end)) = query.range else {
        return;
    };
    if full_rollup_range.is_none() {
        return;
    }
    let Some(overlay) = &projection.coverage_overlay else {
        return;
    };
    for record in &overlay.boundary_records {
        if record.occurred_at >= range_start
            && record.occurred_at < range_end
            && query
                .upstream_account_id
                .is_none_or(|account_id| record.row.upstream_account_id == Some(account_id))
            && !full_rollup_range.is_some_and(|(start, end)| {
                let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                bucket >= start && bucket < end
            })
        {
            let record_totals = summary_projection_record_totals(record);
            *totals = totals.add(record_totals);
            usage.add_row(&record.row);
            if record_totals.failure_count > 0 {
                *non_success_tokens += record.row.total_tokens.max(0);
            }
        }
    }
}

fn build_summary_projection_response(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
    totals: StatsTotals,
    usage: UsageBreakdownAccumulator,
    non_success_tokens: i64,
) -> StatsResponse {
    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    if query.range.is_some() {
        response.usage_breakdown = Some(usage.into_response());
    }
    let policy = summary_live_augmentation_policy(&query.window, query.range, query.now);
    if policy.include_in_progress {
        let in_progress = projection
            .in_progress_by_account
            .get(&query.upstream_account_id)
            .copied()
            .unwrap_or_default();
        response.in_progress_conversation_count = Some(in_progress.in_progress_count);
        response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
        response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
        response.in_progress_phase_counts = Some(in_progress.phase_counts);
    }
    if policy.include_non_success_tokens {
        response.non_success_tokens = Some(non_success_tokens);
    }
    response.maintenance = projection.maintenance.clone();
    response
}

fn calculate_summary_projection_response(
    projection: &SummaryProjection,
    query: &SummaryProjectionQuery,
) -> Result<StatsResponse, ApiError> {
    let candidates = summary_projection_candidate_rows(projection, query)?;
    let selected = select_summary_projection_total_records(projection, query, &candidates);
    let usage_selected = select_summary_projection_usage_records(projection, query, &candidates);
    let (mut totals, mut usage, mut non_success_tokens) =
        aggregate_summary_projection_records(&selected, &usage_selected);
    let exact_total_buckets = match query.upstream_account_id {
        None => &projection.exact_global_total_rollup_buckets,
        Some(_) => &projection.exact_account_total_rollup_buckets,
    };
    let exact_usage_buckets = match query.upstream_account_id {
        None => &projection.exact_global_usage_rollup_buckets,
        Some(_) => &projection.exact_account_usage_rollup_buckets,
    };
    merge_summary_projection_hourly_rollups(SummaryProjectionRollupMergeInput {
        projection,
        query,
        full_rollup_range: candidates.full_rollup_range,
        exact_total_buckets,
        exact_usage_buckets,
        totals: &mut totals,
        usage: &mut usage,
        non_success_tokens: &mut non_success_tokens,
    });
    merge_summary_projection_overlay(SummaryProjectionRollupMergeInput {
        projection,
        query,
        full_rollup_range: candidates.full_rollup_range,
        exact_total_buckets,
        exact_usage_buckets,
        totals: &mut totals,
        usage: &mut usage,
        non_success_tokens: &mut non_success_tokens,
    });
    merge_summary_projection_boundary_records(
        projection,
        query,
        candidates.full_rollup_range,
        &mut totals,
        &mut usage,
        &mut non_success_tokens,
    );
    Ok(build_summary_projection_response(
        projection,
        query,
        totals,
        usage,
        non_success_tokens,
    ))
}
