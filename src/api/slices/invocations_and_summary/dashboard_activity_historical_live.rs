struct SummaryProjectionLiveRowsQuery<'a> {
    pool: &'a Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    high_watermark_id: i64,
    min_id_exclusive: Option<i64>,
    upstream_account_id: Option<Option<i64>>,
    limit: usize,
    in_progress_only: bool,
    preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
}

async fn query_summary_projection_live_rows_with_budget(
    query_input: SummaryProjectionLiveRowsQuery<'_>,
) -> Result<SummaryProjectionLiveAdmission, anyhow::Error> {
    let SummaryProjectionLiveRowsQuery {
        pool,
        source_scope,
        range,
        high_watermark_id,
        min_id_exclusive,
        upstream_account_id,
        limit,
        in_progress_only,
        preview_cache,
        telemetry,
    } = query_input;
    let text_columns = load_summary_projection_live_text_columns(pool).await?;
    let candidates =
        query_summary_projection_live_candidates(SummaryProjectionLiveCandidatesQuery {
            pool,
            text_columns: &text_columns,
            source_scope,
            range,
            high_watermark_id,
            min_id_exclusive,
            upstream_account_id,
            in_progress_only,
            limit,
        })
        .await?;
    let (pages, gaps, overflow) = pack_summary_projection_live_candidates(candidates, limit);
    let pages_to_fetch = pages
        .into_iter()
        .map(|page| {
            let uncached_ids = page
                .iter()
                .filter(|candidate| !preview_cache.contains_key(&candidate.id))
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            (page, uncached_ids)
        })
        .collect::<Vec<_>>();
    let page_results =
        hydrate_summary_projection_live_pages(pool, source_scope, telemetry, pages_to_fetch)
            .await?;
    let (mut rows, gaps) =
        collect_summary_projection_live_page_rows(page_results, gaps, preview_cache);
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut connection = pool.acquire().await?;
    restore_summary_projection_persisted_statuses(&mut connection, &mut rows)
        .await
        .context("summary projection live status hydration failed")?;
    Ok(SummaryProjectionLiveAdmission {
        rows,
        gaps,
        overflow,
    })
}

async fn hydrate_summary_projection_live_pages(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
    pages_to_fetch: Vec<(Vec<SummaryProjectionLiveCandidate>, Vec<i64>)>,
) -> Result<
    Vec<(
        Vec<SummaryProjectionLiveCandidate>,
        Vec<UpstreamAccountInvocationPreviewRow>,
    )>,
    anyhow::Error,
> {
    futures_util::stream::iter(
        pages_to_fetch
            .into_iter()
            .map(|(page, uncached_ids)| async move {
                let mut fetched_rows = if uncached_ids.is_empty() {
                    Vec::new()
                } else {
                    query_summary_projection_preview_rows_by_ids(
                        pool,
                        source_scope,
                        &uncached_ids,
                        telemetry,
                    )
                    .await
                    .map_err(|error| {
                        anyhow!("summary projection live preview fetch failed: {error:?}")
                    })?
                };
                let upstream_account_ids = page
                    .iter()
                    .map(|candidate| (candidate.id, candidate.upstream_account_id))
                    .collect::<HashMap<_, _>>();
                for row in &mut fetched_rows {
                    row.upstream_account_id = upstream_account_ids.get(&row.id).copied().flatten();
                }
                Ok::<_, anyhow::Error>((page, fetched_rows))
            }),
    )
    .buffer_unordered(8)
    .try_collect::<Vec<_>>()
    .await
}

fn collect_summary_projection_live_page_rows(
    page_results: Vec<(
        Vec<SummaryProjectionLiveCandidate>,
        Vec<UpstreamAccountInvocationPreviewRow>,
    )>,
    mut gaps: Vec<SummaryProjectionLiveCandidate>,
    preview_cache: &mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
) -> (
    Vec<UpstreamAccountInvocationPreviewRow>,
    Vec<SummaryProjectionLiveCandidate>,
) {
    let mut rows = Vec::new();
    for (page, fetched_rows) in page_results {
        for row in fetched_rows {
            preview_cache.insert(row.id, row);
        }
        let mut page_rows = Vec::with_capacity(page.len());
        for candidate in page {
            if let Some(row) = preview_cache.get(&candidate.id) {
                page_rows.push(row.clone());
            } else {
                gaps.push(candidate);
            }
        }
        rows.extend(page_rows);
    }
    (rows, gaps)
}

