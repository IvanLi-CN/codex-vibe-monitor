async fn summary_v2_exact_coverage_buckets(
    pool: &Pool<Sqlite>,
    candidate_buckets: &HashSet<i64>,
) -> Result<HashSet<i64>> {
    let Some(first_bucket) = candidate_buckets.iter().min().copied() else {
        return Ok(HashSet::new());
    };
    let Some(last_bucket) = candidate_buckets.iter().max().copied() else {
        return Ok(HashSet::new());
    };
    let Some(first_start) = Utc.timestamp_opt(first_bucket, 0).single() else {
        return Ok(HashSet::new());
    };
    let Some(last_end) = Utc
        .timestamp_opt(last_bucket.saturating_add(3_600), 0)
        .single()
    else {
        return Ok(HashSet::new());
    };

    // Do this as one range lookup, rather than one EXISTS query per candidate hour. A long
    // recovery can legitimately hold the 4096-bucket proof budget; per-bucket queries made an
    // otherwise bounded overlay publication dominate V2 recovery throughput.
    let unresolved_ranges = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "WITH duplicate_paths AS ( \
             SELECT file_path FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed' \
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
             GROUP BY file_path HAVING COUNT(*) > 1 \
         ) \
         SELECT batches.month_key, batches.coverage_start_at, batches.coverage_end_at \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.historical_rollups_materialized_at IS NULL \
           AND ( \
               batches.coverage_start_at IS NULL \
               OR batches.coverage_end_at IS NULL \
               OR (batches.coverage_end_at >= ?1 AND batches.coverage_start_at < ?2) \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
               WHERE proof.archive_batch_id = batches.id \
                 AND proof.manifest_sha256 = batches.sha256 \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM duplicate_paths \
               WHERE duplicate_paths.file_path = batches.file_path \
           )",
    )
    .bind(crate::stats::db_occurred_at_lower_bound(first_start))
    .bind(crate::stats::db_occurred_at_lower_bound(last_end))
    .fetch_all(pool)
    .await
    .context("summary V2 exact coverage obligation range lookup failed")?;

    let mut exact_buckets = candidate_buckets.iter().copied().collect::<BTreeSet<_>>();
    for (month_key, coverage_start, coverage_end) in unresolved_ranges {
        let (Some(coverage_start), Some(coverage_end)) = (coverage_start, coverage_end) else {
            let Ok(month_buckets) =
                crate::stats::archive_bucket_start_epochs_from_bounds(Some(&month_key), None, None)
            else {
                exact_buckets.clear();
                break;
            };
            let overlapping = month_buckets
                .into_iter()
                .filter(|bucket| exact_buckets.contains(bucket))
                .collect::<Vec<_>>();
            for bucket in overlapping {
                exact_buckets.remove(&bucket);
            }
            continue;
        };
        let Some(start) = parse_to_utc_datetime(&coverage_start) else {
            exact_buckets.clear();
            break;
        };
        let Some(end) = parse_to_utc_datetime(&coverage_end) else {
            exact_buckets.clear();
            break;
        };
        let first_overlapping_bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
        let last_overlapping_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
        let overlapping = exact_buckets
            .range(first_overlapping_bucket..=last_overlapping_bucket)
            .copied()
            .collect::<Vec<_>>();
        for bucket in overlapping {
            exact_buckets.remove(&bucket);
        }
    }
    Ok(exact_buckets.into_iter().collect())
}

fn summary_projection_remove_covered_ranges(
    ranges: &mut Vec<ExactUtcRange>,
    covered_buckets: &HashSet<i64>,
) {
    if covered_buckets.is_empty() || ranges.is_empty() {
        return;
    }
    let mut covered = covered_buckets
        .iter()
        .filter_map(|bucket| {
            let start = Utc.timestamp_opt(*bucket, 0).single()?;
            Some(ExactUtcRange {
                start,
                end: start + ChronoDuration::hours(1),
            })
        })
        .collect::<Vec<_>>();
    covered.sort_by_key(|range| range.start);
    let mut remaining = Vec::new();
    for range in ranges.drain(..) {
        let mut fragments = vec![range];
        for bucket_range in &covered {
            let mut next = Vec::new();
            for fragment in fragments {
                if fragment.end <= bucket_range.start || bucket_range.end <= fragment.start {
                    next.push(fragment);
                    continue;
                }
                if fragment.start < bucket_range.start {
                    next.push(ExactUtcRange {
                        start: fragment.start,
                        end: bucket_range.start.min(fragment.end),
                    });
                }
                if bucket_range.end < fragment.end {
                    next.push(ExactUtcRange {
                        start: bucket_range.end.max(fragment.start),
                        end: fragment.end,
                    });
                }
            }
            fragments = next;
            if fragments.is_empty() {
                break;
            }
        }
        remaining.extend(
            fragments
                .into_iter()
                .filter(|range| range.start < range.end),
        );
    }
    *ranges = summary_projection_merge_exact_ranges(remaining);
}

