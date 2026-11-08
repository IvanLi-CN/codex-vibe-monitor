use super::*;

pub(crate) struct SummaryProjectionLiveRowsQuery<'a> {
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) range: ExactUtcRange,
    pub(crate) high_watermark_id: i64,
    pub(crate) min_id_exclusive: Option<i64>,
    pub(crate) upstream_account_id: Option<Option<i64>>,
    pub(crate) limit: usize,
    pub(crate) in_progress_only: bool,
    pub(crate) preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    pub(crate) telemetry: UpstreamAccountActivityPreviewReadTelemetry,
}

pub(crate) async fn query_summary_projection_live_rows_with_query(
    input: SummaryProjectionLiveRowsQuery<'_>,
) -> Result<SummaryProjectionLiveAdmission, anyhow::Error> {
    query_summary_projection_live_rows_with_budget(
        input.pool,
        input.source_scope,
        input.range,
        input.high_watermark_id,
        input.min_id_exclusive,
        input.upstream_account_id,
        input.limit,
        input.in_progress_only,
        SUMMARY_PROJECTION_MAX_PREVIEW_BYTES,
        input.preview_cache,
        input.telemetry,
    )
    .await
}

pub(crate) struct SummaryProjectionHistoricalLiveReuseInput<'a> {
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) previous_coverage: Option<&'a SummaryProjectionHistoricalLiveCoverage>,
    pub(crate) range: ExactUtcRange,
    pub(crate) high_watermark_id: i64,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    pub(crate) hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    pub(crate) fully_admitted_live_buckets: &'a BTreeSet<i64>,
    pub(crate) previous_unavailable_global_ranges: &'a [ExactUtcRange],
    pub(crate) previous_unavailable_account_ranges: &'a HashMap<i64, Vec<ExactUtcRange>>,
    pub(crate) previous_terminal_identities: &'a HashSet<String>,
    pub(crate) pending_terminal_identities: &'a HashSet<String>,
    pub(crate) unavailable_global_buckets: &'a mut BTreeSet<i64>,
    pub(crate) unavailable_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    pub(crate) global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
}

pub(crate) async fn reuse_summary_projection_historical_live_coverage_with_input(
    input: SummaryProjectionHistoricalLiveReuseInput<'_>,
) -> Result<SummaryProjectionHistoricalLiveCoverage> {
    reuse_summary_projection_historical_live_coverage(
        input.pool,
        input.previous_coverage,
        input.range,
        input.high_watermark_id,
        input.rollup_live_cursor,
        input.account_rollup_live_cursor,
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.fully_admitted_live_buckets,
        input.previous_unavailable_global_ranges,
        input.previous_unavailable_account_ranges,
        input.previous_terminal_identities,
        input.pending_terminal_identities,
        input.unavailable_global_buckets,
        input.unavailable_account_buckets,
        input.global_covered_terminal_invoke_ids,
    )
    .await
}

pub(crate) struct SummaryProjectionHistoricalLiveCoverageInput<'a> {
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) range: ExactUtcRange,
    pub(crate) high_watermark_id: i64,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    pub(crate) hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    pub(crate) fully_admitted_live_buckets: &'a BTreeSet<i64>,
    pub(crate) pending_terminal_identities: &'a HashSet<String>,
    pub(crate) unavailable_global_buckets: &'a mut BTreeSet<i64>,
    pub(crate) unavailable_account_buckets: &'a mut HashMap<i64, BTreeSet<i64>>,
    pub(crate) global_covered_terminal_invoke_ids: &'a mut HashSet<String>,
}

pub(crate) async fn mark_summary_projection_uncovered_historical_live_ranges_with_input(
    input: SummaryProjectionHistoricalLiveCoverageInput<'_>,
) -> Result<()> {
    mark_summary_projection_uncovered_historical_live_ranges(
        input.pool,
        input.range,
        input.high_watermark_id,
        input.rollup_live_cursor,
        input.account_rollup_live_cursor,
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.fully_admitted_live_buckets,
        input.pending_terminal_identities,
        input.unavailable_global_buckets,
        input.unavailable_account_buckets,
        input.global_covered_terminal_invoke_ids,
    )
    .await
}

pub(crate) struct SummaryProjectionArchiveReplayFlags {
    pub(crate) overall: Option<bool>,
    pub(crate) account: Option<bool>,
    pub(crate) usage: Option<bool>,
}

pub(crate) fn summary_projection_archive_exact_ranges_with_replay_flags(
    archive_has_materialized_rollups: bool,
    replay_flags: SummaryProjectionArchiveReplayFlags,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    summary_projection_archive_exact_ranges_with_coverage(
        archive_has_materialized_rollups,
        replay_flags.overall,
        replay_flags.account,
        replay_flags.usage,
        exact_range,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
    )
}

pub(crate) struct SummaryProjectionExactReplacementInput<'a> {
    pub(crate) archive_has_materialized_rollups: bool,
    pub(crate) replay_coverage: SummaryProjectionArchiveReplayCoverage,
    pub(crate) exact_range: ExactUtcRange,
    pub(crate) _protected_boundary_buckets: &'a HashSet<i64>,
    pub(crate) exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    pub(crate) exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    pub(crate) exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    pub(crate) exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
}

