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
use sqlx::{PgPool, Pool, Postgres, Sqlite, Transaction};
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
///
/// `range` remains the long-term presentation range from the public analytics contract for legacy
/// requests. Consumer views which are not one of those fixed windows use `consumer_range` instead;
/// the latter is deliberately part of this request rather than an alias for `coverage_range`. The
/// range tag is normalized to `all`, `current`, or `explicit` by `with_consumer_range`, so a new
/// consumer request does not masquerade as a long-term window. The query layer still reads one
/// published prefix and its admitted coverage. Consumers apply the `Current` limit or `Explicit`
/// boundary to that response, while both the tag and typed value participate in the shared cache
/// identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsQueryRequest {
    pub(crate) consumer_id: String,
    pub(crate) query_name: String,
    pub(crate) range: String,
    #[serde(default)]
    pub(crate) consumer_range: Option<AnalyticsQueryRange>,
    pub(crate) dimension: String,
    pub(crate) series_key: String,
    pub(crate) timezone: String,
    pub(crate) coverage_key: CoverageKey,
    pub(crate) coverage_range: CoverageRange,
    pub(crate) required_seed_capability: Option<SeedCapability>,
    pub(crate) now_ms: i64,
    pub(crate) max_freshness_age_ms: Option<i64>,
}

/// Consumer-facing window semantics which are not part of the fixed long-term range set.
///
/// `All` means all facts visible in the admitted publication response, not unbounded history.
/// `Current` is a bounded newest-N view over that same response. `Explicit` carries the consumer
/// boundary independently from the coverage proof; validation requires the explicit boundary to
/// be contained by `coverage_range` before the response can be admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum AnalyticsQueryRange {
    All,
    Current { limit: i64 },
    Explicit { start: String, end: String },
}

impl AnalyticsQueryRange {
    pub(crate) fn all() -> Self {
        Self::All
    }

    pub(crate) fn current(limit: i64) -> Self {
        Self::Current { limit }
    }

    pub(crate) fn explicit(start: impl Into<String>, end: impl Into<String>) -> Self {
        Self::Explicit {
            start: start.into(),
            end: end.into(),
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Current { .. } => "current",
            Self::Explicit { .. } => "explicit",
        }
    }

    fn validate_against(&self, coverage_range: &CoverageRange) -> QueryResult<()> {
        match self {
            Self::All => Ok(()),
            Self::Current { limit } if *limit > 0 => Ok(()),
            Self::Current { .. } => Err(QueryError::invalid_query()),
            Self::Explicit { start, end } => {
                let explicit = CoverageRange::new(start.clone(), end.clone())
                    .map_err(|_| QueryError::invalid_query())?;
                if coverage_range.contains(&explicit) {
                    Ok(())
                } else {
                    Err(QueryError::invalid_query())
                }
            }
        }
    }
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
        let long_term_range = matches!(self.range.as_str(), "7d" | "30d" | "180d" | "365d");
        let range_valid = match &self.consumer_range {
            None => long_term_range,
            Some(consumer_range) => self.range == consumer_range.tag(),
        };
        if !range_valid
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
        if let Some(consumer_range) = &self.consumer_range {
            consumer_range.validate_against(&self.coverage_range)?;
        }
        Ok(())
    }

    pub(crate) fn with_consumer_range(mut self, consumer_range: AnalyticsQueryRange) -> Self {
        self.range = consumer_range.tag().to_string();
        self.consumer_range = Some(consumer_range);
        self
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
    pub(crate) live_snapshot_revision: Option<u64>,
    pub(crate) live_snapshot_digest: Option<String>,
}

impl QueryCacheKey {
    pub(crate) fn for_request(
        request: &AnalyticsQueryRequest,
        identity: &QueryIdentity,
    ) -> QueryResult<Self> {
        Self::for_response(request, identity, None)
    }

