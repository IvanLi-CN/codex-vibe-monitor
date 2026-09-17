struct SummaryProjectionBuildHydrationContext<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    prepared: SummaryProjectionBuildPreparation,
    maintenance: Option<StatsMaintenanceResponse>,
    records: Option<Vec<SummaryProjectionRecord>>,
    current_records: Option<Vec<SummaryProjectionRecord>>,
    hourly_buckets: Option<BTreeMap<i64, Vec<usize>>>,
    recent_indexes: Option<HashMap<Option<i64>, Vec<usize>>>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    account_ids: Option<HashSet<i64>>,
    active_in_progress_accounts: Option<HashSet<i64>>,
    global_in_progress: Option<InProgressSummarySnapshot>,
    account_ids_with_projection_data: Option<HashSet<i64>>,
    account_all_time_unavailable: bool,
    batched_all_time_by_account: HashMap<i64, StatsTotals>,
    all_time_built_at: Option<Instant>,
    rebuilt_all_time_account_ids: HashSet<i64>,
    in_progress_by_account: HashMap<Option<i64>, InProgressSummarySnapshot>,
}

struct SummaryProjectionBuildFinalizationParts<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    exact_horizon: ExactUtcRange,
    all_time_was_fully_rebuilt: bool,
    rebuilt_all_time_account_ids: HashSet<i64>,
    previous_all_time: PreviousSummaryProjectionAllTime,
    durable_terminal_sequence_watermark: u64,
    current_archive_admission_exceeded: bool,
    all_time_archive: SummaryProjectionAllTimeArchiveAdmissionSetup,
    live_indexes: SummaryProjectionLiveIndexes,
    window: SummaryProjectionBuildWindow,
    derived: SummaryProjectionBuildDerivedState,
    account_all_time_unavailable: bool,
    records: Vec<SummaryProjectionRecord>,
    current_records: Vec<SummaryProjectionRecord>,
    hourly_buckets: BTreeMap<i64, Vec<usize>>,
    recent_indexes: HashMap<Option<i64>, Vec<usize>>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    account_ids_with_projection_data: HashSet<i64>,
    account_ids: HashSet<i64>,
    in_progress_by_account: HashMap<Option<i64>, InProgressSummarySnapshot>,
    maintenance: StatsMaintenanceResponse,
}

impl<'a> SummaryProjectionBuildHydrationContext<'a> {
    fn new(
        state: &'a AppState,
        pool: &'a Pool<Sqlite>,
        mode: SummaryProjectionBuildMode,
        prepared: SummaryProjectionBuildPreparation,
    ) -> Self {
        Self {
            state,
            pool,
            mode,
            prepared,
            maintenance: None,
            records: None,
            current_records: None,
            hourly_buckets: None,
            recent_indexes: None,
            recent_index_complete: false,
            recent_index_overflow_at: None,
            account_ids: None,
            active_in_progress_accounts: None,
            global_in_progress: None,
            account_ids_with_projection_data: None,
            account_all_time_unavailable: false,
            batched_all_time_by_account: HashMap::new(),
            all_time_built_at: None,
            rebuilt_all_time_account_ids: HashSet::new(),
            in_progress_by_account: HashMap::new(),
        }
    }

