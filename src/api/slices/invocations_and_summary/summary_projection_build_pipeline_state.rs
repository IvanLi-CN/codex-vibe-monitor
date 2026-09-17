#[derive(Default)]
struct SummaryProjectionBuildDerivedState {
    current_source_unavailable: bool,
    unavailable_unmaterialized_archive_buckets: BTreeSet<i64>,
    unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_buckets: BTreeSet<i64>,
    unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_exact_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_exact_ranges: Vec<ExactUtcRange>,
    unavailable_exact_live_buckets: BTreeSet<i64>,
    unavailable_exact_live_account_buckets: HashMap<i64, BTreeSet<i64>>,
    historical_global_covered_terminal_invoke_ids: HashSet<String>,
    exact_global_total_rollup_buckets: HashSet<i64>,
    exact_account_total_rollup_buckets: HashSet<i64>,
    exact_global_usage_rollup_buckets: HashSet<i64>,
    exact_account_usage_rollup_buckets: HashSet<i64>,
    exact_archive_buckets: HashSet<i64>,
    current_complete_archive_paths: HashSet<String>,
    current_archive_compact_rollup_proven_ids: HashSet<String>,
    all_time_source_unavailable_from_archive_ranges: bool,
    global_all_time_source_unavailable: bool,
    all_time_global_exact_source_unavailable: bool,
    all_time_account_exact_source_unavailable: bool,
}

struct SummaryProjectionBuildPipelineState {
    initial: SummaryProjectionBuildInitialState,
    durable_terminal_sequence_watermark: u64,
    derived: SummaryProjectionBuildDerivedState,
}

struct SummaryProjectionBuildExactSourceState {
    exact_bucket_requirements: HashSet<i64>,
    persisted_live_ids: HashSet<SummarySourceIdentity>,
    exact_record_budget: usize,
}

impl SummaryProjectionBuildPipelineState {
    fn new(
        initial: SummaryProjectionBuildInitialState,
        durable_terminal_sequence_watermark: u64,
    ) -> Self {
        Self {
            initial,
            durable_terminal_sequence_watermark,
            derived: SummaryProjectionBuildDerivedState::default(),
        }
    }

    fn initialize_coverage(&mut self, mode: SummaryProjectionBuildMode) {
        let preserve_archive_gaps = matches!(mode, SummaryProjectionBuildMode::RollingDelta);
        let previous = &self.initial.previous_all_time;
        let derived = &mut self.derived;
        if preserve_archive_gaps {
            derived.unavailable_boundary_archive_ranges =
                previous.unavailable_boundary_archive_ranges.clone();
            derived.unavailable_unmaterialized_archive_current_ranges = previous
                .unavailable_unmaterialized_archive_current_ranges
                .clone();
            derived.unavailable_boundary_archive_account_ranges =
                previous.unavailable_boundary_archive_account_ranges.clone();
            derived.unavailable_unmaterialized_archive_account_current_ranges = previous
                .unavailable_unmaterialized_archive_account_current_ranges
                .clone();
            derived.unavailable_unmaterialized_archive_account_exact_ranges = previous
                .unavailable_unmaterialized_archive_account_ranges
                .clone();
            derived.unavailable_unmaterialized_archive_exact_ranges =
                previous.unavailable_unmaterialized_archive_ranges.clone();
        }
        let exact_horizon = ExactUtcRange {
            start: self.initial.window.archive_start,
            end: self.initial.end,
        };
        if !self
            .initial
            .boundary
            .paged_boundary_manifest_unknown_coverage_ranges
            .is_empty()
        {
            derived
                .unavailable_unmaterialized_archive_exact_ranges
                .extend(
                    self.initial
                        .boundary
                        .paged_boundary_manifest_unknown_coverage_ranges
                        .iter()
                        .copied(),
                );
            derived
                .unavailable_unmaterialized_archive_current_ranges
                .extend(
                    self.initial
                        .boundary
                        .paged_boundary_manifest_unknown_coverage_ranges
                        .iter()
                        .copied(),
                );
        }
        derived
            .unavailable_unmaterialized_archive_account_exact_ranges
            .extend(
                self.initial
                    .current_archive
                    .current_archive_admission
                    .iter()
                    .filter_map(|archive| {
                        if archive.has_materialized_historical_rollups()
                            || !summary_projection_archive_has_coverage_bounds(archive)
                            || self
                                .initial
                                .current_archive
                                .current_archive_account_manifest_refreshed_paths
                                .contains(archive.file_path())
                        {
                            return None;
                        }
                        summary_projection_archive_overlap_range(archive, exact_horizon)
                    }),
            );
        derived
            .unavailable_unmaterialized_archive_account_exact_ranges
            .extend(
                self.initial
                    .boundary
                    .paged_boundary_manifest_unknown_account_coverage_ranges
                    .iter()
                    .copied(),
            );
        for candidate in self
            .initial
            .live_indexes
            .initial_live_admission_gaps
            .iter()
            .chain(
                self.initial
                    .live_indexes
                    .initial_live_admission_overflow
                    .iter(),
            )
        {
            let Some(bucket) = summary_projection_live_candidate_bucket(candidate) else {
                continue;
            };
            derived.unavailable_exact_live_buckets.insert(bucket);
            if let Some(account_id) = candidate
                .upstream_account_id
                .filter(|account_id| *account_id > 0)
            {
                derived
                    .unavailable_exact_live_account_buckets
                    .entry(account_id)
                    .or_default()
                    .insert(bucket);
            }
        }
    }

