use super::coverage::CoverageLedger;
use super::coverage::{CoverageKey, CoverageRange};
use super::importer::{
    HistoryArtifact, HistoryFact, SourceOperation, StableEntityIdentity, facts_digest,
};
use super::seed::CertifiedSeed;
use super::snapshot::{BuildSnapshot, ManifestIdentity, SourceFence, sha256_hex};
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
    pub(crate) operation: SourceOperation,
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
            !matches!(self.operation, SourceOperation::Upsert)
                || self.entity_kind == "invocation"
                || self.entity_kind == "pool_attempt",
            "source event entity kind is not a normalized analytics entity"
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
    pub(crate) effective_change_id: i64,
    pub(crate) operation: SourceOperation,
    pub(crate) contribution_key: String,
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
                entity.effective_change_id >= 1,
                "bucket entity effective change id must be positive"
            );
            ensure!(
                !entity.contribution_key.trim().is_empty(),
                "bucket entity contribution key is empty"
            );
            ensure!(
                is_digest(&entity.content_digest),
                "bucket entity content digest is invalid"
            );
            ensure!(
                is_digest(&entity.contribution_digest),
                "bucket entity contribution digest is invalid"
            );
            ensure!(
                identities.insert((entity.identity.clone(), entity.effective_change_id)),
                "bucket proof contains a duplicate entity version"
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
    CertifiedSourceFence {
        build_id: String,
        source_instance_id: String,
        generation: i64,
        owner_epoch: i64,
        through_change_id: i64,
        coverage_revision: i64,
        certificate_digest: String,
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
    pub(crate) manifest_identities: Vec<ManifestIdentity>,
    pub(crate) manifest_digest: String,
    pub(crate) artifact_id: String,
    pub(crate) artifact_bytes_digest: String,
    pub(crate) artifact_facts_digest: String,
    pub(crate) events: Vec<SourceEventProof>,
    pub(crate) bucket_proofs: Vec<BucketProof>,
    pub(crate) certificate_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpectedHistoryBinding {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) build_id: String,
    pub(crate) coverage_revision: i64,
    pub(crate) target_high_watermark_change_id: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) manifest_identities: Vec<ManifestIdentity>,
    pub(crate) fact_semantics_version: i64,
    pub(crate) stats_semantics_version: i64,
    pub(crate) artifact_id: String,
    pub(crate) artifact_bytes_digest: String,
    pub(crate) artifact_facts_digest: String,
}

impl ExpectedHistoryBinding {
    pub(crate) fn for_snapshot(snapshot: &BuildSnapshot) -> Self {
        Self {
            source_instance_id: snapshot.source.source_instance_id.clone(),
            generation: snapshot.source.generation,
            build_id: snapshot.build_id.clone(),
            coverage_revision: snapshot.source.coverage_revision,
            target_high_watermark_change_id: snapshot.source.high_watermark_change_id,
            owner_epoch: snapshot.source.owner_epoch,
            manifest_identities: snapshot.manifest_identities.clone(),
            fact_semantics_version: snapshot.source.fact_semantics_version,
            stats_semantics_version: snapshot.source.stats_semantics_version,
            artifact_id: String::new(),
            artifact_bytes_digest: String::new(),
            artifact_facts_digest: String::new(),
        }
    }

    pub(crate) fn for_snapshot_and_artifact(
        snapshot: &BuildSnapshot,
        artifact: &HistoryArtifact,
    ) -> Result<Self> {
        let mut binding = Self::for_snapshot(snapshot);
        binding.artifact_id = artifact.identity().artifact_id.clone();
        binding.artifact_bytes_digest = artifact.identity().manifest.sha256.clone();
        binding.artifact_facts_digest = artifact.decoded_facts_digest()?.to_string();
        Ok(binding)
    }