/// Publish verified V2 archive contributions without invoking the generic RollingDelta builder.
/// The live tail cursor is a CAS fence: if a terminal commits while the proof is being reduced,
/// the overlay is discarded and the next maintenance pass retries against the newer projection.
async fn publish_summary_coverage_overlay(
    state: &AppState,
    force_full_coverage_reduction: bool,
) -> Result<bool> {
    // Rolling refreshes and historical proof pages publish through the same immutable hub. A
    // bounded CAS retry lets a proof page rebase on a concurrent rolling swap instead of
    // silently leaving a newly verified 30-day range behind the previous unavailable overlay.
    for _attempt in 0..4 {
        if publish_summary_coverage_overlay_once(state, force_full_coverage_reduction).await? {
            return Ok(true);
        }
        tokio::task::yield_now().await;
    }
    Ok(false)
}

fn summary_coverage_overlay_requires_full_reduction(
    force_full_coverage_reduction: bool,
    has_previous_overlay: bool,
    has_revoked_proof: bool,
) -> bool {
    // Once an overlay exists, a completed recovery turn can add only the newly verified
    // manifests. Re-scanning every retained V2 page on each coverage-fence revision defeats the
    // bounded supervisor. A full reduction is required only for the first publication or after
    // proof revocation; the final no-pending pass refreshes gap metadata separately.
    force_full_coverage_reduction || !has_previous_overlay || has_revoked_proof
}

struct SummaryCoverageOverlayReduction {
    full_reduction: bool,
    totals: SummaryV2ArchiveTotals,
}

async fn load_summary_coverage_overlay_reduction(
    state: &AppState,
    force_full_coverage_reduction: bool,
    next: &mut SummaryProjection,
    previous_overlay: Option<&SummaryCoverageOverlay>,
    previous_proof_identities: &HashSet<SummaryArchiveSnapshotProofIdentity>,
    proof_identities: &HashSet<SummaryArchiveSnapshotProofIdentity>,
    existing_source_identities: &HashSet<SummarySourceIdentity>,
) -> Result<SummaryCoverageOverlayReduction> {
    let revoked_proof_identities = previous_proof_identities
        .difference(proof_identities)
        .cloned()
        .collect::<HashSet<_>>();
    let added_proof_identities = proof_identities
        .difference(previous_proof_identities)
        .cloned()
        .collect::<HashSet<_>>();
    let full_reduction = summary_coverage_overlay_requires_full_reduction(
        force_full_coverage_reduction,
        previous_overlay.is_some(),
        !revoked_proof_identities.is_empty(),
    );
    if full_reduction || force_full_coverage_reduction {
        let coverage_end = Utc::now() + ChronoDuration::seconds(1);
        let coverage_start =
            coverage_end - summary_projection_exact_horizon(state.config.invocation_max_days);
        if let Some(high_watermark_id) =
            summary_projection_completed_manifest_high_watermark(&state.pool).await?
        {
            let unproven = summary_projection_overflowed_boundary_unproven_ranges_scoped(
                &state.pool,
                ExactUtcRange {
                    start: coverage_start,
                    end: coverage_end,
                },
                high_watermark_id,
                None,
                None,
            )
            .await?;
            next.unavailable_unmaterialized_archive_ranges = unproven.global;
            next.unavailable_unmaterialized_archive_account_ranges = unproven.account;
        } else {
            next.unavailable_unmaterialized_archive_ranges.clear();
        }
    }
    let mut boundary_ranges = next.unavailable_boundary_archive_ranges.clone();
    for range in &next.unavailable_unmaterialized_archive_ranges {
        boundary_ranges.extend(summary_projection_partial_rollup_ranges(
            range.start,
            range.end,
        ));
    }
    let boundary_ranges = summary_projection_merge_exact_ranges(boundary_ranges);
    let identities = if full_reduction {
        proof_identities
    } else {
        &added_proof_identities
    };
    let totals = load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
        &state.pool,
        identities,
        existing_source_identities,
        &boundary_ranges,
    )
    .await?;
    Ok(SummaryCoverageOverlayReduction {
        full_reduction,
        totals,
    })
}

