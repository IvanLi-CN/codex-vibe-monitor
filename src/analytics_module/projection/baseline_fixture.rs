use super::super::history::{
    certificate::{
        CertificateBaseline, CertifiedReceiptPrefix, ExpectedHistoryBinding, SourceEventProof,
        TrustedPrefixAnchor, build_certified_receipt_prefix,
    },
    coverage::{CoverageKey, CoverageRange, SeedCapability},
    importer::{
        HistoryArtifact, HistoryFact, ImportSource, NormalizedFactPayload, SourceOperation,
    },
    seed::{
        HistoricalSeedArtifact, HistoricalSeedInput, SeedRow, authenticate_historical_seed,
        create_historical_seed_proof,
    },
    snapshot::{
        ArtifactKind, BuildSnapshot, ManifestIdentity, SourceFence, StableArtifact, sha256_hex,
    },
};
use super::baseline::PinnedArtifact;
use super::*;

pub(super) fn fixture() -> Result<(BuildSnapshot, Vec<PinnedArtifact>, CertifiedReceiptPrefix)> {
    fixture_variant(false, true)
}

pub(super) fn fixture_variant(
    change_payload: bool,
    include_seed: bool,
) -> Result<(BuildSnapshot, Vec<PinnedArtifact>, CertifiedReceiptPrefix)> {
    let mut facts = Vec::new();
    let mut events = Vec::new();
    for change in 1..=2 {
        let payload = NormalizedFactPayload::test_fixture("live", &format!("row-{change}"), change);
        let digest = payload.digest()?;
        facts.push(HistoryFact {
            source: ImportSource::Live,
            source_instance_id: "source-c4".into(),
            generation: 1,
            artifact_id: "history".into(),
            artifact_bytes_digest: "a".repeat(64),
            event_id: format!("history-{change}"),
            identity: super::super::history::importer::StableEntityIdentity {
                source_instance_id: "source-c4".into(),
                entity_kind: "invocation".into(),
                entity_key: format!("row-{change}"),
            },
            bucket: CoverageKey::new("calls", "overall", "day")?,
            range: CoverageRange::new("2026-01-01", "2026-01-02")?,
            effective_change_id: change,
            operation: SourceOperation::Upsert,
            payload,
            content_digest: digest.clone(),
            contribution_key: format!("contribution-{change}"),
            contribution_digest: "b".repeat(64),
        });
        events.push(SourceEventProof {
            change_id: change,
            event_id: format!("history-{change}"),
            source_instance_id: "source-c4".into(),
            generation: 1,
            coverage_revision: 0,
            entity_kind: "invocation".into(),
            entity_key: format!("row-{change}"),
            operation: SourceOperation::Upsert,
            content_digest: digest,
        });
    }
    let bytes = serde_json::to_vec(&facts)?;
    let manifest = ManifestIdentity::from_bytes("history", "source-c4", 1, 0, &bytes)?;
    for fact in &mut facts {
        fact.artifact_bytes_digest = manifest.sha256.clone();
    }
    if change_payload {
        facts[1].payload.model = Some("forged-candidate-model".into());
        let mut canonical = serde_json::to_value(&facts[1].payload)?;
        canonical.as_object_mut().unwrap().remove("rawPayload");
        facts[1].payload.raw_payload = canonical;
        facts[1].content_digest = facts[1].payload.digest()?;
        events[1].content_digest = facts[1].content_digest.clone();
    }
    let rows = vec![SeedRow {
        source_instance_id: "source-c4".into(),
        generation: 1,
        identity: super::super::history::importer::StableEntityIdentity {
            source_instance_id: "source-c4".into(),
            entity_kind: "historical_seed".into(),
            entity_key: "seed-row".into(),
        },
        key: CoverageKey::new("calls", "overall", "day")?,
        range: CoverageRange::new("2025-12-01", "2025-12-02")?,
        content_digest: "c".repeat(64),
        contribution_digest: "d".repeat(64),
        aggregate_value: Some(7.0),
        sample_count: None,
    }];
    let seed_bytes = serde_json::to_vec(&rows)?;
    let seed_manifest = ManifestIdentity::from_bytes("seed", "source-c4", 1, 0, &seed_bytes)?;
    let mut snapshot = BuildSnapshot {
        build_id: "build-baseline".into(),
        source: SourceFence {
            source_instance_id: "source-c4".into(),
            generation: 1,
            owner_epoch: 1,
            high_watermark_change_id: 1,
            coverage_revision: 0,
            retained_from_change_id: 1,
            fact_semantics_version: 1,
            stats_semantics_version: 2,
        },
        retention_pin_id: "source-pin".into(),
        pin_expires_at: "2999-01-01T00:00:00Z".into(),
        manifest_identities: vec![manifest.clone(), seed_manifest.clone()],
        recovery_artifact_id: "recovery-history".into(),
    };
    let predecessor = build_certified_receipt_prefix(
        snapshot.clone(),
        CertificateBaseline::SourceFence {
            source_instance_id: "source-c4".into(),
            generation: 1,
            owner_epoch: 1,
            through_change_id: 0,
            coverage_revision: 0,
        },
        None,
        vec![events[0].clone()],
        vec![facts[0].clone()],
        2,
    )?;
    let digest = predecessor.certificate_digest_for_test().to_owned();
    snapshot.source.high_watermark_change_id = 2;
    snapshot.source.retained_from_change_id = 2;
    let prefix = build_certified_receipt_prefix(
        snapshot.clone(),
        CertificateBaseline::CertifiedSourceFence {
            build_id: snapshot.build_id.clone(),
            source_instance_id: "source-c4".into(),
            generation: 1,
            owner_epoch: 1,
            through_change_id: 1,
            coverage_revision: 0,
            certificate_digest: digest,
        },
        Some(Box::new(predecessor)),
        vec![events[1].clone()],
        vec![facts[1].clone()],
        2,
    )?;
    let stable = StableArtifact {
        identity: manifest.clone(),
        bytes: bytes.clone(),
    };
    let expected = ExpectedHistoryBinding::for_snapshot_and_artifact(
        &snapshot,
        &HistoryArtifact::from_verified_artifact(&stable, &snapshot, ArtifactKind::Live)?,
    )?;
    let proof = create_historical_seed_proof(HistoricalSeedInput {
        seed_id: "seed-proof".into(),
        source: snapshot.source.clone(),
        manifest: seed_manifest.clone(),
        coverage: vec![(rows[0].key.clone(), rows[0].range.clone())],
        capabilities: [SeedCapability::Calls].into_iter().collect(),
        rows,
        certificate_digest: prefix.certificate_digest_for_test().to_owned(),
        sample_limit: 2,
    })?;
    if change_payload || !include_seed {
        return Ok((
            snapshot,
            vec![
                PinnedArtifact {
                    kind: ArtifactKind::Live,
                    manifest,
                    bytes,
                },
                PinnedArtifact {
                    kind: ArtifactKind::Seed,
                    manifest: seed_manifest,
                    bytes: seed_bytes,
                },
            ],
            prefix,
        ));
    }
    let seed = authenticate_historical_seed(
        &snapshot,
        &proof,
        &HistoricalSeedArtifact::new(seed_manifest.clone(), seed_bytes.clone())?,
        &prefix,
        &expected,
        &TrustedPrefixAnchor::genesis(2, 64, 100_000),
    )?;
    let prefix = prefix.attach_certified_seed(seed)?;
    Ok((
        snapshot,
        vec![
            PinnedArtifact {
                kind: ArtifactKind::Live,
                manifest,
                bytes,
            },
            PinnedArtifact {
                kind: ArtifactKind::Seed,
                manifest: seed_manifest,
                bytes: seed_bytes,
            },
        ],
        prefix,
    ))
}