    fn validate_snapshot(&self, snapshot: &BuildSnapshot) -> Result<()> {
        ensure!(
            self.build_id == snapshot.build_id,
            "history binding build identity changed"
        );
        ensure!(
            self.source_instance_id == snapshot.source.source_instance_id,
            "history binding source instance changed"
        );
        ensure!(
            self.generation == snapshot.source.generation,
            "history binding generation changed"
        );
        ensure!(
            self.owner_epoch == snapshot.source.owner_epoch,
            "history binding owner epoch changed"
        );
        ensure!(
            self.coverage_revision == snapshot.source.coverage_revision,
            "history binding coverage revision changed"
        );
        ensure!(
            self.target_high_watermark_change_id == snapshot.source.high_watermark_change_id,
            "history binding target watermark changed"
        );
        ensure!(
            self.fact_semantics_version == snapshot.source.fact_semantics_version,
            "history binding fact semantics version changed"
        );
        ensure!(
            self.stats_semantics_version == snapshot.source.stats_semantics_version,
            "history binding stats semantics version changed"
        );
        ensure!(
            self.manifest_identities == snapshot.manifest_identities,
            "history binding manifest identity changed"
        );
        if snapshot.source.high_watermark_change_id > 0 {
            ensure!(
                !self.artifact_id.trim().is_empty(),
                "history binding artifact id is empty"
            );
            ensure!(
                is_digest(&self.artifact_bytes_digest),
                "history binding artifact bytes digest is invalid"
            );
            ensure!(
                is_digest(&self.artifact_facts_digest),
                "history binding artifact facts digest is invalid"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrustedPrefixAnchor {
    pub(crate) certificate_digest: Option<String>,
    pub(crate) through_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) sample_limit: usize,
    pub(crate) max_chain_depth: usize,
    pub(crate) max_fact_count: usize,
}

impl TrustedPrefixAnchor {
    pub(crate) fn genesis(
        sample_limit: usize,
        max_chain_depth: usize,
        max_fact_count: usize,
    ) -> Self {
        Self {
            certificate_digest: None,
            through_change_id: 0,
            coverage_revision: 0,
            sample_limit,
            max_chain_depth,
            max_fact_count,
        }
    }

    pub(crate) fn certified(
        certificate_digest: impl Into<String>,
        through_change_id: i64,
        coverage_revision: i64,
        sample_limit: usize,
        max_chain_depth: usize,
        max_fact_count: usize,
    ) -> Self {
        Self {
            certificate_digest: Some(certificate_digest.into()),
            through_change_id,
            coverage_revision,
            sample_limit,
            max_chain_depth,
            max_fact_count,
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.sample_limit > 0,
            "trusted sample limit must be positive"
        );
        ensure!(
            self.max_chain_depth > 0,
            "trusted chain depth must be positive"
        );
        ensure!(
            self.max_fact_count > 0,
            "trusted fact budget must be positive"
        );
        ensure!(
            self.through_change_id >= 0,
            "trusted anchor change id is negative"
        );
        ensure!(
            self.coverage_revision >= 0,
            "trusted anchor coverage is negative"
        );
        if let Some(digest) = &self.certificate_digest {
            ensure!(is_digest(digest), "trusted certificate digest is invalid");
            ensure!(
                self.through_change_id > 0,
                "certified anchor must be non-zero"
            );
        } else {
            ensure!(
                self.through_change_id == 0 && self.coverage_revision == 0,
                "genesis anchor must be 0/0"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CertifiedReceiptPrefix {
    snapshot: BuildSnapshot,
    certificate: ReconciliationCertificate,
    events: Vec<SourceEventProof>,
    exact_facts: Vec<HistoryFact>,
    sample_limit: usize,
    predecessor: Option<Box<CertifiedReceiptPrefix>>,
    coverage: CoverageLedger,
    certified_seeds: Vec<CertifiedSeed>,
}

pub(crate) struct VerifiedReceiptPrefix<'a> {
    prefix: &'a CertifiedReceiptPrefix,
}

impl CertifiedReceiptPrefix {
    pub(crate) fn verify<'a>(
        &'a self,
        expected: &ExpectedHistoryBinding,
        trust: &TrustedPrefixAnchor,
    ) -> Result<VerifiedReceiptPrefix<'a>> {
        trust.validate()?;
        self.verify_chain(expected, trust, 0, true)?;
        Ok(VerifiedReceiptPrefix { prefix: self })
    }

    pub(crate) fn attach_certified_seed(mut self, seed: CertifiedSeed) -> Result<Self> {
        ensure!(
            seed.provenance_digest() == self.certificate.certificate_digest,
            "certified seed provenance is not bound to this certificate"
        );
        for incoming in seed.coverage().claims() {
            ensure!(
                !self.coverage.claims().iter().any(|existing| {
                    existing.key == incoming.key && existing.range.overlaps(&incoming.range)
                }),
                "certified seed overlaps existing history coverage"
            );
        }
        self.coverage.merge(seed.coverage())?;
        self.certified_seeds.push(seed);
        Ok(self)
    }

    fn verify_chain(
        &self,
        expected: &ExpectedHistoryBinding,
        trust: &TrustedPrefixAnchor,
        depth: usize,
        is_trusted_endpoint: bool,
    ) -> Result<()> {
        ensure!(
            depth < trust.max_chain_depth,
            "history certificate chain exceeds trusted depth"
        );
        ensure!(
            self.exact_facts.len() <= trust.max_fact_count,
            "history certificate fact batch exceeds trusted budget"
        );
        self.snapshot.validate()?;
        expected.validate_snapshot(&self.snapshot)?;
        ensure!(
            self.sample_limit == trust.sample_limit,
            "certificate sample limit is not the trusted local budget"
        );
        if let Some(predecessor) = &self.predecessor {
            let mut predecessor_expected =
                ExpectedHistoryBinding::for_snapshot(&predecessor.snapshot);
            predecessor_expected.artifact_id = predecessor.certificate.artifact_id.clone();
            predecessor_expected.artifact_bytes_digest =
                predecessor.certificate.artifact_bytes_digest.clone();
            predecessor_expected.artifact_facts_digest =
                predecessor.certificate.artifact_facts_digest.clone();
            predecessor.verify_chain(&predecessor_expected, trust, depth + 1, false)?;
        }
        if is_trusted_endpoint {
            if trust.certificate_digest.is_some() {
                validate_trusted_anchor(&self.certificate.baseline, trust)?;
            } else {
                ensure!(
                    self.anchored_at_genesis(),
                    "history certificate chain is not anchored at genesis"
                );
            }
        }
        let expected_certificate = build_reconciliation_certificate_with_baseline(
            &self.snapshot,
            self.certificate.baseline.clone(),
            self.predecessor.as_deref(),
            self.events.clone(),
            &self.exact_facts,
            self.sample_limit,
        )?;
        ensure!(
            expected_certificate.artifact_id == expected.artifact_id
                && expected_certificate.artifact_bytes_digest == expected.artifact_bytes_digest
                && expected_certificate.artifact_facts_digest == expected.artifact_facts_digest,
            "certificate facts are not bound to the trusted artifact receipt"
        );
        ensure!(
            expected_certificate == self.certificate,
            "reconciliation certificate or predecessor facts changed"
        );
        let mut expected_coverage = self
            .predecessor
            .as_ref()
            .map_or_else(CoverageLedger::default, |prefix| prefix.coverage.clone());
        for fact in &self.exact_facts {
            if !expected_coverage.claims().iter().any(|claim| {
                claim.kind == super::coverage::CoverageKind::ExactFacts
                    && claim.key == fact.bucket
                    && claim.range.contains(&fact.range)
            }) {
                expected_coverage.admit_exact(
                    fact.bucket.clone(),
                    fact.range.clone(),
                    self.certificate.certificate_digest.clone(),
                )?;
            }
        }
        for seed in &self.certified_seeds {
            ensure!(
                seed.provenance_digest() == self.certificate.certificate_digest,
                "certified seed provenance changed"
            );
            expected_coverage.merge(seed.coverage())?;
        }
        ensure!(
            expected_coverage == self.coverage,
            "history coverage is not bound to the complete certificate chain"
        );
        Ok(())
    }

    fn anchored_at_genesis(&self) -> bool {
        match (&self.certificate.baseline, &self.predecessor) {
            (CertificateBaseline::SourceFence { .. }, None) => true,
            (CertificateBaseline::CertifiedSourceFence { .. }, Some(predecessor)) => {
                predecessor.anchored_at_genesis()
            }
            _ => false,
        }
    }
}

impl VerifiedReceiptPrefix<'_> {
    pub(crate) fn snapshot(&self) -> &BuildSnapshot {
        &self.prefix.snapshot
    }

    pub(crate) fn source_fence(&self) -> &SourceFence {
        &self.prefix.snapshot.source
    }

    pub(crate) fn certificate_digest(&self) -> &str {
        &self.prefix.certificate.certificate_digest
    }

    pub(crate) fn prefix_digest(&self) -> &str {
        &self.prefix.certificate.source_prefix_digest
    }

    pub(crate) fn predecessor_digest(&self) -> Option<&str> {
        match &self.prefix.certificate.baseline {
            CertificateBaseline::CertifiedSourceFence {
                certificate_digest, ..
            } => Some(certificate_digest),
            CertificateBaseline::SourceFence { .. }
            | CertificateBaseline::LegacyConsumerCheckpoint { .. } => None,
        }
    }

    pub(crate) fn baseline_through_change_id(&self) -> i64 {
        baseline_change_id(&self.prefix.certificate.baseline)
    }

    pub(crate) fn through_change_id(&self) -> i64 {
        self.prefix.certificate.through_change_id
    }

    pub(crate) fn manifest_digest(&self) -> &str {
        &self.prefix.certificate.manifest_digest
    }

    pub(crate) fn exact_facts(&self) -> impl Iterator<Item = &HistoryFact> {
        self.prefix.all_exact_facts()
    }

    pub(crate) fn certified_seeds(&self) -> impl Iterator<Item = &CertifiedSeed> {
        self.prefix.all_certified_seeds()
    }

    pub(crate) fn coverage(&self) -> &CoverageLedger {
        &self.prefix.coverage
    }

    pub(crate) fn bucket_proofs(&self) -> Box<dyn Iterator<Item = &BucketProof> + '_> {
        self.prefix.all_bucket_proofs()
    }
}

impl CertifiedReceiptPrefix {
    #[cfg(test)]
    pub(crate) fn certificate_digest_for_test(&self) -> &str {
        &self.certificate.certificate_digest
    }

    fn all_exact_facts(&self) -> Box<dyn Iterator<Item = &HistoryFact> + '_> {
        match &self.predecessor {
            Some(predecessor) => {
                Box::new(predecessor.all_exact_facts().chain(self.exact_facts.iter()))
            }
            None => Box::new(self.exact_facts.iter()),
        }
    }

    fn all_certified_seeds(&self) -> Box<dyn Iterator<Item = &CertifiedSeed> + '_> {
        match &self.predecessor {
            Some(predecessor) => Box::new(
                predecessor
                    .all_certified_seeds()
                    .chain(self.certified_seeds.iter()),
            ),
            None => Box::new(self.certified_seeds.iter()),
        }
    }

