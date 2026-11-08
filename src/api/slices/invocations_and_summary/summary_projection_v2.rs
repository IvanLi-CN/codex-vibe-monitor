use super::*;
pub(crate) fn summary_snapshot_v2_record_preview(
    record: SummaryArchiveSnapshotV2Record,
) -> UpstreamAccountInvocationPreviewRow {
    UpstreamAccountInvocationPreviewRow {
        upstream_account_id: record.upstream_account_id,
        id: record.id,
        invoke_id: record.invoke_id,
        prompt_cache_key: None,
        occurred_at: record.occurred_at,
        conversation_created_at: None,
        status: record.status,
        live_phase: None,
        failure_class: record.failure_class,
        route_mode: None,
        model: record.model,
        request_model: None,
        response_model: record.response_model,
        total_tokens: record.total_tokens,
        cost: record.cost,
        cost_input: record.cost_input,
        cost_cache_write: record.cost_cache_write,
        cost_cache_read: record.cost_cache_read,
        cost_output: record.cost_output,
        cost_reasoning: record.cost_reasoning,
        source: Some(record.source),
        input_tokens: Some(record.input_tokens),
        output_tokens: Some(record.output_tokens),
        cache_input_tokens: Some(record.cache_input_tokens),
        reasoning_tokens: Some(record.reasoning_tokens),
        reasoning_effort: record.reasoning_effort,
        error_message: record.error_message,
        downstream_status_code: None,
        downstream_error_message: None,
        failure_kind: record.failure_kind,
        is_actionable: Some(i64::from(record.is_actionable)),
        proxy_display_name: None,
        upstream_account_name: None,
        upstream_account_plan_type: None,
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        t_total_ms: None,
        endpoint: None,
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
    }
}

#[derive(Default)]
pub(crate) struct SummaryV2ArchiveTotals {
    pub(crate) global: StatsTotals,
    pub(crate) accounts: HashMap<i64, StatsTotals>,
    pub(crate) global_usage: UsageBreakdownAccumulator,
    pub(crate) account_usage: HashMap<i64, UsageBreakdownAccumulator>,
    pub(crate) global_by_bucket: HashMap<i64, StatsTotals>,
    pub(crate) account_by_bucket: HashMap<(i64, i64), StatsTotals>,
    pub(crate) global_usage_by_bucket: HashMap<i64, UsageBreakdownAccumulator>,
    pub(crate) account_usage_by_bucket: HashMap<(i64, i64), UsageBreakdownAccumulator>,
    pub(crate) global_non_success_tokens_by_bucket: HashMap<i64, i64>,
    pub(crate) account_non_success_tokens_by_bucket: HashMap<(i64, i64), i64>,
    pub(crate) global_coverage_buckets: HashSet<i64>,
    pub(crate) account_coverage_buckets: HashSet<i64>,
    pub(crate) replacement_buckets: HashSet<i64>,
    pub(crate) materialized_buckets: HashSet<i64>,
    pub(crate) materialized_months_without_coverage: HashSet<String>,
    pub(crate) global_usage_gap_buckets: HashSet<i64>,
    pub(crate) account_usage_gap_buckets: HashSet<i64>,
    pub(crate) boundary_records: Vec<SummaryProjectionRecord>,
}

pub(crate) fn summary_v2_archive_totals_from_coverage_overlay(
    overlay: &SummaryCoverageOverlay,
) -> SummaryV2ArchiveTotals {
    let mut totals = SummaryV2ArchiveTotals {
        global_coverage_buckets: overlay.global_coverage_buckets.clone(),
        account_coverage_buckets: overlay.account_coverage_buckets.clone(),
        materialized_buckets: overlay.materialized_coverage_buckets.clone(),
        materialized_months_without_coverage: overlay.materialized_coverage_months.clone(),
        replacement_buckets: overlay.global_by_bucket.keys().copied().collect(),
        ..SummaryV2ArchiveTotals::default()
    };
    for ((bucket, account_id), contribution) in &overlay.account_by_bucket {
        totals
            .account_by_bucket
            .insert((*bucket, *account_id), *contribution);
        let account_totals = totals.accounts.entry(*account_id).or_default();
        *account_totals = account_totals.add(*contribution);
        totals.account_coverage_buckets.insert(*bucket);
        totals.replacement_buckets.insert(*bucket);
    }
    for (bucket, contribution) in &overlay.global_by_bucket {
        totals.global_by_bucket.insert(*bucket, *contribution);
        totals.global = totals.global.add(*contribution);
        totals.global_coverage_buckets.insert(*bucket);
    }
    for (bucket, usage) in &overlay.global_usage_by_bucket {
        let mut accumulator = UsageBreakdownAccumulator::default();
        accumulator.merge_response(usage);
        totals.global_usage_by_bucket.insert(*bucket, accumulator);
    }
    for ((bucket, account_id), usage) in &overlay.account_usage_by_bucket {
        let mut accumulator = UsageBreakdownAccumulator::default();
        accumulator.merge_response(usage);
        totals
            .account_usage_by_bucket
            .insert((*bucket, *account_id), accumulator);
    }
    totals.global_non_success_tokens_by_bucket =
        overlay.global_non_success_tokens_by_bucket.clone();
    totals.account_non_success_tokens_by_bucket =
        overlay.account_non_success_tokens_by_bucket.clone();
    totals.boundary_records = overlay.boundary_records.clone();
    totals
}