async fn query_summary_projection_preview_rows_by_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let mut connection = pool.acquire().await?;
    let mut rows = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        build_summary_projection_preview_select(&mut query);
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query.push(" ORDER BY occurred_at DESC, id DESC");
        rows.extend(
            query
                .build_query_as::<UpstreamAccountInvocationPreviewRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/summary",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            candidate_preview_id_count = ids.len(),
            hydrated_preview_row_count = rows.len(),
            ?source_scope,
            selected_preview_row_count = ids.len(),
            row_count = rows.len(),
            elapsed_ms,
            "slow summary projection preview hydration"
        );
    }
    Ok(rows)
}

#[derive(Debug, FromRow)]
struct SummaryProjectionHistoricalLiveBucketRow {
    occurred_at: String,
    max_id: i64,
}

#[derive(Debug, FromRow)]
struct SummaryProjectionHistoricalLiveAccountBucketRow {
    occurred_at: String,
    upstream_account_id: Option<i64>,
    max_id: i64,
}

#[derive(Debug, FromRow)]
struct SummaryProjectionHistoricalLiveTerminalRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    status: Option<String>,
}

#[derive(Debug, FromRow)]
struct SummaryProjectionHistoricalLiveChangeRow {
    occurred_at: String,
    upstream_account_id: Option<i64>,
}

fn summary_projection_intersect_exact_ranges(
    left: ExactUtcRange,
    right: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let range = ExactUtcRange {
        start: left.start.max(right.start),
        end: left.end.min(right.end),
    };
    (range.start < range.end).then_some(range)
}

fn summary_projection_bucket_intersects_range(
    bucket_start_epoch: i64,
    range: ExactUtcRange,
) -> bool {
    let Some(bucket_start) = Utc.timestamp_opt(bucket_start_epoch, 0).single() else {
        return false;
    };
    bucket_start < range.end && range.start < bucket_start + ChronoDuration::hours(1)
}

struct SummaryProjectionHistoricalLiveReuseInput<'a> {
    pool: &'a Pool<Sqlite>,
    previous_coverage: Option<&'a SummaryProjectionHistoricalLiveCoverage>,
    range: ExactUtcRange,
    high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    fully_admitted_live_buckets: &'a BTreeSet<i64>,
    previous_unavailable_global_ranges: &'a [ExactUtcRange],
    previous_unavailable_account_ranges: &'a HashMap<i64, Vec<ExactUtcRange>>,
    previous_terminal_identities: &'a HashSet<String>,
    pending_terminal_identities: &'a HashSet<String>,
    unavailable_global_buckets: &'a mut BTreeSet<i64>,
    unavailable_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
}

async fn validate_summary_projection_historical_live_tail(
    input: &mut SummaryProjectionHistoricalLiveReuseInput<'_>,
    previous_end: DateTime<Utc>,
) -> Result<bool> {
    let tail = ExactUtcRange {
        start: previous_end.max(input.range.start),
        end: input.range.end,
    };
    if tail.start >= tail.end {
        return Ok(false);
    }
    if tail.end - tail.start <= ChronoDuration::hours(1) {
        mark_summary_projection_uncovered_historical_live_ranges(
            SummaryProjectionHistoricalLiveCoverageInput {
                pool: input.pool,
                range: tail,
                high_watermark_id: input.high_watermark_id,
                rollup_live_cursor: input.rollup_live_cursor,
                account_rollup_live_cursor: input.account_rollup_live_cursor,
                hourly_rollup_totals: input.hourly_rollup_totals,
                hourly_rollup_usage: input.hourly_rollup_usage,
                fully_admitted_live_buckets: input.fully_admitted_live_buckets,
                pending_terminal_identities: input.pending_terminal_identities,
                unavailable_global_buckets: input.unavailable_global_buckets,
                unavailable_account_buckets: input.unavailable_account_buckets,
                global_covered_terminal_invoke_ids: input.global_covered_terminal_invoke_ids,
            },
        )
        .await?;
        return Ok(false);
    }
    summary_projection_mark_unavailable_archive_ranges(input.unavailable_global_buckets, [tail])?;
    Ok(true)
}