    fn all_bucket_proofs(&self) -> Box<dyn Iterator<Item = &BucketProof> + '_> {
        match &self.predecessor {
            Some(predecessor) => Box::new(
                predecessor
                    .all_bucket_proofs()
                    .chain(self.certificate.bucket_proofs.iter()),
            ),
            None => Box::new(self.certificate.bucket_proofs.iter()),
        }
    }
}

pub(crate) fn build_reconciliation_certificate(
    snapshot: &BuildSnapshot,
    baseline: CertificateBaseline,
    events: Vec<SourceEventProof>,
    facts: &[HistoryFact],
    sample_limit: usize,
) -> Result<ReconciliationCertificate> {
    build_reconciliation_certificate_with_baseline(
        snapshot,
        baseline,
        None,
        events,
        facts,
        sample_limit,
    )
}

pub(crate) fn build_reconciliation_certificate_with_baseline(
    snapshot: &BuildSnapshot,
    baseline: CertificateBaseline,
    predecessor: Option<&CertifiedReceiptPrefix>,
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
        source_baseline(&baseline, &snapshot.source, predecessor)?;
    ensure!(
        snapshot.source.retained_from_change_id <= baseline_change_id.saturating_add(1),
        "retention gap prevents a complete source prefix certificate"
    );
    validate_source_prefix(
        snapshot,
        baseline_change_id,
        baseline_coverage_revision,
        &events,
    )?;
    validate_event_facts(&events, facts, &snapshot.source)?;
    let bucket_proofs = build_bucket_proofs(facts, &snapshot.source, sample_limit)?;
    let source_prefix_digest = sha256_hex(&serde_json::to_vec(&(
        baseline_certificate_digest(&baseline),
        &events,
    ))?);
    let manifest_identities = snapshot.manifest_identities.clone();
    let manifest_digest = sha256_hex(&serde_json::to_vec(&manifest_identities)?);
    let mut complete_facts = predecessor.map_or_else(Vec::new, |prefix| {
        prefix.all_exact_facts().cloned().collect()
    });
    complete_facts.extend_from_slice(facts);
    let (artifact_id, artifact_bytes_digest, artifact_facts_digest) = if complete_facts.is_empty() {
        (String::new(), String::new(), facts_digest(&complete_facts)?)
    } else {
        let (artifact_id, artifact_bytes_digest) = artifact_binding(&complete_facts)?;
        (
            artifact_id,
            artifact_bytes_digest,
            facts_digest(&complete_facts)?,
        )
    };
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
        manifest_identities,
        manifest_digest,
        artifact_id,
        artifact_bytes_digest,
        artifact_facts_digest,
        events,
        bucket_proofs,
        certificate_digest: String::new(),
    };
    certificate.certificate_digest = certificate.digest_without_self()?;
    Ok(certificate)
}

