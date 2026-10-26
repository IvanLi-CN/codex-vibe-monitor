//! Analytics authority and bounded cross-backend projection reads.
//!
//! Analytics projections are rebuildable state, but their cursor is a correctness boundary. A
//! source page is read under the traffic authority and released before the analytics transaction
//! starts. The projection writer then commits its rows and the corresponding cursor together in
//! one analytics transaction. This module intentionally does not install the store in `AppState`;
//! runtime routing and cutover remain a later lifecycle concern.

use std::fmt;

use anyhow::{Result, bail};
use sqlx::{FromRow, PgConnection, Postgres, QueryBuilder, Sqlite, SqliteConnection};

use super::core::{
    AnalyticsTransaction, BackendKind, ModuleAuthority, ModuleGenerationGate, StorageModule,
    StoragePools, TrafficArchiveTransaction, TypedStorageTransaction,
};

const SQLITE_ANALYTICS_AUTHORITY_GENERATION: u64 = 1;
const SQLITE_TRAFFIC_AUTHORITY_GENERATION: u64 = 1;
const MAX_TRAFFIC_STATS_PAGE_ROWS: usize = 1_024;
const DEFAULT_TRAFFIC_STATS_PAGE_ROWS: usize = 256;

/// Rebuildable consumers share the analytics authority, but each one keeps an independent
/// checkpoint and data-quality state. Keeping the names here prevents a consumer from silently
/// borrowing another projection's cursor during a backend cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AnalyticsProjection {
    Summary,
    Timeseries,
    LongTerm,
    PromptCache,
    Subscription,
}

impl AnalyticsProjection {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Timeseries => "timeseries",
            Self::LongTerm => "long_term",
            Self::PromptCache => "prompt_cache",
            Self::Subscription => "subscription",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsProjectionOwner {
    pub(crate) projection: AnalyticsProjection,
    pub(crate) target_generation: u64,
}

impl AnalyticsProjectionOwner {
    pub(crate) fn validate(self) -> Result<()> {
        if self.target_generation == 0 {
            bail!(
                "analytics projection {} requires a non-zero target generation",
                self.projection.name()
            );
        }
        Ok(())
    }
}

/// A projection can publish only a complete bounded page. Incomplete or unavailable input keeps
/// the previous payload and records degraded quality for a later retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionDataQuality {
    Fresh,
    Partial,
    Unavailable,
}

impl ProjectionDataQuality {
    pub(crate) const fn can_replace_last_good(self) -> bool {
        matches!(self, Self::Fresh)
    }

    pub(crate) const fn retains_last_good(self) -> bool {
        !self.can_replace_last_good()
    }
}

/// The durable identity of a projection cursor. Source and target generations are both part of
/// the identity so a stale consumer cannot acknowledge a page after either module has cut over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionCursor {
    pub(crate) target_generation: u64,
    pub(crate) source_generation: u64,
    pub(crate) row_id: i64,
    pub(crate) source_watermark: i64,
}

impl ProjectionCursor {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.target_generation == 0 || self.source_generation == 0 {
            bail!("projection cursor generations must be non-zero");
        }
        if self.row_id < 0 || self.source_watermark < 0 {
            bail!("projection cursor values must be non-negative");
        }
        if self.row_id > self.source_watermark {
            bail!("projection cursor cannot pass its source watermark");
        }
        Ok(())
    }
}

/// A bounded source page handed to a projection writer. `has_more` is deliberately separate from
/// quality: a fully read page may have another obligation, but its cursor can still advance to the
/// last row actually consumed. The caller must not substitute `source_watermark` for `next_row_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionBatch {
    pub(crate) source_generation: u64,
    pub(crate) source_watermark: i64,
    pub(crate) next_row_id: i64,
    pub(crate) has_more: bool,
    pub(crate) quality: ProjectionDataQuality,
}

impl ProjectionBatch {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.source_generation == 0 {
            bail!("projection batch source generation must be non-zero");
        }
        if self.next_row_id < 0 || self.source_watermark < 0 {
            bail!("projection batch values must be non-negative");
        }
        if self.next_row_id > self.source_watermark {
            bail!("projection batch cursor cannot pass its source watermark");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionCommitDecision {
    Publish {
        cursor: ProjectionCursor,
        has_more: bool,
    },
    RetainLastGood {
        cursor: ProjectionCursor,
        quality: ProjectionDataQuality,
    },
    Unavailable {
        quality: ProjectionDataQuality,
    },
}

impl ProjectionCommitDecision {
    pub(crate) const fn cursor(self) -> Option<ProjectionCursor> {
        match self {
            Self::Publish { cursor, .. } | Self::RetainLastGood { cursor, .. } => Some(cursor),
            Self::Unavailable { .. } => None,
        }
    }

    pub(crate) const fn advanced(self) -> bool {
        matches!(self, Self::Publish { .. })
    }
}

/// In-memory publication state used by each projection adapter. The durable cursor is updated
/// only after the target transaction commits; a failed batch changes quality but never invents a
/// new cursor. The payload itself remains owned by the existing module-specific read model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionReadState {
    pub(crate) owner: AnalyticsProjectionOwner,
    pub(crate) last_good_cursor: Option<ProjectionCursor>,
    pub(crate) quality: ProjectionDataQuality,
    pub(crate) has_more: bool,
}

impl ProjectionReadState {
    pub(crate) fn new(owner: AnalyticsProjectionOwner) -> Result<Self> {
        owner.validate()?;
        Ok(Self {
            owner,
            last_good_cursor: None,
            quality: ProjectionDataQuality::Unavailable,
            has_more: false,
        })
    }

