use super::super::{
    AnalyticsModuleId,
    ingest::{SourceBinding, apply_analytics_ingest_migration},
    source::{SourceEnvelope, SourceIdentity, SourceOperation},
};
use super::baseline::{import_certified_baseline, register_baseline_source};
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
    // A released v1 installation keeps its checksum/identity when upgrading to v2.
    sqlx::raw_sql(ANALYTICS_PROJECTION_SCHEMA_SQL)
        .execute(&mut owner)
        .await?;
    sqlx::query("INSERT INTO cvm_analytics.analytics_module_ddl_ledger
        (component,schema_version,step_order,step_id,checksum,state,min_reader_version,requires_bootstrap,applied_at)
        VALUES ($1,1,1,$2,$3,'applied',1,TRUE,NOW())")
        .bind(ANALYTICS_PROJECTION_DDL_COMPONENT).bind(ANALYTICS_PROJECTION_DDL_STEP)
        .bind(format!("{:x}",Sha256::digest(ANALYTICS_PROJECTION_SCHEMA_SQL.as_bytes())))
        .execute(&mut owner).await?;
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
        2
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
    certified_baseline_matrix(&mut owner, &pool, &mut actor).await?;
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

async fn certified_baseline_matrix(
    owner: &mut PgConnection,
    pool: &PgPool,
    actor: &mut PgIngestActor,
) -> Result<()> {
    let (snapshot, artifacts, candidate) = baseline_fixture::fixture()?;
    let source = baseline_fixture::sqlite_source(&snapshot).await?;
    let target = ProjectionTarget {
        consumer_id: "invocation".into(),
        build_id: snapshot.build_id.clone(),
        owner_epoch: 1,
    };
    sqlx::query("INSERT INTO cvm_analytics.analytics_module_builds
        (build_id,module_id,consumer_id,source_instance_id,source_generation,source_high_watermark,source_coverage_revision,owner_epoch,state,projection_version,retention_pin_id)
        VALUES ($1,'invocation','invocation','source-c4',1,2,0,1,'ready',1,$2)")
        .bind(&snapshot.build_id).bind(&snapshot.retention_pin_id).execute(&mut *owner).await?;
    // Fixture bootstrap has C=4 with only retained tail receipts 3..4; the complete
    // historical prefix must come from proof, never from the numerical C checkpoint.
    sqlx::query("DELETE FROM cvm_analytics.analytics_module_ingest_receipts WHERE consumer_id='invocation' AND change_id<=2")
        .execute(&mut *owner).await?;
    assert!(materialize_batch(actor, &target, 64).await.is_err());
    // Missing receipt cannot skip a retained prefix, even though C is already ahead.
    assert!(
        import_certified_baseline(actor, &target, &candidate)
            .await
            .is_err()
    );
    let mut bad = artifacts.clone();
    bad[0].bytes.push(b' ');
    assert!(
        register_baseline_source(actor, &target, &source, &snapshot, "history", &bad)
            .await
            .is_err()
    );
    let mut bad_snapshot = snapshot.clone();
    bad_snapshot.source.stats_semantics_version += 1;
    assert!(
        register_baseline_source(
            actor,
            &target,
            &source,
            &bad_snapshot,
            "history",
            &artifacts
        )
        .await
        .is_err()
    );
    register_baseline_source(actor, &target, &source, &snapshot, "history", &artifacts).await?;
    register_baseline_source(actor, &target, &source, &snapshot, "history", &artifacts).await?;
    let (_, _, forged) = baseline_fixture::fixture_variant(true, true)?;
    assert!(
        import_certified_baseline(actor, &target, &forged)
            .await
            .is_err()
    );
    let (_, _, missing_seed) = baseline_fixture::fixture_variant(false, false)?;
    assert!(
        import_certified_baseline(actor, &target, &missing_seed)
            .await
            .is_err()
    );
    // Inject failure after all fact inserts, before baseline receipt/progress activation.
    sqlx::raw_sql("CREATE FUNCTION cvm_analytics.c4_fail_baseline() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected baseline activation failure'; END $$;
        CREATE TRIGGER c4_fail_baseline BEFORE INSERT ON cvm_analytics.analytics_projection_baselines FOR EACH ROW EXECUTE FUNCTION cvm_analytics.c4_fail_baseline()")
        .execute(&mut *owner).await?;
    assert!(
        import_certified_baseline(actor, &target, &candidate)
            .await
            .is_err()
    );
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cvm_analytics.analytics_projection_facts WHERE build_id=$1",
    )
    .bind(&target.build_id)
    .fetch_one(&mut *owner)
    .await?;
    assert_eq!(count, 0);
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cvm_analytics.analytics_projection_progress WHERE build_id=$1",
    )
    .bind(&target.build_id)
    .fetch_one(&mut *owner)
    .await?;
    assert_eq!(count, 0);
    sqlx::raw_sql("DROP TRIGGER c4_fail_baseline ON cvm_analytics.analytics_projection_baselines; DROP FUNCTION cvm_analytics.c4_fail_baseline()")
        .execute(&mut *owner).await?;
    assert_eq!(
        import_certified_baseline(actor, &target, &candidate).await?,
        2
    );
    assert_eq!(
        import_certified_baseline(actor, &target, &candidate).await?,
        2
    );
    // C=4 requires replay 3..4; certification is not a forged C checkpoint update.
    assert!(
        stage_publication(actor, &target, "baseline-incomplete")
            .await
            .is_err()
    );
    assert_eq!(materialize_batch(actor, &target, 64).await?, 4);
    let token = stage_publication(actor, &target, "baseline-publication").await?;
    let mut reader = pool.begin().await?;
    assert!(facts_at(&mut reader, &token).await.is_err());
    reader.rollback().await?;
    publish(actor, &token).await?;
    let mut reader = pool.begin().await?;
    let facts = facts_at(&mut reader, &token).await?;
    assert_eq!(facts.len(), 3);
    assert!(facts.iter().any(|f| f.entity_key == "row-1"));
    let models = models_at(&mut reader, &token).await?;
    assert_eq!(models.certified_seeds[0]["rows"][0]["aggregateValue"], 7.0);
    assert!(
        models
            .history_coverage
            .as_ref()
            .unwrap()
            .as_array()
            .unwrap()
            .len()
            >= 2
    );
    // Seed remains separate from exact facts and rollup calls.
    assert_eq!(models.certified_seeds.as_array().unwrap().len(), 1);
    reader.commit().await?;
    let mut correction = event(5, SourceOperation::Correction, 99);
    correction.identity = SourceIdentity::new("source-c4", "invocation", "row-1")?;
    actor.receive_batch(&[correction]).await?;
    materialize_batch(actor, &target, 64).await?;
    let mut reader = pool.begin().await?;
    assert_eq!(
        facts_at(&mut reader, &token)
            .await?
            .iter()
            .find(|f| f.entity_key == "row-1")
            .unwrap()
            .payload["totalTokens"],
        2
    );
    reader.commit().await?;
    let revised = stage_publication(actor, &target, "baseline-correction").await?;
    publish(actor, &revised).await?;
    let mut revoke = event(6, SourceOperation::LogicalRevoke, 0);
    revoke.identity = SourceIdentity::new("source-c4", "invocation", "row-1")?;
    actor.receive_batch(&[revoke]).await?;
    materialize_batch(actor, &target, 64).await?;
    let mut reader = pool.begin().await?;
    assert_eq!(
        facts_at(&mut reader, &revised)
            .await?
            .iter()
            .find(|f| f.entity_key == "row-1")
            .unwrap()
            .payload["totalTokens"],
        99
    );
    reader.commit().await?;
    let deleted = stage_publication(actor, &target, "baseline-deleted").await?;
    publish(actor, &deleted).await?;
    let mut reader = pool.begin().await?;
    assert!(
        !facts_at(&mut reader, &deleted)
            .await?
            .iter()
            .any(|f| f.entity_key == "row-1")
    );
    assert_eq!(
        models_at(&mut reader, &deleted).await?.certified_seeds,
        models.certified_seeds
    );
    reader.commit().await?;
    // Actual consumer entry generates the blocked obligation atomically for an unknown
    // old seed contribution; neither correction nor revoke may publish a guessed result.
    for (change, operation) in [
        (7, SourceOperation::Correction),
        (8, SourceOperation::LogicalRevoke),
    ] {
        let mut unknown = event(change, operation, 12);
        unknown.identity = SourceIdentity::new("source-c4", "invocation", "unknown-seed-entity")?;
        unknown.payload["occurredAt"] = json!("2025-12-01T12:00:00Z");
        unknown.occurred_at = "2025-12-01T12:00:00Z".into();
        unknown.payload_digest = super::super::digest_json(&unknown.payload)?;
        actor.receive_batch(&[unknown]).await?;
        assert_eq!(materialize_batch(actor, &target, 64).await?, change);
        assert!(
            stage_publication(actor, &target, &format!("blocked-seed-{change}"))
                .await
                .is_err()
        );
    }
    let blocked=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cvm_analytics.analytics_module_repairs WHERE build_id=$1 AND reason_code='unknown_seed_old_contribution' AND state='blocked'")
        .bind(&target.build_id).fetch_one(&mut *owner).await?;
    assert_eq!(blocked, 2);
    let pointer=sqlx::query_scalar::<_,String>("SELECT active_publication_id FROM cvm_analytics.analytics_module_consumers WHERE consumer_id='invocation'")
        .fetch_one(&mut *owner).await?;
    assert_eq!(pointer, deleted.publication_id);
    let mut reader = pool.begin().await?;
    assert!(
        current_publication(&mut reader, "invocation")
            .await
            .is_err()
    );
    reader.rollback().await?;
    source.close().await;
    Ok(())
}
