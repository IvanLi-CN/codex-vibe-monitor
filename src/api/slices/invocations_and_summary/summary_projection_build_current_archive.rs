struct SummaryProjectionCurrentArchiveAdmissionInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    live_start: DateTime<Utc>,
    end: DateTime<Utc>,
    current_archive_admission: &'a [crate::stats::ArchiveBatchPathRow],
    current_archive_admission_exceeded: bool,
    archive_row_counts: &'a HashMap<String, i64>,
    archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &'a HashSet<i64>,
    records_by_invoke_id: &'a HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: usize,
    current_records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    current_record_bytes: &'a mut usize,
    current_source_unavailable: &'a mut bool,
}

struct SummaryProjectionCurrentArchiveAdmission {
    complete_archive_paths: HashSet<String>,
    compact_rollup_proven_ids: HashSet<String>,
}

struct SummaryProjectionCurrentArchiveState<'a> {
    pool: &'a Pool<Sqlite>,
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    current_range: ExactUtcRange,
    archive_row_counts: &'a HashMap<String, i64>,
    archive_manifest_sha256: &'a HashMap<String, String>,
    archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &'a HashSet<i64>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_archive_candidate_records: &'a mut HashMap<String, SummaryProjectionRecord>,
    current_archive_candidate_budget: &'a mut usize,
    current_archive_candidate_bytes: &'a mut usize,
    exact_record_bytes: usize,
    current_record_bytes: usize,
    current_complete_archive_paths: &'a mut HashSet<String>,
    current_archive_compact_rollup_proven_ids: &'a mut HashSet<String>,
    current_source_unavailable: &'a mut bool,
}

async fn admit_summary_projection_current_archives(
    input: SummaryProjectionCurrentArchiveAdmissionInput<'_>,
) -> Result<SummaryProjectionCurrentArchiveAdmission> {
    let SummaryProjectionCurrentArchiveAdmissionInput {
        pool,
        mode,
        build_started_at,
        live_start,
        end,
        current_archive_admission,
        current_archive_admission_exceeded,
        archive_row_counts,
        archive_replay_coverage,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
        records_by_invoke_id,
        exact_record_bytes,
        current_records_by_invoke_id,
        current_record_bytes,
        current_source_unavailable,
    } = input;
    let current_materialized_archives = if current_archive_admission_exceeded {
        Vec::new()
    } else {
        current_archive_admission
            .iter()
            .filter(|archive| archive.has_materialized_historical_rollups())
            .collect::<Vec<_>>()
    };
    info!(
        ?mode,
        stage = "current_archive_admission",
        "summary projection build stage started"
    );
    let current_materialized_sha256 =
        load_current_materialized_archive_sha256(pool, &current_materialized_archives).await?;
    let mut current_archive_candidate_records = HashMap::<String, SummaryProjectionRecord>::new();
    let mut current_archive_compact_rollup_proven_ids = HashSet::<String>::new();
    let mut current_archive_candidate_budget = 0usize;
    let mut current_archive_candidate_bytes = 0usize;
    let current_persisted_live_ids = records_by_invoke_id
        .values()
        .filter(|record| record.is_persisted_live_record)
        .map(summary_projection_record_source_identity)
        .collect::<HashSet<_>>();
    let mut current_complete_archive_paths = HashSet::new();
    let current_range = ExactUtcRange {
        start: live_start,
        end,
    };
    for archive in current_materialized_archives {
        let Some(archive_range) = summary_projection_archive_overlap_range(archive, current_range)
        else {
            continue;
        };
        hydrate_summary_projection_current_archive(SummaryProjectionCurrentArchiveState {
            pool,
            archive,
            archive_range,
            current_range,
            archive_row_counts,
            archive_manifest_sha256: &current_materialized_sha256,
            archive_replay_coverage,
            hourly_rollup_totals,
            hourly_rollup_usage,
            known_account_ids,
            persisted_live_ids: &current_persisted_live_ids,
            current_archive_candidate_records: &mut current_archive_candidate_records,
            current_archive_candidate_budget: &mut current_archive_candidate_budget,
            current_archive_candidate_bytes: &mut current_archive_candidate_bytes,
            exact_record_bytes,
            current_record_bytes: *current_record_bytes,
            current_complete_archive_paths: &mut current_complete_archive_paths,
            current_archive_compact_rollup_proven_ids:
                &mut current_archive_compact_rollup_proven_ids,
            current_source_unavailable,
        })
        .await?;
    }
    merge_summary_projection_current_archive_candidates(
        current_archive_candidate_records,
        current_records_by_invoke_id,
        exact_record_bytes,
        current_record_bytes,
        current_source_unavailable,
    )?;
    info!(
        ?mode,
        stage = "current_archive_admission",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(SummaryProjectionCurrentArchiveAdmission {
        complete_archive_paths: current_complete_archive_paths,
        compact_rollup_proven_ids: current_archive_compact_rollup_proven_ids,
    })
}

async fn load_current_materialized_archive_sha256(
    pool: &Pool<Sqlite>,
    archives: &[&crate::stats::ArchiveBatchPathRow],
) -> Result<HashMap<String, String>> {
    let paths = archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    load_summary_projection_archive_manifest_sha256(pool, &paths).await
}

async fn hydrate_summary_projection_current_archive(
    mut state: SummaryProjectionCurrentArchiveState<'_>,
) -> Result<()> {
    if state.archive.has_materialized_historical_rollups()
        && state
            .archive_row_counts
            .get(state.archive.file_path())
            .is_some_and(|count| *count > summary_projection_exact_record_limit() as i64)
        && summary_projection_archive_is_fully_within_exact_horizon(
            state.archive,
            state.current_range,
        )
    {
        *state.current_source_unavailable = true;
        return Ok(());
    }
    let Some(manifest_sha256) = state.archive_manifest_sha256.get(state.archive.file_path()) else {
        *state.current_source_unavailable = true;
        return Ok(());
    };
    let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
        state.archive,
        "summary-projection-current",
    )
    .await?
    else {
        return Ok(());
    };
    let candidate_ids_before_archive = state
        .current_archive_candidate_records
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    require_summary_projection_archive_file_sha256(state.archive, manifest_sha256)?;
    let archive_range = state.archive_range;
    let merge_result =
        merge_summary_projection_current_archive_records(&archive_pool, &mut state, archive_range)
            .await;
    require_summary_projection_archive_file_sha256(state.archive, manifest_sha256)?;
    archive_pool.close().await;
    drop(temp_cleanup);
    match merge_result {
        Ok(()) => {
            let replay_coverage = summary_projection_effective_replay_coverage(
                state.archive,
                state
                    .archive_replay_coverage
                    .get(state.archive.file_path())
                    .copied()
                    .unwrap_or_default(),
            );
            mark_summary_projection_current_archive_rollup_proof(
                &mut state,
                &candidate_ids_before_archive,
                replay_coverage,
            );
            if summary_projection_archive_is_fully_represented_for_current(
                state.archive,
                state.current_range,
            ) {
                state
                    .current_complete_archive_paths
                    .insert(state.archive.file_path().to_string());
            }
        }
        Err(error) if summary_projection_archive_raw_admission_exceeded(&error) => {
            *state.current_source_unavailable = true;
        }
        Err(error) => {
            return Err(anyhow!(
                "summary projection current archive hydration failed for {}: {error:?}",
                state.archive.file_path()
            ));
        }
    }
    Ok(())
}

