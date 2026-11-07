//! Unified analytics reads over immutable publication snapshots.
//!
//! The query boundary is deliberately narrower than either storage implementation: a response
//! is admitted against one immutable publication identity, one source prefix, and one coverage
//! proof. SQLite and PostgreSQL adapters therefore share the same cache identity, freshness, and
//! unavailable semantics instead of each growing a subtly different read contract. Runtime tails
//! are reserved for an independently authenticated in-flight source; durable rows after P are
//! never treated as a query-visible suffix.

use super::history::coverage::{
    CoverageClaim, CoverageKey, CoverageKind, CoverageLedger, CoverageRange, SeedCapability,
};
use super::projection::{self, ProjectionFact, ProjectionModels, PublicationToken};
use crate::app_state::RuntimeProjectionHub;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Pool, Sqlite, Transaction};
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

/// Admission proof for a runtime-only suffix. Durable journal/projection rows cannot manufacture
/// this proof: the caller must bind one source identity to one runtime stream and one exclusive
/// contribution claim covering the complete change interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTailProvenance {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) stream_id: String,
    pub(crate) exclusive_claim_id: String,
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTailReceipt {
    provenance: RuntimeTailProvenance,
    rows_digest: String,
    entity_digest: String,
    runtime_snapshot_revision: u64,
    runtime_snapshot_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTail {
    pub(crate) provenance: Option<RuntimeTailProvenance>,
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) rows: Vec<RuntimeTailRow>,
    #[serde(skip)]
    receipt: Option<RuntimeTailReceipt>,
}