pub(crate) type SummaryArchiveSnapshotProofIdentity = (i64, String);
pub(crate) type SummaryV2ArchiveManifest = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

pub(crate) async fn load_summary_v2_archive_totals_excluding(
    pool: &Pool<Sqlite>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
) -> Result<SummaryV2ArchiveTotals> {
    // Materialized archives are represented by the compact rollup baseline in the all-time
    // finalizer. Only V2-proof identities can contribute archive rows here, so avoid loading and
    // iterating every historical manifest when the proof set is empty or sparse.
    let proof_identities = load_summary_v2_archive_proof_identities(pool).await?;
    load_summary_v2_archive_totals_for_proof_identities(
        pool,
        &proof_identities,
        excluded_source_identities,
    )
    .await
}

pub(crate) async fn load_summary_v2_archive_proof_identities(
    pool: &Pool<Sqlite>,
) -> Result<HashSet<SummaryArchiveSnapshotProofIdentity>> {
    Ok(sqlx::query_as::<_, SummaryArchiveSnapshotProofIdentity>(
        "WITH duplicate_paths AS (
             SELECT file_path FROM archive_batches
             WHERE dataset = 'codex_invocations' AND status = 'completed'
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
             GROUP BY file_path HAVING COUNT(*) > 1
         )
         SELECT proof.archive_batch_id, proof.manifest_sha256
         FROM summary_archive_snapshot_v2_proof AS proof
         INNER JOIN archive_batches AS batches
           ON batches.id = proof.archive_batch_id
          AND batches.sha256 = proof.manifest_sha256
         WHERE NOT EXISTS (
             SELECT 1 FROM duplicate_paths
             WHERE duplicate_paths.file_path = batches.file_path
         )",
    )
    .fetch_all(pool)
    .await
    .context("summary V2 archive proof identity lookup failed")?
    .into_iter()
    .collect())
}

pub(crate) async fn load_summary_v2_archive_proof_identities_in_range(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<HashSet<SummaryArchiveSnapshotProofIdentity>> {
    Ok(sqlx::query_as::<_, SummaryArchiveSnapshotProofIdentity>(
        "WITH duplicate_paths AS ( \
             SELECT file_path FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed' \
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
             GROUP BY file_path HAVING COUNT(*) > 1 \
         ) \
         SELECT proof.archive_batch_id, proof.manifest_sha256 \
         FROM summary_archive_snapshot_v2_proof AS proof \
         INNER JOIN archive_batches AS batches \
           ON batches.id = proof.archive_batch_id \
          AND batches.sha256 = proof.manifest_sha256 \
         WHERE batches.coverage_end_epoch IS NOT NULL \
           AND batches.coverage_end_epoch > ?1 \
           AND batches.coverage_start_epoch IS NOT NULL \
           AND batches.coverage_start_epoch < ?2 \
           AND NOT EXISTS ( \
               SELECT 1 FROM duplicate_paths \
               WHERE duplicate_paths.file_path = batches.file_path \
           )",
    )
    .bind(start.timestamp())
    .bind(end.timestamp())
    .fetch_all(pool)
    .await
    .context("summary V2 recent proof identity lookup failed")?
    .into_iter()
    .collect())
}

pub(crate) async fn load_summary_v2_archive_totals_for_proof_identities(
    pool: &Pool<Sqlite>,
    proof_identities: &HashSet<SummaryArchiveSnapshotProofIdentity>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
) -> Result<SummaryV2ArchiveTotals> {
    load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
        pool,
        proof_identities,
        excluded_source_identities,
        &[],
    )
    .await
}