async fn merge_summary_projection_current_archive_records(
    archive_pool: &Pool<Sqlite>,
    state: &mut SummaryProjectionCurrentArchiveState<'_>,
    archive_range: ExactUtcRange,
) -> Result<()> {
    merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
        archive_pool,
        persisted_live_ids: state.persisted_live_ids,
        hourly_rollup_totals: state.hourly_rollup_totals,
        hourly_rollup_usage: state.hourly_rollup_usage,
        archive_has_materialized_rollups: false,
        overall_rollup_archive_replayed: None,
        account_rollup_archive_replayed: None,
        usage_rollup_archive_replayed: None,
        exact_range: archive_range,
        protected_boundary_buckets: &HashSet::new(),
        usage_rollup_cursor: None,
        known_account_ids: state.known_account_ids,
        records_by_invoke_id: state.current_archive_candidate_records,
        exact_record_budget: state.current_archive_candidate_budget,
        exact_record_bytes: state.current_archive_candidate_bytes,
        resident_current_record_bytes: state
            .exact_record_bytes
            .saturating_add(state.current_record_bytes),
        snapshot_rows: None,
    })
    .await
}

fn mark_summary_projection_current_archive_rollup_proof(
    state: &mut SummaryProjectionCurrentArchiveState<'_>,
    candidate_ids_before_archive: &HashSet<String>,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
) {
    if !replay_coverage.supports_unavailable_archive() {
        return;
    }
    for (invoke_id, record) in state.current_archive_candidate_records.iter() {
        if candidate_ids_before_archive.contains(invoke_id) {
            continue;
        }
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        let global_proven = state.hourly_rollup_totals.contains_key(&(bucket, None))
            && state.hourly_rollup_usage.contains_key(&(bucket, None));
        let account_proven = record.row.upstream_account_id.is_none_or(|account_id| {
            state
                .hourly_rollup_totals
                .contains_key(&(bucket, Some(account_id)))
                && state
                    .hourly_rollup_usage
                    .contains_key(&(bucket, Some(account_id)))
        });
        if global_proven && account_proven {
            state
                .current_archive_compact_rollup_proven_ids
                .insert(summary_projection_record_identity_key(record));
        }
    }
}

fn merge_summary_projection_current_archive_candidates(
    candidates: HashMap<String, SummaryProjectionRecord>,
    current_records_by_invoke_id: &mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: usize,
    current_record_bytes: &mut usize,
    current_source_unavailable: &mut bool,
) -> Result<()> {
    for record in candidates.into_values() {
        if summary_projection_records_contains_identity(
            current_records_by_invoke_id,
            record.row.id,
            &record.row.invoke_id,
            &record.row.occurred_at,
        ) {
            continue;
        }
        let next_current_record_bytes =
            current_record_bytes.saturating_add(summary_projection_preview_row_bytes(&record.row));
        if let Err(error) = ensure_summary_projection_resident_record_bytes(
            exact_record_bytes,
            next_current_record_bytes,
        ) {
            if !summary_projection_resident_record_budget_exceeded(&error) {
                return Err(error);
            }
            *current_source_unavailable = true;
            break;
        }
        *current_record_bytes = next_current_record_bytes;
        let record_key =
            summary_projection_record_insert_key(current_records_by_invoke_id, &record);
        current_records_by_invoke_id.insert(record_key, record);
    }
    Ok(())
}
