use super::*;
pub(crate) const SUMMARY_SNAPSHOT_MAX_STALE: Duration = Duration::from_secs(15);
pub(crate) const SUMMARY_SNAPSHOT_MAX_KEYS: usize = 48;
// Owner checks are memory-only. Polling at this cadence gives the runtime refresh a bounded
// scheduling phase without performing SQLite/archive work while no consumer owns Summary.
pub(crate) const SUMMARY_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const SUMMARY_LIVE_TAIL_RECONCILIATION_QUEUE_WAIT: Duration = Duration::from_secs(2);
pub(crate) const SUMMARY_HISTORICAL_COVERAGE_RECOVERY_QUEUE_WAIT: Duration =
    Duration::from_secs(30);
pub(crate) const SUMMARY_HISTORICAL_COVERAGE_RECOVERY_CONTINUATION_QUEUE_WAIT: Duration =
    Duration::from_secs(1);
// Keep each low-priority recovery turn long enough to amortize archive open/hash/decode setup
// across several bounded pages. The per-page 64 MiB/400-ID limits and pressure permit remain
// unchanged; this is a scheduler batch budget, not an HTTP or rolling freshness deadline.
pub(crate) const SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET: Duration = Duration::from_secs(30);
pub(crate) const SUMMARY_SNAPSHOT_EVENT_DEBOUNCE: Duration = Duration::from_millis(250);
// Allow for hub coordination after a due tick. Hub state mutex holders never await durable I/O,
// but this explicit budget keeps that short handoff out of the freshness critical path.
pub(crate) const SUMMARY_SNAPSHOT_COORDINATION_SLACK: Duration = Duration::from_millis(500);
pub(crate) const SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
// Do not turn a locked or overloaded SQLite database into a continuous sequence of timed-out
// full hydrations. Mutations remain coalesced as dirty while this bounded retry gate is active.
pub(crate) const SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF: Duration = Duration::from_secs(30);
pub(crate) const SUMMARY_PROJECTION_MANIFEST_ADMISSION_RETRY_BACKOFF: Duration =
    Duration::from_secs(5 * 60);
// A cadence refresh begins after the 10-second floor without event debounce. Its scheduling
// phase, hub coordination allowance, and build deadline remain strictly inside the 15-second
// serving ceiling; mutations still use the separate debounce below.
pub(crate) const SUMMARY_PROJECTION_BUILD_DEADLINE: Duration = Duration::from_secs(4);
// Finalizing an all-time checkpoint may still need to assemble the bounded current/rolling
// source set. Keep this background-only phase independent from the cadence refresh budget: it
// has enough time to finish a proven historical reconciliation, while Bootstrap publication and
// changed-generation rolling refreshes retain their shorter availability deadlines.
pub(crate) const SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE: Duration =
    Duration::from_secs(30);
// Startup publishes the independently exact current and rolling selections before background
// all-time reconciliation. The initial budget therefore remains a bounded readiness gate rather
// than a deadline for a full-history scan.
pub(crate) const SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const SUMMARY_PROJECTION_MAX_EXACT_RECORDS: usize = 50_000;

#[derive(Debug)]
pub(crate) struct SummaryProjectionAllTimeGenerationChanged;

impl std::fmt::Display for SummaryProjectionAllTimeGenerationChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("summary all-time reconciliation observed a changed durable generation")
    }
}

impl std::error::Error for SummaryProjectionAllTimeGenerationChanged {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SummaryProjectionBuildMode {
    Bootstrap,
    // A cold Bootstrap which could not finish bounded historical admission. This mode publishes
    // only the live/rollup proof and leaves the older boundary range explicitly unavailable for
    // the independent coverage supervisor to repair.
    BootstrapFallback,
    // A published rolling baseline can consume the hub-owned committed terminal journal
    // without re-admitting the complete live source.
    RollingDelta,
    Rolling,
    HistoricalLiveCoverage,
    AllTime,
}

impl SummaryProjectionBuildMode {
    pub(crate) const fn includes_all_time(self) -> bool {
        matches!(self, Self::AllTime)
    }

    pub(crate) const fn requires_full_historical_live_coverage(self) -> bool {
        matches!(self, Self::Bootstrap | Self::HistoricalLiveCoverage)
    }
}

pub(crate) fn summary_projection_bootstrap_fallback_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("summary projection build exceeded")
        || message.contains("summary projection")
            && (message.contains("exceeded bounded") || message.contains("admission exceeded"))
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT: usize;
    pub(crate) static SUMMARY_PROJECTION_TEST_HISTORICAL_IDENTITY_HYDRATION_ROWS: Cell<usize>;
    pub(crate) static SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS: Cell<usize>;
    pub(crate) static SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS: Cell<usize>;
    pub(crate) static SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS: Cell<usize>;
}

pub(crate) fn summary_projection_exact_record_limit() -> usize {
    #[cfg(test)]
    {
        SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT
            .try_with(|limit| *limit)
            .unwrap_or(SUMMARY_PROJECTION_MAX_EXACT_RECORDS)
    }

    #[cfg(not(test))]
    {
        SUMMARY_PROJECTION_MAX_EXACT_RECORDS
    }
}

#[cfg(test)]
pub(crate) async fn with_summary_projection_test_exact_record_limit<T>(
    limit: usize,
    future: impl std::future::Future<Output = T>,
) -> T {
    SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT
        .scope(limit, future)
        .await
}

#[cfg(test)]
pub(crate) fn summary_projection_test_invocation() -> ApiInvocation {
    invocation_cost_audit_tests::sample_invocation(None)
}

// Account and archive metadata are admission-controlled independently from exact invocation
// rows.  A refresh which cannot represent the durable cardinality fails closed and keeps the
// previous projection, rather than publishing a partial aggregate.
pub(crate) const SUMMARY_PROJECTION_MAX_ACCOUNTS: usize = 50_000;
// Exact archive-boundary hydration remains bounded independently from the compact all-time
// rollup baseline. Path-based SQLite reads are chunked below to stay well under bind limits.
pub(crate) const SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES: usize = 4_096;
pub(crate) const SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE: usize = 512;
// Exact replacement is intentionally exceptional. Keeping this separately bounded prevents a
// wide retention horizon with a compact-coverage gap from turning a background rebuild into an
// uninterruptible hour-by-hour allocation and CPU sweep.
pub(crate) const SUMMARY_PROJECTION_MAX_EXACT_BUCKETS: usize = 4_096;
pub(crate) const SUMMARY_PROJECTION_MAX_PREVIEW_BYTES: usize = 64 * 1024 * 1024;
// Live source admission is independent from the resident preview budget. Each source record
// and packed hydration page remains bounded, while multiple pages may be admitted in one build.
pub(crate) const SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const SUMMARY_PROJECTION_MAX_SOURCE_PAGE_BYTES: usize = 64 * 1024 * 1024;
// Rollup tables are compact but can still become unbounded when account/model cardinality
// grows. Count and size-limit their background hydration before materializing rows in memory.
pub(crate) const SUMMARY_PROJECTION_MAX_ROLLUP_ROWS: usize = 200_000;
pub(crate) const SUMMARY_PROJECTION_MAX_ROLLUP_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const SUMMARY_PROJECTION_MAX_LIVE_AGGREGATE_ROWS: usize =
    SUMMARY_PROJECTION_MAX_ROLLUP_ROWS;
pub(crate) const SUMMARY_PROJECTION_MIN_EXACT_HORIZON: ChronoDuration = ChronoDuration::hours(48);
// Summary duration/calendar ranges are backed by the configured live retention plus the
// archive grace used by the existing range readers. A legal `thisMonth` can span nearly 31
// days, so the finite calendar guard must cover that longest named range even when retention
// is zero. HTTP still reads only the resulting in-memory projection.
pub(crate) const SUMMARY_PROJECTION_ARCHIVE_GRACE_DAYS: u64 = 31;
// The public Summary UI exposes rolling windows through 1mo. Keep arbitrary parser inputs
// bounded at the HTTP contract so the projection never has to retain an unbounded set of
// partial-hour boundaries; longer requests fail before hub/SQLite access.
pub(crate) const SUMMARY_PROJECTION_MAX_DURATION: ChronoDuration = ChronoDuration::days(30);

pub(crate) fn summary_projection_exact_horizon(invocation_max_days: u64) -> ChronoDuration {
    let supported_days = invocation_max_days.saturating_add(SUMMARY_PROJECTION_ARCHIVE_GRACE_DAYS);
    ChronoDuration::days(supported_days.max(2) as i64).max(SUMMARY_PROJECTION_MIN_EXACT_HORIZON)
}

pub(crate) fn validate_summary_projection_window(
    query: &SummaryQuery,
    default_limit: i64,
) -> Result<(), ApiError> {
    if let SummaryWindow::Duration(duration) =
        parse_summary_window(query, default_limit).map_err(ApiError::bad_request)?
    {
        if duration <= ChronoDuration::zero() {
            return Err(ApiError::bad_request(anyhow!(
                "summary duration must be greater than zero"
            )));
        }
        if duration > SUMMARY_PROJECTION_MAX_DURATION {
            return Err(ApiError::bad_request(anyhow!(
                "summary duration exceeds the supported 30d window"
            )));
        }
        if duration > SUMMARY_PROJECTION_MIN_EXACT_HORIZON
            && duration.num_minutes() % ChronoDuration::days(1).num_minutes() != 0
        {
            return Err(ApiError::bad_request(anyhow!(
                "summary durations over 48h must use whole-day granularity"
            )));
        }
    }
    Ok(())
}