    fn promote_current_records(
        &mut self,
        state: &AppState,
        live_start: DateTime<Utc>,
    ) -> Result<()> {
        let live_indexes = &mut self.initial.live_indexes;
        let derived = &mut self.derived;
        let mut current_visible_persisted_live_ids = HashSet::<String>::new();
        let mut current_visible_candidates =
            HashMap::<Option<i64>, Vec<&SummaryProjectionRecord>>::new();
        for record in live_indexes.current_records_by_invoke_id.values() {
            current_visible_candidates
                .entry(None)
                .or_default()
                .push(record);
            if let Some(account_id) = record.row.upstream_account_id {
                current_visible_candidates
                    .entry(Some(account_id))
                    .or_default()
                    .push(record);
            }
        }
        for candidates in current_visible_candidates.values_mut() {
            candidates.sort_by(|left, right| {
                right
                    .occurred_at
                    .cmp(&left.occurred_at)
                    .then_with(|| right.row.id.cmp(&left.row.id))
            });
            current_visible_persisted_live_ids.extend(
                candidates
                    .iter()
                    .take(state.config.list_limit_max)
                    .filter(|record| record.is_persisted_live_record)
                    .map(|record| summary_projection_record_identity_key(record)),
            );
        }
        for record in live_indexes.current_records_by_invoke_id.values() {
            if !record.is_persisted_live_record
                || record.occurred_at >= live_start
                || !current_visible_persisted_live_ids
                    .contains(&summary_projection_record_identity_key(record))
                || summary_projection_records_contains_identity(
                    &live_indexes.records_by_invoke_id,
                    record.row.id,
                    &record.row.invoke_id,
                    &record.row.occurred_at,
                )
            {
                continue;
            }
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if live_indexes.records_by_invoke_id.len() >= summary_projection_exact_record_limit() {
                derived.unavailable_exact_live_buckets.insert(bucket);
                continue;
            }
            let next_rolling_record_bytes = live_indexes
                .exact_record_bytes
                .saturating_add(summary_projection_preview_row_bytes(&record.row));
            if let Err(error) = ensure_summary_projection_resident_record_bytes(
                next_rolling_record_bytes,
                live_indexes.current_record_bytes,
            ) {
                if !summary_projection_resident_record_budget_exceeded(&error) {
                    return Err(error);
                }
                derived.unavailable_exact_live_buckets.insert(bucket);
                continue;
            }
            live_indexes.exact_record_bytes = next_rolling_record_bytes;
            let record_key =
                summary_projection_record_insert_key(&live_indexes.records_by_invoke_id, record);
            live_indexes
                .records_by_invoke_id
                .insert(record_key, record.clone());
        }
        Ok(())
    }