    pub(crate) fn plan(&self, batch: ProjectionBatch) -> Result<ProjectionCommitDecision> {
        plan_projection_commit(self.owner.target_generation, self.last_good_cursor, batch)
    }

    pub(crate) fn apply_commit(&mut self, decision: ProjectionCommitDecision) -> Result<()> {
        let cursor = decision.cursor();
        if let Some(cursor) = cursor {
            cursor.validate()?;
            if cursor.target_generation != self.owner.target_generation {
                bail!(
                    "projection {} commit target generation does not match its owner",
                    self.owner.projection.name()
                );
            }
        }
        match decision {
            ProjectionCommitDecision::Publish { cursor, has_more } => {
                self.last_good_cursor = Some(cursor);
                self.quality = ProjectionDataQuality::Fresh;
                self.has_more = has_more;
            }
            ProjectionCommitDecision::RetainLastGood { quality, .. } => {
                self.quality = quality;
                self.has_more = true;
            }
            ProjectionCommitDecision::Unavailable { quality } => {
                self.quality = quality;
                self.has_more = true;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionCommitReceipt {
    pub(crate) cursor: ProjectionCursor,
    pub(crate) advanced: bool,
    pub(crate) has_more: bool,
}

/// Validate the source page against the last committed cursor before opening a target commit.
///
/// A source generation change is an explicit recovery boundary. It must not be handled by
/// silently resetting a cursor, because doing so could acknowledge rows from the old source
/// generation. The recovery owner must first install a new baseline and call this with `None`.
pub(crate) fn plan_projection_commit(
    target_generation: u64,
    previous: Option<ProjectionCursor>,
    batch: ProjectionBatch,
) -> Result<ProjectionCommitDecision> {
    if target_generation == 0 {
        bail!("projection target generation must be non-zero");
    }
    batch.validate()?;
    if let Some(previous) = previous {
        previous.validate()?;
        if previous.target_generation != target_generation {
            bail!("projection target generation changed without cursor recovery");
        }
        if previous.source_generation != batch.source_generation {
            bail!("projection source generation changed without cursor recovery");
        }
        if batch.source_watermark < previous.source_watermark {
            bail!("projection source watermark regressed");
        }
        if batch.next_row_id < previous.row_id {
            bail!("projection cursor regressed");
        }
    }

    if batch.quality.can_replace_last_good() {
        return Ok(ProjectionCommitDecision::Publish {
            cursor: ProjectionCursor {
                target_generation,
                source_generation: batch.source_generation,
                row_id: batch.next_row_id,
                source_watermark: batch.source_watermark,
            },
            has_more: batch.has_more,
        });
    }

    match previous {
        Some(cursor) => Ok(ProjectionCommitDecision::RetainLastGood {
            cursor,
            quality: batch.quality,
        }),
        None => Ok(ProjectionCommitDecision::Unavailable {
            quality: batch.quality,
        }),
    }
}

/// A small source page for hot reads and background projection consumers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrafficStatsPage {
    pub(crate) source_backend: BackendKind,
    pub(crate) source_generation: u64,
    pub(crate) source_watermark: i64,
    pub(crate) read_through_id: i64,
    pub(crate) after_row_id: i64,
    pub(crate) next_row_id: i64,
    pub(crate) row_count: usize,
    pub(crate) has_more: bool,
    pub(crate) aggregate: crate::StatsTotals,
}

impl TrafficStatsPage {
    pub(crate) fn projection_batch(self) -> ProjectionBatch {
        ProjectionBatch {
            source_generation: self.source_generation,
            source_watermark: self.source_watermark,
            next_row_id: self.next_row_id,
            has_more: self.has_more,
            quality: ProjectionDataQuality::Fresh,
        }
    }

    pub(crate) const fn quality_for_complete_read(self) -> ProjectionDataQuality {
        if self.has_more || self.read_through_id < self.source_watermark {
            ProjectionDataQuality::Partial
        } else {
            ProjectionDataQuality::Fresh
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrafficStatsReadRequest {
    pub(crate) after_row_id: i64,
    pub(crate) upper_bound_id: Option<i64>,
    pub(crate) limit: usize,
    pub(crate) source_scope: crate::InvocationSourceScope,
}

impl Default for TrafficStatsReadRequest {
    fn default() -> Self {
        Self {
            after_row_id: 0,
            upper_bound_id: None,
            limit: DEFAULT_TRAFFIC_STATS_PAGE_ROWS,
            source_scope: crate::InvocationSourceScope::All,
        }
    }
}

impl TrafficStatsReadRequest {
    fn validate(&self) -> Result<()> {
        if self.after_row_id < 0 {
            bail!("traffic stats page cursor must be non-negative");
        }
        if self
            .upper_bound_id
            .is_some_and(|value| value < self.after_row_id)
        {
            bail!("traffic stats page upper bound cannot precede its cursor");
        }
        if !(1..=MAX_TRAFFIC_STATS_PAGE_ROWS).contains(&self.limit) {
            bail!("traffic stats page limit must be between 1 and 1024");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalyticsRollupRead {
    pub(crate) target_generation: u64,
    pub(crate) aggregate: crate::StatsTotals,
    pub(crate) bucket_count: i64,
    pub(crate) quality: ProjectionDataQuality,
}

/// A bounded aggregate read directly from the authoritative traffic facts. This is used for the
/// live tail while the analytics projections are still on SQLite; it is intentionally not an
/// unbounded replacement for the historical rollup/read-model path.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrafficStatsAggregateRead {
    pub(crate) source_backend: BackendKind,
    pub(crate) source_generation: u64,
    pub(crate) source_watermark: i64,
    pub(crate) read_through_id: i64,
    pub(crate) aggregate: crate::StatsTotals,
    pub(crate) quality: ProjectionDataQuality,
}

#[derive(Debug, Clone)]
pub(crate) struct TrafficStatsAggregateRequest {
    pub(crate) filter: crate::stats::StatsFilter,
    pub(crate) source_scope: crate::InvocationSourceScope,
    /// The caller's admitted source fence. Reads must never select beyond this value.
    pub(crate) upper_bound_id: i64,
}

impl TrafficStatsAggregateRequest {
    fn validate(&self) -> Result<()> {
        if self.upper_bound_id < 0 {
            bail!("traffic stats aggregate upper bound must be non-negative");
        }
        if let crate::stats::StatsFilter::RecentLimit(limit) = self.filter
            && limit <= 0
        {
            bail!("traffic stats recent limit must be positive");
        }
        Ok(())
    }
}

/// Analytics authority adapter. It is deliberately detached from runtime wiring so a migration
/// worker can construct it with the shared pools and both module gates before the global router is
/// changed by the lifecycle owner.
pub(crate) struct AnalyticsProjectionStore {
    pools: StoragePools,
    analytics_gate: ModuleGenerationGate,
    traffic_gate: ModuleGenerationGate,
}

impl fmt::Debug for AnalyticsProjectionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnalyticsProjectionStore")
            .field("analytics_gate", &self.analytics_gate)
            .field("traffic_gate", &self.traffic_gate)
            .finish_non_exhaustive()
    }
}

impl AnalyticsProjectionStore {
    pub(crate) fn with_authority(
        pools: StoragePools,
        analytics_gate: ModuleGenerationGate,
        traffic_gate: ModuleGenerationGate,
    ) -> Self {
        Self {
            pools,
            analytics_gate,
            traffic_gate,
        }
    }

    pub(crate) fn sqlite(pool: sqlx::Pool<Sqlite>) -> Self {
        Self::with_authority(
            StoragePools::sqlite_only(pool),
            ModuleGenerationGate::new(
                StorageModule::Analytics,
                ModuleAuthority {
                    backend: BackendKind::Sqlite,
                    generation: SQLITE_ANALYTICS_AUTHORITY_GENERATION,
                },
            ),
            ModuleGenerationGate::new(
                StorageModule::TrafficArchive,
                ModuleAuthority {
                    backend: BackendKind::Sqlite,
                    generation: SQLITE_TRAFFIC_AUTHORITY_GENERATION,
                },
            ),
        )
    }

    pub(crate) async fn projection_owner(
        &self,
        projection: AnalyticsProjection,
    ) -> Result<AnalyticsProjectionOwner> {
        let owner = AnalyticsProjectionOwner {
            projection,
            target_generation: self.analytics_gate.current_authority().await.generation,
        };
        owner.validate()?;
        Ok(owner)
    }

    pub(crate) async fn begin(&self) -> Result<AnalyticsProjectionTransaction> {
        let lease = self
            .analytics_gate
            .acquire(StorageModule::Analytics)
            .await?;
        let target_generation = lease.lease.authority.generation;
        let backend = lease.lease.authority.backend;
        let transaction = self
            .pools
            .begin_typed::<AnalyticsTransaction>(lease)
            .await?;
        Ok(AnalyticsProjectionTransaction {
            transaction,
            backend,
            target_generation,
            projection: None,
        })
    }

    pub(crate) async fn begin_projection(
        &self,
        projection: AnalyticsProjection,
    ) -> Result<AnalyticsProjectionTransaction> {
        let mut transaction = self.begin().await?;
        transaction.projection = Some(projection);
        Ok(transaction)
    }

    pub(crate) async fn query_rollup_totals(
        &self,
        start_epoch: i64,
        end_epoch: i64,
        source_scope: crate::InvocationSourceScope,
    ) -> Result<AnalyticsRollupRead> {
        if start_epoch >= end_epoch {
            return Ok(AnalyticsRollupRead {
                target_generation: self.analytics_gate.current_authority().await.generation,
                aggregate: crate::StatsTotals::default(),
                bucket_count: 0,
                quality: ProjectionDataQuality::Fresh,
            });
        }
        let lease = self
            .analytics_gate
            .acquire(StorageModule::Analytics)
            .await?;
        let target_generation = lease.lease.authority.generation;
        let backend = lease.lease.authority.backend;
        let mut transaction = self
            .pools
            .begin_typed::<AnalyticsTransaction>(lease)
            .await?;
        let result = match backend {
            BackendKind::Sqlite => {
                query_rollup_totals_sqlite(
                    transaction.sqlite_transaction()?.as_mut(),
                    start_epoch,
                    end_epoch,
                    source_scope,
                )
                .await
            }
            BackendKind::Postgres => {
                query_rollup_totals_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                    start_epoch,
                    end_epoch,
                    source_scope,
                )
                .await
            }
        };
        let rollback_result = transaction.rollback().await;
        finish_authority_read(result, rollback_result).map(|(aggregate, bucket_count, quality)| {
            AnalyticsRollupRead {
                target_generation,
                aggregate,
                bucket_count,
                quality,
            }
        })
    }

    /// Read one bounded source page without holding a target transaction. This is the explicit
    /// cross-database seam used while traffic is on PostgreSQL and analytics remains on SQLite.
    pub(crate) async fn read_bounded_traffic_stats(
        &self,
        request: TrafficStatsReadRequest,
    ) -> Result<TrafficStatsPage> {
        request.validate()?;
        let lease = self
            .traffic_gate
            .acquire(StorageModule::TrafficArchive)
            .await?;
        let source_generation = lease.lease.authority.generation;
        let source_backend = lease.lease.authority.backend;
        let mut transaction = self
            .pools
            .begin_typed::<TrafficArchiveTransaction>(lease)
            .await?;
        let result = match source_backend {
            BackendKind::Sqlite => {
                read_traffic_stats_page_sqlite(transaction.sqlite_transaction()?.as_mut(), request)
                    .await
            }
            BackendKind::Postgres => {
                read_traffic_stats_page_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                    request,
                )
                .await
            }
        };
        let rollback_result = transaction.rollback().await;
        finish_authority_read(result, rollback_result).map(|mut page| {
            page.source_backend = source_backend;
            page.source_generation = source_generation;
            page
        })
    }

    /// Aggregate a bounded traffic-fact selection through the current traffic authority. The
    /// source watermark and the aggregate are captured by one source transaction, so callers can
    /// distinguish a complete fence from a read which intentionally stopped before the source
    /// head. The returned result is read-only and never opens an analytics write transaction.
    pub(crate) async fn query_bounded_traffic_stats(
        &self,
        request: TrafficStatsAggregateRequest,
    ) -> Result<TrafficStatsAggregateRead> {
        request.validate()?;
        let lease = self
            .traffic_gate
            .acquire(StorageModule::TrafficArchive)
            .await?;
        let source_generation = lease.lease.authority.generation;
        let source_backend = lease.lease.authority.backend;
        let mut transaction = self
            .pools
            .begin_typed::<TrafficArchiveTransaction>(lease)
            .await?;
        let result = match source_backend {
            BackendKind::Sqlite => {
                query_bounded_traffic_stats_sqlite(
                    transaction.sqlite_transaction()?.as_mut(),
                    &request,
                )
                .await
            }
            BackendKind::Postgres => {
                query_bounded_traffic_stats_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                    &request,
                )
                .await
            }
        };
        let rollback_result = transaction.rollback().await;
        finish_authority_read(result, rollback_result).map(|(source_watermark, aggregate)| {
            let read_through_id = request.upper_bound_id.min(source_watermark);
            let quality = if read_through_id == source_watermark {
                ProjectionDataQuality::Fresh
            } else {
                ProjectionDataQuality::Partial
            };
            TrafficStatsAggregateRead {
                source_backend,
                source_generation,
                source_watermark,
                read_through_id,
                aggregate,
                quality,
            }
        })
    }
}

pub(crate) struct AnalyticsProjectionTransaction {
    transaction: TypedStorageTransaction<AnalyticsTransaction>,
    backend: BackendKind,
    target_generation: u64,
    projection: Option<AnalyticsProjection>,
}

impl AnalyticsProjectionTransaction {
    pub(crate) fn backend_kind(&self) -> BackendKind {
        self.backend
    }

    pub(crate) fn target_generation(&self) -> u64 {
        self.target_generation
    }

    pub(crate) fn projection(&self) -> Option<AnalyticsProjection> {
        self.projection
    }

    pub(crate) fn sqlite_transaction(&mut self) -> Result<&mut sqlx::Transaction<'static, Sqlite>> {
        self.transaction.sqlite_transaction()
    }

    pub(crate) fn sqlite_connection(&mut self) -> Result<&mut SqliteConnection> {
        Ok(self.transaction.sqlite_transaction()?.as_mut())
    }

    pub(crate) fn postgres_transaction(
        &mut self,
    ) -> Result<&mut sqlx::Transaction<'static, Postgres>> {
        self.transaction.postgres_transaction()
    }

    pub(crate) fn postgres_connection(&mut self) -> Result<&mut PgConnection> {
        Ok(self.transaction.postgres_transaction()?.as_mut())
    }

    pub(crate) fn plan_projection_commit(
        &self,
        previous: Option<ProjectionCursor>,
        batch: ProjectionBatch,
    ) -> Result<ProjectionCommitDecision> {
        plan_projection_commit(self.target_generation, previous, batch)
    }

    pub(crate) async fn commit(self) -> Result<()> {
        self.transaction.commit().await
    }

    /// Start an atomic projection publication. The returned guard refuses to commit until the
    /// caller has marked the durable cursor/quality mutation as written through the same target
    /// transaction. `Unavailable` is rejected so a missing baseline cannot be published.
    pub(crate) fn prepare_projection_commit(
        self,
        decision: ProjectionCommitDecision,
    ) -> Result<AnalyticsProjectionCommit> {
        let Some(projection) = self.projection else {
            bail!("analytics projection commit requires a named projection owner");
        };
        let Some(cursor) = decision.cursor() else {
            bail!("cannot prepare an unavailable projection without a last-good baseline");
        };
        if cursor.target_generation != self.target_generation {
            bail!("projection commit target generation does not match its transaction");
        }
        Ok(AnalyticsProjectionCommit {
            transaction: self,
            projection,
            decision,
            cursor_persisted: false,
        })
    }

    pub(crate) async fn rollback(self) -> Result<()> {
        self.transaction.rollback().await
    }
}

pub(crate) struct AnalyticsProjectionCommit {
    transaction: AnalyticsProjectionTransaction,
    projection: AnalyticsProjection,
    decision: ProjectionCommitDecision,
    cursor_persisted: bool,
}

impl AnalyticsProjectionCommit {
    pub(crate) fn transaction_mut(&mut self) -> &mut AnalyticsProjectionTransaction {
        &mut self.transaction
    }