pub(crate) fn summary_projection_mark_exact_replacement_buckets_with_input(
    input: SummaryProjectionExactReplacementInput<'_>,
) -> Result<()> {
    summary_projection_mark_exact_replacement_buckets(
        input.archive_has_materialized_rollups,
        input.replay_coverage,
        input.exact_range,
        input._protected_boundary_buckets,
        input.exact_global_total_rollup_buckets,
        input.exact_account_total_rollup_buckets,
        input.exact_global_usage_rollup_buckets,
        input.exact_account_usage_rollup_buckets,
    )
}

pub(crate) struct SummaryProjectionArchiveMergeInput<'a> {
    pub(crate) archive_pool: &'a Pool<Sqlite>,
    pub(crate) persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    pub(crate) hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    pub(crate) hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    pub(crate) archive_has_materialized_rollups: bool,
    pub(crate) overall_rollup_archive_replayed: Option<bool>,
    pub(crate) account_rollup_archive_replayed: Option<bool>,
    pub(crate) usage_rollup_archive_replayed: Option<bool>,
    pub(crate) exact_range: ExactUtcRange,
    pub(crate) protected_boundary_buckets: &'a HashSet<i64>,
    pub(crate) usage_rollup_cursor: Option<i64>,
    pub(crate) known_account_ids: &'a HashSet<i64>,
    pub(crate) records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    pub(crate) exact_record_budget: &'a mut usize,
    pub(crate) exact_record_bytes: &'a mut usize,
    pub(crate) resident_current_record_bytes: usize,
    pub(crate) snapshot_rows: Option<Vec<SummaryProjectionArchiveRow>>,
}

pub(crate) async fn merge_summary_projection_archive_records_with_input(
    input: SummaryProjectionArchiveMergeInput<'_>,
) -> Result<()> {
    merge_summary_projection_archive_records_with_coverage(
        input.archive_pool,
        input.archive_pool,
        input.persisted_live_ids,
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.archive_has_materialized_rollups,
        input.overall_rollup_archive_replayed,
        input.account_rollup_archive_replayed,
        input.usage_rollup_archive_replayed,
        input.exact_range,
        input.protected_boundary_buckets,
        input.usage_rollup_cursor,
        input.known_account_ids,
        input.records_by_invoke_id,
        input.exact_record_budget,
        input.exact_record_bytes,
        input.resident_current_record_bytes,
        input.snapshot_rows,
    )
    .await
}

pub(crate) fn summary_projection_live_record_from_preview(
    row: UpstreamAccountInvocationPreviewRow,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
) -> Option<(String, SummaryProjectionRecord)> {
    let occurred_at = parse_to_utc_datetime(&row.occurred_at)?;
    let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
    let global_rollup_covered =
        row.id <= rollup_live_cursor && hourly_rollup_totals.contains_key(&(bucket, None));
    let account_rollup_covered = account_rollup_live_cursor.is_some_and(|cursor| row.id <= cursor)
        && hourly_rollup_totals.contains_key(&(bucket, row.upstream_account_id));
    let usage_global_rollup_covered =
        row.id <= rollup_live_cursor && hourly_rollup_usage.contains_key(&(bucket, None));
    let usage_account_rollup_covered = account_rollup_live_cursor
        .is_some_and(|cursor| row.id <= cursor)
        && hourly_rollup_usage.contains_key(&(bucket, row.upstream_account_id));
    Some((
        row.invoke_id.clone(),
        SummaryProjectionRecord {
            global_rollup_covered,
            account_rollup_covered,
            usage_global_rollup_covered,
            usage_account_rollup_covered,
            is_persisted_live_record: true,
            is_archive_record: false,
            archive_has_materialized_rollups: false,
            account_archive_totals_fallback_included: false,
            row,
            occurred_at,
        },
    ))
}

#[derive(Clone, Default)]
pub(crate) struct PreviousSummaryProjectionAllTime {
    pub(crate) all_time_by_account: HashMap<Option<i64>, StatsResponse>,
    pub(crate) all_time_refreshed_at: Option<Instant>,
    pub(crate) all_time_manifest_admission_blocked_at: Option<Instant>,
    pub(crate) all_time_account_manifest_admission_blocked_at: Option<Instant>,
    pub(crate) all_time_account_refreshed_at: HashMap<i64, Instant>,
    pub(crate) archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    pub(crate) archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    pub(crate) global_all_time_eligible: bool,
    pub(crate) account_all_time_eligible: HashSet<i64>,
    pub(crate) all_time_terminal_coverage_complete: bool,
    pub(crate) global_all_time_coverage_fence: Option<SummaryCoverageFence>,
    pub(crate) account_all_time_coverage_fence: Option<SummaryCoverageFence>,
    pub(crate) all_time_terminal_sequence_watermark: u64,
    pub(crate) all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    pub(crate) all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
    pub(crate) historical_live_coverage: Option<SummaryProjectionHistoricalLiveCoverage>,
    pub(crate) coverage_overlay: Option<SummaryCoverageOverlay>,
    pub(crate) unavailable_exact_live_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_exact_live_account_ranges: HashMap<i64, Vec<ExactUtcRange>>,
    pub(crate) unavailable_unmaterialized_archive_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_unmaterialized_archive_account_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    pub(crate) persisted_live_terminal_invoke_ids: HashSet<String>,
}