    async fn discover_archive_accounts(&mut self, mode: SummaryProjectionBuildMode) -> Result<()> {
        let initial = &mut self.initial;
        let derived = &mut self.derived;
        let mut archive_actual_coverage_ranges =
            initial.metadata.archive_coverage_ranges_by_file.clone();
        info!(
            ?mode,
            stage = "archive_account_discovery",
            "summary projection build stage started"
        );
        discover_summary_projection_archive_account_ids(
            SummaryProjectionArchiveAccountDiscoveryState {
                mode,
                build_started_at: initial.build_started_at,
                archive_start: initial.window.archive_start,
                end: initial.end,
                archive_replay_coverage: &initial.metadata.archive_replay_coverage,
                archive_row_counts: &initial.metadata.archive_row_counts,
                archive_manifest_sha256: &initial.metadata.archive_manifest_sha256,
                archive_account_ids_by_file: &mut initial.metadata.archive_account_ids_by_file,
                archive_actual_coverage_ranges: &mut archive_actual_coverage_ranges,
                known_account_ids: &mut initial.live_indexes.known_account_ids,
                cached_archive_account_id_count: &mut initial
                    .metadata
                    .cached_archive_account_id_count,
                all_time_source_unavailable_from_archive_ranges: &mut derived
                    .all_time_source_unavailable_from_archive_ranges,
            },
            &initial.boundary.archives,
        )
        .await?;
        initial.metadata.archive_coverage_ranges_by_file = archive_actual_coverage_ranges;
        Ok(())
    }