impl RuntimeTail {
    pub(crate) fn empty(at: i64) -> Self {
        Self {
            provenance: None,
            from_change_id: at.saturating_add(1),
            through_change_id: at,
            rows: Vec::new(),
            receipt: None,
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeTailClaim {
    identity: QueryIdentity,
    provenance: RuntimeTailProvenance,
    source_proof: Option<RuntimeTailSourceProof>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTailSourceProof {
    runtime_snapshot_revision: u64,
    runtime_snapshot_digest: String,
}

impl RuntimeTailClaim {
    fn seal(self, rows: Vec<RuntimeTailRow>) -> QueryResult<RuntimeTail> {
        validate_runtime_tail_rows(&self.identity, &self.provenance, &rows)?;
        let source_proof = self.source_proof.ok_or_else(QueryError::tail_invalid)?;
        let receipt = RuntimeTailReceipt {
            provenance: self.provenance.clone(),
            rows_digest: digest_runtime_tail_rows(&rows)?,
            entity_digest: digest_runtime_tail_entities(&rows)?,
            runtime_snapshot_revision: source_proof.runtime_snapshot_revision,
            runtime_snapshot_digest: source_proof.runtime_snapshot_digest,
        };
        Ok(RuntimeTail {
            provenance: Some(self.provenance.clone()),
            from_change_id: self.provenance.from_change_id,
            through_change_id: self.provenance.through_change_id,
            rows,
            receipt: Some(receipt),
        })
    }
}

/// Controlled producer seam for the in-flight source. The producer captures the current-state
/// `RuntimeProjectionHub` directly; it does not accept query-shaped rows or durable journal
/// envelopes. Its single-use claim, snapshot fence, and stable entity set remain private so a
/// durable projection suffix cannot be relabelled by changing strings.
#[derive(Debug)]
pub(crate) struct RuntimeTailProducer<'hub> {
    identity: QueryIdentity,
    hub: &'hub RuntimeProjectionHub,
    stream_id: String,
    claim: Option<RuntimeTailClaim>,
    finished: bool,
}

impl<'hub> RuntimeTailProducer<'hub> {
    pub(crate) fn new(
        identity: QueryIdentity,
        hub: &'hub RuntimeProjectionHub,
        stream_id: String,
    ) -> QueryResult<Self> {
        if stream_id.trim().is_empty()
            || identity.checkpoint <= identity.published_through_change_id
        {
            return Err(QueryError::tail_invalid());
        }
        Ok(Self {
            identity,
            hub,
            stream_id,
            claim: None,
            finished: false,
        })
    }

    pub(crate) fn claim(&mut self, exclusive_claim_id: String) -> QueryResult<()> {
        if self.finished || self.claim.is_some() || exclusive_claim_id.trim().is_empty() {
            return Err(QueryError::tail_invalid());
        }
        let from_change_id = self.identity.published_through_change_id.saturating_add(1);
        self.claim = Some(RuntimeTailClaim {
            identity: self.identity.clone(),
            provenance: RuntimeTailProvenance {
                source_instance_id: self.identity.source_instance_id.clone(),
                generation: self.identity.generation,
                owner_epoch: self.identity.owner_epoch,
                stream_id: self.stream_id.clone(),
                exclusive_claim_id,
                from_change_id,
                through_change_id: self.identity.checkpoint,
            },
            source_proof: None,
        });
        Ok(())
    }

    pub(crate) fn finish(mut self) -> QueryResult<RuntimeTail> {
        if self.finished {
            return Err(QueryError::tail_invalid());
        }
        self.finished = true;
        let snapshot_revision_before = self.hub.dashboard_generation();
        let mut records = self.hub.snapshot();
        let snapshot_revision_after = self.hub.dashboard_generation();
        if snapshot_revision_before != snapshot_revision_after {
            return Err(QueryError::tail_invalid());
        }
        let Some(claim) = self.claim.take() else {
            return Err(QueryError::tail_invalid());
        };
        let expected_records = usize::try_from(
            claim
                .provenance
                .through_change_id
                .saturating_sub(claim.provenance.from_change_id)
                .saturating_add(1),
        )
        .map_err(|_| QueryError::tail_invalid())?;
        if records.len() != expected_records {
            return Err(QueryError::tail_invalid());
        }
        records.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.invoke_id.cmp(&right.invoke_id))
        });
        let mut seen_entities = BTreeSet::new();
        let mut rows = Vec::with_capacity(records.len());
        for (offset, record) in records.into_iter().enumerate() {
            let status = record
                .status
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if record.id != 0
                || !matches!(status.as_str(), "running" | "pending")
                || record.invoke_id.trim().is_empty()
                || record.occurred_at.trim().is_empty()
            {
                return Err(QueryError::tail_invalid());
            }
            let entity_key = format!("{}\u{001f}{}", record.invoke_id, record.occurred_at);
            if !seen_entities.insert(entity_key.clone()) {
                return Err(QueryError::tail_invalid());
            }
            let mut payload =
                serde_json::to_value(&record).map_err(|_| QueryError::tail_invalid())?;
            let Some(payload) = payload.as_object_mut() else {
                return Err(QueryError::tail_invalid());
            };
            payload.insert("factVersion".into(), serde_json::json!(1));
            payload.insert(
                "occurredAt".into(),
                serde_json::Value::String(record.occurred_at.clone()),
            );
            payload.insert("successLike".into(), serde_json::json!(false));
            let payload = serde_json::Value::Object(std::mem::take(payload));
            let payload_digest =
                super::digest_json(&payload).map_err(|_| QueryError::tail_invalid())?;
            rows.push(RuntimeTailRow {
                source_instance_id: claim.provenance.source_instance_id.clone(),
                generation: claim.provenance.generation,
                owner_epoch: claim.provenance.owner_epoch,
                change_id: claim
                    .provenance
                    .from_change_id
                    .saturating_add(offset as i64),
                coverage_revision: self.identity.coverage_revision,
                entity_kind: "invocation".into(),
                entity_key,
                operation: "upsert".into(),
                occurred_at: record.occurred_at,
                payload_digest,
                payload,
            });
        }
        let runtime_snapshot_digest = digest_runtime_tail_entities(&rows)?;
        let mut claim = claim;
        claim.source_proof = Some(RuntimeTailSourceProof {
            runtime_snapshot_revision: snapshot_revision_after,
            runtime_snapshot_digest,
        });
        claim.seal(rows)
    }
}

