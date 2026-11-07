//! Admission of a complete C5 history build. A durable source receipt is registered before
//! examining a candidate; neither its expected artifact hashes nor its trust root come from it.
use super::super::history::{
    certificate::{CertifiedReceiptPrefix, ExpectedHistoryBinding, TrustedPrefixAnchor},
    importer::{HistoryArtifact, HistoryFact, SourceOperation, facts_digest},
    seed::{HistoricalSeedArtifact, HistoricalSeedInput, SeedRow, create_historical_seed_proof},
    snapshot::{ArtifactKind, BuildSnapshot, StableArtifact, sha256_hex},
};
use super::*;
use sqlx::{Pool, Sqlite};

const MAX_BASELINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_BASELINE_FACTS: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PinnedArtifact {
    pub(crate) kind: ArtifactKind,
    pub(crate) manifest: super::super::history::snapshot::ManifestIdentity,
    pub(crate) bytes: Vec<u8>,
}

/// Persist the actual source snapshot/pin and decoded artifact receipts. This API accepts no
/// certificate or caller-supplied ExpectedHistoryBinding. The manifest set must already be
/// committed by the SQLite source snapshot producer; a candidate cannot bless its own bytes.
pub(crate) async fn register_baseline_source(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    source: &Pool<Sqlite>,
    snapshot: &BuildSnapshot,
    primary_artifact_id: &str,
    artifacts: &[PinnedArtifact],
) -> Result<()> {
    snapshot.validate()?;
    ensure!(artifacts.len() <= 256, "too many baseline artifacts");
    ensure!(
        artifacts
            .iter()
            .map(|a| a.bytes.len())
            .try_fold(0usize, usize::checked_add)
            .is_some_and(|n| n <= MAX_BASELINE_BYTES),
        "baseline exceeds byte budget"
    );
    let mut tx = actor.begin_writer_transaction().await?;
    lock_target(&mut tx, target).await?;
    check_build(&mut tx, target, snapshot).await?;
    let source_snapshot = super::super::source::assert_source_snapshot_usable(
        source,
        &target.build_id,
        &snapshot.source.source_instance_id,
        snapshot.source.generation,
        snapshot.source.high_watermark_change_id,
    )
    .await?;
    let manifests = snapshot.manifest_identities.clone();
    let manifest_digest = sha256_hex(&serde_json::to_vec(&manifests)?);
    ensure!(
        source_snapshot.high_watermark_change_id == snapshot.source.high_watermark_change_id
            && source_snapshot.coverage_revision == snapshot.source.coverage_revision
            && source_snapshot.retained_from_change_id == snapshot.source.retained_from_change_id
            && source_snapshot.fact_semantics_version == snapshot.source.fact_semantics_version
            && source_snapshot.stats_semantics_version == snapshot.source.stats_semantics_version
            && source_snapshot.manifest_digest.as_deref() == Some(&manifest_digest),
        "baseline snapshot differs from durable source manifest receipt"
    );
    let pin = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_source_retention_pins
        WHERE pin_id=?1 AND build_id=?2 AND state='active' AND watermark_change_id>=?3
          AND julianday(expires_at)>=julianday(?4) AND julianday(expires_at)>julianday('now')",
    )
    .bind(&snapshot.retention_pin_id)
    .bind(&snapshot.build_id)
    .bind(snapshot.source.high_watermark_change_id)
    .bind(&snapshot.pin_expires_at)
    .fetch_one(source)
    .await?;
    ensure!(pin == 1, "baseline source retention pin is unavailable");
    let mut facts = Vec::new();
    let mut seeds = Vec::new();
    let mut expected = None;
    let mut seen = std::collections::BTreeSet::new();
    for artifact in artifacts {
        ensure!(
            seen.insert(artifact.manifest.manifest_id.clone()),
            "duplicate baseline artifact"
        );
        ensure!(
            artifact.manifest.matches_bytes(&artifact.bytes)
                && manifests
                    .iter()
                    .any(|m| m.same_identity(&artifact.manifest)),
            "baseline artifact is not independently pinned"
        );
        let stable = StableArtifact {
            identity: artifact.manifest.clone(),
            bytes: artifact.bytes.clone(),
        };
        if artifact.kind == ArtifactKind::Seed {
            HistoricalSeedArtifact::from_verified_artifact(&stable, snapshot)?;
            seeds.push(artifact.clone());
        } else {
            let decoded =
                HistoryArtifact::from_verified_artifact(&stable, snapshot, artifact.kind)?;
            if artifact.manifest.manifest_id == primary_artifact_id {
                expected = Some(ExpectedHistoryBinding::for_snapshot_and_artifact(
                    snapshot, &decoded,
                )?);
            }
            let mut rows: Vec<HistoryFact> = serde_json::from_slice(&artifact.bytes)?;
            for row in &mut rows {
                row.artifact_id = artifact.manifest.manifest_id.clone();
                row.artifact_bytes_digest = artifact.manifest.sha256.clone();
            }
            facts.extend(rows);
        }
    }
    ensure!(
        seen.len() == manifests.len(),
        "baseline manifest artifacts are incomplete"
    );
    ensure!(
        facts.len() <= MAX_BASELINE_FACTS,
        "baseline exceeds fact budget"
    );
    let expected = expected.context("baseline primary artifact is missing")?;
    let trust = TrustedPrefixAnchor::genesis(2, 64, MAX_BASELINE_FACTS);
    let inserted = sqlx::query(
        "INSERT INTO cvm_analytics.analytics_projection_sources
        VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
    )
    .bind(&target.consumer_id)
    .bind(&target.build_id)
    .bind(Json(snapshot))
    .bind(Json(&expected))
    .bind(Json(&trust))
    .bind(Json(&facts))
    .bind(Json(&seeds))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        let stored = sqlx::query_as::<_, (Json<Value>,Json<Value>,Json<Value>,Json<Value>,Json<Value>)>(
            "SELECT snapshot,expected,trust,facts,seed_artifacts FROM cvm_analytics.analytics_projection_sources WHERE consumer_id=$1 AND build_id=$2")
            .bind(&target.consumer_id).bind(&target.build_id).fetch_one(&mut *tx).await?;
        ensure!(
            stored
                == (
                    Json(serde_json::to_value(snapshot)?),
                    Json(serde_json::to_value(expected)?),
                    Json(serde_json::to_value(trust)?),
                    Json(serde_json::to_value(facts)?),
                    Json(serde_json::to_value(seeds)?)
                ),
            "baseline source receipt already has another identity"
        );
    }
    tx.commit()
        .await
        .context("baseline source receipt commit outcome may be unknown")?;
    Ok(())
}

