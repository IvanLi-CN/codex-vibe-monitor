use crate::storage::archive::*;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fixture_identity() -> (String, i64) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    let suffix = format!("{}-{nanos}", std::process::id());
    let batch_id = i64::try_from(nanos).expect("test timestamp must fit PostgreSQL BIGINT")
        % 1_000_000_000_000_000;
    (suffix, batch_id)
}

fn manifest(batch_id: i64, suffix: &str, cleanup_state: &str) -> ArchiveManifest {
    let file_path = format!("pg-archive-tests/{suffix}/batch-{batch_id}.sqlite.gz");
    ArchiveManifest {
        archive_batch_id: batch_id,
        dataset: "pool_upstream_request_attempts".to_owned(),
        month_key: "2026-10".to_owned(),
        day_key: "2026-10-26".to_owned(),
        part: 0,
        sha256: digest(&format!("archive:{file_path}")),
        source_identity_sha256: digest(&format!("source:{file_path}")),
        row_count: 0,
        layout: "segment_v1".to_owned(),
        codec: "none".to_owned(),
        cleanup_state: cleanup_state.to_owned(),
        summary_source_kind: "authoritative".to_owned(),
        coverage_start_at: Some("2026-10-26T00:00:00Z".to_owned()),
        coverage_end_at: Some("2026-10-26T01:00:00Z".to_owned()),
        historical_rollups_materialized_at: Some("2026-10-26T01:01:00Z".to_owned()),
        file_path,
    }
}

fn rollup(manifest: &ArchiveManifest) -> ArchiveRollupMarker {
    ArchiveRollupMarker {
        archive_batch_id: manifest.archive_batch_id,
        target: "pg_test_rollup".to_owned(),
        bucket_start_epoch_ms: 0,
        source_identity_sha256: manifest.source_identity_sha256.clone(),
        materialized_rows: 0,
        complete: true,
    }
}

fn operation(
    manifest: ArchiveManifest,
    raw_reference_changes: Vec<RawReferenceChange>,
) -> ArchivePublishOperation {
    let replay_marker = ArchiveReplayMarker {
        target: "pg_test_replay".to_owned(),
        dataset: manifest.dataset.clone(),
        file_path: manifest.file_path.clone(),
        archive_sha256: manifest.sha256.clone(),
    };
    ArchivePublishOperation {
        proof: Some(ArchiveProof {
            archive_batch_id: manifest.archive_batch_id,
            manifest_sha256: manifest.sha256.clone(),
            page_count: 0,
            row_count: manifest.row_count,
            coverage_start_at: manifest.coverage_start_at.clone(),
            coverage_end_at: manifest.coverage_end_at.clone(),
            semantic_sha256: digest(&format!("semantic:{}", manifest.file_path)),
            verified_at_epoch_ms: 1,
        }),
        rollups: vec![rollup(&manifest)],
        replay_markers: vec![replay_marker],
        node_health_rows: Vec::new(),
        node_health_hourly_rows: Vec::new(),
        raw_reference_changes,
        historical_activity: Vec::new(),
        retention_prepared_archive: None,
        retention_recovery_cursor: None,
        retention_raw_reconciliation: Vec::new(),
        manifest,
    }
}

fn raw_change(
    manifest: &ArchiveManifest,
    owner_id: i64,
    action: RawReferenceAction,
    raw_path: &str,
    identity: &str,
) -> RawReferenceChange {
    RawReferenceChange {
        archive_batch_id: manifest.archive_batch_id,
        raw_path: raw_path.to_owned(),
        owner_kind: "invocation".to_owned(),
        owner_id,
        raw_role: "request".to_owned(),
        action,
        raw_identity_sha256: Some(identity.to_owned()),
    }
}

async fn adapter() -> Result<Option<(PgPool, ArchiveStorageAdapter)>> {
    let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
        return Ok(None);
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .context("connect PostgreSQL archive test database")?;
    let archive = ArchiveStorageAdapter::new(
        pool.clone(),
        PathBuf::from("/var/lib/codex/archive"),
        PathBuf::from("/var/lib/codex/raw"),
    );
    archive.ensure_schema().await?;
    Ok(Some((pool, archive)))
}