    pub(crate) fn for_response(
        request: &AnalyticsQueryRequest,
        identity: &QueryIdentity,
        live: Option<&RuntimeLiveSnapshot>,
    ) -> QueryResult<Self> {
        request.validate()?;
        let live_snapshot_revision = live.map(|snapshot| snapshot.snapshot_revision);
        let live_snapshot_digest = live
            .map(|snapshot| digest_runtime_live_entities(&snapshot.rows))
            .transpose()?;
        let encoded = serde_json::to_vec(&(
            request,
            identity,
            live_snapshot_revision,
            &live_snapshot_digest,
        ))
        .map_err(|_| QueryError::internal_contract())?;
        let digest = format!("{:x}", Sha256::digest(encoded));
        Ok(Self {
            digest,
            identity: identity.clone(),
            live_snapshot_revision,
            live_snapshot_digest,
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

/// Admission proof for a durable publication suffix. Durable journal/projection rows cannot be
/// relabelled across source identity, stream, or publication interval.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeLiveProvenance {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) stream_id: String,
    pub(crate) exclusive_claim_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeLiveRow {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) occurred_at: String,
    pub(crate) payload_digest: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeLiveSnapshot {
    pub(crate) provenance: RuntimeLiveProvenance,
    pub(crate) snapshot_revision: u64,
    pub(crate) rows: Vec<RuntimeLiveRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTailReceipt {
    persistent_provenance: Option<RuntimeTailProvenance>,
    live_provenance: Option<RuntimeLiveProvenance>,
    rows_digest: String,
    entity_digest: String,
    runtime_snapshot_revision: Option<u64>,
    runtime_snapshot_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTail {
    pub(crate) provenance: Option<RuntimeTailProvenance>,
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) rows: Vec<RuntimeTailRow>,
    pub(crate) live: Option<RuntimeLiveSnapshot>,
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
            live: None,
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

#[derive(Debug, Clone)]
struct RuntimeLiveClaim {
    identity: QueryIdentity,
    provenance: RuntimeLiveProvenance,
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
            persistent_provenance: Some(self.provenance.clone()),
            live_provenance: None,
            rows_digest: digest_runtime_tail_rows(&rows)?,
            entity_digest: digest_runtime_tail_entities(&rows)?,
            runtime_snapshot_revision: Some(source_proof.runtime_snapshot_revision),
            runtime_snapshot_digest: Some(source_proof.runtime_snapshot_digest),
        };
        Ok(RuntimeTail {
            provenance: Some(self.provenance.clone()),
            from_change_id: self.provenance.from_change_id,
            through_change_id: self.provenance.through_change_id,
            rows,
            live: None,
            receipt: Some(receipt),
        })
    }
}

impl RuntimeLiveClaim {
    fn seal(self, snapshot_revision: u64, rows: Vec<RuntimeLiveRow>) -> QueryResult<RuntimeTail> {
        validate_runtime_live_rows(&self.identity, &self.provenance, &rows)?;
        let rows_digest = digest_runtime_live_rows(&rows)?;
        let entity_digest = digest_runtime_live_entities(&rows)?;
        Ok(RuntimeTail {
            provenance: None,
            from_change_id: self.identity.published_through_change_id.saturating_add(1),
            through_change_id: self.identity.published_through_change_id,
            rows: Vec::new(),
            live: Some(RuntimeLiveSnapshot {
                provenance: self.provenance.clone(),
                snapshot_revision,
                rows,
            }),
            receipt: Some(RuntimeTailReceipt {
                persistent_provenance: None,
                live_provenance: Some(self.provenance),
                rows_digest,
                entity_digest: entity_digest.clone(),
                runtime_snapshot_revision: Some(snapshot_revision),
                runtime_snapshot_digest: Some(entity_digest),
            }),
        })
    }
}

/// Controlled producer seam for the in-flight source. The producer captures the current-state
/// `RuntimeProjectionHub` directly; it does not accept query-shaped rows or durable journal
/// envelopes. Its per-response claim, snapshot fence, and stable entity set remain private so a
/// durable projection suffix cannot be relabelled by changing strings.
#[derive(Debug)]
pub(crate) struct RuntimeTailProducer<'hub> {
    identity: QueryIdentity,
    hub: &'hub RuntimeProjectionHub,
    stream_id: String,
    claim: Option<RuntimeLiveClaim>,
    finished: bool,
}

impl<'hub> RuntimeTailProducer<'hub> {
    pub(crate) fn new(
        identity: QueryIdentity,
        hub: &'hub RuntimeProjectionHub,
        stream_id: String,
    ) -> QueryResult<Self> {
        if stream_id.trim().is_empty() || identity.checkpoint < identity.published_through_change_id
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
        self.claim = Some(RuntimeLiveClaim {
            identity: self.identity.clone(),
            provenance: RuntimeLiveProvenance {
                source_instance_id: self.identity.source_instance_id.clone(),
                generation: self.identity.generation,
                owner_epoch: self.identity.owner_epoch,
                stream_id: self.stream_id.clone(),
                exclusive_claim_id,
            },
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
        records.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.invoke_id.cmp(&right.invoke_id))
        });
        let mut seen_entities = BTreeSet::new();
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
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
            rows.push(RuntimeLiveRow {
                source_instance_id: claim.provenance.source_instance_id.clone(),
                generation: claim.provenance.generation,
                owner_epoch: claim.provenance.owner_epoch,
                coverage_revision: self.identity.coverage_revision,
                entity_kind: "invocation".into(),
                entity_key,
                occurred_at: record.occurred_at,
                payload_digest,
                payload,
            });
        }
        claim.seal(snapshot_revision_after, rows)
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

fn digest_runtime_live_rows(rows: &[RuntimeLiveRow]) -> QueryResult<String> {
    let value = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "sourceInstanceId": row.source_instance_id,
                "generation": row.generation,
                "ownerEpoch": row.owner_epoch,
                "coverageRevision": row.coverage_revision,
                "entityKind": row.entity_kind,
                "entityKey": row.entity_key,
                "occurredAt": row.occurred_at,
                "payloadDigest": row.payload_digest,
                "payload": row.payload,
            })
        })
        .collect::<Vec<_>>();
    super::digest_json(&Value::Array(value)).map_err(|_| QueryError::tail_invalid())
}