    /// Mark the cursor and quality marker after the caller has written them through
    /// `transaction_mut`. Keeping this acknowledgement explicit prevents a projection row from
    /// being committed without its source obligation, while still allowing each projection to
    /// keep its existing schema-specific checkpoint table.
    pub(crate) fn mark_cursor_persisted(&mut self) {
        self.cursor_persisted = true;
    }

    pub(crate) async fn commit(self) -> Result<ProjectionCommitReceipt> {
        if !self.cursor_persisted {
            bail!(
                "analytics projection commit requires its cursor and data-quality marker to be persisted"
            );
        }
        let cursor = self
            .decision
            .cursor()
            .expect("projection commit was validated before creating the guard");
        let _projection = self.projection;
        self.transaction.transaction.commit().await?;
        Ok(ProjectionCommitReceipt {
            cursor,
            advanced: self.decision.advanced(),
            has_more: matches!(
                self.decision,
                ProjectionCommitDecision::Publish { has_more: true, .. }
            ),
        })
    }

    pub(crate) async fn rollback(self) -> Result<()> {
        self.transaction.rollback().await
    }
}

#[derive(Debug, FromRow)]
struct AnalyticsRollupAggregateRow {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_cost: f64,
    total_tokens: i64,
    non_success_cost: f64,
    bucket_count: i64,
    incomplete_bucket_count: i64,
}

#[derive(Debug, FromRow)]
struct TrafficStatsFactRow {
    id: i64,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
}

#[derive(Debug, FromRow)]
struct TrafficStatsAggregateRow {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_cost: f64,
    total_tokens: i64,
    non_success_cost: f64,
}

const TRAFFIC_STATS_AGGREGATE_SELECT: &str = "COUNT(*) AS total_count, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') AND COALESCE(NULLIF(TRIM(failure_class), ''), NULLIF(TRIM(failure_kind), ''), 'none') = 'none' THEN 1 ELSE 0 END), 0) AS success_count, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND COALESCE(NULLIF(TRIM(failure_class), ''), NULLIF(TRIM(failure_kind), ''), 'none') IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, COALESCE(SUM(cost), 0.0) AS total_cost, COALESCE(SUM(total_tokens), 0) AS total_tokens, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND COALESCE(NULLIF(TRIM(failure_class), ''), NULLIF(TRIM(failure_kind), ''), 'none') IN ('service_failure', 'client_failure', 'client_abort') THEN COALESCE(cost, 0.0) ELSE 0.0 END), 0.0) AS non_success_cost";

async fn query_bounded_traffic_stats_sqlite(
    connection: &mut SqliteConnection,
    request: &TrafficStatsAggregateRequest,
) -> Result<(i64, crate::StatsTotals)> {
    let source_watermark =
        traffic_source_watermark_sqlite(connection, request.source_scope).await?;
    let upper_bound_id = request.upper_bound_id.min(source_watermark);
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query.push(TRAFFIC_STATS_AGGREGATE_SELECT).push(" FROM ");
    append_traffic_stats_source(&mut query, request, upper_bound_id);
    let row = query
        .build_query_as::<TrafficStatsAggregateRow>()
        .fetch_one(&mut *connection)
        .await?;
    Ok((source_watermark, row.into_totals()))
}

async fn query_bounded_traffic_stats_postgres(
    connection: &mut PgConnection,
    request: &TrafficStatsAggregateRequest,
) -> Result<(i64, crate::StatsTotals)> {
    let source_watermark =
        traffic_source_watermark_postgres(connection, request.source_scope).await?;
    let upper_bound_id = request.upper_bound_id.min(source_watermark);
    let mut query = QueryBuilder::<Postgres>::new("SELECT ");
    query.push(TRAFFIC_STATS_AGGREGATE_SELECT).push(" FROM ");
    append_traffic_stats_source(&mut query, request, upper_bound_id);
    let row = query
        .build_query_as::<TrafficStatsAggregateRow>()
        .fetch_one(&mut *connection)
        .await?;
    Ok((source_watermark, row.into_totals()))
}

impl TrafficStatsAggregateRow {
    fn into_totals(self) -> crate::StatsTotals {
        crate::StatsTotals {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            non_success_cost: self.non_success_cost,
        }
    }
}

fn append_traffic_stats_source<DB>(
    query: &mut QueryBuilder<'_, DB>,
    request: &TrafficStatsAggregateRequest,
    upper_bound_id: i64,
) where
    DB: sqlx::Database,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    match &request.filter {
        crate::stats::StatsFilter::RecentLimit(limit) => {
            query.push("(SELECT * FROM codex_invocations WHERE id <= ");
            query.push_bind(upper_bound_id).push(" AND ");
            append_traffic_source_scope(query, request.source_scope);
            query
                .push(" ORDER BY occurred_at DESC, id DESC LIMIT ")
                .push_bind(*limit)
                .push(") AS recent");
        }
        crate::stats::StatsFilter::All => {
            query
                .push("codex_invocations WHERE id <= ")
                .push_bind(upper_bound_id);
            append_traffic_source_scope_predicate(query, request.source_scope);
        }
        crate::stats::StatsFilter::Since(start) => {
            query
                .push("codex_invocations WHERE id <= ")
                .push_bind(upper_bound_id)
                .push(" AND occurred_at >= ")
                .push_bind(crate::stats::db_occurred_at_lower_bound(*start));
            append_traffic_source_scope_predicate(query, request.source_scope);
        }
        crate::stats::StatsFilter::Range(start, end) => {
            query
                .push("codex_invocations WHERE id <= ")
                .push_bind(upper_bound_id)
                .push(" AND occurred_at >= ")
                .push_bind(crate::stats::db_occurred_at_lower_bound(*start))
                .push(" AND occurred_at < ")
                .push_bind(crate::stats::db_occurred_at_lower_bound(*end));
            append_traffic_source_scope_predicate(query, request.source_scope);
        }
    }
}

fn append_traffic_source_scope<DB>(
    query: &mut QueryBuilder<'_, DB>,
    source_scope: crate::InvocationSourceScope,
) where
    DB: sqlx::Database,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query.push("source = ").push_bind(crate::SOURCE_PROXY);
    } else {
        query.push("1 = 1");
    }
}