pub(crate) fn build_certified_receipt_prefix(
    snapshot: BuildSnapshot,
    baseline: CertificateBaseline,
    predecessor: Option<Box<CertifiedReceiptPrefix>>,
    events: Vec<SourceEventProof>,
    facts: Vec<HistoryFact>,
    sample_limit: usize,
) -> Result<CertifiedReceiptPrefix> {
    let certificate = build_reconciliation_certificate_with_baseline(
        &snapshot,
        baseline,
        predecessor.as_deref(),
        events.clone(),
        &facts,
        sample_limit,
    )?;
    let mut coverage = predecessor
        .as_ref()
        .map_or_else(CoverageLedger::default, |prefix| prefix.coverage.clone());
    for fact in &facts {
        if !coverage.claims().iter().any(|claim| {
            claim.kind == super::coverage::CoverageKind::ExactFacts
                && claim.key == fact.bucket
                && claim.range.contains(&fact.range)
        }) {
            coverage.admit_exact(
                fact.bucket.clone(),
                fact.range.clone(),
                certificate.certificate_digest.clone(),
            )?;
        }
    }
    Ok(CertifiedReceiptPrefix {
        snapshot,
        certificate,
        events,
        exact_facts: facts,
        sample_limit,
        predecessor,
        coverage,
        certified_seeds: Vec::new(),
    })
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
            expected.manifest_identities == self.manifest_identities,
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

fn source_baseline(
    baseline: &CertificateBaseline,
    source: &SourceFence,
    predecessor: Option<&CertifiedReceiptPrefix>,
) -> Result<(i64, i64)> {
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
            ensure!(
                *through_change_id == 0 && *coverage_revision == 0,
                "only a complete 0/0 source fence can be a genesis baseline"
            );
            ensure!(
                predecessor.is_none(),
                "genesis baseline cannot carry a predecessor"
            );
            Ok((*through_change_id, *coverage_revision))
        }
        CertificateBaseline::CertifiedSourceFence {
            build_id,
            source_instance_id,
            generation,
            owner_epoch,
            through_change_id,
            coverage_revision,
            certificate_digest,
        } => {
            ensure!(
                source_instance_id == &source.source_instance_id,
                "certified baseline belongs to another source instance"
            );
            ensure!(
                *generation == source.generation,
                "certified baseline generation changed"
            );
            ensure!(
                *owner_epoch == source.owner_epoch,
                "certified baseline owner fence changed"
            );
            ensure!(
                *through_change_id > 0,
                "certified baseline must be non-zero"
            );
            ensure!(
                *coverage_revision >= 0,
                "certified baseline coverage is negative"
            );
            ensure!(
                *through_change_id <= source.high_watermark_change_id,
                "certificate baseline is beyond the source snapshot"
            );
            ensure!(
                *coverage_revision <= source.coverage_revision,
                "certificate baseline coverage is beyond the source snapshot"
            );
            ensure!(
                is_digest(certificate_digest),
                "certified baseline digest is invalid"
            );
            let predecessor = predecessor.ok_or_else(|| {
                anyhow::anyhow!("non-zero baseline requires an independently certified predecessor")
            })?;
            ensure!(
                predecessor.certificate.certificate_digest == *certificate_digest,
                "certified baseline predecessor digest changed"
            );
            ensure!(
                predecessor.snapshot.build_id == *build_id,
                "certified baseline predecessor build changed"
            );
            ensure!(
                predecessor.certificate.through_change_id == *through_change_id
                    && predecessor.certificate.coverage_revision == *coverage_revision,
                "certified baseline predecessor watermark changed"
            );
            ensure!(
                predecessor.certificate.source.same_source(source),
                "certified baseline predecessor source changed"
            );
            Ok((*through_change_id, *coverage_revision))
        }
        CertificateBaseline::LegacyConsumerCheckpoint { .. } => {
            bail!("old consumer checkpoint cannot establish a new analytics baseline")
        }
    }
}

