struct SummaryProjectionPagedBoundaryHydrationInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    exact_horizon: ExactUtcRange,
    build_started_at: Instant,
    paged_boundary_manifest_high_watermark_id: Option<i64>,
    exact_archive_buckets: &'a HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_bucket_requirements: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_record_bytes: usize,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
}

struct SummaryProjectionPagedBoundaryPageInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
    after_id: Option<i64>,
    exact_archive_buckets: &'a HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_bucket_requirements: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_record_bytes: usize,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    paged_boundary_raw_archive_admissions: &'a mut usize,
}

struct SummaryProjectionPagedBoundaryArchiveInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    exact_horizon: ExactUtcRange,
    archive: crate::stats::ArchiveBatchPathRow,
    page_manifest_sha256: &'a HashMap<String, String>,
    page_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    page_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
    page_account_manifest_refreshed_paths: &'a HashSet<String>,
    exact_archive_buckets: &'a HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_bucket_requirements: &'a HashSet<i64>,
    usage_rollup_progress: &'a HashMap<String, i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_record_bytes: usize,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    paged_boundary_raw_archive_admissions: &'a mut usize,
}

struct SummaryProjectionPagedBoundaryArchivePlan<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    archive: crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    manifest_sha256: &'a str,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_replayed: Option<bool>,
    usage_replayed: Option<bool>,
    account_ids: &'a HashSet<i64>,
    exact_ranges: Vec<ExactUtcRange>,
    exact_archive_buckets: &'a HashSet<i64>,
    exact_bucket_requirements: &'a HashSet<i64>,
    usage_rollup_progress: &'a HashMap<String, i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_record_bytes: usize,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    paged_boundary_raw_archive_admissions: &'a mut usize,
}

