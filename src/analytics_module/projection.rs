//! Durable projection work on C3's fenced writer session. Latest received facts are never
//! query-visible until a complete staging model and its publication pointer commit together.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Postgres, Transaction, types::Json};

use super::ingest::PgIngestActor;

#[path = "projection/baseline.rs"]
pub(crate) mod baseline;

#[path = "projection/migration.rs"]
mod migration;
#[path = "projection/model.rs"]
mod model;
pub(crate) async fn apply_analytics_projection_migration(
    connection: &mut PgConnection,
) -> Result<AnalyticsProjectionMigrationResult> {
    migration::apply_analytics_projection_migration(connection).await
}
pub(crate) use model::{ProjectionFact, ProjectionModels, build_models};

pub(crate) const ANALYTICS_PROJECTION_DDL_COMPONENT: &str = "analytics-module-projection";
pub(crate) const ANALYTICS_PROJECTION_DDL_STEP: &str = "versioned-projection-v1";
pub(crate) const ANALYTICS_PROJECTION_SCHEMA_VERSION: i64 = 2;
pub(crate) const ANALYTICS_PROJECTION_READER_VERSION: i64 = 2;
const ANALYTICS_PROJECTION_MIGRATION_LOCK_KEY: i64 = 0x0043_564d_5f50_524a;
pub(crate) const ANALYTICS_PROJECTION_SCHEMA_SQL: &str = include_str!("projection/schema.sql");

#[derive(Debug)]
pub(crate) struct AnalyticsProjectionMigrationResult {
    pub(crate) component: &'static str,
    pub(crate) version: i64,
    pub(crate) checksum: String,
    pub(crate) recovered_steps: usize,
}

/// Identity is checked against both the C3 session fence and durable module metadata.
#[derive(Debug, Clone)]
pub(crate) struct ProjectionTarget {
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublicationToken {
    pub(crate) publication_id: String,
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) through_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) projection_version: i64,
}

async fn lock_target(
    tx: &mut Transaction<'_, Postgres>,
    target: &ProjectionTarget,
) -> Result<(i64, i64)> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        "SELECT c.checkpoint, c.coverage_revision
         FROM cvm_analytics.analytics_module_consumers c
         JOIN cvm_analytics.analytics_module_builds b ON b.consumer_id = c.consumer_id
         JOIN cvm_analytics.analytics_module_owners o ON o.module_id = c.module_id
         WHERE c.consumer_id = $1 AND b.build_id = $2 AND c.owner_epoch = $3
           AND b.owner_epoch = c.owner_epoch AND o.owner_epoch = c.owner_epoch
           AND o.status = 'active' AND o.source_instance_id = c.source_instance_id
           AND b.source_instance_id = c.source_instance_id AND b.module_id = c.module_id
           AND c.state IN ('building', 'active') AND b.state IN ('building', 'ready')
         FOR UPDATE OF c, b, o",
    )
    .bind(&target.consumer_id)
    .bind(&target.build_id)
    .bind(target.owner_epoch)
    .fetch_optional(&mut **tx)
    .await?
    .context("projection target is fenced or unavailable")?;
    Ok(row)
}