pub(crate) async fn load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
    pool: &Pool<Sqlite>,
    proof_identities: &HashSet<SummaryArchiveSnapshotProofIdentity>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
    boundary_ranges: &[ExactUtcRange],
) -> Result<SummaryV2ArchiveTotals> {
    let mut archives = Vec::with_capacity(proof_identities.len());
    for (archive_batch_id, manifest_sha256) in proof_identities {
        if let Some(archive) = sqlx::query_as::<_, SummaryV2ArchiveManifest>(
            "SELECT id, sha256, historical_rollups_materialized_at, coverage_start_at, coverage_end_at, month_key, file_path \
             FROM archive_batches \
             WHERE id = ?1 AND sha256 = ?2 AND dataset = 'codex_invocations' AND status = 'completed' \
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
        )
        .bind(archive_batch_id)
        .bind(manifest_sha256)
        .fetch_optional(pool)
        .await
        .context("summary V2 incremental archive manifest lookup failed")?
        {
            archives.push(archive);
        }
    }
    load_summary_v2_archive_totals_from_archives(
        pool,
        archives,
        excluded_source_identities,
        boundary_ranges,
    )
    .await
}

pub(crate) async fn load_summary_materialized_archive_coverage(
    pool: &Pool<Sqlite>,
) -> Result<(HashSet<i64>, HashSet<String>)> {
    let manifests = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT coverage_start_at, coverage_end_at, month_key
         FROM archive_batches
         WHERE dataset = 'codex_invocations'
           AND status = 'completed'
           AND historical_rollups_materialized_at IS NOT NULL
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_all(pool)
    .await
    .context("summary materialized archive coverage lookup failed")?;
    let mut buckets = HashSet::new();
    let mut months = HashSet::new();
    for (coverage_start, coverage_end, month_key) in manifests {
        if let (Some(start), Some(end)) = (
            coverage_start.as_deref().and_then(parse_to_utc_datetime),
            coverage_end.as_deref().and_then(parse_to_utc_datetime),
        ) {
            let mut bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
            let end_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
            while bucket <= end_bucket && buckets.len() < 4096 {
                buckets.insert(bucket);
                bucket = bucket.saturating_add(3_600);
            }
        } else if let Some(month_key) = month_key {
            months.insert(month_key);
        }
    }
    Ok((buckets, months))
}

