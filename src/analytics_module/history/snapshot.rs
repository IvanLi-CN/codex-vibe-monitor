use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const MAX_ID_CHARS: usize = 256;
const SHA256_HEX_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactKind {
    Live,
    Archive,
    Seed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceFence {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) high_watermark_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) retained_from_change_id: i64,
    pub(crate) fact_semantics_version: i64,
    pub(crate) stats_semantics_version: i64,
}

impl SourceFence {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            !self.source_instance_id.trim().is_empty(),
            "source instance id is empty"
        );
        ensure!(
            self.source_instance_id.chars().count() <= MAX_ID_CHARS,
            "source instance id is too long"
        );
        ensure!(
            self.generation >= 0,
            "source generation must not be negative"
        );
        ensure!(self.owner_epoch > 0, "owner epoch must be positive");
        ensure!(
            self.high_watermark_change_id >= 0,
            "source watermark must not be negative"
        );
        ensure!(
            self.coverage_revision >= 0,
            "source coverage revision must not be negative"
        );
        ensure!(
            self.retained_from_change_id >= 0,
            "retained source floor must not be negative"
        );
        ensure!(
            self.retained_from_change_id <= self.high_watermark_change_id.saturating_add(1),
            "retained source floor is beyond the source watermark"
        );
        ensure!(
            self.fact_semantics_version > 0,
            "fact semantics version must be positive"
        );
        ensure!(
            self.stats_semantics_version > 0,
            "stats semantics version must be positive"
        );
        Ok(())
    }

    pub(crate) fn same_source(&self, other: &Self) -> bool {
        self.source_instance_id == other.source_instance_id
            && self.generation == other.generation
            && self.owner_epoch == other.owner_epoch
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestIdentity {
    pub(crate) manifest_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

impl ManifestIdentity {
    pub(crate) fn from_bytes(
        manifest_id: impl Into<String>,
        source_instance_id: impl Into<String>,
        generation: i64,
        coverage_revision: i64,
        bytes: &[u8],
    ) -> Result<Self> {
        let identity = Self {
            manifest_id: manifest_id.into(),
            source_instance_id: source_instance_id.into(),
            generation,
            coverage_revision,
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (value, label) in [
            (&self.manifest_id, "manifest id"),
            (&self.source_instance_id, "manifest source instance id"),
        ] {
            ensure!(!value.trim().is_empty(), "{label} is empty");
            ensure!(value.chars().count() <= MAX_ID_CHARS, "{label} is too long");
        }
        ensure!(
            self.generation >= 0,
            "manifest generation must not be negative"
        );
        ensure!(
            self.coverage_revision >= 0,
            "manifest coverage revision must not be negative"
        );
        ensure!(is_sha256(&self.sha256), "manifest SHA-256 is invalid");
        Ok(())
    }

    pub(crate) fn matches_bytes(&self, bytes: &[u8]) -> bool {
        self.size_bytes == bytes.len() as u64 && self.sha256 == sha256_hex(bytes)
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.manifest_id == other.manifest_id
            && self.source_instance_id == other.source_instance_id
            && self.generation == other.generation
            && self.coverage_revision == other.coverage_revision
            && self.size_bytes == other.size_bytes
            && self.sha256 == other.sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactIdentity {
    pub(crate) artifact_id: String,
    pub(crate) kind: ArtifactKind,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) manifest: ManifestIdentity,
}

impl ArtifactIdentity {
    pub(crate) fn validate_against(&self, source: &SourceFence) -> Result<()> {
        ensure!(!self.artifact_id.trim().is_empty(), "artifact id is empty");
        ensure!(
            self.source_instance_id == source.source_instance_id,
            "artifact belongs to another source instance"
        );
        ensure!(
            self.generation == source.generation,
            "artifact belongs to another generation"
        );
        ensure!(
            self.manifest.source_instance_id == source.source_instance_id,
            "manifest belongs to another source instance"
        );
        ensure!(
            self.manifest.generation == source.generation,
            "manifest belongs to another generation"
        );
        self.manifest.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StableArtifact {
    pub(crate) identity: ManifestIdentity,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuildSnapshot {
    pub(crate) build_id: String,
    pub(crate) source: SourceFence,
    pub(crate) retention_pin_id: String,
    pub(crate) pin_expires_at: String,
    pub(crate) manifest_identities: Vec<ManifestIdentity>,
    pub(crate) recovery_artifact_id: String,
}

impl BuildSnapshot {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(!self.build_id.trim().is_empty(), "build id is empty");
        ensure!(
            self.build_id.chars().count() <= MAX_ID_CHARS,
            "build id is too long"
        );
        self.source.validate()?;
        ensure!(
            !self.retention_pin_id.trim().is_empty(),
            "retention pin id is empty"
        );
        ensure!(
            !self.recovery_artifact_id.trim().is_empty(),
            "recovery artifact id is empty"
        );
        let expires_at = DateTime::parse_from_rfc3339(&self.pin_expires_at)
            .context("snapshot retention pin expiry must be RFC3339")?;
        ensure!(
            expires_at.with_timezone(&Utc) > Utc::now(),
            "snapshot retention pin has expired"
        );
        for manifest in &self.manifest_identities {
            manifest.validate()?;
            ensure!(
                manifest.source_instance_id == self.source.source_instance_id,
                "snapshot manifest source instance mismatch"
            );
            ensure!(
                manifest.generation == self.source.generation,
                "snapshot manifest generation mismatch"
            );
        }
        Ok(())
    }

    pub(crate) fn snapshot_id(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }

    pub(crate) fn validate_observation(&self, observation: &SnapshotObservation) -> Result<()> {
        self.validate()?;
        ensure!(
            observation.active_retention_pin,
            "snapshot retention pin is not active"
        );
        ensure!(
            self.source.same_source(&observation.source),
            "snapshot source fence changed"
        );
        ensure!(
            observation.source.coverage_revision >= self.source.coverage_revision,
            "snapshot coverage revision moved backwards"
        );
        ensure!(
            observation.source.high_watermark_change_id >= self.source.high_watermark_change_id,
            "snapshot source watermark moved backwards"
        );
        ensure!(
            observation.source.retained_from_change_id <= self.source.high_watermark_change_id,
            "retention gap invalidates the snapshot"
        );
        for expected in &self.manifest_identities {
            let Some(observed) = observation
                .manifest_identities
                .iter()
                .find(|candidate| candidate.manifest_id == expected.manifest_id)
            else {
                bail!("manifest identity is missing for snapshot");
            };
            ensure!(
                expected.same_identity(observed),
                "manifest identity was replaced"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotObservation {
    pub(crate) source: SourceFence,
    pub(crate) active_retention_pin: bool,
    pub(crate) manifest_identities: Vec<ManifestIdentity>,
}

pub(crate) fn read_verified_artifact(
    path: &Path,
    expected: &ManifestIdentity,
    max_bytes: usize,
) -> Result<StableArtifact> {
    expected.validate()?;
    let before = fs::metadata(path)
        .with_context(|| format!("read artifact metadata: {}", path.display()))?;
    ensure!(
        before.len() <= max_bytes as u64,
        "artifact exceeds bounded import size"
    );
    let file = File::open(path).with_context(|| format!("open artifact: {}", path.display()))?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read bounded artifact: {}", path.display()))?;
    ensure!(
        bytes.len() <= max_bytes,
        "artifact exceeds bounded import size"
    );
    let after = fs::metadata(path)
        .with_context(|| format!("re-read artifact metadata: {}", path.display()))?;
    ensure!(
        same_file_metadata(&before, &after),
        "artifact changed while it was being read"
    );
    ensure!(
        expected.matches_bytes(&bytes),
        "artifact manifest identity mismatch"
    );
    Ok(StableArtifact {
        identity: expected.clone(),
        bytes,
    })
}

fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    if left.len() != right.len() || left.modified().ok() != right.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == SHA256_HEX_CHARS && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn source() -> SourceFence {
        SourceFence {
            source_instance_id: "source-1".to_string(),
            generation: 5,
            owner_epoch: 2,
            high_watermark_change_id: 4,
            coverage_revision: 4,
            retained_from_change_id: 0,
            fact_semantics_version: 1,
            stats_semantics_version: 2,
        }
    }

    fn snapshot() -> BuildSnapshot {
        BuildSnapshot {
            build_id: "build-1".to_string(),
            source: source(),
            retention_pin_id: "pin-1".to_string(),
            pin_expires_at: "2999-01-01T00:00:00Z".to_string(),
            manifest_identities: Vec::new(),
            recovery_artifact_id: "recovery-1".to_string(),
        }
    }

    #[test]
    fn artifact_manifest_must_match_the_bytes_read() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cvm-history-{unique}.archive"));
        std::fs::write(&path, b"first").unwrap();
        let expected =
            ManifestIdentity::from_bytes("manifest-1", "source-1", 5, 4, b"first").unwrap();
        assert!(read_verified_artifact(&path, &expected, 1024).is_ok());
        std::fs::write(&path, b"second").unwrap();
        assert!(read_verified_artifact(&path, &expected, 1024).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retention_gap_invalidates_a_snapshot_observation() {
        let snapshot = snapshot();
        let mut observed_source = source();
        observed_source.retained_from_change_id = 5;
        let observation = SnapshotObservation {
            source: observed_source,
            active_retention_pin: true,
            manifest_identities: Vec::new(),
        };
        assert!(snapshot.validate_observation(&observation).is_err());
    }

    #[test]
    fn generation_change_invalidates_a_snapshot_observation() {
        let snapshot = snapshot();
        let mut observed_source = source();
        observed_source.generation = 6;
        let observation = SnapshotObservation {
            source: observed_source,
            active_retention_pin: true,
            manifest_identities: Vec::new(),
        };
        assert!(snapshot.validate_observation(&observation).is_err());
    }
}
