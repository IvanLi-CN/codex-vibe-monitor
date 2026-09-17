#[derive(Default)]
struct SummaryV2ArchiveTotals {
    global: StatsTotals,
    accounts: HashMap<i64, StatsTotals>,
    global_usage: UsageBreakdownAccumulator,
    account_usage: HashMap<i64, UsageBreakdownAccumulator>,
    global_by_bucket: HashMap<i64, StatsTotals>,
    account_by_bucket: HashMap<(i64, i64), StatsTotals>,
    global_usage_by_bucket: HashMap<i64, UsageBreakdownAccumulator>,
    account_usage_by_bucket: HashMap<(i64, i64), UsageBreakdownAccumulator>,
    global_non_success_tokens_by_bucket: HashMap<i64, i64>,
    account_non_success_tokens_by_bucket: HashMap<(i64, i64), i64>,
    global_coverage_buckets: HashSet<i64>,
    account_coverage_buckets: HashSet<i64>,
    replacement_buckets: HashSet<i64>,
    materialized_buckets: HashSet<i64>,
    materialized_months_without_coverage: HashSet<String>,
    global_usage_gap_buckets: HashSet<i64>,
    account_usage_gap_buckets: HashSet<i64>,
    boundary_records: Vec<SummaryProjectionRecord>,
}

