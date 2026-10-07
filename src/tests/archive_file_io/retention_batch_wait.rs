use super::*;
use futures_util::FutureExt;

#[tokio::test]
async fn retention_selected_batch_resumes_after_background_competition_without_repreparing_file() {
    for attempts in [false, true] {
        let (pool, mut config, temp_dir) =
            retention_test_pool_and_config("batch-slot-resume").await;
        config.retention_batch_rows = 1_000;
        let occurred_at =
            shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
        let dataset = if attempts {
            "pool_upstream_request_attempts"
        } else {
            "codex_invocations"
        };
        if attempts {
            sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,finished_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,proxy_binding_key_snapshot) SELECT i,'batch-attempt-'||i,?1,?1,'/v1/responses','pool',0,0,0,'success','batch-binding' FROM n")
                .bind(&occurred_at).execute(&pool).await.expect("attempt candidates");
        } else {
            sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) SELECT 'batch-invocation-'||i,?1,'proxy','success',7,0.07,'{}','{}' FROM n")
                .bind(&occurred_at).execute(&pool).await.expect("invocation candidates");
        }
        let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
        let target = archive_batch_file_path(&config, dataset, &month).expect("canonical target");
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(30),
        ));
        let probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
        let ready = Arc::new(Notify::new());
        let work = async {
            if attempts {
                archive_timestamped_dataset(&pool, &config, archive_table_spec(dataset),
                    "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
                    "9999-01-01 00:00:00".to_string(), false).await.map(|result| result.0)
            } else {
                archive_old_invocations(&pool, &config, None, false)
                    .await
                    .map(|result| result.0)
            }
        };
        // The deadline is only a deadlock guard here. CPU speed is not the acceptance
        // criterion; every transition is synchronized by an explicit committed chunk.
        let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator.clone(),
            crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate.clone(),
                crate::maintenance::RETENTION_TEST_WRITE_CONNECTION_POOL_READY.scope(
                    ready.clone(),
                    crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
                        probe.clone(),
                        crate::maintenance::retention_test_with_work_budget(
                            Duration::from_secs(86_400),
                            work,
                        ),
                    ),
                ),
            ),
        );
        let work = async {
            let committed = work.await?;
            anyhow::ensure!(
                committed == 1_000,
                "selected batch returned before its commit probe"
            );
            Ok::<_, anyhow::Error>(committed)
        };
        let competitor = async {
            probe.committed.notified().await;
            let remaining: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {dataset}"))
                .fetch_one(&pool)
                .await
                .expect("committed source boundary");
            assert_eq!(remaining, 936);
            let digest = sha256_hex_file(&target).expect("published batch file");
            #[cfg(unix)]
            let inode = Some({
                use std::os::unix::fs::MetadataExt;
                fs::metadata(&target).expect("target metadata").ino()
            });
            #[cfg(not(unix))]
            let inode: Option<u64> = None;
            let owner = gate
                .try_begin_background("competing_owner")
                .expect("competitor slot");
            let fence = crate::maintenance::retention_archive_file_try_lock(&target)
                .expect("batch released file fence before waiting");
            drop(fence);
            while ready.notified().now_or_never().is_some() {}
            probe.resume.notify_one();
            ready.notified().await;
            assert_eq!(coordinator.snapshot().await.active_write_class, None);
            let p1 = coordinator
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
                .await;
            let mut foreground = pool.acquire().await.expect("pool available to foreground");
            sqlx::query("CREATE TABLE foreground_batch_wait_probe(id INTEGER)")
                .execute(&mut *foreground)
                .await
                .expect("foreground write during batch wait");
            foreground.return_to_pool().await;
            drop(foreground);
            drop(owner);
            drop(p1);
            Ok::<_, anyhow::Error>((digest, inode))
        };
        let (committed, identity) =
            tokio::try_join!(work, competitor).expect("same task resumes its selected batch");
        assert_eq!(committed, 1_000);
        assert_eq!(sha256_hex_file(&target).expect("final digest"), identity.0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&target).expect("final inode").ino(),
                identity.1.expect("Unix inode")
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {dataset}"))
                .fetch_one(&pool)
                .await
                .expect("source closed"),
            0
        );
        assert_eq!(coordinator.snapshot().await.active_write_class, None);
        pool.close().await;
        cleanup_temp_test_dir(&temp_dir);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn retention_selected_batch_defers_archive_fence_collision_without_deleting_remaining_source()
{
    for attempts in [false, true] {
        let (pool, mut config, temp_dir) =
            retention_test_pool_and_config("batch-fence-collision").await;
        config.retention_batch_rows = 1_000;
        let occurred_at =
            shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
        let dataset = if attempts {
            "pool_upstream_request_attempts"
        } else {
            "codex_invocations"
        };
        if attempts {
            sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,finished_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,proxy_binding_key_snapshot) SELECT i,'fence-attempt-'||i,?1,?1,'/v1/responses','pool',0,0,0,'success','fence-binding' FROM n")
                .bind(&occurred_at).execute(&pool).await.expect("attempt candidates");
        } else {
            sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) SELECT 'fence-invocation-'||i,?1,'proxy','success',7,0.07,'{}','{}' FROM n")
                .bind(&occurred_at).execute(&pool).await.expect("invocation candidates");
        }
        let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
        let target = archive_batch_file_path(&config, dataset, &month).expect("canonical target");
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(30),
        ));
        let probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
        let ready = Arc::new(Notify::new());
        let stopped = Arc::new(Notify::new());
        let work = async {
            if attempts {
                archive_timestamped_dataset(&pool, &config, archive_table_spec(dataset),
                    "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
                    "9999-01-01 00:00:00".to_string(), false).await.map(|result| result.0)
            } else {
                archive_old_invocations(&pool, &config, None, false)
                    .await
                    .map(|result| result.0)
            }
        };
        // This guard bounds a deadlock, not the machine's throughput. The collision
        // is created only after the first committed chunk, using explicit probes.
        let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator.clone(),
            crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate.clone(),
                crate::maintenance::RETENTION_TEST_WRITE_CONNECTION_POOL_READY.scope(
                    ready.clone(),
                    crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
                        probe.clone(),
                        crate::maintenance::retention_test_with_work_budget(
                            Duration::from_secs(86_400),
                            work,
                        ),
                    ),
                ),
            ),
        );
        let work = async {
            let result = work.await;
            stopped.notify_one();
            let error = match result {
                Err(error) => error,
                Ok(committed) => anyhow::bail!("fence collision committed {committed} rows"),
            };
            anyhow::ensure!(
                crate::maintenance::is_retention_write_deferred(&error),
                "fence collision became fatal: {error:#}"
            );
            let remaining: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {dataset}"))
                .fetch_one(&pool)
                .await?;
            anyhow::ensure!(remaining == 936, "unexpected source boundary: {remaining}");
            Ok::<_, anyhow::Error>(())
        };
        let competitor = async {
            probe.committed.notified().await;
            let digest = sha256_hex_file(&target).expect("published batch file");
            let owner = gate
                .try_begin_background("competing_fence_owner")
                .expect("competitor slot");
            let fence = crate::maintenance::retention_archive_file_try_lock(&target)
                .expect("batch released file fence before waiting");
            while ready.notified().now_or_never().is_some() {}
            probe.resume.notify_one();
            ready.notified().await;
            assert_eq!(coordinator.snapshot().await.active_write_class, None);
            drop(owner);
            // Retain the fence until the selected batch observes its collision.
            stopped.notified().await;
            drop(fence);
            Ok::<_, anyhow::Error>(digest)
        };
        let (_, digest) = tokio::try_join!(work, competitor).expect("retryable archive fence stop");
        assert_eq!(sha256_hex_file(&target).expect("final digest"), digest);
        assert_eq!(coordinator.snapshot().await.active_write_class, None);
        let available = pool
            .acquire()
            .await
            .expect("deferred batch released connection");
        drop(available);
        pool.close().await;
        cleanup_temp_test_dir(&temp_dir);
    }
}
