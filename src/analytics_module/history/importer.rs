use super::coverage::{CoverageKey, CoverageRange};
use super::snapshot::{ArtifactIdentity, ArtifactKind, BuildSnapshot, SourceFence};
use anyhow::{Result, ensure};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryFact {
    pub(crate) source: ImportSource,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) event_id: String,
    pub(crate) identity: StableEntityIdentity,
    pub(crate) bucket: CoverageKey,
    pub(crate) range: CoverageRange,
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
        CoverageRange::new(self.range.start.clone(), self.range.end.clone())?;
        Ok(())
    }

    fn signature(&self) -> String {
        format!("{}:{}", self.content_digest, self.contribution_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryArtifact {
    identity: ArtifactIdentity,
}

impl HistoryArtifact {
    pub(crate) fn new(identity: ArtifactIdentity, source: &SourceFence) -> Result<Self> {
        identity.validate_against(source)?;
        ensure!(
            matches!(identity.kind, ArtifactKind::Live | ArtifactKind::Archive),
            "history import artifact must be live or archive"
        );
        Ok(Self { identity })
    }

    pub(crate) fn identity(&self) -> &ArtifactIdentity {
        &self.identity
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
    pub(crate) next_key: Option<String>,
}

impl ImportCursor {
    fn for_start(snapshot: &BuildSnapshot, artifact: &HistoryArtifact) -> Result<Self> {
        Ok(Self {
            snapshot_id: snapshot.snapshot_id()?,
            artifact_id: artifact.identity.artifact_id.clone(),
            manifest_id: artifact.identity.manifest.manifest_id.clone(),
            manifest_sha256: artifact.identity.manifest.sha256.clone(),
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
    entities: BTreeMap<StableEntityIdentity, HistoryFact>,
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
        self.cursor = cursor;
        Ok(())
    }

    pub(crate) fn import_page(
        &mut self,
        cursor: ImportCursor,
        facts: &[HistoryFact],
        next_key: Option<String>,
    ) -> Result<ImportOutcome> {
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
            fact.validate_against(&self.snapshot.source, &self.artifact)?;
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
        let signature = fact.signature();
        if let Some(existing_signature) = self.event_ids.get(&fact.event_id) {
            ensure!(
                existing_signature == &signature,
                "history event identity has conflicting content"
            );
            return Ok(false);
        }
        if let Some(existing) = self.entities.get(&fact.identity) {
            ensure!(
                existing.signature() == signature,
                "live/archive entity identity has conflicting content"
            );
            self.event_ids.insert(fact.event_id, signature);
            return Ok(false);
        }
        if let Some(existing_identity) = self.contributions.get(&fact.contribution_key) {
            ensure!(
                existing_identity == &entity_key(&fact.identity),
                "live/archive facts would contribute the same history unit twice"
            );
            return Ok(false);
        }
        self.event_ids.insert(fact.event_id.clone(), signature);
        self.contributions
            .insert(fact.contribution_key.clone(), entity_key(&fact.identity));
        self.entities.insert(fact.identity.clone(), fact);
        Ok(true)
    }
}

fn entity_key(identity: &StableEntityIdentity) -> String {
    format!(
        "{}:{}:{}",
        identity.source_instance_id, identity.entity_kind, identity.entity_key
    )
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{
        ArtifactIdentity, ArtifactKind, BuildSnapshot, ManifestIdentity, SourceFence,
    };
    use super::*;

    fn snapshot() -> BuildSnapshot {
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
            manifest_identities: Vec::new(),
            recovery_artifact_id: "recovery-1".to_string(),
        }
    }

    fn importer() -> HistoryImporter {
        let bytes = b"archive";
        let manifest = ManifestIdentity::from_bytes("manifest-1", "source-1", 4, 4, bytes).unwrap();
        let artifact = HistoryArtifact::new(
            ArtifactIdentity {
                artifact_id: "archive-1".to_string(),
                kind: ArtifactKind::Archive,
                source_instance_id: "source-1".to_string(),
                generation: 4,
                manifest,
            },
            &snapshot().source,
        )
        .unwrap();
        HistoryImporter::new(snapshot(), artifact, BoundedImport::default()).unwrap()
    }

    fn fact(event_id: &str, key: &str) -> HistoryFact {
        HistoryFact {
            source: ImportSource::Archive,
            source_instance_id: "source-1".to_string(),
            generation: 4,
            event_id: event_id.to_string(),
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "invocation".to_string(),
                entity_key: key.to_string(),
            },
            bucket: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            content_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            contribution_key: format!("bucket:{key}"),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        }
    }

    #[test]
    fn live_and_archive_duplicate_entity_contributes_once() {
        let mut importer = importer();
        let first = fact("event-1", "row-1");
        let mut duplicate = first.clone();
        duplicate.source = ImportSource::Archive;
        duplicate.event_id = "event-2".to_string();
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
        let first = fact("event-1", "row-1");
        let mut conflicting = first.clone();
        conflicting.event_id = "event-2".to_string();
        conflicting.content_digest =
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string();
        assert!(
            importer
                .import_page(importer.cursor().clone(), &[first, conflicting], None)
                .is_err()
        );
    }
}