async fn mark_summary_projection_historical_live_changes(
    pool: &Pool<Sqlite>,
    previous_high_watermark_id: i64,
    overlap: ExactUtcRange,
    unavailable_global_buckets: &mut BTreeSet<i64>,
    unavailable_account_buckets: &mut HashMap<i64, BTreeSet<i64>>,
) -> Result<bool> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut changes_query = QueryBuilder::<Sqlite>::new("SELECT occurred_at, ");
    changes_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id FROM codex_invocations WHERE id > ")
        .push_bind(previous_high_watermark_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(overlap.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(overlap.end))
        .push(SUMMARY_PROJECTION_TERMINAL_WHERE)
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind(summary_projection_exact_record_limit().saturating_add(1) as i64);
    let changes = changes_query
        .build_query_as::<SummaryProjectionHistoricalLiveChangeRow>()
        .fetch_all(pool)
        .await
        .context("summary projection historical live delta hydration failed")?;
    if changes.len() > summary_projection_exact_record_limit() {
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [overlap])?;
        return Ok(true);
    }
    let saw_change = !changes.is_empty();
    for change in changes {
        let Some(occurred_at) = parse_to_utc_datetime(&change.occurred_at) else {
            continue;
        };
        let changed_range = ExactUtcRange {
            start: occurred_at,
            end: occurred_at + ChronoDuration::seconds(1),
        };
        summary_projection_mark_unavailable_archive_ranges(
            unavailable_global_buckets,
            [changed_range],
        )?;
        if let Some(account_id) = change
            .upstream_account_id
            .filter(|account_id| *account_id > 0)
        {
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_account_buckets.entry(account_id).or_default(),
                [changed_range],
            )?;
        }
    }
    Ok(saw_change)
}

async fn reuse_summary_projection_historical_live_coverage(
    input: SummaryProjectionHistoricalLiveReuseInput<'_>,
) -> Result<SummaryProjectionHistoricalLiveCoverage> {
    let SummaryProjectionHistoricalLiveReuseInput {
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
        pending_terminal_identities,
        unavailable_global_buckets,
        unavailable_account_buckets,
        global_covered_terminal_invoke_ids,
    } = input;
    let Some(previous_coverage) = previous_coverage else {
        // A Rolling revision can be reached only after an older process failed before it
        // published Bootstrap proof. Preserve its independent current/recent selections, but
        // never infer historical exactness without the bounded coverage snapshot.
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [range])?;
        return Ok(SummaryProjectionHistoricalLiveCoverage {
            range,
            high_watermark_id,
            reconciliation_required: true,
        });
    };
    let Some(overlap) = summary_projection_intersect_exact_ranges(previous_coverage.range, range)
    else {
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [range])?;
        return Ok(SummaryProjectionHistoricalLiveCoverage {
            range,
            high_watermark_id,
            reconciliation_required: true,
        });
    };

    let mut reconciliation_required = previous_coverage.reconciliation_required;
    reuse_summary_projection_historical_live_proof(
        overlap,
        previous_unavailable_global_ranges,
        previous_unavailable_account_ranges,
        previous_terminal_identities,
        unavailable_global_buckets,
        unavailable_account_buckets,
        global_covered_terminal_invoke_ids,
    )?;

    if previous_coverage.range.end < range.end {
        reconciliation_required |= validate_summary_projection_historical_live_tail(
            &mut SummaryProjectionHistoricalLiveReuseInput {
                pool,
                previous_coverage: Some(previous_coverage),
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
                pending_terminal_identities,
                unavailable_global_buckets,
                unavailable_account_buckets,
                global_covered_terminal_invoke_ids,
            },
            previous_coverage.range.end,
        )
        .await?;
    }

    reconciliation_required |= mark_summary_projection_historical_live_changes(
        pool,
        previous_coverage.high_watermark_id,
        overlap,
        unavailable_global_buckets,
        unavailable_account_buckets,
    )
    .await?;

    Ok(SummaryProjectionHistoricalLiveCoverage {
        range,
        high_watermark_id,
        reconciliation_required,
    })
}

