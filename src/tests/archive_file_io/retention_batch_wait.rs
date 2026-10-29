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
                    "9999-01-01 00:00:00".to_string(), false, None).await.map(|result| result.0)
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
            let target = PathBuf::from(
                sqlx::query_scalar::<_, String>(
                    "SELECT file_path FROM archive_batches
                     WHERE dataset=?1 ORDER BY id DESC LIMIT 1",
                )
                .bind(dataset)
                .fetch_one(&pool)
                .await
                .expect("published batch target"),
            );
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
            Ok::<_, anyhow::Error>((digest, inode, target))
        };
        // The retention future carries the full bounded archive state machine. Keep both
        // concurrent test futures off the small test-harness stack as the state machine grows.
        let (committed, identity) = tokio::try_join!(Box::pin(work), Box::pin(competitor))
            .expect("same task resumes its selected batch");
        assert_eq!(committed, 1_000);
        if !attempts {
            assert_eq!(
                sha256_hex_file(&identity.2).expect("final digest"),
                identity.0,
                "segment archive identity changed for path={}",
                identity.2.display()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(
                    fs::metadata(&identity.2).expect("final inode").ino(),
                    identity.1.expect("Unix inode")
                );
            }
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

#[tokio::test]
async fn retention_selected_invocation_batch_retries_pressure_without_repreparing_file() {
    let (pool, mut config, temp_dir) = retention_test_pool_and_config("batch-pressure-retry").await;
    config.retention_batch_rows = 1_000;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) SELECT 'pressure-invocation-'||i,?1,'proxy','success',7,0.07,'{}','{}' FROM n")
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("pressure retry candidates");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_millis(100),
    ));
    let probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
    let work = archive_old_invocations(&pool, &config, None, false);
    let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator,
        crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate.clone(),
            crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
                probe.clone(),
                crate::maintenance::retention_test_with_work_budget(
                    Duration::from_secs(86_400),
                    work,
                ),
            ),
        ),
    );
    let work = async {
        let committed = work.await?;
        anyhow::ensure!(
            committed.0 == 1_000,
            "pressure retry returned before the selected batch committed: {committed:?}"
        );
        Ok::<_, anyhow::Error>(committed)
    };
    let competitor = async {
        probe.committed.notified().await;
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("committed source boundary");
        assert_eq!(remaining, 936);
        let target: String = sqlx::query_scalar(
            "SELECT file_path FROM archive_batches
             WHERE dataset='codex_invocations' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("staged invocation archive target");
        let target = PathBuf::from(target);
        let digest = sha256_hex_file(&target).expect("published segment file");
        #[cfg(unix)]
        let inode = Some({
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&target).expect("segment metadata").ino()
        });
        #[cfg(not(unix))]
        let inode: Option<u64> = None;

        gate.record_pressure("retention_pressure_retry_test", "forced_cooldown");
        probe.resume.notify_one();
        Ok::<_, anyhow::Error>((target, digest, inode))
    };

    let (committed, identity) = tokio::try_join!(Box::pin(work), Box::pin(competitor))
        .expect("same task retries its selected batch after pressure cooldown");
    assert_eq!(committed.0, 1_000);
    assert_eq!(
        sha256_hex_file(&identity.0).expect("final segment digest"),
        identity.1
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(&identity.0)
                .expect("final segment metadata")
                .ino(),
            identity.2.expect("Unix inode")
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source closed after pressure retry"),
        0
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_selected_timestamped_batch_retries_after_admission_timeout() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("timestamped-admission-timeout").await;
    config.retention_batch_rows = 1_000;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,finished_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,proxy_binding_key_snapshot) SELECT i,'timeout-attempt-'||i,?1,?1,'/v1/responses','pool',0,0,0,'success','timeout-binding' FROM n")
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("timestamped admission timeout candidates");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
    let work = archive_timestamped_dataset(
        &pool,
        &config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
        "9999-01-01 00:00:00".to_string(),
        false,
        None,
    );
    let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator,
        crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate.clone(),
            crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
                probe.clone(),
                crate::maintenance::retention_test_with_write_admission_budget(
                    Duration::from_millis(20),
                    crate::maintenance::retention_test_with_work_budget(
                        Duration::from_secs(86_400),
                        work,
                    ),
                ),
            ),
        ),
    );
    let competitor = async {
        probe.committed.notified().await;
        let owner = gate
            .try_begin_background("timestamped_admission_timeout_owner")
            .expect("occupy the background slot");
        probe.resume.notify_one();
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(owner);
    };

    let (result, ()) = tokio::join!(Box::pin(work), competitor);
    let result = result.expect("selected timestamped batch retries after admission timeout");
    assert_eq!(result.0, 1_000);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts",)
            .fetch_one(&pool)
            .await
            .expect("timestamped source closed after admission timeout"),
        0
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_timestamped_archive_stops_after_one_selected_window() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("timestamped-selected-window").await;
    config.retention_batch_rows = 1_000;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1200) INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,finished_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,proxy_binding_key_snapshot) SELECT i,'selected-window-attempt-'||i,?1,?1,'/v1/responses','pool',0,0,0,'success','selected-window-binding' FROM n")
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("timestamped selected-window candidates");

    let first = archive_timestamped_dataset(
        &pool,
        &config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
        "9999-01-01 00:00:00".to_string(),
        false,
        None,
    )
    .await
    .expect("archive the first timestamped selection window");
    assert_eq!(first.0, 1_000);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts",)
            .fetch_one(&pool)
            .await
            .expect("remaining timestamped rows after first window"),
        200,
    );

    let second = archive_timestamped_dataset(
        &pool,
        &config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
        "9999-01-01 00:00:00".to_string(),
        false,
        None,
    )
    .await
    .expect("archive the remaining timestamped selection window");
    assert_eq!(second.0, 200);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts",)
            .fetch_one(&pool)
            .await
            .expect("remaining timestamped rows after second window"),
        0,
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
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
        let target = if dataset == "codex_invocations"
            && config.codex_invocation_archive_layout == ArchiveBatchLayout::SegmentV1
        {
            // The segment artifact is prepared for the complete selected window, while source
            // deletion may still be split into smaller SQL microbatches.
            let selected_ids = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM codex_invocations ORDER BY id ASC LIMIT 1000",
            )
            .fetch_all(&pool)
            .await
            .expect("selected invocation ids");
            let day = shanghai_day_key_from_local_naive(&occurred_at).expect("day");
            let part = archive_segment_part_key_for_ids(&selected_ids).expect("segment part");
            archive_segment_file_path(
                &config,
                dataset,
                &day,
                &part,
                config.invocation_archive_codec,
            )
            .expect("segment target")
        } else {
            let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
            archive_batch_file_path(&config, dataset, &month).expect("monthly target")
        };
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(30),
        ));
        let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
        let stopped = Arc::new(Notify::new());
        let work = async {
            if attempts {
                archive_timestamped_dataset(&pool, &config, archive_table_spec(dataset),
                    "SELECT id,occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at,id LIMIT ?2",
                    "9999-01-01 00:00:00".to_string(), false, None).await.map(|result| result.0)
            } else {
                archive_old_invocations(&pool, &config, None, false)
                    .await
                    .map(|result| result.0)
            }
        };
        // Pause after artifact verification so the test can inject a directory-fence
        // collision before any source row is deleted.
        let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator.clone(),
            crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate.clone(),
                crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                    probe.clone(),
                    crate::maintenance::retention_test_with_work_budget(
                        Duration::from_secs(86_400),
                        work,
                    ),
                ),
            ),
        );
        let work = crate::maintenance::retention_test_with_recorded_defer_reason(work);
        let work = async {
            let (result, defer_reason) = work.await;
            stopped.notify_one();
            anyhow::ensure!(
                result.is_err(),
                "fence collision completed without deferring: {result:?}"
            );
            let error = result.expect_err("fence collision error");
            anyhow::ensure!(
                crate::maintenance::is_retention_write_deferred(&error),
                "fence collision became fatal: {error:#}"
            );
            anyhow::ensure!(
                defer_reason.as_deref() == Some("archive_directory_lock_busy"),
                "deferred result lost the actual fence cause: {defer_reason:?}"
            );
            let remaining: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {dataset}"))
                .fetch_one(&pool)
                .await?;
            anyhow::ensure!(
                remaining == 1_000,
                "unexpected source boundary: {remaining}"
            );
            Ok::<_, anyhow::Error>(())
        };
        let competitor = async {
            probe.verified.notified().await;
            let digest = sha256_hex_file(&target).expect("published batch file");
            let fence = crate::maintenance::retention_archive_file_try_lock(&target)
                .expect("batch released file fence before waiting");
            probe.resume.notify_one();
            assert_eq!(coordinator.snapshot().await.active_write_class, None);
            // Retain the fence until the selected batch observes its collision.
            stopped.notified().await;
            drop(fence);
            Ok::<_, anyhow::Error>((digest, target))
        };
        let (_, (digest, target)) = tokio::try_join!(Box::pin(work), Box::pin(competitor))
            .expect("retryable archive fence stop");
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
