use super::certificate::{CertifiedReceiptPrefix, ExpectedHistoryBinding, TrustedPrefixAnchor};
use super::coverage::{CoverageKey, CoverageLedger, CoverageRange, SeedCapability};
use super::importer::StableEntityIdentity;
use super::snapshot::{BuildSnapshot, ManifestIdentity, SourceFence, sha256_hex};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedRow {
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) identity: StableEntityIdentity,
    pub(crate) key: CoverageKey,
    pub(crate) range: CoverageRange,
    pub(crate) content_digest: String,
    pub(crate) contribution_digest: String,
    pub(crate) aggregate_value: Option<f64>,
    pub(crate) sample_count: Option<u64>,
}

impl SeedRow {
    fn validate_against(
        &self,
        source: &SourceFence,
        coverage: &[(CoverageKey, CoverageRange)],
    ) -> Result<()> {
        ensure!(
            self.source_instance_id == source.source_instance_id,
            "historical seed row belongs to another source instance"
        );
        ensure!(
            self.generation == source.generation,
            "historical seed row generation changed"
        );
        ensure!(
            self.identity.source_instance_id == source.source_instance_id,
            "historical seed entity belongs to another source instance"
        );
        self.key.validate()?;
        ensure!(
            coverage
                .iter()
                .any(|(key, range)| key == &self.key && range.contains(&self.range)),
            "historical seed row is outside its certified coverage"
        );
        ensure!(
            is_digest(&self.content_digest),
            "historical seed content digest is invalid"
        );
        ensure!(
            is_digest(&self.contribution_digest),
            "historical seed contribution digest is invalid"
        );
        if let Some(value) = self.aggregate_value {
            ensure!(
                value.is_finite(),
                "historical seed aggregate value is invalid"
            );
        }
        if let Some(count) = self.sample_count {
            ensure!(count > 0, "historical seed sample count must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricalSeedProof {
    pub(crate) seed_id: String,
    pub(crate) source: SourceFence,
    pub(crate) manifest: ManifestIdentity,
    pub(crate) coverage: Vec<(CoverageKey, CoverageRange)>,
    pub(crate) capabilities: BTreeSet<SeedCapability>,
    pub(crate) row_count: usize,
    pub(crate) content_digest: String,
    pub(crate) sample_digests: Vec<String>,
    pub(crate) certificate_digest: String,
    pub(crate) proof_digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoricalSeedInput {
    pub(crate) seed_id: String,
    pub(crate) source: SourceFence,
    pub(crate) manifest: ManifestIdentity,
    pub(crate) coverage: Vec<(CoverageKey, CoverageRange)>,
    pub(crate) capabilities: BTreeSet<SeedCapability>,
    pub(crate) rows: Vec<SeedRow>,
    pub(crate) certificate_digest: String,
    pub(crate) sample_limit: usize,
}

pub(crate) fn create_historical_seed_proof(
    input: HistoricalSeedInput,
) -> Result<HistoricalSeedProof> {
    let proof = build_proof(input)?;
    proof.validate_shape()?;
    Ok(proof)
}

pub(crate) fn authenticate_historical_seed(
    snapshot: &BuildSnapshot,
    proof: &HistoricalSeedProof,
    rows: &[SeedRow],
    receipt: &CertifiedReceiptPrefix,
    expected: &ExpectedHistoryBinding,
    trust: &TrustedPrefixAnchor,
) -> Result<CertifiedSeed> {
    snapshot.validate()?;
    let verified = receipt.verify(expected, trust)?;
    ensure!(
        verified.snapshot().build_id == snapshot.build_id,
        "historical seed receipt build changed"
    );
    proof.validate_shape()?;
    ensure!(
        proof.source.same_source(&snapshot.source),
        "historical seed source fence changed"
    );
    ensure!(
        proof.source.high_watermark_change_id <= snapshot.source.high_watermark_change_id,
        "historical seed is beyond the source snapshot watermark"
    );
    ensure!(
        snapshot
            .manifest_identities
            .iter()
            .any(|manifest| manifest.same_identity(&proof.manifest)),
        "historical seed manifest identity is not pinned by the snapshot"
    );
    ensure!(
        proof.certificate_digest == verified.certificate_digest(),
        "historical seed certificate digest is not independently authenticated"
    );
    ensure!(
        proof.source == *verified.source_fence(),
        "historical seed watermark or coverage fence is not authenticated"
    );
    validate_seed_bucket_evidence(&verified, rows)?;
    let expected = build_proof(HistoricalSeedInput {
        seed_id: proof.seed_id.clone(),
        source: proof.source.clone(),
        manifest: proof.manifest.clone(),
        coverage: proof.coverage.clone(),
        capabilities: proof.capabilities.clone(),
        rows: rows.to_vec(),
        certificate_digest: verified.certificate_digest().to_string(),
        sample_limit: proof.sample_digests.len(),
    })?;
    ensure!(
        expected.row_count == proof.row_count,
        "historical seed row count changed"
    );
    ensure!(
        expected.content_digest == proof.content_digest,
        "historical seed content digest changed"
    );
    ensure!(
        expected.sample_digests == proof.sample_digests,
        "historical seed sample evidence changed"
    );
    ensure!(
        expected.proof_digest == proof.proof_digest,
        "historical seed proof digest changed"
    );
    let mut coverage_ledger = CoverageLedger::default();
    for (key, range) in &proof.coverage {
        coverage_ledger.admit_seed(
            key.clone(),
            range.clone(),
            verified.certificate_digest(),
            proof.capabilities.clone(),
        )?;
    }
    Ok(CertifiedSeed {
        proof: proof.clone(),
        rows: rows.to_vec(),
        provenance_digest: verified.certificate_digest().to_string(),
        coverage: coverage_ledger,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct CertifiedSeed {
    proof: HistoricalSeedProof,
    rows: Vec<SeedRow>,
    provenance_digest: String,
    coverage: CoverageLedger,
}

impl CertifiedSeed {
    pub(crate) fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }

    pub(crate) fn coverage(&self) -> &CoverageLedger {
        &self.coverage
    }

    pub(crate) fn proof(&self) -> &HistoricalSeedProof {
        &self.proof
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = &SeedRow> {
        self.rows.iter()
    }

    pub(crate) fn capabilities(&self) -> &BTreeSet<SeedCapability> {
        &self.proof.capabilities
    }
}

impl HistoricalSeedProof {
    pub(crate) fn validate_shape(&self) -> Result<()> {
        ensure!(
            !self.seed_id.trim().is_empty(),
            "historical seed id is empty"
        );
        self.source.validate()?;
        self.manifest.validate()?;
        ensure!(
            self.manifest.source_instance_id == self.source.source_instance_id,
            "historical seed manifest source instance mismatch"
        );
        ensure!(
            self.manifest.generation == self.source.generation,
            "historical seed manifest generation mismatch"
        );
        ensure!(
            self.manifest.coverage_revision <= self.source.coverage_revision,
            "historical seed manifest is beyond the source fence"
        );
        ensure!(
            !self.coverage.is_empty(),
            "historical seed has no certified coverage"
        );
        ensure!(
            !self.capabilities.is_empty(),
            "historical seed declares no capabilities"
        );
        ensure!(
            is_digest(&self.certificate_digest),
            "historical seed certificate digest is invalid"
        );
        ensure!(
            is_digest(&self.content_digest),
            "historical seed content digest is invalid"
        );
        ensure!(
            self.sample_digests.iter().all(|digest| is_digest(digest)),
            "historical seed sample digest is invalid"
        );
        ensure!(
            is_digest(&self.proof_digest),
            "historical seed proof digest is invalid"
        );
        let mut ledger = CoverageLedger::default();
        for (key, range) in &self.coverage {
            ledger.admit_seed(
                key.clone(),
                range.clone(),
                self.seed_id.clone(),
                self.capabilities.clone(),
            )?;
        }
        validate_capabilities(&self.coverage, &self.capabilities)
    }
}

fn build_proof(input: HistoricalSeedInput) -> Result<HistoricalSeedProof> {
    let HistoricalSeedInput {
        seed_id,
        source,
        manifest,
        coverage,
        capabilities,
        rows,
        certificate_digest,
        sample_limit,
    } = input;
    ensure!(
        sample_limit > 0,
        "historical seed sample limit must be positive"
    );
    ensure!(!rows.is_empty(), "historical seed requires rows");
    source.validate()?;
    manifest.validate()?;
    ensure!(
        manifest.source_instance_id == source.source_instance_id,
        "historical seed manifest source instance mismatch"
    );
    ensure!(
        manifest.generation == source.generation,
        "historical seed manifest generation mismatch"
    );
    ensure!(
        manifest.coverage_revision <= source.coverage_revision,
        "historical seed manifest is beyond the source fence"
    );
    ensure!(
        is_digest(&certificate_digest),
        "historical seed certificate digest is invalid"
    );
    let mut ledger = CoverageLedger::default();
    for (key, range) in &coverage {
        ledger.admit_seed(
            key.clone(),
            range.clone(),
            seed_id.clone(),
            capabilities.clone(),
        )?;
    }
    validate_capabilities(&coverage, &capabilities)?;

    let mut sorted_rows = rows;
    sorted_rows.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.range.start.cmp(&right.range.start))
            .then_with(|| left.identity.cmp(&right.identity))
            .then_with(|| left.content_digest.cmp(&right.content_digest))
    });
    let mut identities = BTreeSet::new();
    for row in &sorted_rows {
        row.validate_against(&source, &coverage)?;
        ensure!(
            identities.insert(row.identity.clone()),
            "historical seed contains the same source entity twice"
        );
    }
    let content_digest = digest_rows(&sorted_rows)?;
    let sample_digests = sorted_rows
        .iter()
        .take(sample_limit)
        .map(|row| row.content_digest.clone())
        .collect();
    let mut proof = HistoricalSeedProof {
        seed_id,
        source,
        manifest,
        coverage,
        capabilities,
        row_count: sorted_rows.len(),
        content_digest,
        sample_digests,
        certificate_digest,
        proof_digest: String::new(),
    };
    proof.proof_digest = proof_digest(&proof)?;
    Ok(proof)
}

fn digest_rows(rows: &[SeedRow]) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(rows)?))
}

fn proof_digest(proof: &HistoricalSeedProof) -> Result<String> {
    let mut material = proof.clone();
    material.proof_digest.clear();
    Ok(sha256_hex(&serde_json::to_vec(&material)?))
}

fn validate_capabilities(
    coverage: &[(CoverageKey, CoverageRange)],
    capabilities: &BTreeSet<SeedCapability>,
) -> Result<()> {
    for capability in capabilities {
        ensure!(
            coverage
                .iter()
                .all(|(key, _)| capability_matches_key(*capability, key)),
            "historical seed capability is not supported by every certified coverage key"
        );
    }
    Ok(())
}

fn capability_matches_key(capability: SeedCapability, key: &CoverageKey) -> bool {
    match capability {
        SeedCapability::Calls => key.metric == "calls",
        SeedCapability::Tokens => key.metric == "tokens",
        SeedCapability::Cost => key.metric == "cost",
        SeedCapability::OverallTotals => key.dimension == "overall",
        SeedCapability::ModelBreakdown => key.dimension == "model",
        SeedCapability::UpstreamBreakdown => key.dimension == "upstream",
        SeedCapability::ErrorBreakdown => key.dimension == "error",
        SeedCapability::WallTime => key.metric == "wall_time",
        SeedCapability::PerformanceSamples => key.metric == "performance_samples",
        SeedCapability::InvocationDetails => {
            matches!(key.dimension.as_str(), "invocation" | "pool_attempt")
        }
    }
}

fn validate_seed_bucket_evidence(
    verified: &super::certificate::VerifiedReceiptPrefix<'_>,
    rows: &[SeedRow],
) -> Result<()> {
    for row in rows {
        let mut covered = false;
        for proof in verified
            .bucket_proofs()
            .filter(|proof| proof.key == row.key && proof.range.contains(&row.range))
        {
            if proof.entities.iter().any(|entity| {
                entity.identity == row.identity
                    && entity.content_digest == row.content_digest
                    && entity.contribution_digest == row.contribution_digest
            }) {
                covered = true;
                break;
            }
        }
        ensure!(
            covered,
            "historical seed row lacks certificate bucket evidence"
        );
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{BuildSnapshot, SourceFence};
    use super::*;

    fn source() -> SourceFence {
        SourceFence {
            source_instance_id: "source-1".to_string(),
            generation: 7,
            owner_epoch: 3,
            high_watermark_change_id: 10,
            coverage_revision: 10,
            retained_from_change_id: 0,
            fact_semantics_version: 1,
            stats_semantics_version: 2,
        }
    }

    fn row(key: &str) -> SeedRow {
        SeedRow {
            source_instance_id: "source-1".to_string(),
            generation: 7,
            identity: StableEntityIdentity {
                source_instance_id: "source-1".to_string(),
                entity_kind: "rollup".to_string(),
                entity_key: key.to_string(),
            },
            key: CoverageKey::new("calls", "overall", "day").unwrap(),
            range: CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            content_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            contribution_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            aggregate_value: Some(1.0),
            sample_count: Some(1),
        }
    }

    fn proof(rows: &[SeedRow]) -> HistoricalSeedProof {
        let mut capabilities = BTreeSet::new();
        capabilities.insert(SeedCapability::Calls);
        capabilities.insert(SeedCapability::OverallTotals);
        let manifest =
            ManifestIdentity::from_bytes("seed-manifest", "source-1", 7, 10, b"seed").unwrap();
        create_historical_seed_proof(HistoricalSeedInput {
            seed_id: "seed-1".to_string(),
            source: source(),
            manifest,
            coverage: vec![(
                CoverageKey::new("calls", "overall", "day").unwrap(),
                CoverageRange::new("2026-01-01", "2026-01-02").unwrap(),
            )],
            capabilities,
            rows: rows.to_vec(),
            certificate_digest: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            sample_limit: 2,
        })
        .unwrap()
    }

    #[test]
    fn authenticated_seed_does_not_invent_model_detail() {
        let rows = vec![row("row-1")];
        let mut historical = proof(&rows);
        historical
            .capabilities
            .insert(SeedCapability::ModelBreakdown);
        assert!(historical.validate_shape().is_err());
    }

    #[test]
    fn authentication_rejects_changed_rows() {
        let rows = vec![row("row-1")];
        let historical = proof(&rows);
        let _snapshot = BuildSnapshot {
            build_id: "build-1".to_string(),
            source: source(),
            retention_pin_id: "pin-1".to_string(),
            pin_expires_at: "2999-01-01T00:00:00Z".to_string(),
            manifest_identities: vec![historical.manifest.clone()],
            recovery_artifact_id: "recovery-1".to_string(),
        };
        let mut changed = rows;
        changed[0].content_digest =
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string();
        let result = create_historical_seed_proof(HistoricalSeedInput {
            seed_id: historical.seed_id.clone(),
            source: historical.source.clone(),
            manifest: historical.manifest.clone(),
            coverage: historical.coverage.clone(),
            capabilities: historical.capabilities.clone(),
            rows: changed,
            certificate_digest: historical.certificate_digest.clone(),
            sample_limit: historical.sample_digests.len(),
        });
        assert!(result.is_err() || result.unwrap().proof_digest != historical.proof_digest);
    }
}