fn reuse_summary_projection_historical_live_proof(
    overlap: ExactUtcRange,
    previous_unavailable_global_ranges: &[ExactUtcRange],
    previous_unavailable_account_ranges: &HashMap<i64, Vec<ExactUtcRange>>,
    previous_terminal_identities: &HashSet<String>,
    unavailable_global_buckets: &mut BTreeSet<i64>,
    unavailable_account_buckets: &mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &mut HashSet<String>,
) -> Result<()> {
    for previous_range in previous_unavailable_global_ranges {
        if let Some(intersection) =
            summary_projection_intersect_exact_ranges(*previous_range, overlap)
        {
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_global_buckets,
                [intersection],
            )?;
        }
    }
    for (account_id, ranges) in previous_unavailable_account_ranges {
        let account_buckets = unavailable_account_buckets.entry(*account_id).or_default();
        for previous_range in ranges {
            if let Some(intersection) =
                summary_projection_intersect_exact_ranges(*previous_range, overlap)
            {
                summary_projection_mark_unavailable_archive_ranges(
                    account_buckets,
                    [intersection],
                )?;
            }
        }
    }
    for identity in previous_terminal_identities {
        let Some((_, occurred_at)) = identity.split_once('\0') else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(occurred_at) else {
            continue;
        };
        if overlap.start <= occurred_at && occurred_at < overlap.end {
            global_covered_terminal_invoke_ids.insert(identity.clone());
        }
    }
    Ok(())
}

struct SummaryProjectionHistoricalLiveCoverageInput<'a> {
    pool: &'a Pool<Sqlite>,
    range: ExactUtcRange,
    high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    fully_admitted_live_buckets: &'a BTreeSet<i64>,
    pending_terminal_identities: &'a HashSet<String>,
    unavailable_global_buckets: &'a mut BTreeSet<i64>,
    unavailable_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
}

const SUMMARY_PROJECTION_TERMINAL_WHERE: &str =
    " AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')";
const SUMMARY_PROJECTION_BUCKET_SQL: &str = "substr(occurred_at, 1, 13)";

async fn mark_summary_projection_uncovered_historical_live_ranges(
    mut input: SummaryProjectionHistoricalLiveCoverageInput<'_>,
) -> Result<()> {
    let globally_covered_buckets =
        mark_summary_projection_global_historical_live_buckets(&mut input).await?;
    mark_summary_projection_account_historical_live_buckets(&mut input).await?;
    let terminal_count =
        mark_summary_projection_bounded_terminal_identities(&mut input, &globally_covered_buckets)
            .await?;
    mark_summary_projection_pending_terminal_identities(
        &mut input,
        terminal_count,
        &globally_covered_buckets,
    )
    .await
}

async fn mark_summary_projection_global_historical_live_buckets(
    input: &mut SummaryProjectionHistoricalLiveCoverageInput<'_>,
) -> Result<HashSet<i64>> {
    let mut globally_covered_buckets = HashSet::new();
    // Historical coverage is a bucket property: a compact rollup either covers every terminal
    // row in an hour or it does not. Reading one metadata row per source record made a retained
    // 34-day horizon fail closed once its *aggregate* record count crossed the resident record
    // limit, even when a requested 7-day selection had complete compact proof.
    // `occurred_at` is stored as a naive local timestamp in legacy databases. Group by its raw
    // hour text and use the project parser below; SQLite's `strftime` would reinterpret that
    // text as UTC and shift every historical proof by the local offset.
    // The durable rollup cursor already proves every source row at or below its fence. Clear
    // stale resident-budget gaps from those buckets before checking only the unrolled tail.
    for bucket in input
        .unavailable_global_buckets
        .iter()
        .copied()
        .collect::<Vec<_>>()
    {
        if summary_projection_bucket_intersects_range(bucket, input.range)
            && input.hourly_rollup_totals.contains_key(&(bucket, None))
            && input.hourly_rollup_usage.contains_key(&(bucket, None))
        {
            globally_covered_buckets.insert(bucket);
            input.unavailable_global_buckets.remove(&bucket);
        }
    }
    let mut global_query = QueryBuilder::<Sqlite>::new("SELECT ");
    global_query
        .push("MIN(occurred_at) AS occurred_at, MAX(id) AS max_id FROM codex_invocations WHERE id <= ")
        .push_bind(input.high_watermark_id)
        .push(" AND id > ")
        .push_bind(input.rollup_live_cursor)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(input.range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(input.range.end))
        .push(SUMMARY_PROJECTION_TERMINAL_WHERE)
        .push(" GROUP BY ")
        .push(SUMMARY_PROJECTION_BUCKET_SQL);
    let global_rows = global_query
        .build_query_as::<SummaryProjectionHistoricalLiveBucketRow>()
        .fetch_all(input.pool)
        .await
        .context("summary projection historical live coverage hydration failed")?;

    for row in global_rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket_start_epoch = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        let globally_covered = input
            .fully_admitted_live_buckets
            .contains(&bucket_start_epoch)
            || (row.max_id <= input.rollup_live_cursor
                && input
                    .hourly_rollup_totals
                    .contains_key(&(bucket_start_epoch, None))
                && input
                    .hourly_rollup_usage
                    .contains_key(&(bucket_start_epoch, None)));
        if globally_covered {
            globally_covered_buckets.insert(bucket_start_epoch);
        } else {
            input.unavailable_global_buckets.insert(bucket_start_epoch);
        }
    }
    // The initial bounded live admission may have marked a bucket unavailable before its
    // durable rollup was committed.  A complete global rollup is stronger proof than that
    // stale resident-budget gap, so remove only the buckets this pass proves covered.
    for bucket in &globally_covered_buckets {
        input.unavailable_global_buckets.remove(bucket);
    }
    Ok(globally_covered_buckets)
}