pub(crate) fn summary_projection_boundary_buckets(end: DateTime<Utc>) -> HashSet<i64> {
    let mut buckets = HashSet::new();
    let mut add_bucket = |timestamp: DateTime<Utc>| {
        buckets.insert(align_bucket_epoch(timestamp.timestamp(), 3_600, 0));
    };
    // Any legal duration through the 48-hour exact horizon can begin in an arbitrary partial
    // hour. Keep every possible start bucket exact instead of enumerating a few UI presets;
    // the middle of longer ranges still comes from compact hourly rollups.
    for hours in 1..=SUMMARY_PROJECTION_MIN_EXACT_HORIZON.num_hours() {
        add_bucket(end - ChronoDuration::hours(hours));
    }
    // Whole-day durations beyond the exact horizon are also part of the bounded public
    // contract. Their partial boundary hours remain exact without retaining raw history.
    for days in 1..=SUMMARY_PROJECTION_MAX_DURATION.num_days() {
        add_bucket(end - ChronoDuration::days(days));
    }
    add_bucket(end);

    // Calendar boundaries belong to the request timezone, not an approximate UTC offset.  The
    // projection can serve for fifteen seconds, so cover the finite set of IANA timezone
    // boundaries at build time and again at the far edge of that serving interval.  This keeps
    // non-whole-hour zones, the date line, DST, and a just-crossed local day/week/month exact
    // without adding a per-request archive read.
    for boundary_now in [
        end,
        end + ChronoDuration::seconds(SUMMARY_SNAPSHOT_MAX_STALE.as_secs() as i64),
    ] {
        for timezone in TZ_VARIANTS {
            for spec in [
                "today",
                "yesterday",
                "thisWeek",
                "thisMonth",
                "recent180d",
                "recent365d",
            ] {
                if let Some((start, end)) = named_range_bounds(spec, boundary_now, timezone) {
                    add_bucket(start);
                    add_bucket(end);
                }
            }
            if let Some((start, end)) = previous_full_days_range_bounds(7, boundary_now, timezone) {
                add_bucket(start);
                add_bucket(end);
            }
        }
    }
    buckets
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SummarySnapshotKey {
    pub(crate) window: String,
    pub(crate) limit: Option<i64>,
    pub(crate) time_zone: String,
    pub(crate) upstream_account_id: Option<i64>,
}

impl SummarySnapshotKey {
    pub(crate) fn try_from_query(query: &SummaryQuery, default_limit: i64) -> Result<Self> {
        let (window, limit) = match parse_summary_window(query, default_limit)? {
            SummaryWindow::All => ("all".to_string(), None),
            SummaryWindow::Current(limit) => ("current".to_string(), Some(limit)),
            SummaryWindow::Duration(duration) => (format!("{}m", duration.num_minutes()), None),
            SummaryWindow::Calendar(window) => (window, None),
            SummaryWindow::PreviousFullDays(7) => ("previous7d".to_string(), None),
            SummaryWindow::PreviousFullDays(days) => (format!("previous{days}d"), None),
        };
        let time_zone = parse_reporting_tz(query.time_zone.as_deref())?.to_string();
        let upstream_account_id = match query.upstream_account_id {
            Some(account_id) if account_id > 0 => Some(account_id),
            Some(_) => return Err(anyhow!("invalid upstream account")),
            None => None,
        };

        Ok(Self {
            window,
            limit,
            time_zone,
            upstream_account_id,
        })
    }

    pub(crate) fn from_query(query: &SummaryQuery) -> Self {
        Self::try_from_query(query, 50).expect("test summary snapshot query must be valid")
    }

    pub(crate) fn query(&self) -> SummaryQuery {
        SummaryQuery {
            window: Some(self.window.clone()),
            limit: self.limit,
            time_zone: Some(self.time_zone.clone()),
            upstream_account_id: self.upstream_account_id,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummarySnapshotEntry {
    pub(crate) response: Option<StatsResponse>,
    pub(crate) refreshed_at: Option<Instant>,
}

/// Canonical, process-local source for summary reads.  The projection intentionally stores
/// records rather than URL-keyed responses: a timezone, account, rolling window, or current-N
/// selection is a pure view over the same durable input and can therefore never borrow another
/// selection's result.
#[derive(Debug, Clone)]
pub(crate) struct SummaryProjectionRecord {
    pub(crate) row: UpstreamAccountInvocationPreviewRow,
    pub(crate) occurred_at: DateTime<Utc>,
    // Full-hour rollups cover persisted records only through their durable cursor. Archive
    // materialization has separate global and account coverage, so a global bucket never
    // suppresses an account-only fallback record.
    pub(crate) global_rollup_covered: bool,
    pub(crate) account_rollup_covered: bool,
    pub(crate) usage_global_rollup_covered: bool,
    pub(crate) usage_account_rollup_covered: bool,
    pub(crate) is_persisted_live_record: bool,
    // Unmaterialized archive rows are folded by the bounded account archive aggregate. Keep
    // them out of the exact-record repair pass so all-time account totals are not doubled.
    pub(crate) is_archive_record: bool,
    pub(crate) archive_has_materialized_rollups: bool,
    // When account replay is unavailable, the durable account archive aggregate already
    // contains this archive row. Keep the exact row for boundary/range views, but do not fold it
    // into the all-time account aggregate a second time.
    pub(crate) account_archive_totals_fallback_included: bool,
}

pub(crate) fn summary_projection_record_identity_matches(
    record: &SummaryProjectionRecord,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> bool {
    record.row.id == id
        && record.row.invoke_id == invoke_id
        && record.row.occurred_at == occurred_at
}

pub(crate) fn summary_projection_records_contains_identity(
    records: &HashMap<String, SummaryProjectionRecord>,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> bool {
    records.values().any(|record| {
        summary_projection_record_identity_matches(record, id, invoke_id, occurred_at)
    })
}

pub(crate) fn summary_projection_record_bytes_for_identity(
    records: &HashMap<String, SummaryProjectionRecord>,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> usize {
    records
        .values()
        .find(|record| {
            summary_projection_record_identity_matches(record, id, invoke_id, occurred_at)
        })
        .map(|record| summary_projection_preview_row_bytes(&record.row))
        .unwrap_or_default()
}

pub(crate) fn summary_projection_record_source_identity(
    record: &SummaryProjectionRecord,
) -> SummarySourceIdentity {
    SummarySourceIdentity {
        row_id: record.row.id,
        invoke_id: record.row.invoke_id.clone(),
        occurred_at: record.row.occurred_at.clone(),
    }
}

pub(crate) fn summary_projection_record_identity_key(record: &SummaryProjectionRecord) -> String {
    summary_projection_source_identity_key(
        record.row.id,
        &record.row.invoke_id,
        &record.row.occurred_at,
    )
}

pub(crate) fn summary_projection_source_identity_key(
    row_id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> String {
    format!("{row_id}\0{occurred_at}\0{invoke_id}")
}

pub(crate) fn summary_projection_delta_identity_key(
    delta: &DashboardActivityTerminalDelta,
) -> String {
    delta.persisted_row_id.map_or_else(
        || format!("legacy\0{}\0{}", delta.occurred_at, delta.invoke_id),
        |row_id| {
            summary_projection_source_identity_key(row_id, &delta.invoke_id, &delta.occurred_at)
        },
    )
}

pub(crate) fn decode_summary_projection_pending_terminal_identity(
    identity: &str,
) -> Option<(i64, &str, &str)> {
    let mut fields = identity.split('\0');
    let row_id = fields
        .next()?
        .parse::<i64>()
        .ok()
        .filter(|row_id| *row_id > 0)?;
    let occurred_at = fields.next().filter(|value| !value.is_empty())?;
    let invoke_id = fields.next().filter(|value| !value.is_empty())?;
    fields
        .next()
        .is_none()
        .then_some((row_id, occurred_at, invoke_id))
}

pub(crate) fn summary_projection_record_insert_key(
    records: &HashMap<String, SummaryProjectionRecord>,
    record: &SummaryProjectionRecord,
) -> String {
    let invoke_id = record.row.invoke_id.clone();
    if !records.contains_key(&invoke_id) {
        return invoke_id;
    }
    if records.get(&invoke_id).is_some_and(|existing| {
        summary_projection_record_identity_matches(
            existing,
            record.row.id,
            &record.row.invoke_id,
            &record.row.occurred_at,
        )
    }) {
        return invoke_id;
    }
    summary_projection_record_identity_key(record)
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SummaryProjectionArchiveReplayCoverage {
    pub(crate) overall: bool,
    pub(crate) account_stats: bool,
    pub(crate) usage_breakdown: bool,
}

impl SummaryProjectionArchiveReplayCoverage {
    pub(crate) fn supports_unavailable_archive(self) -> bool {
        self.overall && self.account_stats && self.usage_breakdown
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryProjectionFreshness {
    pub(crate) global_all_time_eligible: bool,
    pub(crate) account_all_time_eligible: HashSet<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryProjectionHistoricalLiveCoverage {
    pub(crate) range: ExactUtcRange,
    pub(crate) high_watermark_id: i64,
    pub(crate) reconciliation_required: bool,
}

/// A verified historical contribution which can be atomically attached to an existing
/// projection without rerunning live admission.  The overlay retains only normalized totals;
/// raw archive payloads and preview rows remain outside the serving read model.
#[derive(Debug, Clone)]
pub(crate) struct SummaryCoverageOverlay {
    pub(crate) coverage_fence: SummaryCoverageFence,
    pub(crate) live_tail_cursor: SummaryLiveTailCursor,
    // This is the durable identity set represented by the normalized maps below. Keeping it
    // with the immutable overlay lets recovery reduce only newly verified V2 pages instead of
    // rereading every historical manifest on each publication turn.
    pub(crate) proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    pub(crate) recent_proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    pub(crate) global_coverage_buckets: HashSet<i64>,
    pub(crate) account_coverage_buckets: HashSet<i64>,
    // Materialized archives retain their durable rollup as the aggregate baseline. Keep this
    // distinction alongside V2 coverage so an unmaterialized sibling in the same hour does not
    // cause finalization to subtract the materialized rollup before adding only the sibling.
    pub(crate) materialized_coverage_buckets: HashSet<i64>,
    // Legacy manifests without explicit coverage bounds can only prove materialization at the
    // month granularity. Preserve that proof so mixed archive siblings do not replace a whole
    // month of compact rollups merely because one sibling contributes V2 rows.
    pub(crate) materialized_coverage_months: HashSet<String>,
    pub(crate) global_by_bucket: HashMap<i64, StatsTotals>,
    pub(crate) account_by_bucket: HashMap<(i64, i64), StatsTotals>,
    pub(crate) global_usage_by_bucket: HashMap<i64, UsageBreakdownResponse>,
    pub(crate) account_usage_by_bucket: HashMap<(i64, i64), UsageBreakdownResponse>,
    pub(crate) global_non_success_tokens_by_bucket: HashMap<i64, i64>,
    pub(crate) account_non_success_tokens_by_bucket: HashMap<(i64, i64), i64>,
    // Boundary rows are retained only for ranges that still need partial-hour exactness. Full
    // hours continue to use compact totals so a large archive never enters the resident record
    // budget merely because its V2 proof was published.
    pub(crate) boundary_records: Vec<SummaryProjectionRecord>,
}

impl Default for SummaryCoverageOverlay {
    fn default() -> Self {
        Self {
            coverage_fence: SummaryCoverageFence {
                completed_manifest_high_watermark_id: None,
                coverage_revision: 0,
                account_coverage_revision: 0,
            },
            live_tail_cursor: SummaryLiveTailCursor {
                live_high_watermark_id: 0,
                rollup_live_cursor: 0,
                account_rollup_live_cursor: None,
                durable_terminal_sequence_watermark: 0,
            },
            proof_identities: HashSet::new(),
            recent_proof_identities: HashSet::new(),
            global_coverage_buckets: HashSet::new(),
            account_coverage_buckets: HashSet::new(),
            materialized_coverage_buckets: HashSet::new(),
            materialized_coverage_months: HashSet::new(),
            global_by_bucket: HashMap::new(),
            account_by_bucket: HashMap::new(),
            global_usage_by_bucket: HashMap::new(),
            account_usage_by_bucket: HashMap::new(),
            global_non_success_tokens_by_bucket: HashMap::new(),
            account_non_success_tokens_by_bucket: HashMap::new(),
            boundary_records: Vec::new(),
        }
    }
}

// A projection can be renewed only when each durable source boundary remains unchanged.  This
// is deliberately smaller than the projection itself: it lets the maintenance loop keep an
// already-proven immutable response alive without cloning its resident preview rows.
#[derive(Debug, Clone)]
pub(crate) struct SummaryProjectionFreshnessLease {
    pub(crate) origin: Instant,
    pub(crate) renewed_elapsed_ms: Arc<AtomicU64>,
}

impl Default for SummaryProjectionFreshnessLease {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            renewed_elapsed_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl SummaryProjectionFreshnessLease {
    pub(crate) fn renewed_at(&self) -> Option<Instant> {
        let elapsed_ms = self.renewed_elapsed_ms.load(Ordering::Relaxed);
        (elapsed_ms > 0).then(|| self.origin + Duration::from_millis(elapsed_ms))
    }

    pub(crate) fn latest(&self, refreshed_at: Option<Instant>) -> Option<Instant> {
        match (refreshed_at, self.renewed_at()) {
            (Some(refreshed_at), Some(renewed_at)) => Some(refreshed_at.max(renewed_at)),
            (Some(refreshed_at), None) => Some(refreshed_at),
            (None, Some(renewed_at)) => Some(renewed_at),
            (None, None) => None,
        }
    }

    pub(crate) fn renew(&self) {
        let elapsed_ms = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.renewed_elapsed_ms
            .fetch_max(elapsed_ms.max(1), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct SummaryProjectionGenerationFence {
    pub(crate) live_high_watermark_id: i64,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) completed_manifest_high_watermark_id: Option<i64>,
    pub(crate) coverage_revision: i64,
    pub(crate) account_coverage_revision: i64,
    pub(crate) durable_terminal_sequence_watermark: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct SummaryCoverageFence {
    pub(crate) completed_manifest_high_watermark_id: Option<i64>,
    pub(crate) coverage_revision: i64,
    pub(crate) account_coverage_revision: i64,
}

impl SummaryCoverageFence {
    pub(crate) fn global_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.coverage_revision == other.coverage_revision
    }

    pub(crate) fn account_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.account_coverage_revision == other.account_coverage_revision
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct SummaryLiveTailCursor {
    pub(crate) live_high_watermark_id: i64,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) durable_terminal_sequence_watermark: u64,
}

impl SummaryLiveTailCursor {
    pub(crate) fn terminal_sources_match(self, other: Self) -> bool {
        self.live_high_watermark_id == other.live_high_watermark_id
            && self.rollup_live_cursor == other.rollup_live_cursor
            && self.account_rollup_live_cursor == other.account_rollup_live_cursor
            && self.durable_terminal_sequence_watermark == other.durable_terminal_sequence_watermark
    }

    pub(crate) fn rollup_sources_match(self, other: Self) -> bool {
        self.rollup_live_cursor == other.rollup_live_cursor
            && self.account_rollup_live_cursor == other.account_rollup_live_cursor
    }

    pub(crate) fn at_or_behind(self, other: Self) -> bool {
        self.live_high_watermark_id <= other.live_high_watermark_id
            && self.rollup_live_cursor <= other.rollup_live_cursor
            && match (
                self.account_rollup_live_cursor,
                other.account_rollup_live_cursor,
            ) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(current), Some(expected)) => current <= expected,
            }
            && self.durable_terminal_sequence_watermark <= other.durable_terminal_sequence_watermark
    }
}

impl SummaryProjectionGenerationFence {
    // Live terminal progress is represented by the bounded in-memory/durable tail overlay.
    // Historical checkpoint validity therefore depends only on immutable coverage inputs; a
    // newly committed terminal must not restart already-proven archive pages.
    pub(crate) fn coverage_sources_match(self, other: Self) -> bool {
        self.global_coverage_sources_match(other) && self.account_coverage_sources_match(other)
    }

    /// Coverage publication may advance an older in-memory projection to a newer durable proof
    /// fence. It must never move backwards or overwrite a projection based on a newer fence.
    pub(crate) fn coverage_sources_at_or_behind(self, other: Self) -> bool {
        let manifest_high_watermark_is_compatible = match (
            self.completed_manifest_high_watermark_id,
            other.completed_manifest_high_watermark_id,
        ) {
            (None, _) => true,
            (Some(current), Some(expected)) => current <= expected,
            (Some(_), None) => false,
        };
        manifest_high_watermark_is_compatible
            && self.coverage_revision <= other.coverage_revision
            && self.account_coverage_revision <= other.account_coverage_revision
    }

    pub(crate) fn global_coverage_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.coverage_revision == other.coverage_revision
    }

    pub(crate) fn account_coverage_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.account_coverage_revision == other.account_coverage_revision
    }

    // A checkpoint cursor is valid only for the exact coverage revision that produced it. Any
    // proof mutation is a new input generation and must force the worker to re-establish its
    // cursor from the reset checkpoint rather than continuing a stale page.
    pub(crate) fn global_coverage_checkpoint_compatible(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && other.coverage_revision == self.coverage_revision
    }

    pub(crate) fn account_coverage_checkpoint_compatible(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && other.account_coverage_revision == self.account_coverage_revision
    }

    pub(crate) fn coverage_fence(self) -> SummaryCoverageFence {
        SummaryCoverageFence {
            completed_manifest_high_watermark_id: self.completed_manifest_high_watermark_id,
            coverage_revision: self.coverage_revision,
            account_coverage_revision: self.account_coverage_revision,
        }
    }

    pub(crate) fn live_tail_cursor(self) -> SummaryLiveTailCursor {
        SummaryLiveTailCursor {
            live_high_watermark_id: self.live_high_watermark_id,
            rollup_live_cursor: self.rollup_live_cursor,
            account_rollup_live_cursor: self.account_rollup_live_cursor,
            durable_terminal_sequence_watermark: self.durable_terminal_sequence_watermark,
        }
    }
}

impl SummaryProjectionFreshness {
    pub(crate) fn rolling_at(&self, built_at: Option<Instant>) -> Option<Instant> {
        built_at
    }

    pub(crate) fn all_time_at(
        &self,
        built_at: Option<Instant>,
        account_id: Option<i64>,
    ) -> Option<Instant> {
        let eligible = match account_id {
            None => self.global_all_time_eligible,
            Some(account_id) => self.account_all_time_eligible.contains(&account_id),
        };
        if eligible { built_at } else { None }
    }
}

pub(crate) fn summary_projection_manifest_admission_retry_is_due(
    blocked_at: Option<Instant>,
    now: Instant,
) -> bool {
    blocked_at.is_none_or(|blocked_at| {
        now.saturating_duration_since(blocked_at)
            >= SUMMARY_PROJECTION_MANIFEST_ADMISSION_RETRY_BACKOFF
    })
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryProjection {
    pub(crate) records: Vec<SummaryProjectionRecord>,
    // `current` is a newest-N selection rather than a time range. Keep its bounded prefix
    // separate from the rolling/archive repair records so a large historical prefix cannot make
    // two individually-admitted read-model inputs exceed one shared record budget.
    pub(crate) current_records: Vec<SummaryProjectionRecord>,
    // A Summary SSE base can safely omit a terminal overlay only after the durable record is
    // known to be in this exact projection revision. Keep the bounded identity set alongside
    // the canonical live records so reconnects never infer that coverage from a global cursor.
    pub(crate) persisted_live_terminal_invoke_ids: HashSet<String>,
    // Rolling refreshes may reuse the previous all-time aggregate. Until a successful all-time
    // rebuild, that aggregate cannot suppress a newly acknowledged terminal delta. This flag is
    // deliberately global-scope only; account eligibility is tracked independently in freshness.
    pub(crate) all_time_terminal_coverage_complete: bool,
    // RollingDelta advances the current generation fence while retaining the last exact
    // all-time aggregate. Keep the coverage fence that actually produced each all-time scope
    // separate so a newly ready checkpoint cannot be mistaken for already published data.
    pub(crate) global_all_time_coverage_fence: Option<SummaryCoverageFence>,
    pub(crate) account_all_time_coverage_fence: Option<SummaryCoverageFence>,
    // The settled terminal watermark actually represented by the retained all-time aggregate.
    // Rolling-only revisions must not advance this proof while they reuse the old aggregate.
    pub(crate) all_time_terminal_sequence_watermark: u64,
    // Exact persisted live identities observed by a complete global all-time rebuild. Keeping
    // this separate from the rolling identity set closes the hydration race where a terminal is
    // committed after the settled watermark sample but before the aggregate query sees it.
    pub(crate) all_time_persisted_live_terminal_invoke_ids: HashSet<String>,
    // Account all-time aggregates have an independent durable source and therefore need an
    // independent terminal proof. A global watermark must never suppress an account overlay
    // when that account's aggregate was retained or rebuilt under a different source boundary.
    pub(crate) all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    pub(crate) all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
    // Captured immediately before the durable hydration queries. A settled sequence means every
    // preceding terminal write has committed, so a successful projection can safely recover a
    // bounded SSE overlay that overflowed before its next rebuild.
    pub(crate) durable_terminal_sequence_watermark: u64,
    // Buckets are an index over canonical records, not a second source of truth.  They bound
    // rolling/calendar selection work without approximating a timezone boundary.
    pub(crate) hourly_buckets: BTreeMap<i64, Vec<usize>>,
    // Materialized history fallback.  Only compact totals remain resident; archive rows are
    // released after hydration.
    pub(crate) hourly_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
    pub(crate) hourly_rollup_non_success_tokens: HashMap<(i64, Option<i64>), i64>,
    // Usage rollups carry the model/reasoning dimension which is absent from StatsTotals.
    // Keep both account and global keys so a materialized archive never turns an exact
    // usage breakdown into an empty response on the memory-only request path.
    pub(crate) hourly_rollup_usage: HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    // A replay marker can lag after a historical rollup has been partially materialized.  For
    // these buckets hydration retains the archive plus every persisted live row as the exact
    // replacement source.  The request reducer must therefore suppress the compact bucket only
    // for the affected scope, never merely because one archive row happened to be selected.
    pub(crate) exact_global_total_rollup_buckets: HashSet<i64>,
    pub(crate) exact_account_total_rollup_buckets: HashSet<i64>,
    pub(crate) exact_global_usage_rollup_buckets: HashSet<i64>,
    pub(crate) exact_account_usage_rollup_buckets: HashSet<i64>,
    pub(crate) rollup_live_cursor: i64,
    // Historical live coverage is verified during Bootstrap or a dedicated background pass.
    // Rolling rebuilds reuse its unchanged proof and localize only newly unproven buckets.
    pub(crate) historical_live_coverage: Option<SummaryProjectionHistoricalLiveCoverage>,
    // Verified V2 archive contributions are attached independently from the live-tail rebuild.
    // Keeping the overlay in the projection lets subsequent rolling refreshes retain a proof
    // published by the historical supervisor.
    pub(crate) coverage_overlay: Option<SummaryCoverageOverlay>,
    // These indexes are constructed once off-request.  They retain only the largest legal
    // current-window result for the global view and each account, so a hot `current` read never
    // sorts or allocates in proportion to retained history.
    pub(crate) recent_indexes: HashMap<Option<i64>, Vec<usize>>,
    // The bounded global recent scan proves an account's top N only when N entries survived in
    // its index. If the scan overflowed and fewer entries are retained, serving a shorter list
    // would silently omit older invocations, so that selection uses unavailable instead.
    pub(crate) recent_index_complete: bool,
    // The first row omitted by the globally ordered `current` index. A rolling range that reaches
    // this boundary cannot prove its live/rollup partition is complete without request-time I/O,
    // even though its separate boundary-record view may retain selected older rows.
    pub(crate) recent_index_overflow_at: Option<DateTime<Utc>>,
    // `all` has no reporting-timezone boundary.  Hydration therefore folds the legacy
    // archive/rollup coverage rules into a canonical aggregate per account once, instead of
    // attempting to recreate those rules from raw archive rows on every request.
    pub(crate) all_time_by_account: HashMap<Option<i64>, StatsResponse>,
    // Regular rolling refreshes may reuse all-time responses when no all-time owner is active;
    // keep their freshness independent from the projection-wide timestamp.
    pub(crate) all_time_refreshed_at: Option<Instant>,
    // An all-history manifest set beyond admission cannot be made exact without an unbounded
    // proof. Retry only after a long, controlled backoff so maintenance does not recreate the
    // production pressure pattern while a recoverable all-time source is unavailable.
    pub(crate) all_time_manifest_admission_blocked_at: Option<Instant>,
    // Account discovery has a distinct admission budget. It must not suppress an exact global
    // all-time aggregate when only the account scope cannot be proven.
    pub(crate) all_time_account_manifest_admission_blocked_at: Option<Instant>,
    // Global all-time rollups and account archive fallbacks have independent durable coverage.
    // A failed account rebuild must not make a fresh global aggregate stale, and an old account
    // response must never borrow the global aggregate's freshness.
    pub(crate) all_time_account_refreshed_at: HashMap<i64, Instant>,
    // Pre-computed at build time so the 250ms owner cadence never scans one timestamp per
    // account merely to decide whether an all-time refresh is due.
    pub(crate) all_time_oldest_account_refreshed_at: Option<Instant>,
    // This separates a caller-provided, unknown account (whose all-time aggregate is exactly
    // zero once the global projection is fresh) from an account whose durable history could not
    // be hydrated. The latter must retain the unavailable contract rather than become zero.
    pub(crate) all_time_account_ids_with_projection_data: HashSet<i64>,
    pub(crate) known_account_ids: HashSet<i64>,
    // Archive files are immutable once completed. Reuse their bounded account-key discovery on
    // later refreshes so maintenance never repeatedly scans a large low-cardinality archive.
    pub(crate) archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    // Legacy archive manifests can lack coverage bounds. Preserve a successfully measured
    // immutable range with its account discovery cache so later all-time refreshes retain the
    // same exact-source proof instead of degrading a valid account response.
    pub(crate) archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    // An unreadable archive without complete durable replay has no exact in-memory source.
    // Keep its bounded overlap so only affected rolling/calendar selections fail closed rather
    // than publishing an undercount or turning a selection-level unavailability into an HTTP
    // builder failure.
    pub(crate) unavailable_unmaterialized_archive_ranges: Vec<ExactUtcRange>,
    // A materialized compact bucket can answer a whole-hour query while an unreadable archive is
    // still required for a partial boundary within that bucket. Keep this narrower condition
    // separate from a durable-coverage gap which makes every overlapping range unavailable.
    pub(crate) unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    // Rolling selections conservatively round an unavailable archive contribution to its hour,
    // but newest-N admission can prove safety against the manifest's exact endpoint. Keep that
    // narrower proof separate so an older archive in the same hour does not reject `current`.
    pub(crate) unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    // Account replay and account-manifest proof can lag the global compact rollup. A missing
    // account manifest may conceal an archive-only account, so the exact gap applies to every
    // account-scoped request without making the independently proven global response fail.
    pub(crate) unavailable_unmaterialized_archive_account_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    pub(crate) unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    // An oversized persisted-live boundary affects only ranges that intersect that exact hour.
    // The independent `current` prefix remains exact and must stay available.
    pub(crate) unavailable_exact_live_ranges: Vec<ExactUtcRange>,
    // Account rollups can lag independently of their global counterpart. Keep that proof gap
    // scoped so global rolling summaries remain available when their own compact bucket is
    // complete.
    pub(crate) unavailable_exact_live_account_ranges: HashMap<i64, Vec<ExactUtcRange>>,
    // Archive/runtime failures without an ordering proof remain a coarse unavailable state.
    pub(crate) current_source_unavailable: bool,
    // Live source admission preserves the first unproven global/account rank. Requests below
    // that boundary remain exact even when an older candidate could not be admitted.
    pub(crate) current_source_unavailable_from_rank: Option<usize>,
    pub(crate) current_account_source_unavailable_from_rank: HashMap<i64, usize>,
    // Archive manifests are not replayed in the resident current index. Their latest durable
    // coverage end is enough to prove that an individual newest-N prefix is unaffected. Keep
    // the proof separate from a resident-byte failure so `current?limit=1` is not rejected
    // merely because a much larger legal selection would reach an archive boundary.
    pub(crate) current_archive_latest_coverage_end: Option<DateTime<Utc>>,
    pub(crate) current_archive_has_unknown_coverage: bool,
    pub(crate) current_archive_admission_exceeded: bool,
    // Archive newest-N candidates are admitted globally. Per-account newest-N cannot borrow
    // that proof until every completed archive is represented: a quiet account can have its
    // newest row in history which falls outside the global current-admission horizon.
    pub(crate) current_account_source_unavailable: bool,
    // In-progress state is not the same thing as a row whose persisted status happens to be
    // `running`: the typed runtime overlay reconciles terminal replacements and retry lineage.
    // Keep that reconciled view alongside the immutable history projection.
    pub(crate) in_progress_by_account: HashMap<Option<i64>, InProgressSummarySnapshot>,
    pub(crate) maintenance: Option<StatsMaintenanceResponse>,
    // The generation fence is captured from durable source boundaries in the same build cycle.
    // A cadence renewal compares it before extending freshness; a changed source must rebuild.
    pub(crate) generation_fence: SummaryProjectionGenerationFence,
    pub(crate) freshness_lease: SummaryProjectionFreshnessLease,
    pub(crate) refreshed_at: Option<Instant>,
    pub(crate) freshness: SummaryProjectionFreshness,
    pub(crate) revision: u64,
}

impl SummaryProjection {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn generation_fence(&self) -> SummaryProjectionGenerationFence {
        self.generation_fence
    }

    pub(crate) fn advance_live_tail_fence(&mut self, live_tail: SummaryLiveTailCursor) {
        self.generation_fence.live_high_watermark_id = live_tail.live_high_watermark_id;
        self.generation_fence.rollup_live_cursor = live_tail.rollup_live_cursor;
        self.generation_fence.account_rollup_live_cursor = live_tail.account_rollup_live_cursor;
        self.generation_fence.durable_terminal_sequence_watermark =
            live_tail.durable_terminal_sequence_watermark;
        self.durable_terminal_sequence_watermark = live_tail.durable_terminal_sequence_watermark;
    }

    pub(crate) fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub(crate) fn revoke_stale_all_time_coverage(
        &mut self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let coverage_fence = generation_fence.coverage_fence();
        let revoke_global = self.freshness.global_all_time_eligible
            && self
                .global_all_time_coverage_fence
                .is_none_or(|published| !published.global_sources_match(coverage_fence));
        let revoke_account = !self.freshness.account_all_time_eligible.is_empty()
            && self
                .account_all_time_coverage_fence
                .is_none_or(|published| !published.account_sources_match(coverage_fence));
        let revoke_overlay = self
            .coverage_overlay
            .as_ref()
            .is_some_and(|overlay| overlay.coverage_fence != coverage_fence);
        let coverage_changed = self.generation_fence.coverage_fence() != coverage_fence;
        if !revoke_global && !revoke_account && !revoke_overlay {
            if coverage_changed {
                self.generation_fence.completed_manifest_high_watermark_id =
                    generation_fence.completed_manifest_high_watermark_id;
                self.generation_fence.coverage_revision = generation_fence.coverage_revision;
                self.generation_fence.account_coverage_revision =
                    generation_fence.account_coverage_revision;
                return true;
            }
            return false;
        }
        if revoke_global {
            self.all_time_by_account.remove(&None);
            self.freshness.global_all_time_eligible = false;
            self.all_time_refreshed_at = None;
            self.global_all_time_coverage_fence = None;
            self.all_time_terminal_coverage_complete = false;
            self.all_time_terminal_sequence_watermark = 0;
            self.all_time_persisted_live_terminal_invoke_ids.clear();
        }
        if revoke_account {
            self.all_time_by_account.retain(|scope, _| scope.is_none());
            self.freshness.account_all_time_eligible.clear();
            self.all_time_account_refreshed_at.clear();
            self.account_all_time_coverage_fence = None;
            self.all_time_account_terminal_sequence_watermarks.clear();
            self.all_time_account_persisted_live_terminal_invoke_ids
                .clear();
            self.all_time_oldest_account_refreshed_at = None;
        }
        if revoke_overlay && let Some(overlay) = self.coverage_overlay.take() {
            self.unavailable_unmaterialized_archive_ranges.extend(
                summary_projection_unavailable_bucket_ranges(
                    overlay.global_coverage_buckets.into_iter().collect(),
                ),
            );
            self.unavailable_unmaterialized_archive_account_ranges
                .extend(summary_projection_unavailable_bucket_ranges(
                    overlay.account_coverage_buckets.into_iter().collect(),
                ));
            self.unavailable_unmaterialized_archive_ranges = summary_projection_merge_exact_ranges(
                std::mem::take(&mut self.unavailable_unmaterialized_archive_ranges),
            );
            self.unavailable_unmaterialized_archive_account_ranges =
                summary_projection_merge_exact_ranges(std::mem::take(
                    &mut self.unavailable_unmaterialized_archive_account_ranges,
                ));
        }
        self.generation_fence.completed_manifest_high_watermark_id =
            generation_fence.completed_manifest_high_watermark_id;
        self.generation_fence.coverage_revision = generation_fence.coverage_revision;
        self.generation_fence.account_coverage_revision =
            generation_fence.account_coverage_revision;
        true
    }

    pub(crate) fn rolling_refreshed_at(&self) -> Option<Instant> {
        self.freshness_lease
            .latest(self.freshness.rolling_at(self.refreshed_at))
    }

    pub(crate) fn renew_freshness_if_generation_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if self.generation_fence != generation_fence {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    // Historical coverage revisions revoke the prior immutable overlay until a replacement is
    // published. Even when the live tail is unchanged, do not renew a lease over stale history;
    // the caller must perform the bounded replacement or leave the affected selection unavailable.
    pub(crate) fn renew_freshness_if_live_tail_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if !self
            .generation_fence
            .live_tail_cursor()
            .terminal_sources_match(generation_fence.live_tail_cursor())
            || self.generation_fence.completed_manifest_high_watermark_id
                != generation_fence.completed_manifest_high_watermark_id
            || self.generation_fence.coverage_revision != generation_fence.coverage_revision
            || self.generation_fence.account_coverage_revision
                != generation_fence.account_coverage_revision
        {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    // All-time reconciliation owns historical coverage only. A newly committed terminal
    // advances the live tail cursor and must be served by the bounded overlay without cancelling
    // an otherwise valid archive/rollup recovery page.
    pub(crate) fn renew_freshness_if_coverage_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if !self
            .generation_fence
            .coverage_sources_match(generation_fence)
            || !self
                .generation_fence
                .live_tail_cursor()
                .rollup_sources_match(generation_fence.live_tail_cursor())
        {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    pub(crate) fn coverage_sources_match(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        self.generation_fence
            .coverage_sources_match(generation_fence)
            && self
                .generation_fence
                .live_tail_cursor()
                .rollup_sources_match(generation_fence.live_tail_cursor())
    }

    pub(crate) fn renew_freshness_from_delta_journal(&self) {
        self.freshness_lease.renew();
    }

    pub(crate) fn contains_persisted_live_terminal_by_row_id(&self, row_id: i64) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| record.is_persisted_live_record && record.row.id == row_id)
    }

    pub(crate) fn contains_persisted_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
            })
    }

    pub(crate) fn contains_persisted_live_terminal_delta(
        &self,
        delta: &DashboardActivityTerminalDelta,
    ) -> bool {
        delta.persisted_row_id.map_or_else(
            || self.contains_persisted_live_terminal(&delta.invoke_id, &delta.occurred_at),
            |row_id| {
                self.contains_persisted_live_terminal_identity(
                    row_id,
                    &delta.invoke_id,
                    &delta.occurred_at,
                )
            },
        )
    }

    pub(crate) fn mark_historical_live_recovery_required(&mut self) {
        if let Some(coverage) = self.historical_live_coverage.as_mut() {
            coverage.reconciliation_required = true;
        }
    }
    pub(crate) fn current_selection_cutoff_for_scope(
        &self,
        upstream_account_id: Option<i64>,
        limit: usize,
    ) -> Option<DateTime<Utc>> {
        if limit == 0 {
            return None;
        }
        let indexes = self.recent_indexes.get(&upstream_account_id)?;
        // A shorter resident prefix has no Nth cutoff. Clamping it to the oldest retained row
        // would incorrectly prove that an unrepresented archive cannot supply the missing rank.
        let index = *indexes.get(limit.saturating_sub(1))?;
        self.current_records
            .get(index)
            .map(|record| record.occurred_at)
    }

    pub(crate) fn current_selection_cutoff(&self, limit: usize) -> Option<DateTime<Utc>> {
        self.current_selection_cutoff_for_scope(None, limit)
    }

    pub(crate) fn delta_gap_affects_current_selection(
        &self,
        gap: &DeltaGapProof,
        limit: usize,
        upstream_account_id: Option<i64>,
        deltas: &[DashboardActivityTerminalDelta],
    ) -> bool {
        if limit == 0 {
            return false;
        }
        // A legacy source change may have been reconstructed exactly into the rolling overlay,
        // even though its descriptor was absent.  The current index can then prove its rank from
        // the overlay; do not reject that newest-N selection solely because the historical proof
        // marker is still waiting for background reconciliation.
        if summary_delta_gap_has_valid_overlay_identity(gap, deltas, upstream_account_id) {
            return false;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&gap.occurred_at) else {
            return true;
        };
        let indexes = self
            .recent_indexes
            .get(&upstream_account_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let row_id = gap.row_id.unwrap_or(i64::MAX);
        let newer_base_count = indexes
            .iter()
            .filter_map(|index| self.current_records.get(*index))
            .filter(|record| {
                record.occurred_at > occurred_at
                    || (record.occurred_at == occurred_at && record.row.id > row_id)
            })
            .count();
        let newer_delta_count = deltas
            .iter()
            .filter(|delta| {
                upstream_account_id
                    .is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
            })
            .filter_map(|delta| {
                parse_to_utc_datetime(&delta.occurred_at).map(|delta_occurred_at| {
                    (
                        delta_occurred_at,
                        delta.persisted_row_id.unwrap_or(i64::MAX),
                    )
                })
            })
            .filter(|(delta_occurred_at, delta_row_id)| {
                *delta_occurred_at > occurred_at
                    || (*delta_occurred_at == occurred_at && *delta_row_id > row_id)
            })
            .count();
        let newer_count = newer_base_count.saturating_add(newer_delta_count);
        // A gap outside the resident prefix cannot alter a smaller current selection. If its
        // rank reaches the requested limit, the response cannot be proved exact in memory.
        limit >= newer_count.saturating_add(1)
    }

    pub(crate) fn apply_rolling_delta_to_current_response(
        &self,
        response: &mut StatsResponse,
        limit: i64,
        upstream_account_id: Option<i64>,
        deltas: &[DashboardActivityTerminalDelta],
    ) -> Result<(), ApiError> {
        if deltas.is_empty() {
            return Ok(());
        }
        let limit = limit.max(0) as usize;
        let indexes = self
            .recent_indexes
            .get(&upstream_account_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut candidates = Vec::<(DateTime<Utc>, i64, StatsTotals, i64)>::new();
        for index in indexes {
            let Some(record) = self.current_records.get(*index) else {
                return Err(ApiError::unavailable(anyhow!(
                    "summary current delta base is outside the resident index"
                )));
            };
            if upstream_account_id
                .is_some_and(|account_id| record.row.upstream_account_id != Some(account_id))
            {
                continue;
            }
            let totals = summary_projection_record_totals(record);
            let non_success_tokens = (totals.failure_count > 0).then_some(record.row.total_tokens);
            candidates.push((
                record.occurred_at,
                record.row.id,
                totals,
                non_success_tokens.unwrap_or_default(),
            ));
        }
        for delta in deltas {
            if upstream_account_id
                .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
            {
                continue;
            }
            let occurred_at = parse_to_utc_datetime(&delta.occurred_at).ok_or_else(|| {
                ApiError::unavailable(anyhow!(
                    "summary delta journal has an invalid current ordering timestamp"
                ))
            })?;
            let totals = StatsTotals {
                total_count: 1,
                success_count: i64::from(delta.success),
                failure_count: i64::from(delta.failure),
                total_tokens: delta.total_tokens,
                total_cost: delta.total_cost,
                non_success_cost: if delta.failure { delta.total_cost } else { 0.0 },
            };
            candidates.push((
                occurred_at,
                delta.persisted_row_id.unwrap_or(i64::MAX),
                totals,
                if delta.failure { delta.total_tokens } else { 0 },
            ));
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let mut totals = StatsTotals::default();
        let mut non_success_tokens = 0_i64;
        for (_, _, candidate, candidate_non_success_tokens) in candidates.into_iter().take(limit) {
            totals = totals.add(candidate);
            non_success_tokens += candidate_non_success_tokens;
        }
        response.total_count = totals.total_count;
        response.success_count = totals.success_count;
        response.failure_count = totals.failure_count;
        response.total_tokens = totals.total_tokens;
        response.total_cost = totals.total_cost;
        response.non_success_cost = Some(totals.non_success_cost);
        if response.non_success_tokens.is_some() {
            response.non_success_tokens = Some(non_success_tokens);
        }
        Ok(())
    }

    pub(crate) fn current_archive_may_affect_global_current(&self, limit: usize) -> bool {
        if limit == 0 {
            return false;
        }
        if self.current_archive_has_unknown_coverage {
            return true;
        }
        let Some(latest_coverage_end) = self.current_archive_latest_coverage_end else {
            return false;
        };
        self.current_selection_cutoff(limit)
            .is_none_or(|cutoff| latest_coverage_end > cutoff)
    }

    pub(crate) fn unavailable_archive_may_affect_global_current(&self, limit: usize) -> bool {
        if limit == 0
            || self
                .unavailable_unmaterialized_archive_current_ranges
                .is_empty()
        {
            return false;
        }
        self.current_selection_cutoff(limit).is_none_or(|cutoff| {
            self.unavailable_unmaterialized_archive_current_ranges
                .iter()
                .any(|range| range.end > cutoff)
        })
    }

    pub(crate) fn unavailable_boundary_archive_may_affect_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> bool {
        summary_projection_partial_rollup_ranges(start, end)
            .into_iter()
            .any(|partial| {
                self.unavailable_boundary_archive_ranges
                    .iter()
                    .any(|unavailable| {
                        unavailable.start < partial.end && partial.start < unavailable.end
                    })
            })
    }

    pub(crate) fn unavailable_archive_may_affect_account_current(
        &self,
        account_id: i64,
        limit: usize,
    ) -> bool {
        if limit == 0
            || self
                .unavailable_unmaterialized_archive_account_current_ranges
                .is_empty()
        {
            return false;
        }
        self.current_selection_cutoff_for_scope(Some(account_id), limit)
            .is_none_or(|cutoff| {
                self.unavailable_unmaterialized_archive_account_current_ranges
                    .iter()
                    .any(|range| range.end > cutoff)
            })
    }

    pub(crate) fn unavailable_account_boundary_archive_may_affect_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> bool {
        summary_projection_partial_rollup_ranges(start, end)
            .into_iter()
            .any(|partial| {
                self.unavailable_boundary_archive_account_ranges
                    .iter()
                    .any(|unavailable| {
                        unavailable.start < partial.end && partial.start < unavailable.end
                    })
            })
    }

    pub(crate) fn contains_persisted_live_terminal(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.persisted_live_terminal_invoke_ids
            .contains(&format!("{invoke_id}\0{occurred_at}"))
            || self
                .persisted_live_terminal_invoke_ids
                .iter()
                .any(|identity| {
                    identity
                        .split_once('\0')
                        .and_then(|(_, remainder)| remainder.split_once('\0'))
                        .is_some_and(|(identity_occurred_at, identity_invoke_id)| {
                            identity_occurred_at == occurred_at && identity_invoke_id == invoke_id
                        })
                })
    }

    pub(crate) fn durable_terminal_sequence_watermark(&self) -> u64 {
        self.durable_terminal_sequence_watermark
    }

    pub(crate) fn all_time_terminal_coverage_complete(&self) -> bool {
        self.all_time_terminal_coverage_complete
    }

    pub(crate) fn all_time_terminal_sequence_watermark(&self) -> u64 {
        self.all_time_terminal_sequence_watermark
    }

    pub(crate) fn all_time_terminal_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        _invoke_id: &str,
        _occurred_at: &str,
        terminal_sequence: u64,
    ) -> bool {
        if let Some(account_id) = upstream_account_id {
            let Some(account_watermark) = self
                .all_time_account_terminal_sequence_watermarks
                .get(&account_id)
                .copied()
            else {
                return false;
            };
            // This legacy API has no durable row ID, so a full identity proof cannot be
            // reconstructed here.  Require the scoped contiguous watermark instead of falling
            // back to the weaker invoke_id+timestamp pair.
            return account_watermark >= terminal_sequence;
        }
        summary_projection_all_time_scope_covered(
            self.all_time_terminal_coverage_complete,
            self.all_time_terminal_sequence_watermark,
            terminal_sequence,
            self.all_time_terminal_coverage_complete,
        )
    }

    pub(crate) fn all_time_terminal_identity_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
                    && record.row.upstream_account_id == upstream_account_id
            })
    }

    pub(crate) fn contains_global_rollup_covered_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.global_rollup_covered
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
            })
    }

    pub(crate) fn contains_global_all_time_covered_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.all_time_persisted_live_terminal_invoke_ids.contains(
            &summary_projection_source_identity_key(row_id, invoke_id, occurred_at),
        )
    }

    pub(crate) fn global_rollup_covers_live_terminal_identity(
        &self,
        row_id: i64,
        occurred_at: &str,
    ) -> bool {
        let Some(occurred_at) = parse_to_utc_datetime(occurred_at) else {
            return false;
        };
        let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        row_id > 0
            && row_id <= self.rollup_live_cursor
            && self.hourly_rollup_totals.contains_key(&(bucket, None))
            && self.hourly_rollup_usage.contains_key(&(bucket, None))
    }

    pub(crate) fn all_time_terminal_delta_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        persisted_row_id: Option<i64>,
        invoke_id: &str,
        occurred_at: &str,
        terminal_sequence: u64,
    ) -> bool {
        if let Some(row_id) = persisted_row_id {
            let identity = summary_projection_source_identity_key(row_id, invoke_id, occurred_at);
            if let Some(account_id) = upstream_account_id {
                return self
                    .all_time_account_persisted_live_terminal_invoke_ids
                    .get(&account_id)
                    .is_some_and(|identities| identities.contains(&identity))
                    || self.all_time_terminal_identity_scope_covers(
                        Some(account_id),
                        row_id,
                        invoke_id,
                        occurred_at,
                    );
            }
            let persisted_identity = self
                .all_time_persisted_live_terminal_invoke_ids
                .contains(&identity);
            let global_rollup_identity = self
                .contains_global_rollup_covered_live_terminal_identity(
                    row_id,
                    invoke_id,
                    occurred_at,
                );
            return persisted_identity || global_rollup_identity;
        }
        self.all_time_terminal_scope_covers(
            upstream_account_id,
            invoke_id,
            occurred_at,
            terminal_sequence,
        )
    }

    pub(crate) fn all_time_terminal_account_scopes_cover(&self, terminal_sequence: u64) -> bool {
        self.all_time_account_ids_with_projection_data
            .iter()
            .all(|account_id| {
                self.all_time_terminal_scope_covers(Some(*account_id), "", "", terminal_sequence)
            })
    }

    pub(crate) fn needs_cadence_refresh(&self, has_all_time_owner: bool) -> bool {
        let refresh_due = |refreshed_at: Option<Instant>| {
            refreshed_at.is_none_or(|refreshed_at| {
                refreshed_at.elapsed() >= SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL
            })
        };
        let now = Instant::now();
        let all_time_refresh_due = if self.all_time_manifest_admission_blocked_at.is_some() {
            summary_projection_manifest_admission_retry_is_due(
                self.all_time_manifest_admission_blocked_at,
                now,
            )
        } else {
            refresh_due(self.all_time_refreshed_at)
        };
        let account_refresh_due = if self.all_time_manifest_admission_blocked_at.is_some() {
            false
        } else if self
            .all_time_account_manifest_admission_blocked_at
            .is_some()
        {
            summary_projection_manifest_admission_retry_is_due(
                self.all_time_account_manifest_admission_blocked_at,
                now,
            )
        } else {
            refresh_due(self.all_time_oldest_account_refreshed_at)
        };
        refresh_due(self.rolling_refreshed_at())
            || (has_all_time_owner && (all_time_refresh_due || account_refresh_due))
    }

    pub(crate) fn empty_all_time_account_response(
        &self,
        upstream_account_id: Option<i64>,
    ) -> StatsResponse {
        let in_progress = self
            .in_progress_by_account
            .get(&upstream_account_id)
            .copied()
            .unwrap_or_default();
        let mut response = StatsTotals::default().into_response();
        response.non_success_cost = Some(0.0);
        response.in_progress_conversation_count = Some(in_progress.in_progress_count);
        response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
        response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
        response.in_progress_phase_counts = Some(in_progress.phase_counts);
        response.maintenance = self.maintenance.clone();
        response
    }

    pub(crate) fn response_for_query(
        &self,
        params: &SummaryQuery,
        default_limit: i64,
    ) -> Result<StatsResponse, ApiError> {
        self.response_for_query_with_rolling_delta(params, default_limit, false)
    }

    pub(crate) fn response_for_query_with_rolling_delta(
        &self,
        params: &SummaryQuery,
        default_limit: i64,
        rolling_delta_is_exact: bool,
    ) -> Result<StatsResponse, ApiError> {
        validate_summary_projection_window(params, default_limit)?;
        let rolling_refreshed_at = self.rolling_refreshed_at();
        let last_good_age_ms =
            rolling_refreshed_at.map(|refreshed_at| refreshed_at.elapsed().as_millis() as u64);
        debug!(
            ?last_good_age_ms,
            revision = self.revision,
            "serving summary projection from memory"
        );

        let window = parse_summary_window(params, default_limit)?;
        let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
        let upstream_account_id = match params.upstream_account_id {
            Some(account_id) if account_id > 0 => Some(account_id),
            Some(_) => return Err(ApiError::bad_request(anyhow!("invalid upstream account"))),
            None => None,
        };
        if matches!(window, SummaryWindow::All)
            && upstream_account_id.is_some()
            && self
                .all_time_account_manifest_admission_blocked_at
                .is_some()
            && !self.all_time_by_account.contains_key(&upstream_account_id)
        {
            return Err(ApiError::unavailable(anyhow!(
                "summary all-time account manifest admission is unavailable"
            )));
        }
        let now = Utc::now();
        let range = summary_window_range(&window, reporting_tz, now)?;
        let requested_current_limit = match window {
            SummaryWindow::Current(limit) => Some(limit.max(0) as usize),
            _ => None,
        };
        let unavailable_archive_range_affects_query = match range {
            Some((start, end)) => {
                self.unavailable_unmaterialized_archive_ranges
                    .iter()
                    .any(|unavailable| unavailable.start < end && start < unavailable.end)
                    || self.unavailable_boundary_archive_may_affect_range(start, end)
            }
            // `current` is a newest-N selection. A retained archive gap only invalidates the
            // selection if its coverage can reach this request's actual cutoff.
            None => requested_current_limit
                .is_some_and(|limit| self.unavailable_archive_may_affect_global_current(limit)),
        };
        if unavailable_archive_range_affects_query {
            let message = if range.is_none() {
                // `current` is a newest-N selection rather than a temporal interval. Preserve
                // its established boundary in the unavailable reason when an unmaterialized
                // archive can still reach the requested cutoff.
                "summary projection current source is unavailable"
            } else {
                "summary projection archive source is unavailable for the requested range"
            };
            return Err(ApiError::unavailable(anyhow!("{message}")));
        }
        let unavailable_account_archive_range_affects_query =
            upstream_account_id.is_some_and(|account_id| match range {
                Some((start, end)) => {
                    self.unavailable_unmaterialized_archive_account_ranges
                        .iter()
                        .any(|unavailable| unavailable.start < end && start < unavailable.end)
                        || self.unavailable_account_boundary_archive_may_affect_range(start, end)
                }
                None => requested_current_limit.is_some_and(|limit| {
                    self.unavailable_archive_may_affect_account_current(account_id, limit)
                }),
            });
        if unavailable_account_archive_range_affects_query {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection account archive source is unavailable for the requested range"
            )));
        }
        // A completed archive changes the historical input generation even when no bounded
        // archive-range proof was materialized into the rolling projection yet.  Do not let a
        // retained last-good rolling snapshot (or an empty projection) answer a historical
        // selection while that authority is still pending.  This is deliberately scoped to
        // archive-backed windows and leaves current/short recent windows available when their
        // own live/rollup proof is independent.
        let historical_archive_coverage_pending = range.is_some_and(|(start, end)| {
            !self.freshness.global_all_time_eligible
                && self
                    .generation_fence
                    .completed_manifest_high_watermark_id
                    .is_some()
                && self.coverage_overlay.is_none()
                // The durable historical-authority gate is required for the 30-day boundary
                // (and `all`, which is handled separately). Shorter rolling windows continue to
                // use their own exact bucket/live-tail proofs; rejecting every range beyond the
                // 48-hour live horizon here would turn valid 7-day archive-backed selections into
                // false unavailable responses before their bounded rolling proof is consulted.
                && end - start >= ChronoDuration::days(30)
        });
        if historical_archive_coverage_pending {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection historical archive coverage is not yet proven for the requested range"
            )));
        }
        if range.is_none()
            && matches!(window, SummaryWindow::Current(_))
            && (self.current_source_unavailable
                || (upstream_account_id.is_none()
                    && (self
                        .current_source_unavailable_from_rank
                        .is_some_and(|rank| {
                            requested_current_limit.is_some_and(|limit| limit >= rank)
                        })
                        || requested_current_limit.is_some_and(|limit| {
                            self.current_archive_may_affect_global_current(limit)
                        })))
                || (upstream_account_id.is_some()
                    && (self.current_account_source_unavailable
                        || upstream_account_id.is_some_and(|account_id| {
                            self.current_account_source_unavailable_from_rank
                                .get(&account_id)
                                .is_some_and(|rank| {
                                    requested_current_limit.is_some_and(|limit| limit >= *rank)
                                })
                        }))))
        {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection current source is unavailable"
            )));
        }
        if range.is_some_and(|(start, end)| {
            self.unavailable_exact_live_ranges
                .iter()
                .any(|unavailable| unavailable.start < end && start < unavailable.end)
        }) {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection persisted-live source is unavailable for the requested range"
            )));
        }
        if let Some(account_id) = upstream_account_id
            && range.is_some_and(|(start, end)| {
                self.unavailable_exact_live_account_ranges
                    .get(&account_id)
                    .is_some_and(|ranges| {
                        ranges
                            .iter()
                            .any(|unavailable| unavailable.start < end && start < unavailable.end)
                    })
            })
        {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection persisted-live account source is unavailable for the requested range"
            )));
        }
        if !rolling_delta_is_exact
            && !matches!(window, SummaryWindow::All)
            && self
                .rolling_refreshed_at()
                .is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE)
        {
            return Err(ApiError::unavailable(anyhow!(
                "summary projection last-good snapshot exceeded the freshness budget"
            )));
        }
        if matches!(window, SummaryWindow::All) {
            let account_has_projection_data = upstream_account_id.is_some_and(|account_id| {
                self.all_time_account_ids_with_projection_data
                    .contains(&account_id)
            });
            let all_time_refreshed_at = self.freshness.all_time_at(
                match upstream_account_id {
                    None => self.all_time_refreshed_at,
                    Some(_) if !account_has_projection_data => self.all_time_refreshed_at,
                    Some(account_id) => {
                        self.all_time_account_refreshed_at.get(&account_id).copied()
                    }
                },
                if account_has_projection_data {
                    upstream_account_id
                } else {
                    None
                },
            );
            if !rolling_delta_is_exact
                && all_time_refreshed_at
                    .is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE)
            {
                return Err(ApiError::unavailable(anyhow!(
                    "summary all-time last-good snapshot exceeded the freshness budget"
                )));
            }
            if let Some(response) = self.all_time_by_account.get(&upstream_account_id) {
                return Ok(response.clone());
            }
            if let Some(account_id) = upstream_account_id {
                if !account_has_projection_data && !self.known_account_ids.contains(&account_id) {
                    return Ok(self.empty_all_time_account_response(upstream_account_id));
                }
                return Err(ApiError::unavailable(anyhow!(
                    "summary all-time account snapshot is not hydrated"
                )));
            }
            return Err(ApiError::unavailable(anyhow!(
                "summary all-time snapshot is not hydrated"
            )));
        }
        let (candidate_indexes, candidate_records) = match range {
            None if matches!(window, SummaryWindow::Current(_)) => (
                std::borrow::Cow::Borrowed(
                    self.recent_indexes
                        .get(&upstream_account_id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                ),
                self.current_records.as_slice(),
            ),
            None => (
                std::borrow::Cow::Owned((0..self.records.len()).collect::<Vec<_>>()),
                self.records.as_slice(),
            ),
            Some((start, end)) => {
                let first_bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
                let last_bucket = align_bucket_epoch(end.timestamp(), 3_600, 0);
                (
                    std::borrow::Cow::Owned(
                        self.hourly_buckets
                            .range(first_bucket..=last_bucket)
                            .flat_map(|(_, indexes)| indexes.iter().copied())
                            .collect(),
                    ),
                    self.records.as_slice(),
                )
            }
        };
        let current_limit = if let SummaryWindow::Current(limit) = window {
            let limit = limit.max(0) as usize;
            if upstream_account_id.is_some()
                && !self.recent_index_complete
                && candidate_indexes.len() < limit
            {
                return Err(ApiError::unavailable(anyhow!(
                    "summary current account snapshot is outside the bounded recent index"
                )));
            }
            limit
        } else {
            usize::MAX
        };
        let full_rollup_range = range.and_then(|(start, end)| {
            let first_full_hour = ceil_hour_epoch(start.timestamp());
            let end_full_hour = align_bucket_epoch(end.timestamp(), 3_600, 0);
            (first_full_hour < end_full_hour).then_some((first_full_hour, end_full_hour))
        });
        let is_full_rollup_bucket = |record: &SummaryProjectionRecord| {
            full_rollup_range.is_some_and(|(start, end)| {
                let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                bucket >= start && bucket < end
            })
        };
        // A verified V2 overlay owns archive rows for its covered bucket. Do not retain the same
        // archive rows in the bounded candidate selection, otherwise range totals would add the
        // normalized page contribution and the resident archive records twice. Materialized
        // replacement buckets remain resident because their exact-row path supersedes the rollup.
        let exact_total_rollup_buckets = match upstream_account_id {
            None => &self.exact_global_total_rollup_buckets,
            Some(_) => &self.exact_account_total_rollup_buckets,
        };
        let exact_usage_rollup_buckets = match upstream_account_id {
            None => &self.exact_global_usage_rollup_buckets,
            Some(_) => &self.exact_account_usage_rollup_buckets,
        };
        let selected = candidate_indexes
            .iter()
            .copied()
            .take(current_limit)
            .map(|index| &candidate_records[index])
            .filter(|record| {
                upstream_account_id
                    .is_none_or(|account_id| record.row.upstream_account_id == Some(account_id))
            })
            .filter(|record| {
                range.is_none_or(|(start, end)| {
                    record.occurred_at >= start && record.occurred_at < end
                })
            })
            .filter(|record| {
                if !is_full_rollup_bucket(record) {
                    return true;
                }
                if record.is_archive_record
                    && let Some(overlay) = &self.coverage_overlay
                {
                    let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                    let overlay_owns_bucket = match upstream_account_id {
                        None => {
                            overlay.global_coverage_buckets.contains(&bucket)
                                && overlay.global_by_bucket.contains_key(&bucket)
                        }
                        Some(account_id) => {
                            overlay.account_coverage_buckets.contains(&bucket)
                                && overlay
                                    .account_by_bucket
                                    .contains_key(&(bucket, account_id))
                        }
                    };
                    if overlay_owns_bucket && !exact_total_rollup_buckets.contains(&bucket) {
                        return false;
                    }
                }
                if record.is_persisted_live_record {
                    let scope_covered = match upstream_account_id {
                        None => record.global_rollup_covered,
                        Some(_) => record.account_rollup_covered,
                    };
                    if !scope_covered {
                        return true;
                    }
                }
                match upstream_account_id {
                    None => !record.global_rollup_covered,
                    Some(_) => !record.account_rollup_covered,
                }
            })
            .collect::<Vec<_>>();
        let usage_selected = candidate_indexes
            .iter()
            .copied()
            .take(current_limit)
            .map(|index| &candidate_records[index])
            .filter(|record| {
                upstream_account_id
                    .is_none_or(|account_id| record.row.upstream_account_id == Some(account_id))
            })
            .filter(|record| {
                range.is_none_or(|(start, end)| {
                    record.occurred_at >= start && record.occurred_at < end
                })
            })
            .filter(|record| {
                if !is_full_rollup_bucket(record) {
                    return true;
                }
                if record.is_archive_record
                    && let Some(overlay) = &self.coverage_overlay
                {
                    let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                    let overlay_owns_bucket = match upstream_account_id {
                        None => {
                            overlay.global_coverage_buckets.contains(&bucket)
                                && overlay.global_usage_by_bucket.contains_key(&bucket)
                        }
                        Some(account_id) => {
                            overlay.account_coverage_buckets.contains(&bucket)
                                && overlay
                                    .account_usage_by_bucket
                                    .contains_key(&(bucket, account_id))
                        }
                    };
                    if overlay_owns_bucket && !exact_usage_rollup_buckets.contains(&bucket) {
                        return false;
                    }
                }
                if record.is_persisted_live_record {
                    let scope_covered = match upstream_account_id {
                        None => record.usage_global_rollup_covered,
                        Some(_) => record.usage_account_rollup_covered,
                    };
                    if !scope_covered {
                        return true;
                    }
                }
                let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                let usage_rollup_covered = match upstream_account_id {
                    None => record.usage_global_rollup_covered,
                    Some(_) => record.usage_account_rollup_covered,
                } && self
                    .hourly_rollup_usage
                    .contains_key(&(bucket, upstream_account_id));
                !usage_rollup_covered
            })
            .collect::<Vec<_>>();

        let mut totals = StatsTotals::default();
        let mut usage_breakdown = UsageBreakdownAccumulator::default();
        let mut non_success_tokens = 0_i64;

        for record in &selected {
            let row = &record.row;
            let classification = resolve_failure_classification(
                Some(row.status.as_str()),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            let status = normalized_runtime_text(Some(row.status.as_str()));
            let terminal = !matches!(status.as_str(), "running" | "pending");
            let success = runtime_record_is_success_for_summary_row(row)
                && classification.failure_class == FailureClass::None;
            let failure = terminal && classification.failure_class != FailureClass::None;

            totals.total_count += 1;
            totals.total_cost += row.cost.unwrap_or_default();
            totals.total_tokens += row.total_tokens;
            if success {
                totals.success_count += 1;
            }
            if failure {
                totals.failure_count += 1;
                totals.non_success_cost += row.cost.unwrap_or_default();
                non_success_tokens += row.total_tokens.max(0);
            }
        }
        for record in &usage_selected {
            usage_breakdown.add_row(&record.row);
        }

        if let Some((start, end)) = full_rollup_range {
            for ((bucket, account), bucket_totals) in &self.hourly_rollup_totals {
                if *bucket < start || *bucket >= end {
                    continue;
                }
                if *account != upstream_account_id {
                    continue;
                }
                if exact_total_rollup_buckets.contains(bucket) {
                    continue;
                }
                totals = totals.add(*bucket_totals);
                non_success_tokens += self
                    .hourly_rollup_non_success_tokens
                    .get(&(*bucket, *account))
                    .copied()
                    .unwrap_or_default();
            }
            for ((bucket, account), usage) in &self.hourly_rollup_usage {
                if *bucket >= start
                    && *bucket < end
                    && *account == upstream_account_id
                    && !exact_usage_rollup_buckets.contains(bucket)
                {
                    usage_breakdown.merge_response(usage);
                }
            }
            // Verified V2 pages are compact historical contributions for buckets which are not
            // represented by the resident rollup. Merge them only for complete buckets; a
            // partial boundary still requires the existing exact-row proof and remains local
            // unavailable when that proof is absent.
            if let Some(overlay) = &self.coverage_overlay {
                match upstream_account_id {
                    None => {
                        for (bucket, bucket_totals) in &overlay.global_by_bucket {
                            if *bucket >= start
                                && *bucket < end
                                && overlay.global_coverage_buckets.contains(bucket)
                                && !exact_total_rollup_buckets.contains(bucket)
                            {
                                totals = totals.add(*bucket_totals);
                                non_success_tokens += overlay
                                    .global_non_success_tokens_by_bucket
                                    .get(bucket)
                                    .copied()
                                    .unwrap_or_default();
                            }
                        }
                        for (bucket, usage) in &overlay.global_usage_by_bucket {
                            if *bucket >= start
                                && *bucket < end
                                && overlay.global_coverage_buckets.contains(bucket)
                                && !exact_usage_rollup_buckets.contains(bucket)
                            {
                                usage_breakdown.merge_response(usage);
                            }
                        }
                    }
                    Some(account_id) => {
                        for ((bucket, candidate_account), bucket_totals) in
                            &overlay.account_by_bucket
                        {
                            if *candidate_account == account_id
                                && *bucket >= start
                                && *bucket < end
                                && overlay.account_coverage_buckets.contains(bucket)
                                && !exact_total_rollup_buckets.contains(bucket)
                            {
                                totals = totals.add(*bucket_totals);
                                non_success_tokens += overlay
                                    .account_non_success_tokens_by_bucket
                                    .get(&(*bucket, account_id))
                                    .copied()
                                    .unwrap_or_default();
                            }
                        }
                        for ((bucket, candidate_account), usage) in &overlay.account_usage_by_bucket
                        {
                            if *candidate_account == account_id
                                && *bucket >= start
                                && *bucket < end
                                && overlay.account_coverage_buckets.contains(bucket)
                                && !exact_usage_rollup_buckets.contains(bucket)
                            {
                                usage_breakdown.merge_response(usage);
                            }
                        }
                    }
                }
            }
            // V2 Snapshot rows are the exact source for a partial boundary.  They are kept
            // separate from the compact bucket totals so a 30d/calendar range beginning inside
            // an hour does not silently include the whole hour or undercount it.
            if let (Some(overlay), Some((range_start, range_end))) = (&self.coverage_overlay, range)
            {
                for record in &overlay.boundary_records {
                    if !is_full_rollup_bucket(record)
                        && record.occurred_at >= range_start
                        && record.occurred_at < range_end
                        && upstream_account_id.is_none_or(|account_id| {
                            record.row.upstream_account_id == Some(account_id)
                        })
                    {
                        let record_totals = summary_projection_record_totals(record);
                        totals = totals.add(record_totals);
                        usage_breakdown.add_row(&record.row);
                        if record_totals.failure_count > 0 {
                            non_success_tokens += record.row.total_tokens.max(0);
                        }
                    }
                }
            }
        }

        let mut response = totals.into_response();
        response.non_success_cost = Some(totals.non_success_cost);
        if range.is_some() {
            response.usage_breakdown = Some(usage_breakdown.into_response());
        }

        let policy = summary_live_augmentation_policy(&window, range, now);
        if policy.include_in_progress {
            let in_progress = self
                .in_progress_by_account
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            response.in_progress_conversation_count = Some(in_progress.in_progress_count);
            response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
            response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
            response.in_progress_phase_counts = Some(in_progress.phase_counts);
        }
        if policy.include_non_success_tokens {
            response.non_success_tokens = Some(non_success_tokens);
        }
        response.maintenance = self.maintenance.clone();
        Ok(response)
    }
}