fn digest_runtime_live_entities(rows: &[RuntimeLiveRow]) -> QueryResult<String> {
    let value = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "entityKind": row.entity_kind,
                "entityKey": row.entity_key,
                "occurredAt": row.occurred_at,
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

fn validate_runtime_live_rows(
    identity: &QueryIdentity,
    provenance: &RuntimeLiveProvenance,
    rows: &[RuntimeLiveRow],
) -> QueryResult<()> {
    if provenance.source_instance_id != identity.source_instance_id
        || provenance.generation != identity.generation
        || provenance.owner_epoch != identity.owner_epoch
        || provenance.stream_id.trim().is_empty()
        || provenance.exclusive_claim_id.trim().is_empty()
    {
        return Err(QueryError::tail_invalid());
    }

    let mut seen_entities = BTreeSet::new();
    for row in rows {
        if row.source_instance_id != provenance.source_instance_id
            || row.generation != provenance.generation
            || row.owner_epoch != provenance.owner_epoch
            || row.coverage_revision > identity.coverage_revision
            || row.entity_kind != "invocation"
            || row.entity_key.trim().is_empty()
            || row.occurred_at.trim().is_empty()
            || row.payload_digest.trim().is_empty()
            || !row.payload.is_object()
        {
            return Err(QueryError::tail_invalid());
        }
        let digest = super::digest_json(&row.payload).map_err(|_| QueryError::tail_invalid())?;
        if digest != row.payload_digest
            || !seen_entities
                .insert(stable_live_entity_key(row)?.ok_or_else(QueryError::tail_invalid)?)
        {
            return Err(QueryError::tail_invalid());
        }
        normalize_tail_fact(&row.entity_kind, &row.entity_key, &row.payload)?;
    }
    Ok(())
}

fn stable_live_entity_key(row: &RuntimeLiveRow) -> QueryResult<Option<String>> {
    let invoke_id = row
        .payload
        .get("invokeId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let occurred_at = row
        .payload
        .get("occurredAt")
        .and_then(Value::as_str)
        .unwrap_or(&row.occurred_at);
    let Some(invoke_id) = invoke_id else {
        return Ok(None);
    };
    let occurred_at = crate::stats::parse_to_utc_datetime(occurred_at)
        .ok_or_else(QueryError::tail_invalid)?
        .timestamp_millis();
    Ok(Some(format!(
        "invocation\u{001f}{invoke_id}\u{001f}{occurred_at}"
    )))
}

fn stable_fact_entity_key(fact: &ProjectionFact) -> QueryResult<Option<String>> {
    if fact.entity_kind != "invocation" {
        return Ok(None);
    }
    let Some(invoke_id) = fact
        .payload
        .get("invokeId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(occurred_at) = fact.payload.get("occurredAt").and_then(Value::as_str) else {
        return Ok(None);
    };
    let occurred_at = crate::stats::parse_to_utc_datetime(occurred_at)
        .ok_or_else(QueryError::tail_invalid)?
        .timestamp_millis();
    Ok(Some(format!(
        "invocation\u{001f}{invoke_id}\u{001f}{occurred_at}"
    )))
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

/// Fold a bounded durable receipt suffix and an independently fenced live snapshot over the
/// immutable publication facts. Durable rows must be a complete `(P, C]` prefix; live rows never
/// receive journal change IDs. A missing receipt, duplicate change, stale source generation, or
/// mixed owner epoch therefore fails closed instead of producing a partial chart.
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
    if tail.rows.is_empty() && tail.provenance.is_none() && tail.live.is_some() {
        if tail
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.persistent_provenance.is_none())
        {
            return Ok(());
        }
        return Err(QueryError::tail_invalid());
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
    if receipt.persistent_provenance.as_ref() != Some(provenance)
        || receipt.rows_digest != digest_runtime_tail_rows(&tail.rows)?
        || receipt.entity_digest != digest_runtime_tail_entities(&tail.rows)?
        || receipt.runtime_snapshot_digest.as_deref() != Some(receipt.entity_digest.as_str())
    {
        return Err(QueryError::tail_invalid());
    }
    validate_runtime_tail_rows(identity, provenance, &tail.rows)?;
    Ok(())
}

fn validate_runtime_live_snapshot(identity: &QueryIdentity, tail: &RuntimeTail) -> QueryResult<()> {
    let Some(live) = tail.live.as_ref() else {
        return Ok(());
    };
    let Some(receipt) = tail.receipt.as_ref() else {
        return Err(QueryError::tail_invalid());
    };
    if receipt.live_provenance.as_ref() != Some(&live.provenance)
        || receipt.runtime_snapshot_revision != Some(live.snapshot_revision)
        || receipt.rows_digest != digest_runtime_live_rows(&live.rows)?
        || receipt.entity_digest != digest_runtime_live_entities(&live.rows)?
        || receipt.runtime_snapshot_digest.as_deref() != Some(receipt.entity_digest.as_str())
    {
        return Err(QueryError::tail_invalid());
    }
    validate_runtime_live_rows(identity, &live.provenance, &live.rows)
}

pub(crate) fn compose_runtime_tail(
    identity: &QueryIdentity,
    base_facts: &[ProjectionFact],
    tail: RuntimeTail,
) -> QueryResult<(Vec<ProjectionFact>, ProjectionModels)> {
    let is_last_good_fallback = tail.rows.is_empty()
        && tail.live.is_none()
        && tail.through_change_id == identity.published_through_change_id;
    let is_live_only = tail.rows.is_empty() && tail.live.is_some();
    if tail.from_change_id != identity.published_through_change_id.saturating_add(1)
        || (tail.through_change_id != identity.checkpoint
            && !is_last_good_fallback
            && !is_live_only)
    {
        return Err(QueryError::tail_invalid());
    }
    validate_runtime_tail_provenance(identity, &tail, is_last_good_fallback)?;
    validate_runtime_live_snapshot(identity, &tail)?;
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
    let mut stable_entities = BTreeMap::<String, (String, String)>::new();
    for fact in facts.values() {
        if let Some(stable_key) = stable_fact_entity_key(fact)?
            && stable_entities
                .insert(
                    stable_key,
                    (fact.entity_kind.clone(), fact.entity_key.clone()),
                )
                .is_some()
        {
            return Err(QueryError::snapshot_inconsistent());
        }
    }
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
                if let Some(stable_key) = stable_fact_entity_key(&fact)? {
                    stable_entities.insert(stable_key, entity.clone());
                }
                facts.insert(entity, fact);
            }
            "logical_revoke" => {
                facts.remove(&entity);
            }
            _ => return Err(QueryError::tail_invalid()),
        }
        expected_change = expected_change.saturating_add(1);
    }
    if let Some(live) = tail.live.as_ref() {
        for row in &live.rows {
            let stable_key = stable_live_entity_key(row)?.ok_or_else(QueryError::tail_invalid)?;
            if stable_entities.contains_key(&stable_key) {
                return Err(QueryError::tail_invalid());
            }
            let entity = (row.entity_kind.clone(), row.entity_key.clone());
            let fact = normalize_tail_fact(&row.entity_kind, &row.entity_key, &row.payload)?;
            if facts.insert(entity.clone(), fact).is_some() {
                return Err(QueryError::tail_invalid());
            }
            stable_entities.insert(stable_key, entity);
        }
    }
    if !is_last_good_fallback
        && !is_live_only
        && expected_change != identity.checkpoint.saturating_add(1)
    {
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

/// Complete one response handoff after the durable prefix and its coverage admission were read
/// from the same backend snapshot. A caller that has a live source must construct the producer
/// from that exact identity, capture once, and pass the resulting tail here; `compose_runtime_tail`
/// verifies the source/epoch fence before folding it. Seed models and coverage admission are
/// retained while live facts are recomputed. A live capture error must use the last-good durable
/// tail (stale when `C > P`) or return unavailable; it must not reuse a prior live snapshot. The
/// cache key includes the live snapshot revision and entity digest, so a changed live snapshot
/// cannot reuse an older response identity.
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
    let has_unpublished_contribution = !tail.rows.is_empty()
        || tail
            .live
            .as_ref()
            .is_some_and(|snapshot| !snapshot.rows.is_empty());
    if !has_unpublished_contribution {
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
    let cache_key = QueryCacheKey::for_response(request, &identity, tail.live.as_ref())?;
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

/// Execute a consumer-range request through the same SQLite publication, coverage, tail,
/// freshness, and cache-identity path as the legacy long-term entry point.
pub(crate) async fn query_sqlite_snapshot_for_consumer_range(
    pool: &Pool<Sqlite>,
    request: &AnalyticsQueryRequest,
    consumer_range: AnalyticsQueryRange,
    loader: &dyn SqlitePrefixLoader,
) -> QueryResult<AnalyticsQueryResponse> {
    query_sqlite_snapshot(
        pool,
        &request.clone().with_consumer_range(consumer_range),
        loader,
    )
    .await
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
    let result = query_postgres_snapshot_in_transaction(&mut tx, request).await;
    if result.is_ok() {
        tx.commit().await.map_err(|_| QueryError::unavailable())?;
    } else {
        let _ = tx.rollback().await;
    }
    result
}

/// Read one PostgreSQL publication snapshot through a caller-owned repeatable-read transaction.
///
/// The caller must have already started and configured the transaction, and is responsible for
/// its final commit or rollback. This function never begins, configures, commits, rolls back, or
/// otherwise replaces the supplied transaction, so an admitted C8 read permit can remain held
/// through the caller's response handoff and transaction completion.
pub(crate) async fn query_postgres_snapshot_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    request: &AnalyticsQueryRequest,
) -> QueryResult<AnalyticsQueryResponse> {
    request.validate()?;
    let token = projection::current_publication(&mut *tx, &request.consumer_id)
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
    .fetch_optional(&mut **tx)
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
    let models = projection::models_at(tx, &token)
        .await
        .map_err(|_| QueryError::publication_unavailable())?;
    let facts = projection::facts_at(tx, &token)
        .await
        .map_err(|_| QueryError::publication_unavailable())?;
    let tail = last_good_tail_for_persistent_snapshot(&identity)?;
    let coverage = coverage_from_json(models.history_coverage.as_ref())?;
    assemble_response(
        request,
        identity,
        facts,
        models,
        tail,
        coverage,
        Some(source_updated_at_ms),
    )
}

/// Execute a consumer-range request through the same PostgreSQL publication, coverage, tail,
/// freshness, and cache-identity path as the legacy long-term entry point.
pub(crate) async fn query_postgres_snapshot_for_consumer_range(
    pool: &PgPool,
    request: &AnalyticsQueryRequest,
    consumer_range: AnalyticsQueryRange,
) -> QueryResult<AnalyticsQueryResponse> {
    query_postgres_snapshot(pool, &request.clone().with_consumer_range(consumer_range)).await
}

/// Read a consumer-range request through a caller-owned PostgreSQL publication transaction.
/// The transaction lifecycle remains with the caller, just like the base snapshot seam.
pub(crate) async fn query_postgres_snapshot_for_consumer_range_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    request: &AnalyticsQueryRequest,
    consumer_range: AnalyticsQueryRange,
) -> QueryResult<AnalyticsQueryResponse> {
    query_postgres_snapshot_in_transaction(tx, &request.clone().with_consumer_range(consumer_range))
        .await
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
            consumer_range: None,
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

    fn fact_with_runtime_identity(
        storage_key: &str,
        invoke_id: &str,
        occurred_at: &str,
        tokens: i64,
    ) -> ProjectionFact {
        normalize_tail_fact(
            "invocation",
            storage_key,
            &json!({
                "factVersion": 1,
                "invokeId": invoke_id,
                "occurredAt": occurred_at,
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
            live: None,
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
        missing_provenance.receipt = None;
        assert_eq!(
            compose_runtime_tail(&identity(), &[], missing_provenance)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );

        let mut wrong_claim = runtime_tail(vec![row(2, "row-a", "upsert", payload.clone())]);
        wrong_claim
            .live
            .as_mut()
            .expect("live snapshot")
            .provenance
            .exclusive_claim_id = "".into();
        assert_eq!(
            compose_runtime_tail(&identity(), &[], wrong_claim)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );

        let mut changed_row = runtime_tail(vec![row(2, "row-a", "upsert", payload.clone())]);
        changed_row.live.as_mut().expect("live snapshot").rows[0].payload["totalTokens"] =
            json!(99);
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

        let live_row = &tail.live.as_ref().expect("live snapshot").rows[0];
        let invoke_id = live_row
            .payload
            .get("invokeId")
            .and_then(Value::as_str)
            .expect("runtime invoke id");
        let overlapping_base =
            fact_with_runtime_identity("invocation:proxy:1", invoke_id, &live_row.occurred_at, 1);
        assert_eq!(
            compose_runtime_tail(&identity(), &[overlapping_base], tail)
                .unwrap_err()
                .code,
            QueryErrorCode::TailInvalid
        );
    }

    #[test]
    fn runtime_snapshot_is_independent_of_persistent_publication_delta() {
        let live_row = row(
            0,
            "runtime-row-independent",
            "upsert",
            json!({
                "factVersion": 1,
                "occurredAt": "2026-11-01T00:00:00Z",
                "successLike": false,
                "totalTokens": 3
            }),
        );

        let mut published_only = identity();
        published_only.checkpoint = published_only.published_through_change_id;
        let hub = RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        hub.upsert(runtime_record(&live_row));
        let mut producer =
            RuntimeTailProducer::new(published_only.clone(), &hub, "runtime-stream-p".into())
                .unwrap();
        producer.claim("claim-published-only".into()).unwrap();
        let tail = producer.finish().unwrap();
        assert_eq!(
            tail.through_change_id,
            published_only.published_through_change_id
        );
        assert!(tail.rows.is_empty());
        assert_eq!(tail.live.as_ref().unwrap().rows.len(), 1);
        let first_cache_key =
            QueryCacheKey::for_response(&request(), &published_only, tail.live.as_ref()).unwrap();
        compose_runtime_tail(&published_only, &[], tail).unwrap();

        let second_live_row = row(
            0,
            "runtime-row-independent-2",
            "upsert",
            json!({
                "factVersion": 1,
                "occurredAt": "2026-11-01T00:00:01Z",
                "successLike": false,
                "totalTokens": 4
            }),
        );
        hub.upsert(runtime_record(&second_live_row));
        let mut second_producer =
            RuntimeTailProducer::new(published_only.clone(), &hub, "runtime-stream-p".into())
                .unwrap();
        second_producer
            .claim("claim-published-only-2".into())
            .unwrap();
        let second_tail = second_producer.finish().unwrap();
        let second_cache_key =
            QueryCacheKey::for_response(&request(), &published_only, second_tail.live.as_ref())
                .unwrap();
        assert_ne!(first_cache_key.digest, second_cache_key.digest);
        assert_ne!(
            first_cache_key.live_snapshot_digest,
            second_cache_key.live_snapshot_digest
        );

        let mut unpublished_gap = identity();
        unpublished_gap.checkpoint = unpublished_gap.published_through_change_id + 2;
        let empty_hub = RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        let mut empty_producer = RuntimeTailProducer::new(
            unpublished_gap.clone(),
            &empty_hub,
            "runtime-stream-e".into(),
        )
        .unwrap();
        empty_producer.claim("claim-empty-live".into()).unwrap();
        let empty_tail = empty_producer.finish().unwrap();
        assert!(empty_tail.rows.is_empty());
        assert!(empty_tail.live.as_ref().unwrap().rows.is_empty());
        compose_runtime_tail(&unpublished_gap, &[], empty_tail).unwrap();

        let non_matching_hub =
            RuntimeProjectionHub::new(crate::app_state::RuntimeProjectionMode::Auto);
        non_matching_hub.upsert(runtime_record(&live_row));
        let mut non_matching_producer = RuntimeTailProducer::new(
            unpublished_gap.clone(),
            &non_matching_hub,
            "runtime-stream-n".into(),
        )
        .unwrap();
        non_matching_producer
            .claim("claim-one-of-three".into())
            .unwrap();
        let non_matching_tail = non_matching_producer.finish().unwrap();
        assert_eq!(non_matching_tail.live.as_ref().unwrap().rows.len(), 1);
        compose_runtime_tail(&unpublished_gap, &[], non_matching_tail).unwrap();
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

    #[tokio::test]
    #[ignore = "requires an owner-created temporary PostgreSQL 18 reader fixture"]
    async fn postgres_caller_owned_snapshot_leaves_transaction_with_caller() {
        let url = std::env::var("C6_PG_QUERY_APP_URL").expect("C6_PG_QUERY_APP_URL");
        let case = std::env::var("C6_PG_QUERY_CASE").expect("C6_PG_QUERY_CASE");
        let pool = PgPool::connect(&url).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .unwrap();

        let result = query_postgres_snapshot_in_transaction(&mut tx, &request()).await;
        match case.as_str() {
            "published" | "unpublished" => {
                result.unwrap();
                let sentinel: i32 = sqlx::query_scalar("SELECT 1")
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
                assert_eq!(sentinel, 1);
                tx.commit().await.unwrap();
            }
            "blocked" => {
                assert_eq!(
                    result.unwrap_err().code,
                    QueryErrorCode::PublicationUnavailable
                );
                tx.rollback().await.unwrap();
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
        let mut gap = sealed_test_tail(vec![row(2, "row-a", "upsert", json!({"factVersion": 1}))]);
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
    fn consumer_ranges_are_typed_and_explicit_ranges_need_coverage() {
        let mut all = request().with_consumer_range(AnalyticsQueryRange::all());
        assert_eq!(all.range, "all");
        assert!(all.validate().is_ok());

        all = request().with_consumer_range(AnalyticsQueryRange::current(10));
        assert!(all.validate().is_ok());

        all = request().with_consumer_range(AnalyticsQueryRange::current(0));
        assert_eq!(
            all.validate().unwrap_err().code,
            QueryErrorCode::InvalidQuery
        );

        all = request()
            .with_consumer_range(AnalyticsQueryRange::explicit("2026-11-02", "2026-11-04"));
        assert!(all.validate().is_ok());

        all = request()
            .with_consumer_range(AnalyticsQueryRange::explicit("2026-10-31", "2026-11-04"));
        assert_eq!(
            all.validate().unwrap_err().code,
            QueryErrorCode::InvalidQuery
        );

        let mut mismatched = request();
        mismatched.consumer_range = Some(AnalyticsQueryRange::all());
        assert_eq!(
            mismatched.validate().unwrap_err().code,
            QueryErrorCode::InvalidQuery
        );
    }

    #[test]
    fn consumer_range_is_part_of_the_shared_cache_identity() {
        let legacy = QueryCacheKey::for_request(&request(), &identity()).unwrap();
        let all = QueryCacheKey::for_request(
            &request().with_consumer_range(AnalyticsQueryRange::all()),
            &identity(),
        )
        .unwrap();
        let current = QueryCacheKey::for_request(
            &request().with_consumer_range(AnalyticsQueryRange::current(10)),
            &identity(),
        )
        .unwrap();
        let explicit = QueryCacheKey::for_request(
            &request()
                .with_consumer_range(AnalyticsQueryRange::explicit("2026-11-02", "2026-11-04")),
            &identity(),
        )
        .unwrap();

        assert_ne!(legacy.digest, all.digest);
        assert_ne!(all.digest, current.digest);
        assert_ne!(current.digest, explicit.digest);
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
