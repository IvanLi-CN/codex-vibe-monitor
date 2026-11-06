//! Continuous, at-least-once source consumption for analytics module consumers.
//!
//! The actor owns one detached PostgreSQL connection for its whole lifetime. The connection
//! holds the session advisory lock, so a pool checkout cannot accidentally outlive the lock or
//! return a locked session to another writer. PostgreSQL receive (C) and the per-consumer receipt
//! log are committed before SQLite source acknowledgement (A) is attempted.

use super::{
    AnalyticsModuleId, LEGACY_SOURCE_INSTANCE_ID, SOURCE_ENVELOPE_VERSION,
    SOURCE_FACT_SEMANTICS_VERSION, SOURCE_STATS_SEMANTICS_VERSION, canonical_json, digest_json,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use sqlx::{Connection, FromRow, PgConnection, PgPool, Pool, Sqlite};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::source::{SourceEnvelope, SourceIdentity, SourceOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IngestWorkKind {
    Projection,
    Archive,
}

impl IngestWorkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::Archive => "archive",
        }
    }
}

/// A successor registers an authenticated operation handler here. The handler runs inside the
/// actor's receive transaction; it must stage the complete input and any obligation before
/// returning. The receipt, work item, and consumer checkpoint are then committed by C3 together.
#[async_trait::async_trait]
pub(crate) trait IngestOperationHandler: Send + Sync {
    fn operation(&self) -> SourceOperation;

    async fn stage(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event: &SourceEnvelope,
    ) -> Result<IngestWorkKind>;
}

pub(crate) struct IngestHandlerRegistry {
    handlers: Vec<Box<dyn IngestOperationHandler>>,
}

impl IngestHandlerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub(crate) fn register<H>(&mut self, handler: H) -> Result<()>
    where
        H: IngestOperationHandler + 'static,
    {
        ensure!(
            self.handlers
                .iter()
                .all(|registered| registered.operation() != handler.operation()),
            "analytics ingest operation handler is already registered"
        );
        self.handlers.push(Box::new(handler));
        Ok(())
    }

    fn handler_for(&self, operation: SourceOperation) -> Option<&dyn IngestOperationHandler> {
        self.handlers
            .iter()
            .find(|handler| handler.operation() == operation)
            .map(Box::as_ref)
    }
}

impl Default for IngestHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) const ANALYTICS_INGEST_MAX_EVENTS_PER_BATCH: usize = 256;
pub(crate) const ANALYTICS_INGEST_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub(crate) const ANALYTICS_INGEST_IDLE_CADENCE: Duration = Duration::from_secs(5);
pub(crate) const ANALYTICS_INGEST_RETRY_INITIAL: Duration = Duration::from_secs(1);
pub(crate) const ANALYTICS_INGEST_RETRY_MAX: Duration = Duration::from_secs(60);

// Distinct from the legacy pg-analytics bootstrap lock and the module migration lock.
const ANALYTICS_INGEST_LOCK_KEY: i64 = 0x4356_4d5f_494e_4754;
const ANALYTICS_INGEST_MIGRATION_LOCK_KEY: i64 = 0x4356_4d5f_494e_4744;
pub(crate) const ANALYTICS_INGEST_DDL_COMPONENT: &str = "analytics-module-ingest";
pub(crate) const ANALYTICS_INGEST_DDL_STEP: &str = "ingest-receipt-v1";
pub(crate) const ANALYTICS_INGEST_SCHEMA_VERSION: i64 = 1;
pub(crate) const ANALYTICS_INGEST_READER_VERSION: i64 = 1;
const ANALYTICS_INGEST_MAX_BATCHES_PER_DRAIN: usize = 4096;

/// Owner-applied additive SQL. It is intentionally not included in the C2 module checksum: the
/// C3 receipt contract has its own deployment seam and must be provisioned before the actor runs.
pub(crate) const ANALYTICS_INGEST_SCHEMA_SQL: &str = include_str!("ingest/schema.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsIngestMigrationResult {
    pub(crate) component: &'static str,
    pub(crate) version: i64,
    pub(crate) checksum: String,
    pub(crate) recovered_steps: usize,
}

#[derive(Debug)]
pub(crate) enum IngestError {
    Retryable(anyhow::Error),
    Blocked(anyhow::Error),
    AmbiguousCommit(anyhow::Error),
}

impl IngestError {
    fn retryable(error: impl Into<anyhow::Error>) -> Self {
        Self::Retryable(error.into())
    }

    fn blocked(error: impl Into<anyhow::Error>) -> Self {
        Self::Blocked(error.into())
    }

    fn ambiguous_commit(error: impl Into<anyhow::Error>) -> Self {
        Self::AmbiguousCommit(error.into())
    }

    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

impl fmt::Display for IngestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable(error) => {
                write!(formatter, "analytics ingest retryable failure: {error}")
            }
            Self::Blocked(error) => write!(formatter, "analytics ingest blocked: {error}"),
            Self::AmbiguousCommit(error) => {
                write!(
                    formatter,
                    "analytics ingest commit outcome is ambiguous: {error}"
                )
            }
        }
    }
}

impl std::error::Error for IngestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Retryable(error) | Self::Blocked(error) | Self::AmbiguousCommit(error) => {
                Some(error.as_ref())
            }
        }
    }
}

pub(crate) type IngestResult<T> = std::result::Result<T, IngestError>;