fn apply_summary_coverage_overlay_usage_gaps(
    next: &mut SummaryProjection,
    totals: &SummaryV2ArchiveTotals,
) -> bool {
    if !totals.global_usage_gap_buckets.is_empty() {
        let gap_ranges = summary_projection_unavailable_bucket_ranges(
            totals.global_usage_gap_buckets.iter().copied().collect(),
        );
        next.unavailable_unmaterialized_archive_ranges
            .extend(gap_ranges.iter().copied());
        next.unavailable_boundary_archive_ranges
            .extend(gap_ranges.iter().copied());
        next.all_time_by_account.remove(&None);
        next.freshness.global_all_time_eligible = false;
        next.all_time_terminal_coverage_complete = false;
        next.all_time_terminal_sequence_watermark = 0;
        next.all_time_persisted_live_terminal_invoke_ids.clear();
    }
    if !totals.account_usage_gap_buckets.is_empty() {
        let gap_ranges = summary_projection_unavailable_bucket_ranges(
            totals.account_usage_gap_buckets.iter().copied().collect(),
        );
        next.unavailable_unmaterialized_archive_account_ranges
            .extend(gap_ranges.iter().copied());
        next.unavailable_boundary_archive_account_ranges
            .extend(gap_ranges.iter().copied());
        next.freshness.account_all_time_eligible.clear();
        next.all_time_by_account.retain(|scope, _| scope.is_none());
    }
    !next.unavailable_unmaterialized_archive_ranges.is_empty()
        || !next.unavailable_boundary_archive_ranges.is_empty()
        || !next
            .unavailable_unmaterialized_archive_account_ranges
            .is_empty()
        || !next.unavailable_boundary_archive_account_ranges.is_empty()
}

struct SummaryCoverageOverlayBuckets {
    global: HashSet<i64>,
    account: HashSet<i64>,
    materialized: HashSet<i64>,
    materialized_months: HashSet<String>,
    previous_global: HashSet<i64>,
    previous_account: HashSet<i64>,
}

async fn build_summary_coverage_overlay_buckets(
    state: &AppState,
    previous_overlay: Option<&SummaryCoverageOverlay>,
    totals: &SummaryV2ArchiveTotals,
    full_reduction: bool,
) -> Result<SummaryCoverageOverlayBuckets> {
    let previous_global = previous_overlay
        .map(|overlay| overlay.global_coverage_buckets.clone())
        .unwrap_or_default();
    let previous_account = previous_overlay
        .map(|overlay| overlay.account_coverage_buckets.clone())
        .unwrap_or_default();
    let mut global = if full_reduction {
        totals.global_coverage_buckets.clone()
    } else {
        previous_global.clone()
    };
    let mut account = if full_reduction {
        totals.account_coverage_buckets.clone()
    } else {
        previous_account.clone()
    };
    if !full_reduction {
        global.extend(totals.global_coverage_buckets.iter().copied());
        account.extend(totals.account_coverage_buckets.iter().copied());
    }
    let mut materialized = previous_overlay
        .filter(|_| !full_reduction)
        .map(|overlay| overlay.materialized_coverage_buckets.clone())
        .unwrap_or_else(|| totals.materialized_buckets.clone());
    let mut materialized_months = previous_overlay
        .filter(|_| !full_reduction)
        .map(|overlay| overlay.materialized_coverage_months.clone())
        .unwrap_or_else(|| totals.materialized_months_without_coverage.clone());
    if !full_reduction {
        materialized.extend(totals.materialized_buckets.iter().copied());
        materialized_months.extend(totals.materialized_months_without_coverage.iter().cloned());
    }
    global = summary_v2_exact_coverage_buckets(&state.pool, &global).await?;
    account = summary_v2_exact_coverage_buckets(&state.pool, &account).await?;
    Ok(SummaryCoverageOverlayBuckets {
        global,
        account,
        materialized,
        materialized_months,
        previous_global,
        previous_account,
    })
}