async fn mark_summary_projection_account_historical_live_buckets(
    input: &mut SummaryProjectionHistoricalLiveCoverageInput<'_>,
) -> Result<()> {
    let mut covered_account_buckets = HashMap::<i64, BTreeSet<i64>>::new();
    for (account_id, unavailable_buckets) in input.unavailable_account_buckets.iter_mut() {
        for bucket in unavailable_buckets.clone() {
            if summary_projection_bucket_intersects_range(bucket, input.range)
                && input.account_rollup_live_cursor.is_some_and(|cursor| {
                    input
                        .hourly_rollup_totals
                        .contains_key(&(bucket, Some(*account_id)))
                        && input
                            .hourly_rollup_usage
                            .contains_key(&(bucket, Some(*account_id)))
                        && cursor >= 0
                })
            {
                covered_account_buckets
                    .entry(*account_id)
                    .or_default()
                    .insert(bucket);
                unavailable_buckets.remove(&bucket);
            }
        }
    }
    input
        .unavailable_account_buckets
        .retain(|_, buckets| !buckets.is_empty());
    let account_rows = query_summary_projection_historical_live_account_buckets(input).await?;
    for row in account_rows {
        let Some(account_id) = row.upstream_account_id.filter(|account_id| *account_id > 0) else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket_start_epoch = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        let account_covered = input
            .fully_admitted_live_buckets
            .contains(&bucket_start_epoch)
            || (input
                .account_rollup_live_cursor
                .is_some_and(|cursor| row.max_id <= cursor)
                && input
                    .hourly_rollup_totals
                    .contains_key(&(bucket_start_epoch, Some(account_id)))
                && input
                    .hourly_rollup_usage
                    .contains_key(&(bucket_start_epoch, Some(account_id))));
        if account_covered {
            covered_account_buckets
                .entry(account_id)
                .or_default()
                .insert(bucket_start_epoch);
        } else {
            input
                .unavailable_account_buckets
                .entry(account_id)
                .or_default()
                .insert(bucket_start_epoch);
        }
    }
    for (account_id, buckets) in covered_account_buckets {
        if let Some(unavailable) = input.unavailable_account_buckets.get_mut(&account_id) {
            for bucket in buckets {
                unavailable.remove(&bucket);
            }
            if unavailable.is_empty() {
                input.unavailable_account_buckets.remove(&account_id);
            }
        }
    }
    Ok(())
}

async fn query_summary_projection_historical_live_account_buckets(
    input: &SummaryProjectionHistoricalLiveCoverageInput<'_>,
) -> Result<Vec<SummaryProjectionHistoricalLiveAccountBucketRow>> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut account_query = QueryBuilder::<Sqlite>::new("SELECT ");
    account_query
        .push("MIN(occurred_at) AS occurred_at, ")
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, MAX(id) AS max_id FROM codex_invocations WHERE id <= ")
        .push_bind(input.high_watermark_id)
        .push(" AND id > ");
    account_query.push_bind(input.account_rollup_live_cursor.unwrap_or(0_i64));
    account_query
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(input.range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(input.range.end))
        .push(SUMMARY_PROJECTION_TERMINAL_WHERE)
        .push(" GROUP BY ")
        .push(SUMMARY_PROJECTION_BUCKET_SQL)
        .push(", ")
        .push(resolved_upstream_account_id_sql.as_str());
    account_query
        .build_query_as::<SummaryProjectionHistoricalLiveAccountBucketRow>()
        .fetch_all(input.pool)
        .await
        .context("summary projection historical live account coverage hydration failed")
}