macro_rules! ingest_ensure {
    ($condition:expr, $($arg:tt)+) => {
        if !($condition) {
            return Err(IngestError::blocked(anyhow!($($arg)+)));
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceBinding {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) consumer_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    checkpoint: i64,
    coverage_revision: i64,
}

impl SourceBinding {
    pub(crate) fn new(
        module_id: AnalyticsModuleId,
        consumer_id: impl Into<String>,
        source_instance_id: impl Into<String>,
        generation: i64,
        owner_epoch: i64,
        checkpoint: i64,
        coverage_revision: i64,
    ) -> Result<Self> {
        let binding = Self {
            module_id,
            consumer_id: consumer_id.into(),
            source_instance_id: source_instance_id.into(),
            generation,
            owner_epoch,
            checkpoint,
            coverage_revision,
        };
        ensure!(
            !binding.consumer_id.trim().is_empty(),
            "consumer id must not be empty"
        );
        ensure!(
            !binding.source_instance_id.trim().is_empty(),
            "source instance id must not be empty"
        );
        ensure!(
            binding.generation >= 0,
            "source generation must not be negative"
        );
        ensure!(binding.owner_epoch > 0, "owner epoch must be positive");
        ensure!(
            binding.checkpoint >= 0,
            "consumer checkpoint must not be negative"
        );
        ensure!(
            binding.coverage_revision >= 0,
            "consumer coverage revision must not be negative"
        );
        Ok(binding)
    }

    pub(crate) fn checkpoint(&self) -> i64 {
        self.checkpoint
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourcePrefix {
    events: Vec<SourceEnvelope>,
    first_change_id: i64,
    through_change_id: i64,
}

impl SourcePrefix {
    pub(crate) fn events(&self) -> &[SourceEnvelope] {
        &self.events
    }

    pub(crate) fn first_change_id(&self) -> i64 {
        self.first_change_id
    }

    pub(crate) fn through_change_id(&self) -> i64 {
        self.through_change_id
    }
}

/// Validate a bounded, complete source prefix without sorting it. Sorting would turn a source
/// gap or an out-of-order read into an apparently valid prefix and could make A exceed C.
pub(crate) fn validate_complete_prefix(
    binding: &SourceBinding,
    expected_checkpoint: i64,
    events: &[SourceEnvelope],
) -> Result<SourcePrefix> {
    ensure!(
        !events.is_empty(),
        "analytics ingest cannot receive an empty prefix"
    );
    ensure!(
        events.len() <= ANALYTICS_INGEST_MAX_EVENTS_PER_BATCH,
        "analytics ingest prefix exceeds the bounded event count"
    );

    let mut unique_events = Vec::with_capacity(events.len());
    let mut by_event_id = HashMap::<&str, usize>::new();
    let mut encoded_bytes = 0usize;
    for event in events {
        ensure!(
            event.envelope_version == 1 || event.envelope_version == SOURCE_ENVELOPE_VERSION,
            "unknown analytics source envelope version {} at change {}",
            event.envelope_version,
            event.change_id
        );
        ensure!(
            event.fact_semantics_version == SOURCE_FACT_SEMANTICS_VERSION,
            "unknown analytics fact semantics version {} at change {}",
            event.fact_semantics_version,
            event.change_id
        );
        ensure!(
            event.stats_semantics_version == 1
                || event.stats_semantics_version == SOURCE_STATS_SEMANTICS_VERSION,
            "unknown analytics stats semantics version {} at change {}",
            event.stats_semantics_version,
            event.change_id
        );
        ensure!(
            event.source_instance_id == binding.source_instance_id
                && event.identity.source_instance_id == binding.source_instance_id,
            "analytics source instance binding mismatch at change {}",
            event.change_id
        );
        ensure!(
            event.generation == binding.generation,
            "analytics source generation mismatch at change {}",
            event.change_id
        );
        ensure!(
            event.change_id >= 1,
            "analytics source change id must be positive"
        );
        ensure!(
            event.coverage_revision >= 0,
            "analytics source coverage revision must not be negative"
        );
        event.identity.validate()?;
        ensure!(
            !event.event_id.trim().is_empty(),
            "analytics source event id must not be empty"
        );
        ensure!(
            !event.occurred_at.trim().is_empty(),
            "analytics source occurred_at must not be empty"
        );
        ensure!(
            event.payload.is_object(),
            "analytics source payload must be a JSON object"
        );
        ensure!(
            digest_json(&event.payload)? == event.payload_digest,
            "analytics source payload digest mismatch at change {}",
            event.change_id
        );
        let payload_bytes = serde_json::to_vec(&canonical_json(&event.payload))?.len();
        encoded_bytes = encoded_bytes
            .checked_add(payload_bytes)
            .context("analytics ingest payload byte count overflow")?;
        ensure!(
            encoded_bytes <= ANALYTICS_INGEST_MAX_PAYLOAD_BYTES,
            "analytics ingest prefix exceeds the bounded payload size"
        );

        if let Some(previous_index) = by_event_id.get(event.event_id.as_str()) {
            let previous = &unique_events[*previous_index];
            ensure!(
                same_event_identity(previous, event),
                "analytics source event id {} has conflicting content",
                event.event_id
            );
            continue;
        }
        by_event_id.insert(event.event_id.as_str(), unique_events.len());
        unique_events.push(event.clone());
    }

    ensure!(
        !unique_events.is_empty(),
        "analytics ingest prefix contains no events"
    );
    for window in unique_events.windows(2) {
        ensure!(
            window[1].change_id == window[0].change_id.saturating_add(1),
            "analytics source prefix is not contiguous: {} followed by {}",
            window[0].change_id,
            window[1].change_id
        );
        ensure!(
            window[1].coverage_revision >= window[0].coverage_revision,
            "analytics source coverage revision moved backwards at change {}",
            window[1].change_id
        );
    }
    let first_change_id = unique_events[0].change_id;
    let through_change_id = unique_events
        .last()
        .expect("non-empty source prefix")
        .change_id;
    ensure!(
        first_change_id <= expected_checkpoint
            || first_change_id == expected_checkpoint.saturating_add(1),
        "analytics source prefix starts at {}, expected {}",
        first_change_id,
        expected_checkpoint.saturating_add(1)
    );
    ensure!(
        through_change_id <= expected_checkpoint || first_change_id > expected_checkpoint,
        "analytics source prefix overlaps the checkpoint and contains new events"
    );
    Ok(SourcePrefix {
        events: unique_events,
        first_change_id,
        through_change_id,
    })
}

fn same_event_identity(left: &SourceEnvelope, right: &SourceEnvelope) -> bool {
    left.source_instance_id == right.source_instance_id
        && left.event_id == right.event_id
        && left.generation == right.generation
        && left.change_id == right.change_id
        && left.coverage_revision == right.coverage_revision
        && left.fact_semantics_version == right.fact_semantics_version
        && left.stats_semantics_version == right.stats_semantics_version
        && left.identity == right.identity
        && left.operation == right.operation
        && left.occurred_at == right.occurred_at
        && left.payload_digest == right.payload_digest
}

fn ensure_receivable_operation(event: &SourceEnvelope) -> Result<()> {
    if builtin_work_kind(event.operation).is_some() {
        return Ok(());
    }
    bail!(
        "analytics source operation {} at change {} has no C3 authenticated staging handler; refusing to ack",
        event.operation.as_str(),
        event.change_id
    )
}

fn builtin_work_kind(operation: SourceOperation) -> Option<IngestWorkKind> {
    match operation {
        SourceOperation::Upsert | SourceOperation::Correction | SourceOperation::LogicalRevoke => {
            Some(IngestWorkKind::Projection)
        }
        SourceOperation::ArchiveRelocation
        | SourceOperation::RetentionRetirement
        | SourceOperation::Coverage => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveReceipt {
    pub(crate) first_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) inserted_events: usize,
    pub(crate) duplicate_events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsumeStatus {
    Idle {
        checkpoint: i64,
    },
    Progressed(ReceiveReceipt),
    AckPending {
        checkpoint: i64,
        through_change_id: i64,
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct PendingAck {
    generation: i64,
    from_change_id: i64,
    through_change_id: i64,
}

#[derive(Debug, FromRow)]
struct OwnerRow {
    source_instance_id: String,
    owner_epoch: i64,
    status: String,
}

#[derive(Debug, FromRow)]
struct ConsumerRow {
    source_instance_id: String,
    owner_epoch: i64,
    state: String,
    checkpoint: i64,
    coverage_revision: i64,
}

#[derive(Debug, FromRow)]
struct ReceiptRow {
    event_id: String,
    generation: i64,
    change_id: i64,
    coverage_revision: i64,
    entity_kind: String,
    entity_key: String,
    operation_kind: String,
    payload_digest: String,
    payload: Json<Value>,
}

#[derive(Debug, FromRow)]
struct SourceStateRow {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
}

#[derive(Debug, FromRow)]
struct SourceCoverageRow {
    generation: i64,
    covered_through_change_id: i64,
}

#[derive(Debug, FromRow)]
struct SourceJournalRow {
    change_id: i64,
    event_version: i64,
    fact_date_semantics_version: i64,
    source_instance_id: String,
    event_id: String,
    operation_kind: String,
    fact_semantics_version: i64,
    stats_semantics_version: i64,
    payload_digest: String,
    event_kind: String,
    entity_kind: String,
    entity_key: String,
    generation: i64,
    coverage_revision: i64,
    occurred_at: String,
    fact_json: String,
}

/// The single PG writer actor. The detached connection is never returned to a pool while the
/// session lock is held.
pub(crate) struct PgIngestActor {
    connection: PgConnection,
    binding: SourceBinding,
    pending_ack: Option<PendingAck>,
}

impl PgIngestActor {
    pub(crate) async fn acquire(pool: &PgPool, binding: SourceBinding) -> IngestResult<Self> {
        let mut connection = pool
            .acquire()
            .await
            .map_err(|error| {
                IngestError::retryable(
                    anyhow!(error).context("acquire PostgreSQL ingest connection"),
                )
            })?
            .detach();
        let locked = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(ANALYTICS_INGEST_LOCK_KEY)
            .fetch_one(&mut connection)
            .await
            .map_err(|error| {
                IngestError::retryable(
                    anyhow!(error).context("acquire analytics ingest writer lock"),
                )
            })?;
        if !locked {
            let _ = connection.close().await;
            return Err(IngestError::retryable(anyhow!(
                "another analytics ingest writer owns the session lock"
            )));
        }
        let mut actor = Self {
            connection,
            binding,
            pending_ack: None,
        };
        if let Err(error) = actor.refresh_binding().await {
            let _ = actor.close().await;
            return Err(error);
        }
        Ok(actor)
    }

    async fn refresh_binding(&mut self) -> IngestResult<()> {
        let row = sqlx::query_as::<_, ConsumerRow>(
            "SELECT source_instance_id, owner_epoch, state, checkpoint, coverage_revision
             FROM cvm_analytics.analytics_module_consumers
             WHERE consumer_id = $1 AND module_id = $2",
        )
        .bind(&self.binding.consumer_id)
        .bind(self.binding.module_id.as_str())
        .fetch_optional(&mut self.connection)
        .await
        .map_err(|error| {
            IngestError::retryable(anyhow!(error).context("load analytics consumer binding"))
        })?
        .ok_or_else(|| IngestError::blocked(anyhow!("analytics consumer binding is missing")))?;
        ensure_binding_consumer(&self.binding, &row).map_err(IngestError::blocked)?;
        self.binding.checkpoint = row.checkpoint;
        self.binding.coverage_revision = row.coverage_revision;
        Ok(())
    }

    pub(crate) async fn receive_batch(
        &mut self,
        events: &[SourceEnvelope],
    ) -> IngestResult<ReceiveReceipt> {
        let handlers = IngestHandlerRegistry::default();
        self.receive_batch_with_handlers(events, &handlers).await
    }

    pub(crate) async fn receive_batch_with_handlers(
        &mut self,
        events: &[SourceEnvelope],
        handlers: &IngestHandlerRegistry,
    ) -> IngestResult<ReceiveReceipt> {
        let mut transaction = self.connection.begin().await.map_err(|error| {
            IngestError::retryable(anyhow!(error).context("begin analytics ingest transaction"))
        })?;
        let owner = sqlx::query_as::<_, OwnerRow>(
            "SELECT source_instance_id, owner_epoch, status
             FROM cvm_analytics.analytics_module_owners
             WHERE module_id = $1
             FOR UPDATE",
        )
        .bind(self.binding.module_id.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| {
            IngestError::retryable(anyhow!(error).context("lock analytics module owner"))
        })?
        .ok_or_else(|| IngestError::blocked(anyhow!("analytics module owner row is missing")))?;
        ingest_ensure!(
            owner.status == "active",
            "analytics module owner is not active"
        );
        ingest_ensure!(
            owner.owner_epoch == self.binding.owner_epoch,
            "analytics module owner epoch was fenced"
        );
        ingest_ensure!(
            owner.source_instance_id == self.binding.source_instance_id,
            "analytics module owner is bound to a different source instance"
        );

        let consumer = sqlx::query_as::<_, ConsumerRow>(
            "SELECT source_instance_id, owner_epoch, state, checkpoint, coverage_revision
             FROM cvm_analytics.analytics_module_consumers
             WHERE consumer_id = $1 AND module_id = $2
             FOR UPDATE",
        )
        .bind(&self.binding.consumer_id)
        .bind(self.binding.module_id.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| {
            IngestError::retryable(anyhow!(error).context("lock analytics consumer checkpoint"))
        })?
        .ok_or_else(|| IngestError::blocked(anyhow!("analytics consumer row is missing")))?;
        ensure_binding_consumer(&self.binding, &consumer).map_err(IngestError::blocked)?;

        let prefix = validate_complete_prefix(&self.binding, consumer.checkpoint, events)
            .map_err(IngestError::blocked)?;
        for event in prefix.events() {
            ingest_ensure!(
                builtin_work_kind(event.operation).is_some()
                    || handlers.handler_for(event.operation).is_some(),
                "analytics source operation {} at change {} has no registered authenticated handler; refusing to ack",
                event.operation.as_str(),
                event.change_id
            );
        }
        let mut inserted = Vec::new();
        let mut duplicates = 0usize;
        for event in prefix.events() {
            let existing_by_event = sqlx::query_as::<_, ReceiptRow>(
                "SELECT event_id, generation, change_id, coverage_revision,
                        entity_kind, entity_key, operation_kind, payload_digest, payload
                 FROM cvm_analytics.analytics_module_ingest_receipts
                 WHERE consumer_id = $1 AND source_instance_id = $2 AND event_id = $3",
            )
            .bind(&self.binding.consumer_id)
            .bind(&self.binding.source_instance_id)
            .bind(&event.event_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| {
                IngestError::retryable(anyhow!(error).context("load analytics ingest receipt"))
            })?;
            if let Some(existing) = existing_by_event {
                ensure_receipt_matches(&existing, event).map_err(IngestError::blocked)?;
                ingest_ensure!(
                    event.change_id <= consumer.checkpoint,
                    "analytics receipt exists beyond the durable consumer checkpoint"
                );
                duplicates = duplicates.saturating_add(1);
                continue;
            }

            let existing_by_change = sqlx::query_as::<_, ReceiptRow>(
                "SELECT event_id, generation, change_id, coverage_revision,
                        entity_kind, entity_key, operation_kind, payload_digest, payload
                 FROM cvm_analytics.analytics_module_ingest_receipts
                 WHERE consumer_id = $1 AND source_instance_id = $2
                   AND generation = $3 AND change_id = $4",
            )
            .bind(&self.binding.consumer_id)
            .bind(&self.binding.source_instance_id)
            .bind(event.generation)
            .bind(event.change_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| {
                IngestError::retryable(
                    anyhow!(error).context("load analytics ingest cursor receipt"),
                )
            })?;
            ingest_ensure!(
                existing_by_change.is_none(),
                "analytics change id {} is already registered with a different event identity",
                event.change_id
            );
            inserted.push(event);
        }

        if inserted.is_empty() {
            let receipt = ReceiveReceipt {
                first_change_id: prefix.first_change_id(),
                through_change_id: prefix.through_change_id(),
                generation: self.binding.generation,
                coverage_revision: prefix
                    .events()
                    .last()
                    .map(|event| event.coverage_revision)
                    .unwrap_or(consumer.coverage_revision),
                inserted_events: 0,
                duplicate_events: duplicates,
            };
            transaction.commit().await.map_err(|error| {
                IngestError::ambiguous_commit(
                    anyhow!(error).context(
                        "analytics duplicate receipt transaction commit outcome is ambiguous",
                    ),
                )
            })?;
            return Ok(receipt);
        }

        ingest_ensure!(
            prefix.first_change_id() == consumer.checkpoint.saturating_add(1),
            "analytics ingest has new events but the source prefix is not immediately after the checkpoint"
        );
        ingest_ensure!(
            duplicates == 0,
            "analytics ingest found a mixed duplicate/new prefix; resolve the durable checkpoint before retrying"
        );

        for event in &inserted {
            let work_kind = if let Some(work_kind) = builtin_work_kind(event.operation) {
                work_kind
            } else {
                let handler = handlers
                    .handler_for(event.operation)
                    .expect("operation handler was checked before receive writes");
                let work_kind = handler
                    .stage(&mut transaction, event)
                    .await
                    .map_err(IngestError::blocked)?;
                ingest_ensure!(
                    work_kind == IngestWorkKind::Archive,
                    "registered handler for {} did not return an archive work item",
                    event.operation.as_str()
                );
                work_kind
            };
            let payload_bytes = serde_json::to_vec(&canonical_json(&event.payload))
                .map_err(|error| IngestError::blocked(anyhow!(error)))?
                .len();
            let payload_bytes = i32::try_from(payload_bytes)
                .map_err(|error| IngestError::blocked(anyhow!(error)))?;
            sqlx::query(
                "INSERT INTO cvm_analytics.analytics_module_ingest_receipts
                    (consumer_id, source_instance_id, event_id, generation, change_id,
                     coverage_revision, entity_kind, entity_key, operation_kind,
                     payload_digest, payload, payload_bytes)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(&self.binding.consumer_id)
            .bind(&self.binding.source_instance_id)
            .bind(&event.event_id)
            .bind(event.generation)
            .bind(event.change_id)
            .bind(event.coverage_revision)
            .bind(&event.identity.entity_kind)
            .bind(&event.identity.entity_key)
            .bind(event.operation.as_str())
            .bind(&event.payload_digest)
            .bind(Json(canonical_json(&event.payload)))
            .bind(payload_bytes)
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                IngestError::retryable(anyhow!(error).context("persist analytics ingest receipt"))
            })?;
            sqlx::query(
                "INSERT INTO cvm_analytics.analytics_module_ingest_work
                    (consumer_id, source_instance_id, generation, change_id, event_id,
                     work_kind, state, attempts, last_error_code)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, NULL)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&self.binding.consumer_id)
            .bind(&self.binding.source_instance_id)
            .bind(event.generation)
            .bind(event.change_id)
            .bind(&event.event_id)
            .bind(work_kind.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                IngestError::retryable(anyhow!(error).context("persist analytics ingest work item"))
            })?;
        }

        let last_coverage_revision = inserted
            .last()
            .map(|event| event.coverage_revision)
            .unwrap_or(consumer.coverage_revision);
        let updated = sqlx::query(
            "UPDATE cvm_analytics.analytics_module_consumers
             SET checkpoint = $1, coverage_revision = $2, updated_at = NOW()
             WHERE consumer_id = $3 AND module_id = $4
               AND owner_epoch = $5 AND checkpoint = $6",
        )
        .bind(prefix.through_change_id())
        .bind(last_coverage_revision)
        .bind(&self.binding.consumer_id)
        .bind(self.binding.module_id.as_str())
        .bind(self.binding.owner_epoch)
        .bind(consumer.checkpoint)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            IngestError::retryable(anyhow!(error).context("advance analytics consumer checkpoint"))
        })?;
        ingest_ensure!(
            updated.rows_affected() == 1,
            "analytics consumer checkpoint changed while receiving the source prefix"
        );
        transaction.commit().await.map_err(|error| {
            IngestError::ambiguous_commit(anyhow!(error).context(
                "analytics receive C commit outcome is ambiguous; retry from the durable receipt/checkpoint",
            ))
        })?;
        self.binding.checkpoint = prefix.through_change_id();
        self.binding.coverage_revision = last_coverage_revision;
        Ok(ReceiveReceipt {
            first_change_id: prefix.first_change_id(),
            through_change_id: prefix.through_change_id(),
            generation: self.binding.generation,
            coverage_revision: last_coverage_revision,
            inserted_events: inserted.len(),
            duplicate_events: duplicates,
        })
    }

    pub(crate) async fn consume_once(
        &mut self,
        source_pool: &Pool<Sqlite>,
    ) -> IngestResult<ConsumeStatus> {
        let handlers = IngestHandlerRegistry::default();
        self.consume_once_with_handlers(source_pool, &handlers)
            .await
    }

    pub(crate) async fn consume_once_with_handlers(
        &mut self,
        source_pool: &Pool<Sqlite>,
        handlers: &IngestHandlerRegistry,
    ) -> IngestResult<ConsumeStatus> {
        self.refresh_binding().await?;
        let source_state = load_source_state(source_pool)
            .await
            .map_err(IngestError::retryable)?;
        ingest_ensure!(
            source_state.generation == self.binding.generation,
            "analytics source generation changed; rebuild the consumer before continuing"
        );
        let source_coverage = load_source_coverage(source_pool, &self.binding.consumer_id)
            .await
            .map_err(IngestError::retryable)?;
        if let Some(coverage) = &source_coverage {
            ingest_ensure!(
                coverage.generation == source_state.generation,
                "analytics source acknowledgement belongs to a stale generation"
            );
            ingest_ensure!(
                coverage.covered_through_change_id <= self.binding.checkpoint,
                "source acknowledgement is ahead of the PG consumer checkpoint"
            );
        }
        if self.pending_ack.is_none()
            && source_coverage
                .as_ref()
                .is_none_or(|coverage| coverage.covered_through_change_id < self.binding.checkpoint)
        {
            let covered_through = source_coverage
                .as_ref()
                .map(|coverage| coverage.covered_through_change_id)
                .unwrap_or(source_state.retained_from_change_id.saturating_sub(1));
            self.pending_ack = Some(PendingAck {
                generation: source_state.generation,
                from_change_id: covered_through
                    .saturating_add(1)
                    .max(source_state.retained_from_change_id),
                through_change_id: self.binding.checkpoint,
            });
        }

        if self.pending_ack.is_some() {
            match self.try_ack(source_pool).await {
                Ok(()) => self.pending_ack = None,
                Err(reason) => {
                    return Ok(ConsumeStatus::AckPending {
                        checkpoint: self.binding.checkpoint,
                        through_change_id: self
                            .pending_ack
                            .as_ref()
                            .map(|pending| pending.through_change_id)
                            .unwrap_or(self.binding.checkpoint),
                        reason,
                    });
                }
            }
        }

        if self.binding.checkpoint.saturating_add(1) < source_state.retained_from_change_id {
            return Err(IngestError::blocked(anyhow!(
                "analytics source retention gap starts at {}, but consumer requires {}",
                source_state.retained_from_change_id,
                self.binding.checkpoint.saturating_add(1)
            )));
        }
        let rows = load_source_rows(source_pool, self.binding.checkpoint)
            .await
            .map_err(IngestError::retryable)?;
        if rows.is_empty() {
            return Ok(ConsumeStatus::Idle {
                checkpoint: self.binding.checkpoint,
            });
        }
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(decode_source_row(&row).map_err(IngestError::blocked)?);
        }
        let receipt = self.receive_batch_with_handlers(&events, handlers).await?;
        self.pending_ack = Some(PendingAck {
            generation: source_state.generation,
            from_change_id: receipt.first_change_id,
            through_change_id: receipt.through_change_id,
        });
        match self.try_ack(source_pool).await {
            Ok(()) => {
                self.pending_ack = None;
                Ok(ConsumeStatus::Progressed(receipt))
            }
            Err(reason) => Ok(ConsumeStatus::AckPending {
                checkpoint: self.binding.checkpoint,
                through_change_id: receipt.through_change_id,
                reason,
            }),
        }
    }

    /// Begin a later projection/history write on this actor's already-locked session. The caller
    /// must perform all durable target writes in the returned transaction, then commit it before
    /// reporting success. It must not acquire a second pool connection or leave work in memory.
    pub(crate) async fn begin_writer_transaction(
        &mut self,
    ) -> IngestResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        self.connection.begin().await.map_err(|error| {
            IngestError::retryable(anyhow!(error).context("begin analytics writer transaction"))
        })
    }

    async fn try_ack(&self, source_pool: &Pool<Sqlite>) -> std::result::Result<(), String> {
        let Some(pending) = &self.pending_ack else {
            return Ok(());
        };
        let source_state = load_source_state(source_pool)
            .await
            .map_err(|error| format!("load source state for acknowledgement: {error:#}"))?;
        if source_state.generation != pending.generation {
            return Err(format!(
                "source generation changed from {} to {} before acknowledgement",
                pending.generation, source_state.generation
            ));
        }
        let current_coverage = load_source_coverage(source_pool, &self.binding.consumer_id)
            .await
            .map_err(|error| format!("load source acknowledgement coverage: {error:#}"))?;
        if let Some(coverage) = current_coverage {
            if coverage.generation != pending.generation {
                return Err(
                    "source acknowledgement coverage belongs to a stale generation".to_string(),
                );
            }
            if coverage.covered_through_change_id >= pending.through_change_id {
                return Ok(());
            }
        }
        crate::analytics_changes::claim_analytics_change_coverage(
            source_pool,
            &self.binding.consumer_id,
            source_state.generation,
            source_state.coverage_revision,
            pending.from_change_id,
            pending.through_change_id,
        )
        .await
        .map(|_| ())
        .map_err(|error| format!("source ack A remains pending after PG receive C: {error:#}"))
    }

    pub(crate) async fn drain(&mut self, source_pool: &Pool<Sqlite>) -> IngestResult<usize> {
        let handlers = IngestHandlerRegistry::default();
        self.drain_with_handlers(source_pool, &handlers).await
    }

    pub(crate) async fn drain_with_handlers(
        &mut self,
        source_pool: &Pool<Sqlite>,
        handlers: &IngestHandlerRegistry,
    ) -> IngestResult<usize> {
        let mut progressed_batches = 0usize;
        for _ in 0..ANALYTICS_INGEST_MAX_BATCHES_PER_DRAIN {
            match self
                .consume_once_with_handlers(source_pool, handlers)
                .await?
            {
                ConsumeStatus::Idle { .. } => return Ok(progressed_batches),
                ConsumeStatus::Progressed(_) => {
                    progressed_batches = progressed_batches.saturating_add(1)
                }
                ConsumeStatus::AckPending { reason, .. } => {
                    return Err(IngestError::retryable(anyhow!(reason)));
                }
            }
        }
        Err(IngestError::blocked(anyhow!(
            "analytics ingest drain exceeded the bounded batch limit"
        )))
    }

    pub(crate) async fn run_continuously(
        &mut self,
        source_pool: &Pool<Sqlite>,
        shutdown: &CancellationToken,
        wake: &Notify,
    ) -> IngestResult<()> {
        let handlers = IngestHandlerRegistry::default();
        self.run_continuously_with_handlers(source_pool, shutdown, wake, &handlers)
            .await
    }

    pub(crate) async fn run_continuously_with_handlers(
        &mut self,
        source_pool: &Pool<Sqlite>,
        shutdown: &CancellationToken,
        wake: &Notify,
        handlers: &IngestHandlerRegistry,
    ) -> IngestResult<()> {
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            match self
                .consume_once_with_handlers(source_pool, handlers)
                .await?
            {
                ConsumeStatus::Progressed(_) => continue,
                ConsumeStatus::AckPending { .. } | ConsumeStatus::Idle { .. } => {
                    tokio::select! {
                        _ = shutdown.cancelled() => return Ok(()),
                        _ = wake.notified() => {},
                        _ = tokio::time::sleep(ANALYTICS_INGEST_IDLE_CADENCE) => {},
                    }
                }
            }
        }
    }

    pub(crate) async fn close(mut self) -> IngestResult<()> {
        let unlock = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
            .bind(ANALYTICS_INGEST_LOCK_KEY)
            .fetch_one(&mut self.connection)
            .await
            .map_err(|error| {
                IngestError::retryable(
                    anyhow!(error).context("release analytics ingest writer lock"),
                )
            })?;
        ingest_ensure!(unlock, "analytics ingest writer lock was not held");
        self.connection.close().await.map_err(|error| {
            IngestError::retryable(anyhow!(error).context("close analytics ingest connection"))
        })
    }
}