async fn revoke_summary_coverage_overlay_if_empty(
    state: &AppState,
    next: &mut SummaryProjection,
    buckets: &SummaryCoverageOverlayBuckets,
    durable_fence: SummaryProjectionGenerationFence,
) -> Result<bool> {
    for (current, previous, global) in [
        (&buckets.global, &buckets.previous_global, true),
        (&buckets.account, &buckets.previous_account, false),
    ] {
        let revoked = previous
            .difference(current)
            .copied()
            .collect::<BTreeSet<_>>();
        if revoked.is_empty() {
            continue;
        }
        let ranges = summary_projection_unavailable_bucket_ranges(revoked);
        if global {
            next.unavailable_unmaterialized_archive_ranges
                .extend(ranges.iter().copied());
            next.unavailable_boundary_archive_ranges
                .extend(ranges.iter().copied());
            next.unavailable_unmaterialized_archive_current_ranges
                .extend(ranges);
        } else {
            next.unavailable_unmaterialized_archive_account_ranges
                .extend(ranges.iter().copied());
            next.unavailable_boundary_archive_account_ranges
                .extend(ranges.iter().copied());
            next.unavailable_unmaterialized_archive_account_current_ranges
                .extend(ranges);
        }
    }
    if !buckets.global.is_empty() || !buckets.account.is_empty() || next.coverage_overlay.is_none()
    {
        return Ok(false);
    }
    next.coverage_overlay = None;
    next.generation_fence = durable_fence;
    let latest_fence = load_summary_projection_generation_fence(state).await?;
    let latest_projection = state.subscription_hub.summary_projection().await;
    if !latest_fence
        .live_tail_cursor()
        .terminal_sources_match(durable_fence.live_tail_cursor())
        || !latest_fence.coverage_sources_match(durable_fence)
        || latest_projection
            .as_ref()
            .is_none_or(|current| current.revision != next.revision)
    {
        return Ok(false);
    }
    let expected_revision = next.revision;
    if !state
        .subscription_hub
        .store_summary_projection_if_revision(next.clone(), expected_revision)
        .await
    {
        return Ok(false);
    }
    info!(
        stage = "coverage_overlay_revocation",
        revoked_bucket_count = buckets.previous_global.len() + buckets.previous_account.len(),
        "summary V2 coverage overlay revoked after proof invalidation"
    );
    Ok(true)
}

fn remove_summary_coverage_overlay_ranges(
    next: &mut SummaryProjection,
    buckets: &SummaryCoverageOverlayBuckets,
) {
    for ranges in [
        &mut next.unavailable_unmaterialized_archive_ranges,
        &mut next.unavailable_boundary_archive_ranges,
        &mut next.unavailable_unmaterialized_archive_current_ranges,
    ] {
        summary_projection_remove_covered_ranges(ranges, &buckets.global);
        *ranges = summary_projection_merge_exact_ranges(std::mem::take(ranges));
    }
    for ranges in [
        &mut next.unavailable_unmaterialized_archive_account_ranges,
        &mut next.unavailable_boundary_archive_account_ranges,
        &mut next.unavailable_unmaterialized_archive_account_current_ranges,
    ] {
        summary_projection_remove_covered_ranges(ranges, &buckets.account);
        *ranges = summary_projection_merge_exact_ranges(std::mem::take(ranges));
    }
}

struct SummaryCoverageOverlayContributions {
    global_by_bucket: HashMap<i64, StatsTotals>,
    account_by_bucket: HashMap<(i64, i64), StatsTotals>,
    global_usage_by_bucket: HashMap<i64, UsageBreakdownResponse>,
    account_usage_by_bucket: HashMap<(i64, i64), UsageBreakdownResponse>,
    global_non_success_tokens_by_bucket: HashMap<i64, i64>,
    account_non_success_tokens_by_bucket: HashMap<(i64, i64), i64>,
    boundary_records: Vec<SummaryProjectionRecord>,
}

fn initial_summary_coverage_overlay_contributions(
    previous_overlay: Option<&SummaryCoverageOverlay>,
    full_reduction: bool,
) -> SummaryCoverageOverlayContributions {
    SummaryCoverageOverlayContributions {
        global_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.global_by_bucket.clone())
                .unwrap_or_default()
        },
        account_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.account_by_bucket.clone())
                .unwrap_or_default()
        },
        global_usage_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.global_usage_by_bucket.clone())
                .unwrap_or_default()
        },
        account_usage_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.account_usage_by_bucket.clone())
                .unwrap_or_default()
        },
        global_non_success_tokens_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.global_non_success_tokens_by_bucket.clone())
                .unwrap_or_default()
        },
        account_non_success_tokens_by_bucket: if full_reduction {
            HashMap::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.account_non_success_tokens_by_bucket.clone())
                .unwrap_or_default()
        },
        boundary_records: if full_reduction {
            Vec::new()
        } else {
            previous_overlay
                .map(|overlay| overlay.boundary_records.clone())
                .unwrap_or_default()
        },
    }
}

