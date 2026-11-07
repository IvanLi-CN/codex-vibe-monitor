//! Unified analytics reads over the SQLite source journal and the PostgreSQL projection.
//!
//! The query boundary is deliberately narrower than either storage implementation: a response
//! is admitted against one immutable publication identity, one source prefix, and one coverage
//! proof. SQLite and PostgreSQL adapters therefore share the same tail folding, cache identity,
//! and unavailable semantics instead of each growing a subtly different read contract.

use super::history::coverage::{
    CoverageClaim, CoverageKey, CoverageKind, CoverageLedger, CoverageRange, SeedCapability,
};
use super::projection::{self, ProjectionFact, ProjectionModels, PublicationToken};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Pool, Postgres, Sqlite, Transaction, types::Json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAX_QUERY_NAME_BYTES: usize = 128;
const MAX_QUERY_KEY_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AnalyticsQueryBackend {
    Sqlite,
    Postgres,
}

impl AnalyticsQueryBackend {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

/// Parameters which must participate in a response cache identity. The storage backend is
/// intentionally not a parameter here; it is part of `QueryIdentity`, so a backend switch can
/// never reuse a response produced by the other store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsQueryRequest {
    pub(crate) consumer_id: String,
    pub(crate) query_name: String,
    pub(crate) range: String,
    pub(crate) dimension: String,
    pub(crate) series_key: String,
    pub(crate) timezone: String,
    pub(crate) coverage_key: CoverageKey,
    pub(crate) coverage_range: CoverageRange,
    pub(crate) required_seed_capability: Option<SeedCapability>,
    pub(crate) now_ms: i64,
    pub(crate) max_freshness_age_ms: Option<i64>,
}