fn append_traffic_source_scope_predicate<DB>(
    query: &mut QueryBuilder<'_, DB>,
    source_scope: crate::InvocationSourceScope,
) where
    DB: sqlx::Database,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(crate::SOURCE_PROXY);
    }
}

async fn query_rollup_totals_sqlite(
    connection: &mut SqliteConnection,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: crate::InvocationSourceScope,
) -> Result<(crate::StatsTotals, i64, ProjectionDataQuality)> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(SUM(total_count), 0) AS total_count, COALESCE(SUM(success_count), 0) AS success_count, COALESCE(SUM(failure_count), 0) AS failure_count, COALESCE(SUM(total_cost), 0.0) AS total_cost, COALESCE(SUM(total_tokens), 0) AS total_tokens, COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost, COUNT(*) AS bucket_count, COALESCE(SUM(CASE WHEN COALESCE(terminal_proof_complete, 0) = 1 THEN 0 ELSE 1 END), 0) AS incomplete_bucket_count FROM invocation_rollup_hourly WHERE bucket_start_epoch >= ",
    );
    query
        .push_bind(start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(end_epoch);
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(crate::SOURCE_PROXY);
    }
    let row = query
        .build_query_as::<AnalyticsRollupAggregateRow>()
        .fetch_one(&mut *connection)
        .await?;
    Ok(rollup_read_from_row(row))
}

