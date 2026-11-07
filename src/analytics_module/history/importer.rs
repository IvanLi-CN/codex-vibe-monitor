use super::coverage::{CoverageKey, CoverageRange};
use super::snapshot::{ArtifactIdentity, ArtifactKind, BuildSnapshot, SourceFence, sha256_hex};
use anyhow::{Result, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_MAX_ROWS: usize = 256;
const DEFAULT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoundedImport {
    pub(crate) max_rows: usize,
    pub(crate) max_bytes: usize,
}

impl Default for BoundedImport {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_MAX_ROWS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl BoundedImport {
    pub(crate) fn validate(self) -> Result<()> {
        ensure!(
            self.max_rows > 0,
            "history import row bound must be positive"
        );
        ensure!(
            self.max_bytes > 0,
            "history import byte bound must be positive"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImportSource {
    Live,
    Archive,
}

impl ImportSource {
    fn artifact_kind(self) -> ArtifactKind {
        match self {
            Self::Live => ArtifactKind::Live,
            Self::Archive => ArtifactKind::Archive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StableEntityIdentity {
    pub(crate) source_instance_id: String,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
}

impl StableEntityIdentity {
    pub(crate) fn validate(&self) -> Result<()> {
        for (value, label) in [
            (&self.source_instance_id, "history source instance id"),
            (&self.entity_kind, "history entity kind"),
            (&self.entity_key, "history entity key"),
        ] {
            ensure!(!value.trim().is_empty(), "{label} is empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NormalizedRecordKind {
    Invocation,
    PoolAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceOperation {
    Upsert,
    Correction,
    Tombstone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NormalizedFactPayload {
    pub(crate) fact_version: i16,
    pub(crate) record_kind: NormalizedRecordKind,
    pub(crate) source: String,
    pub(crate) source_row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) invocation_id: String,
    pub(crate) entity_key: String,
    pub(crate) attempt_index: Option<i32>,
    pub(crate) occurred_at: String,
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) occurred_at_precision_ms: i64,
    pub(crate) stats_date: String,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) prompt_cache_key_digest: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) success_like: bool,
    pub(crate) terminal: bool,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<bool>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) raw_payload: serde_json::Value,
}

impl NormalizedFactPayload {
    pub(crate) fn canonicalize_timestamp(value: &str) -> Result<(String, i64, i64)> {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
            let epoch_ms = parsed.timestamp_millis();
            return Ok((
                parsed
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
                epoch_ms,
                1,
            ));
        }
        let numeric = value
            .trim()
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("legacy timestamp is invalid: {error}"))?;
        ensure!(
            numeric.is_finite() && numeric >= 0.0,
            "legacy timestamp is invalid"
        );
        let magnitude = numeric.abs();
        let (epoch_ms, precision_ms) = if magnitude >= 1.0e14 {
            ((numeric / 1_000.0).round() as i64, 1)
        } else if magnitude >= 1.0e11 {
            (numeric.round() as i64, 1)
        } else {
            ((numeric * 1_000.0).round() as i64, 1_000)
        };
        let occurred_at = DateTime::<Utc>::from_timestamp_millis(epoch_ms)
            .ok_or_else(|| anyhow::anyhow!("legacy timestamp is outside supported range"))?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        Ok((occurred_at, epoch_ms, precision_ms))
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.fact_version > 0,
            "normalized fact version must be positive"
        );
        ensure!(
            !self.source.trim().is_empty(),
            "normalized fact source is empty"
        );
        ensure!(
            self.source_row_id > 0,
            "normalized fact source row id must be positive"
        );
        ensure!(
            !self.invoke_id.trim().is_empty(),
            "normalized fact invoke id is empty"
        );
        ensure!(
            !self.invocation_id.trim().is_empty(),
            "normalized fact invocation id is empty"
        );
        match self.record_kind {
            NormalizedRecordKind::Invocation => ensure!(
                self.attempt_index.is_none(),
                "invocation fact cannot carry an attempt index"
            ),
            NormalizedRecordKind::PoolAttempt => ensure!(
                self.attempt_index.is_some_and(|index| index >= 0),
                "attempt fact requires a non-negative attempt index"
            ),
        }
        ensure!(
            !self.entity_key.trim().is_empty(),
            "normalized fact entity key is empty"
        );
        let occurred_at = DateTime::parse_from_rfc3339(&self.occurred_at)
            .map_err(|error| anyhow::anyhow!("normalized fact occurred_at is invalid: {error}"))?;
        ensure!(
            occurred_at.timestamp_millis() == self.occurred_at_epoch_ms,
            "normalized fact timestamp precision does not match occurred_at"
        );
        ensure!(
            self.occurred_at_precision_ms > 0
                && self.occurred_at_precision_ms <= 1_000
                && self.occurred_at_epoch_ms % self.occurred_at_precision_ms == 0,
            "normalized fact timestamp precision is invalid"
        );
        ensure!(
            self.stats_date.len() == 10
                && self.stats_date.as_bytes().get(4) == Some(&b'-')
                && self.stats_date.as_bytes().get(7) == Some(&b'-'),
            "normalized fact stats date is invalid"
        );
        ensure!(
            self.raw_payload.is_object(),
            "normalized fact raw payload must be an object"
        );
        let mut typed_payload = serde_json::to_value(self)?;
        typed_payload
            .as_object_mut()
            .expect("normalized fact payload is an object")
            .remove("rawPayload");
        ensure!(
            self.raw_payload == typed_payload,
            "normalized fact raw payload is not the canonical typed payload"
        );
        for (value, label) in [
            (self.input_tokens, "input tokens"),
            (self.output_tokens, "output tokens"),
            (self.cache_input_tokens, "cache input tokens"),
            (self.reasoning_tokens, "reasoning tokens"),
            (self.total_tokens, "total tokens"),
        ] {
            if let Some(value) = value {
                ensure!(value >= 0, "normalized fact {label} must not be negative");
            }
        }
        if let (Some(input), Some(output), Some(total)) =
            (self.input_tokens, self.output_tokens, self.total_tokens)
        {
            ensure!(
                input.checked_add(output) == Some(total),
                "normalized fact token total is inconsistent"
            );
        }
        for (value, label) in [
            (self.cost, "cost"),
            (self.cost_input, "input cost"),
            (self.cost_cache_write, "cache write cost"),
            (self.cost_cache_read, "cache read cost"),
            (self.cost_output, "output cost"),
            (self.cost_reasoning, "reasoning cost"),
            (self.t_total_ms, "total duration"),
            (self.t_req_read_ms, "request read duration"),
            (self.t_req_parse_ms, "request parse duration"),
            (self.t_upstream_connect_ms, "upstream connect duration"),
            (self.t_upstream_ttfb_ms, "upstream ttfb duration"),
            (self.t_upstream_stream_ms, "upstream stream duration"),
            (self.first_token_ms, "first token duration"),
            (self.t_resp_parse_ms, "response parse duration"),
        ] {
            if let Some(value) = value {
                ensure!(
                    value.is_finite() && value >= 0.0,
                    "normalized fact {label} is invalid"
                );
            }
        }
        if let Some(digest) = &self.prompt_cache_key_digest {
            ensure!(
                is_digest(digest),
                "normalized fact prompt cache digest is invalid"
            );
        }
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }

    pub(crate) fn entity_kind(&self) -> &str {
        match self.record_kind {
            NormalizedRecordKind::Invocation => "invocation",
            NormalizedRecordKind::PoolAttempt => "pool_attempt",
        }
    }

    pub(crate) fn payload(&self) -> &serde_json::Value {
        &self.raw_payload
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(source: &str, entity_key: &str, source_row_id: i64) -> Self {
        let mut payload = Self {
            fact_version: 1,
            record_kind: NormalizedRecordKind::Invocation,
            source: source.to_string(),
            source_row_id,
            invoke_id: format!("invoke-{entity_key}"),
            invocation_id: entity_key.to_string(),
            entity_key: entity_key.to_string(),
            attempt_index: None,
            occurred_at: "2026-01-01T00:00:00.000Z".to_string(),
            occurred_at_epoch_ms: 1_767_225_600_000,
            occurred_at_precision_ms: 1,
            stats_date: "2026-01-01".to_string(),
            model: None,
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            prompt_cache_key_digest: None,
            upstream_account_id: None,
            upstream_account_name: None,
            status: Some("completed".to_string()),
            success_like: true,
            terminal: true,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            input_tokens: Some(1),
            output_tokens: Some(1),
            cache_input_tokens: Some(0),
            reasoning_tokens: None,
            total_tokens: Some(2),
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            t_total_ms: Some(1.0),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            t_upstream_stream_ms: None,
            first_token_ms: None,
            t_resp_parse_ms: None,
            raw_payload: serde_json::Value::Null,
        };
        let mut canonical = serde_json::to_value(&payload).expect("fixture payload serializes");
        canonical
            .as_object_mut()
            .expect("fixture payload is an object")
            .remove("rawPayload");
        payload.raw_payload = canonical;
        payload
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryFact {
    pub(crate) source: ImportSource,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) artifact_id: String,
    pub(crate) artifact_bytes_digest: String,
    pub(crate) event_id: String,
    pub(crate) identity: StableEntityIdentity,
    pub(crate) bucket: CoverageKey,
    pub(crate) range: CoverageRange,
    pub(crate) effective_change_id: i64,
    pub(crate) operation: SourceOperation,
    pub(crate) payload: NormalizedFactPayload,
    pub(crate) content_digest: String,
    pub(crate) contribution_key: String,
    pub(crate) contribution_digest: String,
}

impl HistoryFact {
    pub(crate) fn validate_against(
        &self,
        source: &SourceFence,
        artifact: &HistoryArtifact,
    ) -> Result<()> {
        self.identity.validate()?;
        self.payload.validate()?;
        ensure!(
            self.source == artifact.source_kind(),
            "history fact source does not match artifact"
        );
        ensure!(
            self.source_instance_id == source.source_instance_id,
            "history fact belongs to another source instance"
        );
        ensure!(
            self.identity.source_instance_id == source.source_instance_id,
            "history entity belongs to another source instance"
        );
        ensure!(
            self.generation == source.generation,
            "history fact generation changed"
        );
        ensure!(
            !self.event_id.trim().is_empty(),
            "history event id is empty"
        );
        ensure!(
            self.effective_change_id >= 1,
            "history effective change id must be positive"
        );
        ensure!(
            !self.contribution_key.trim().is_empty(),
            "history contribution key is empty"
        );
        ensure!(
            is_digest(&self.content_digest),
            "history content digest is invalid"
        );
        ensure!(
            is_digest(&self.contribution_digest),
            "history contribution digest is invalid"
        );
        ensure!(
            self.payload.source == self.source.as_str(),
            "normalized fact source does not match history source"
        );
        ensure!(
            self.identity.entity_kind == self.payload.entity_kind(),
            "history entity kind is not bound to normalized payload"
        );
        ensure!(
            self.identity.entity_key == self.payload.entity_key,
            "history entity key is not bound to normalized payload"
        );
        ensure!(
            !self.artifact_id.trim().is_empty(),
            "history fact artifact id is empty"
        );
        ensure!(
            is_digest(&self.artifact_bytes_digest),
            "history fact artifact bytes digest is invalid"
        );
        ensure!(
            self.effective_change_id <= source.high_watermark_change_id,
            "history fact effective change is beyond the source watermark"
        );
        ensure!(
            self.payload.source_row_id > 0,
            "normalized fact source row id must be positive"
        );
        ensure!(
            self.payload.digest()? == self.content_digest,
            "history content digest is not bound to normalized payload"
        );
        CoverageRange::new(self.range.start.clone(), self.range.end.clone())?;
        Ok(())
    }

    pub(crate) fn validate_against_source(&self, source: &SourceFence) -> Result<()> {
        self.identity.validate()?;
        self.payload.validate()?;
        ensure!(
            self.source_instance_id == source.source_instance_id
                && self.identity.source_instance_id == source.source_instance_id,
            "history fact belongs to another source instance"
        );
        ensure!(
            !self.artifact_id.trim().is_empty(),
            "history fact artifact id is empty"
        );
        ensure!(
            is_digest(&self.artifact_bytes_digest),
            "history fact artifact bytes digest is invalid"
        );
        ensure!(
            self.generation == source.generation,
            "history fact generation changed"
        );
        ensure!(
            self.effective_change_id >= 1
                && self.effective_change_id <= source.high_watermark_change_id,
            "history fact effective change is outside the source watermark"
        );
        ensure!(
            !self.event_id.trim().is_empty(),
            "history event id is empty"
        );
        ensure!(
            !self.contribution_key.trim().is_empty(),
            "history contribution key is empty"
        );
        ensure!(
            is_digest(&self.content_digest),
            "history content digest is invalid"
        );
        ensure!(
            is_digest(&self.contribution_digest),
            "history contribution digest is invalid"
        );
        ensure!(
            self.payload.source == self.source.as_str()
                && self.identity.entity_kind == self.payload.entity_kind()
                && self.identity.entity_key == self.payload.entity_key,
            "history fact identity is not bound to normalized payload"
        );
        ensure!(
            self.payload.digest()? == self.content_digest,
            "history content digest is not bound to normalized payload"
        );
        self.bucket.validate()?;
        CoverageRange::new(self.range.start.clone(), self.range.end.clone())?;
        Ok(())
    }

    fn signature(&self) -> Result<String> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }

    pub(crate) fn identity(&self) -> &StableEntityIdentity {
        &self.identity
    }

    pub(crate) fn effective_change_id(&self) -> i64 {
        self.effective_change_id
    }

    pub(crate) fn operation(&self) -> SourceOperation {
        self.operation
    }

    pub(crate) fn normalized_payload(&self) -> &NormalizedFactPayload {
        &self.payload
    }

    pub(crate) fn content_digest(&self) -> &str {
        &self.content_digest
    }
}

impl ImportSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Archive => "archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoryArtifact {
    identity: ArtifactIdentity,
    verified_bytes_digest: Option<String>,
    decoded_facts: Vec<HistoryFact>,
    decoded_facts_digest: Option<String>,
}

impl HistoryArtifact {
    pub(crate) fn new(identity: ArtifactIdentity, source: &SourceFence) -> Result<Self> {
        identity.validate_against(source)?;
        ensure!(
            matches!(identity.kind, ArtifactKind::Live | ArtifactKind::Archive),
            "history import artifact must be live or archive"
        );
        Ok(Self {
            identity,
            verified_bytes_digest: None,
            decoded_facts: Vec::new(),
            decoded_facts_digest: None,
        })
    }

    pub(crate) fn from_verified_artifact(
        artifact: &super::snapshot::StableArtifact,
        snapshot: &BuildSnapshot,
        kind: ArtifactKind,
    ) -> Result<Self> {
        snapshot.validate()?;
        let identity = ArtifactIdentity {
            artifact_id: artifact.identity.manifest_id.clone(),
            kind,
            source_instance_id: artifact.identity.source_instance_id.clone(),
            generation: artifact.identity.generation,
            manifest: artifact.identity.clone(),
        };
        let history_artifact = Self::new(identity, &snapshot.source)?;
        ensure!(
            snapshot
                .manifest_identities
                .iter()
                .any(|manifest| manifest.same_identity(&history_artifact.identity.manifest)),
            "verified history artifact manifest identity is not pinned by the snapshot"
        );
        let bytes_digest = sha256_hex(&artifact.bytes);
        ensure!(
            bytes_digest == artifact.identity.sha256,
            "verified history artifact bytes changed after manifest validation"
        );
        let mut decoded_facts: Vec<HistoryFact> =
            serde_json::from_slice(&artifact.bytes).map_err(|error| {
                anyhow::anyhow!("verified history artifact facts are invalid JSON: {error}")
            })?;
        let expected_source = match kind {
            ArtifactKind::Live => ImportSource::Live,
            ArtifactKind::Archive => ImportSource::Archive,
            ArtifactKind::Seed => unreachable!("seed artifact cannot import facts"),
        };
        for fact in &mut decoded_facts {
            fact.artifact_id = history_artifact.identity.artifact_id.clone();
            fact.artifact_bytes_digest = bytes_digest.clone();
            fact.validate_against_source(&snapshot.source)?;
            ensure!(
                fact.source == expected_source,
                "verified history artifact fact source does not match artifact kind"
            );
        }
        let decoded_facts_digest = Some(facts_digest(&decoded_facts)?);
        Ok(Self {
            verified_bytes_digest: Some(bytes_digest),
            decoded_facts,
            decoded_facts_digest,
            ..history_artifact
        })
    }

    pub(crate) fn new_for_snapshot(
        identity: ArtifactIdentity,
        snapshot: &BuildSnapshot,
    ) -> Result<Self> {
        snapshot.validate()?;
        let artifact = Self::new(identity, &snapshot.source)?;
        ensure!(
            snapshot
                .manifest_identities
                .iter()
                .any(|manifest| manifest.same_identity(&artifact.identity.manifest)),
            "history artifact manifest identity is not pinned by the snapshot"
        );
        Ok(artifact)
    }

    pub(crate) fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn verified_bytes_digest(&self) -> Result<&str> {
        self.verified_bytes_digest
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("history artifact lacks verified bytes evidence"))
    }

    fn decoded_facts(&self) -> &[HistoryFact] {
        &self.decoded_facts
    }

    pub(crate) fn decoded_facts_digest(&self) -> Result<&str> {
        self.decoded_facts_digest
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("history artifact lacks decoded fact evidence"))
    }

    fn source_kind(&self) -> ImportSource {
        match self.identity.kind {
            ArtifactKind::Live => ImportSource::Live,
            ArtifactKind::Archive => ImportSource::Archive,
            ArtifactKind::Seed => unreachable!("seed artifacts are not fact import artifacts"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCursor {
    pub(crate) snapshot_id: String,
    pub(crate) artifact_id: String,
    pub(crate) manifest_id: String,
    pub(crate) manifest_sha256: String,
    pub(crate) artifact_bytes_digest: String,
    pub(crate) next_key: Option<String>,
}

impl ImportCursor {
    fn for_start(snapshot: &BuildSnapshot, artifact: &HistoryArtifact) -> Result<Self> {
        Ok(Self {
            snapshot_id: snapshot.snapshot_id()?,
            artifact_id: artifact.identity.artifact_id.clone(),
            manifest_id: artifact.identity.manifest.manifest_id.clone(),
            manifest_sha256: artifact.identity.manifest.sha256.clone(),
            artifact_bytes_digest: artifact.verified_bytes_digest()?.to_string(),
            next_key: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportOutcome {
    pub(crate) inserted: usize,
    pub(crate) duplicates: usize,
    pub(crate) next_cursor_present: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryImporter {
    snapshot: BuildSnapshot,
    artifact: HistoryArtifact,
    bounds: BoundedImport,
    entities: BTreeMap<(StableEntityIdentity, i64), HistoryFact>,
    event_ids: BTreeMap<String, String>,
    contributions: BTreeMap<String, String>,
    cursor: ImportCursor,
}

impl HistoryImporter {
    pub(crate) fn new(
        snapshot: BuildSnapshot,
        artifact: HistoryArtifact,
        bounds: BoundedImport,
    ) -> Result<Self> {
        snapshot.validate()?;
        bounds.validate()?;
        artifact.identity.validate_against(&snapshot.source)?;
        artifact.verified_bytes_digest()?;
        artifact.decoded_facts_digest()?;
        ensure!(
            snapshot
                .manifest_identities
                .iter()
                .any(|manifest| manifest.same_identity(&artifact.identity.manifest)),
            "history artifact manifest identity is not pinned by the snapshot"
        );
        let cursor = ImportCursor::for_start(&snapshot, &artifact)?;
        Ok(Self {
            snapshot,
            artifact,
            bounds,
            entities: BTreeMap::new(),
            event_ids: BTreeMap::new(),
            contributions: BTreeMap::new(),
            cursor,
        })
    }

    pub(crate) fn cursor(&self) -> &ImportCursor {
        &self.cursor
    }

    pub(crate) fn imported_facts(&self) -> impl Iterator<Item = &HistoryFact> {
        self.entities.values()
    }

    pub(crate) fn resume(&mut self, cursor: ImportCursor) -> Result<()> {
        let expected = ImportCursor::for_start(&self.snapshot, &self.artifact)?;
        ensure!(
            cursor.snapshot_id == expected.snapshot_id,
            "import cursor belongs to another snapshot"
        );
        ensure!(
            cursor.artifact_id == expected.artifact_id,
            "import cursor belongs to another artifact"
        );
        ensure!(
            cursor.manifest_id == expected.manifest_id,
            "import cursor manifest identity changed"
        );
        ensure!(
            cursor.manifest_sha256 == expected.manifest_sha256,
            "import cursor manifest digest changed"
        );
        ensure!(
            cursor.artifact_bytes_digest == expected.artifact_bytes_digest,
            "import cursor artifact bytes digest changed"
        );
        self.cursor = cursor;
        Ok(())
    }

    pub(crate) fn import_page(
        &mut self,
        cursor: ImportCursor,
        facts: &[HistoryFact],
        next_key: Option<String>,
    ) -> Result<ImportOutcome> {
        self.ensure_artifact_pinned()?;
        self.resume(cursor)?;
        ensure!(
            facts.len() <= self.bounds.max_rows,
            "history import page exceeds row bound"
        );
        let encoded_bytes = facts.iter().try_fold(0usize, |total, fact| {
            let bytes = serde_json::to_vec(fact)?.len();
            total
                .checked_add(bytes)
                .ok_or_else(|| anyhow::anyhow!("history import byte count overflow"))
        })?;
        ensure!(
            encoded_bytes <= self.bounds.max_bytes,
            "history import page exceeds byte bound"
        );
        if let Some(key) = &next_key {
            ensure!(!key.trim().is_empty(), "history import cursor key is empty");
        }

        let mut inserted = 0;
        let mut duplicates = 0;
        for fact in facts {
            ensure!(
                self.artifact
                    .decoded_facts()
                    .iter()
                    .any(|decoded| decoded == fact),
                "history fact is not decoded from the verified artifact bytes"
            );
        }
        for fact in facts {
            fact.validate_against(&self.snapshot.source, &self.artifact)?;
            ensure!(
                fact.artifact_id == self.artifact.identity.artifact_id
                    && fact.artifact_bytes_digest == self.artifact.verified_bytes_digest()?,
                "history fact is not bound to verified artifact bytes"
            );
            match self.insert_fact(fact.clone())? {
                true => inserted += 1,
                false => duplicates += 1,
            }
        }
        self.cursor.next_key = next_key;
        Ok(ImportOutcome {
            inserted,
            duplicates,
            next_cursor_present: self.cursor.next_key.is_some(),
        })
    }

    fn insert_fact(&mut self, fact: HistoryFact) -> Result<bool> {
        let signature = fact.signature()?;
        if let Some(existing_signature) = self.event_ids.get(&fact.event_id) {
            ensure!(
                existing_signature == &signature,
                "history event identity has conflicting content"
            );
            return Ok(false);
        }
        let entity_key = (fact.identity.clone(), fact.effective_change_id);
        if let Some(existing) = self.entities.get(&entity_key) {
            ensure!(
                existing.signature()? == signature,
                "live/archive entity identity has conflicting content"
            );
            self.event_ids.insert(fact.event_id, signature);
            return Ok(false);
        }
        if let Some(existing_signature) = self.contributions.get(&fact.contribution_key) {
            ensure!(
                existing_signature == &signature,
                "live/archive facts would contribute the same history unit twice"
            );
            return Ok(false);
        }
        self.event_ids.insert(fact.event_id.clone(), signature);
        self.contributions
            .insert(fact.contribution_key.clone(), fact.signature()?);
        self.entities.insert(entity_key, fact);
        Ok(true)
    }

    fn ensure_artifact_pinned(&self) -> Result<()> {
        self.snapshot.validate()?;
        self.artifact
            .identity
            .validate_against(&self.snapshot.source)?;
        self.artifact.verified_bytes_digest()?;
        self.artifact.decoded_facts_digest()?;
        ensure!(
            self.snapshot
                .manifest_identities
                .iter()
                .any(|manifest| manifest.same_identity(&self.artifact.identity.manifest)),
            "history artifact manifest identity is not pinned by the snapshot"
        );
        Ok(())
    }
}

pub(crate) fn facts_digest(facts: &[HistoryFact]) -> Result<String> {
    let mut signatures = facts
        .iter()
        .map(HistoryFact::signature)
        .collect::<Result<Vec<_>>>()?;
    signatures.sort();
    Ok(sha256_hex(&serde_json::to_vec(&signatures)?))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{ArtifactKind, BuildSnapshot, ManifestIdentity, SourceFence};
    use super::*;

    fn snapshot(bytes: &[u8]) -> BuildSnapshot {
        let manifest = ManifestIdentity::from_bytes("manifest-1", "source-1", 4, 4, bytes).unwrap();
        BuildSnapshot {
            build_id: "build-1".to_string(),
            source: SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 4,
                owner_epoch: 2,
                high_watermark_change_id: 4,
                coverage_revision: 4,
                retained_from_change_id: 0,
                fact_semantics_version: 1,
                stats_semantics_version: 2,
            },
            retention_pin_id: "pin-1".to_string(),
            pin_expires_at: "2999-01-01T00:00:00Z".to_string(),
            manifest_identities: vec![manifest],
            recovery_artifact_id: "recovery-1".to_string(),
        }
    }

    fn wire_fact() -> HistoryFact {
        let key = "row-1";
        let payload = NormalizedFactPayload::test_fixture("archive", key, 1);
        HistoryFact {
            source: ImportSource::Archive,
            source_instance_id: "source-1".to_string(),
            generation: 4,
            artifact_id: "wire-artifact".to_string(),
            artifact_bytes_digest:
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
            event_id: "event-1".to_string(),
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "invocation".to_string(),
                entity_key: key.to_string(),
            },
            bucket: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            effective_change_id: 1,
            operation: SourceOperation::Upsert,
            content_digest: payload.digest().unwrap(),
            payload,
            contribution_key: "bucket:row-1".to_string(),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        }
    }

    fn importer() -> HistoryImporter {
        let bytes = serde_json::to_vec(&vec![wire_fact()]).unwrap();
        let snapshot = snapshot(&bytes);
        let manifest = snapshot.manifest_identities[0].clone();
        let artifact = HistoryArtifact::from_verified_artifact(
            &super::super::snapshot::StableArtifact {
                identity: manifest,
                bytes: bytes.to_vec(),
            },
            &snapshot,
            ArtifactKind::Archive,
        )
        .unwrap();
        HistoryImporter::new(snapshot, artifact, BoundedImport::default()).unwrap()
    }

    fn fact(event_id: &str, key: &str, artifact_bytes_digest: &str) -> HistoryFact {
        let payload = NormalizedFactPayload::test_fixture("archive", key, 1);
        let content_digest = payload.digest().unwrap();
        HistoryFact {
            source: ImportSource::Archive,
            source_instance_id: "source-1".to_string(),
            generation: 4,
            artifact_id: "manifest-1".to_string(),
            artifact_bytes_digest: artifact_bytes_digest.to_string(),
            event_id: event_id.to_string(),
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "invocation".to_string(),
                entity_key: key.to_string(),
            },
            bucket: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            effective_change_id: 1,
            operation: SourceOperation::Upsert,
            payload,
            content_digest,
            contribution_key: format!("bucket:{key}"),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        }
    }

    #[test]
    fn live_and_archive_duplicate_entity_contributes_once() {
        let mut importer = importer();
        let digest = importer
            .artifact
            .verified_bytes_digest()
            .unwrap()
            .to_string();
        let first = fact("event-1", "row-1", &digest);
        let mut duplicate = first.clone();
        duplicate.source = ImportSource::Archive;
        let cursor = importer.cursor().clone();
        let result = importer
            .import_page(cursor, &[first, duplicate], None)
            .unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.duplicates, 1);
    }

    #[test]
    fn conflicting_same_entity_is_blocked() {
        let mut importer = importer();
        let digest = importer
            .artifact
            .verified_bytes_digest()
            .unwrap()
            .to_string();
        let first = fact("event-1", "row-1", &digest);
        let mut conflicting = first.clone();
        conflicting.event_id = "event-2".to_string();
        conflicting.payload.status = Some("failed".to_string());
        conflicting.payload.raw_payload = serde_json::to_value(&conflicting.payload).unwrap();
        conflicting
            .payload
            .raw_payload
            .as_object_mut()
            .unwrap()
            .remove("rawPayload");
        conflicting.content_digest = conflicting.payload.digest().unwrap();
        assert!(
            importer
                .import_page(importer.cursor().clone(), &[first, conflicting], None)
                .is_err()
        );
    }

    #[test]
    fn same_source_different_manifest_is_rejected_before_import() {
        let bytes = serde_json::to_vec(&vec![wire_fact()]).unwrap();
        let snapshot = snapshot(&bytes);
        let replacement =
            ManifestIdentity::from_bytes("manifest-replacement", "source-1", 4, 4, b"replacement")
                .unwrap();
        assert!(
            HistoryArtifact::from_verified_artifact(
                &super::super::snapshot::StableArtifact {
                    identity: replacement,
                    bytes: b"replacement".to_vec(),
                },
                &snapshot,
                ArtifactKind::Archive,
            )
            .is_err()
        );
    }

    #[test]
    fn event_and_contribution_conflicts_compare_full_semantics() {
        let mut importer = importer();
        let digest = importer
            .artifact
            .verified_bytes_digest()
            .unwrap()
            .to_string();
        let first = fact("event-1", "row-1", &digest);
        importer.insert_fact(first.clone()).unwrap();

        let mut different_bucket = first.clone();
        different_bucket.bucket = CoverageKey::new("tokens", "overall", "day").unwrap();
        assert!(importer.insert_fact(different_bucket).is_err());

        let mut different_contribution = first.clone();
        different_contribution.event_id = "event-2".to_string();
        different_contribution.contribution_key = "bucket:other".to_string();
        assert!(importer.insert_fact(different_contribution).is_err());
    }

    #[test]
    fn legacy_seconds_and_microseconds_are_canonicalized_once() {
        let (seconds_text, seconds_ms, seconds_precision) =
            NormalizedFactPayload::canonicalize_timestamp("1767225600").unwrap();
        assert_eq!(seconds_text, "2026-01-01T00:00:00.000Z");
        assert_eq!(seconds_ms, 1_767_225_600_000);
        assert_eq!(seconds_precision, 1_000);

        let (micros_text, micros_ms, micros_precision) =
            NormalizedFactPayload::canonicalize_timestamp("1767225600123456").unwrap();
        assert_eq!(micros_text, "2026-01-01T00:00:00.123Z");
        assert_eq!(micros_ms, 1_767_225_600_123);
        assert_eq!(micros_precision, 1);
    }

    #[test]
    fn canonical_payload_rejects_raw_typed_divergence() {
        let mut payload = NormalizedFactPayload::test_fixture("archive", "row-1", 1);
        payload
            .raw_payload
            .as_object_mut()
            .unwrap()
            .insert("cost".to_string(), serde_json::json!(999.0));
        assert!(payload.validate().is_err());
    }

    #[test]
    fn same_artifact_receipt_rejects_replaced_fact_bytes() {
        let mut importer = importer();
        let digest = importer
            .artifact
            .verified_bytes_digest()
            .unwrap()
            .to_string();
        let first = fact("event-1", "row-1", &digest);
        let mut replaced = first.clone();
        replaced.payload.status = Some("failed".to_string());
        replaced.payload.raw_payload = serde_json::to_value(&replaced.payload).unwrap();
        replaced
            .payload
            .raw_payload
            .as_object_mut()
            .unwrap()
            .remove("rawPayload");
        replaced.content_digest = replaced.payload.digest().unwrap();
        assert!(
            importer
                .import_page(importer.cursor().clone(), &[replaced], None)
                .is_err()
        );
    }
}