    async fn materialize(&mut self) -> Result<()> {
        let initial = &mut self.prepared.initial;
        let records_by_invoke_id = std::mem::take(&mut initial.live_indexes.records_by_invoke_id);
        let current_records_by_invoke_id =
            std::mem::take(&mut initial.live_indexes.current_records_by_invoke_id);
        let known_account_ids = std::mem::take(&mut initial.live_indexes.known_account_ids);
        info!(
            ?self.mode,
            stage = "projection_materialization",
            "summary projection build stage started"
        );
        let materialization =
            materialize_summary_projection(SummaryProjectionMaterializationInput {
                state: self.state,
                pool: self.pool,
                records_by_invoke_id,
                current_records_by_invoke_id,
                current_archive_compact_rollup_proven_ids: &self
                    .prepared
                    .derived
                    .current_archive_compact_rollup_proven_ids,
                recent_index_complete: initial.live_indexes.recent_index_complete,
                recent_index_overflow_at: initial.live_indexes.recent_index_overflow_at,
                hourly_rollup_totals: &initial.window.hourly_rollup_totals,
                hourly_rollup_usage: &initial.window.hourly_rollup_usage,
                known_account_ids,
                archive_account_ids_by_file: &initial.previous_all_time.archive_account_ids_by_file,
            })
            .await?;
        self.maintenance = Some(materialization.maintenance);
        self.records = Some(materialization.records);
        self.current_records = Some(materialization.current_records);
        self.hourly_buckets = Some(materialization.hourly_buckets);
        self.recent_indexes = Some(materialization.recent_indexes);
        self.recent_index_complete = materialization.recent_index_complete;
        self.recent_index_overflow_at = materialization.recent_index_overflow_at;
        self.account_ids = Some(materialization.account_ids);
        self.active_in_progress_accounts = Some(materialization.active_in_progress_accounts);
        self.global_in_progress = Some(materialization.global_in_progress);
        self.account_ids_with_projection_data =
            Some(materialization.account_ids_with_projection_data);
        Ok(())
    }