async fn query_rollup_totals_postgres(
    connection: &mut PgConnection,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: crate::InvocationSourceScope,
) -> Result<(crate::StatsTotals, i64, ProjectionDataQuality)> {
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT COALESCE(SUM(total_count), 0) AS total_count, COALESCE(SUM(success_count), 0) AS success_count, COALESCE(SUM(failure_count), 0) AS failure_count, COALESCE(SUM(total_cost), 0.0) AS total_cost, COALESCE(SUM(total_tokens), 0) AS total_tokens, COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost, COUNT(*) AS bucket_count, COALESCE(SUM(CASE WHEN COALESCE(terminal_proof_complete, 0) = 1 THEN 0 ELSE 1 END), 0) AS incomplete_bucket_count FROM invocation_rollup_hourly WHERE bucket_start_epoch >= ",
    );
    query
        .push_bind(start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(end_epoch);
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(crate::SOURCE_PROXY);
    }
    let row = query
        .build_query_as::<AnalyticsRollupAggregateRow>()
        .fetch_one(&mut *connection)
        .await?;
    Ok(rollup_read_from_row(row))
}

fn rollup_read_from_row(
    row: AnalyticsRollupAggregateRow,
) -> (crate::StatsTotals, i64, ProjectionDataQuality) {
    let quality = if row.incomplete_bucket_count > 0 {
        ProjectionDataQuality::Partial
    } else {
        ProjectionDataQuality::Fresh
    };
    (
        crate::StatsTotals {
            total_count: row.total_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            total_cost: row.total_cost,
            total_tokens: row.total_tokens,
            non_success_cost: row.non_success_cost,
        },
        row.bucket_count,
        quality,
    )
}