pub(crate) fn summary_projection_all_time_sequence_watermark(
    all_time_was_fully_rebuilt: bool,
    all_time_terminal_coverage_complete: bool,
    durable_terminal_sequence_watermark: u64,
    previous_all_time_terminal_sequence_watermark: u64,
) -> u64 {
    if all_time_was_fully_rebuilt && all_time_terminal_coverage_complete {
        durable_terminal_sequence_watermark
    } else {
        previous_all_time_terminal_sequence_watermark
    }
}

pub(crate) fn summary_projection_all_time_scope_covered(
    global_coverage_complete: bool,
    terminal_sequence_watermark: u64,
    terminal_sequence: u64,
    identity_covered: bool,
) -> bool {
    global_coverage_complete
        && (terminal_sequence_watermark >= terminal_sequence || identity_covered)
}

pub(crate) fn runtime_record_is_success_for_summary_row(
    row: &UpstreamAccountInvocationPreviewRow,
) -> bool {
    let status = normalized_runtime_text(Some(row.status.as_str()));
    matches!(
        status.as_str(),
        "success" | "succeeded" | "completed" | "ok" | INVOCATION_STATUS_WARNING_SUCCESS
    ) || (status == "http_200" && normalized_runtime_text(row.error_message.as_deref()).is_empty())
}