/// Replay a bounded received prefix. All payloads already live durably in C3 receipts;
/// C can safely lead this cursor. Dirty obligations and immutable versions commit together.
pub(crate) async fn materialize_batch(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    limit: i64,
) -> Result<i64> {
    ensure!(
        (1..=256).contains(&limit),
        "projection batch limit must be 1..256"
    );
    let mut tx = actor.begin_writer_transaction().await?;
    let (checkpoint, _) = lock_target(&mut tx, target).await?;
    sqlx::query("INSERT INTO cvm_analytics.analytics_projection_progress VALUES ($1,$2,0) ON CONFLICT DO NOTHING")
        .bind(&target.consumer_id).bind(&target.build_id).execute(&mut *tx).await?;
    let cursor = sqlx::query_scalar::<_, i64>("SELECT materialized_through FROM cvm_analytics.analytics_projection_progress WHERE consumer_id=$1 AND build_id=$2 FOR UPDATE")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_one(&mut *tx).await?;
    let rows = sqlx::query_as::<_, (i64, String, String, String, Json<Value>)>(
        "SELECT r.change_id,r.entity_kind,r.entity_key,r.operation_kind,r.payload
         FROM cvm_analytics.analytics_module_ingest_receipts r
         JOIN cvm_analytics.analytics_module_consumers c ON c.consumer_id=r.consumer_id
         WHERE r.consumer_id=$1 AND r.source_instance_id=c.source_instance_id
           AND r.generation=(SELECT source_generation FROM cvm_analytics.analytics_module_builds WHERE build_id=$5)
           AND r.change_id>$2 AND r.change_id<=$3 ORDER BY r.change_id LIMIT $4",
    )
    .bind(&target.consumer_id)
    .bind(cursor)
    .bind(checkpoint)
    .bind(limit)
    .bind(&target.build_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut through = cursor;
    for (change, kind, key, operation, Json(payload)) in rows {
        ensure!(change == through + 1, "projection receipt prefix has a gap");
        ensure!(
            matches!(
                operation.as_str(),
                "upsert" | "correction" | "logical_revoke"
            ),
            "projection requires an authenticated history handler for this operation"
        );
        if operation != "logical_revoke" {
            model::normalize(&kind, &key, &payload)?;
        }
        // Without an exact old version, a correction/revoke may overlap an aggregated
        // seed contribution which cannot be subtracted safely. Conservatively block this
        // consumer until a proof-backed rebuild resolves the obligation (range admission
        // belongs to C6). Never silently classify it as an ordinary dirty projection.
        let unknown_seed_old = operation != "upsert" && sqlx::query_scalar::<_,bool>(
            "SELECT EXISTS(SELECT 1 FROM cvm_analytics.analytics_projection_baselines
                WHERE consumer_id=$1 AND build_id=$2 AND jsonb_array_length(seeds)>0)
             AND NOT EXISTS(SELECT 1 FROM cvm_analytics.analytics_projection_facts
                WHERE consumer_id=$1 AND build_id=$2 AND entity_kind=$3 AND entity_key=$4 AND change_id<$5)")
            .bind(&target.consumer_id).bind(&target.build_id).bind(&kind).bind(&key).bind(change)
            .fetch_one(&mut *tx).await?;
        if !unknown_seed_old {
            sqlx::query(
            "INSERT INTO cvm_analytics.analytics_projection_facts VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(&target.consumer_id)
        .bind(&target.build_id)
        .bind(kind)
        .bind(key)
        .bind(change)
        .bind(operation == "logical_revoke")
        .bind(Json(payload))
        .execute(&mut *tx)
        .await?;
        }
        let repair_id = format!(
            "projection:{}:{}:{}",
            target.consumer_id, target.build_id, change
        );
        sqlx::query("INSERT INTO cvm_analytics.analytics_module_repairs
            (repair_id,module_id,consumer_id,build_id,owner_epoch,from_change_id,through_change_id,reason_code,state)
            SELECT $1,module_id,consumer_id,$2,owner_epoch,$3,$3,$5,$6
            FROM cvm_analytics.analytics_module_consumers WHERE consumer_id=$4")
            .bind(repair_id).bind(&target.build_id).bind(change).bind(&target.consumer_id)
            .bind(if unknown_seed_old { "unknown_seed_old_contribution" } else { "projection_dirty" })
            .bind(if unknown_seed_old { "blocked" } else { "pending" }).execute(&mut *tx).await?;
        through = change;
    }
    sqlx::query("UPDATE cvm_analytics.analytics_projection_progress SET materialized_through=$3 WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).bind(through).execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("projection materialization commit outcome may be unknown")?;
    Ok(through)
}

/// Select each entity's version at P, including tombstones before filtering. Applying
/// `deleted = false` before DISTINCT would incorrectly resurrect a revoked contribution.
async fn load_versions(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
) -> Result<Vec<ProjectionFact>> {
    let rows = sqlx::query_as::<_, (String, String, Json<Value>)>(
        "SELECT entity_kind,entity_key,payload FROM (
           SELECT DISTINCT ON (entity_kind,entity_key) entity_kind,entity_key,payload,deleted
           FROM cvm_analytics.analytics_projection_facts
           WHERE consumer_id=$1 AND build_id=$2 AND change_id<=$3
           ORDER BY entity_kind,entity_key,change_id DESC
         ) visible WHERE NOT deleted ORDER BY entity_kind,entity_key",
    )
    .bind(&token.consumer_id)
    .bind(&token.build_id)
    .bind(token.through_change_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|(kind, key, Json(payload))| model::normalize(&kind, &key, &payload))
        .collect()
}

/// Build a complete immutable staging snapshot. Failure rolls back the token and all rows;
/// last-good remains readable. A bounded fact count prevents an unbounded in-memory build.
pub(crate) async fn stage_publication(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    publication_id: &str,
) -> Result<PublicationToken> {
    ensure!(!publication_id.is_empty(), "publication id is empty");
    let mut tx = actor.begin_writer_transaction().await?;
    let (checkpoint, coverage) = lock_target(&mut tx, target).await?;
    let blocked = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM cvm_analytics.analytics_module_repairs WHERE consumer_id=$1 AND state='blocked')")
        .bind(&target.consumer_id).fetch_one(&mut *tx).await?;
    ensure!(
        !blocked,
        "projection staging has blocked repair obligations"
    );
    let through=sqlx::query_scalar::<_,i64>("SELECT materialized_through FROM cvm_analytics.analytics_projection_progress WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_one(&mut *tx).await?;
    ensure!(
        through == checkpoint,
        "projection has not materialized the complete received prefix"
    );
    let version=sqlx::query_scalar::<_,i64>("SELECT COALESCE(MAX(projection_version),0)+1 FROM cvm_analytics.analytics_module_publications WHERE consumer_id=$1 AND build_id=$2")
        .bind(&target.consumer_id).bind(&target.build_id).fetch_one(&mut *tx).await?;
    let token = PublicationToken {
        publication_id: publication_id.into(),
        consumer_id: target.consumer_id.clone(),
        build_id: target.build_id.clone(),
        owner_epoch: target.owner_epoch,
        through_change_id: through,
        coverage_revision: coverage,
        projection_version: version,
    };
    let count=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cvm_analytics.analytics_projection_facts WHERE consumer_id=$1 AND build_id=$2 AND change_id<=$3")
        .bind(&token.consumer_id).bind(&token.build_id).bind(through).fetch_one(&mut *tx).await?;
    ensure!(
        count <= 100_000,
        "projection staging exceeds bounded build budget"
    );
    let mut models = build_models(&load_versions(&mut tx, &token).await?)?;
    let baseline = sqlx::query_as::<_, (Json<Value>,Json<Value>)>("SELECT coverage,seeds FROM cvm_analytics.analytics_projection_baselines WHERE consumer_id=$1 AND build_id=$2 AND through_change_id<=$3")
        .bind(&target.consumer_id).bind(&target.build_id).bind(through).fetch_optional(&mut *tx).await?;
    if let Some((Json(coverage), Json(seeds))) = baseline {
        models.history_coverage = Some(coverage);
        models.certified_seeds = seeds;
    }
    sqlx::query("INSERT INTO cvm_analytics.analytics_module_publications
        (publication_id,module_id,consumer_id,build_id,owner_epoch,published_through_change_id,coverage_revision,projection_version,reader_version,state)
        SELECT $1,module_id,consumer_id,$2,owner_epoch,$3,$4,$5,2,'staging'
        FROM cvm_analytics.analytics_module_consumers WHERE consumer_id=$6")
        .bind(publication_id).bind(&token.build_id).bind(through).bind(coverage).bind(version).bind(&token.consumer_id).execute(&mut *tx).await?;
    // One certified model row makes an empty projection distinguishable from interrupted staging.
    sqlx::query("INSERT INTO cvm_analytics.analytics_projection_models VALUES ($1,'complete','snapshot',$2)")
        .bind(publication_id).bind(Json(serde_json::to_value(models)?)).execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("staging commit outcome may be unknown; reconcile publication id before retry")?;
    Ok(token)
}

/// Publish only a complete staged token, resolving its own dirty obligations atomically.
/// C may advance after staging; the selected P and all models still refer to the staged prefix.
pub(crate) async fn publish(actor: &mut PgIngestActor, token: &PublicationToken) -> Result<()> {
    let mut tx = actor.begin_writer_transaction().await?;
    let target = ProjectionTarget {
        consumer_id: token.consumer_id.clone(),
        build_id: token.build_id.clone(),
        owner_epoch: token.owner_epoch,
    };
    let (checkpoint, _) = lock_target(&mut tx, &target).await?;
    ensure!(
        token.through_change_id <= checkpoint,
        "publication P exceeds C"
    );
    validate_token(&mut tx, token, &["staging"]).await?;
    let blocked=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cvm_analytics.analytics_module_repairs WHERE consumer_id=$1 AND build_id=$2 AND from_change_id<=$3 AND state<>'resolved' AND (reason_code<>'projection_dirty' OR state='blocked')")
        .bind(&token.consumer_id).bind(&token.build_id).bind(token.through_change_id).fetch_one(&mut *tx).await?;
    ensure!(
        blocked == 0,
        "publication has unresolved repair obligations"
    );
    let complete=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cvm_analytics.analytics_projection_models WHERE publication_id=$1 AND model_kind='complete' AND model_key='snapshot'")
        .bind(&token.publication_id).fetch_one(&mut *tx).await?;
    ensure!(complete == 1, "publication staging is incomplete");
    let previous=sqlx::query_scalar::<_,i64>("SELECT p.published_through_change_id FROM cvm_analytics.analytics_module_consumers c JOIN cvm_analytics.analytics_module_publications p ON p.publication_id=c.active_publication_id WHERE c.consumer_id=$1")
        .bind(&token.consumer_id).fetch_optional(&mut *tx).await?;
    ensure!(
        previous.is_none_or(|p| p <= token.through_change_id),
        "publication would move P backwards"
    );
    sqlx::query("UPDATE cvm_analytics.analytics_module_publications SET state='superseded',updated_at=NOW() WHERE consumer_id=$1 AND state='published'")
        .bind(&token.consumer_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE cvm_analytics.analytics_module_publications SET state='published',published_at=NOW(),updated_at=NOW() WHERE publication_id=$1")
        .bind(&token.publication_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE cvm_analytics.analytics_module_consumers SET active_build_id=$2,active_publication_id=$3,updated_at=NOW() WHERE consumer_id=$1")
        .bind(&token.consumer_id).bind(&token.build_id).bind(&token.publication_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE cvm_analytics.analytics_module_repairs SET state='resolved',updated_at=NOW() WHERE consumer_id=$1 AND build_id=$2 AND through_change_id<=$3 AND reason_code='projection_dirty' AND state IN ('pending','running')")
        .bind(&token.consumer_id).bind(&token.build_id).bind(token.through_change_id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_ingest_work w SET state='completed',updated_at=NOW()
        FROM cvm_analytics.analytics_module_consumers c,cvm_analytics.analytics_module_builds b
        WHERE w.consumer_id=$1 AND c.consumer_id=w.consumer_id AND b.build_id=$2
          AND w.source_instance_id=c.source_instance_id AND w.generation=b.source_generation
          AND w.work_kind='projection' AND w.change_id<=$3 AND w.state IN ('pending','running')",
    )
    .bind(&token.consumer_id)
    .bind(&token.build_id)
    .bind(token.through_change_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await.context(
        "publication commit outcome may be unknown; reconcile active pointer before retry",
    )?;
    Ok(())
}

async fn validate_token(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
    states: &[&str],
) -> Result<()> {
    let state=sqlx::query_scalar::<_,String>("SELECT p.state FROM cvm_analytics.analytics_module_publications p
        JOIN cvm_analytics.analytics_module_consumers c ON c.consumer_id=p.consumer_id
        JOIN cvm_analytics.analytics_module_owners o ON o.module_id=c.module_id
        WHERE p.publication_id=$1 AND p.consumer_id=$2 AND p.build_id=$3 AND p.owner_epoch=$4
          AND p.published_through_change_id=$5 AND p.coverage_revision=$6 AND p.projection_version=$7
          AND p.reader_version<=2 AND p.legacy_reader_fenced AND c.owner_epoch=p.owner_epoch
          AND o.owner_epoch=c.owner_epoch AND o.status='active' AND o.source_instance_id=c.source_instance_id
          AND p.published_through_change_id<=c.checkpoint
          AND NOT EXISTS(SELECT 1 FROM cvm_analytics.analytics_module_repairs r
              WHERE r.consumer_id=c.consumer_id AND r.state='blocked')")
        .bind(&token.publication_id).bind(&token.consumer_id).bind(&token.build_id).bind(token.owner_epoch).bind(token.through_change_id).bind(token.coverage_revision).bind(token.projection_version).fetch_optional(&mut **tx).await?;
    ensure!(
        state.as_deref().is_some_and(|s| states.contains(&s)),
        "publication token is unavailable or incompatible"
    );
    Ok(())
}

pub(crate) async fn facts_at(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
) -> Result<Vec<ProjectionFact>> {
    validate_token(tx, token, &["published", "superseded"]).await?;
    load_versions(tx, token).await
}

pub(crate) async fn models_at(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
) -> Result<ProjectionModels> {
    validate_token(tx, token, &["published", "superseded"]).await?;
    let Json(value)=sqlx::query_scalar::<_,Json<Value>>("SELECT payload FROM cvm_analytics.analytics_projection_models WHERE publication_id=$1 AND model_kind='complete' AND model_key='snapshot'")
        .bind(&token.publication_id).fetch_one(&mut **tx).await?;
    Ok(serde_json::from_value(value)?)
}

/// Reader/build pins persist independently of a request transaction. TTL expiry only removes
/// the cleanup fence; admission must reject an expired pin before starting another read.
pub(crate) async fn retain_publication(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
    pin_id: &str,
    kind: &str,
    ttl_seconds: i64,
) -> Result<()> {
    ensure!(
        matches!(kind, "reader" | "build") && (1..=3600).contains(&ttl_seconds),
        "invalid publication pin"
    );
    validate_token(tx, token, &["published", "superseded"]).await?;
    let affected=sqlx::query("INSERT INTO cvm_analytics.analytics_projection_pins VALUES ($1,$2,$3,$4,$5,$6,NOW()+($7 * INTERVAL '1 second')) ON CONFLICT (pin_id) DO UPDATE SET expires_at=EXCLUDED.expires_at WHERE analytics_projection_pins.consumer_id=EXCLUDED.consumer_id AND analytics_projection_pins.build_id=EXCLUDED.build_id AND analytics_projection_pins.publication_id=EXCLUDED.publication_id AND analytics_projection_pins.pin_kind=EXCLUDED.pin_kind")
        .bind(pin_id).bind(&token.consumer_id).bind(&token.build_id).bind(&token.publication_id).bind(token.through_change_id).bind(kind).bind(ttl_seconds as f64).execute(&mut **tx).await?.rows_affected();
    ensure!(affected == 1, "pin id belongs to another publication");
    Ok(())
}

pub(crate) async fn release_pin(
    tx: &mut Transaction<'_, Postgres>,
    token: &PublicationToken,
    pin_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM cvm_analytics.analytics_projection_pins WHERE pin_id=$1 AND consumer_id=$2 AND publication_id=$3")
        .bind(pin_id).bind(&token.consumer_id).bind(&token.publication_id).execute(&mut **tx).await?;
    Ok(())
}

/// C6 calls this once in its repeatable-read, read-only request transaction and keeps the
/// returned token for facts, tail and interval reads. Blocked repairs fail closed globally
/// until range-specific coverage admission is supplied by the query layer.
pub(crate) async fn current_publication(
    tx: &mut Transaction<'_, Postgres>,
    consumer_id: &str,
) -> Result<PublicationToken> {
    let row=sqlx::query_as::<_,(String,String,i64,i64,i64,i64)>(
        "SELECT p.publication_id,p.build_id,p.owner_epoch,p.published_through_change_id,p.coverage_revision,p.projection_version
         FROM cvm_analytics.analytics_module_consumers c
         JOIN cvm_analytics.analytics_module_publications p ON p.publication_id=c.active_publication_id
         JOIN cvm_analytics.analytics_module_owners o ON o.module_id=c.module_id
         WHERE c.consumer_id=$1 AND c.state='active' AND p.state='published'
           AND p.consumer_id=c.consumer_id AND p.build_id=c.active_build_id
           AND p.owner_epoch=c.owner_epoch AND o.owner_epoch=c.owner_epoch AND o.status='active'
           AND o.source_instance_id=c.source_instance_id AND p.published_through_change_id<=c.checkpoint
           AND p.reader_version<=2 AND p.legacy_reader_fenced
           AND NOT EXISTS (SELECT 1 FROM cvm_analytics.analytics_module_repairs r
               WHERE r.consumer_id=c.consumer_id AND r.state='blocked')")
        .bind(consumer_id).fetch_optional(&mut **tx).await?.context("analytics publication is unavailable")?;
    Ok(PublicationToken {
        publication_id: row.0,
        consumer_id: consumer_id.into(),
        build_id: row.1,
        owner_epoch: row.2,
        through_change_id: row.3,
        coverage_revision: row.4,
        projection_version: row.5,
    })
}

/// No cleanup is performed here. Even expired pins cannot authorize source/recovery cleanup;
/// callers must separately prove recovery coverage before proposing any deletion.
pub(crate) async fn has_live_pins(
    tx: &mut Transaction<'_, Postgres>,
    consumer_id: &str,
    build_id: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM cvm_analytics.analytics_projection_pins WHERE consumer_id=$1 AND build_id=$2 AND expires_at>NOW())")
        .bind(consumer_id).bind(build_id).fetch_one(&mut **tx).await?)
}

#[cfg(test)]
#[path = "projection/lightweight.rs"]
mod lightweight;

#[cfg(test)]
#[path = "projection/postgres.rs"]
mod postgres;

/// Persist a repair which cannot safely be inferred from the retained baseline (for example
/// an unknown old contribution in a certified seed bucket). A retry never clears this fence;
/// the repair owner must prove complete reconstruction before resolving it.
pub(crate) async fn block_repair(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    repair_id: &str,
    from: i64,
    through: i64,
    reason: &str,
) -> Result<()> {
    ensure!(
        !repair_id.is_empty()
            && !reason.is_empty()
            && reason.len() <= 128
            && from >= 0
            && through >= from,
        "invalid repair obligation"
    );
    let mut tx = actor.begin_writer_transaction().await?;
    let (checkpoint, _) = lock_target(&mut tx, target).await?;
    ensure!(through <= checkpoint, "repair obligation exceeds C");
    let affected=sqlx::query("INSERT INTO cvm_analytics.analytics_module_repairs
        (repair_id,module_id,consumer_id,build_id,owner_epoch,from_change_id,through_change_id,reason_code,state)
        SELECT $1,module_id,consumer_id,$2,owner_epoch,$3,$4,$5,'blocked' FROM cvm_analytics.analytics_module_consumers WHERE consumer_id=$6
        ON CONFLICT (repair_id) DO UPDATE SET state='blocked',updated_at=NOW()
        WHERE analytics_module_repairs.consumer_id=EXCLUDED.consumer_id AND analytics_module_repairs.build_id=EXCLUDED.build_id
          AND analytics_module_repairs.owner_epoch=EXCLUDED.owner_epoch AND analytics_module_repairs.from_change_id=EXCLUDED.from_change_id
          AND analytics_module_repairs.through_change_id=EXCLUDED.through_change_id AND analytics_module_repairs.reason_code=EXCLUDED.reason_code")
        .bind(repair_id).bind(&target.build_id).bind(from).bind(through).bind(reason).bind(&target.consumer_id).execute(&mut *tx).await?.rows_affected();
    ensure!(affected == 1, "repair id has conflicting identity");
    tx.commit()
        .await
        .context("repair commit outcome may be unknown")?;
    Ok(())
}

/// A build may need a retained prefix before its first publication exists. The fenced
/// writer records that retention explicitly; source snapshot pins remain C5's responsibility.
pub(crate) async fn retain_build(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    pin_id: &str,
    through: i64,
    ttl_seconds: i64,
) -> Result<()> {
    ensure!(
        !pin_id.is_empty() && through >= 0 && (1..=3600).contains(&ttl_seconds),
        "invalid build pin"
    );
    let mut tx = actor.begin_writer_transaction().await?;
    let (checkpoint, _) = lock_target(&mut tx, target).await?;
    ensure!(through <= checkpoint, "build pin exceeds received prefix");
    let affected=sqlx::query("INSERT INTO cvm_analytics.analytics_projection_pins VALUES ($1,$2,$3,NULL,$4,'build',NOW()+($5 * INTERVAL '1 second'))
        ON CONFLICT (pin_id) DO UPDATE SET expires_at=EXCLUDED.expires_at
        WHERE analytics_projection_pins.consumer_id=EXCLUDED.consumer_id AND analytics_projection_pins.build_id=EXCLUDED.build_id
          AND analytics_projection_pins.publication_id IS NULL AND analytics_projection_pins.pin_kind='build'
          AND analytics_projection_pins.through_change_id=EXCLUDED.through_change_id")
        .bind(pin_id).bind(&target.consumer_id).bind(&target.build_id).bind(through).bind(ttl_seconds as f64).execute(&mut *tx).await?.rows_affected();
    ensure!(affected == 1, "pin id belongs to another build prefix");
    tx.commit()
        .await
        .context("build pin commit outcome may be unknown")?;
    Ok(())
}

pub(crate) async fn release_build_pin(
    actor: &mut PgIngestActor,
    target: &ProjectionTarget,
    pin_id: &str,
) -> Result<()> {
    let mut tx = actor.begin_writer_transaction().await?;
    lock_target(&mut tx, target).await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_projection_pins WHERE pin_id=$1 AND consumer_id=$2 AND build_id=$3 AND publication_id IS NULL AND pin_kind='build'")
        .bind(pin_id).bind(&target.consumer_id).bind(&target.build_id).execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("build pin release commit outcome may be unknown")?;
    Ok(())
}

#[cfg(test)]
#[path = "projection/baseline_fixture.rs"]
mod baseline_fixture;