pub(super) async fn sqlite_source(snapshot: &BuildSnapshot) -> Result<sqlx::SqlitePool> {
    let source = sqlx::SqlitePool::connect("sqlite::memory:").await?;
    super::super::source::ensure_source_snapshot_schema(&source).await?;
    // Minimal source journal control fixture; source snapshot APIs remain the production path.
    sqlx::raw_sql("CREATE TABLE analytics_change_state(id INTEGER,generation INTEGER,retained_from_change_id INTEGER,capacity INTEGER,invalidation_pending INTEGER);
        INSERT INTO analytics_change_state VALUES(1,1,2,100,0); CREATE TABLE analytics_change_events(change_id INTEGER)")
        .execute(&source).await?;
    super::super::source::create_source_snapshot(
        &source,
        &super::super::source::SourceSnapshot {
            build_id: snapshot.build_id.clone(),
            source_instance_id: snapshot.source.source_instance_id.clone(),
            generation: 1,
            high_watermark_change_id: 2,
            coverage_revision: 0,
            retained_from_change_id: 2,
            fact_semantics_version: 1,
            stats_semantics_version: 2,
            manifest_digest: Some(sha256_hex(&serde_json::to_vec(
                &snapshot.manifest_identities,
            )?)),
        },
    )
    .await?;
    super::super::source::renew_source_retention_pin(
        &source,
        &super::super::source::SourceRetentionPin {
            pin_id: snapshot.retention_pin_id.clone(),
            build_id: snapshot.build_id.clone(),
            source_kind: "recovery".into(),
            source_identity: snapshot.recovery_artifact_id.clone(),
            watermark_change_id: 2,
            expires_at: snapshot.pin_expires_at.clone(),
        },
    )
    .await?;
    Ok(source)
}

#[test]
fn retained_floor_chain_and_seed_fixture_is_independently_bound() -> Result<()> {
    let (snapshot, artifacts, candidate) = fixture()?;
    let expected = ExpectedHistoryBinding::for_snapshot_and_artifact(
        &snapshot,
        &HistoryArtifact::from_verified_artifact(
            &StableArtifact {
                identity: artifacts[0].manifest.clone(),
                bytes: artifacts[0].bytes.clone(),
            },
            &snapshot,
            ArtifactKind::Live,
        )?,
    )?;
    let verified = candidate.verify(&expected, &TrustedPrefixAnchor::genesis(2, 64, 100_000))?;
    assert_eq!(verified.exact_facts().count(), 2);
    assert_eq!(verified.certified_seeds().count(), 1);
    assert_eq!(verified.source_fence().retained_from_change_id, 2);
    let mut wrong = expected;
    wrong.artifact_bytes_digest = "f".repeat(64);
    assert!(
        candidate
            .verify(&wrong, &TrustedPrefixAnchor::genesis(2, 64, 100_000))
            .is_err()
    );
    Ok(())
}