async fn read_traffic_stats_page_sqlite(
    connection: &mut SqliteConnection,
    request: TrafficStatsReadRequest,
) -> Result<TrafficStatsPage> {
    let source_watermark =
        traffic_source_watermark_sqlite(connection, request.source_scope).await?;
    let upper_bound_id = request
        .upper_bound_id
        .unwrap_or(source_watermark)
        .min(source_watermark);
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, status, error_message, failure_kind, failure_class, total_tokens, cost FROM codex_invocations WHERE id > ",
    );
    query
        .push_bind(request.after_row_id)
        .push(" AND id <= ")
        .push_bind(upper_bound_id);
    append_source_scope_predicate(&mut query, request.source_scope);
    query
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind((request.limit + 1) as i64);
    let rows = query
        .build_query_as::<TrafficStatsFactRow>()
        .fetch_all(&mut *connection)
        .await?;
    Ok(traffic_page_from_rows(
        rows,
        request,
        upper_bound_id,
        BackendKind::Sqlite,
        SQLITE_TRAFFIC_AUTHORITY_GENERATION,
    ))
}

async fn read_traffic_stats_page_postgres(
    connection: &mut PgConnection,
    request: TrafficStatsReadRequest,
) -> Result<TrafficStatsPage> {
    let source_watermark =
        traffic_source_watermark_postgres(connection, request.source_scope).await?;
    let upper_bound_id = request
        .upper_bound_id
        .unwrap_or(source_watermark)
        .min(source_watermark);
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT id, status, error_message, failure_kind, failure_class, total_tokens, cost FROM codex_invocations WHERE id > ",
    );
    query
        .push_bind(request.after_row_id)
        .push(" AND id <= ")
        .push_bind(upper_bound_id);
    append_source_scope_predicate(&mut query, request.source_scope);
    query
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind((request.limit + 1) as i64);
    let rows = query
        .build_query_as::<TrafficStatsFactRow>()
        .fetch_all(&mut *connection)
        .await?;
    Ok(traffic_page_from_rows(
        rows,
        request,
        upper_bound_id,
        BackendKind::Postgres,
        0,
    ))
}

