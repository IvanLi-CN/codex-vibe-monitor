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
const ANALYTICS_PROJECTION_CHECKPOINTS_TABLE: &str = "analytics_projection_checkpoints";
const SQLITE_ANALYTICS_PROJECTION_CHECKPOINTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS analytics_projection_checkpoints (
    projection TEXT PRIMARY KEY,
    target_generation INTEGER NOT NULL CHECK (target_generation > 0),
    last_good_source_generation INTEGER,
    last_good_row_id INTEGER,
    last_good_source_watermark INTEGER,
    quality TEXT NOT NULL CHECK (quality IN ('fresh', 'partial', 'unavailable')),
    has_more INTEGER NOT NULL CHECK (has_more IN (0, 1)),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (
        (last_good_source_generation IS NULL
            AND last_good_row_id IS NULL
            AND last_good_source_watermark IS NULL)
        OR (last_good_source_generation IS NOT NULL
            AND last_good_row_id IS NOT NULL
            AND last_good_source_watermark IS NOT NULL
            AND last_good_source_generation > 0
            AND last_good_row_id >= 0
            AND last_good_source_watermark >= 0
            AND last_good_row_id <= last_good_source_watermark)
    )
)
"#;
const POSTGRES_ANALYTICS_PROJECTION_CHECKPOINTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS analytics_projection_checkpoints (
    projection TEXT PRIMARY KEY,
    target_generation BIGINT NOT NULL CHECK (target_generation > 0),
    last_good_source_generation BIGINT,
    last_good_row_id BIGINT,
    last_good_source_watermark BIGINT,
    quality TEXT NOT NULL CHECK (quality IN ('fresh', 'partial', 'unavailable')),
    has_more BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (
        (last_good_source_generation IS NULL
            AND last_good_row_id IS NULL
            AND last_good_source_watermark IS NULL)
        OR (last_good_source_generation IS NOT NULL
            AND last_good_row_id IS NOT NULL
            AND last_good_source_watermark IS NOT NULL
            AND last_good_source_generation > 0
            AND last_good_row_id >= 0
            AND last_good_source_watermark >= 0
            AND last_good_row_id <= last_good_source_watermark)
    )
)
"#;

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
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "fresh" => Ok(Self::Fresh),
            "partial" => Ok(Self::Partial),
            "unavailable" => Ok(Self::Unavailable),
            _ => bail!("unknown analytics projection data quality: {value}"),
        }
    }

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

    pub(crate) const fn quality(self) -> ProjectionDataQuality {
        match self {
            Self::Publish { .. } => ProjectionDataQuality::Fresh,
            Self::RetainLastGood { quality, .. } | Self::Unavailable { quality } => quality,
        }
    }

    pub(crate) const fn has_more(self) -> bool {
        match self {
            Self::Publish { has_more, .. } => has_more,
            Self::RetainLastGood { .. } | Self::Unavailable { .. } => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DurableProjectionCheckpoint {
    target_generation: u64,
    last_good_cursor: Option<ProjectionCursor>,
    quality: ProjectionDataQuality,
    has_more: bool,
}

impl DurableProjectionCheckpoint {
    fn from_decision(target_generation: u64, decision: ProjectionCommitDecision) -> Result<Self> {
        let checkpoint = Self {
            target_generation,
            last_good_cursor: decision.cursor(),
            quality: decision.quality(),
            has_more: decision.has_more(),
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    fn validate(&self) -> Result<()> {
        if self.target_generation == 0 {
            bail!("analytics projection checkpoint target generation must be non-zero");
        }
        if self.quality == ProjectionDataQuality::Fresh && self.last_good_cursor.is_none() {
            bail!("fresh analytics projection checkpoint requires a last-good cursor");
        }
        if let Some(cursor) = self.last_good_cursor {
            cursor.validate()?;
            if cursor.target_generation != self.target_generation {
                bail!("analytics projection checkpoint cursor target generation mismatch");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct ProjectionCheckpointExpectation {
    exists: bool,
    checkpoint: DurableProjectionCheckpoint,
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
    checkpoint_exists: bool,
}

impl ProjectionReadState {
    pub(crate) fn new(owner: AnalyticsProjectionOwner) -> Result<Self> {
        owner.validate()?;
        Ok(Self {
            owner,
            last_good_cursor: None,
            quality: ProjectionDataQuality::Unavailable,
            has_more: false,
            checkpoint_exists: false,
        })
    }

    fn from_checkpoint(
        owner: AnalyticsProjectionOwner,
        checkpoint: DurableProjectionCheckpoint,
    ) -> Result<Self> {
        owner.validate()?;
        checkpoint.validate()?;
        if checkpoint.target_generation != owner.target_generation {
            bail!(
                "analytics projection {} checkpoint belongs to target generation {}, expected {}",
                owner.projection.name(),
                checkpoint.target_generation,
                owner.target_generation
            );
        }
        Ok(Self {
            owner,
            last_good_cursor: checkpoint.last_good_cursor,
            quality: checkpoint.quality,
            has_more: checkpoint.has_more,
            checkpoint_exists: true,
        })
    }

    fn checkpoint_expectation(self) -> ProjectionCheckpointExpectation {
        ProjectionCheckpointExpectation {
            exists: self.checkpoint_exists,
            checkpoint: DurableProjectionCheckpoint {
                target_generation: self.owner.target_generation,
                last_good_cursor: self.last_good_cursor,
                quality: self.quality,
                has_more: self.has_more,
            },
        }
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
        self.checkpoint_exists = true;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionCommitReceipt {
    pub(crate) cursor: Option<ProjectionCursor>,
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

    /// Ensure the shared checkpoint table exists on the current analytics authority. The write
    /// path also repeats this as part of its target transaction so first-use initialization and
    /// projection publication remain atomic.
    pub(crate) async fn ensure_projection_checkpoint_schema(&self) -> Result<()> {
        let lease = self
            .analytics_gate
            .acquire(StorageModule::Analytics)
            .await?;
        let backend = lease.lease.authority.backend;
        let mut transaction = self
            .pools
            .begin_typed::<AnalyticsTransaction>(lease)
            .await?;
        let result = match backend {
            BackendKind::Sqlite => {
                ensure_projection_checkpoint_schema_sqlite(
                    transaction.sqlite_transaction()?.as_mut(),
                )
                .await
            }
            BackendKind::Postgres => {
                ensure_projection_checkpoint_schema_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                )
                .await
            }
        };
        match result {
            Ok(()) => transaction.commit().await,
            Err(error) => match transaction.rollback().await {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(anyhow::anyhow!(
                    "analytics checkpoint schema setup failed: {error:#}; rollback failed: {rollback_error:#}"
                )),
            },
        }
    }

    /// Restore a projection's durable cursor and last-good quality marker. A missing row is the
    /// explicit cold-start state; a generation mismatch remains an error so recovery cannot
    /// silently reuse a cursor from a previous authority.
    pub(crate) async fn load_projection_state(
        &self,
        projection: AnalyticsProjection,
    ) -> Result<ProjectionReadState> {
        self.ensure_projection_checkpoint_schema().await?;
        let lease = self
            .analytics_gate
            .acquire(StorageModule::Analytics)
            .await?;
        let backend = lease.lease.authority.backend;
        let owner = AnalyticsProjectionOwner {
            projection,
            target_generation: lease.lease.authority.generation,
        };
        owner.validate()?;
        let mut transaction = self
            .pools
            .begin_typed::<AnalyticsTransaction>(lease)
            .await?;
        let result = match backend {
            BackendKind::Sqlite => {
                load_projection_checkpoint_sqlite(
                    transaction.sqlite_transaction()?.as_mut(),
                    projection,
                )
                .await
            }
            BackendKind::Postgres => {
                load_projection_checkpoint_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                    projection,
                )
                .await
            }
        };
        let rollback_result = transaction.rollback().await;
        let checkpoint = finish_authority_read(result, rollback_result)?;
        match checkpoint {
            Some(checkpoint) => ProjectionReadState::from_checkpoint(owner, checkpoint),
            None => ProjectionReadState::new(owner),
        }
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
        expected: &ProjectionReadState,
        batch: ProjectionBatch,
    ) -> Result<ProjectionCommitDecision> {
        expected.owner.validate()?;
        if expected.owner.target_generation != self.target_generation {
            bail!(
                "projection {} state target generation does not match its transaction",
                expected.owner.projection.name()
            );
        }
        if let Some(projection) = self.projection
            && expected.owner.projection != projection
        {
            bail!(
                "projection {} state does not match its transaction owner",
                expected.owner.projection.name()
            );
        }
        plan_projection_commit(self.target_generation, expected.last_good_cursor, batch)
    }

    async fn persist_projection_checkpoint(
        &mut self,
        projection: AnalyticsProjection,
        decision: ProjectionCommitDecision,
        expected: ProjectionCheckpointExpectation,
    ) -> Result<()> {
        if self.projection != Some(projection) {
            bail!(
                "analytics projection checkpoint {} does not match its transaction owner",
                projection.name()
            );
        }
        let checkpoint =
            DurableProjectionCheckpoint::from_decision(self.target_generation, decision)?;
        match self.backend {
            BackendKind::Sqlite => {
                persist_projection_checkpoint_sqlite(
                    self.sqlite_connection()?,
                    projection,
                    checkpoint,
                    expected,
                )
                .await
            }
            BackendKind::Postgres => {
                persist_projection_checkpoint_postgres(
                    self.postgres_connection()?,
                    projection,
                    checkpoint,
                    expected,
                )
                .await
            }
        }
    }

    pub(crate) async fn commit(self) -> Result<()> {
        if self.projection.is_some() {
            bail!("named analytics projections must commit through AnalyticsProjectionCommit");
        }
        self.transaction.commit().await
    }

    /// Start an atomic projection publication. The expected state is written into the checkpoint
    /// compare-and-swap predicate so a stale worker cannot publish after another worker wins.
    pub(crate) fn prepare_projection_commit(
        self,
        expected: ProjectionReadState,
        decision: ProjectionCommitDecision,
    ) -> Result<AnalyticsProjectionCommit> {
        let Some(projection) = self.projection else {
            bail!("analytics projection commit requires a named projection owner");
        };
        expected.owner.validate()?;
        if expected.owner.projection != projection {
            bail!(
                "projection {} state does not match its transaction owner",
                expected.owner.projection.name()
            );
        }
        if expected.owner.target_generation != self.target_generation {
            bail!(
                "projection {} state target generation does not match its transaction",
                projection.name()
            );
        }
        let expected = expected.checkpoint_expectation();
        expected.checkpoint.validate()?;
        DurableProjectionCheckpoint::from_decision(self.target_generation, decision)?;
        Ok(AnalyticsProjectionCommit {
            transaction: self,
            projection,
            decision,
            expected,
            checkpoint_persisted: false,
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
    expected: ProjectionCheckpointExpectation,
    checkpoint_persisted: bool,
}

impl AnalyticsProjectionCommit {
    pub(crate) fn transaction_mut(&mut self) -> &mut AnalyticsProjectionTransaction {
        &mut self.transaction
    }

    /// Persist the cursor, last-good baseline, quality and remaining-obligation marker through
    /// the same transaction as the projection rows. The guard records completion only after the
    /// database write succeeds; callers cannot self-report a durable checkpoint.
    pub(crate) async fn persist_checkpoint(&mut self) -> Result<()> {
        self.transaction
            .persist_projection_checkpoint(self.projection, self.decision, self.expected)
            .await?;
        self.checkpoint_persisted = true;
        Ok(())
    }

    pub(crate) async fn commit(self) -> Result<ProjectionCommitReceipt> {
        if !self.checkpoint_persisted {
            bail!("analytics projection commit requires its durable checkpoint to be persisted");
        }
        let cursor = self.decision.cursor();
        self.transaction.transaction.commit().await?;
        Ok(ProjectionCommitReceipt {
            cursor,
            advanced: self.decision.advanced(),
            has_more: self.decision.has_more(),
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
    payload: Option<String>,
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

#[derive(Debug, FromRow)]
struct SqliteProjectionCheckpointRow {
    target_generation: i64,
    last_good_source_generation: Option<i64>,
    last_good_row_id: Option<i64>,
    last_good_source_watermark: Option<i64>,
    quality: String,
    has_more: i64,
}

#[derive(Debug, FromRow)]
struct PostgresProjectionCheckpointRow {
    target_generation: i64,
    last_good_source_generation: Option<i64>,
    last_good_row_id: Option<i64>,
    last_good_source_watermark: Option<i64>,
    quality: String,
    has_more: bool,
}

#[derive(Debug, Clone, Copy)]
struct ProjectionCheckpointSqlValues {
    target_generation: i64,
    source_generation: Option<i64>,
    row_id: Option<i64>,
    source_watermark: Option<i64>,
}

async fn ensure_projection_checkpoint_schema_sqlite(
    connection: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query(SQLITE_ANALYTICS_PROJECTION_CHECKPOINTS_SCHEMA)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn ensure_projection_checkpoint_schema_postgres(connection: &mut PgConnection) -> Result<()> {
    sqlx::query(POSTGRES_ANALYTICS_PROJECTION_CHECKPOINTS_SCHEMA)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

fn projection_checkpoint_values(
    checkpoint: DurableProjectionCheckpoint,
) -> Result<ProjectionCheckpointSqlValues> {
    checkpoint.validate()?;
    let target_generation = i64::try_from(checkpoint.target_generation)
        .map_err(|_| anyhow::anyhow!("analytics projection target generation exceeds BIGINT"))?;
    let cursor_values = checkpoint.last_good_cursor.map(|cursor| {
        (
            cursor.source_generation,
            cursor.row_id,
            cursor.source_watermark,
        )
    });
    let (source_generation, row_id, source_watermark) = cursor_values.map_or(
        (None, None, None),
        |(source_generation, row_id, source_watermark)| {
            (
                Some(source_generation),
                Some(row_id),
                Some(source_watermark),
            )
        },
    );
    let source_generation = source_generation
        .map(|value| {
            i64::try_from(value)
                .map_err(|_| anyhow::anyhow!("analytics source generation exceeds BIGINT"))
        })
        .transpose()?;
    Ok(ProjectionCheckpointSqlValues {
        target_generation,
        source_generation,
        row_id,
        source_watermark,
    })
}

fn decode_projection_checkpoint(
    target_generation: i64,
    last_good_source_generation: Option<i64>,
    last_good_row_id: Option<i64>,
    last_good_source_watermark: Option<i64>,
    quality: String,
    has_more: bool,
) -> Result<DurableProjectionCheckpoint> {
    let target_generation = u64::try_from(target_generation)
        .map_err(|_| anyhow::anyhow!("analytics projection target generation is invalid"))?;
    let last_good_cursor = match (
        last_good_source_generation,
        last_good_row_id,
        last_good_source_watermark,
    ) {
        (None, None, None) => None,
        (Some(source_generation), Some(row_id), Some(source_watermark)) => Some(ProjectionCursor {
            target_generation,
            source_generation: u64::try_from(source_generation)
                .map_err(|_| anyhow::anyhow!("analytics source generation is invalid"))?,
            row_id,
            source_watermark,
        }),
        _ => bail!("analytics projection checkpoint cursor columns are incomplete"),
    };
    let checkpoint = DurableProjectionCheckpoint {
        target_generation,
        last_good_cursor,
        quality: ProjectionDataQuality::parse(&quality)?,
        has_more,
    };
    checkpoint.validate()?;
    Ok(checkpoint)
}

async fn load_projection_checkpoint_sqlite(
    connection: &mut SqliteConnection,
    projection: AnalyticsProjection,
) -> Result<Option<DurableProjectionCheckpoint>> {
    let query = format!(
        "SELECT target_generation, last_good_source_generation, last_good_row_id, last_good_source_watermark, quality, has_more FROM {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} WHERE projection = ?"
    );
    let row = sqlx::query_as::<_, SqliteProjectionCheckpointRow>(&query)
        .bind(projection.name())
        .fetch_optional(&mut *connection)
        .await?;
    row.map(|row| {
        let has_more = match row.has_more {
            0 => false,
            1 => true,
            value => bail!("analytics projection checkpoint has_more is invalid: {value}"),
        };
        decode_projection_checkpoint(
            row.target_generation,
            row.last_good_source_generation,
            row.last_good_row_id,
            row.last_good_source_watermark,
            row.quality,
            has_more,
        )
    })
    .transpose()
}

async fn load_projection_checkpoint_postgres(
    connection: &mut PgConnection,
    projection: AnalyticsProjection,
) -> Result<Option<DurableProjectionCheckpoint>> {
    let query = format!(
        "SELECT target_generation, last_good_source_generation, last_good_row_id, last_good_source_watermark, quality, has_more FROM {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} WHERE projection = $1"
    );
    let row = sqlx::query_as::<_, PostgresProjectionCheckpointRow>(&query)
        .bind(projection.name())
        .fetch_optional(&mut *connection)
        .await?;
    row.map(|row| {
        decode_projection_checkpoint(
            row.target_generation,
            row.last_good_source_generation,
            row.last_good_row_id,
            row.last_good_source_watermark,
            row.quality,
            row.has_more,
        )
    })
    .transpose()
}

async fn persist_projection_checkpoint_sqlite(
    connection: &mut SqliteConnection,
    projection: AnalyticsProjection,
    checkpoint: DurableProjectionCheckpoint,
    expected: ProjectionCheckpointExpectation,
) -> Result<()> {
    ensure_projection_checkpoint_schema_sqlite(connection).await?;
    let values = projection_checkpoint_values(checkpoint)?;
    expected.checkpoint.validate()?;
    let result = if expected.exists {
        let expected_values = projection_checkpoint_values(expected.checkpoint)?;
        let query = format!(
            "UPDATE {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} SET target_generation = ?, last_good_source_generation = ?, last_good_row_id = ?, last_good_source_watermark = ?, quality = ?, has_more = ?, updated_at = datetime('now') WHERE projection = ? AND target_generation = ? AND last_good_source_generation IS ? AND last_good_row_id IS ? AND last_good_source_watermark IS ? AND quality = ? AND has_more = ?"
        );
        sqlx::query(&query)
            .bind(values.target_generation)
            .bind(values.source_generation)
            .bind(values.row_id)
            .bind(values.source_watermark)
            .bind(checkpoint.quality.as_str())
            .bind(i64::from(checkpoint.has_more))
            .bind(projection.name())
            .bind(expected_values.target_generation)
            .bind(expected_values.source_generation)
            .bind(expected_values.row_id)
            .bind(expected_values.source_watermark)
            .bind(expected.checkpoint.quality.as_str())
            .bind(i64::from(expected.checkpoint.has_more))
            .execute(&mut *connection)
            .await?
    } else {
        let query = format!(
            "INSERT INTO {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} (projection, target_generation, last_good_source_generation, last_good_row_id, last_good_source_watermark, quality, has_more) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(projection) DO NOTHING"
        );
        sqlx::query(&query)
            .bind(projection.name())
            .bind(values.target_generation)
            .bind(values.source_generation)
            .bind(values.row_id)
            .bind(values.source_watermark)
            .bind(checkpoint.quality.as_str())
            .bind(i64::from(checkpoint.has_more))
            .execute(&mut *connection)
            .await?
    };
    if result.rows_affected() != 1 {
        bail!(
            "analytics projection checkpoint conflict for {}",
            projection.name()
        );
    }
    Ok(())
}

async fn persist_projection_checkpoint_postgres(
    connection: &mut PgConnection,
    projection: AnalyticsProjection,
    checkpoint: DurableProjectionCheckpoint,
    expected: ProjectionCheckpointExpectation,
) -> Result<()> {
    ensure_projection_checkpoint_schema_postgres(connection).await?;
    let values = projection_checkpoint_values(checkpoint)?;
    expected.checkpoint.validate()?;
    let result = if expected.exists {
        let expected_values = projection_checkpoint_values(expected.checkpoint)?;
        let query = format!(
            "UPDATE {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} SET target_generation = $1, last_good_source_generation = $2, last_good_row_id = $3, last_good_source_watermark = $4, quality = $5, has_more = $6, updated_at = CURRENT_TIMESTAMP WHERE projection = $7 AND target_generation = $8 AND last_good_source_generation IS NOT DISTINCT FROM $9 AND last_good_row_id IS NOT DISTINCT FROM $10 AND last_good_source_watermark IS NOT DISTINCT FROM $11 AND quality = $12 AND has_more = $13"
        );
        sqlx::query(&query)
            .bind(values.target_generation)
            .bind(values.source_generation)
            .bind(values.row_id)
            .bind(values.source_watermark)
            .bind(checkpoint.quality.as_str())
            .bind(checkpoint.has_more)
            .bind(projection.name())
            .bind(expected_values.target_generation)
            .bind(expected_values.source_generation)
            .bind(expected_values.row_id)
            .bind(expected_values.source_watermark)
            .bind(expected.checkpoint.quality.as_str())
            .bind(expected.checkpoint.has_more)
            .execute(&mut *connection)
            .await?
    } else {
        let query = format!(
            "INSERT INTO {ANALYTICS_PROJECTION_CHECKPOINTS_TABLE} (projection, target_generation, last_good_source_generation, last_good_row_id, last_good_source_watermark, quality, has_more) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT(projection) DO NOTHING"
        );
        sqlx::query(&query)
            .bind(projection.name())
            .bind(values.target_generation)
            .bind(values.source_generation)
            .bind(values.row_id)
            .bind(values.source_watermark)
            .bind(checkpoint.quality.as_str())
            .bind(checkpoint.has_more)
            .execute(&mut *connection)
            .await?
    };
    if result.rows_affected() != 1 {
        bail!(
            "analytics projection checkpoint conflict for {}",
            projection.name()
        );
    }
    Ok(())
}

const TRAFFIC_STATS_RESOLVED_FAILURE_CLASS_SQL_TEMPLATE: &str = concat!(
    "CASE ",
    "  WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort') ",
    "    THEN LOWER(TRIM(COALESCE(failure_class, ''))) ",
    "  ELSE CASE ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') ",
    "      AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({failure_kind}, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({downstream_error_message}, ''))) = '' THEN 'none' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) = 'warning_success' ",
    "      AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({failure_kind}, ''))) = 'downstream_closed' THEN 'none' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending') ",
    "      AND LOWER(TRIM(COALESCE(error_message, ''))) = '' THEN 'none' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({failure_kind}, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({downstream_error_message}, ''))) = '' THEN 'none' ",
    "    WHEN LOWER(TRIM(COALESCE({failure_kind}, ''))) = 'downstream_closed' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%' ",
    "      OR LOWER(TRIM(COALESCE({downstream_error_message}, ''))) LIKE '%downstream closed while streaming upstream response%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%' THEN 'client_abort' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429' ",
    "      OR LOWER(TRIM(COALESCE({failure_kind}, ''))) = 'upstream_http_429' THEN 'service_failure' ",
    "    WHEN LOWER(TRIM(COALESCE({failure_kind}, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing') ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%' ",
    "      OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) <> 'http_429') ",
    "      OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403') THEN 'client_failure' ",
    "    WHEN LOWER(TRIM(COALESCE({failure_kind}, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout', 'upstream_error') ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%' ",
    "      OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%' ",
    "      OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%' THEN 'service_failure' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') THEN 'none' ",
    "    WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_200' ",
    "      AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({failure_kind}, ''))) = '' ",
    "      AND LOWER(TRIM(COALESCE({downstream_error_message}, ''))) = '' THEN 'none' ",
    "    ELSE 'service_failure' ",
    "  END ",
    "END"
);

const SQLITE_PAYLOAD_FAILURE_KIND_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END";
const SQLITE_PAYLOAD_DOWNSTREAM_ERROR_MESSAGE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END";
// Legacy traffic rows can contain empty or malformed payload text. PostgreSQL 18's SQL/JSON
// predicate makes the cast conditional, so invalid or non-object payloads yield NULL and the
// column fallback still applies without aborting the aggregate query.
const POSTGRES_PAYLOAD_FAILURE_KIND_SQL: &str =
    "CASE WHEN payload IS JSON OBJECT THEN payload::jsonb ->> 'failureKind' END";
const POSTGRES_PAYLOAD_DOWNSTREAM_ERROR_MESSAGE_SQL: &str =
    "CASE WHEN payload IS JSON OBJECT THEN payload::jsonb ->> 'downstreamErrorMessage' END";

fn traffic_stats_resolved_failure_class_sql(
    payload_failure_kind_sql: &str,
    payload_downstream_error_message_sql: &str,
) -> String {
    let failure_kind_sql = format!("COALESCE({payload_failure_kind_sql}, failure_kind, '')");
    let downstream_error_message_sql =
        format!("COALESCE({payload_downstream_error_message_sql}, '')");
    TRAFFIC_STATS_RESOLVED_FAILURE_CLASS_SQL_TEMPLATE
        .replace("{failure_kind}", &failure_kind_sql)
        .replace("{downstream_error_message}", &downstream_error_message_sql)
}

fn traffic_stats_aggregate_select_sql(backend: BackendKind) -> String {
    let (payload_failure_kind_sql, payload_downstream_error_message_sql) = match backend {
        BackendKind::Sqlite => (
            SQLITE_PAYLOAD_FAILURE_KIND_SQL,
            SQLITE_PAYLOAD_DOWNSTREAM_ERROR_MESSAGE_SQL,
        ),
        BackendKind::Postgres => (
            POSTGRES_PAYLOAD_FAILURE_KIND_SQL,
            POSTGRES_PAYLOAD_DOWNSTREAM_ERROR_MESSAGE_SQL,
        ),
    };
    let resolved_failure_class = traffic_stats_resolved_failure_class_sql(
        payload_failure_kind_sql,
        payload_downstream_error_message_sql,
    );
    format!(
        "COUNT(*) AS total_count, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') AND ({resolved_failure_class}) = 'none' THEN 1 ELSE 0 END), 0) AS success_count, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND ({resolved_failure_class}) IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, COALESCE(SUM(cost), 0.0) AS total_cost, COALESCE(SUM(total_tokens), 0) AS total_tokens, COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND ({resolved_failure_class}) IN ('service_failure', 'client_failure', 'client_abort') THEN COALESCE(cost, 0.0) ELSE 0.0 END), 0.0) AS non_success_cost"
    )
}

async fn query_bounded_traffic_stats_sqlite(
    connection: &mut SqliteConnection,
    request: &TrafficStatsAggregateRequest,
) -> Result<(i64, crate::StatsTotals)> {
    let source_watermark =
        traffic_source_watermark_sqlite(connection, request.source_scope).await?;
    let upper_bound_id = request.upper_bound_id.min(source_watermark);
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(traffic_stats_aggregate_select_sql(BackendKind::Sqlite))
        .push(" FROM ");
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
    query
        .push(traffic_stats_aggregate_select_sql(BackendKind::Postgres))
        .push(" FROM ");
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
        "SELECT id, status, error_message, failure_kind, failure_class, payload, total_tokens, cost FROM codex_invocations WHERE id > ",
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
        source_watermark,
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
        "SELECT id, status, error_message, failure_kind, failure_class, payload, total_tokens, cost FROM codex_invocations WHERE id > ",
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
        source_watermark,
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
    read_through_id: i64,
    source_backend: BackendKind,
    source_generation: u64,
) -> TrafficStatsPage {
    let has_more = rows.len() > request.limit || read_through_id < source_watermark;
    rows.truncate(request.limit);
    let next_row_id = rows.last().map_or(request.after_row_id, |row| row.id);
    TrafficStatsPage {
        source_backend,
        source_generation,
        source_watermark,
        read_through_id,
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
    matches!(
        traffic_row_resolved_failure_class(row),
        "service_failure" | "client_failure" | "client_abort"
    )
}

fn traffic_row_payload_fields(row: &TrafficStatsFactRow) -> (Option<String>, Option<String>) {
    let Some(payload) = row.payload.as_deref() else {
        return (None, None);
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload) else {
        return (None, None);
    };
    (
        payload
            .get("failureKind")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        payload
            .get("downstreamErrorMessage")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    )
}

fn traffic_row_resolved_failure_class(row: &TrafficStatsFactRow) -> &'static str {
    let failure_class = row
        .failure_class
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(
        failure_class.as_str(),
        "service_failure" | "client_failure" | "client_abort"
    ) {
        return match failure_class.as_str() {
            "client_failure" => "client_failure",
            "client_abort" => "client_abort",
            _ => "service_failure",
        };
    }

    let status = row
        .status
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let (payload_failure_kind, payload_downstream_error_message) = traffic_row_payload_fields(row);
    let failure_kind = payload_failure_kind
        .or_else(|| row.failure_kind.clone())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let error_message = row
        .error_message
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let downstream_error_message = payload_downstream_error_message
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let error_empty = error_message.is_empty();
    let downstream_error_empty = downstream_error_message.is_empty();

    if matches!(status.as_str(), "success" | "completed")
        && error_empty
        && failure_kind.is_empty()
        && downstream_error_empty
    {
        return "none";
    }
    if status == "warning_success" && error_empty && failure_kind == "downstream_closed" {
        return "none";
    }
    if matches!(status.as_str(), "running" | "pending") && error_empty {
        return "none";
    }
    if status.is_empty() && error_empty && failure_kind.is_empty() && downstream_error_empty {
        return "none";
    }
    if failure_kind == "downstream_closed"
        || error_message.starts_with("[downstream_closed]")
        || downstream_error_message.contains("downstream closed while streaming upstream response")
        || error_message.contains("downstream closed while streaming upstream response")
    {
        return "client_abort";
    }
    if status == "http_429" || failure_kind == "upstream_http_429" {
        return "service_failure";
    }
    if matches!(
        failure_kind.as_str(),
        "request_body_stream_error_client_closed"
            | "invalid_api_key"
            | "api_key_not_found"
            | "api_key_missing"
    ) || error_message.starts_with("[request_body_stream_error_client_closed]")
        || error_message.contains("failed to read request body stream")
        || error_message.contains("invalid api key format")
        || error_message.contains("api key format is invalid")
        || error_message.contains("incorrect api key provided")
        || error_message.contains("api key not found")
        || error_message.contains("please provide an api key")
        || (status.starts_with("http_4") && status != "http_429")
        || matches!(status.as_str(), "http_401" | "http_403")
    {
        return "client_failure";
    }
    if matches!(
        failure_kind.as_str(),
        "failed_contact_upstream"
            | "upstream_response_failed"
            | "upstream_stream_error"
            | "request_body_read_timeout"
            | "upstream_handshake_timeout"
            | "upstream_error"
    ) || error_message.starts_with("[failed_contact_upstream]")
        || error_message.starts_with("[upstream_response_failed]")
        || error_message.starts_with("[upstream_stream_error]")
        || error_message.starts_with("[request_body_read_timeout]")
        || error_message.starts_with("[upstream_handshake_timeout]")
        || error_message.contains("failed to contact upstream")
        || error_message.contains("upstream response stream reported failure")
        || error_message.contains("upstream stream error")
        || error_message.contains("request body read timed out")
        || error_message.contains("upstream handshake timed out")
        || status.starts_with("http_5")
    {
        return "service_failure";
    }
    if matches!(status.as_str(), "success" | "completed" | "warning_success") {
        return "none";
    }
    if status == "http_200" && error_empty && failure_kind.is_empty() && downstream_error_empty {
        return "none";
    }
    "service_failure"
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

    static SQLITE_TEST_DATABASE_ID: AtomicUsize = AtomicUsize::new(0);

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
        sqlite_store_with_max_connections(1).await
    }

    async fn sqlite_store_with_max_connections(
        max_connections: u32,
    ) -> Result<(AnalyticsProjectionStore, SqlitePool)> {
        let database_url = if max_connections == 1 {
            "sqlite::memory:".to_string()
        } else {
            let database_id = SQLITE_TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            format!("sqlite:file:analytics-projection-test-{database_id}?mode=memory&cache=shared")
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect(&database_url)
            .await?;
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, occurred_at TEXT, status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT, payload TEXT, total_tokens INTEGER, cost REAL)",
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
    async fn bounded_sqlite_empty_page_below_source_watermark_remains_partial() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        sqlx::query("INSERT INTO codex_invocations (id, source, status, error_message) VALUES (2, 'proxy', 'completed', '')")
            .execute(&pool)
            .await?;
        let page = store
            .read_bounded_traffic_stats(TrafficStatsReadRequest {
                after_row_id: 1,
                upper_bound_id: Some(1),
                limit: 1,
                source_scope: crate::InvocationSourceScope::All,
            })
            .await?;
        assert_eq!(page.row_count, 0);
        assert_eq!(page.next_row_id, 1);
        assert_eq!(page.source_watermark, 2);
        assert_eq!(page.read_through_id, 1);
        assert!(page.has_more);
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
    async fn legacy_none_failure_class_falls_back_to_failure_kind() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        sqlx::query("INSERT INTO codex_invocations (id, source, status, error_message, failure_kind, failure_class, total_tokens, cost) VALUES (1, 'proxy', 'failed', '', 'upstream_error', 'none', 10, 0.5)")
            .execute(&pool)
            .await?;

        let read = store
            .query_bounded_traffic_stats(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::All,
                upper_bound_id: 1,
            })
            .await?;
        assert_eq!(read.aggregate.failure_count, 1);
        assert_eq!(read.aggregate.non_success_cost, 0.5);

        let page = store
            .read_bounded_traffic_stats(TrafficStatsReadRequest::default())
            .await?;
        assert_eq!(page.aggregate.failure_count, 1);
        assert_eq!(page.aggregate.non_success_cost, 0.5);
        Ok(())
    }

    #[tokio::test]
    async fn payload_only_failure_fields_are_used_by_aggregate_and_page() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        for (id, payload) in [
            (1, r#"{"failureKind":"upstream_response_failed"}"#),
            (
                2,
                r#"{"downstreamErrorMessage":"downstream closed while streaming upstream response"}"#,
            ),
        ] {
            sqlx::query("INSERT INTO codex_invocations (id, source, status, error_message, failure_kind, failure_class, payload, total_tokens, cost) VALUES (?1, 'proxy', 'failed', '', NULL, 'none', ?2, 10, 0.5)")
                .bind(id)
                .bind(payload)
                .execute(&pool)
                .await?;
        }

        let read = store
            .query_bounded_traffic_stats(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::All,
                upper_bound_id: 2,
            })
            .await?;
        assert_eq!(read.aggregate.failure_count, 2);
        assert_eq!(read.aggregate.non_success_cost, 1.0);

        let page = store
            .read_bounded_traffic_stats(TrafficStatsReadRequest::default())
            .await?;
        assert_eq!(page.row_count, 2);
        assert_eq!(page.aggregate.failure_count, 2);
        assert_eq!(page.aggregate.non_success_cost, 1.0);
        Ok(())
    }

    #[test]
    fn postgres_payload_classification_is_fail_soft_for_malformed_text() {
        let postgres_sql = traffic_stats_aggregate_select_sql(BackendKind::Postgres);
        assert!(postgres_sql.contains("payload IS JSON OBJECT"));
        assert!(postgres_sql.contains("failureKind"));
        assert!(postgres_sql.contains("downstreamErrorMessage"));
        assert!(postgres_sql.contains("payload::jsonb ->> 'failureKind'"));

        let sqlite_sql = traffic_stats_aggregate_select_sql(BackendKind::Sqlite);
        assert!(sqlite_sql.contains("json_valid(payload)"));
    }

    #[tokio::test]
    async fn malformed_payload_is_ignored_while_valid_payload_is_classified() -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        for (id, status, payload, cost) in [
            (1, "http_200", "{bad-json", 0.25),
            (
                2,
                "failed",
                r#"{"failureKind":"upstream_response_failed"}"#,
                0.5,
            ),
        ] {
            sqlx::query("INSERT INTO codex_invocations (id, source, status, error_message, failure_kind, failure_class, payload, total_tokens, cost) VALUES (?1, 'proxy', ?2, '', NULL, 'none', ?3, 10, ?4)")
                .bind(id)
                .bind(status)
                .bind(payload)
                .bind(cost)
                .execute(&pool)
                .await?;
        }

        let read = store
            .query_bounded_traffic_stats(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::All,
                upper_bound_id: 2,
            })
            .await?;
        assert_eq!(read.aggregate.total_count, 2);
        assert_eq!(read.aggregate.failure_count, 1);
        assert_eq!(read.aggregate.non_success_cost, 0.5);

        let page = store
            .read_bounded_traffic_stats(TrafficStatsReadRequest::default())
            .await?;
        assert_eq!(page.row_count, 2);
        assert_eq!(page.aggregate.failure_count, 1);
        assert_eq!(page.aggregate.non_success_cost, 0.5);
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
        let expected = store
            .load_projection_state(AnalyticsProjection::Summary)
            .await?;
        let transaction = store.begin_projection(AnalyticsProjection::Summary).await?;
        let decision = transaction
            .plan_projection_commit(&expected, batch(ProjectionDataQuality::Fresh, 1, false))?;
        let mut commit = transaction.prepare_projection_commit(expected, decision)?;
        sqlx::query("INSERT INTO projection_rows (id) VALUES (1)")
            .execute(commit.transaction_mut().sqlite_connection()?)
            .await?;
        commit.persist_checkpoint().await?;
        let receipt = commit.commit().await?;
        assert!(receipt.advanced);
        assert_eq!(receipt.cursor.unwrap().row_id, 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projection_rows")
                .fetch_one(&pool)
                .await?,
            1
        );
        let restored = store
            .load_projection_state(AnalyticsProjection::Summary)
            .await?;
        assert_eq!(restored.quality, ProjectionDataQuality::Fresh);
        assert_eq!(restored.last_good_cursor.unwrap().row_id, 1);
        assert!(!restored.has_more);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM analytics_projection_checkpoints WHERE projection = 'summary'",
            )
            .fetch_one(&pool)
            .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn projection_checkpoint_restores_last_good_after_partial_commit() -> Result<()> {
        let (store, _pool) = sqlite_store().await?;
        let expected = store
            .load_projection_state(AnalyticsProjection::Timeseries)
            .await?;
        let transaction = store
            .begin_projection(AnalyticsProjection::Timeseries)
            .await?;
        let decision = transaction
            .plan_projection_commit(&expected, batch(ProjectionDataQuality::Fresh, 4, true))?;
        let mut commit = transaction.prepare_projection_commit(expected, decision)?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;

        let fresh = store
            .load_projection_state(AnalyticsProjection::Timeseries)
            .await?;
        let last_good_cursor = fresh.last_good_cursor;
        let transaction = store
            .begin_projection(AnalyticsProjection::Timeseries)
            .await?;
        let decision = transaction
            .plan_projection_commit(&fresh, batch(ProjectionDataQuality::Partial, 6, true))?;
        assert!(matches!(
            decision,
            ProjectionCommitDecision::RetainLastGood {
                quality: ProjectionDataQuality::Partial,
                ..
            }
        ));
        let mut commit = transaction.prepare_projection_commit(fresh, decision)?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;

        let restored = store
            .load_projection_state(AnalyticsProjection::Timeseries)
            .await?;
        assert_eq!(restored.last_good_cursor, last_good_cursor);
        assert_eq!(restored.quality, ProjectionDataQuality::Partial);
        assert!(restored.has_more);
        Ok(())
    }

    #[tokio::test]
    async fn stale_projection_checkpoint_commit_conflicts_without_regressing_fresh_state()
    -> Result<()> {
        let (store, _pool) = sqlite_store_with_max_connections(2).await?;
        let store = std::sync::Arc::new(store);
        let initial = store
            .load_projection_state(AnalyticsProjection::Summary)
            .await?;
        let transaction = store.begin_projection(AnalyticsProjection::Summary).await?;
        let decision = transaction
            .plan_projection_commit(&initial, batch(ProjectionDataQuality::Fresh, 4, true))?;
        let mut commit = transaction.prepare_projection_commit(initial, decision)?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;

        // Both workers captured this same durable snapshot before either one planned its batch.
        let expected = store
            .load_projection_state(AnalyticsProjection::Summary)
            .await?;
        let stale_decision = plan_projection_commit(
            expected.owner.target_generation,
            expected.last_good_cursor,
            batch(ProjectionDataQuality::Partial, 4, true),
        )?;
        let fresh_decision = plan_projection_commit(
            expected.owner.target_generation,
            expected.last_good_cursor,
            batch(ProjectionDataQuality::Fresh, 6, false),
        )?;

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let fresh_done = std::sync::Arc::new(tokio::sync::Notify::new());
        let fresh_task = {
            let store = std::sync::Arc::clone(&store);
            let barrier = std::sync::Arc::clone(&barrier);
            let fresh_done = std::sync::Arc::clone(&fresh_done);
            tokio::spawn(async move {
                let transaction = store.begin_projection(AnalyticsProjection::Summary).await?;
                let mut commit = transaction.prepare_projection_commit(expected, fresh_decision)?;
                barrier.wait().await;
                commit.persist_checkpoint().await?;
                commit.commit().await?;
                fresh_done.notify_one();
                Ok::<(), anyhow::Error>(())
            })
        };
        let stale_task = {
            let store = std::sync::Arc::clone(&store);
            let barrier = std::sync::Arc::clone(&barrier);
            let fresh_done = std::sync::Arc::clone(&fresh_done);
            tokio::spawn(async move {
                let transaction = store.begin_projection(AnalyticsProjection::Summary).await?;
                let mut commit = transaction.prepare_projection_commit(expected, stale_decision)?;
                let fresh_committed = fresh_done.notified();
                barrier.wait().await;
                fresh_committed.await;
                let error = commit.persist_checkpoint().await.unwrap_err();
                assert!(error.to_string().contains("checkpoint conflict"));
                commit.rollback().await?;
                Ok::<(), anyhow::Error>(())
            })
        };
        fresh_task.await??;
        stale_task.await??;

        let restored = store
            .load_projection_state(AnalyticsProjection::Summary)
            .await?;
        assert_eq!(restored.last_good_cursor.unwrap().row_id, 6);
        assert_eq!(restored.quality, ProjectionDataQuality::Fresh);
        assert!(!restored.has_more);
        Ok(())
    }

    #[tokio::test]
    async fn projection_commit_requires_checkpoint_write_and_persists_unavailable_quality()
    -> Result<()> {
        let (store, pool) = sqlite_store().await?;
        sqlx::query("CREATE TABLE projection_rows (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await?;
        let expected = store
            .load_projection_state(AnalyticsProjection::LongTerm)
            .await?;
        let transaction = store
            .begin_projection(AnalyticsProjection::LongTerm)
            .await?;
        let decision = transaction
            .plan_projection_commit(&expected, batch(ProjectionDataQuality::Partial, 1, true))?;
        assert!(matches!(
            decision,
            ProjectionCommitDecision::Unavailable { .. }
        ));
        let commit = transaction.prepare_projection_commit(expected, decision)?;
        let error = commit.commit().await.unwrap_err();
        assert!(error.to_string().contains("durable checkpoint"));

        let expected = store
            .load_projection_state(AnalyticsProjection::LongTerm)
            .await?;
        let transaction = store
            .begin_projection(AnalyticsProjection::LongTerm)
            .await?;
        let decision = transaction
            .plan_projection_commit(&expected, batch(ProjectionDataQuality::Partial, 1, true))?;
        let mut commit = transaction.prepare_projection_commit(expected, decision)?;
        commit.persist_checkpoint().await?;
        let receipt = commit.commit().await?;
        assert!(receipt.cursor.is_none());
        let restored = store
            .load_projection_state(AnalyticsProjection::LongTerm)
            .await?;
        assert_eq!(restored.last_good_cursor, None);
        assert_eq!(restored.quality, ProjectionDataQuality::Partial);
        assert!(restored.has_more);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projection_rows")
                .fetch_one(&pool)
                .await?,
            0
        );
        Ok(())
    }
}