    async fn plan_archive_sources(
        &mut self,
        pool: &Pool<Sqlite>,
        mode: SummaryProjectionBuildMode,
    ) -> Result<()> {
        let initial = &mut self.initial;
        let derived = &mut self.derived;
        let replacement = mark_summary_projection_archive_replacement_buckets(
            &initial.boundary.archives,
            initial.window.archive_start,
            initial.end,
            &initial.window.protected_boundary_buckets,
            &initial.metadata.archive_replay_coverage,
        )?;
        derived.exact_global_total_rollup_buckets = replacement.exact_global_total_rollup_buckets;
        derived.exact_account_total_rollup_buckets = replacement.exact_account_total_rollup_buckets;
        derived.exact_global_usage_rollup_buckets = replacement.exact_global_usage_rollup_buckets;
        derived.exact_account_usage_rollup_buckets = replacement.exact_account_usage_rollup_buckets;
        derived.exact_archive_buckets = initial.window.protected_boundary_buckets.clone();
        derived
            .exact_archive_buckets
            .extend(derived.exact_global_total_rollup_buckets.iter().copied());
        derived
            .exact_archive_buckets
            .extend(derived.exact_account_total_rollup_buckets.iter().copied());
        derived
            .exact_archive_buckets
            .extend(derived.exact_global_usage_rollup_buckets.iter().copied());
        derived
            .exact_archive_buckets
            .extend(derived.exact_account_usage_rollup_buckets.iter().copied());
        summary_projection_ensure_exact_bucket_budget(&derived.exact_archive_buckets)?;
        extend_summary_projection_archive_fallback_buckets(
            SummaryProjectionArchiveFallbackBucketInput {
                archives: &initial.boundary.archives,
                archive_start: initial.window.archive_start,
                end: initial.end,
                known_account_ids: &initial.live_indexes.known_account_ids,
                archive_replay_coverage: &initial.metadata.archive_replay_coverage,
                hourly_rollup_totals: &initial.window.hourly_rollup_totals,
                hourly_rollup_usage: &initial.window.hourly_rollup_usage,
                exact_archive_buckets: &mut derived.exact_archive_buckets,
            },
        )?;
        plan_summary_projection_boundary_manifests(SummaryProjectionBoundaryPlanningInput {
            pool,
            mode,
            build_started_at: initial.build_started_at,
            exact_horizon: ExactUtcRange {
                start: initial.window.archive_start,
                end: initial.end,
            },
            paged_boundary_manifest_high_watermark_id: if mode.includes_all_time() {
                initial.boundary.paged_boundary_manifest_high_watermark_id
            } else {
                None
            },
            protected_boundary_buckets: &initial.window.protected_boundary_buckets,
            hourly_rollup_totals: &initial.window.hourly_rollup_totals,
            hourly_rollup_usage: &initial.window.hourly_rollup_usage,
            known_account_ids: &mut initial.live_indexes.known_account_ids,
            exact_archive_buckets: &mut derived.exact_archive_buckets,
            exact_global_total_rollup_buckets: &mut derived.exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets: &mut derived.exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets: &mut derived.exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets: &mut derived.exact_account_usage_rollup_buckets,
            unavailable_unmaterialized_archive_buckets: &mut derived
                .unavailable_unmaterialized_archive_buckets,
            unavailable_boundary_archive_ranges: &mut derived.unavailable_boundary_archive_ranges,
            unavailable_unmaterialized_archive_current_ranges: &mut derived
                .unavailable_unmaterialized_archive_current_ranges,
            unavailable_unmaterialized_archive_account_buckets: &mut derived
                .unavailable_unmaterialized_archive_account_buckets,
            unavailable_boundary_archive_account_ranges: &mut derived
                .unavailable_boundary_archive_account_ranges,
            unavailable_unmaterialized_archive_account_current_ranges: &mut derived
                .unavailable_unmaterialized_archive_account_current_ranges,
        })
        .await
    }

    async fn admit_exact_live_sources(
        &mut self,
        state: &AppState,
        pool: &Pool<Sqlite>,
        mode: SummaryProjectionBuildMode,
    ) -> Result<SummaryProjectionBuildExactSourceState> {
        let initial = &mut self.initial;
        let derived = &mut self.derived;
        let admission =
            admit_summary_projection_exact_live_sources(SummaryProjectionExactLiveAdmissionInput {
                pool,
                state,
                mode,
                build_started_at: initial.build_started_at,
                archive_start: initial.window.archive_start,
                live_start: initial.window.live_start,
                live_high_watermark_id: initial.live_indexes.live_high_watermark_id,
                rollup_live_cursor: initial.live_indexes.rollup_live_cursor,
                account_rollup_live_cursor: initial.live_indexes.account_rollup_live_cursor,
                exact_archive_buckets: &derived.exact_archive_buckets,
                protected_boundary_buckets: &initial.window.protected_boundary_buckets,
                exact_global_total_rollup_buckets: &derived.exact_global_total_rollup_buckets,
                exact_account_total_rollup_buckets: &derived.exact_account_total_rollup_buckets,
                exact_global_usage_rollup_buckets: &derived.exact_global_usage_rollup_buckets,
                exact_account_usage_rollup_buckets: &derived.exact_account_usage_rollup_buckets,
                hourly_rollup_totals: &initial.window.hourly_rollup_totals,
                hourly_rollup_usage: &initial.window.hourly_rollup_usage,
                previous_historical_live_coverage: initial
                    .previous_all_time
                    .historical_live_coverage
                    .as_ref(),
                previous_unavailable_exact_live_ranges: &initial
                    .previous_all_time
                    .unavailable_exact_live_ranges,
                previous_unavailable_exact_live_account_ranges: &initial
                    .previous_all_time
                    .unavailable_exact_live_account_ranges,
                previous_persisted_live_terminal_invoke_ids: &initial
                    .previous_all_time
                    .persisted_live_terminal_invoke_ids,
                live_preview_cache: &mut initial.live_indexes.live_preview_cache,
                records_by_invoke_id: &mut initial.live_indexes.records_by_invoke_id,
                exact_record_bytes: &mut initial.live_indexes.exact_record_bytes,
                current_record_bytes: initial.live_indexes.current_record_bytes,
                recent_index_complete: initial.live_indexes.recent_index_complete,
                recent_index_overflow_at: initial.live_indexes.recent_index_overflow_at,
                unavailable_exact_live_buckets: &mut derived.unavailable_exact_live_buckets,
                unavailable_exact_live_account_buckets: &mut derived
                    .unavailable_exact_live_account_buckets,
                historical_global_covered_terminal_invoke_ids: &mut derived
                    .historical_global_covered_terminal_invoke_ids,
                known_account_ids: &mut initial.live_indexes.known_account_ids,
            })
            .await?;
        initial.previous_all_time.historical_live_coverage = admission.historical_live_coverage;
        Ok(SummaryProjectionBuildExactSourceState {
            exact_bucket_requirements: admission.exact_bucket_requirements,
            persisted_live_ids: admission.persisted_live_ids,
            exact_record_budget: admission.exact_record_budget,
        })
    }