async fn hydrate_summary_projection_paged_boundary_archives(
    input: SummaryProjectionPagedBoundaryHydrationInput<'_>,
) -> Result<()> {
    let SummaryProjectionPagedBoundaryHydrationInput {
        pool,
        mode,
        exact_horizon,
        build_started_at,
        paged_boundary_manifest_high_watermark_id,
        exact_archive_buckets,
        protected_boundary_buckets,
        exact_bucket_requirements,
        hourly_rollup_totals,
        hourly_rollup_usage,
        persisted_live_ids,
        current_record_bytes,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
    } = input;
    if !mode.includes_all_time() {
        return Ok(());
    }
    let Some(high_watermark_id) = paged_boundary_manifest_high_watermark_id else {
        return Ok(());
    };
    info!(
        ?mode,
        stage = "paged_boundary_archive_hydration",
        "summary projection build stage started"
    );
    #[cfg(test)]
    pause_summary_projection_test_interleave(
        pool,
        mode,
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    )
    .await?;
    let mut after_id = None;
    let mut paged_boundary_raw_archive_admissions = 0usize;
    loop {
        let Some(next_after_id) = hydrate_summary_projection_paged_boundary_page(
            SummaryProjectionPagedBoundaryPageInput {
                pool,
                mode,
                exact_horizon,
                high_watermark_id,
                after_id,
                exact_archive_buckets,
                protected_boundary_buckets,
                exact_bucket_requirements,
                hourly_rollup_totals,
                hourly_rollup_usage,
                persisted_live_ids,
                current_record_bytes,
                records_by_invoke_id,
                exact_record_budget,
                exact_record_bytes,
                unavailable_unmaterialized_archive_buckets,
                unavailable_boundary_archive_ranges,
                unavailable_unmaterialized_archive_current_ranges,
                paged_boundary_raw_archive_admissions: &mut paged_boundary_raw_archive_admissions,
            },
        )
        .await?
        else {
            break;
        };
        after_id = Some(next_after_id);
    }
    info!(
        ?mode,
        stage = "paged_boundary_archive_hydration",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(())
}

async fn hydrate_summary_projection_paged_boundary_page(
    input: SummaryProjectionPagedBoundaryPageInput<'_>,
) -> Result<Option<i64>> {
    let SummaryProjectionPagedBoundaryPageInput {
        pool,
        mode,
        exact_horizon,
        high_watermark_id,
        after_id,
        exact_archive_buckets,
        protected_boundary_buckets,
        exact_bucket_requirements,
        hourly_rollup_totals,
        hourly_rollup_usage,
        persisted_live_ids,
        current_record_bytes,
        records_by_invoke_id,
        exact_record_budget,
        exact_record_bytes,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        paged_boundary_raw_archive_admissions,
    } = input;
    let page = load_summary_projection_boundary_manifest_page(
        pool,
        Some(exact_horizon),
        after_id,
        high_watermark_id,
    )
    .await?;
    if page.archives.is_empty() {
        return Ok(None);
    }
    let page_paths = page
        .archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let page_manifest_sha256 =
        load_summary_projection_archive_manifest_sha256(pool, &page_paths).await?;
    let page_replay_coverage =
        load_summary_projection_archive_replay_coverage(pool, &page_paths).await?;
    let usage_rollup_progress =
        load_usage_breakdown_archive_progress_by_file_path(pool, &page_paths)
            .await
            .map_err(|error| {
                anyhow!(
                    "summary projection paged boundary usage progress hydration failed: {error:?}"
                )
            })?;
    let page_account_ids_by_file =
        load_summary_projection_archive_manifest_account_sets(pool, &page_paths).await?;
    for archive in page.archives {
        hydrate_summary_projection_paged_boundary_archive(
            SummaryProjectionPagedBoundaryArchiveInput {
                pool,
                mode,
                exact_horizon,
                archive,
                page_manifest_sha256: &page_manifest_sha256,
                page_replay_coverage: &page_replay_coverage,
                page_account_ids_by_file: &page_account_ids_by_file,
                page_account_manifest_refreshed_paths: &page.account_manifest_refreshed_paths,
                exact_archive_buckets,
                protected_boundary_buckets,
                exact_bucket_requirements,
                usage_rollup_progress: &usage_rollup_progress,
                hourly_rollup_totals,
                hourly_rollup_usage,
                persisted_live_ids,
                current_record_bytes,
                records_by_invoke_id,
                exact_record_budget,
                exact_record_bytes,
                unavailable_unmaterialized_archive_buckets,
                unavailable_boundary_archive_ranges,
                unavailable_unmaterialized_archive_current_ranges,
                paged_boundary_raw_archive_admissions,
            },
        )
        .await?;
    }
    Ok(page.next_after_id)
}

fn summary_projection_paged_boundary_exact_ranges(
    input: &SummaryProjectionPagedBoundaryArchiveInput<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_replayed: Option<bool>,
    usage_replayed: Option<bool>,
    account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    let mut exact_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive.has_materialized_historical_rollups(),
        SummaryProjectionArchiveReplayFlags {
            overall: Some(replay_coverage.overall),
            account: account_replayed,
            usage: usage_replayed,
        },
        archive_range,
        input.exact_archive_buckets,
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        account_ids,
    );
    if !input.mode.includes_all_time() {
        exact_ranges = summary_projection_split_ranges_by_buckets(
            exact_ranges,
            input.protected_boundary_buckets,
        )
        .0;
    }
    exact_ranges
}