pub(crate) fn summary_projection_record_totals(record: &SummaryProjectionRecord) -> StatsTotals {
    summary_projection_record_totals_with_archived_pending_terminal(record, false)
}

pub(crate) fn summary_projection_all_time_record_totals(
    record: &SummaryProjectionRecord,
) -> StatsTotals {
    // Completed archive batches can contain legacy rows whose status stayed `pending` after the
    // terminal writer failed. The all-time rollup contract treats those retained historical rows
    // as terminal; their normal failure classification still determines non-success totals.
    summary_projection_record_totals_with_archived_pending_terminal(record, true)
}

pub(crate) fn summary_projection_all_time_uses_global_exact_record(
    record: &SummaryProjectionRecord,
    rollup_live_cursor: i64,
) -> bool {
    (record.is_persisted_live_record || record.archive_has_materialized_rollups)
        && !record.global_rollup_covered
        // The bounded global live-tail aggregate already owns rows after its cursor.
        && !(record.is_persisted_live_record && record.row.id > rollup_live_cursor)
}

pub(crate) fn summary_projection_all_time_uses_account_exact_record(
    record: &SummaryProjectionRecord,
    account_rollup_live_cursor: Option<i64>,
) -> bool {
    (record.is_persisted_live_record || record.archive_has_materialized_rollups)
        && !record.account_rollup_covered
        // The bounded account live-tail aggregate already owns rows after its cursor. An absent
        // cursor deliberately leaves the exact retained record visible instead of undercounting.
        && !(record.is_persisted_live_record
            && account_rollup_live_cursor.is_some_and(|cursor| record.row.id > cursor))
}