async fn traffic_source_watermark_sqlite(
    connection: &mut SqliteConnection,
    source_scope: crate::InvocationSourceScope,
) -> Result<i64> {
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COALESCE(MAX(id), 0) FROM codex_invocations");
    append_source_scope_where(&mut query, source_scope);
    Ok(query
        .build_query_scalar()
        .fetch_one(&mut *connection)
        .await?)
}

async fn traffic_source_watermark_postgres(
    connection: &mut PgConnection,
    source_scope: crate::InvocationSourceScope,
) -> Result<i64> {
    let mut query =
        QueryBuilder::<Postgres>::new("SELECT COALESCE(MAX(id), 0) FROM codex_invocations");
    append_source_scope_where(&mut query, source_scope);
    Ok(query
        .build_query_scalar()
        .fetch_one(&mut *connection)
        .await?)
}

fn append_source_scope_predicate<DB>(
    query: &mut QueryBuilder<'_, DB>,
    source_scope: crate::InvocationSourceScope,
) where
    DB: sqlx::Database,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(crate::SOURCE_PROXY);
    }
}

fn append_source_scope_where<DB>(
    query: &mut QueryBuilder<'_, DB>,
    source_scope: crate::InvocationSourceScope,
) where
    DB: sqlx::Database,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    if source_scope == crate::InvocationSourceScope::ProxyOnly {
        query
            .push(" WHERE source = ")
            .push_bind(crate::SOURCE_PROXY);
    }
}

fn traffic_page_from_rows(
    mut rows: Vec<TrafficStatsFactRow>,
    request: TrafficStatsReadRequest,
    source_watermark: i64,
    source_backend: BackendKind,
    source_generation: u64,
) -> TrafficStatsPage {
    let has_more = rows.len() > request.limit
        || request
            .upper_bound_id
            .is_some_and(|upper_bound_id| upper_bound_id < source_watermark);
    rows.truncate(request.limit);
    let next_row_id = rows.last().map_or(request.after_row_id, |row| row.id);
    TrafficStatsPage {
        source_backend,
        source_generation,
        source_watermark,
        read_through_id: source_watermark.min(request.upper_bound_id.unwrap_or(source_watermark)),
        after_row_id: request.after_row_id,
        next_row_id,
        row_count: rows.len(),
        has_more,
        aggregate: aggregate_traffic_rows(&rows),
    }
}

fn aggregate_traffic_rows(rows: &[TrafficStatsFactRow]) -> crate::StatsTotals {
    let mut totals = crate::StatsTotals::default();
    for row in rows {
        totals.total_count = totals.total_count.saturating_add(1);
        totals.total_tokens = totals
            .total_tokens
            .saturating_add(row.total_tokens.unwrap_or_default());
        totals.total_cost += row.cost.unwrap_or_default();
        if traffic_row_is_failure(row) {
            totals.failure_count = totals.failure_count.saturating_add(1);
            totals.non_success_cost += row.cost.unwrap_or_default();
        } else if traffic_row_is_success(row) {
            totals.success_count = totals.success_count.saturating_add(1);
        }
    }
    totals
}

fn traffic_row_is_success(row: &TrafficStatsFactRow) -> bool {
    let status = row
        .status
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let error_empty = row
        .error_message
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty();
    !traffic_row_is_failure(row)
        && (matches!(status.as_str(), "success" | "completed" | "warning_success")
            || (status == "http_200" && error_empty))
}

fn traffic_row_is_failure(row: &TrafficStatsFactRow) -> bool {
    let class = row
        .failure_class
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(row.failure_kind.as_deref())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        class.as_str(),
        "service_failure" | "client_failure" | "client_abort"
    )
}