impl<'a> SummaryProjectionPagedBoundaryArchivePlan<'a> {
    fn from_input(input: SummaryProjectionPagedBoundaryArchiveInput<'a>) -> Result<Option<Self>> {
        let Some(archive_range) =
            summary_projection_archive_overlap_range(&input.archive, input.exact_horizon)
        else {
            return Ok(None);
        };
        let account_ids = input
            .page_account_ids_by_file
            .get(input.archive.file_path())
            .expect("page account manifest includes every requested archive path");
        let account_manifest_complete = input
            .page_account_manifest_refreshed_paths
            .contains(input.archive.file_path());
        let replay_coverage = summary_projection_effective_replay_coverage(
            &input.archive,
            input
                .page_replay_coverage
                .get(input.archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        let (account_replayed, usage_replayed) = if !account_manifest_complete {
            (Some(false), Some(replay_coverage.usage_breakdown))
        } else if account_ids.is_empty() {
            (Some(true), Some(replay_coverage.usage_breakdown))
        } else {
            (None, None)
        };
        let exact_ranges = summary_projection_paged_boundary_exact_ranges(
            &input,
            &input.archive,
            archive_range,
            replay_coverage,
            account_replayed,
            usage_replayed,
            account_ids,
        );
        if exact_ranges.is_empty() {
            return Ok(None);
        }
        let manifest_sha256 = input
            .page_manifest_sha256
            .get(input.archive.file_path())
            .ok_or_else(|| {
                anyhow!(
                    "summary projection paged archive manifest SHA is missing for {}",
                    input.archive.file_path()
                )
            })?
            .as_str();
        Ok(Some(Self {
            pool: input.pool,
            mode: input.mode,
            archive: input.archive,
            archive_range,
            manifest_sha256,
            replay_coverage,
            account_replayed,
            usage_replayed,
            account_ids,
            exact_ranges,
            exact_archive_buckets: input.exact_archive_buckets,
            exact_bucket_requirements: input.exact_bucket_requirements,
            usage_rollup_progress: input.usage_rollup_progress,
            hourly_rollup_totals: input.hourly_rollup_totals,
            hourly_rollup_usage: input.hourly_rollup_usage,
            persisted_live_ids: input.persisted_live_ids,
            current_record_bytes: input.current_record_bytes,
            records_by_invoke_id: input.records_by_invoke_id,
            exact_record_budget: input.exact_record_budget,
            exact_record_bytes: input.exact_record_bytes,
            unavailable_unmaterialized_archive_buckets: input
                .unavailable_unmaterialized_archive_buckets,
            unavailable_boundary_archive_ranges: input.unavailable_boundary_archive_ranges,
            unavailable_unmaterialized_archive_current_ranges: input
                .unavailable_unmaterialized_archive_current_ranges,
            paged_boundary_raw_archive_admissions: input.paged_boundary_raw_archive_admissions,
        }))
    }

    async fn hydrate(mut self) -> Result<()> {
        if let Err(error) = summary_projection_admit_paged_boundary_raw_archive(
            self.paged_boundary_raw_archive_admissions,
        ) {
            if !summary_projection_archive_raw_admission_exceeded(&error) {
                return Err(error);
            }
            return self
                .hydrate_snapshot_or_mark(
                    "used verified Summary Archive Snapshot V2 after paged archive admission pressure",
                )
                .await;
        }
        if Path::new(self.archive.file_path()).exists()
            && verify_summary_projection_archive_file_sha256(&self.archive, self.manifest_sha256)?
                == SummaryProjectionArchiveFileIdentity::Unavailable
        {
            return self
                .hydrate_snapshot_or_mark(
                    "used verified Summary Archive Snapshot V2 after paged archive identity failure",
                )
                .await;
        }
        let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
            &self.archive,
            "summary-projection-paged-boundary",
        )
        .await?
        else {
            return self
                .hydrate_snapshot_or_mark(
                    "used verified Summary Archive Snapshot V2 when paged archive source was unavailable",
                )
                .await;
        };
        require_summary_projection_archive_file_sha256(&self.archive, self.manifest_sha256)?;
        let records_before_archive = self.records_by_invoke_id.clone();
        let exact_record_budget_before_archive = *self.exact_record_budget;
        let exact_record_bytes_before_archive = *self.exact_record_bytes;
        let merge_result = self.merge_archive(&archive_pool).await;
        require_summary_projection_archive_file_sha256(&self.archive, self.manifest_sha256)?;
        archive_pool.close().await;
        drop(temp_cleanup);
        if let Err(error) = merge_result {
            if !summary_projection_archive_raw_admission_exceeded(&error) {
                return Err(anyhow!(
                    "summary projection paged boundary archive hydration failed for {}: {error:?}",
                    self.archive.file_path()
                ));
            }
            *self.records_by_invoke_id = records_before_archive;
            *self.exact_record_budget = exact_record_budget_before_archive;
            *self.exact_record_bytes = exact_record_bytes_before_archive;
            self.mark_unavailable()?;
        }
        Ok(())
    }

    async fn hydrate_snapshot_or_mark(&mut self, fallback_message: &'static str) -> Result<()> {
        if self.try_snapshot(fallback_message).await? {
            return Ok(());
        }
        self.mark_unavailable()
    }

    async fn try_snapshot(&mut self, fallback_message: &'static str) -> Result<bool> {
        let Some(snapshot_rows) = load_summary_projection_snapshot_records(
            self.pool,
            self.archive.file_path(),
            &self.exact_ranges,
        )
        .await?
        else {
            return Ok(false);
        };
        if self.merge_snapshot(snapshot_rows).await.is_ok() {
            debug!(
                ?self.mode,
                archive = self.archive.file_path(),
                stage = "archive_snapshot_v2_fallback",
                "{}",
                fallback_message
            );
            return Ok(true);
        }
        Ok(false)
    }

    async fn merge_snapshot(
        &mut self,
        snapshot_rows: Vec<SummaryProjectionArchiveRow>,
    ) -> Result<()> {
        merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
            archive_pool: self.pool,
            persisted_live_ids: self.persisted_live_ids,
            hourly_rollup_totals: self.hourly_rollup_totals,
            hourly_rollup_usage: self.hourly_rollup_usage,
            archive_has_materialized_rollups: self.archive.has_materialized_historical_rollups(),
            overall_rollup_archive_replayed: Some(self.replay_coverage.overall),
            account_rollup_archive_replayed: self.account_replayed,
            usage_rollup_archive_replayed: self.usage_replayed,
            exact_range: self.archive_range,
            protected_boundary_buckets: self.exact_archive_buckets,
            usage_rollup_cursor: self
                .usage_rollup_progress
                .get(self.archive.file_path())
                .copied(),
            known_account_ids: self.account_ids,
            records_by_invoke_id: self.records_by_invoke_id,
            exact_record_budget: self.exact_record_budget,
            exact_record_bytes: self.exact_record_bytes,
            resident_current_record_bytes: self.current_record_bytes,
            snapshot_rows: Some(snapshot_rows),
        })
        .await
    }

    async fn merge_archive(&mut self, archive_pool: &Pool<Sqlite>) -> Result<()> {
        merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
            archive_pool,
            persisted_live_ids: self.persisted_live_ids,
            hourly_rollup_totals: self.hourly_rollup_totals,
            hourly_rollup_usage: self.hourly_rollup_usage,
            archive_has_materialized_rollups: self.archive.has_materialized_historical_rollups(),
            overall_rollup_archive_replayed: Some(self.replay_coverage.overall),
            account_rollup_archive_replayed: self.account_replayed,
            usage_rollup_archive_replayed: self.usage_replayed,
            exact_range: self.archive_range,
            protected_boundary_buckets: self.exact_archive_buckets,
            usage_rollup_cursor: self
                .usage_rollup_progress
                .get(self.archive.file_path())
                .copied(),
            known_account_ids: self.account_ids,
            records_by_invoke_id: self.records_by_invoke_id,
            exact_record_budget: self.exact_record_budget,
            exact_record_bytes: self.exact_record_bytes,
            resident_current_record_bytes: self.current_record_bytes,
            snapshot_rows: None,
        })
        .await
    }

    fn mark_unavailable(&mut self) -> Result<()> {
        summary_projection_mark_unavailable_archive_ranges_by_requirement(
            self.unavailable_unmaterialized_archive_buckets,
            self.unavailable_boundary_archive_ranges,
            self.archive.has_materialized_historical_rollups(),
            &self.exact_ranges,
            self.exact_bucket_requirements,
        )?;
        self.unavailable_unmaterialized_archive_current_ranges
            .push(self.archive_range);
        Ok(())
    }
}

async fn hydrate_summary_projection_paged_boundary_archive(
    input: SummaryProjectionPagedBoundaryArchiveInput<'_>,
) -> Result<()> {
    let Some(plan) = SummaryProjectionPagedBoundaryArchivePlan::from_input(input)? else {
        return Ok(());
    };
    plan.hydrate().await
}