pub(crate) fn summary_projection_record_totals_with_archived_pending_terminal(
    record: &SummaryProjectionRecord,
    archived_pending_is_terminal: bool,
) -> StatsTotals {
    let classification = resolve_failure_classification(
        Some(record.row.status.as_str()),
        record.row.error_message.as_deref(),
        record.row.failure_kind.as_deref(),
        record.row.failure_class.as_deref(),
        record.row.is_actionable,
    );
    let status = normalized_runtime_text(Some(record.row.status.as_str()));
    let terminal = !matches!(status.as_str(), "running" | "pending")
        || (archived_pending_is_terminal && record.is_archive_record && status == "pending");
    let mut totals = StatsTotals {
        total_count: 1,
        total_tokens: record.row.total_tokens,
        total_cost: record.row.cost.unwrap_or_default(),
        ..StatsTotals::default()
    };
    if runtime_record_is_success_for_summary_row(&record.row)
        && classification.failure_class == FailureClass::None
    {
        totals.success_count = 1;
    }
    if terminal && classification.failure_class != FailureClass::None {
        totals.failure_count = 1;
        totals.non_success_cost = record.row.cost.unwrap_or_default();
    }
    totals
}

pub(crate) fn summary_projection_account_record_totals(
    record: &SummaryProjectionRecord,
) -> Option<(i64, StatsTotals)> {
    let account_id = record.row.upstream_account_id.filter(|id| *id > 0)?;
    if record.account_rollup_covered
        || record.account_archive_totals_fallback_included
        // Unmaterialized archive totals are loaded once by the bounded account aggregate below;
        // retaining the exact row for rolling/usage views must not add it to all-time a second time.
        || (record.is_archive_record && !record.archive_has_materialized_rollups)
    {
        return None;
    }
    Some((account_id, summary_projection_record_totals(record)))
}