async fn cleanup(
    pool: &PgPool,
    batch_ids: &[i64],
    file_paths: &[String],
    raw_path: &str,
) -> Result<()> {
    for batch_id in batch_ids {
        for table in [
            "archive_account_activity_outbox",
            "archive_raw_reference_changes",
            "archive_batch_upstream_activity",
            "pool_upstream_node_health_hourly_archive",
            "archive_rollup_markers",
            "archive_proofs",
            "archive_prune_ledger",
        ] {
            let query = format!("DELETE FROM {table} WHERE archive_batch_id = $1");
            sqlx::query(&query)
                .bind(batch_id)
                .execute(pool)
                .await
                .with_context(|| format!("clean {table} test rows"))?;
        }
        sqlx::query("DELETE FROM archive_batches WHERE archive_batch_id = $1")
            .bind(batch_id)
            .execute(pool)
            .await
            .context("clean archive batch test row")?;
    }
    for file_path in file_paths {
        sqlx::query(
            "DELETE FROM pool_upstream_node_health_archive
              WHERE archive_file_path = $1",
        )
        .bind(file_path)
        .execute(pool)
        .await
        .context("clean node-health archive test rows")?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay
              WHERE dataset = 'pool_upstream_request_attempts' AND file_path = $1",
        )
        .bind(file_path)
        .execute(pool)
        .await
        .context("clean archive replay test row")?;
    }
    sqlx::query("DELETE FROM proxy_raw_payload_blob_links WHERE raw_path = $1")
        .bind(raw_path)
        .execute(pool)
        .await
        .context("clean live Raw links")?;
    sqlx::query("DELETE FROM proxy_raw_payload_blobs WHERE raw_path = $1")
        .bind(raw_path)
        .execute(pool)
        .await
        .context("clean Raw blob parent")?;
    Ok(())
}

