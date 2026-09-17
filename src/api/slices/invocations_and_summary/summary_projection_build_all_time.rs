struct SummaryProjectionAllTimeAggregationInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    has_any_completed_archive: bool,
    live_history_admission: Option<crate::stats::BoundedLiveInvocationIds>,
    all_time_archives: &'a [crate::stats::ArchiveBatchPathRow],
    all_time_manifest_high_watermark_id: Option<i64>,
    all_time_archive_manifest_sha256: &'a HashMap<String, String>,
    all_time_archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    all_time_archive_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
    all_time_archive_account_manifest_refreshed_paths: &'a HashSet<String>,
    all_time_rollup_totals: Option<HashMap<(i64, Option<i64>), StatsTotals>>,
    exact_global_total_rollup_buckets: &'a HashSet<i64>,
    exact_account_total_rollup_buckets: &'a HashSet<i64>,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    records: &'a mut [SummaryProjectionRecord],
    current_records: &'a mut [SummaryProjectionRecord],
    account_ids: &'a mut HashSet<i64>,
    account_ids_with_projection_data: &'a mut HashSet<i64>,
    all_time_archive_admission_exceeded: &'a mut bool,
    all_time_account_manifest_admission_exceeded: &'a mut bool,
    global_all_time_source_unavailable: bool,
    account_all_time_unavailable: bool,
    previous_all_time_by_account: Option<&'a HashMap<Option<i64>, StatsResponse>>,
    all_time_by_account: &'a mut HashMap<Option<i64>, StatsResponse>,
    maintenance: &'a StatsMaintenanceResponse,
}

struct SummaryProjectionAllTimeAggregation {
    global_all_time_source_unavailable: bool,
    account_all_time_unavailable: bool,
    batched_all_time_by_account: HashMap<i64, StatsTotals>,
}

async fn hydrate_summary_projection_all_time(
    mut input: SummaryProjectionAllTimeAggregationInput<'_>,
) -> Result<SummaryProjectionAllTimeAggregation> {
    let Some(all_time_rollup_totals) = input.all_time_rollup_totals.take() else {
        return Ok(SummaryProjectionAllTimeAggregation {
            global_all_time_source_unavailable: input.global_all_time_source_unavailable,
            account_all_time_unavailable: input.account_all_time_unavailable,
            batched_all_time_by_account: HashMap::new(),
        });
    };
    let mut context = SummaryProjectionAllTimeHydrationContext::new(input, all_time_rollup_totals);
    let preparation = prepare_summary_projection_all_time(&mut context).await?;
    hydrate_summary_projection_all_time_global(&mut context, &preparation).await?;
    hydrate_summary_projection_all_time_accounts(&mut context, &preparation).await?;
    Ok(context.finish())
}