pub(crate) async fn load_summary_v2_archive_totals_from_archives(
    pool: &Pool<Sqlite>,
    archives: Vec<SummaryV2ArchiveManifest>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
    boundary_ranges: &[ExactUtcRange],
) -> Result<SummaryV2ArchiveTotals> {
    // Finalization only consumes identities which already have a V2 proof marker.  The normal
    // backfill path performs lazy promotion before this reducer runs, so probing every missing
    // marker through `summary_archive_snapshot_has_final_proof` would needlessly attempt a
    // BEGIN IMMEDIATE for each quarantined or not-yet-processed archive.
    let proof_identities = load_summary_v2_archive_proof_identities(pool).await?;
    let mut totals = SummaryV2ArchiveTotals::default();
    for (
        archive_batch_id,
        manifest_sha256,
        materialized_at,
        coverage_start,
        coverage_end,
        month_key,
        file_path,
    ) in archives
    {
        // A materialized compact rollup remains an aggregate baseline even when the archive file
        // is no longer readable or its V2 proof is absent. Record that baseline before the V2
        // authority gate so an unmaterialized sibling in the same hour is added to, rather than
        // replacing, the already-proven compact contribution. The V2 proof gate below still
        // controls archive boundary rows and normalized Snapshot authority.
        if materialized_at.is_some() {
            if let (Some(start), Some(end)) = (
                coverage_start.as_deref().and_then(parse_to_utc_datetime),
                coverage_end.as_deref().and_then(parse_to_utc_datetime),
            ) {
                let mut bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
                let end_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
                let mut bucket_count = 0usize;
                while bucket <= end_bucket && bucket_count < 4096 {
                    totals.global_coverage_buckets.insert(bucket);
                    totals.account_coverage_buckets.insert(bucket);
                    totals.materialized_buckets.insert(bucket);
                    bucket = bucket.saturating_add(3_600);
                    bucket_count += 1;
                }
            } else if let Some(month_key) = month_key.as_deref() {
                totals
                    .materialized_months_without_coverage
                    .insert(month_key.to_string());
            }
        }
        // A final V2 proof is authority for the archive's exact boundary coverage even when
        // compact rollups already provide its aggregate totals. Check it before the materialized
        // fast path: otherwise a verified Snapshot could never remove the old boundary
        // unavailable proof, leaving a selection permanently unavailable after recovery.
        if !proof_identities.contains(&(archive_batch_id, manifest_sha256.clone()))
            || !summary_archive_snapshot_has_final_proof_read_only(
                pool,
                archive_batch_id,
                &manifest_sha256,
            )
            .await?
        {
            continue;
        }
        let archive_boundary_intersects = match (
            coverage_start.as_deref().and_then(parse_to_utc_datetime),
            coverage_end.as_deref().and_then(parse_to_utc_datetime),
        ) {
            (Some(start), Some(end)) => boundary_ranges
                .iter()
                .any(|range| range.start < end && start < range.end),
            _ => false,
        };
        if let (Some(start), Some(end)) = (
            coverage_start.as_deref().and_then(parse_to_utc_datetime),
            coverage_end.as_deref().and_then(parse_to_utc_datetime),
        ) {
            let mut bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
            let end_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
            let mut bucket_count = 0usize;
            while bucket <= end_bucket && bucket_count < 4096 {
                totals.global_coverage_buckets.insert(bucket);
                totals.account_coverage_buckets.insert(bucket);
                bucket = bucket.saturating_add(3_600);
                bucket_count += 1;
            }
        }
        if materialized_at.is_some() {
            // A materialized archive with all three replay markers already has an exact
            // rollup-backed contribution. Its V2 pages are not needed merely because the
            // bounded rolling projection retains possible boundary hours; loading every row
            // here would turn a large archive into a resident-boundary overflow. Only consult
            // V2 rows when replay proof is incomplete and the boundary actually needs repair.
            let replay = load_summary_projection_archive_replay_coverage(
                pool,
                std::slice::from_ref(&file_path),
            )
            .await?
            .remove(&file_path)
            .unwrap_or_default();
            if !replay.usage_breakdown {
                if let (Some(start), Some(end)) = (
                    coverage_start.as_deref().and_then(parse_to_utc_datetime),
                    coverage_end.as_deref().and_then(parse_to_utc_datetime),
                ) {
                    let mut bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
                    let end_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
                    while bucket <= end_bucket && totals.global_usage_gap_buckets.len() < 4096 {
                        totals.global_usage_gap_buckets.insert(bucket);
                        totals.account_usage_gap_buckets.insert(bucket);
                        bucket = bucket.saturating_add(3_600);
                    }
                } else if let Some(month_key) = month_key.as_deref()
                    && let Ok(buckets) = crate::stats::archive_bucket_start_epochs_from_bounds(
                        Some(month_key),
                        None,
                        None,
                    )
                {
                    for bucket in buckets.into_iter().take(4096) {
                        totals.global_usage_gap_buckets.insert(bucket);
                        totals.account_usage_gap_buckets.insert(bucket);
                    }
                }
            }
            if archive_boundary_intersects
                && !(replay.overall && replay.account_stats && replay.usage_breakdown)
            {
                let mut pages = sqlx::query_scalar::<_, Vec<u8>>(
                    "SELECT payload FROM summary_archive_snapshot
                     WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
                )
                .bind(archive_batch_id)
                .bind(&manifest_sha256)
                .fetch(pool);
                while let Some(payload) = pages.try_next().await? {
                    for record in decode_summary_archive_snapshot_v2_payload(&payload)? {
                        if excluded_source_identities.contains(&SummarySourceIdentity {
                            row_id: record.id,
                            invoke_id: record.invoke_id.clone(),
                            occurred_at: record.occurred_at.clone(),
                        }) {
                            continue;
                        }
                        let preview = summary_snapshot_v2_record_preview(record);
                        let Some(occurred_at) = parse_to_utc_datetime(&preview.occurred_at) else {
                            continue;
                        };
                        if !boundary_ranges
                            .iter()
                            .any(|range| range.start <= occurred_at && occurred_at < range.end)
                        {
                            continue;
                        }
                        if totals.boundary_records.len() >= summary_projection_exact_record_limit()
                        {
                            return Err(anyhow!(
                                "summary V2 boundary record budget exceeded while publishing coverage overlay"
                            ));
                        }
                        totals.boundary_records.push(SummaryProjectionRecord {
                            occurred_at,
                            global_rollup_covered: false,
                            account_rollup_covered: false,
                            usage_global_rollup_covered: false,
                            usage_account_rollup_covered: false,
                            is_persisted_live_record: false,
                            is_archive_record: true,
                            archive_has_materialized_rollups: true,
                            account_archive_totals_fallback_included: false,
                            row: preview,
                        });
                    }
                }
            }
            continue;
        }
        // Materialized archives are already represented by the durable rollup checkpoint. V2
        // pages are only an aggregate source for an unmaterialized archive whose stale or
        // missing replay markers would otherwise leave the checkpoint without exact totals.
        // Adding a materialized page set here would double count its rollup totals.
        let mut pages = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT payload FROM summary_archive_snapshot
             WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
        )
        .bind(archive_batch_id)
        .bind(&manifest_sha256)
        .fetch(pool);
        while let Some(payload) = pages.try_next().await? {
            let records = decode_summary_archive_snapshot_v2_payload(&payload)?;
            for record in &records {
                let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
                    continue;
                };
                let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
                if totals.global_coverage_buckets.len() < 4096 {
                    totals.global_coverage_buckets.insert(bucket);
                }
                if totals.account_coverage_buckets.len() < 4096 {
                    totals.account_coverage_buckets.insert(bucket);
                }
            }
            let mut identity_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT id, invoke_id, occurred_at FROM codex_invocations WHERE id IN (",
            );
            {
                let mut separated = identity_query.separated(", ");
                for record in &records {
                    separated.push_bind(record.id);
                }
            }
            identity_query.push(") OR invoke_id IN (");
            {
                let mut separated = identity_query.separated(", ");
                for record in &records {
                    separated.push_bind(&record.invoke_id);
                }
            }
            identity_query.push(")");
            let live_identity = identity_query
                .build_query_as::<(i64, String, String)>()
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect::<HashSet<_>>();
            for record in records {
                let preview = summary_snapshot_v2_record_preview(record);
                let summary_record = SummaryProjectionRecord {
                    occurred_at: parse_to_utc_datetime(&preview.occurred_at)
                        .ok_or_else(|| anyhow!("V2 Snapshot record has invalid occurred_at"))?,
                    global_rollup_covered: false,
                    account_rollup_covered: false,
                    usage_global_rollup_covered: false,
                    usage_account_rollup_covered: false,
                    is_persisted_live_record: false,
                    is_archive_record: true,
                    archive_has_materialized_rollups: false,
                    account_archive_totals_fallback_included: false,
                    row: preview.clone(),
                };
                if boundary_ranges.iter().any(|range| {
                    range.start <= summary_record.occurred_at
                        && summary_record.occurred_at < range.end
                }) {
                    if totals.boundary_records.len() >= summary_projection_exact_record_limit() {
                        return Err(anyhow!(
                            "summary V2 boundary record budget exceeded while publishing coverage overlay"
                        ));
                    }
                    totals.boundary_records.push(summary_record.clone());
                }
                // Unmaterialized V2 pages are the authoritative aggregate source for their
                // archive. The Bootstrap projection may already retain the same raw row for
                // boundary filtering, but that row is intentionally not counted by the
                // all-time exact-record reducer. Only canonical live identities must be
                // excluded here; filtering every resident projection identity would drop the
                // archive contribution entirely.
                if live_identity.contains(&(
                    preview.id,
                    preview.invoke_id.clone(),
                    preview.occurred_at.clone(),
                )) {
                    continue;
                }
                let record_totals = summary_projection_all_time_record_totals(&summary_record);
                let bucket = align_bucket_epoch(summary_record.occurred_at.timestamp(), 3_600, 0);
                let archive_month = preview.occurred_at.get(..7).unwrap_or_default();
                if !totals.materialized_buckets.contains(&bucket)
                    && !totals
                        .materialized_months_without_coverage
                        .contains(archive_month)
                {
                    totals.replacement_buckets.insert(bucket);
                }
                totals.global = totals.global.add(record_totals);
                let bucket_totals = totals.global_by_bucket.entry(bucket).or_default();
                *bucket_totals = bucket_totals.add(record_totals);
                totals
                    .global_usage_by_bucket
                    .entry(bucket)
                    .or_default()
                    .add_row(&preview);
                if record_totals.failure_count > 0 {
                    *totals
                        .global_non_success_tokens_by_bucket
                        .entry(bucket)
                        .or_default() += preview.total_tokens.max(0);
                }
                if let Some(account_id) = preview.upstream_account_id.filter(|id| *id > 0) {
                    let entry = totals.accounts.entry(account_id).or_default();
                    *entry = entry.add(record_totals);
                    let bucket_entry = totals
                        .account_by_bucket
                        .entry((bucket, account_id))
                        .or_default();
                    *bucket_entry = bucket_entry.add(record_totals);
                    totals
                        .account_usage
                        .entry(account_id)
                        .or_default()
                        .add_row(&preview);
                    totals
                        .account_usage_by_bucket
                        .entry((bucket, account_id))
                        .or_default()
                        .add_row(&preview);
                    if record_totals.failure_count > 0 {
                        *totals
                            .account_non_success_tokens_by_bucket
                            .entry((bucket, account_id))
                            .or_default() += preview.total_tokens.max(0);
                    }
                }
                totals.global_usage.add_row(&preview);
            }
        }
    }
    Ok(totals)
}