pub(crate) fn summary_projection_optional_string_bytes(value: Option<&String>) -> usize {
    value.map_or(0, String::len)
}

pub(crate) fn summary_projection_preview_row_bytes(
    row: &UpstreamAccountInvocationPreviewRow,
) -> usize {
    row.invoke_id.len()
        + row.occurred_at.len()
        + row.status.len()
        + summary_projection_optional_string_bytes(row.prompt_cache_key.as_ref())
        + summary_projection_optional_string_bytes(row.conversation_created_at.as_ref())
        + summary_projection_optional_string_bytes(row.live_phase.as_ref())
        + summary_projection_optional_string_bytes(row.failure_class.as_ref())
        + summary_projection_optional_string_bytes(row.route_mode.as_ref())
        + summary_projection_optional_string_bytes(row.model.as_ref())
        + summary_projection_optional_string_bytes(row.request_model.as_ref())
        + summary_projection_optional_string_bytes(row.response_model.as_ref())
        + summary_projection_optional_string_bytes(row.reasoning_effort.as_ref())
        + summary_projection_optional_string_bytes(row.source.as_ref())
        + summary_projection_optional_string_bytes(row.error_message.as_ref())
        + summary_projection_optional_string_bytes(row.downstream_error_message.as_ref())
        + summary_projection_optional_string_bytes(row.failure_kind.as_ref())
        + summary_projection_optional_string_bytes(row.proxy_display_name.as_ref())
        + summary_projection_optional_string_bytes(row.upstream_account_name.as_ref())
        + summary_projection_optional_string_bytes(row.upstream_account_plan_type.as_ref())
        + summary_projection_optional_string_bytes(row.response_content_encoding.as_ref())
        + summary_projection_optional_string_bytes(row.request_compression_algorithm.as_ref())
        + summary_projection_optional_string_bytes(row.transport.as_ref())
        + summary_projection_optional_string_bytes(row.requested_service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.billing_service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.endpoint.as_ref())
        + summary_projection_optional_string_bytes(row.compaction_request_kind.as_ref())
        + summary_projection_optional_string_bytes(row.compaction_response_kind.as_ref())
        + summary_projection_optional_string_bytes(row.image_intent.as_ref())
}