fn baseline_change_id(baseline: &CertificateBaseline) -> i64 {
    match baseline {
        CertificateBaseline::SourceFence {
            through_change_id, ..
        }
        | CertificateBaseline::CertifiedSourceFence {
            through_change_id, ..
        } => *through_change_id,
        CertificateBaseline::LegacyConsumerCheckpoint { checkpoint, .. } => *checkpoint,
    }
}

fn baseline_certificate_digest(baseline: &CertificateBaseline) -> Option<&str> {
    match baseline {
        CertificateBaseline::CertifiedSourceFence {
            certificate_digest, ..
        } => Some(certificate_digest),
        CertificateBaseline::SourceFence { .. }
        | CertificateBaseline::LegacyConsumerCheckpoint { .. } => None,
    }
}

fn validate_trusted_anchor(
    baseline: &CertificateBaseline,
    trust: &TrustedPrefixAnchor,
) -> Result<()> {
    ensure!(
        baseline_change_id(baseline) == trust.through_change_id,
        "certificate baseline is not the trusted anchor"
    );
    let coverage_revision = match baseline {
        CertificateBaseline::SourceFence {
            coverage_revision, ..
        }
        | CertificateBaseline::CertifiedSourceFence {
            coverage_revision, ..
        } => *coverage_revision,
        CertificateBaseline::LegacyConsumerCheckpoint { .. } => {
            bail!("legacy checkpoint cannot be a trusted history anchor")
        }
    };
    ensure!(
        coverage_revision == trust.coverage_revision,
        "certificate baseline coverage is not the trusted anchor"
    );
    ensure!(
        baseline_certificate_digest(baseline) == trust.certificate_digest.as_deref(),
        "certificate predecessor is not the trusted anchor"
    );
    Ok(())
}