fn finish_authority_read<T>(result: Result<T>, rollback_result: Result<()>) -> Result<T> {
    match (result, rollback_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(rollback_error)) => Err(anyhow::anyhow!(
            "analytics authority read rollback failed: {rollback_error:#}"
        )),
        (Err(error), Err(rollback_error)) => Err(anyhow::anyhow!(
            "analytics authority read failed: {error:#}; rollback failed: {rollback_error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

    fn batch(quality: ProjectionDataQuality, next_row_id: i64, has_more: bool) -> ProjectionBatch {
        ProjectionBatch {
            source_generation: 7,
            source_watermark: 10,
            next_row_id,
            has_more,
            quality,
        }
    }

    #[test]
    fn partial_page_retains_last_good_cursor() {
        let previous = ProjectionCursor {
            target_generation: 3,
            source_generation: 7,
            row_id: 4,
            source_watermark: 8,
        };
        let decision = plan_projection_commit(
            3,
            Some(previous),
            batch(ProjectionDataQuality::Partial, 6, true),
        )
        .unwrap();
        assert_eq!(decision.cursor(), Some(previous));
        assert!(!decision.advanced());
        assert!(matches!(
            decision,
            ProjectionCommitDecision::RetainLastGood {
                quality: ProjectionDataQuality::Partial,
                ..
            }
        ));
    }

    #[test]
    fn fresh_page_advances_only_to_the_consumed_row() {
        let previous = ProjectionCursor {
            target_generation: 3,
            source_generation: 7,
            row_id: 4,
            source_watermark: 8,
        };
        let decision = plan_projection_commit(
            3,
            Some(previous),
            batch(ProjectionDataQuality::Fresh, 6, true),
        )
        .unwrap();
        assert_eq!(
            decision.cursor().unwrap().row_id,
            6,
            "a page cannot acknowledge its whole source watermark"
        );
        assert!(decision.advanced());
    }

    #[test]
    fn generation_change_requires_explicit_recovery() {
        let previous = ProjectionCursor {
            target_generation: 3,
            source_generation: 7,
            row_id: 4,
            source_watermark: 8,
        };
        let error = plan_projection_commit(
            3,
            Some(previous),
            ProjectionBatch {
                source_generation: 8,
                ..batch(ProjectionDataQuality::Fresh, 5, false)
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("source generation"));
    }

    #[test]
    fn projection_state_keeps_last_good_cursor_when_quality_degrades() {
        let owner = AnalyticsProjectionOwner {
            projection: AnalyticsProjection::LongTerm,
            target_generation: 3,
        };
        let mut state = ProjectionReadState::new(owner).unwrap();
        let fresh = state
            .plan(batch(ProjectionDataQuality::Fresh, 4, true))
            .unwrap();
        state.apply_commit(fresh).unwrap();
        let cursor = state.last_good_cursor;

        let partial = state
            .plan(batch(ProjectionDataQuality::Partial, 6, true))
            .unwrap();
        state.apply_commit(partial).unwrap();
        assert_eq!(state.last_good_cursor, cursor);
        assert_eq!(state.quality, ProjectionDataQuality::Partial);
        assert!(state.has_more);
    }

    async fn sqlite_store() -> Result<(AnalyticsProjectionStore, SqlitePool)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, occurred_at TEXT, status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT, total_tokens INTEGER, cost REAL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE invocation_rollup_hourly (bucket_start_epoch INTEGER, source TEXT, total_count INTEGER, success_count INTEGER, failure_count INTEGER, total_cost REAL, total_tokens INTEGER, non_success_cost REAL, terminal_proof_complete INTEGER)",
        )
        .execute(&pool)
        .await?;
        Ok((AnalyticsProjectionStore::sqlite(pool.clone()), pool))
    }

    #[tokio::test]
    async fn bounded_sqlite_source_page_reports_remaining_obligation() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        for (id, status, class) in [(1, "completed", "none"), (2, "failed", "service_failure")] {
            sqlx::query("INSERT INTO codex_invocations (id, source, status, error_message, failure_kind, failure_class, total_tokens, cost) VALUES (?1, 'proxy', ?2, '', NULL, ?3, 10, 0.5)")
                .bind(id)
                .bind(status)
                .bind(class)
                .execute(&pool)
                .await?;
        }
        let page = store
            .read_bounded_traffic_stats(TrafficStatsReadRequest {
                limit: 1,
                ..TrafficStatsReadRequest::default()
            })
            .await?;
        assert_eq!(page.row_count, 1);
        assert_eq!(page.next_row_id, 1);
        assert_eq!(page.source_watermark, 2);
        assert!(page.has_more);
        assert_eq!(page.aggregate.total_count, 1);
        assert_eq!(
            page.quality_for_complete_read(),
            ProjectionDataQuality::Partial
        );
        Ok(())
    }

    #[tokio::test]
    async fn bounded_sqlite_aggregate_respects_source_fence() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        for (id, occurred_at, status, class) in [
            (1, "2026-10-26 10:00:00", "completed", "none"),
            (2, "2026-10-26 10:01:00", "failed", "service_failure"),
        ] {
            sqlx::query("INSERT INTO codex_invocations (id, source, occurred_at, status, error_message, failure_kind, failure_class, total_tokens, cost) VALUES (?1, 'proxy', ?2, ?3, '', NULL, ?4, 10, 0.5)")
                .bind(id)
                .bind(occurred_at)
                .bind(status)
                .bind(class)
                .execute(&pool)
                .await?;
        }
        let read = store
            .query_bounded_traffic_stats(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::All,
                upper_bound_id: 1,
            })
            .await?;
        assert_eq!(read.source_watermark, 2);
        assert_eq!(read.read_through_id, 1);
        assert_eq!(read.aggregate.total_count, 1);
        assert_eq!(read.aggregate.success_count, 1);
        assert_eq!(read.quality, ProjectionDataQuality::Partial);
        Ok(())
    }

    #[tokio::test]
    async fn sqlite_rollup_read_marks_unproven_buckets_partial() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly VALUES (100, 'proxy', 2, 1, 1, 1.5, 20, 0.5, 0)",
        )
        .execute(&pool)
        .await?;
        let read = store
            .query_rollup_totals(0, 200, crate::InvocationSourceScope::ProxyOnly)
            .await?;
        assert_eq!(read.aggregate.total_count, 2);
        assert_eq!(read.quality, ProjectionDataQuality::Partial);
        Ok(())
    }

    #[tokio::test]
    async fn projection_commit_publishes_rows_and_cursor_ack_together() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        sqlx::query("CREATE TABLE projection_rows (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await?;
        let transaction = store.begin_projection(AnalyticsProjection::Summary).await?;
        let decision = transaction
            .plan_projection_commit(None, batch(ProjectionDataQuality::Fresh, 1, false))?;
        let mut commit = transaction.prepare_projection_commit(decision)?;
        sqlx::query("INSERT INTO projection_rows (id) VALUES (1)")
            .execute(commit.transaction_mut().sqlite_connection()?)
            .await?;
        commit.mark_cursor_persisted();
        let receipt = commit.commit().await?;
        assert!(receipt.advanced);
        assert_eq!(receipt.cursor.row_id, 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projection_rows")
                .fetch_one(&pool)
                .await?,
            1
        );
        Ok(())
    }
}
