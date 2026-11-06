use super::coverage::{CoverageKey, CoverageRange};
use super::importer::{HistoryFact, StableEntityIdentity};
use super::snapshot::{BuildSnapshot, SourceFence, sha256_hex};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceEventProof {
    pub(crate) change_id: i64,
    pub(crate) event_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) content_digest: String,
}

impl SourceEventProof {
    fn validate_against(&self, source: &SourceFence) -> Result<()> {
        ensure!(
            self.change_id >= 1,
            "source event change id must be positive"
        );
        ensure!(!self.event_id.trim().is_empty(), "source event id is empty");
        ensure!(
            self.source_instance_id == source.source_instance_id,
            "source event belongs to another source instance"
        );
        ensure!(
            self.generation == source.generation,
            "source event generation changed"
        );
        ensure!(
            self.coverage_revision <= source.coverage_revision,
            "source event is beyond the snapshot coverage revision"
        );
        ensure!(
            !self.entity_kind.trim().is_empty(),
            "source event entity kind is empty"
        );
        ensure!(
            !self.entity_key.trim().is_empty(),
            "source event entity key is empty"
        );
        ensure!(
            is_digest(&self.content_digest),
            "source event content digest is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BucketEntityProof {
    pub(crate) identity: StableEntityIdentity,
    pub(crate) content_digest: String,
    pub(crate) contribution_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BucketProof {
    pub(crate) key: CoverageKey,
    pub(crate) range: CoverageRange,
    pub(crate) entity_count: usize,
    pub(crate) content_digest: String,
    pub(crate) sample_digests: Vec<String>,
    pub(crate) entities: Vec<BucketEntityProof>,
}

impl BucketProof {
    fn validate(&self) -> Result<()> {
        self.key.validate()?;
        CoverageRange::new(self.range.start.clone(), self.range.end.clone())?;
        ensure!(
            self.entity_count == self.entities.len(),
            "bucket entity count does not match proof"
        );
        ensure!(
            is_digest(&self.content_digest),
            "bucket content digest is invalid"
        );
        ensure!(
            self.sample_digests.iter().all(|digest| is_digest(digest)),
            "bucket sample digest is invalid"
        );
        let mut identities = BTreeSet::new();
        for entity in &self.entities {
            entity.identity.validate()?;
            ensure!(
                is_digest(&entity.content_digest),
                "bucket entity content digest is invalid"
            );
            ensure!(
                is_digest(&entity.contribution_digest),
                "bucket entity contribution digest is invalid"
            );
            ensure!(
                identities.insert(entity.identity.clone()),
                "bucket proof contains a duplicate entity"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CertificateBaseline {
    SourceFence {
        source_instance_id: String,
        generation: i64,
        owner_epoch: i64,
        through_change_id: i64,
        coverage_revision: i64,
    },
    LegacyConsumerCheckpoint {
        consumer_id: String,
        checkpoint: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconciliationCertificate {
    pub(crate) build_id: String,
    pub(crate) source: SourceFence,
    pub(crate) baseline: CertificateBaseline,
    pub(crate) through_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) source_prefix_digest: String,
    pub(crate) manifest_digests: Vec<String>,
    pub(crate) events: Vec<SourceEventProof>,
    pub(crate) bucket_proofs: Vec<BucketProof>,
    pub(crate) certificate_digest: String,
}

pub(crate) fn build_reconciliation_certificate(
    snapshot: &BuildSnapshot,
    baseline: CertificateBaseline,
    events: Vec<SourceEventProof>,
    facts: &[HistoryFact],
    sample_limit: usize,
) -> Result<ReconciliationCertificate> {
    snapshot.validate()?;
    ensure!(
        sample_limit > 0,
        "certificate sample limit must be positive"
    );
    let (baseline_change_id, baseline_coverage_revision) =
        source_baseline(&baseline, &snapshot.source)?;
    ensure!(
        snapshot.source.retained_from_change_id <= baseline_change_id.saturating_add(1),
        "retention gap prevents a complete source prefix certificate"
    );
    validate_source_prefix(snapshot, baseline_change_id, &events)?;
    let bucket_proofs = build_bucket_proofs(facts, &snapshot.source, sample_limit)?;
    let source_prefix_digest = sha256_hex(&serde_json::to_vec(&events)?);
    let manifest_digests = snapshot
        .manifest_identities
        .iter()
        .map(|manifest| manifest.sha256.clone())
        .collect::<Vec<_>>();
    let through_change_id = events
        .last()
        .map_or(baseline_change_id, |event| event.change_id);
    let coverage_revision = events
        .last()
        .map_or(baseline_coverage_revision, |event| event.coverage_revision);
    ensure!(
        through_change_id == snapshot.source.high_watermark_change_id,
        "source prefix certificate does not reach the snapshot watermark"
    );
    ensure!(
        coverage_revision == snapshot.source.coverage_revision,
        "source prefix certificate does not reach the snapshot coverage revision"
    );
    let mut certificate = ReconciliationCertificate {
        build_id: snapshot.build_id.clone(),
        source: snapshot.source.clone(),
        baseline,
        through_change_id,
        coverage_revision,
        source_prefix_digest,
        manifest_digests,
        events,
        bucket_proofs,
        certificate_digest: String::new(),
    };
    certificate.certificate_digest = certificate.digest_without_self()?;
    Ok(certificate)
}

impl ReconciliationCertificate {
    pub(crate) fn verify(
        &self,
        snapshot: &BuildSnapshot,
        events: &[SourceEventProof],
        facts: &[HistoryFact],
        sample_limit: usize,
    ) -> Result<()> {
        snapshot.validate()?;
        ensure!(
            self.build_id == snapshot.build_id,
            "certificate belongs to another build"
        );
        ensure!(
            self.source == snapshot.source,
            "certificate source fence changed"
        );
        let expected = build_reconciliation_certificate(
            snapshot,
            self.baseline.clone(),
            events.to_vec(),
            facts,
            sample_limit,
        )?;
        ensure!(
            expected.source_prefix_digest == self.source_prefix_digest,
            "source prefix proof changed"
        );
        ensure!(
            expected.bucket_proofs == self.bucket_proofs,
            "bucket entity/content/sample proof changed"
        );
        ensure!(
            expected.manifest_digests == self.manifest_digests,
            "certificate manifest identity changed"
        );
        ensure!(
            expected.certificate_digest == self.certificate_digest,
            "reconciliation certificate digest changed"
        );
        Ok(())
    }

    fn digest_without_self(&self) -> Result<String> {
        let mut material = self.clone();
        material.certificate_digest.clear();
        Ok(sha256_hex(&serde_json::to_vec(&material)?))
    }
}

fn source_baseline(baseline: &CertificateBaseline, source: &SourceFence) -> Result<(i64, i64)> {
    match baseline {
        CertificateBaseline::SourceFence {
            source_instance_id,
            generation,
            owner_epoch,
            through_change_id,
            coverage_revision,
        } => {
            ensure!(
                source_instance_id == &source.source_instance_id,
                "certificate baseline belongs to another source instance"
            );
            ensure!(
                *generation == source.generation,
                "certificate baseline generation changed"
            );
            ensure!(
                *owner_epoch == source.owner_epoch,
                "certificate baseline owner fence changed"
            );
            ensure!(*through_change_id >= 0, "certificate baseline is negative");
            ensure!(
                *through_change_id <= source.high_watermark_change_id,
                "certificate baseline is beyond the source snapshot"
            );
            ensure!(
                *coverage_revision <= source.coverage_revision,
                "certificate baseline coverage is beyond the source snapshot"
            );
            Ok((*through_change_id, *coverage_revision))
        }
        CertificateBaseline::LegacyConsumerCheckpoint { .. } => {
            bail!("old consumer checkpoint cannot establish a new analytics baseline")
        }
    }
}

fn validate_source_prefix(
    snapshot: &BuildSnapshot,
    baseline_change_id: i64,
    events: &[SourceEventProof],
) -> Result<()> {
    let mut event_ids = BTreeMap::<String, String>::new();
    let mut expected_change_id = baseline_change_id.saturating_add(1);
    let mut last_coverage_revision = 0;
    for event in events {
        event.validate_against(&snapshot.source)?;
        ensure!(
            event.change_id == expected_change_id,
            "source prefix is incomplete or out of order at change {}",
            event.change_id
        );
        expected_change_id = expected_change_id.saturating_add(1);
        ensure!(
            event.change_id <= snapshot.source.high_watermark_change_id,
            "source prefix exceeds the snapshot watermark"
        );
        ensure!(
            event.coverage_revision >= last_coverage_revision,
            "source coverage revision moved backwards"
        );
        last_coverage_revision = event.coverage_revision;
        if let Some(previous) =
            event_ids.insert(event.event_id.clone(), event.content_digest.clone())
        {
            ensure!(
                previous == event.content_digest,
                "source event identity has conflicting content"
            );
        }
    }
    Ok(())
}

fn build_bucket_proofs(
    facts: &[HistoryFact],
    source: &SourceFence,
    sample_limit: usize,
) -> Result<Vec<BucketProof>> {
    let mut grouped = BTreeMap::<(CoverageKey, CoverageRange), Vec<BucketEntityProof>>::new();
    for fact in facts {
        fact.identity.validate()?;
        ensure!(
            fact.source_instance_id == source.source_instance_id,
            "bucket fact belongs to another source instance"
        );
        ensure!(
            fact.generation == source.generation,
            "bucket fact generation changed"
        );
        ensure!(
            is_digest(&fact.content_digest),
            "history fact content digest is invalid"
        );
        ensure!(
            is_digest(&fact.contribution_digest),
            "history fact contribution digest is invalid"
        );
        grouped
            .entry((fact.bucket.clone(), fact.range.clone()))
            .or_default()
            .push(BucketEntityProof {
                identity: fact.identity.clone(),
                content_digest: fact.content_digest.clone(),
                contribution_digest: fact.contribution_digest.clone(),
            });
    }
    let mut proofs = Vec::with_capacity(grouped.len());
    for ((key, range), mut entities) in grouped {
        entities.sort_by(|left, right| left.identity.cmp(&right.identity));
        let content_digest = sha256_hex(&serde_json::to_vec(&entities)?);
        let sample_digests = entities
            .iter()
            .take(sample_limit)
            .map(|entity| entity.content_digest.clone())
            .collect();
        let proof = BucketProof {
            key,
            range,
            entity_count: entities.len(),
            content_digest,
            sample_digests,
            entities,
        };
        proof.validate()?;
        proofs.push(proof);
    }
    Ok(proofs)
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::super::coverage::{CoverageKey, CoverageRange};
    use super::super::importer::{HistoryFact, ImportSource, StableEntityIdentity};
    use super::super::snapshot::{BuildSnapshot, SourceFence};
    use super::*;

    fn snapshot() -> BuildSnapshot {
        BuildSnapshot {
            build_id: "build-1".to_string(),
            source: SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                high_watermark_change_id: 3,
                coverage_revision: 3,
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

    fn event(change_id: i64) -> SourceEventProof {
        SourceEventProof {
            change_id,
            event_id: format!("event-{change_id}"),
            source_instance_id: "source-1".to_string(),
            generation: 2,
            coverage_revision: change_id,
            entity_kind: "invocation".to_string(),
            entity_key: format!("row-{change_id}"),
            content_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        }
    }

    fn fact() -> HistoryFact {
        HistoryFact {
            source: ImportSource::Live,
            source_instance_id: "source-1".to_string(),
            generation: 2,
            event_id: "event-1".to_string(),
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "invocation".to_string(),
                entity_key: "row-1".to_string(),
            },
            bucket: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            content_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            contribution_key: "bucket:1".to_string(),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        }
    }

    #[test]
    fn source_prefix_requires_every_change() {
        let result = build_reconciliation_certificate(
            &snapshot(),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            vec![event(1), event(3)],
            &[fact()],
            2,
        );
        assert!(result.is_err());
    }

    #[test]
    fn old_consumer_checkpoint_cannot_be_a_new_baseline() {
        let result = build_reconciliation_certificate(
            &snapshot(),
            CertificateBaseline::LegacyConsumerCheckpoint {
                consumer_id: "old-consumer".to_string(),
                checkpoint: 0,
            },
            vec![event(1)],
            &[fact()],
            2,
        );
        assert!(result.is_err());
    }
}