    async fn hydrate_archives(
        &mut self,
        pool: &Pool<Sqlite>,
        mode: SummaryProjectionBuildMode,
        exact_sources: SummaryProjectionBuildExactSourceState,
    ) -> Result<()> {
        let initial = &mut self.initial;
        let derived = &mut self.derived;
        let exact_horizon = ExactUtcRange {
            start: initial.window.archive_start,
            end: initial.end,
        };
        let mut exact_record_budget = exact_sources.exact_record_budget;
        let mut exact_record_bytes = initial.live_indexes.exact_record_bytes;
        hydrate_summary_projection_build_archives(SummaryProjectionBuildArchiveHydrationInput {
            pool,
            mode,
            archive_start: initial.window.archive_start,
            end: initial.end,
            exact_horizon,
            build_started_at: initial.build_started_at,
            archives: initial.boundary.archives.clone(),
            paged_boundary_manifest_high_watermark_id: initial
                .boundary
                .paged_boundary_manifest_high_watermark_id,
            archive_replay_coverage: &initial.metadata.archive_replay_coverage,
            archive_row_counts: &initial.metadata.archive_row_counts,
            archive_manifest_sha256: &initial.metadata.archive_manifest_sha256,
            hourly_rollup_totals: &initial.window.hourly_rollup_totals,
            hourly_rollup_usage: &initial.window.hourly_rollup_usage,
            exact_archive_buckets: &derived.exact_archive_buckets,
            protected_boundary_buckets: &initial.window.protected_boundary_buckets,
            exact_bucket_requirements: &exact_sources.exact_bucket_requirements,
            usage_rollup_progress: &initial.metadata.usage_rollup_progress,
            known_account_ids: &initial.live_indexes.known_account_ids,
            persisted_live_ids: &exact_sources.persisted_live_ids,
            current_record_bytes: initial.live_indexes.current_record_bytes,
            records_by_invoke_id: &mut initial.live_indexes.records_by_invoke_id,
            exact_record_budget: &mut exact_record_budget,
            exact_record_bytes: &mut exact_record_bytes,
            unavailable_unmaterialized_archive_buckets: &mut derived
                .unavailable_unmaterialized_archive_buckets,
            unavailable_boundary_archive_ranges: &mut derived.unavailable_boundary_archive_ranges,
            unavailable_unmaterialized_archive_current_ranges: &mut derived
                .unavailable_unmaterialized_archive_current_ranges,
            all_time_source_unavailable_from_archive_ranges: &mut derived
                .all_time_source_unavailable_from_archive_ranges,
        })
        .await?;
        initial.live_indexes.exact_record_bytes = exact_record_bytes;
        for record in initial.live_indexes.records_by_invoke_id.values_mut() {
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if derived.exact_global_total_rollup_buckets.contains(&bucket) {
                record.global_rollup_covered = false;
            }
            if derived.exact_account_total_rollup_buckets.contains(&bucket) {
                record.account_rollup_covered = false;
            }
            if derived.exact_global_usage_rollup_buckets.contains(&bucket) {
                record.usage_global_rollup_covered = false;
            }
            if derived.exact_account_usage_rollup_buckets.contains(&bucket) {
                record.usage_account_rollup_covered = false;
            }
        }
        Ok(())
    }