impl AnalyticsQueryRequest {
    pub(crate) fn validate(&self) -> QueryResult<()> {
        for (value, max_bytes) in [
            (&self.consumer_id, MAX_QUERY_NAME_BYTES),
            (&self.query_name, MAX_QUERY_NAME_BYTES),
            (&self.range, MAX_QUERY_NAME_BYTES),
            (&self.dimension, MAX_QUERY_NAME_BYTES),
            (&self.timezone, MAX_QUERY_NAME_BYTES),
        ] {
            if value.trim().is_empty() || value.len() > max_bytes {
                return Err(QueryError::invalid_query());
            }
        }
        if self.series_key.len() > MAX_QUERY_KEY_BYTES {
            return Err(QueryError::invalid_query());
        }
        if !matches!(self.range.as_str(), "7d" | "30d" | "180d" | "365d")
            || !matches!(self.dimension.as_str(), "overall" | "model" | "upstream")
            || self.now_ms < 0
            || self.max_freshness_age_ms.is_some_and(|age| age < 0)
        {
            return Err(QueryError::invalid_query());
        }
        self.coverage_key
            .validate()
            .map_err(|_| QueryError::invalid_query())?;
        CoverageRange::new(
            self.coverage_range.start.clone(),
            self.coverage_range.end.clone(),
        )
        .map_err(|_| QueryError::invalid_query())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryIdentity {
    pub(crate) backend: AnalyticsQueryBackend,
    pub(crate) consumer_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) build_id: String,
    pub(crate) publication_id: String,
    pub(crate) projection_version: i64,
    pub(crate) published_through_change_id: i64,
    pub(crate) checkpoint: i64,
    pub(crate) coverage_revision: i64,
}

impl QueryIdentity {
    fn from_publication(
        backend: AnalyticsQueryBackend,
        token: &PublicationToken,
        source_instance_id: String,
        generation: i64,
        checkpoint: i64,
    ) -> QueryResult<Self> {
        if token.consumer_id.is_empty()
            || token.build_id.is_empty()
            || token.publication_id.is_empty()
            || source_instance_id.is_empty()
            || generation < 0
            || checkpoint < token.through_change_id
            || token.owner_epoch < 0
            || token.coverage_revision < 0
            || token.projection_version <= 0
        {
            return Err(QueryError::snapshot_inconsistent());
        }
        Ok(Self {
            backend,
            consumer_id: token.consumer_id.clone(),
            source_instance_id,
            generation,
            owner_epoch: token.owner_epoch,
            build_id: token.build_id.clone(),
            publication_id: token.publication_id.clone(),
            projection_version: token.projection_version,
            published_through_change_id: token.through_change_id,
            checkpoint,
            coverage_revision: token.coverage_revision,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryCacheKey {
    pub(crate) digest: String,
    pub(crate) identity: QueryIdentity,
}

impl QueryCacheKey {
    pub(crate) fn for_request(
        request: &AnalyticsQueryRequest,
        identity: &QueryIdentity,
    ) -> QueryResult<Self> {
        request.validate()?;
        let encoded = serde_json::to_vec(&(request, identity))
            .map_err(|_| QueryError::internal_contract())?;
        let digest = format!("{:x}", Sha256::digest(encoded));
        Ok(Self {
            digest,
            identity: identity.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FreshnessState {
    Fresh,
    Stale,
    Unknown,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FreshnessEvidence {
    pub(crate) observed_at_ms: i64,
    pub(crate) source_updated_at_ms: Option<i64>,
    pub(crate) source_checkpoint: i64,
    pub(crate) effective_through_change_id: i64,
}

impl FreshnessEvidence {
    pub(crate) fn assess(&self, max_age_ms: Option<i64>) -> FreshnessState {
        if self.observed_at_ms < 0
            || self.source_checkpoint < 0
            || self.effective_through_change_id < 0
            || self.effective_through_change_id > self.source_checkpoint
        {
            return FreshnessState::Unavailable;
        }
        if self.effective_through_change_id < self.source_checkpoint {
            return FreshnessState::Stale;
        }
        let Some(source_updated_at_ms) = self.source_updated_at_ms else {
            return FreshnessState::Unknown;
        };
        if source_updated_at_ms > self.observed_at_ms {
            return FreshnessState::Unknown;
        }
        let Some(max_age_ms) = max_age_ms else {
            return FreshnessState::Fresh;
        };
        if self.observed_at_ms.saturating_sub(source_updated_at_ms) > max_age_ms {
            FreshnessState::Stale
        } else {
            FreshnessState::Fresh
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverageAdmission {
    pub(crate) kind: CoverageKind,
    pub(crate) proof_id: Option<String>,
    pub(crate) capabilities: BTreeSet<SeedCapability>,
}

pub(crate) fn admit_coverage(
    ledger: &CoverageLedger,
    key: &CoverageKey,
    range: &CoverageRange,
    required_capability: Option<SeedCapability>,
) -> QueryResult<CoverageAdmission> {
    if !ledger.is_covered(key, range, required_capability) {
        return Err(QueryError::coverage_unavailable());
    }
    let claim = ledger
        .claims()
        .iter()
        .find(|claim| {
            claim.key == *key
                && claim.kind != CoverageKind::Unavailable
                && claim.range.contains(range)
                && match (claim.kind, required_capability) {
                    (CoverageKind::ExactFacts, _) => true,
                    (CoverageKind::CertifiedSeed, Some(capability)) => {
                        claim.capabilities.contains(&capability)
                    }
                    (CoverageKind::CertifiedSeed, None) => true,
                    (CoverageKind::Unavailable, _) => false,
                }
        })
        .ok_or_else(QueryError::coverage_unavailable)?;
    Ok(CoverageAdmission {
        kind: claim.kind,
        proof_id: claim.proof_id.clone(),
        capabilities: claim.capabilities.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTailRow {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) operation: String,
    pub(crate) occurred_at: String,
    pub(crate) payload_digest: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTail {
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) rows: Vec<RuntimeTailRow>,
}

impl RuntimeTail {
    pub(crate) fn empty(at: i64) -> Self {
        Self {
            from_change_id: at.saturating_add(1),
            through_change_id: at,
            rows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsQueryResponse {
    pub(crate) backend: AnalyticsQueryBackend,
    pub(crate) identity: QueryIdentity,
    pub(crate) facts: Vec<ProjectionFact>,
    pub(crate) models: ProjectionModels,
    pub(crate) tail: RuntimeTail,
    pub(crate) coverage: CoverageAdmission,
    pub(crate) freshness: FreshnessState,
    pub(crate) cache_key: QueryCacheKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryErrorCode {
    InvalidQuery,
    Unavailable,
    PublicationUnavailable,
    CoverageUnavailable,
    SnapshotInconsistent,
    TailInvalid,
    InternalContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryError {
    pub(crate) code: QueryErrorCode,
    pub(crate) message: &'static str,
}

impl QueryError {
    const fn invalid_query() -> Self {
        Self {
            code: QueryErrorCode::InvalidQuery,
            message: "analytics query parameters are invalid",
        }
    }

    const fn unavailable() -> Self {
        Self {
            code: QueryErrorCode::Unavailable,
            message: "analytics backend is unavailable",
        }
    }

    const fn publication_unavailable() -> Self {
        Self {
            code: QueryErrorCode::PublicationUnavailable,
            message: "analytics publication is unavailable",
        }
    }

    const fn coverage_unavailable() -> Self {
        Self {
            code: QueryErrorCode::CoverageUnavailable,
            message: "requested analytics coverage is unavailable",
        }
    }

    const fn snapshot_inconsistent() -> Self {
        Self {
            code: QueryErrorCode::SnapshotInconsistent,
            message: "analytics snapshot identity is inconsistent",
        }
    }

    const fn tail_invalid() -> Self {
        Self {
            code: QueryErrorCode::TailInvalid,
            message: "analytics runtime tail is invalid",
        }
    }

    const fn internal_contract() -> Self {
        Self {
            code: QueryErrorCode::InternalContract,
            message: "analytics query contract could not be encoded",
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for QueryError {}

type QueryResult<T> = std::result::Result<T, QueryError>;

/// Fold a bounded receipt suffix over the immutable publication facts. The suffix is required to
/// be a complete `(P, C]` prefix. A missing receipt, duplicate change, stale source generation,
/// or mixed owner epoch therefore fails closed instead of producing a partial chart.
pub(crate) fn compose_runtime_tail(
    identity: &QueryIdentity,
    base_facts: &[ProjectionFact],
    tail: RuntimeTail,
) -> QueryResult<(Vec<ProjectionFact>, ProjectionModels)> {
    if tail.from_change_id != identity.published_through_change_id.saturating_add(1)
        || tail.through_change_id != identity.checkpoint
        || (tail.rows.is_empty() && tail.through_change_id != identity.published_through_change_id)
    {
        return Err(QueryError::tail_invalid());
    }
    if tail.through_change_id < identity.published_through_change_id || tail.through_change_id < 0 {
        return Err(QueryError::tail_invalid());
    }

    let mut facts = BTreeMap::<(String, String), ProjectionFact>::new();
    for fact in base_facts {
        let key = (fact.entity_kind.clone(), fact.entity_key.clone());
        if facts.insert(key, fact.clone()).is_some() {
            return Err(QueryError::snapshot_inconsistent());
        }
    }
    let mut expected_change = identity.published_through_change_id.saturating_add(1);
    let mut seen_changes = BTreeSet::new();
    let mut seen_payloads = BTreeMap::<(String, String), String>::new();
    for row in &tail.rows {
        if row.source_instance_id != identity.source_instance_id
            || row.generation != identity.generation
            || row.owner_epoch != identity.owner_epoch
            || row.change_id != expected_change
            || row.change_id <= identity.published_through_change_id
            || row.change_id > identity.checkpoint
            || row.coverage_revision > identity.coverage_revision
            || row.entity_kind.trim().is_empty()
            || row.entity_key.trim().is_empty()
            || row.occurred_at.trim().is_empty()
            || row.payload_digest.trim().is_empty()
            || !row.payload.is_object()
            || !seen_changes.insert(row.change_id)
        {
            return Err(QueryError::tail_invalid());
        }
        let digest = super::digest_json(&row.payload).map_err(|_| QueryError::tail_invalid())?;
        if digest != row.payload_digest {
            return Err(QueryError::tail_invalid());
        }
        let entity = (row.entity_kind.clone(), row.entity_key.clone());
        if let Some(previous_digest) =
            seen_payloads.insert(entity.clone(), row.payload_digest.clone())
            && previous_digest == row.payload_digest
        {
            return Err(QueryError::tail_invalid());
        }
        match row.operation.as_str() {
            "upsert" | "correction" => {
                let fact = normalize_tail_fact(&row.entity_kind, &row.entity_key, &row.payload)?;
                facts.insert(entity, fact);
            }
            "logical_revoke" => {
                facts.remove(&entity);
            }
            _ => return Err(QueryError::tail_invalid()),
        }
        expected_change = expected_change.saturating_add(1);
    }
    if expected_change != identity.checkpoint.saturating_add(1) {
        return Err(QueryError::tail_invalid());
    }
    let facts: Vec<_> = facts.into_values().collect();
    let models = projection::build_models(&facts).map_err(|_| QueryError::tail_invalid())?;
    Ok((facts, models))
}

fn normalize_tail_fact(
    entity_kind: &str,
    entity_key: &str,
    payload: &Value,
) -> QueryResult<ProjectionFact> {
    if !matches!(entity_kind, "invocation" | "pool_attempt")
        || payload.get("factVersion").and_then(Value::as_i64) != Some(1)
    {
        return Err(QueryError::tail_invalid());
    }
    let occurred_at = payload
        .get("occurredAt")
        .and_then(Value::as_str)
        .ok_or_else(QueryError::tail_invalid)?;
    let occurred_at =
        crate::stats::parse_to_utc_datetime(occurred_at).ok_or_else(QueryError::tail_invalid)?;
    let occurred_at_ms = occurred_at.timestamp_millis();
    let mut dimensions = BTreeMap::new();
    for field in [
        "model",
        "requestModel",
        "responseModel",
        "reasoningEffort",
        "upstreamAccountId",
        "upstreamAccountName",
        "promptCacheKeyDigest",
        "failureClass",
        "failureKind",
        "status",
        "endpoint",
    ] {
        if let Some(value) = payload.get(field).filter(|value| !value.is_null()) {
            dimensions.insert(field.to_owned(), value.clone());
        }
    }
    let interval = if entity_kind == "invocation"
        && payload.get("successLike").and_then(Value::as_bool) == Some(true)
    {
        payload
            .get("tTotalMs")
            .and_then(Value::as_f64)
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .and_then(|duration| {
                let duration_ms = duration.ceil() as i64;
                occurred_at_ms
                    .checked_add(duration_ms)
                    .map(|end_ms| serde_json::json!({"start_ms": occurred_at_ms, "end_ms": end_ms}))
            })
    } else {
        None
    };
    serde_json::from_value(serde_json::json!({
        "entity_kind": entity_kind,
        "entity_key": entity_key,
        "occurred_at_ms": occurred_at_ms,
        "dimensions": dimensions,
        "interval": interval,
        "payload": payload,
    }))
    .map_err(|_| QueryError::tail_invalid())
}

fn coverage_from_json(value: Option<&Value>) -> QueryResult<CoverageLedger> {
    let claims: Vec<CoverageClaim> = value
        .ok_or_else(QueryError::coverage_unavailable)
        .and_then(|value| {
            serde_json::from_value(value.clone()).map_err(|_| QueryError::coverage_unavailable())
        })?;
    let mut ledger = CoverageLedger::default();
    for claim in claims {
        match claim.kind {
            CoverageKind::ExactFacts => ledger
                .admit_exact(
                    claim.key,
                    claim.range,
                    claim
                        .proof_id
                        .ok_or_else(QueryError::coverage_unavailable)?,
                )
                .map_err(|_| QueryError::coverage_unavailable())?,
            CoverageKind::CertifiedSeed => ledger
                .admit_seed(
                    claim.key,
                    claim.range,
                    claim
                        .proof_id
                        .ok_or_else(QueryError::coverage_unavailable)?,
                    claim.capabilities,
                )
                .map_err(|_| QueryError::coverage_unavailable())?,
            CoverageKind::Unavailable => ledger
                .mark_unavailable(
                    claim.key,
                    claim.range,
                    claim
                        .proof_id
                        .ok_or_else(QueryError::coverage_unavailable)?,
                )
                .map_err(|_| QueryError::coverage_unavailable())?,
        }
    }
    Ok(ledger)
}

fn assemble_response(
    request: &AnalyticsQueryRequest,
    identity: QueryIdentity,
    base_facts: Vec<ProjectionFact>,
    mut models: ProjectionModels,
    tail: RuntimeTail,
    coverage: CoverageLedger,
    source_updated_at_ms: Option<i64>,
) -> QueryResult<AnalyticsQueryResponse> {
    request.validate()?;
    let admission = admit_coverage(
        &coverage,
        &request.coverage_key,
        &request.coverage_range,
        request.required_seed_capability,
    )?;
    let (facts, mut tail_models) = compose_runtime_tail(&identity, &base_facts, tail.clone())?;
    // With no live suffix the publication model is already authoritative, including any
    // certified-seed rollups that cannot be reconstructed from exact facts. A non-empty suffix
    // is recomputed from the visible facts and carries the publication's provenance metadata.
    if tail.rows.is_empty() {
        tail_models = models;
    } else {
        tail_models.history_coverage = models.history_coverage.take();
        tail_models.certified_seeds = models.certified_seeds.take();
    }
    let freshness = FreshnessEvidence {
        observed_at_ms: request.now_ms,
        source_updated_at_ms,
        source_checkpoint: identity.checkpoint,
        effective_through_change_id: tail.through_change_id,
    }
    .assess(request.max_freshness_age_ms);
    let cache_key = QueryCacheKey::for_request(request, &identity)?;
    Ok(AnalyticsQueryResponse {
        backend: identity.backend,
        identity,
        facts,
        models: tail_models,
        tail,
        coverage: admission,
        freshness,
        cache_key,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlitePublicationState {
    pub(crate) consumer_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) build_id: String,
    pub(crate) publication_id: String,
    pub(crate) projection_version: i64,
    pub(crate) published_through_change_id: i64,
    pub(crate) checkpoint: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) source_updated_at_ms: Option<i64>,
}

impl SqlitePublicationState {
    fn identity(&self) -> QueryResult<QueryIdentity> {
        QueryIdentity::from_publication(
            AnalyticsQueryBackend::Sqlite,
            &PublicationToken {
                publication_id: self.publication_id.clone(),
                consumer_id: self.consumer_id.clone(),
                build_id: self.build_id.clone(),
                owner_epoch: self.owner_epoch,
                through_change_id: self.published_through_change_id,
                coverage_revision: self.coverage_revision,
                projection_version: self.projection_version,
            },
            self.source_instance_id.clone(),
            self.generation,
            self.checkpoint,
        )
    }
}

/// Execute the SQLite adapter inside one transaction. Prefix facts/models are supplied by the
/// existing SQLite read store, while the source journal tail is read from that same transaction.
pub(crate) async fn query_sqlite_snapshot(
    pool: &Pool<Sqlite>,
    request: &AnalyticsQueryRequest,
    state: &SqlitePublicationState,
    prefix_facts: Vec<ProjectionFact>,
    prefix_models: ProjectionModels,
    coverage: CoverageLedger,
) -> QueryResult<AnalyticsQueryResponse> {
    request.validate()?;
    if request.consumer_id != state.consumer_id {
        return Err(QueryError::snapshot_inconsistent());
    }
    let mut tx = pool.begin().await.map_err(|_| QueryError::unavailable())?;
    let identity = state.identity()?;
    let tail = load_sqlite_tail(
        &mut tx,
        &identity.source_instance_id,
        identity.generation,
        identity.published_through_change_id,
        identity.checkpoint,
        identity.coverage_revision,
        identity.owner_epoch,
    )
    .await?;
    let result = assemble_response(
        request,
        identity,
        prefix_facts,
        prefix_models,
        tail,
        coverage,
        state.source_updated_at_ms,
    );
    if result.is_ok() {
        tx.commit().await.map_err(|_| QueryError::unavailable())?;
    } else {
        let _ = tx.rollback().await;
    }
    result
}

async fn load_sqlite_tail(
    tx: &mut Transaction<'_, Sqlite>,
    source_instance_id: &str,
    generation: i64,
    published_through: i64,
    checkpoint: i64,
    _coverage_revision: i64,
    owner_epoch: i64,
) -> QueryResult<RuntimeTail> {
    if checkpoint == published_through {
        return Ok(RuntimeTail::empty(checkpoint));
    }
    let rows = sqlx::query_as::<_, (i64, i64, String, String, String, String, String, String, String)>(
        "SELECT change_id, coverage_revision, entity_kind, entity_key, operation_kind, occurred_at, payload_digest, fact_json, source_instance_id
         FROM analytics_change_events
         WHERE source_instance_id = ?1 AND generation = ?2 AND change_id > ?3 AND change_id <= ?4
         ORDER BY change_id",
    )
    .bind(source_instance_id)
    .bind(generation)
    .bind(published_through)
    .bind(checkpoint)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| QueryError::unavailable())?;
    let rows = rows
        .into_iter()
        .map(
            |(
                change_id,
                row_coverage_revision,
                entity_kind,
                entity_key,
                operation,
                occurred_at,
                payload_digest,
                fact_json,
                row_source_instance_id,
            )| {
                let payload =
                    serde_json::from_str(&fact_json).map_err(|_| QueryError::tail_invalid())?;
                Ok(RuntimeTailRow {
                    source_instance_id: row_source_instance_id,
                    generation,
                    owner_epoch,
                    change_id,
                    coverage_revision: row_coverage_revision,
                    entity_kind,
                    entity_key,
                    operation,
                    occurred_at,
                    payload_digest,
                    payload,
                })
            },
        )
        .collect::<QueryResult<Vec<_>>>()?;
    Ok(RuntimeTail {
        from_change_id: published_through.saturating_add(1),
        through_change_id: checkpoint,
        rows,
    })
}

/// Read admission and every PostgreSQL component query use one transaction. In particular,
/// `current_publication`, `models_at`, `facts_at`, and the receipt tail cannot observe different
/// owner epochs or publication pointers.
pub(crate) async fn query_postgres_snapshot(
    pool: &PgPool,
    request: &AnalyticsQueryRequest,
) -> QueryResult<AnalyticsQueryResponse> {
    request.validate()?;
    let mut tx = pool.begin().await.map_err(|_| QueryError::unavailable())?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|_| QueryError::unavailable())?;
    let token = projection::current_publication(&mut tx, &request.consumer_id)
        .await
        .map_err(|_| QueryError::publication_unavailable())?;
    let metadata = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT c.source_instance_id, b.source_generation, c.checkpoint, c.coverage_revision,
                (EXTRACT(EPOCH FROM c.updated_at) * 1000)::BIGINT
         FROM cvm_analytics.analytics_module_consumers c
         JOIN cvm_analytics.analytics_module_builds b
           ON b.build_id = c.active_build_id AND b.consumer_id = c.consumer_id
         WHERE c.consumer_id = $1 AND c.active_publication_id = $2
           AND c.owner_epoch = $3 AND b.owner_epoch = c.owner_epoch
           AND b.source_instance_id = c.source_instance_id
           AND c.checkpoint >= $4",
    )
    .bind(&request.consumer_id)
    .bind(&token.publication_id)
    .bind(token.owner_epoch)
    .bind(token.through_change_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| QueryError::publication_unavailable())?
    .ok_or_else(QueryError::publication_unavailable)?;
    let (source_instance_id, generation, checkpoint, coverage_revision, source_updated_at_ms) =
        metadata;
    if coverage_revision != token.coverage_revision {
        return Err(QueryError::snapshot_inconsistent());
    }
    let identity = QueryIdentity::from_publication(
        AnalyticsQueryBackend::Postgres,
        &token,
        source_instance_id,
        generation,
        checkpoint,
    )?;
    let models = projection::models_at(&mut tx, &token)
        .await
        .map_err(|_| QueryError::publication_unavailable())?;
    let facts = projection::facts_at(&mut tx, &token)
        .await
        .map_err(|_| QueryError::publication_unavailable())?;
    let tail = load_postgres_tail(&mut tx, &identity).await?;
    let coverage = coverage_from_json(models.history_coverage.as_ref())?;
    let result = assemble_response(
        request,
        identity,
        facts,
        models,
        tail,
        coverage,
        Some(source_updated_at_ms),
    );
    if result.is_ok() {
        tx.commit().await.map_err(|_| QueryError::unavailable())?;
    } else {
        let _ = tx.rollback().await;
    }
    result
}

async fn load_postgres_tail(
    tx: &mut Transaction<'_, Postgres>,
    identity: &QueryIdentity,
) -> QueryResult<RuntimeTail> {
    if identity.checkpoint == identity.published_through_change_id {
        return Ok(RuntimeTail::empty(identity.checkpoint));
    }
    let rows = sqlx::query_as::<
        _,
        (
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
            String,
            String,
            Json<Value>,
        ),
    >(
        "SELECT source_instance_id, generation, change_id, coverage_revision,
                entity_kind, entity_key, operation_kind, occurred_at, payload_digest, payload
         FROM cvm_analytics.analytics_module_ingest_receipts
         WHERE consumer_id = $1 AND source_instance_id = $2 AND generation = $3
           AND change_id > $4 AND change_id <= $5
         ORDER BY change_id",
    )
    .bind(&identity.consumer_id)
    .bind(&identity.source_instance_id)
    .bind(identity.generation)
    .bind(identity.published_through_change_id)
    .bind(identity.checkpoint)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| QueryError::unavailable())?;
    Ok(RuntimeTail {
        from_change_id: identity.published_through_change_id.saturating_add(1),
        through_change_id: identity.checkpoint,
        rows: rows
            .into_iter()
            .map(
                |(
                    source_instance_id,
                    generation,
                    change_id,
                    coverage_revision,
                    entity_kind,
                    entity_key,
                    operation,
                    occurred_at,
                    payload_digest,
                    Json(payload),
                )| RuntimeTailRow {
                    source_instance_id,
                    generation,
                    owner_epoch: identity.owner_epoch,
                    change_id,
                    coverage_revision,
                    entity_kind,
                    entity_key,
                    operation,
                    occurred_at,
                    payload_digest,
                    payload,
                },
            )
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn coverage() -> CoverageLedger {
        let mut ledger = CoverageLedger::default();
        ledger
            .admit_exact(
                CoverageKey::new("calls", "overall", "day").unwrap(),
                CoverageRange::new("2026-11-01", "2026-11-08").unwrap(),
                "proof-exact",
            )
            .unwrap();
        ledger
    }

    fn request() -> AnalyticsQueryRequest {
        AnalyticsQueryRequest {
            consumer_id: "invocation".into(),
            query_name: "overview".into(),
            range: "7d".into(),
            dimension: "overall".into(),
            series_key: String::new(),
            timezone: "Asia/Shanghai".into(),
            coverage_key: CoverageKey::new("calls", "overall", "day").unwrap(),
            coverage_range: CoverageRange::new("2026-11-01", "2026-11-08").unwrap(),
            required_seed_capability: None,
            now_ms: 1_000,
            max_freshness_age_ms: Some(100),
        }
    }

    fn identity() -> QueryIdentity {
        QueryIdentity {
            backend: AnalyticsQueryBackend::Postgres,
            consumer_id: "invocation".into(),
            source_instance_id: "source-a".into(),
            generation: 1,
            owner_epoch: 4,
            build_id: "build-a".into(),
            publication_id: "publication-a".into(),
            projection_version: 1,
            published_through_change_id: 1,
            checkpoint: 2,
            coverage_revision: 2,
        }
    }

    fn fact(key: &str, tokens: i64) -> ProjectionFact {
        normalize_tail_fact(
            "invocation",
            key,
            &json!({
                "factVersion": 1,
                "occurredAt": "2026-11-01T00:00:00Z",
                "successLike": true,
                "totalTokens": tokens,
                "model": "fixture"
            }),
        )
        .unwrap()
    }

    fn row(change_id: i64, key: &str, operation: &str, payload: Value) -> RuntimeTailRow {
        RuntimeTailRow {
            source_instance_id: "source-a".into(),
            generation: 1,
            owner_epoch: 4,
            change_id,
            coverage_revision: change_id,
            entity_kind: "invocation".into(),
            entity_key: key.into(),
            operation: operation.into(),
            occurred_at: "2026-11-01T00:00:00Z".into(),
            payload_digest: super::super::digest_json(&payload).unwrap(),
            payload,
        }
    }

    #[test]
    fn tail_correction_and_revoke_are_folded_once() {
        let replacement = json!({
            "factVersion": 1,
            "occurredAt": "2026-11-01T00:00:00Z",
            "successLike": true,
            "totalTokens": 3,
            "model": "fixture"
        });
        let tail = RuntimeTail {
            from_change_id: 2,
            through_change_id: 2,
            rows: vec![row(2, "row-a", "correction", replacement)],
        };
        let (facts, models) =
            compose_runtime_tail(&identity(), &[fact("row-a", 10)], tail).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].payload["totalTokens"], 3);
        assert!(!models.rollups.is_empty());

        let revoked = RuntimeTail {
            from_change_id: 2,
            through_change_id: 2,
            rows: vec![row(2, "row-a", "logical_revoke", json!({"factVersion": 1}))],
        };
        let (facts, models) =
            compose_runtime_tail(&identity(), &[fact("row-a", 10)], revoked).unwrap();
        assert!(facts.is_empty());
        assert!(models.rollups.is_empty());
    }

    #[test]
    fn tail_gap_and_digest_mismatch_fail_closed() {
        let payload = json!({
            "factVersion": 1,
            "occurredAt": "2026-11-01T00:00:00Z",
            "successLike": true
        });
        let mut bad = row(2, "row-a", "upsert", payload);
        bad.payload_digest = "wrong".into();
        let tail = RuntimeTail {
            from_change_id: 2,
            through_change_id: 2,
            rows: vec![bad],
        };
        assert_eq!(
            compose_runtime_tail(&identity(), &[], tail)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
        let gap = RuntimeTail {
            from_change_id: 2,
            through_change_id: 2,
            rows: vec![row(3, "row-a", "upsert", json!({"factVersion": 1}))],
        };
        assert_eq!(
            compose_runtime_tail(&identity(), &[], gap)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
    }

    #[test]
    fn cache_identity_includes_backend_and_publication_fence() {
        let first = QueryCacheKey::for_request(&request(), &identity()).unwrap();
        let mut other_identity = identity();
        other_identity.publication_id = "publication-b".into();
        let second = QueryCacheKey::for_request(&request(), &other_identity).unwrap();
        assert_ne!(first.digest, second.digest);
        let mut sqlite_identity = identity();
        sqlite_identity.backend = AnalyticsQueryBackend::Sqlite;
        let third = QueryCacheKey::for_request(&request(), &sqlite_identity).unwrap();
        assert_ne!(first.digest, third.digest);
    }

    #[test]
    fn coverage_requires_seed_capability_and_unavailable_is_not_zero() {
        let key = CoverageKey::new("cost", "model", "day").unwrap();
        let range = CoverageRange::new("2026-11-01", "2026-11-08").unwrap();
        let mut ledger = CoverageLedger::default();
        ledger
            .admit_seed(
                key.clone(),
                range.clone(),
                "seed-proof",
                BTreeSet::from([SeedCapability::Cost]),
            )
            .unwrap();
        assert!(admit_coverage(&ledger, &key, &range, Some(SeedCapability::Cost)).is_ok());
        assert_eq!(
            admit_coverage(&ledger, &key, &range, Some(SeedCapability::Tokens))
                .unwrap_err()
                .code,
            QueryErrorCode::CoverageUnavailable
        );
        ledger
            .mark_unavailable(key.clone(), range.clone(), "missing proof")
            .unwrap();
        assert!(admit_coverage(&ledger, &key, &range, None).is_err());
    }

    #[test]
    fn freshness_is_explicit_and_policy_driven() {
        let evidence = FreshnessEvidence {
            observed_at_ms: 1_000,
            source_updated_at_ms: Some(800),
            source_checkpoint: 2,
            effective_through_change_id: 2,
        };
        assert_eq!(evidence.assess(Some(100)), FreshnessState::Stale);
        assert_eq!(evidence.assess(Some(300)), FreshnessState::Fresh);
        assert_eq!(
            FreshnessEvidence {
                source_updated_at_ms: None,
                ..evidence.clone()
            }
            .assess(Some(100)),
            FreshnessState::Unknown
        );
        assert_eq!(
            FreshnessEvidence {
                effective_through_change_id: 1,
                ..evidence
            }
            .assess(Some(300)),
            FreshnessState::Stale
        );
    }
}