/// Reacquire the locked actor after a transient PG failure. Semantic/source errors are returned
/// as blocked; a lost connection or an occupied lock is retried with bounded exponential delay.
pub(crate) async fn supervise_continuously(
    pool: &PgPool,
    source_pool: &Pool<Sqlite>,
    binding: SourceBinding,
    shutdown: &CancellationToken,
    wake: &Notify,
) -> IngestResult<()> {
    let handlers = IngestHandlerRegistry::default();
    supervise_continuously_with_handlers(pool, source_pool, binding, shutdown, wake, &handlers)
        .await
}

pub(crate) async fn supervise_continuously_with_handlers(
    pool: &PgPool,
    source_pool: &Pool<Sqlite>,
    binding: SourceBinding,
    shutdown: &CancellationToken,
    wake: &Notify,
    handlers: &IngestHandlerRegistry,
) -> IngestResult<()> {
    let mut delay = ANALYTICS_INGEST_RETRY_INITIAL;
    loop {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        let mut actor = match PgIngestActor::acquire(pool, binding.clone()).await {
            Ok(actor) => actor,
            Err(error) if error.is_retryable() => {
                if !wait_for_retry(shutdown, delay).await {
                    return Ok(());
                }
                delay = std::cmp::min(delay.saturating_mul(2), ANALYTICS_INGEST_RETRY_MAX);
                continue;
            }
            Err(error) => return Err(error),
        };
        delay = ANALYTICS_INGEST_RETRY_INITIAL;
        match actor
            .run_continuously_with_handlers(source_pool, shutdown, wake, handlers)
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) if error.is_retryable() => {
                let _ = actor.close().await;
                if !wait_for_retry(shutdown, delay).await {
                    return Ok(());
                }
                delay = std::cmp::min(delay.saturating_mul(2), ANALYTICS_INGEST_RETRY_MAX);
            }
            Err(error) => {
                let _ = actor.close().await;
                return Err(error);
            }
        }
    }
}