fn merge_summary_coverage_overlay_contributions(
    mut contributions: SummaryCoverageOverlayContributions,
    totals: &SummaryV2ArchiveTotals,
) -> SummaryCoverageOverlayContributions {
    let mut boundary_record_keys = contributions
        .boundary_records
        .iter()
        .map(summary_projection_record_identity_key)
        .collect::<HashSet<_>>();
    for record in &totals.boundary_records {
        if boundary_record_keys.insert(summary_projection_record_identity_key(record)) {
            contributions.boundary_records.push(record.clone());
        }
    }
    for (bucket, contribution) in &totals.global_by_bucket {
        let entry = contributions.global_by_bucket.entry(*bucket).or_default();
        *entry = entry.add(*contribution);
    }
    for (key, contribution) in &totals.account_by_bucket {
        let entry = contributions.account_by_bucket.entry(*key).or_default();
        *entry = entry.add(*contribution);
    }
    merge_summary_coverage_overlay_usage(
        &mut contributions.global_usage_by_bucket,
        &totals.global_usage_by_bucket,
    );
    merge_summary_coverage_overlay_usage(
        &mut contributions.account_usage_by_bucket,
        &totals.account_usage_by_bucket,
    );
    for (bucket, contribution) in &totals.global_non_success_tokens_by_bucket {
        *contributions
            .global_non_success_tokens_by_bucket
            .entry(*bucket)
            .or_default() += contribution;
    }
    for (key, contribution) in &totals.account_non_success_tokens_by_bucket {
        *contributions
            .account_non_success_tokens_by_bucket
            .entry(*key)
            .or_default() += contribution;
    }
    contributions
}

fn merge_summary_coverage_overlay_usage<K: Eq + std::hash::Hash + Copy>(
    target: &mut HashMap<K, UsageBreakdownResponse>,
    additions: &HashMap<K, UsageBreakdownAccumulator>,
) {
    for (key, contribution) in additions {
        let mut merged = UsageBreakdownAccumulator::default();
        if let Some(existing) = target.get(key) {
            merged.merge_response(existing);
        }
        merged.merge_response(&contribution.clone().into_response());
        target.insert(*key, merged.into_response());
    }
}

struct SummaryCoverageOverlayPublicationInput {
    durable_fence: SummaryProjectionGenerationFence,
    proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    recent_proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    buckets: SummaryCoverageOverlayBuckets,
    contributions: SummaryCoverageOverlayContributions,
}

async fn publish_summary_coverage_overlay_value(
    state: &AppState,
    mut next: SummaryProjection,
    input: SummaryCoverageOverlayPublicationInput,
) -> Result<bool> {
    let SummaryCoverageOverlayPublicationInput {
        durable_fence,
        proof_identities,
        recent_proof_identities,
        buckets,
        contributions,
    } = input;
    let global_bucket_count = buckets.global.len();
    let account_bucket_count = buckets.account.len();
    remove_summary_coverage_overlay_ranges(&mut next, &buckets);
    let SummaryCoverageOverlayBuckets {
        global: global_buckets,
        account: account_buckets,
        materialized: materialized_coverage_buckets,
        materialized_months: materialized_coverage_months,
        ..
    } = buckets;
    let SummaryCoverageOverlayContributions {
        global_by_bucket,
        account_by_bucket,
        global_usage_by_bucket,
        account_usage_by_bucket,
        global_non_success_tokens_by_bucket,
        account_non_success_tokens_by_bucket,
        boundary_records,
    } = contributions;
    let boundary_record_count = boundary_records.len();
    next.coverage_overlay = Some(SummaryCoverageOverlay {
        coverage_fence: durable_fence.coverage_fence(),
        live_tail_cursor: durable_fence.live_tail_cursor(),
        proof_identities,
        recent_proof_identities,
        global_coverage_buckets: global_buckets,
        account_coverage_buckets: account_buckets,
        materialized_coverage_buckets,
        materialized_coverage_months,
        global_by_bucket,
        account_by_bucket,
        global_usage_by_bucket,
        account_usage_by_bucket,
        global_non_success_tokens_by_bucket,
        account_non_success_tokens_by_bucket,
        boundary_records,
    });
    next.generation_fence = durable_fence;
    let expected_revision = next.revision;
    let unavailable_unmaterialized_range_count =
        next.unavailable_unmaterialized_archive_ranges.len();
    let unavailable_boundary_range_count = next.unavailable_boundary_archive_ranges.len();
    let now = Utc::now();
    let unavailable_unmaterialized_30d_range_count = next
        .unavailable_unmaterialized_archive_ranges
        .iter()
        .filter(|range| range.start < now && now - ChronoDuration::days(30) < range.end)
        .count();
    if !publish_summary_projection_with_durable_fence(state, next, expected_revision, durable_fence)
        .await?
    {
        debug!(
            stage = "coverage_overlay_stale_revision",
            "discarding coverage overlay because a concurrent projection or durable source mutation won the CAS"
        );
        return Ok(false);
    }
    info!(
        stage = "coverage_overlay_publication",
        global_bucket_count,
        account_bucket_count,
        boundary_record_count,
        unavailable_unmaterialized_range_count,
        unavailable_unmaterialized_30d_range_count,
        unavailable_boundary_range_count,
        "summary verified coverage overlay published"
    );
    Ok(true)
}

