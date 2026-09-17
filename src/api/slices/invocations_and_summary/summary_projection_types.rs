pub(crate) const SUMMARY_SNAPSHOT_MAX_STALE: Duration = Duration::from_secs(15);
pub(crate) const SUMMARY_SNAPSHOT_MAX_KEYS: usize = 48;
// Owner checks are memory-only. Polling at this cadence gives the runtime refresh a bounded
// scheduling phase without performing SQLite/archive work while no consumer owns Summary.
const SUMMARY_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const SUMMARY_LIVE_TAIL_RECONCILIATION_QUEUE_WAIT: Duration = Duration::from_secs(2);
const SUMMARY_HISTORICAL_COVERAGE_RECOVERY_QUEUE_WAIT: Duration = Duration::from_secs(30);
const SUMMARY_HISTORICAL_COVERAGE_RECOVERY_CONTINUATION_QUEUE_WAIT: Duration =
    Duration::from_secs(1);
// A V2 page first becomes durable authority. Publishing that authority is intentionally less
// frequent than page commits: replacing the immutable read model copies its bounded resident
// preview, so publishing every page turns long historical recovery into copy-bound work.
const SUMMARY_HISTORICAL_COVERAGE_OVERLAY_PUBLICATION_PAGE_BATCHES: usize = 16;
// Keep each low-priority recovery turn long enough to amortize archive open/hash/decode setup
// across several bounded pages. The per-page 64 MiB/400-ID limits and pressure permit remain
// unchanged; this is a scheduler batch budget, not an HTTP or rolling freshness deadline.
const SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET: Duration = Duration::from_secs(30);
const SUMMARY_SNAPSHOT_EVENT_DEBOUNCE: Duration = Duration::from_millis(250);
// Allow for hub coordination after a due tick. Hub state mutex holders never await durable I/O,
// but this explicit budget keeps that short handoff out of the freshness critical path.
const SUMMARY_SNAPSHOT_COORDINATION_SLACK: Duration = Duration::from_millis(500);
const SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
// Do not turn a locked or overloaded SQLite database into a continuous sequence of timed-out
// full hydrations. Mutations remain coalesced as dirty while this bounded retry gate is active.
const SUMMARY_SNAPSHOT_FAILURE_RETRY_BACKOFF: Duration = Duration::from_secs(30);
const SUMMARY_PROJECTION_MANIFEST_ADMISSION_RETRY_BACKOFF: Duration = Duration::from_secs(5 * 60);
// A cadence refresh begins after the 10-second floor without event debounce. Its scheduling
// phase, hub coordination allowance, and build deadline remain strictly inside the 15-second
// serving ceiling; mutations still use the separate debounce below.
const SUMMARY_PROJECTION_BUILD_DEADLINE: Duration = Duration::from_secs(4);
// Finalizing an all-time checkpoint may still need to assemble the bounded current/rolling
// source set. Keep this background-only phase independent from the cadence refresh budget: it
// has enough time to finish a proven historical reconciliation, while Bootstrap publication and
// changed-generation rolling refreshes retain their shorter availability deadlines.
const SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE: Duration = Duration::from_secs(30);
// Startup publishes the independently exact current and rolling selections before background
// all-time reconciliation. The initial budget therefore remains a bounded readiness gate rather
// than a deadline for a full-history scan.
pub(crate) const SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE: Duration = Duration::from_secs(30);
const SUMMARY_PROJECTION_MAX_EXACT_RECORDS: usize = 50_000;

#[derive(Debug)]
struct SummaryProjectionAllTimeGenerationChanged;

impl std::fmt::Display for SummaryProjectionAllTimeGenerationChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("summary all-time reconciliation observed a changed durable generation")
    }
}

impl std::error::Error for SummaryProjectionAllTimeGenerationChanged {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SummaryProjectionBuildMode {
    Bootstrap,
    // A published rolling baseline can consume the hub-owned committed terminal journal
    // without re-admitting the complete live source.
    RollingDelta,
    Rolling,
    HistoricalLiveCoverage,
    AllTime,
}

impl SummaryProjectionBuildMode {
    const fn includes_all_time(self) -> bool {
        matches!(self, Self::AllTime)
    }