async fn wait_for_retry(shutdown: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

/// Apply the C3 receipt schema through the owner-only migration path.
///
/// The C2 ledger must already exist. This keeps module DDL, ingest DDL, role reconciliation and
/// actor startup ordered and independently auditable: legacy schema -> module migration -> this
/// migration -> actor. Runtime maintenance credentials never invoke this function.
pub(crate) async fn apply_analytics_ingest_migration(
    connection: &mut PgConnection,
) -> Result<AnalyticsIngestMigrationResult> {
    let mut transaction = connection
        .begin()
        .await
        .context("begin analytics ingest migration transaction")?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ANALYTICS_INGEST_MIGRATION_LOCK_KEY)
        .execute(&mut *transaction)
        .await
        .context("acquire analytics ingest migration lock")?;
    let module_schema_ready = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = 'analytics-module'
           AND step_order = 1
           AND state = 'applied'",
    )
    .fetch_one(&mut *transaction)
    .await
    .context("inspect prerequisite analytics module migration")?;
    ensure!(
        module_schema_ready == 1,
        "C3 ingest migration requires the applied C2 analytics module schema"
    );

    let checksum = format!(
        "{:x}",
        Sha256::digest(ANALYTICS_INGEST_SCHEMA_SQL.as_bytes())
    );
    let existing = sqlx::query_as::<_, (i64, String, String, String)>(
        "SELECT schema_version, step_id, checksum, state
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1 AND step_order = 1
         FOR UPDATE",
    )
    .bind(ANALYTICS_INGEST_DDL_COMPONENT)
    .fetch_optional(&mut *transaction)
    .await
    .context("read analytics ingest migration ledger")?;
    let newer = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1
           AND (schema_version > $2 OR step_order > 1 OR min_reader_version > $3)",
    )
    .bind(ANALYTICS_INGEST_DDL_COMPONENT)
    .bind(ANALYTICS_INGEST_SCHEMA_VERSION)
    .bind(ANALYTICS_INGEST_READER_VERSION)
    .fetch_one(&mut *transaction)
    .await
    .context("inspect analytics ingest migration version")?;
    ensure!(
        newer == 0,
        "analytics ingest migration is newer than this reader"
    );

    let mut recovered_steps = 0usize;
    let existing_state = if let Some((schema_version, step_id, stored_checksum, state)) = existing {
        ensure!(
            schema_version <= ANALYTICS_INGEST_SCHEMA_VERSION,
            "analytics ingest migration schema is newer than this reader"
        );
        ensure!(
            step_id == ANALYTICS_INGEST_DDL_STEP,
            "analytics ingest migration step identity changed"
        );
        ensure!(
            stored_checksum == checksum,
            "analytics ingest migration checksum mismatch"
        );
        ensure!(
            matches!(state.as_str(), "applying" | "applied" | "failed"),
            "analytics ingest migration ledger state is invalid"
        );
        Some(state)
    } else {
        None
    };

    if existing_state.as_deref() != Some("applied") {
        sqlx::query(
            "INSERT INTO cvm_analytics.analytics_module_ddl_ledger
                (component, schema_version, step_order, step_id, checksum, state,
                 min_reader_version, requires_bootstrap, error_code, applied_at)
             VALUES ($1, $2, 1, $3, $4, 'applying', $5, TRUE, NULL, NULL)
             ON CONFLICT (component, step_order) DO UPDATE SET
                 schema_version = EXCLUDED.schema_version,
                 step_id = EXCLUDED.step_id,
                 checksum = EXCLUDED.checksum,
                 state = 'applying',
                 min_reader_version = EXCLUDED.min_reader_version,
                 requires_bootstrap = EXCLUDED.requires_bootstrap,
                 error_code = NULL,
                 applied_at = NULL",
        )
        .bind(ANALYTICS_INGEST_DDL_COMPONENT)
        .bind(ANALYTICS_INGEST_SCHEMA_VERSION)
        .bind(ANALYTICS_INGEST_DDL_STEP)
        .bind(&checksum)
        .bind(ANALYTICS_INGEST_READER_VERSION)
        .execute(&mut *transaction)
        .await
        .context("write analytics ingest migration applying state")?;
        recovered_steps = 1;
    }

    // A savepoint lets us persist a failed state while retaining the outer transaction for a
    // clean retry. An interrupted process rolls the whole transaction back, leaving no false
    // applied marker; a later invocation safely re-enters the applying step.
    let mut ddl = transaction
        .begin()
        .await
        .context("begin analytics ingest DDL savepoint")?;
    if let Err(error) = sqlx::raw_sql(ANALYTICS_INGEST_SCHEMA_SQL)
        .execute(&mut *ddl)
        .await
    {
        drop(ddl);
        sqlx::query(
            "UPDATE cvm_analytics.analytics_module_ddl_ledger
             SET state = 'failed', error_code = 'analytics_ingest_ddl_failed'
             WHERE component = $1 AND step_order = 1",
        )
        .bind(ANALYTICS_INGEST_DDL_COMPONENT)
        .execute(&mut *transaction)
        .await
        .context("record analytics ingest migration failure")?;
        transaction
            .commit()
            .await
            .context("commit analytics ingest migration failure state")?;
        return Err(error).context("apply analytics ingest receipt schema");
    }
    ddl.commit()
        .await
        .context("commit analytics ingest DDL savepoint")?;

    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_ddl_ledger
         SET schema_version = $1, checksum = $2, state = 'applied',
             error_code = NULL, applied_at = NOW()
         WHERE component = $3 AND step_order = 1",
    )
    .bind(ANALYTICS_INGEST_SCHEMA_VERSION)
    .bind(&checksum)
    .bind(ANALYTICS_INGEST_DDL_COMPONENT)
    .execute(&mut *transaction)
    .await
    .context("write analytics ingest migration applied state")?;
    transaction
        .commit()
        .await
        .context("commit analytics ingest migration")?;
    Ok(AnalyticsIngestMigrationResult {
        component: ANALYTICS_INGEST_DDL_COMPONENT,
        version: ANALYTICS_INGEST_SCHEMA_VERSION,
        checksum,
        recovered_steps,
    })
}