fn summary_v2_archive_totals_from_coverage_overlay(
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

type SummaryArchiveSnapshotProofIdentity = (i64, String);
type SummaryV2ArchiveManifest = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

async fn load_summary_v2_archive_totals_excluding(
    pool: &Pool<Sqlite>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
) -> Result<SummaryV2ArchiveTotals> {
    let archives = sqlx::query_as::<_, SummaryV2ArchiveManifest>(
        "SELECT id, sha256, historical_rollups_materialized_at, coverage_start_at, coverage_end_at, month_key, file_path
         FROM archive_batches
         WHERE dataset = 'codex_invocations' AND status = 'completed'
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_all(pool)
    .await
    .context("summary V2 archive totals manifest lookup failed")?;
    load_summary_v2_archive_totals_from_archives(pool, archives, excluded_source_identities, &[])
        .await
}

async fn load_summary_v2_archive_proof_identities(
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

async fn load_summary_v2_archive_proof_identities_in_range(
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

async fn load_summary_v2_archive_totals_for_proof_identities(
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

async fn load_summary_v2_archive_totals_for_proof_identities_with_boundaries(
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

async fn load_summary_materialized_archive_coverage(
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

struct SummaryV2ArchiveTotalsMergeInput<'a> {
    pool: &'a Pool<Sqlite>,
    archive: SummaryV2ArchiveManifest,
    excluded_source_identities: &'a HashSet<SummarySourceIdentity>,
    boundary_ranges: &'a [ExactUtcRange],
    totals: &'a mut SummaryV2ArchiveTotals,
}

async fn load_summary_v2_archive_totals_from_archives(
    pool: &Pool<Sqlite>,
    archives: Vec<SummaryV2ArchiveManifest>,
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
    boundary_ranges: &[ExactUtcRange],
) -> Result<SummaryV2ArchiveTotals> {
    let mut totals = SummaryV2ArchiveTotals::default();
    for archive in archives {
        merge_summary_v2_archive_totals(SummaryV2ArchiveTotalsMergeInput {
            pool,
            archive,
            excluded_source_identities,
            boundary_ranges,
            totals: &mut totals,
        })
        .await?;
    }
    Ok(totals)
}

fn add_summary_v2_archive_coverage(
    totals: &mut SummaryV2ArchiveTotals,
    coverage_start: Option<&str>,
    coverage_end: Option<&str>,
    materialized: bool,
) {
    let (Some(start), Some(end)) = (
        coverage_start.and_then(parse_to_utc_datetime),
        coverage_end.and_then(parse_to_utc_datetime),
    ) else {
        return;
    };
    let mut bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
    let end_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
    let mut bucket_count = 0usize;
    while bucket <= end_bucket && bucket_count < 4096 {
        totals.global_coverage_buckets.insert(bucket);
        totals.account_coverage_buckets.insert(bucket);
        if materialized {
            totals.materialized_buckets.insert(bucket);
        }
        bucket = bucket.saturating_add(3_600);
        bucket_count += 1;
    }
}

async fn merge_summary_v2_archive_totals(
    input: SummaryV2ArchiveTotalsMergeInput<'_>,
) -> Result<()> {
    let SummaryV2ArchiveTotalsMergeInput {
        pool,
        archive,
        excluded_source_identities,
        boundary_ranges,
        totals,
    } = input;
    let (
        archive_batch_id,
        manifest_sha256,
        materialized_at,
        coverage_start,
        coverage_end,
        month_key,
        file_path,
    ) = archive;
    if materialized_at.is_some() {
        add_summary_v2_archive_coverage(
            totals,
            coverage_start.as_deref(),
            coverage_end.as_deref(),
            true,
        );
        if (coverage_start.is_none() || coverage_end.is_none())
            && let Some(month_key) = month_key.as_deref()
        {
            totals
                .materialized_months_without_coverage
                .insert(month_key.to_string());
        }
    }
    if !summary_archive_snapshot_has_final_proof(pool, archive_batch_id, &manifest_sha256).await? {
        return Ok(());
    }
    let archive_boundary_intersects = summary_v2_archive_intersects_boundary(
        coverage_start.as_deref(),
        coverage_end.as_deref(),
        boundary_ranges,
    );
    add_summary_v2_archive_coverage(
        totals,
        coverage_start.as_deref(),
        coverage_end.as_deref(),
        false,
    );
    if materialized_at.is_some() {
        merge_summary_v2_materialized_archive(SummaryV2MaterializedArchiveInput {
            pool,
            archive_batch_id,
            manifest_sha256,
            coverage_start,
            coverage_end,
            month_key,
            file_path,
            archive_boundary_intersects,
            excluded_source_identities,
            boundary_ranges,
            totals,
        })
        .await
    } else {
        merge_summary_v2_unmaterialized_archive(SummaryV2UnmaterializedArchiveInput {
            pool,
            archive_batch_id,
            manifest_sha256,
            excluded_source_identities,
            boundary_ranges,
            totals,
        })
        .await
    }
}

fn summary_v2_archive_intersects_boundary(
    coverage_start: Option<&str>,
    coverage_end: Option<&str>,
    boundary_ranges: &[ExactUtcRange],
) -> bool {
    let (Some(start), Some(end)) = (
        coverage_start.and_then(parse_to_utc_datetime),
        coverage_end.and_then(parse_to_utc_datetime),
    ) else {
        return false;
    };
    boundary_ranges
        .iter()
        .any(|range| range.start < end && start < range.end)
}

struct SummaryV2MaterializedArchiveInput<'a> {
    pool: &'a Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: String,
    coverage_start: Option<String>,
    coverage_end: Option<String>,
    month_key: Option<String>,
    file_path: String,
    archive_boundary_intersects: bool,
    excluded_source_identities: &'a HashSet<SummarySourceIdentity>,
    boundary_ranges: &'a [ExactUtcRange],
    totals: &'a mut SummaryV2ArchiveTotals,
}

async fn merge_summary_v2_materialized_archive(
    input: SummaryV2MaterializedArchiveInput<'_>,
) -> Result<()> {
    let SummaryV2MaterializedArchiveInput {
        pool,
        archive_batch_id,
        manifest_sha256,
        coverage_start,
        coverage_end,
        month_key,
        file_path,
        archive_boundary_intersects,
        excluded_source_identities,
        boundary_ranges,
        totals,
    } = input;
    let replay =
        load_summary_projection_archive_replay_coverage(pool, std::slice::from_ref(&file_path))
            .await?
            .remove(&file_path)
            .unwrap_or_default();
    if !replay.usage_breakdown {
        add_summary_v2_usage_gap_buckets(
            totals,
            coverage_start.as_deref(),
            coverage_end.as_deref(),
            month_key.as_deref(),
        );
    }
    if archive_boundary_intersects
        && !(replay.overall && replay.account_stats && replay.usage_breakdown)
    {
        append_summary_v2_boundary_records(
            pool,
            archive_batch_id,
            &manifest_sha256,
            boundary_ranges,
            excluded_source_identities,
            true,
            totals,
        )
        .await?;
    }
    Ok(())
}

fn add_summary_v2_usage_gap_buckets(
    totals: &mut SummaryV2ArchiveTotals,
    coverage_start: Option<&str>,
    coverage_end: Option<&str>,
    month_key: Option<&str>,
) {
    let buckets = if let (Some(start), Some(end)) = (
        coverage_start.and_then(parse_to_utc_datetime),
        coverage_end.and_then(parse_to_utc_datetime),
    ) {
        let first = align_bucket_epoch(start.timestamp(), 3_600, 0);
        let last = align_bucket_epoch(end.timestamp(), 3_600, 0);
        (first..=last).step_by(3_600).collect::<Vec<_>>()
    } else {
        month_key
            .and_then(|month| {
                crate::stats::archive_bucket_start_epochs_from_bounds(Some(month), None, None).ok()
            })
            .map(|buckets| buckets.into_iter().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for bucket in buckets.into_iter().take(4096) {
        totals.global_usage_gap_buckets.insert(bucket);
        totals.account_usage_gap_buckets.insert(bucket);
    }
}

async fn append_summary_v2_boundary_records(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
    boundary_ranges: &[ExactUtcRange],
    excluded_source_identities: &HashSet<SummarySourceIdentity>,
    archive_has_materialized_rollups: bool,
    totals: &mut SummaryV2ArchiveTotals,
) -> Result<()> {
    let mut pages = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch(pool);
    while let Some(payload) = pages.try_next().await? {
        for record in decode_summary_archive_snapshot_v2_payload(&payload)? {
            let identity = SummarySourceIdentity {
                row_id: record.id,
                invoke_id: record.invoke_id.clone(),
                occurred_at: record.occurred_at.clone(),
            };
            if excluded_source_identities.contains(&identity) {
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
            if totals.boundary_records.len() >= summary_projection_exact_record_limit() {
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
                archive_has_materialized_rollups,
                account_archive_totals_fallback_included: false,
                row: preview,
            });
        }
    }
    Ok(())
}

struct SummaryV2UnmaterializedArchiveInput<'a> {
    pool: &'a Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: String,
    excluded_source_identities: &'a HashSet<SummarySourceIdentity>,
    boundary_ranges: &'a [ExactUtcRange],
    totals: &'a mut SummaryV2ArchiveTotals,
}

async fn merge_summary_v2_unmaterialized_archive(
    input: SummaryV2UnmaterializedArchiveInput<'_>,
) -> Result<()> {
    let SummaryV2UnmaterializedArchiveInput {
        pool,
        archive_batch_id,
        manifest_sha256,
        excluded_source_identities,
        boundary_ranges,
        totals,
    } = input;
    let mut pages = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(&manifest_sha256)
    .fetch(pool);
    while let Some(payload) = pages.try_next().await? {
        let records = decode_summary_archive_snapshot_v2_payload(&payload)?;
        merge_summary_v2_unmaterialized_page(
            pool,
            records,
            boundary_ranges,
            excluded_source_identities,
            totals,
        )
        .await?;
    }
    Ok(())
}

async fn merge_summary_v2_unmaterialized_page(
    pool: &Pool<Sqlite>,
    records: Vec<SummaryArchiveSnapshotV2Record>,
    boundary_ranges: &[ExactUtcRange],
    _excluded_source_identities: &HashSet<SummarySourceIdentity>,
    totals: &mut SummaryV2ArchiveTotals,
) -> Result<()> {
    for record in &records {
        let bucket = parse_to_utc_datetime(&record.occurred_at)
            .map(|occurred_at| align_bucket_epoch(occurred_at.timestamp(), 3_600, 0));
        if let Some(bucket) = bucket {
            if totals.global_coverage_buckets.len() < 4096 {
                totals.global_coverage_buckets.insert(bucket);
            }
            if totals.account_coverage_buckets.len() < 4096 {
                totals.account_coverage_buckets.insert(bucket);
            }
        }
    }
    let live_identity = load_summary_v2_live_identities(pool, &records).await?;
    for record in records {
        merge_summary_v2_unmaterialized_record(totals, record, &live_identity, boundary_ranges)?;
    }
    Ok(())
}

async fn load_summary_v2_live_identities(
    pool: &Pool<Sqlite>,
    records: &[SummaryArchiveSnapshotV2Record],
) -> Result<HashSet<(i64, String, String)>> {
    let mut identity_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT id, invoke_id, occurred_at FROM codex_invocations WHERE id IN (",
    );
    {
        let mut separated = identity_query.separated(", ");
        for record in records {
            separated.push_bind(record.id);
        }
    }
    identity_query.push(") OR invoke_id IN (");
    {
        let mut separated = identity_query.separated(", ");
        for record in records {
            separated.push_bind(&record.invoke_id);
        }
    }
    identity_query.push(")");
    Ok(identity_query
        .build_query_as::<(i64, String, String)>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

fn merge_summary_v2_unmaterialized_record(
    totals: &mut SummaryV2ArchiveTotals,
    record: SummaryArchiveSnapshotV2Record,
    live_identity: &HashSet<(i64, String, String)>,
    boundary_ranges: &[ExactUtcRange],
) -> Result<()> {
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
        range.start <= summary_record.occurred_at && summary_record.occurred_at < range.end
    }) {
        if totals.boundary_records.len() >= summary_projection_exact_record_limit() {
            return Err(anyhow!(
                "summary V2 boundary record budget exceeded while publishing coverage overlay"
            ));
        }
        totals.boundary_records.push(summary_record.clone());
    }
    if live_identity.contains(&(
        preview.id,
        preview.invoke_id.clone(),
        preview.occurred_at.clone(),
    )) {
        return Ok(());
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
        let bucket_totals = totals
            .account_by_bucket
            .entry((bucket, account_id))
            .or_default();
        *bucket_totals = bucket_totals.add(record_totals);
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
    Ok(())
}