async fn check_build(
    tx: &mut Transaction<'_, Postgres>,
    target: &ProjectionTarget,
    snapshot: &BuildSnapshot,
) -> Result<()> {
    let valid=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM cvm_analytics.analytics_module_builds b
        JOIN cvm_analytics.analytics_module_consumers c ON c.consumer_id=b.consumer_id
        WHERE b.build_id=$1 AND b.consumer_id=$2 AND b.source_instance_id=$3 AND b.source_generation=$4
          AND b.source_high_watermark=$5 AND b.source_coverage_revision=$6 AND b.owner_epoch=$7
          AND b.retention_pin_id=$8 AND c.coverage_revision=$6 AND c.checkpoint>=$5)")
        .bind(&target.build_id).bind(&target.consumer_id).bind(&snapshot.source.source_instance_id)
        .bind(snapshot.source.generation).bind(snapshot.source.high_watermark_change_id)
        .bind(snapshot.source.coverage_revision).bind(target.owner_epoch).bind(&snapshot.retention_pin_id)
        .fetch_one(&mut **tx).await?;
    ensure!(
        valid
            && snapshot.build_id == target.build_id
            && snapshot.source.owner_epoch == target.owner_epoch,
        "baseline durable build fence changed"
    );
    Ok(())
}

/// Import all versions and separately certified seeds/coverage in a single bounded transaction.
/// Retained floor > 1 is never interpreted as permission to skip receipts without this proof.
pub(crate) async fn import_certified_baseline(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    candidate: &CertifiedReceiptPrefix,
) -> Result<i64> {
    let mut tx = actor.begin_writer_transaction().await?;
    let (checkpoint, _) = lock_target(&mut tx, target).await?;
    let (Json(snapshot),Json(expected),Json(trust),Json(facts),Json(artifacts)) =
        sqlx::query_as::<_,(Json<BuildSnapshot>,Json<ExpectedHistoryBinding>,Json<TrustedPrefixAnchor>,Json<Vec<HistoryFact>>,Json<Vec<PinnedArtifact>>)>(
        "SELECT snapshot,expected,trust,facts,seed_artifacts FROM cvm_analytics.analytics_projection_sources
         WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_optional(&mut *tx).await?
        .context("baseline has no durable source receipt")?;
    check_build(&mut tx, target, &snapshot).await?;
    let verified = candidate.verify(&expected, &trust)?;
    ensure!(
        verified.snapshot() == &snapshot,
        "baseline snapshot identity changed"
    );
    ensure!(
        verified.through_change_id() == snapshot.source.high_watermark_change_id
            && verified.through_change_id() <= checkpoint,
        "baseline does not cover build prefix"
    );
    let complete: Vec<HistoryFact> = verified
        .exact_facts()
        .take(MAX_BASELINE_FACTS + 1)
        .cloned()
        .collect();
    ensure!(
        facts.len() == complete.len() && facts_digest(&facts)? == facts_digest(&complete)?,
        "baseline complete chain differs from independently decoded artifacts"
    );
    let mut seeds = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for seed in verified.certified_seeds() {
        let artifact = artifacts
            .iter()
            .find(|a| a.manifest.same_identity(&seed.proof().manifest))
            .context("certified seed lacks independent artifact receipt")?;
        let stable = StableArtifact {
            identity: artifact.manifest.clone(),
            bytes: artifact.bytes.clone(),
        };
        // CertifiedSeed is opaque outside C5. Chain provenance/coverage was verified
        // above; independently rebind its rows and complete proof to actual pinned bytes.
        // Re-authenticating an attached seed would overlap its own coverage claim.
        HistoricalSeedArtifact::from_verified_artifact(&stable, &snapshot)?;
        let rows: Vec<SeedRow> = serde_json::from_slice(&stable.bytes)?;
        let proof = seed.proof();
        let rebuilt = create_historical_seed_proof(HistoricalSeedInput {
            seed_id: proof.seed_id.clone(),
            source: proof.source.clone(),
            manifest: artifact.manifest.clone(),
            coverage: proof.coverage.clone(),
            capabilities: proof.capabilities.clone(),
            rows: rows.clone(),
            certificate_digest: seed.provenance_digest().to_owned(),
            sample_limit: proof.sample_digests.len(),
        })?;
        ensure!(
            rebuilt == *proof
                && serde_json::to_value(&rows)?
                    == serde_json::to_value(seed.rows().collect::<Vec<_>>())?,
            "certified seed differs from independent artifact rows/proof"
        );
        ensure!(
            used.insert(artifact.manifest.manifest_id.clone()),
            "duplicate certified seed"
        );
        seeds.push(serde_json::json!({"proof":seed.proof(),
            "rows":seed.rows().collect::<Vec<_>>(),"capabilities":seed.capabilities()}));
    }
    ensure!(
        used.len() == artifacts.len(),
        "baseline seed artifacts are incomplete"
    );
    let existing=sqlx::query_as::<_,(i64,String,String,Json<Value>,Json<Value>)>("SELECT through_change_id,certificate_digest,prefix_digest,coverage,seeds FROM cvm_analytics.analytics_projection_baselines WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_optional(&mut *tx).await?;
    if let Some((through, digest, prefix, Json(coverage), Json(stored_seeds))) = existing {
        ensure!(
            through == verified.through_change_id()
                && digest == verified.certificate_digest()
                && prefix == verified.prefix_digest()
                && coverage == serde_json::to_value(verified.coverage().claims())?
                && stored_seeds == serde_json::to_value(&seeds)?,
            "baseline is already activated with another certificate"
        );
        return Ok(through);
    }
    let cursor=sqlx::query_scalar::<_,i64>("SELECT materialized_through FROM cvm_analytics.analytics_projection_progress WHERE consumer_id=$1 AND build_id=$2 FOR UPDATE")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_optional(&mut *tx).await?;
    ensure!(
        cursor.is_none_or(|n| n == 0),
        "baseline must precede receipt replay"
    );
    let count=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cvm_analytics.analytics_projection_facts WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_one(&mut *tx).await?;
    ensure!(count == 0, "baseline target already contains versions");
    for fact in &complete {
        let payload = fact.normalized_payload();
        payload.validate()?;
        ensure!(
            fact.identity().entity_kind == payload.entity_kind(),
            "baseline entity kind differs from payload"
        );
        model::normalize(
            payload.entity_kind(),
            &fact.identity().entity_key,
            payload.payload(),
        )?;
        sqlx::query(
            "INSERT INTO cvm_analytics.analytics_projection_facts VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(&target.consumer_id)
        .bind(&target.build_id)
        .bind(payload.entity_kind())
        .bind(&fact.identity().entity_key)
        .bind(fact.effective_change_id())
        .bind(fact.operation() == SourceOperation::Tombstone)
        .bind(Json(payload.payload()))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_projection_baselines VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(&target.consumer_id)
    .bind(&target.build_id)
    .bind(verified.through_change_id())
    .bind(verified.certificate_digest())
    .bind(verified.prefix_digest())
    .bind(Json(verified.coverage().claims()))
    .bind(Json(seeds))
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO cvm_analytics.analytics_projection_progress VALUES ($1,$2,$3)
        ON CONFLICT (consumer_id,build_id) DO UPDATE SET materialized_through=EXCLUDED.materialized_through")
        .bind(&target.consumer_id).bind(&target.build_id).bind(verified.through_change_id()).execute(&mut *tx).await?;
    tx.commit().await.context(
        "baseline activation commit outcome may be unknown; reconcile certificate identity",
    )?;
    Ok(verified.through_change_id())
}