fn ensure_binding_consumer(binding: &SourceBinding, row: &ConsumerRow) -> Result<()> {
    ensure!(
        row.source_instance_id == binding.source_instance_id,
        "analytics consumer is bound to a different source instance"
    );
    ensure!(
        row.owner_epoch == binding.owner_epoch,
        "analytics consumer owner epoch was fenced"
    );
    ensure!(
        matches!(row.state.as_str(), "building" | "active"),
        "analytics consumer is not writable in state {}",
        row.state
    );
    Ok(())
}

fn ensure_receipt_matches(row: &ReceiptRow, event: &SourceEnvelope) -> Result<()> {
    ensure!(
        row.event_id == event.event_id,
        "analytics receipt event identity mismatch"
    );
    ensure!(
        row.generation == event.generation,
        "analytics receipt generation mismatch"
    );
    ensure!(
        row.change_id == event.change_id,
        "analytics receipt change id mismatch"
    );
    ensure!(
        row.coverage_revision == event.coverage_revision,
        "analytics receipt coverage revision mismatch"
    );
    ensure!(
        row.entity_kind == event.identity.entity_kind,
        "analytics receipt entity kind mismatch"
    );
    ensure!(
        row.entity_key == event.identity.entity_key,
        "analytics receipt entity key mismatch"
    );
    ensure!(
        row.operation_kind == event.operation.as_str(),
        "analytics receipt operation mismatch"
    );
    ensure!(
        row.payload_digest == event.payload_digest,
        "analytics receipt payload digest mismatch"
    );
    ensure!(
        row.payload.0 == canonical_json(&event.payload),
        "analytics receipt payload content mismatch"
    );
    Ok(())
}