#[tokio::test]
async fn postgres_archive_ownership_is_cross_batch_and_release_retry_monotonic() -> Result<()> {
    let Some((pool, archive)) = adapter().await? else {
        return Ok(());
    };
    let (suffix, base_batch_id) = fixture_identity();
    let raw_path = format!("pg-raw-tests/{suffix}/shared.bin");
    let raw_identity = digest(&format!("raw:{raw_path}"));
    let manifests = (0..6)
        .map(|offset| manifest(base_batch_id + offset, &suffix, "delete_pending"))
        .collect::<Vec<_>>();
    let file_paths = manifests
        .iter()
        .map(|item| item.file_path.clone())
        .collect::<Vec<_>>();
    let batch_ids = manifests
        .iter()
        .map(|item| item.archive_batch_id)
        .collect::<Vec<_>>();

    let result: Result<()> = async {
        archive
            .publish(&operation(
                manifests[0].clone(),
                vec![raw_change(
                    &manifests[0],
                    1,
                    RawReferenceAction::Link,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        archive
            .publish(&operation(
                manifests[1].clone(),
                vec![raw_change(
                    &manifests[1],
                    2,
                    RawReferenceAction::Link,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        archive
            .publish(&operation(
                manifests[2].clone(),
                vec![raw_change(
                    &manifests[2],
                    1,
                    RawReferenceAction::Release,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        let active_owner_two: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM archive_raw_reference_changes
              WHERE raw_path = $1 AND owner_id = 2 AND action = 'link' AND state = 'active'",
        )
        .bind(&raw_path)
        .fetch_one(&pool)
        .await?;
        assert_eq!(active_owner_two, 1);

        archive
            .publish(&operation(
                manifests[3].clone(),
                vec![raw_change(
                    &manifests[3],
                    3,
                    RawReferenceAction::Link,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        archive
            .publish(&operation(
                manifests[3].clone(),
                vec![raw_change(
                    &manifests[3],
                    3,
                    RawReferenceAction::Release,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        archive
            .publish(&operation(
                manifests[3].clone(),
                vec![raw_change(
                    &manifests[3],
                    3,
                    RawReferenceAction::Link,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        let owner_three_state: String = sqlx::query_scalar(
            "SELECT state FROM archive_raw_reference_changes
              WHERE archive_batch_id = $1 AND owner_id = 3",
        )
        .bind(manifests[3].archive_batch_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(owner_three_state, "released");

        archive
            .publish(&operation(
                manifests[4].clone(),
                vec![raw_change(
                    &manifests[4],
                    2,
                    RawReferenceAction::Release,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;
        let parent_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blobs WHERE raw_path = $1")
                .bind(&raw_path)
                .fetch_one(&pool)
                .await?;
        assert_eq!(parent_count, 0);

        let missing_parent = archive
            .publish(&operation(
                manifests[5].clone(),
                vec![raw_change(
                    &manifests[5],
                    4,
                    RawReferenceAction::Release,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await;
        assert!(missing_parent.is_err());
        Ok(())
    }
    .await;
    cleanup(&pool, &batch_ids, &file_paths, &raw_path).await?;
    result
}

#[tokio::test]
async fn postgres_archive_release_and_prune_wait_for_live_raw_link_transactions() -> Result<()> {
    let Some((pool, archive)) = adapter().await? else {
        return Ok(());
    };
    let (suffix, base_batch_id) = fixture_identity();
    let raw_path = format!("pg-raw-tests/{suffix}/concurrent.bin");
    let raw_identity = digest(&format!("raw:{raw_path}"));
    let mut archive_manifest = manifest(base_batch_id, &suffix, "delete_pending");
    let canonical_archive_path = archive_manifest.file_path.clone();
    archive_manifest.file_path = format!("archive/{canonical_archive_path}");
    let cleanup_manifest = manifest(base_batch_id + 1, &suffix, "delete_pending");
    let file_paths = vec![
        canonical_archive_path.clone(),
        cleanup_manifest.file_path.clone(),
    ];
    let batch_ids = vec![
        archive_manifest.archive_batch_id,
        cleanup_manifest.archive_batch_id,
    ];

    let result: Result<()> = async {
        archive
            .publish(&operation(
                archive_manifest.clone(),
                vec![raw_change(
                    &archive_manifest,
                    10,
                    RawReferenceAction::Link,
                    &raw_path,
                    &raw_identity,
                )],
            ))
            .await?;

        let mut live_transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO proxy_raw_payload_blob_links
                (raw_path, owner_kind, owner_id, raw_role)
             VALUES ($1, 'attempt', 9001, 'request')",
        )
        .bind(&raw_path)
        .execute(&mut *live_transaction)
        .await?;
        let release_adapter = archive.clone();
        let release_operation = operation(
            archive_manifest.clone(),
            vec![raw_change(
                &archive_manifest,
                10,
                RawReferenceAction::Release,
                &raw_path,
                &raw_identity,
            )],
        );
        let mut release_task =
            tokio::spawn(async move { release_adapter.publish(&release_operation).await });
        if tokio::time::timeout(Duration::from_millis(200), &mut release_task)
            .await
            .is_ok()
        {
            bail!("archive release did not wait for the live-link transaction");
        }
        live_transaction.commit().await?;
        release_task.await??;

        let parent_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blobs WHERE raw_path = $1")
                .bind(&raw_path)
                .fetch_one(&pool)
                .await?;
        let live_link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path = $1",
        )
        .bind(&raw_path)
        .fetch_one(&pool)
        .await?;
        assert_eq!(parent_count, 1);
        assert_eq!(live_link_count, 1);

        sqlx::query("DELETE FROM proxy_raw_payload_blob_links WHERE raw_path = $1")
            .bind(&raw_path)
            .execute(&pool)
            .await?;
        let plan = ArchivePrunePlan {
            manifest: archive_manifest.clone(),
            proof: operation(archive_manifest.clone(), Vec::new()).proof,
            rollups: vec![rollup(&archive_manifest)],
            raw_reference_checks: vec![RawReferenceCheck {
                raw_path: raw_path.clone(),
                active_reference_count: 0,
                file_identity_sha256: raw_identity.clone(),
                checked_at_epoch_ms: 1,
            }],
            pending_activity_outbox: 0,
            file_identity_sha256: archive_manifest.sha256.clone(),
        };
        let mut second_live_transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO proxy_raw_payload_blob_links
                (raw_path, owner_kind, owner_id, raw_role)
             VALUES ($1, 'attempt', 9002, 'request')",
        )
        .bind(&raw_path)
        .execute(&mut *second_live_transaction)
        .await?;
        let prune_adapter = archive.clone();
        let prune_plan = plan.clone();
        let mut prune_task =
            tokio::spawn(async move { prune_adapter.prune_metadata(&prune_plan).await });
        if tokio::time::timeout(Duration::from_millis(200), &mut prune_task)
            .await
            .is_ok()
        {
            bail!("archive prune did not wait for the live-link transaction");
        }
        second_live_transaction.commit().await?;
        assert!(!prune_task.await??);

        let batch_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches WHERE archive_batch_id = $1")
                .bind(archive_manifest.archive_batch_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(batch_count, 1);
        sqlx::query("DELETE FROM proxy_raw_payload_blob_links WHERE raw_path = $1")
            .bind(&raw_path)
            .execute(&pool)
            .await?;
        assert!(archive.prune_metadata(&plan).await?);
        let ledger_file_path: String = sqlx::query_scalar(
            "SELECT file_path FROM archive_prune_ledger WHERE archive_batch_id = $1",
        )
        .bind(archive_manifest.archive_batch_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(ledger_file_path, canonical_archive_path);
        Ok(())
    }
    .await;
    cleanup(&pool, &batch_ids, &file_paths, &raw_path).await?;
    result
}