async fn publish_summary_coverage_overlay_once(
    state: &AppState,
    force_full_coverage_reduction: bool,
) -> Result<bool> {
    let Some(projection) = state.subscription_hub.summary_projection().await else {
        return Ok(false);
    };
    let durable_fence = load_summary_projection_generation_fence(state).await?;
    if !projection
        .generation_fence
        .live_tail_cursor()
        .terminal_sources_match(durable_fence.live_tail_cursor())
    {
        debug!(
            stage = "coverage_overlay_stale_tail_fence",
            "discarding verified coverage overlay because the live tail advanced"
        );
        return Ok(false);
    }
    let existing_source_identities = projection
        .records
        .iter()
        .chain(projection.current_records.iter())
        .map(summary_projection_record_source_identity)
        .collect::<HashSet<_>>();
    let proof_identities = load_summary_v2_archive_proof_identities(&state.pool).await?;
    let now = Utc::now();
    let recent_proof_identities = load_summary_v2_archive_proof_identities_in_range(
        &state.pool,
        now - ChronoDuration::days(30),
        now,
    )
    .await?;
    let mut next = Arc::unwrap_or_clone(projection);
    let previous_overlay = next.coverage_overlay.clone();
    let previous_proof_identities = previous_overlay
        .as_ref()
        .map(|overlay| overlay.proof_identities.clone())
        .unwrap_or_default();
    if previous_overlay.as_ref().is_some_and(|overlay| {
        overlay.coverage_fence == durable_fence.coverage_fence()
            && overlay
                .live_tail_cursor
                .terminal_sources_match(durable_fence.live_tail_cursor())
            && overlay.proof_identities == proof_identities
    }) {
        return Ok(false);
    }
    let SummaryCoverageOverlayReduction {
        full_reduction,
        totals,
    } = load_summary_coverage_overlay_reduction(
        state,
        force_full_coverage_reduction,
        &mut next,
        previous_overlay.as_ref(),
        &previous_proof_identities,
        &proof_identities,
        &existing_source_identities,
    )
    .await?;
    let has_unavailable_coverage = apply_summary_coverage_overlay_usage_gaps(&mut next, &totals);
    if totals.global_coverage_buckets.is_empty()
        && totals.account_coverage_buckets.is_empty()
        && previous_overlay.is_none()
        && !has_unavailable_coverage
    {
        return Ok(false);
    }
    let buckets = build_summary_coverage_overlay_buckets(
        state,
        previous_overlay.as_ref(),
        &totals,
        full_reduction,
    )
    .await?;
    if revoke_summary_coverage_overlay_if_empty(state, &mut next, &buckets, durable_fence).await? {
        return Ok(true);
    }
    let contributions = merge_summary_coverage_overlay_contributions(
        initial_summary_coverage_overlay_contributions(previous_overlay.as_ref(), full_reduction),
        &totals,
    );
    publish_summary_coverage_overlay_value(
        state,
        next,
        SummaryCoverageOverlayPublicationInput {
            durable_fence,
            proof_identities,
            recent_proof_identities,
            buckets,
            contributions,
        },
    )
    .await
}

fn subtract_summary_totals(left: StatsTotals, right: StatsTotals) -> StatsTotals {
    StatsTotals {
        total_count: left.total_count.saturating_sub(right.total_count),
        success_count: left.success_count.saturating_sub(right.success_count),
        failure_count: left.failure_count.saturating_sub(right.failure_count),
        total_tokens: left.total_tokens.saturating_sub(right.total_tokens),
        total_cost: (left.total_cost - right.total_cost).max(0.0),
        non_success_cost: (left.non_success_cost - right.non_success_cost).max(0.0),
    }
}