    async fn admit_current_archives_and_overlay(
        &mut self,
        state: &AppState,
        pool: &Pool<Sqlite>,
        mode: SummaryProjectionBuildMode,
    ) -> Result<()> {
        let initial = &mut self.initial;
        let derived = &mut self.derived;
        let admission = admit_summary_projection_current_archives(
            SummaryProjectionCurrentArchiveAdmissionInput {
                pool,
                mode,
                build_started_at: initial.build_started_at,
                live_start: initial.window.live_start,
                end: initial.end,
                current_archive_admission: &initial.current_archive.current_archive_admission,
                current_archive_admission_exceeded: initial
                    .current_archive
                    .current_archive_admission_exceeded,
                archive_row_counts: &initial.metadata.archive_row_counts,
                archive_replay_coverage: &initial.metadata.archive_replay_coverage,
                hourly_rollup_totals: &initial.window.hourly_rollup_totals,
                hourly_rollup_usage: &initial.window.hourly_rollup_usage,
                known_account_ids: &initial.live_indexes.known_account_ids,
                records_by_invoke_id: &initial.live_indexes.records_by_invoke_id,
                exact_record_bytes: initial.live_indexes.exact_record_bytes,
                current_records_by_invoke_id: &mut initial
                    .live_indexes
                    .current_records_by_invoke_id,
                current_record_bytes: &mut initial.live_indexes.current_record_bytes,
                current_source_unavailable: &mut derived.current_source_unavailable,
            },
        )
        .await?;
        derived.current_complete_archive_paths = admission.complete_archive_paths;
        derived.current_archive_compact_rollup_proven_ids = admission.compact_rollup_proven_ids;
        derived.global_all_time_source_unavailable = derived
            .all_time_global_exact_source_unavailable
            || initial.all_time_archive.all_time_archive_admission_exceeded
            || derived.all_time_source_unavailable_from_archive_ranges;
        apply_summary_projection_runtime_overlay(SummaryProjectionRuntimeOverlayInput {
            state,
            mode,
            records_by_invoke_id: &mut initial.live_indexes.records_by_invoke_id,
            current_records_by_invoke_id: &mut initial.live_indexes.current_records_by_invoke_id,
            exact_record_bytes: &mut initial.live_indexes.exact_record_bytes,
            current_record_bytes: &mut initial.live_indexes.current_record_bytes,
            unavailable_exact_live_buckets: &mut derived.unavailable_exact_live_buckets,
            current_source_unavailable: &mut derived.current_source_unavailable,
        })?;
        Ok(())
    }

    fn into_preparation(mut self) -> SummaryProjectionBuildPreparation {
        self.initial.previous_all_time.archive_account_ids_by_file =
            std::mem::take(&mut self.initial.metadata.archive_account_ids_by_file);
        self.initial
            .previous_all_time
            .archive_coverage_ranges_by_file =
            std::mem::take(&mut self.initial.metadata.archive_coverage_ranges_by_file);
        SummaryProjectionBuildPreparation {
            initial: self.initial,
            durable_terminal_sequence_watermark: self.durable_terminal_sequence_watermark,
            derived: self.derived,
        }
    }
}