pub(crate) fn ensure_summary_projection_resident_record_bytes(
    rolling_record_bytes: usize,
    current_record_bytes: usize,
) -> Result<()> {
    let resident_record_bytes = rolling_record_bytes
        .checked_add(current_record_bytes)
        .ok_or_else(|| anyhow!("summary projection resident record byte accounting overflowed"))?;
    if resident_record_bytes > SUMMARY_PROJECTION_MAX_PREVIEW_BYTES {
        return Err(anyhow!(
            "summary projection resident record bytes exceeded bounded budget ({SUMMARY_PROJECTION_MAX_PREVIEW_BYTES})"
        ));
    }
    Ok(())
}

pub(crate) fn summary_projection_resident_record_budget_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("resident record bytes exceeded bounded budget")
}

pub(crate) fn summary_projection_archive_row_bytes(row: &SummaryProjectionArchiveRow) -> usize {
    row.invoke_id.len()
        + row.occurred_at.len()
        + row.source.len()
        + summary_projection_optional_string_bytes(row.model.as_ref())
        + summary_projection_optional_string_bytes(row.response_model.as_ref())
        + summary_projection_optional_string_bytes(row.reasoning_effort.as_ref())
        + row.status.len()
        + summary_projection_optional_string_bytes(row.error_message.as_ref())
        + summary_projection_optional_string_bytes(row.failure_kind.as_ref())
        + summary_projection_optional_string_bytes(row.failure_class.as_ref())
}