fn validate_source_prefix(
    snapshot: &BuildSnapshot,
    baseline_change_id: i64,
    baseline_coverage_revision: i64,
    events: &[SourceEventProof],
) -> Result<()> {
    let mut event_ids = BTreeMap::<String, String>::new();
    let mut expected_change_id = baseline_change_id.saturating_add(1);
    let mut last_coverage_revision = baseline_coverage_revision;
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

fn validate_event_facts(
    events: &[SourceEventProof],
    facts: &[HistoryFact],
    source: &SourceFence,
) -> Result<()> {
    ensure!(
        facts.len() == events.len(),
        "certificate facts do not cover every source event"
    );
    let mut facts_by_event = BTreeMap::new();
    for fact in facts {
        fact.validate_against_source(source)?;
        ensure!(
            facts_by_event.insert(fact.event_id.clone(), fact).is_none(),
            "certificate contains duplicate fact event ids"
        );
    }
    let _ = artifact_binding(facts)?;
    for event in events {
        let fact = facts_by_event
            .get(&event.event_id)
            .ok_or_else(|| anyhow::anyhow!("certificate is missing a fact for source event"))?;
        ensure!(
            fact.effective_change_id == event.change_id,
            "history fact effective change is not bound to its source event"
        );
        ensure!(
            fact.identity.entity_kind == event.entity_kind
                && fact.identity.entity_key == event.entity_key,
            "history fact identity is not bound to its source event"
        );
        ensure!(
            fact.operation == event.operation,
            "history fact operation is not bound to its source event"
        );
        ensure!(
            fact.content_digest == event.content_digest,
            "history fact content is not bound to its source event"
        );
    }
    Ok(())
}

fn artifact_binding(facts: &[HistoryFact]) -> Result<(String, String)> {
    let Some(first) = facts.first() else {
        return Ok((String::new(), String::new()));
    };
    ensure!(
        !first.artifact_id.trim().is_empty(),
        "certificate artifact id is empty"
    );
    ensure!(
        is_digest(&first.artifact_bytes_digest),
        "certificate artifact bytes digest is invalid"
    );
    for fact in &facts[1..] {
        ensure!(
            fact.artifact_id == first.artifact_id
                && fact.artifact_bytes_digest == first.artifact_bytes_digest,
            "certificate facts come from different artifacts"
        );
    }
    Ok((
        first.artifact_id.clone(),
        first.artifact_bytes_digest.clone(),
    ))
}

fn build_bucket_proofs(
    facts: &[HistoryFact],
    source: &SourceFence,
    sample_limit: usize,
) -> Result<Vec<BucketProof>> {
    let mut grouped = BTreeMap::<(CoverageKey, CoverageRange), Vec<BucketEntityProof>>::new();
    for fact in facts {
        fact.validate_against_source(source)?;
        ensure!(
            is_digest(&fact.contribution_digest),
            "history fact contribution digest is invalid"
        );
        grouped
            .entry((fact.bucket.clone(), fact.range.clone()))
            .or_default()
            .push(BucketEntityProof {
                identity: fact.identity.clone(),
                effective_change_id: fact.effective_change_id,
                operation: fact.operation,
                contribution_key: fact.contribution_key.clone(),
                content_digest: fact.content_digest.clone(),
                contribution_digest: fact.contribution_digest.clone(),
            });
    }
    let mut proofs = Vec::with_capacity(grouped.len());
    for ((key, range), mut entities) in grouped {
        entities.sort_by(|left, right| {
            left.identity
                .cmp(&right.identity)
                .then_with(|| left.effective_change_id.cmp(&right.effective_change_id))
        });
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
    use super::super::importer::{
        HistoryFact, ImportSource, NormalizedFactPayload, SourceOperation, StableEntityIdentity,
    };
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

    fn snapshot_at(high_watermark: i64) -> BuildSnapshot {
        let mut snapshot = snapshot();
        snapshot.source.high_watermark_change_id = high_watermark;
        snapshot.source.coverage_revision = high_watermark;
        snapshot
    }

    fn fact(change_id: i64) -> HistoryFact {
        let entity_key = format!("row-{change_id}");
        let payload = NormalizedFactPayload::test_fixture("live", &entity_key, change_id);
        let content_digest = payload.digest().unwrap();
        HistoryFact {
            source: ImportSource::Live,
            source_instance_id: "source-1".to_string(),
            generation: 2,
            artifact_id: "artifact-1".to_string(),
            artifact_bytes_digest:
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
            event_id: format!("event-{change_id}"),
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "invocation".to_string(),
                entity_key,
            },
            bucket: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            effective_change_id: change_id,
            operation: SourceOperation::Upsert,
            payload,
            content_digest,
            contribution_key: format!("bucket:{change_id}"),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        }
    }

    fn event(change_id: i64) -> SourceEventProof {
        let fact = fact(change_id);
        SourceEventProof {
            change_id,
            event_id: format!("event-{change_id}"),
            source_instance_id: "source-1".to_string(),
            generation: 2,
            coverage_revision: change_id,
            entity_kind: "invocation".to_string(),
            entity_key: format!("row-{change_id}"),
            operation: SourceOperation::Upsert,
            content_digest: fact.content_digest,
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
            &[fact(1), fact(3)],
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
            &[fact(1)],
            2,
        );
        assert!(result.is_err());
    }

    fn expected_for(prefix: &CertifiedReceiptPrefix) -> ExpectedHistoryBinding {
        let mut expected = ExpectedHistoryBinding::for_snapshot(&prefix.snapshot);
        expected.artifact_id = prefix.certificate.artifact_id.clone();
        expected.artifact_bytes_digest = prefix.certificate.artifact_bytes_digest.clone();
        expected.artifact_facts_digest = prefix.certificate.artifact_facts_digest.clone();
        expected
    }

    #[test]
    fn certified_prefix_supports_genesis_and_durable_predecessor_paths() {
        let p1 = build_certified_receipt_prefix(
            snapshot_at(1),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            None,
            vec![event(1)],
            vec![fact(1)],
            2,
        )
        .unwrap();
        let p1_digest = p1.certificate.certificate_digest.clone();
        let p2 = build_certified_receipt_prefix(
            snapshot_at(3),
            CertificateBaseline::CertifiedSourceFence {
                build_id: "build-1".to_string(),
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 1,
                coverage_revision: 1,
                certificate_digest: p1_digest.clone(),
            },
            Some(Box::new(p1)),
            vec![event(2), event(3)],
            vec![fact(2), fact(3)],
            2,
        )
        .unwrap();
        let expected = expected_for(&p2);
        let genesis = TrustedPrefixAnchor::genesis(2, 4, 8);
        let verified = p2.verify(&expected, &genesis).unwrap();
        assert_eq!(verified.exact_facts().count(), 3);
        assert_eq!(verified.through_change_id(), 3);

        let durable = TrustedPrefixAnchor::certified(p1_digest, 1, 1, 2, 4, 8);
        assert!(p2.verify(&expected, &durable).is_ok());
    }

    #[test]
    fn genesis_verification_rejects_rebuilt_replaced_predecessor() {
        let valid_p1 = build_certified_receipt_prefix(
            snapshot_at(1),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            None,
            vec![event(1)],
            vec![fact(1)],
            2,
        )
        .unwrap();
        let valid_p2 = build_certified_receipt_prefix(
            snapshot_at(3),
            CertificateBaseline::CertifiedSourceFence {
                build_id: "build-1".to_string(),
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 1,
                coverage_revision: 1,
                certificate_digest: valid_p1.certificate.certificate_digest.clone(),
            },
            Some(Box::new(valid_p1)),
            vec![event(2), event(3)],
            vec![fact(2), fact(3)],
            2,
        )
        .unwrap();
        let expected = expected_for(&valid_p2);

        let mut replaced_fact = fact(1);
        replaced_fact.payload.status = Some("failed".to_string());
        replaced_fact.payload.raw_payload = serde_json::to_value(&replaced_fact.payload).unwrap();
        replaced_fact
            .payload
            .raw_payload
            .as_object_mut()
            .unwrap()
            .remove("rawPayload");
        replaced_fact.content_digest = replaced_fact.payload.digest().unwrap();
        let mut replaced_event = event(1);
        replaced_event.content_digest = replaced_fact.content_digest.clone();
        let replaced_p1 = build_certified_receipt_prefix(
            snapshot_at(1),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            None,
            vec![replaced_event],
            vec![replaced_fact],
            2,
        )
        .unwrap();
        let rebuilt_p2 = build_certified_receipt_prefix(
            snapshot_at(3),
            CertificateBaseline::CertifiedSourceFence {
                build_id: "build-1".to_string(),
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 1,
                coverage_revision: 1,
                certificate_digest: replaced_p1.certificate.certificate_digest.clone(),
            },
            Some(Box::new(replaced_p1)),
            vec![event(2), event(3)],
            vec![fact(2), fact(3)],
            2,
        )
        .unwrap();
        assert!(
            rebuilt_p2
                .verify(&expected, &TrustedPrefixAnchor::genesis(2, 4, 8))
                .is_err()
        );
    }

    #[test]
    fn non_zero_baseline_without_predecessor_is_rejected() {
        let result = build_reconciliation_certificate_with_baseline(
            &snapshot_at(3),
            CertificateBaseline::CertifiedSourceFence {
                build_id: "build-1".to_string(),
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 1,
                coverage_revision: 1,
                certificate_digest:
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            },
            None,
            vec![event(2), event(3)],
            &[fact(2), fact(3)],
            2,
        );
        assert!(result.is_err());
    }

    #[test]
    fn source_operation_is_part_of_the_authenticated_certificate() {
        let certificate = build_reconciliation_certificate(
            &snapshot_at(3),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            vec![event(1), event(2), event(3)],
            &[fact(1), fact(2), fact(3)],
            2,
        )
        .unwrap();
        let mut changed = event(1);
        changed.operation = SourceOperation::Tombstone;
        assert!(
            certificate
                .verify(
                    &snapshot_at(3),
                    &[changed.clone(), event(2), event(3)],
                    &[fact(1), fact(2), fact(3)],
                    2,
                )
                .is_err()
        );
        let result = build_reconciliation_certificate(
            &snapshot_at(3),
            CertificateBaseline::SourceFence {
                source_instance_id: "source-1".to_string(),
                generation: 2,
                owner_epoch: 9,
                through_change_id: 0,
                coverage_revision: 0,
            },
            vec![changed, event(2), event(3)],
            &[fact(1), fact(2), fact(3)],
            2,
        );
        assert!(result.is_err());
    }
}