pub(crate) async fn summary_v2_exact_coverage_buckets(
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

pub(crate) fn summary_projection_remove_covered_ranges(
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
pub(crate) async fn publish_summary_coverage_overlay(
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

pub(crate) fn summary_coverage_overlay_requires_full_reduction(
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

pub(crate) async fn publish_summary_coverage_overlay_once(
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
    let revoked_proof_identities = previous_proof_identities
        .difference(&proof_identities)
        .cloned()
        .collect::<HashSet<_>>();
    let added_proof_identities = proof_identities
        .difference(&previous_proof_identities)
        .cloned()
        .collect::<HashSet<_>>();
    // A normal proof publication is append-only. Reduce only the newly proven manifests so
    // that a recovery page is bounded by its own V2 payload rather than every retained archive.
    // Revocation is rare and must reconstruct from all remaining proofs to avoid retaining a
    // contribution which is no longer authoritative.
    // During a long recovery a bucket can be temporarily filtered out because an overlapping
    // manifest is still unproven. Once the durable queue is empty, rebuild coverage from every
    // final proof exactly once so those previously filtered buckets can become available.
    let full_reduction = summary_coverage_overlay_requires_full_reduction(
        force_full_coverage_reduction,
        previous_overlay.is_some(),
        !revoked_proof_identities.is_empty(),
    );
    if full_reduction || force_full_coverage_reduction {
        // A full proof pass is the point at which stale Bootstrap gap ranges can be discarded.
        // Rebuild the durable unproven set from the current manifest/proof state instead of
        // subtracting hourly buckets from the old range vector.  The latter leaves an archive
        // interval unavailable forever when its V2 proof was committed after the initial
        // projection materialization.
        let coverage_end = Utc::now() + ChronoDuration::seconds(1);
        let coverage_horizon = summary_projection_exact_horizon(state.config.invocation_max_days);
        let coverage_start = coverage_end - coverage_horizon;
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
        // Preserve boundary gaps until the verified proof buckets below explicitly cover them.
        // A compact archive can still lack one response dimension (such as usage replay), so
        // clearing these ranges here would turn an under-proven historical response into 200.
    }
    // Boundary records are only needed to repair the global contribution being reduced here.
    // Account-only gaps are already fail-closed through the account availability overlay; adding
    // their ranges would force the reducer to inflate an entire large materialized archive just
    // to preserve an account response that cannot be served as exact anyway.
    let mut boundary_ranges = next.unavailable_boundary_archive_ranges.clone();
    for range in &next.unavailable_unmaterialized_archive_ranges {
        boundary_ranges.extend(summary_projection_partial_rollup_ranges(
            range.start,
            range.end,
        ));
    }
    boundary_ranges = summary_projection_merge_exact_ranges(boundary_ranges);
    let totals = if full_reduction {
        load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
            &state.pool,
            &proof_identities,
            &existing_source_identities,
            &boundary_ranges,
        )
        .await?
    } else {
        load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
            &state.pool,
            &added_proof_identities,
            &existing_source_identities,
            &boundary_ranges,
        )
        .await?
    };
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
    let has_unavailable_coverage = !next.unavailable_unmaterialized_archive_ranges.is_empty()
        || !next.unavailable_boundary_archive_ranges.is_empty()
        || !next
            .unavailable_unmaterialized_archive_account_ranges
            .is_empty()
        || !next.unavailable_boundary_archive_account_ranges.is_empty();
    if totals.global_coverage_buckets.is_empty()
        && totals.account_coverage_buckets.is_empty()
        && previous_overlay.is_none()
        && !has_unavailable_coverage
    {
        return Ok(false);
    }
    let previous_global_buckets = previous_overlay
        .as_ref()
        .map(|overlay| overlay.global_coverage_buckets.clone())
        .unwrap_or_default();
    let previous_account_buckets = previous_overlay
        .as_ref()
        .map(|overlay| overlay.account_coverage_buckets.clone())
        .unwrap_or_default();
    let mut global_buckets = if full_reduction {
        totals.global_coverage_buckets.clone()
    } else {
        previous_global_buckets.clone()
    };
    let mut account_buckets = if full_reduction {
        totals.account_coverage_buckets.clone()
    } else {
        previous_account_buckets.clone()
    };
    if !full_reduction {
        global_buckets.extend(totals.global_coverage_buckets.iter().copied());
        account_buckets.extend(totals.account_coverage_buckets.iter().copied());
    }
    let mut materialized_coverage_buckets = if full_reduction {
        totals.materialized_buckets.clone()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.materialized_coverage_buckets.clone())
            .unwrap_or_default()
    };
    if !full_reduction {
        materialized_coverage_buckets.extend(totals.materialized_buckets.iter().copied());
    }
    let mut materialized_coverage_months = if full_reduction {
        totals.materialized_months_without_coverage.clone()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.materialized_coverage_months.clone())
            .unwrap_or_default()
    };
    if !full_reduction {
        materialized_coverage_months
            .extend(totals.materialized_months_without_coverage.iter().cloned());
    }
    // A V2 page is an authority for its own rows, but it cannot make a whole response bucket
    // exact while another unmaterialized manifest still owes that bucket. Keep those normalized
    // contributions resident for a later atomic publication, yet leave the bucket unavailable
    // until every overlapping obligation has a final proof.
    global_buckets = summary_v2_exact_coverage_buckets(&state.pool, &global_buckets).await?;
    account_buckets = summary_v2_exact_coverage_buckets(&state.pool, &account_buckets).await?;
    let global_bucket_count = global_buckets.len();
    let account_bucket_count = account_buckets.len();
    let revoked_global_buckets = previous_global_buckets
        .difference(&global_buckets)
        .copied()
        .collect::<BTreeSet<_>>();
    let revoked_account_buckets = previous_account_buckets
        .difference(&account_buckets)
        .copied()
        .collect::<BTreeSet<_>>();
    if !revoked_global_buckets.is_empty() {
        let revoked_global_ranges =
            summary_projection_unavailable_bucket_ranges(revoked_global_buckets.clone());
        next.unavailable_unmaterialized_archive_ranges
            .extend(revoked_global_ranges.iter().copied());
        next.unavailable_boundary_archive_ranges
            .extend(revoked_global_ranges.iter().copied());
        next.unavailable_unmaterialized_archive_current_ranges
            .extend(revoked_global_ranges);
    }
    if !revoked_account_buckets.is_empty() {
        let revoked_account_ranges =
            summary_projection_unavailable_bucket_ranges(revoked_account_buckets.clone());
        next.unavailable_unmaterialized_archive_account_ranges
            .extend(revoked_account_ranges.iter().copied());
        next.unavailable_boundary_archive_account_ranges
            .extend(revoked_account_ranges.iter().copied());
        next.unavailable_unmaterialized_archive_account_current_ranges
            .extend(revoked_account_ranges);
    }
    if global_buckets.is_empty() && account_buckets.is_empty() && next.coverage_overlay.is_some() {
        let revoked_global_count = previous_global_buckets.len();
        let revoked_account_count = previous_account_buckets.len();
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
            .store_summary_projection_if_revision(next, expected_revision)
            .await
        {
            return Ok(false);
        }
        info!(
            stage = "coverage_overlay_revocation",
            revoked_bucket_count = revoked_global_count + revoked_account_count,
            "summary V2 coverage overlay revoked after proof invalidation"
        );
        return Ok(true);
    }
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_unmaterialized_archive_ranges,
        &global_buckets,
    );
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_boundary_archive_ranges,
        &global_buckets,
    );
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_unmaterialized_archive_current_ranges,
        &global_buckets,
    );
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_unmaterialized_archive_account_ranges,
        &account_buckets,
    );
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_boundary_archive_account_ranges,
        &account_buckets,
    );
    summary_projection_remove_covered_ranges(
        &mut next.unavailable_unmaterialized_archive_account_current_ranges,
        &account_buckets,
    );
    next.unavailable_unmaterialized_archive_ranges =
        summary_projection_merge_exact_ranges(next.unavailable_unmaterialized_archive_ranges);
    next.unavailable_boundary_archive_ranges =
        summary_projection_merge_exact_ranges(next.unavailable_boundary_archive_ranges);
    next.unavailable_unmaterialized_archive_current_ranges = summary_projection_merge_exact_ranges(
        next.unavailable_unmaterialized_archive_current_ranges,
    );
    next.unavailable_unmaterialized_archive_account_ranges = summary_projection_merge_exact_ranges(
        next.unavailable_unmaterialized_archive_account_ranges,
    );
    next.unavailable_boundary_archive_account_ranges =
        summary_projection_merge_exact_ranges(next.unavailable_boundary_archive_account_ranges);
    next.unavailable_unmaterialized_archive_account_current_ranges =
        summary_projection_merge_exact_ranges(
            next.unavailable_unmaterialized_archive_account_current_ranges,
        );
    let mut global_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.global_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut account_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.account_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut global_usage_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.global_usage_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut account_usage_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.account_usage_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut global_non_success_tokens_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.global_non_success_tokens_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut account_non_success_tokens_by_bucket = if full_reduction {
        HashMap::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.account_non_success_tokens_by_bucket.clone())
            .unwrap_or_default()
    };
    let mut boundary_records = if full_reduction {
        Vec::new()
    } else {
        previous_overlay
            .as_ref()
            .map(|overlay| overlay.boundary_records.clone())
            .unwrap_or_default()
    };
    let mut boundary_record_keys = boundary_records
        .iter()
        .map(summary_projection_record_identity_key)
        .collect::<HashSet<_>>();
    for record in totals.boundary_records {
        let key = summary_projection_record_identity_key(&record);
        if boundary_record_keys.insert(key) {
            boundary_records.push(record);
        }
    }
    for (bucket, contribution) in &totals.global_by_bucket {
        let entry = global_by_bucket.entry(*bucket).or_default();
        *entry = entry.add(*contribution);
    }
    for (key, contribution) in &totals.account_by_bucket {
        let entry = account_by_bucket.entry(*key).or_default();
        *entry = entry.add(*contribution);
    }
    for (bucket, contribution) in &totals.global_usage_by_bucket {
        let mut merged = UsageBreakdownAccumulator::default();
        if let Some(existing) = global_usage_by_bucket.get(bucket) {
            merged.merge_response(existing);
        }
        merged.merge_response(&contribution.clone().into_response());
        global_usage_by_bucket.insert(*bucket, merged.into_response());
    }
    for (key, contribution) in &totals.account_usage_by_bucket {
        let mut merged = UsageBreakdownAccumulator::default();
        if let Some(existing) = account_usage_by_bucket.get(key) {
            merged.merge_response(existing);
        }
        merged.merge_response(&contribution.clone().into_response());
        account_usage_by_bucket.insert(*key, merged.into_response());
    }
    for (bucket, contribution) in &totals.global_non_success_tokens_by_bucket {
        *global_non_success_tokens_by_bucket
            .entry(*bucket)
            .or_default() += contribution;
    }
    for (key, contribution) in &totals.account_non_success_tokens_by_bucket {
        *account_non_success_tokens_by_bucket
            .entry(*key)
            .or_default() += contribution;
    }
    let boundary_record_count = boundary_records.len();
    let overlay = SummaryCoverageOverlay {
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
    };
    next.coverage_overlay = Some(overlay);
    next.generation_fence = durable_fence;
    let expected_revision = next.revision;
    let unavailable_unmaterialized_range_count =
        next.unavailable_unmaterialized_archive_ranges.len();
    let unavailable_boundary_range_count = next.unavailable_boundary_archive_ranges.len();
    let thirty_day_start = Utc::now() - ChronoDuration::days(30);
    let unavailable_unmaterialized_30d_range_count = next
        .unavailable_unmaterialized_archive_ranges
        .iter()
        .filter(|range| range.start < Utc::now() && thirty_day_start < range.end)
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

pub(crate) fn subtract_summary_totals(left: StatsTotals, right: StatsTotals) -> StatsTotals {
    StatsTotals {
        total_count: left.total_count.saturating_sub(right.total_count),
        success_count: left.success_count.saturating_sub(right.success_count),
        failure_count: left.failure_count.saturating_sub(right.failure_count),
        total_tokens: left.total_tokens.saturating_sub(right.total_tokens),
        total_cost: (left.total_cost - right.total_cost).max(0.0),
        non_success_cost: (left.non_success_cost - right.non_success_cost).max(0.0),
    }
}