    async fn load_all_time_rollup_totals(
        &mut self,
    ) -> Result<Option<HashMap<(i64, Option<i64>), StatsTotals>>> {
        let initial = &mut self.prepared.initial;
        if !initial.all_time_was_fully_rebuilt
            || initial.all_time_archive.all_time_archive_admission_exceeded
        {
            return Ok(None);
        }
        match load_summary_projection_rollup_totals(self.pool).await {
            Ok((rollups, _)) => Ok(Some(rollups)),
            Err(error) if summary_projection_rollup_admission_exceeded(&error) => {
                initial.all_time_archive.all_time_archive_admission_exceeded = true;
                self.prepared.derived.global_all_time_source_unavailable = true;
                self.account_all_time_unavailable = true;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn hydrate_all_time(&mut self) -> Result<()> {
        self.all_time_built_at = Some(Instant::now());
        let all_time_rollup_totals = self.load_all_time_rollup_totals().await?;
        let initial = &mut self.prepared.initial;
        let derived = &mut self.prepared.derived;
        self.account_all_time_unavailable |= derived.all_time_account_exact_source_unavailable
            || initial.all_time_archive.all_time_archive_admission_exceeded
            || initial
                .all_time_archive
                .all_time_account_manifest_admission_exceeded
            || derived.all_time_source_unavailable_from_archive_ranges;
        let previous_all_time_by_account = initial
            .all_time_was_fully_rebuilt
            .then(|| initial.previous_all_time.all_time_by_account.clone());
        let maintenance = self.maintenance.as_ref().expect("materialization must run");
        let records = self.records.as_mut().expect("materialization must run");
        let current_records = self
            .current_records
            .as_mut()
            .expect("materialization must run");
        let account_ids = self.account_ids.as_mut().expect("materialization must run");
        let account_ids_with_projection_data = self
            .account_ids_with_projection_data
            .as_mut()
            .expect("materialization must run");
        let aggregation =
            hydrate_summary_projection_all_time(SummaryProjectionAllTimeAggregationInput {
                pool: self.pool,
                mode: self.mode,
                has_any_completed_archive: initial.window.has_any_completed_archive,
                live_history_admission: initial.live_indexes.live_history_admission.clone(),
                all_time_archives: &initial.all_time_archive.all_time_archives,
                all_time_manifest_high_watermark_id: initial
                    .all_time_archive
                    .all_time_manifest_high_watermark_id,
                all_time_archive_manifest_sha256: &initial
                    .all_time_archive
                    .all_time_archive_manifest_sha256,
                all_time_archive_replay_coverage: &initial
                    .all_time_archive
                    .all_time_archive_replay_coverage,
                all_time_archive_account_ids_by_file: &initial
                    .all_time_archive
                    .all_time_archive_account_ids_by_file,
                all_time_archive_account_manifest_refreshed_paths: &initial
                    .all_time_archive
                    .all_time_archive_account_manifest_refreshed_paths,
                all_time_rollup_totals,
                exact_global_total_rollup_buckets: &derived.exact_global_total_rollup_buckets,
                exact_account_total_rollup_buckets: &derived.exact_account_total_rollup_buckets,
                rollup_live_cursor: initial.live_indexes.rollup_live_cursor,
                account_rollup_live_cursor: initial.live_indexes.account_rollup_live_cursor,
                records,
                current_records,
                account_ids,
                account_ids_with_projection_data,
                all_time_archive_admission_exceeded: &mut initial
                    .all_time_archive
                    .all_time_archive_admission_exceeded,
                all_time_account_manifest_admission_exceeded: &mut initial
                    .all_time_archive
                    .all_time_account_manifest_admission_exceeded,
                global_all_time_source_unavailable: derived.global_all_time_source_unavailable,
                account_all_time_unavailable: self.account_all_time_unavailable,
                previous_all_time_by_account: previous_all_time_by_account.as_ref(),
                all_time_by_account: &mut initial.previous_all_time.all_time_by_account,
                maintenance,
            })
            .await?;
        derived.global_all_time_source_unavailable = aggregation.global_all_time_source_unavailable;
        self.account_all_time_unavailable = aggregation.account_all_time_unavailable;
        self.batched_all_time_by_account = aggregation.batched_all_time_by_account;
        Ok(())
    }

    async fn populate_in_progress(&mut self) -> Result<()> {
        let initial = &mut self.prepared.initial;
        let account_ids = self.account_ids.as_ref().expect("materialization must run");
        let active_accounts = self
            .active_in_progress_accounts
            .as_ref()
            .expect("materialization must run");
        let global_in_progress = self.global_in_progress.expect("materialization must run");
        let maintenance = self.maintenance.as_ref().expect("materialization must run");
        for upstream_account_id in
            std::iter::once(None).chain(account_ids.iter().copied().map(Some))
        {
            let in_progress = match upstream_account_id {
                None => global_in_progress,
                Some(account_id) if active_accounts.contains(&account_id) => {
                    load_in_progress_summary_snapshot(
                        self.state,
                        InvocationSourceScope::All,
                        Some(account_id),
                    )
                    .await
                    .map_err(|error| {
                        anyhow!("summary projection in-progress hydration failed: {error:?}")
                    })?
                }
                Some(_) => InProgressSummarySnapshot::default(),
            };
            self.in_progress_by_account
                .insert(upstream_account_id, in_progress);
            if initial.all_time_was_fully_rebuilt
                && let Some(account_id) = upstream_account_id
            {
                if self.account_all_time_unavailable {
                    continue;
                }
                let totals = match self.batched_all_time_by_account.get(&account_id).copied() {
                    Some(totals) => totals,
                    None if self
                        .account_ids_with_projection_data
                        .as_ref()
                        .expect("materialization must run")
                        .contains(&account_id) =>
                    {
                        continue;
                    }
                    None => StatsTotals::default(),
                };
                let mut response = totals.into_response();
                response.non_success_cost = Some(totals.non_success_cost);
                response.maintenance = Some(maintenance.clone());
                initial
                    .previous_all_time
                    .all_time_by_account
                    .insert(Some(account_id), response);
                initial
                    .previous_all_time
                    .all_time_account_refreshed_at
                    .insert(
                        account_id,
                        self.all_time_built_at.expect("all-time hydration must run"),
                    );
                self.rebuilt_all_time_account_ids.insert(account_id);
            }
            if initial.all_time_was_fully_rebuilt
                && let Some(response) = initial
                    .previous_all_time
                    .all_time_by_account
                    .get_mut(&upstream_account_id)
            {
                response.in_progress_conversation_count = Some(in_progress.in_progress_count);
                response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
                response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
                response.in_progress_phase_counts = Some(in_progress.phase_counts);
            }
        }
        Ok(())
    }

    fn into_finalization_input(self) -> SummaryProjectionFinalizationInput<'a> {
        let SummaryProjectionBuildHydrationContext {
            state,
            pool,
            mode,
            prepared,
            maintenance,
            records,
            current_records,
            hourly_buckets,
            recent_indexes,
            recent_index_complete,
            recent_index_overflow_at,
            account_ids,
            account_ids_with_projection_data,
            account_all_time_unavailable,
            rebuilt_all_time_account_ids,
            in_progress_by_account,
            ..
        } = self;
        let SummaryProjectionBuildPreparation {
            initial:
                SummaryProjectionBuildInitialState {
                    build_started_at,
                    all_time_was_fully_rebuilt,
                    previous_all_time,
                    window,
                    current_archive:
                        SummaryProjectionCurrentArchiveAdmissionSetup {
                            current_archive_admission_exceeded,
                            ..
                        },
                    all_time_archive,
                    live_indexes,
                    metadata: _,
                    ..
                },
            durable_terminal_sequence_watermark,
            derived,
        } = prepared;
        build_summary_projection_finalization_input(SummaryProjectionBuildFinalizationParts {
            state,
            pool,
            mode,
            build_started_at,
            exact_horizon: ExactUtcRange {
                start: window.archive_start,
                end: window.end,
            },
            all_time_was_fully_rebuilt,
            rebuilt_all_time_account_ids,
            previous_all_time,
            durable_terminal_sequence_watermark,
            current_archive_admission_exceeded,
            all_time_archive,
            live_indexes,
            window,
            derived,
            account_all_time_unavailable,
            records: records.expect("materialization must run"),
            current_records: current_records.expect("materialization must run"),
            hourly_buckets: hourly_buckets.expect("materialization must run"),
            recent_indexes: recent_indexes.expect("materialization must run"),
            recent_index_complete,
            recent_index_overflow_at,
            account_ids_with_projection_data: account_ids_with_projection_data
                .expect("materialization must run"),
            account_ids: account_ids.expect("materialization must run"),
            in_progress_by_account,
            maintenance: maintenance.expect("materialization must run"),
        })
    }
}

fn build_summary_projection_finalization_input<'a>(
    parts: SummaryProjectionBuildFinalizationParts<'a>,
) -> SummaryProjectionFinalizationInput<'a> {
    let previous_all_time = parts.previous_all_time;
    let all_time_archive = parts.all_time_archive;
    let live_indexes = parts.live_indexes;
    let window = parts.window;
    let derived = parts.derived;
    SummaryProjectionFinalizationInput {
        state: parts.state,
        pool: parts.pool,
        mode: parts.mode,
        build_started_at: parts.build_started_at,
        exact_horizon: parts.exact_horizon,
        all_time_was_fully_rebuilt: parts.all_time_was_fully_rebuilt,
        rebuilt_all_time_account_ids: parts.rebuilt_all_time_account_ids,
        previous_all_time_refreshed_at: previous_all_time.all_time_refreshed_at,
        previous_all_time_manifest_admission_blocked_at: previous_all_time
            .all_time_manifest_admission_blocked_at,
        previous_all_time_account_manifest_admission_blocked_at: previous_all_time
            .all_time_account_manifest_admission_blocked_at,
        previous_global_all_time_eligible: previous_all_time.global_all_time_eligible,
        previous_account_all_time_eligible: previous_all_time.account_all_time_eligible,
        previous_global_all_time_coverage_fence: previous_all_time.global_all_time_coverage_fence,
        previous_account_all_time_coverage_fence: previous_all_time.account_all_time_coverage_fence,
        previous_all_time_terminal_coverage_complete: previous_all_time
            .all_time_terminal_coverage_complete,
        previous_all_time_terminal_sequence_watermark: previous_all_time
            .all_time_terminal_sequence_watermark,
        previous_all_time_account_terminal_sequence_watermarks: previous_all_time
            .all_time_account_terminal_sequence_watermarks,
        previous_all_time_account_persisted_live_terminal_invoke_ids: previous_all_time
            .all_time_account_persisted_live_terminal_invoke_ids,
        all_time_archive_admission_exceeded: all_time_archive.all_time_archive_admission_exceeded,
        all_time_account_manifest_admission_attempted: all_time_archive
            .all_time_account_manifest_admission_attempted,
        all_time_account_manifest_admission_exceeded: all_time_archive
            .all_time_account_manifest_admission_exceeded,
        all_time_account_refreshed_at: previous_all_time.all_time_account_refreshed_at,
        all_time_by_account: previous_all_time.all_time_by_account,
        global_all_time_source_unavailable: derived.global_all_time_source_unavailable,
        account_all_time_unavailable: parts.account_all_time_unavailable,
        historical_global_covered_terminal_invoke_ids: derived
            .historical_global_covered_terminal_invoke_ids,
        durable_terminal_sequence_watermark: parts.durable_terminal_sequence_watermark,
        live_high_watermark_id: live_indexes.live_high_watermark_id,
        account_rollup_live_cursor: live_indexes.account_rollup_live_cursor,
        completed_manifest_high_watermark_id: live_indexes.completed_manifest_high_watermark_id,
        records: parts.records,
        current_records: parts.current_records,
        hourly_buckets: parts.hourly_buckets,
        recent_indexes: parts.recent_indexes,
        recent_index_complete: parts.recent_index_complete,
        recent_index_overflow_at: parts.recent_index_overflow_at,
        hourly_rollup_totals: window.hourly_rollup_totals,
        hourly_rollup_non_success_tokens: window.hourly_rollup_non_success_tokens,
        hourly_rollup_usage: window.hourly_rollup_usage,
        exact_global_total_rollup_buckets: derived.exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets: derived.exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets: derived.exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets: derived.exact_account_usage_rollup_buckets,
        rollup_live_cursor: live_indexes.rollup_live_cursor,
        historical_live_coverage: previous_all_time.historical_live_coverage,
        previous_coverage_overlay: previous_all_time.coverage_overlay,
        account_ids_with_projection_data: parts.account_ids_with_projection_data,
        account_ids: parts.account_ids,
        archive_account_ids_by_file: previous_all_time.archive_account_ids_by_file,
        archive_actual_coverage_ranges: previous_all_time.archive_coverage_ranges_by_file,
        unavailable_unmaterialized_archive_buckets: derived
            .unavailable_unmaterialized_archive_buckets,
        unavailable_unmaterialized_archive_exact_ranges: derived
            .unavailable_unmaterialized_archive_exact_ranges,
        unavailable_boundary_archive_ranges: derived.unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges: derived
            .unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets: derived
            .unavailable_unmaterialized_archive_account_buckets,
        unavailable_unmaterialized_archive_account_exact_ranges: derived
            .unavailable_unmaterialized_archive_account_exact_ranges,
        unavailable_boundary_archive_account_ranges: derived
            .unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges: derived
            .unavailable_unmaterialized_archive_account_current_ranges,
        unavailable_exact_live_buckets: derived.unavailable_exact_live_buckets,
        unavailable_exact_live_account_buckets: derived.unavailable_exact_live_account_buckets,
        current_complete_archive_paths: derived.current_complete_archive_paths,
        current_source_unavailable: derived.current_source_unavailable,
        current_source_unavailable_from_rank: live_indexes.current_source_unavailable_from_rank,
        current_account_source_unavailable_from_rank: live_indexes
            .current_account_source_unavailable_from_rank,
        current_archive_admission_exceeded: parts.current_archive_admission_exceeded,
        in_progress_by_account: parts.in_progress_by_account,
        maintenance: parts.maintenance,
    }
}

async fn hydrate_summary_projection_build_with_context<'a>(
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    prepared: SummaryProjectionBuildPreparation,
) -> Result<SummaryProjectionFinalizationInput<'a>> {
    let mut context = SummaryProjectionBuildHydrationContext::new(state, pool, mode, prepared);
    context.materialize().await?;
    context.hydrate_all_time().await?;
    context.populate_in_progress().await?;
    Ok(context.into_finalization_input())
}
