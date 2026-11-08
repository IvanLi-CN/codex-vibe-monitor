use super::*;
pub(crate) struct SummaryProjectionAllTimeAggregationInput<'a> {
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) mode: SummaryProjectionBuildMode,
    pub(crate) has_any_completed_archive: bool,
    pub(crate) live_history_admission: Option<crate::stats::BoundedLiveInvocationIds>,
    pub(crate) all_time_archives: &'a [crate::stats::ArchiveBatchPathRow],
    pub(crate) all_time_manifest_high_watermark_id: Option<i64>,
    pub(crate) all_time_archive_manifest_sha256: &'a HashMap<String, String>,
    pub(crate) all_time_archive_replay_coverage:
        &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    pub(crate) all_time_archive_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
    pub(crate) all_time_archive_account_manifest_refreshed_paths: &'a HashSet<String>,
    pub(crate) all_time_rollup_totals: Option<HashMap<(i64, Option<i64>), StatsTotals>>,
    pub(crate) exact_global_total_rollup_buckets: &'a HashSet<i64>,
    pub(crate) exact_account_total_rollup_buckets: &'a HashSet<i64>,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) records: &'a mut [SummaryProjectionRecord],
    pub(crate) current_records: &'a mut [SummaryProjectionRecord],
    pub(crate) account_ids: &'a mut HashSet<i64>,
    pub(crate) account_ids_with_projection_data: &'a mut HashSet<i64>,
    pub(crate) all_time_archive_admission_exceeded: &'a mut bool,
    pub(crate) all_time_account_manifest_admission_exceeded: &'a mut bool,
    pub(crate) global_all_time_source_unavailable: bool,
    pub(crate) account_all_time_unavailable: bool,
    pub(crate) previous_all_time_by_account: Option<&'a HashMap<Option<i64>, StatsResponse>>,
    pub(crate) all_time_by_account: &'a mut HashMap<Option<i64>, StatsResponse>,
    pub(crate) maintenance: &'a StatsMaintenanceResponse,
}

pub(crate) struct SummaryProjectionAllTimeAggregation {
    pub(crate) global_all_time_source_unavailable: bool,
    pub(crate) account_all_time_unavailable: bool,
    pub(crate) batched_all_time_by_account: HashMap<i64, StatsTotals>,
}

pub(crate) async fn hydrate_summary_projection_all_time(
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