    const fn requires_full_historical_live_coverage(self) -> bool {
        matches!(self, Self::Bootstrap | Self::HistoricalLiveCoverage)
    }
}

#[cfg(test)]
tokio::task_local! {
    static SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT: usize;
}

fn summary_projection_exact_record_limit() -> usize {
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
const SUMMARY_PROJECTION_MAX_ACCOUNTS: usize = 50_000;
// Exact archive-boundary hydration remains bounded independently from the compact all-time
// rollup baseline. Path-based SQLite reads are chunked below to stay well under bind limits.
const SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES: usize = 4_096;
const SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE: usize = 512;
// Exact replacement is intentionally exceptional. Keeping this separately bounded prevents a
// wide retention horizon with a compact-coverage gap from turning a background rebuild into an
// uninterruptible hour-by-hour allocation and CPU sweep.
const SUMMARY_PROJECTION_MAX_EXACT_BUCKETS: usize = 4_096;
const SUMMARY_PROJECTION_MAX_PREVIEW_BYTES: usize = 64 * 1024 * 1024;
// Live source admission is independent from the resident preview budget. Each source record
// and packed hydration page remains bounded, while multiple pages may be admitted in one build.
const SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES: usize = 64 * 1024 * 1024;
const SUMMARY_PROJECTION_MAX_SOURCE_PAGE_BYTES: usize = 64 * 1024 * 1024;
// Rollup tables are compact but can still become unbounded when account/model cardinality
// grows. Count and size-limit their background hydration before materializing rows in memory.
const SUMMARY_PROJECTION_MAX_ROLLUP_ROWS: usize = 200_000;
const SUMMARY_PROJECTION_MAX_ROLLUP_BYTES: usize = 32 * 1024 * 1024;
const SUMMARY_PROJECTION_MAX_LIVE_AGGREGATE_ROWS: usize = SUMMARY_PROJECTION_MAX_ROLLUP_ROWS;
const SUMMARY_PROJECTION_MIN_EXACT_HORIZON: ChronoDuration = ChronoDuration::hours(48);
// Summary duration/calendar ranges are backed by the configured live retention plus the
// archive grace used by the existing range readers. A legal `thisMonth` can span nearly 31
// days, so the finite calendar guard must cover that longest named range even when retention
// is zero. HTTP still reads only the resulting in-memory projection.
const SUMMARY_PROJECTION_ARCHIVE_GRACE_DAYS: u64 = 31;
// The public Summary UI exposes rolling windows through 1mo. Keep arbitrary parser inputs
// bounded at the HTTP contract so the projection never has to retain an unbounded set of
// partial-hour boundaries; longer requests fail before hub/SQLite access.
const SUMMARY_PROJECTION_MAX_DURATION: ChronoDuration = ChronoDuration::days(30);

fn summary_projection_exact_horizon(invocation_max_days: u64) -> ChronoDuration {
    let supported_days = invocation_max_days.saturating_add(SUMMARY_PROJECTION_ARCHIVE_GRACE_DAYS);
    ChronoDuration::days(supported_days.max(2) as i64).max(SUMMARY_PROJECTION_MIN_EXACT_HORIZON)
}

fn validate_summary_projection_window(
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

fn summary_projection_boundary_buckets(end: DateTime<Utc>) -> HashSet<i64> {
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
            for spec in ["today", "yesterday", "thisWeek", "thisMonth"] {
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
    window: String,
    limit: Option<i64>,
    time_zone: String,
    upstream_account_id: Option<i64>,
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

    fn query(&self) -> SummaryQuery {
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
    row: UpstreamAccountInvocationPreviewRow,
    occurred_at: DateTime<Utc>,
    // Full-hour rollups cover persisted records only through their durable cursor. Archive
    // materialization has separate global and account coverage, so a global bucket never
    // suppresses an account-only fallback record.
    global_rollup_covered: bool,
    account_rollup_covered: bool,
    usage_global_rollup_covered: bool,
    usage_account_rollup_covered: bool,
    is_persisted_live_record: bool,
    // Unmaterialized archive rows are folded by the bounded account archive aggregate. Keep
    // them out of the exact-record repair pass so all-time account totals are not doubled.
    is_archive_record: bool,
    archive_has_materialized_rollups: bool,
    // When account replay is unavailable, the durable account archive aggregate already
    // contains this archive row. Keep the exact row for boundary/range views, but do not fold it
    // into the all-time account aggregate a second time.
    account_archive_totals_fallback_included: bool,
}

fn summary_projection_record_identity_matches(
    record: &SummaryProjectionRecord,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> bool {
    record.row.id == id
        && record.row.invoke_id == invoke_id
        && record.row.occurred_at == occurred_at
}

fn summary_projection_records_contains_identity(
    records: &HashMap<String, SummaryProjectionRecord>,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> bool {
    records.values().any(|record| {
        summary_projection_record_identity_matches(record, id, invoke_id, occurred_at)
    })
}

fn summary_projection_record_bytes_for_identity(
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

fn summary_projection_record_source_identity(
    record: &SummaryProjectionRecord,
) -> SummarySourceIdentity {
    SummarySourceIdentity {
        row_id: record.row.id,
        invoke_id: record.row.invoke_id.clone(),
        occurred_at: record.row.occurred_at.clone(),
    }
}

fn summary_projection_record_identity_key(record: &SummaryProjectionRecord) -> String {
    summary_projection_source_identity_key(
        record.row.id,
        &record.row.invoke_id,
        &record.row.occurred_at,
    )
}

fn summary_projection_source_identity_key(
    row_id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> String {
    format!("{row_id}\0{occurred_at}\0{invoke_id}")
}

fn summary_projection_delta_identity_key(delta: &DashboardActivityTerminalDelta) -> String {
    delta.persisted_row_id.map_or_else(
        || format!("legacy\0{}\0{}", delta.occurred_at, delta.invoke_id),
        |row_id| {
            summary_projection_source_identity_key(row_id, &delta.invoke_id, &delta.occurred_at)
        },
    )
}

fn summary_projection_record_insert_key(
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
struct SummaryProjectionArchiveReplayCoverage {
    overall: bool,
    account_stats: bool,
    usage_breakdown: bool,
}

impl SummaryProjectionArchiveReplayCoverage {
    fn supports_unavailable_archive(self) -> bool {
        self.overall && self.account_stats && self.usage_breakdown
    }
}

#[derive(Debug, Clone, Default)]
struct SummaryProjectionFreshness {
    global_all_time_eligible: bool,
    account_all_time_eligible: HashSet<i64>,
}

#[derive(Debug, Clone)]
struct SummaryProjectionHistoricalLiveCoverage {
    range: ExactUtcRange,
    high_watermark_id: i64,
    reconciliation_required: bool,
}

/// A verified historical contribution which can be atomically attached to an existing
/// projection without rerunning live admission.  The overlay retains only normalized totals;
/// raw archive payloads and preview rows remain outside the serving read model.
#[derive(Debug, Clone)]
struct SummaryCoverageOverlay {
    coverage_fence: SummaryCoverageFence,
    live_tail_cursor: SummaryLiveTailCursor,
    // This is the durable identity set represented by the normalized maps below. Keeping it
    // with the immutable overlay lets recovery reduce only newly verified V2 pages instead of
    // rereading every historical manifest on each publication turn.
    proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    recent_proof_identities: HashSet<SummaryArchiveSnapshotProofIdentity>,
    global_coverage_buckets: HashSet<i64>,
    account_coverage_buckets: HashSet<i64>,
    // Materialized archives retain their durable rollup as the aggregate baseline. Keep this
    // distinction alongside V2 coverage so an unmaterialized sibling in the same hour does not
    // cause finalization to subtract the materialized rollup before adding only the sibling.
    materialized_coverage_buckets: HashSet<i64>,
    // Legacy manifests without explicit coverage bounds can only prove materialization at the
    // month granularity. Preserve that proof so mixed archive siblings do not replace a whole
    // month of compact rollups merely because one sibling contributes V2 rows.
    materialized_coverage_months: HashSet<String>,
    global_by_bucket: HashMap<i64, StatsTotals>,
    account_by_bucket: HashMap<(i64, i64), StatsTotals>,
    global_usage_by_bucket: HashMap<i64, UsageBreakdownResponse>,
    account_usage_by_bucket: HashMap<(i64, i64), UsageBreakdownResponse>,
    global_non_success_tokens_by_bucket: HashMap<i64, i64>,
    account_non_success_tokens_by_bucket: HashMap<(i64, i64), i64>,
    // Boundary rows are retained only for ranges that still need partial-hour exactness. Full
    // hours continue to use compact totals so a large archive never enters the resident record
    // budget merely because its V2 proof was published.
    boundary_records: Vec<SummaryProjectionRecord>,
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
struct SummaryProjectionFreshnessLease {
    origin: Instant,
    renewed_elapsed_ms: Arc<AtomicU64>,
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
    fn renewed_at(&self) -> Option<Instant> {
        let elapsed_ms = self.renewed_elapsed_ms.load(Ordering::Relaxed);
        (elapsed_ms > 0).then(|| self.origin + Duration::from_millis(elapsed_ms))
    }

    fn latest(&self, refreshed_at: Option<Instant>) -> Option<Instant> {
        match (refreshed_at, self.renewed_at()) {
            (Some(refreshed_at), Some(renewed_at)) => Some(refreshed_at.max(renewed_at)),
            (Some(refreshed_at), None) => Some(refreshed_at),
            (None, Some(renewed_at)) => Some(renewed_at),
            (None, None) => None,
        }
    }

    fn renew(&self) {
        let elapsed_ms = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.renewed_elapsed_ms
            .fetch_max(elapsed_ms.max(1), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct SummaryProjectionGenerationFence {
    live_high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    completed_manifest_high_watermark_id: Option<i64>,
    coverage_revision: i64,
    account_coverage_revision: i64,
    durable_terminal_sequence_watermark: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct SummaryCoverageFence {
    pub(crate) completed_manifest_high_watermark_id: Option<i64>,
    pub(crate) coverage_revision: i64,
    pub(crate) account_coverage_revision: i64,
}

impl SummaryCoverageFence {
    fn global_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.coverage_revision == other.coverage_revision
    }

    fn account_sources_match(self, other: Self) -> bool {
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
    fn terminal_sources_match(self, other: Self) -> bool {
        self.live_high_watermark_id == other.live_high_watermark_id
            && self.rollup_live_cursor == other.rollup_live_cursor
            && self.account_rollup_live_cursor == other.account_rollup_live_cursor
            && self.durable_terminal_sequence_watermark == other.durable_terminal_sequence_watermark
    }

    fn rollup_sources_match(self, other: Self) -> bool {
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

    fn global_coverage_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.coverage_revision == other.coverage_revision
    }

    fn account_coverage_sources_match(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && self.account_coverage_revision == other.account_coverage_revision
    }

    // A checkpoint cursor is valid only for the exact coverage revision that produced it. Any
    // proof mutation is a new input generation and must force the worker to re-establish its
    // cursor from the reset checkpoint rather than continuing a stale page.
    fn global_coverage_checkpoint_compatible(self, other: Self) -> bool {
        self.completed_manifest_high_watermark_id == other.completed_manifest_high_watermark_id
            && other.coverage_revision == self.coverage_revision
    }

    fn account_coverage_checkpoint_compatible(self, other: Self) -> bool {
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
    fn rolling_at(&self, built_at: Option<Instant>) -> Option<Instant> {
        built_at
    }

    fn all_time_at(&self, built_at: Option<Instant>, account_id: Option<i64>) -> Option<Instant> {
        let eligible = match account_id {
            None => self.global_all_time_eligible,
            Some(account_id) => self.account_all_time_eligible.contains(&account_id),
        };
        if eligible { built_at } else { None }
    }
}

fn summary_projection_manifest_admission_retry_is_due(
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
    records: Vec<SummaryProjectionRecord>,
    // `current` is a newest-N selection rather than a time range. Keep its bounded prefix
    // separate from the rolling/archive repair records so a large historical prefix cannot make
    // two individually-admitted read-model inputs exceed one shared record budget.
    current_records: Vec<SummaryProjectionRecord>,
    // A Summary SSE base can safely omit a terminal overlay only after the durable record is
    // known to be in this exact projection revision. Keep the bounded identity set alongside
    // the canonical live records so reconnects never infer that coverage from a global cursor.
    persisted_live_terminal_invoke_ids: HashSet<String>,
    // Rolling refreshes may reuse the previous all-time aggregate. Until a successful all-time
    // rebuild, that aggregate cannot suppress a newly acknowledged terminal delta. This flag is
    // deliberately global-scope only; account eligibility is tracked independently in freshness.
    all_time_terminal_coverage_complete: bool,
    // RollingDelta advances the current generation fence while retaining the last exact
    // all-time aggregate. Keep the coverage fence that actually produced each all-time scope
    // separate so a newly ready checkpoint cannot be mistaken for already published data.
    global_all_time_coverage_fence: Option<SummaryCoverageFence>,
    account_all_time_coverage_fence: Option<SummaryCoverageFence>,
    // The settled terminal watermark actually represented by the retained all-time aggregate.
    // Rolling-only revisions must not advance this proof while they reuse the old aggregate.
    all_time_terminal_sequence_watermark: u64,
    // Exact persisted live identities observed by a complete global all-time rebuild. Keeping
    // this separate from the rolling identity set closes the hydration race where a terminal is
    // committed after the settled watermark sample but before the aggregate query sees it.
    all_time_persisted_live_terminal_invoke_ids: HashSet<String>,
    // Account all-time aggregates have an independent durable source and therefore need an
    // independent terminal proof. A global watermark must never suppress an account overlay
    // when that account's aggregate was retained or rebuilt under a different source boundary.
    all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
    // Captured immediately before the durable hydration queries. A settled sequence means every
    // preceding terminal write has committed, so a successful projection can safely recover a
    // bounded SSE overlay that overflowed before its next rebuild.
    durable_terminal_sequence_watermark: u64,
    // Buckets are an index over canonical records, not a second source of truth.  They bound
    // rolling/calendar selection work without approximating a timezone boundary.
    hourly_buckets: BTreeMap<i64, Vec<usize>>,
    // Materialized history fallback.  Only compact totals remain resident; archive rows are
    // released after hydration.
    hourly_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_non_success_tokens: HashMap<(i64, Option<i64>), i64>,
    // Usage rollups carry the model/reasoning dimension which is absent from StatsTotals.
    // Keep both account and global keys so a materialized archive never turns an exact
    // usage breakdown into an empty response on the memory-only request path.
    hourly_rollup_usage: HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    // A replay marker can lag after a historical rollup has been partially materialized.  For
    // these buckets hydration retains the archive plus every persisted live row as the exact
    // replacement source.  The request reducer must therefore suppress the compact bucket only
    // for the affected scope, never merely because one archive row happened to be selected.
    exact_global_total_rollup_buckets: HashSet<i64>,
    exact_account_total_rollup_buckets: HashSet<i64>,
    exact_global_usage_rollup_buckets: HashSet<i64>,
    exact_account_usage_rollup_buckets: HashSet<i64>,
    rollup_live_cursor: i64,
    // Historical live coverage is verified during Bootstrap or a dedicated background pass.
    // Rolling rebuilds reuse its unchanged proof and localize only newly unproven buckets.
    historical_live_coverage: Option<SummaryProjectionHistoricalLiveCoverage>,
    // Verified V2 archive contributions are attached independently from the live-tail rebuild.
    // Keeping the overlay in the projection lets subsequent rolling refreshes retain a proof
    // published by the historical supervisor.
    coverage_overlay: Option<SummaryCoverageOverlay>,
    // These indexes are constructed once off-request.  They retain only the largest legal
    // current-window result for the global view and each account, so a hot `current` read never
    // sorts or allocates in proportion to retained history.
    recent_indexes: HashMap<Option<i64>, Vec<usize>>,
    // The bounded global recent scan proves an account's top N only when N entries survived in
    // its index. If the scan overflowed and fewer entries are retained, serving a shorter list
    // would silently omit older invocations, so that selection uses unavailable instead.
    recent_index_complete: bool,
    // The first row omitted by the globally ordered `current` index. A rolling range that reaches
    // this boundary cannot prove its live/rollup partition is complete without request-time I/O,
    // even though its separate boundary-record view may retain selected older rows.
    recent_index_overflow_at: Option<DateTime<Utc>>,
    // `all` has no reporting-timezone boundary.  Hydration therefore folds the legacy
    // archive/rollup coverage rules into a canonical aggregate per account once, instead of
    // attempting to recreate those rules from raw archive rows on every request.
    all_time_by_account: HashMap<Option<i64>, StatsResponse>,
    // Regular rolling refreshes may reuse all-time responses when no all-time owner is active;
    // keep their freshness independent from the projection-wide timestamp.
    all_time_refreshed_at: Option<Instant>,
    // An all-history manifest set beyond admission cannot be made exact without an unbounded
    // proof. Retry only after a long, controlled backoff so maintenance does not recreate the
    // production pressure pattern while a recoverable all-time source is unavailable.
    all_time_manifest_admission_blocked_at: Option<Instant>,
    // Account discovery has a distinct admission budget. It must not suppress an exact global
    // all-time aggregate when only the account scope cannot be proven.
    all_time_account_manifest_admission_blocked_at: Option<Instant>,
    // Global all-time rollups and account archive fallbacks have independent durable coverage.
    // A failed account rebuild must not make a fresh global aggregate stale, and an old account
    // response must never borrow the global aggregate's freshness.
    all_time_account_refreshed_at: HashMap<i64, Instant>,
    // Pre-computed at build time so the 250ms owner cadence never scans one timestamp per
    // account merely to decide whether an all-time refresh is due.
    all_time_oldest_account_refreshed_at: Option<Instant>,
    // This separates a caller-provided, unknown account (whose all-time aggregate is exactly
    // zero once the global projection is fresh) from an account whose durable history could not
    // be hydrated. The latter must retain the unavailable contract rather than become zero.
    all_time_account_ids_with_projection_data: HashSet<i64>,
    known_account_ids: HashSet<i64>,
    // Archive files are immutable once completed. Reuse their bounded account-key discovery on
    // later refreshes so maintenance never repeatedly scans a large low-cardinality archive.
    archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    // Legacy archive manifests can lack coverage bounds. Preserve a successfully measured
    // immutable range with its account discovery cache so later all-time refreshes retain the
    // same exact-source proof instead of degrading a valid account response.
    archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    // An unreadable archive without complete durable replay has no exact in-memory source.
    // Keep its bounded overlap so only affected rolling/calendar selections fail closed rather
    // than publishing an undercount or turning a selection-level unavailability into an HTTP
    // builder failure.
    unavailable_unmaterialized_archive_ranges: Vec<ExactUtcRange>,
    // A materialized compact bucket can answer a whole-hour query while an unreadable archive is
    // still required for a partial boundary within that bucket. Keep this narrower condition
    // separate from a durable-coverage gap which makes every overlapping range unavailable.
    unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    // Rolling selections conservatively round an unavailable archive contribution to its hour,
    // but newest-N admission can prove safety against the manifest's exact endpoint. Keep that
    // narrower proof separate so an older archive in the same hour does not reject `current`.
    unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    // Account replay and account-manifest proof can lag the global compact rollup. A missing
    // account manifest may conceal an archive-only account, so the exact gap applies to every
    // account-scoped request without making the independently proven global response fail.
    unavailable_unmaterialized_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    // An oversized persisted-live boundary affects only ranges that intersect that exact hour.
    // The independent `current` prefix remains exact and must stay available.
    unavailable_exact_live_ranges: Vec<ExactUtcRange>,
    // Account rollups can lag independently of their global counterpart. Keep that proof gap
    // scoped so global rolling summaries remain available when their own compact bucket is
    // complete.
    unavailable_exact_live_account_ranges: HashMap<i64, Vec<ExactUtcRange>>,
    // Archive/runtime failures without an ordering proof remain a coarse unavailable state.
    current_source_unavailable: bool,
    // Live source admission preserves the first unproven global/account rank. Requests below
    // that boundary remain exact even when an older candidate could not be admitted.
    current_source_unavailable_from_rank: Option<usize>,
    current_account_source_unavailable_from_rank: HashMap<i64, usize>,
    // Archive manifests are not replayed in the resident current index. Their latest durable
    // coverage end is enough to prove that an individual newest-N prefix is unaffected. Keep
    // the proof separate from a resident-byte failure so `current?limit=1` is not rejected
    // merely because a much larger legal selection would reach an archive boundary.
    current_archive_latest_coverage_end: Option<DateTime<Utc>>,
    current_archive_has_unknown_coverage: bool,
    current_archive_admission_exceeded: bool,
    // Archive newest-N candidates are admitted globally. Per-account newest-N cannot borrow
    // that proof until every completed archive is represented: a quiet account can have its
    // newest row in history which falls outside the global current-admission horizon.
    current_account_source_unavailable: bool,
    // In-progress state is not the same thing as a row whose persisted status happens to be
    // `running`: the typed runtime overlay reconciles terminal replacements and retry lineage.
    // Keep that reconciled view alongside the immutable history projection.
    in_progress_by_account: HashMap<Option<i64>, InProgressSummarySnapshot>,
    maintenance: Option<StatsMaintenanceResponse>,
    // The generation fence is captured from durable source boundaries in the same build cycle.
    // A cadence renewal compares it before extending freshness; a changed source must rebuild.
    generation_fence: SummaryProjectionGenerationFence,
    freshness_lease: SummaryProjectionFreshnessLease,
    refreshed_at: Option<Instant>,
    freshness: SummaryProjectionFreshness,
    revision: u64,
}