async fn load_source_state(pool: &Pool<Sqlite>) -> Result<SourceStateRow> {
    sqlx::query_as::<_, SourceStateRow>(
        "SELECT generation, coverage_revision, retained_from_change_id
         FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .context("load analytics source state")
}

async fn load_source_coverage(
    pool: &Pool<Sqlite>,
    consumer_id: &str,
) -> Result<Option<SourceCoverageRow>> {
    sqlx::query_as::<_, SourceCoverageRow>(
        "SELECT generation, covered_through_change_id
         FROM analytics_change_coverage WHERE replica_id = ?1",
    )
    .bind(consumer_id)
    .fetch_optional(pool)
    .await
    .context("load analytics source acknowledgement coverage")
}

async fn load_source_rows(pool: &Pool<Sqlite>, checkpoint: i64) -> Result<Vec<SourceJournalRow>> {
    sqlx::query_as::<_, SourceJournalRow>(
        "SELECT change_id, event_version, fact_date_semantics_version,
                source_instance_id, event_id, operation_kind, fact_semantics_version,
                stats_semantics_version, payload_digest, event_kind, entity_kind,
                entity_key, generation, coverage_revision, occurred_at, fact_json
         FROM analytics_change_events
         WHERE change_id > ?1
         ORDER BY change_id
         LIMIT ?2",
    )
    .bind(checkpoint)
    .bind(i64::try_from(ANALYTICS_INGEST_MAX_EVENTS_PER_BATCH)?)
    .fetch_all(pool)
    .await
    .context("load bounded analytics source prefix")
}

fn decode_source_row(row: &SourceJournalRow) -> Result<SourceEnvelope> {
    ensure!(
        row.event_version > 0,
        "unknown analytics source journal version at change {}",
        row.change_id
    );
    let payload = serde_json::from_str::<Value>(&row.fact_json).with_context(|| {
        format!(
            "decode analytics source payload at change {}",
            row.change_id
        )
    })?;
    ensure!(
        payload.is_object(),
        "analytics source payload must be an object at change {}",
        row.change_id
    );
    let source_instance_id = if row.source_instance_id.trim().is_empty() {
        LEGACY_SOURCE_INSTANCE_ID.to_string()
    } else {
        row.source_instance_id.clone()
    };
    let event_id = if row.event_id.trim().is_empty() {
        format!("legacy:{}", row.change_id)
    } else {
        row.event_id.clone()
    };
    let operation_kind = if row.operation_kind == "legacy" {
        match row.event_kind.as_str() {
            "upsert" => "upsert",
            "tombstone" => "logical_revoke",
            "coverage" => "coverage",
            other => bail!(
                "unknown analytics source event kind {other} at change {}",
                row.change_id
            ),
        }
    } else {
        row.operation_kind.as_str()
    };
    let operation = match operation_kind {
        "upsert" => SourceOperation::Upsert,
        "correction" => SourceOperation::Correction,
        "logical_revoke" => SourceOperation::LogicalRevoke,
        "archive_relocation" => SourceOperation::ArchiveRelocation,
        "retention_retirement" => SourceOperation::RetentionRetirement,
        "coverage" | "archive_coverage" => SourceOperation::Coverage,
        other => bail!(
            "unknown analytics source operation {other} at change {}",
            row.change_id
        ),
    };
    let identity = SourceIdentity::new(
        source_instance_id.clone(),
        row.entity_kind.clone(),
        row.entity_key.clone(),
    )?;
    let computed_digest = digest_json(&payload)?;
    let payload_digest =
        if row.payload_digest.trim().is_empty() || row.payload_digest.starts_with("legacy:") {
            computed_digest
        } else {
            row.payload_digest.clone()
        };
    Ok(SourceEnvelope {
        envelope_version: if event_id.starts_with("legacy:") {
            1
        } else {
            SOURCE_ENVELOPE_VERSION
        },
        source_instance_id,
        event_id,
        generation: row.generation,
        change_id: row.change_id,
        coverage_revision: row.coverage_revision,
        fact_semantics_version: row.fact_semantics_version.max(row.event_version),
        stats_semantics_version: row
            .stats_semantics_version
            .max(row.fact_date_semantics_version),
        identity,
        operation,
        occurred_at: row.occurred_at.clone(),
        payload: canonical_json(&payload),
        payload_digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> SourceBinding {
        SourceBinding::new(
            AnalyticsModuleId::Invocation,
            "invocation",
            "source-a",
            7,
            3,
            0,
            0,
        )
        .expect("valid binding")
    }

    fn event(change_id: i64, event_id: &str, value: i64) -> SourceEnvelope {
        let payload = serde_json::json!({"value": value});
        SourceEnvelope {
            envelope_version: SOURCE_ENVELOPE_VERSION,
            source_instance_id: "source-a".to_string(),
            event_id: event_id.to_string(),
            generation: 7,
            change_id,
            coverage_revision: change_id,
            fact_semantics_version: super::super::SOURCE_FACT_SEMANTICS_VERSION,
            stats_semantics_version: super::super::SOURCE_STATS_SEMANTICS_VERSION,
            identity: SourceIdentity::new("source-a", "invocation", format!("row-{change_id}"))
                .unwrap(),
            operation: SourceOperation::Upsert,
            occurred_at: "2026-11-06T00:00:00Z".to_string(),
            payload: payload.clone(),
            payload_digest: digest_json(&payload).unwrap(),
        }
    }

    #[test]
    fn complete_prefix_rejects_gaps_and_out_of_order_inputs() {
        let binding = binding();
        let first = event(1, "event-1", 1);
        let third = event(3, "event-3", 3);
        assert!(validate_complete_prefix(&binding, 0, &[first.clone(), third]).is_err());
        assert!(validate_complete_prefix(&binding, 0, &[event(2, "event-2", 2), first]).is_err());
    }

    #[test]
    fn exact_duplicates_are_replayed_without_duplicate_receipts() {
        let binding = binding();
        let first = event(1, "event-1", 1);
        let second = event(2, "event-2", 2);
        let prefix =
            validate_complete_prefix(&binding, 0, &[first.clone(), first, second]).unwrap();
        assert_eq!(prefix.events().len(), 2);
        assert_eq!(prefix.through_change_id(), 2);
    }

    #[test]
    fn same_event_identity_with_different_content_is_blocked() {
        let binding = binding();
        let first = event(1, "event-1", 1);
        let conflicting = event(1, "event-1", 2);
        let error = validate_complete_prefix(&binding, 0, &[first, conflicting]).unwrap_err();
        assert!(error.to_string().contains("conflicting content"));
    }

    #[test]
    fn unknown_version_and_digest_mismatch_are_blocked_before_pg_receive() {
        let binding = binding();
        let mut unknown = event(1, "event-1", 1);
        unknown.envelope_version = SOURCE_ENVELOPE_VERSION + 1;
        assert!(validate_complete_prefix(&binding, 0, &[unknown]).is_err());

        let mut corrupt = event(1, "event-1", 1);
        corrupt.payload_digest = "sha256:wrong".to_string();
        assert!(validate_complete_prefix(&binding, 0, &[corrupt]).is_err());
    }

    #[test]
    fn archive_and_coverage_events_fail_closed_without_authenticated_staging() {
        let mut archive = event(1, "event-1", 1);
        archive.operation = SourceOperation::Coverage;
        let error = ensure_receivable_operation(&archive).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no C3 authenticated staging handler")
        );
    }

    #[test]
    fn replay_after_checkpoint_is_allowed_but_mixed_overlap_is_blocked() {
        let binding = binding();
        let first = event(1, "event-1", 1);
        let second = event(2, "event-2", 2);
        assert!(validate_complete_prefix(&binding, 2, &[first.clone(), second.clone()]).is_ok());
        assert!(validate_complete_prefix(&binding, 1, &[first, second]).is_err());
    }
}

#[cfg(test)]
#[path = "ingest/postgres.rs"]
mod postgres_tests;