fn digest_runtime_tail_rows(rows: &[RuntimeTailRow]) -> QueryResult<String> {
    let value = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "sourceInstanceId": row.source_instance_id,
                "generation": row.generation,
                "ownerEpoch": row.owner_epoch,
                "changeId": row.change_id,
                "coverageRevision": row.coverage_revision,
                "entityKind": row.entity_kind,
                "entityKey": row.entity_key,
                "operation": row.operation,
                "occurredAt": row.occurred_at,
                "payloadDigest": row.payload_digest,
                "payload": row.payload,
            })
        })
        .collect::<Vec<_>>();
    super::digest_json(&Value::Array(value)).map_err(|_| QueryError::tail_invalid())
}

fn digest_runtime_tail_entities(rows: &[RuntimeTailRow]) -> QueryResult<String> {
    let value = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "changeId": row.change_id,
                "entityKind": row.entity_kind,
                "entityKey": row.entity_key,
            })
        })
        .collect::<Vec<_>>();
    super::digest_json(&Value::Array(value)).map_err(|_| QueryError::tail_invalid())
}

fn validate_runtime_tail_rows(
    identity: &QueryIdentity,
    provenance: &RuntimeTailProvenance,
    rows: &[RuntimeTailRow],
) -> QueryResult<()> {
    if provenance.source_instance_id != identity.source_instance_id
        || provenance.generation != identity.generation
        || provenance.owner_epoch != identity.owner_epoch
        || provenance.stream_id.trim().is_empty()
        || provenance.exclusive_claim_id.trim().is_empty()
        || provenance.from_change_id != identity.published_through_change_id.saturating_add(1)
        || provenance.through_change_id != identity.checkpoint
        || rows.is_empty()
    {
        return Err(QueryError::tail_invalid());
    }

    let mut expected_change_id = provenance.from_change_id;
    let mut seen_entities = BTreeSet::new();
    for row in rows {
        if row.source_instance_id != provenance.source_instance_id
            || row.generation != provenance.generation
            || row.owner_epoch != provenance.owner_epoch
            || row.change_id != expected_change_id
            || row.change_id > provenance.through_change_id
            || row.coverage_revision > identity.coverage_revision
            || row.entity_kind.trim().is_empty()
            || row.entity_key.trim().is_empty()
            || row.occurred_at.trim().is_empty()
            || row.payload_digest.trim().is_empty()
            || !row.payload.is_object()
        {
            return Err(QueryError::tail_invalid());
        }
        let digest = super::digest_json(&row.payload).map_err(|_| QueryError::tail_invalid())?;
        if digest != row.payload_digest {
            return Err(QueryError::tail_invalid());
        }
        if !matches!(
            row.operation.as_str(),
            "upsert" | "correction" | "logical_revoke"
        ) {
            return Err(QueryError::tail_invalid());
        }
        if !seen_entities.insert((
            row.change_id,
            row.entity_kind.clone(),
            row.entity_key.clone(),
        )) {
            return Err(QueryError::tail_invalid());
        }
        expected_change_id = expected_change_id.saturating_add(1);
    }
    if expected_change_id != provenance.through_change_id.saturating_add(1) {
        return Err(QueryError::tail_invalid());
    }
    Ok(())
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
fn validate_runtime_tail_provenance(
    identity: &QueryIdentity,
    tail: &RuntimeTail,
    is_last_good_fallback: bool,
) -> QueryResult<()> {
    if is_last_good_fallback {
        return if tail.provenance.is_none() && tail.receipt.is_none() {
            Ok(())
        } else {
            Err(QueryError::tail_invalid())
        };
    }
    let Some(provenance) = tail.provenance.as_ref() else {
        return Err(QueryError::tail_invalid());
    };
    if provenance.source_instance_id != identity.source_instance_id
        || provenance.generation != identity.generation
        || provenance.owner_epoch != identity.owner_epoch
        || provenance.stream_id.trim().is_empty()
        || provenance.exclusive_claim_id.trim().is_empty()
        || provenance.from_change_id != tail.from_change_id
        || provenance.through_change_id != tail.through_change_id
    {
        return Err(QueryError::tail_invalid());
    }
    let Some(receipt) = tail.receipt.as_ref() else {
        return Err(QueryError::tail_invalid());
    };
    if receipt.provenance != *provenance
        || receipt.rows_digest != digest_runtime_tail_rows(&tail.rows)?
        || receipt.entity_digest != digest_runtime_tail_entities(&tail.rows)?
        || receipt.runtime_snapshot_digest != receipt.entity_digest
    {
        return Err(QueryError::tail_invalid());
    }
    validate_runtime_tail_rows(identity, provenance, &tail.rows)?;
    Ok(())
}

pub(crate) fn compose_runtime_tail(
    identity: &QueryIdentity,
    base_facts: &[ProjectionFact],
    tail: RuntimeTail,
) -> QueryResult<(Vec<ProjectionFact>, ProjectionModels)> {
    let is_last_good_fallback =
        tail.rows.is_empty() && tail.through_change_id == identity.published_through_change_id;
    if tail.from_change_id != identity.published_through_change_id.saturating_add(1)
        || (tail.through_change_id != identity.checkpoint && !is_last_good_fallback)
    {
        return Err(QueryError::tail_invalid());
    }
    validate_runtime_tail_provenance(identity, &tail, is_last_good_fallback)?;
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
        if row.entity_key.contains('\u{001f}') && facts.contains_key(&entity) {
            return Err(QueryError::tail_invalid());
        }
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
    if !is_last_good_fallback && expected_change != identity.checkpoint.saturating_add(1) {
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
            .filter(|duration| {
                duration.is_finite() && *duration > 0.0 && *duration < i64::MAX as f64
            })
            .and_then(|duration| {
                let duration_ms = (duration
                    + f64::from(occurred_at.timestamp_subsec_nanos() % 1_000_000) / 1_000_000.0)
                    .ceil() as i64;
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SqlitePrefixSnapshot {
    pub(crate) identity: QueryIdentity,
    pub(crate) facts: Vec<ProjectionFact>,
    pub(crate) models: ProjectionModels,
    pub(crate) coverage: CoverageLedger,
    pub(crate) source_updated_at_ms: Option<i64>,
}

/// Implementations must read the durable SQLite publication identity, prefix facts/models,
/// coverage, and source checkpoint through the transaction supplied here. The query adapter does
/// not accept caller-owned state, so a prefix read cannot be paired with a newer journal
/// transaction by accident. The returned identity is the immutable admission token for the same
/// transaction.
#[async_trait::async_trait]
pub(crate) trait SqlitePrefixLoader: Send + Sync {
    async fn load_prefix(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        consumer_id: &str,
    ) -> QueryResult<SqlitePrefixSnapshot>;
}

fn last_good_tail_for_persistent_snapshot(identity: &QueryIdentity) -> QueryResult<RuntimeTail> {
    if identity.checkpoint < identity.published_through_change_id {
        return Err(QueryError::snapshot_inconsistent());
    }
    // Persistent rows after P are not an in-flight runtime source. Keep the published facts and
    // models intact, and let freshness report C > P as stale until a new publication exists.
    Ok(RuntimeTail::empty(identity.published_through_change_id))
}

/// Execute the SQLite adapter inside one transaction. The loader owns all durable reads for the
/// identity, prefix, models, coverage, and source checkpoint. No persistent `(P, C]` rows are
/// folded into the response.
pub(crate) async fn query_sqlite_snapshot(
    pool: &Pool<Sqlite>,
    request: &AnalyticsQueryRequest,
    loader: &dyn SqlitePrefixLoader,
) -> QueryResult<AnalyticsQueryResponse> {
    request.validate()?;
    let mut tx = pool.begin().await.map_err(|_| QueryError::unavailable())?;
    let prefix = loader.load_prefix(&mut tx, &request.consumer_id).await?;
    if prefix.identity.backend != AnalyticsQueryBackend::Sqlite
        || prefix.identity.consumer_id != request.consumer_id
    {
        return Err(QueryError::snapshot_inconsistent());
    }
    let identity = prefix.identity.clone();
    let tail = last_good_tail_for_persistent_snapshot(&identity)?;
    let result = assemble_response(
        request,
        identity,
        prefix.facts,
        prefix.models,
        tail,
        prefix.coverage,
        prefix.source_updated_at_ms,
    );
    if result.is_ok() {
        tx.commit().await.map_err(|_| QueryError::unavailable())?;
    } else {
        let _ = tx.rollback().await;
    }
    result
}

/// Read admission and every PostgreSQL component query use one transaction. In particular,
/// `current_publication`, `models_at`, and `facts_at` cannot observe different owner epochs or
/// publication pointers. Durable rows after P remain invisible until the next publication.
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
    let tail = last_good_tail_for_persistent_snapshot(&identity)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tokio::sync::{Barrier, Notify};

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

    fn sealed_test_tail(rows: Vec<RuntimeTailRow>) -> RuntimeTail {
        let provenance = RuntimeTailProvenance {
            source_instance_id: "source-a".into(),
            generation: 1,
            owner_epoch: 4,
            stream_id: "runtime-stream-a".into(),
            exclusive_claim_id: "claim-a".into(),
            from_change_id: 2,
            through_change_id: 2,
        };
        let source_proof = RuntimeTailSourceProof {
            runtime_snapshot_revision: 1,
            runtime_snapshot_digest: digest_runtime_tail_entities(&rows).unwrap(),
        };
        RuntimeTailClaim {
            identity: identity(),
            provenance,
            source_proof: Some(source_proof),
        }
        .seal(rows)
        .unwrap()
    }

    fn runtime_record(row: &RuntimeTailRow) -> crate::ApiInvocation {
        crate::ApiInvocation {
            id: 0,
            invoke_id: row.entity_key.clone(),
            occurred_at: row.occurred_at.clone(),
            source: "proxy".into(),
            proxy_display_name: None,
            model: row
                .payload
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            request_model: None,
            response_model: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            reported_cache_write_tokens: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            total_tokens: row.payload.get("totalTokens").and_then(Value::as_i64),
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: None,
            status: Some("running".into()),
            live_phase: Some("requesting".into()),
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: None,
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: None,
            prompt_cache_key: None,
            sticky_key: None,
            route_mode: None,
            upstream_account_id: None,
            upstream_account_name: None,
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: None,
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            price_version: None,
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: "full".into(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: row.payload.get("tTotalMs").and_then(Value::as_f64),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: row.occurred_at.clone(),
        }
    }

    fn runtime_tail(rows: Vec<RuntimeTailRow>) -> RuntimeTail {
        let hub = RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        for row in &rows {
            hub.upsert(runtime_record(row));
        }
        let mut producer =
            RuntimeTailProducer::new(identity(), &hub, "runtime-stream-a".into()).unwrap();
        producer.claim("claim-a".into()).unwrap();
        producer.finish().unwrap()
    }

    fn untrusted_runtime_tail(rows: Vec<RuntimeTailRow>) -> RuntimeTail {
        RuntimeTail {
            provenance: Some(RuntimeTailProvenance {
                source_instance_id: "source-a".into(),
                generation: 1,
                owner_epoch: 4,
                stream_id: "runtime-stream-a".into(),
                exclusive_claim_id: "claim-a".into(),
                from_change_id: 2,
                through_change_id: 2,
            }),
            from_change_id: 2,
            through_change_id: 2,
            rows,
            receipt: None,
        }
    }

    struct SqliteTestLoader {
        first_read: Option<Arc<Barrier>>,
        writer_committed: Option<Arc<Notify>>,
        mismatched_identity: bool,
    }

    #[async_trait::async_trait]
    impl SqlitePrefixLoader for SqliteTestLoader {
        async fn load_prefix(
            &self,
            tx: &mut Transaction<'_, Sqlite>,
            consumer_id: &str,
        ) -> QueryResult<SqlitePrefixSnapshot> {
            let (checkpoint, total_tokens) = sqlx::query_as::<_, (i64, i64)>(
                "SELECT checkpoint, total_tokens FROM query_snapshot WHERE consumer_id = ?1",
            )
            .bind(consumer_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| QueryError::unavailable())?;
            if let Some(barrier) = &self.first_read {
                barrier.wait().await;
            }
            if let Some(writer_committed) = &self.writer_committed {
                writer_committed.notified().await;
                let observed = sqlx::query_as::<_, (i64, i64)>(
                    "SELECT checkpoint, total_tokens FROM query_snapshot WHERE consumer_id = ?1",
                )
                .bind(consumer_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(|_| QueryError::unavailable())?;
                if observed != (checkpoint, total_tokens) {
                    return Err(QueryError::snapshot_inconsistent());
                }
            }
            let mut identity = identity();
            identity.backend = if self.mismatched_identity {
                AnalyticsQueryBackend::Postgres
            } else {
                AnalyticsQueryBackend::Sqlite
            };
            identity.checkpoint = checkpoint;
            Ok(SqlitePrefixSnapshot {
                identity,
                facts: vec![fact("row-a", total_tokens)],
                models: ProjectionModels::default(),
                coverage: coverage(),
                source_updated_at_ms: Some(1_000),
            })
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
        let tail = sealed_test_tail(vec![row(2, "row-a", "correction", replacement)]);
        let (facts, models) =
            compose_runtime_tail(&identity(), &[fact("row-a", 10)], tail).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].payload["totalTokens"], 3);
        assert!(!models.rollups.is_empty());

        let revoked = sealed_test_tail(vec![row(
            2,
            "row-a",
            "logical_revoke",
            json!({"factVersion": 1}),
        )]);
        let (facts, models) =
            compose_runtime_tail(&identity(), &[fact("row-a", 10)], revoked).unwrap();
        assert!(facts.is_empty());
        assert!(models.rollups.is_empty());
    }

    #[test]
    fn runtime_tail_requires_stable_identity_and_exclusive_claim() {
        let payload = json!({
            "factVersion": 1,
            "occurredAt": "2026-11-01T00:00:00Z",
            "successLike": true,
            "totalTokens": 3
        });
        let mut missing_provenance = runtime_tail(vec![row(2, "row-a", "upsert", payload.clone())]);
        missing_provenance.provenance = None;
        assert_eq!(
            compose_runtime_tail(&identity(), &[], missing_provenance)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );

        let mut wrong_claim = runtime_tail(vec![row(2, "row-a", "upsert", payload.clone())]);
        wrong_claim
            .provenance
            .as_mut()
            .expect("fixture provenance")
            .exclusive_claim_id = "".into();
        assert_eq!(
            compose_runtime_tail(&identity(), &[], wrong_claim)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );

        let mut changed_row = runtime_tail(vec![row(2, "row-a", "upsert", payload.clone())]);
        changed_row.rows[0].payload["totalTokens"] = json!(99);
        assert_eq!(
            compose_runtime_tail(&identity(), &[], changed_row)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );

        let durable_relabel = untrusted_runtime_tail(vec![row(2, "row-a", "upsert", payload)]);
        assert_eq!(
            compose_runtime_tail(&identity(), &[], durable_relabel)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
    }

    #[test]
    fn runtime_tail_producer_captures_only_one_live_hub_claim() {
        let live_row = row(
            2,
            "runtime-row",
            "upsert",
            json!({
                "factVersion": 1,
                "occurredAt": "2026-11-01T00:00:00Z",
                "successLike": false,
                "totalTokens": 3
            }),
        );
        let hub = RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        hub.upsert(runtime_record(&live_row));
        let mut producer =
            RuntimeTailProducer::new(identity(), &hub, "runtime-stream-a".into()).unwrap();
        producer.claim("claim-a".into()).unwrap();
        assert_eq!(
            producer.claim("claim-b".into()).unwrap_err().code,
            QueryErrorCode::TailInvalid
        );
        let tail = producer.finish().unwrap();
        compose_runtime_tail(&identity(), &[], tail.clone()).unwrap();

        let persisted_hub =
            RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        let mut persisted = runtime_record(&live_row);
        persisted.id = 42;
        persisted_hub.upsert(persisted);
        let mut persisted_producer =
            RuntimeTailProducer::new(identity(), &persisted_hub, "runtime-stream-a".into())
                .unwrap();
        persisted_producer.claim("claim-a".into()).unwrap();
        assert_eq!(
            persisted_producer.finish().unwrap_err().code,
            QueryErrorCode::TailInvalid
        );

        let overlapping_base = fact(&tail.rows[0].entity_key, 1);
        assert_eq!(
            compose_runtime_tail(&identity(), &[overlapping_base], tail)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
    }

    #[test]
    fn persistent_unpublished_rows_keep_last_good_for_insert_correction_revoke_and_seed() {
        let base = vec![fact("row-a", 10)];
        for (operation, payload) in [
            (
                "upsert",
                json!({
                    "factVersion": 1,
                    "occurredAt": "2026-11-01T00:00:00Z",
                    "successLike": true,
                    "totalTokens": 30
                }),
            ),
            (
                "correction",
                json!({
                    "factVersion": 1,
                    "occurredAt": "2026-11-01T00:00:00Z",
                    "successLike": true,
                    "totalTokens": 3
                }),
            ),
            ("logical_revoke", json!({"factVersion": 1})),
        ] {
            let persistent_tail = untrusted_runtime_tail(vec![row(2, "row-a", operation, payload)]);
            assert_eq!(
                compose_runtime_tail(&identity(), &base, persistent_tail)
                    .unwrap_err()
                    .code,
                QueryErrorCode::TailInvalid
            );
            let admitted = last_good_tail_for_persistent_snapshot(&identity()).unwrap();
            assert_eq!(admitted, RuntimeTail::empty(1));
            let (facts, _) = compose_runtime_tail(&identity(), &base, admitted).unwrap();
            assert_eq!(facts[0].payload["totalTokens"], 10);
        }

        let published_models = ProjectionModels {
            certified_seeds: json!({"seed": "unknown-old-contribution"}),
            history_coverage: Some(json!({"proof": "published"})),
            ..ProjectionModels::default()
        };
        let response = assemble_response(
            &request(),
            identity(),
            base,
            published_models,
            RuntimeTail::empty(1),
            coverage(),
            Some(1_000),
        )
        .unwrap();
        assert_eq!(
            response.models.certified_seeds["seed"],
            "unknown-old-contribution"
        );
        assert_eq!(response.freshness, FreshnessState::Stale);
    }

    #[test]
    fn interval_normalization_preserves_submillisecond_occurrence_precision() {
        let normalized = normalize_tail_fact(
            "invocation",
            "precise-row",
            &json!({
                "factVersion": 1,
                "occurredAt": "2026-11-01T00:00:00.000900Z",
                "successLike": true,
                "tTotalMs": 0.2
            }),
        )
        .unwrap();
        let interval = normalized.interval.unwrap();
        assert_eq!(interval.end_ms - interval.start_ms, 2);
    }

    #[tokio::test]
    async fn sqlite_loader_keeps_one_snapshot_and_rejects_identity_mismatch() {
        let database = std::env::temp_dir().join(format!(
            "cvm-analytics-query-{}-{}.db",
            std::process::id(),
            1
        ));
        let database_url = format!("sqlite://{}?mode=rwc", database.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE query_snapshot (
                consumer_id TEXT PRIMARY KEY,
                checkpoint INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO query_snapshot VALUES ('invocation', 2, 10)")
            .execute(&pool)
            .await
            .unwrap();

        let first_read = Arc::new(Barrier::new(2));
        let writer_committed = Arc::new(Notify::new());
        let loader_first_read = first_read.clone();
        let loader_writer_committed = writer_committed.clone();
        let query_pool = pool.clone();
        let query_task = tokio::spawn(async move {
            query_sqlite_snapshot(
                &query_pool,
                &request(),
                &SqliteTestLoader {
                    first_read: Some(loader_first_read),
                    writer_committed: Some(loader_writer_committed),
                    mismatched_identity: false,
                },
            )
            .await
        });
        first_read.wait().await;
        sqlx::query("UPDATE query_snapshot SET checkpoint = 3, total_tokens = 99 WHERE consumer_id = 'invocation'")
            .execute(&pool)
            .await
            .unwrap();
        let outside = sqlx::query_as::<_, (i64, i64)>(
            "SELECT checkpoint, total_tokens FROM query_snapshot WHERE consumer_id = 'invocation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outside, (3, 99));
        writer_committed.notify_one();
        let response = query_task.await.unwrap().unwrap();
        assert_eq!(response.identity.checkpoint, 2);
        assert_eq!(response.facts[0].payload["totalTokens"], 10);
        assert_eq!(response.tail, RuntimeTail::empty(1));
        assert_eq!(response.freshness, FreshnessState::Stale);

        let mismatch = query_sqlite_snapshot(
            &pool,
            &request(),
            &SqliteTestLoader {
                first_read: None,
                writer_committed: None,
                mismatched_identity: true,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(mismatch.code, QueryErrorCode::SnapshotInconsistent);
        pool.close().await;
        let _ = std::fs::remove_file(database);
    }

    #[tokio::test]
    #[ignore = "requires an owner-created temporary PostgreSQL 18 reader fixture"]
    async fn postgres_reader_hides_unpublished_persistent_tail() {
        let url = std::env::var("C6_PG_QUERY_APP_URL").expect("C6_PG_QUERY_APP_URL");
        let case = std::env::var("C6_PG_QUERY_CASE").expect("C6_PG_QUERY_CASE");
        let pool = PgPool::connect(&url).await.unwrap();
        let result = query_postgres_snapshot(&pool, &request()).await;
        match case.as_str() {
            "published" => {
                let response = result.unwrap();
                assert_eq!(response.identity.published_through_change_id, 1);
                assert_eq!(response.identity.checkpoint, 1);
                assert_eq!(response.facts[0].payload["totalTokens"], 10);
                assert_eq!(response.tail, RuntimeTail::empty(1));
            }
            "unpublished" => {
                let response = result.unwrap();
                assert_eq!(response.identity.published_through_change_id, 1);
                assert_eq!(response.identity.checkpoint, 3);
                assert_eq!(response.facts.len(), 1);
                assert_eq!(response.facts[0].payload["totalTokens"], 10);
                assert_eq!(response.tail, RuntimeTail::empty(1));
                assert_eq!(response.freshness, FreshnessState::Stale);
            }
            "blocked" => {
                assert_eq!(
                    result.unwrap_err().code,
                    QueryErrorCode::PublicationUnavailable
                );
            }
            other => panic!("unknown C6_PG_QUERY_CASE: {other}"),
        }
        pool.close().await;
    }

    #[test]
    fn tail_gap_and_digest_mismatch_fail_closed() {
        let payload = json!({
            "factVersion": 1,
            "occurredAt": "2026-11-01T00:00:00Z",
            "successLike": true
        });
        let mut tail = sealed_test_tail(vec![row(2, "row-a", "upsert", payload)]);
        tail.rows[0].payload_digest = "wrong".into();
        assert_eq!(
            compose_runtime_tail(&identity(), &[], tail)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
        let mut gap = runtime_tail(vec![row(2, "row-a", "upsert", json!({"factVersion": 1}))]);
        gap.rows[0].change_id = 3;
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
