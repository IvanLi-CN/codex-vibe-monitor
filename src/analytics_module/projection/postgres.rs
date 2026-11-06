use super::super::{
    AnalyticsModuleId,
    ingest::{SourceBinding, apply_analytics_ingest_migration},
    source::{SourceEnvelope, SourceIdentity, SourceOperation},
};
use super::*;
use serde_json::json;
use sqlx::PgPool;

fn event(change: i64, operation: SourceOperation, tokens: i64) -> SourceEnvelope {
    let payload = json!({"factVersion":1,"occurredAt":"2026-11-01T23:59:59Z","successLike":true,"tTotalMs":3000,"totalTokens":tokens,"model":"fixture"});
    SourceEnvelope {
        envelope_version: 2,
        source_instance_id: "source-c4".into(),
        event_id: format!("event-{change}"),
        generation: 1,
        change_id: change,
        coverage_revision: 0,
        fact_semantics_version: 1,
        stats_semantics_version: 2,
        identity: SourceIdentity::new("source-c4", "invocation", "invocation:fixture:1").unwrap(),
        operation,
        occurred_at: "2026-11-01T23:59:59Z".into(),
        payload_digest: super::super::digest_json(&payload).unwrap(),
        payload,
    }
}

#[tokio::test]
#[ignore = "requires explicitly authorized isolated *_fixture PostgreSQL database"]
async fn postgres_projection_versions_publication_atomicity_pins_and_forward_reentry() -> Result<()>
{
    ensure!(
        std::env::var("PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET").as_deref() == Ok("1"),
        "schema reset is not authorized"
    );
    let owner_url = std::env::var("PG_ANALYTICS_TEST_OWNER_URL")?;
    let maintenance_url = std::env::var("PG_ANALYTICS_TEST_MAINTENANCE_URL")?;
    let parsed = url::Url::parse(&owner_url)?;
    ensure!(
        parsed.path().ends_with("_fixture"),
        "expected disposable fixture database"
    );
    let maintenance_parsed = url::Url::parse(&maintenance_url)?;
    ensure!(
        parsed.host_str() == maintenance_parsed.host_str()
            && parsed.port_or_known_default() == maintenance_parsed.port_or_known_default()
            && parsed.path() == maintenance_parsed.path(),
        "fixture URLs must address the same database"
    );
    let mut owner = PgConnection::connect(&owner_url).await?;
    sqlx::raw_sql("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
        .execute(&mut owner)
        .await?;
    sqlx::raw_sql(super::super::super::PG_ANALYTICS_SCHEMA_SQL)
        .execute(&mut owner)
        .await?;
    super::super::apply_analytics_module_schema(&mut owner)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    super::super::apply_analytics_module_roles(&mut owner)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    apply_analytics_ingest_migration(&mut owner).await?;
    assert_eq!(
        apply_analytics_projection_migration(&mut owner)
            .await?
            .recovered_steps,
        1
    );
    assert_eq!(
        apply_analytics_projection_migration(&mut owner)
            .await?
            .recovered_steps,
        0
    );
    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_ddl_ledger SET state='failed' WHERE component=$1",
    )
    .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
    .execute(&mut owner)
    .await?;
    assert_eq!(
        apply_analytics_projection_migration(&mut owner)
            .await?
            .recovered_steps,
        1
    );
    sqlx::raw_sql("INSERT INTO cvm_analytics.analytics_module_owners (module_id,owner_id,source_instance_id,owner_epoch,status) VALUES ('invocation','fixture','source-c4',1,'active');
        INSERT INTO cvm_analytics.analytics_module_consumers (consumer_id,module_id,consumer_version,state,source_instance_id,owner_epoch) VALUES ('invocation','invocation',1,'active','source-c4',1);
        INSERT INTO cvm_analytics.analytics_module_builds (build_id,module_id,consumer_id,source_instance_id,source_generation,source_high_watermark,source_coverage_revision,owner_epoch,state,projection_version) VALUES ('build-c4','invocation','invocation','source-c4',1,0,0,1,'ready',1)")
        .execute(&mut owner).await?;
    let pool = PgPool::connect(&maintenance_url).await?;
    let binding = SourceBinding::new(
        AnalyticsModuleId::Invocation,
        "invocation",
        "source-c4",
        1,
        1,
        0,
        0,
    )?;
    let mut actor = PgIngestActor::acquire(&pool, binding).await?;
    let target = ProjectionTarget {
        consumer_id: "invocation".into(),
        build_id: "build-c4".into(),
        owner_epoch: 1,
    };
    actor
        .receive_batch(&[event(1, SourceOperation::Upsert, 10)])
        .await?;
    assert_eq!(materialize_batch(&mut actor, &target, 64).await?, 1);
    let p1 = stage_publication(&mut actor, &target, "publication-1").await?;
    publish(&mut actor, &p1).await?;
    actor
        .receive_batch(&[event(2, SourceOperation::Correction, 3)])
        .await?;
    materialize_batch(&mut actor, &target, 64).await?;
    let mut reader = pool.begin().await?;
    assert_eq!(current_publication(&mut reader, "invocation").await?, p1);
    assert_eq!(
        facts_at(&mut reader, &p1).await?[0].payload["totalTokens"],
        10
    );
    retain_publication(&mut reader, &p1, "pin-old", "reader", 60).await?;
    assert!(has_live_pins(&mut reader, "invocation", "build-c4").await?);
    reader.commit().await?;
    let p2 = stage_publication(&mut actor, &target, "publication-2").await?;
    let mut reader = pool.begin().await?;
    assert!(facts_at(&mut reader, &p2).await.is_err());
    reader.rollback().await?;
    publish(&mut actor, &p2).await?;
    actor
        .receive_batch(&[event(3, SourceOperation::LogicalRevoke, 0)])
        .await?;
    materialize_batch(&mut actor, &target, 64).await?;
    let mut reader = pool.begin().await?;
    assert_eq!(
        facts_at(&mut reader, &p1).await?[0].payload["totalTokens"],
        10
    );
    assert_eq!(
        facts_at(&mut reader, &p2).await?[0].payload["totalTokens"],
        3
    );
    assert!(!models_at(&mut reader, &p2).await?.intervals.is_empty());
    reader.commit().await?;
    let p3 = stage_publication(&mut actor, &target, "publication-3").await?;
    publish(&mut actor, &p3).await?;
    let mut reader = pool.begin().await?;
    assert!(facts_at(&mut reader, &p3).await?.is_empty());
    assert!(models_at(&mut reader, &p3).await?.intervals.is_empty());
    assert_eq!(facts_at(&mut reader, &p1).await?.len(), 1);
    release_pin(&mut reader, &p1, "pin-old").await?;
    assert!(!has_live_pins(&mut reader, "invocation", "build-c4").await?);
    reader.commit().await?;
    // A publication failure rolls back its pointer change and keeps last-good readable.
    actor
        .receive_batch(&[event(4, SourceOperation::Upsert, 8)])
        .await?;
    materialize_batch(&mut actor, &target, 64).await?;
    let p4 = stage_publication(&mut actor, &target, "publication-4").await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_projection_models WHERE publication_id='publication-4'").execute(&mut owner).await?;
    assert!(publish(&mut actor, &p4).await.is_err());
    let mut reader = pool.begin().await?;
    assert_eq!(current_publication(&mut reader, "invocation").await?, p3);
    reader.commit().await?;
    block_repair(
        &mut actor,
        &target,
        "seed-missing",
        4,
        4,
        "unknown_seed_old_contribution",
    )
    .await?;
    let mut reader = pool.begin().await?;
    assert!(
        current_publication(&mut reader, "invocation")
            .await
            .is_err()
    );
    reader.rollback().await?;
    retain_build(&mut actor, &target, "build-pin", 4, 60).await?;
    sqlx::query("UPDATE cvm_analytics.analytics_module_owners SET owner_epoch=2 WHERE module_id='invocation'").execute(&mut owner).await?;
    assert!(materialize_batch(&mut actor, &target, 64).await.is_err());
    let mut reader = pool.begin().await?;
    assert!(facts_at(&mut reader, &p1).await.is_err());
    reader.rollback().await?;
    actor.close().await?;
    pool.close().await;
    Ok(())
}