async fn mark_summary_projection_bounded_terminal_identities(
    input: &mut SummaryProjectionHistoricalLiveCoverageInput<'_>,
    globally_covered_buckets: &HashSet<i64>,
) -> Result<i64> {
    let mut terminal_count_query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM codex_invocations WHERE id <= ");
    terminal_count_query
        .push_bind(input.high_watermark_id)
        .push(" AND id > ")
        .push_bind(input.rollup_live_cursor)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(input.range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(input.range.end))
        .push(SUMMARY_PROJECTION_TERMINAL_WHERE);
    let terminal_count = terminal_count_query
        .build_query_scalar::<i64>()
        .fetch_one(input.pool)
        .await
        .context("summary projection historical terminal count hydration failed")?;
    if terminal_count <= summary_projection_exact_record_limit() as i64 {
        // Preserve the complete identity proof for the common bounded case. Existing overlays
        // then disappear as soon as their durable compact bucket is published, exactly as they
        // did before the production-scale path was introduced.
        let mut identity_query = QueryBuilder::<Sqlite>::new(
            "SELECT id, invoke_id, occurred_at, status FROM codex_invocations WHERE id <= ",
        );
        identity_query
            .push_bind(input.high_watermark_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(input.range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(input.range.end))
            .push(SUMMARY_PROJECTION_TERMINAL_WHERE);
        let rows = identity_query
            .build_query_as::<SummaryProjectionHistoricalLiveTerminalRow>()
            .fetch_all(input.pool)
            .await
            .context("summary projection historical terminal identity hydration failed")?;
        for row in rows {
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
            if row.id <= input.rollup_live_cursor
                && (globally_covered_buckets.contains(&bucket)
                    || (input.hourly_rollup_totals.contains_key(&(bucket, None))
                        && input.hourly_rollup_usage.contains_key(&(bucket, None))))
            {
                input.global_covered_terminal_invoke_ids.insert(
                    summary_projection_source_identity_key(
                        row.id,
                        &row.invoke_id,
                        &row.occurred_at,
                    ),
                );
            }
        }
    }
    Ok(terminal_count)
}

async fn mark_summary_projection_pending_terminal_identities(
    input: &mut SummaryProjectionHistoricalLiveCoverageInput<'_>,
    terminal_count: i64,
    globally_covered_buckets: &HashSet<i64>,
) -> Result<()> {
    // SSE retains identities, not buckets. Query only the bounded terminal overlay currently in
    // memory so a freshly published compact bucket can consume an already-persisted delta without
    // retaining every historical identity.
    let pending_terminal_identity_refs =
        if terminal_count > summary_projection_exact_record_limit() as i64 {
            input.pending_terminal_identities.iter().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
    for identities in pending_terminal_identity_refs.chunks(250) {
        let selected_identities = identities
            .iter()
            .filter_map(|identity| identity.split_once('\0'))
            .collect::<Vec<_>>();
        if selected_identities.is_empty() {
            continue;
        }
        let mut identity_query = QueryBuilder::<Sqlite>::new(
            "SELECT id, invoke_id, occurred_at, status FROM codex_invocations WHERE id <= ",
        );
        identity_query
            .push_bind(input.high_watermark_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(input.range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(input.range.end))
            .push(" AND (");
        for (index, &(invoke_id, occurred_at)) in selected_identities.iter().enumerate() {
            if index > 0 {
                identity_query.push(" OR ");
            }
            identity_query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        identity_query.push(")");
        let rows = identity_query
            .build_query_as::<SummaryProjectionHistoricalLiveTerminalRow>()
            .fetch_all(input.pool)
            .await
            .context("summary projection historical terminal identity hydration failed")?;
        for row in rows {
            if matches!(
                normalized_runtime_text(row.status.as_deref()).as_str(),
                "pending" | "running"
            ) {
                continue;
            }
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
            if row.id <= input.rollup_live_cursor && globally_covered_buckets.contains(&bucket) {
                input.global_covered_terminal_invoke_ids.insert(
                    summary_projection_source_identity_key(
                        row.id,
                        &row.invoke_id,
                        &row.occurred_at,
                    ),
                );
            }
        }
    }
    Ok(())
}