pub(crate) async fn ensure_summary_projection_archive_text_budget(
    archive_pool: &Pool<Sqlite>,
    exact_ranges: &[ExactUtcRange],
    columns: &HashMap<&str, bool>,
    max_resident_bytes: usize,
) -> Result<(), anyhow::Error> {
    if exact_ranges.is_empty() {
        return Ok(());
    }
    let mut text_columns = vec!["invoke_id", "occurred_at"];
    for column in [
        "source",
        "model",
        "payload",
        "status",
        "error_message",
        "failure_kind",
        "failure_class",
    ] {
        if columns.get(column).copied().unwrap_or(false) {
            text_columns.push(column);
        }
    }
    text_columns.sort_unstable();
    text_columns.dedup();
    let row_bytes = text_columns
        .iter()
        .map(|column| format!("length(CAST(COALESCE({column}, '') AS BLOB))"))
        .collect::<Vec<_>>()
        .join(" + ");
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COALESCE(SUM(row_bytes), 0) FROM (SELECT 512 + ");
    query
        .push(row_bytes.as_str())
        .push(" AS row_bytes FROM codex_invocations WHERE ");
    for (index, _) in exact_ranges.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(exact_ranges[index].start));
        query
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(exact_ranges[index].end))
            .push(")");
    }
    query.push(")");
    let estimated_bytes = query
        .build_query_scalar::<i64>()
        .fetch_one(archive_pool)
        .await?;
    if estimated_bytes > max_resident_bytes as i64 {
        return Err(anyhow!(
            "summary projection archive byte budget exceeded before preview fetch ({estimated_bytes} > {max_resident_bytes})"
        ));
    }
    Ok(())
}
