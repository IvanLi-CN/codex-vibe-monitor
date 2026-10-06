use super::*;

#[tokio::test]
async fn retention_exhausted_pool_wait_is_cancellable_without_holding_writer_admission() {
    use crate::maintenance::{
        RETENTION_TEST_DB_PRESSURE_GATE, RETENTION_TEST_WRITE_COORDINATOR,
        acquire_retention_pool_connection, acquire_retention_write_connection,
        retention_test_with_shutdown, retention_test_with_work_budget,
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("single-connection pool");
    let mut held = pool.acquire().await.expect("exhaust pool");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let shutdown = CancellationToken::new();
    let mut waiting = Box::pin(RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        retention_test_with_work_budget(
            Duration::from_secs(60),
            retention_test_with_shutdown(
                shutdown.clone(),
                acquire_retention_pool_connection(&pool, "pool_exhaustion_test"),
            ),
        ),
    ));
    // Poll while the sole connection is held: this boundary is independent of
    // machine speed and leaves the waiter owning no coordinator permit.
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    let snapshot = coordinator.snapshot().await;
    assert_eq!(snapshot.active_write_class, None);
    assert_eq!(snapshot.maintenance_waiter_count, 0);
    assert_eq!(
        snapshot
            .write_admission
            .maintenance_retention
            .admission_count,
        0
    );
    let p1 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    shutdown.cancel();
    assert!(waiting.await.expect("cancel pool wait").is_none());
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
    drop(p1);
    held.return_to_pool().await;
    drop(held);

    RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                Arc::new(crate::db_pressure::DbPressureGate::new(
                    1,
                    Duration::from_secs(30),
                )),
                async {
                    let (mut connection, admission) =
                        acquire_retention_write_connection(&pool, "pool_reselection_test")
                            .await
                            .expect("fresh connection acquisition")
                            .expect("fresh admission");
                    let mut tx = connection
                        .begin()
                        .await
                        .expect("begin on acquired connection");
                    sqlx::query("CREATE TABLE pool_wait_recovery(id INTEGER)")
                        .execute(tx.as_mut())
                        .await
                        .expect("write after cancelled wait");
                    tx.commit().await.expect("commit fresh write");
                    drop(admission);
                },
            ),
        )
        .await;
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    assert_eq!(
        coordinator
            .snapshot()
            .await
            .write_admission
            .maintenance_retention
            .admission_count,
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn retention_write_connection_never_waits_while_holding_the_other_resource() {
    use crate::maintenance::{
        RETENTION_TEST_WRITE_COORDINATOR, acquire_retention_write_connection,
        retention_test_with_shutdown, retention_test_with_work_budget,
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("single-connection pool");
    let mut held = pool.acquire().await.expect("exhaust pool");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let result = RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            retention_test_with_work_budget(
                Duration::from_secs(60),
                acquire_retention_write_connection(&pool, "pool_write_defer_test"),
            ),
        )
        .await
        .expect("pool exhaustion is admission deferral");
    assert!(result.is_none());
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
    held.return_to_pool().await;
    drop(held);

    let p1 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let shutdown = CancellationToken::new();
    let mut waiting = Box::pin(RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        retention_test_with_shutdown(
            shutdown.clone(),
            acquire_retention_write_connection(&pool, "coordinator_write_wait_test"),
        ),
    ));
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 1);
    // P1 owns the writer permit and can still acquire the sole connection. A
    // retention waiter must not introduce the reverse connection/permit cycle.
    let foreground_connection = pool.acquire().await.expect("P1 may acquire the pool");
    shutdown.cancel();
    assert!(waiting.await.expect("cancel coordinator wait").is_none());
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
    drop(foreground_connection);
    drop(p1);
    pool.close().await;
}

#[tokio::test]
async fn retention_expired_pool_acquisition_takes_no_connection_or_writer_permit() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("single-connection pool");
    let held = pool.acquire().await.expect("exhaust pool");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let result = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            crate::maintenance::retention_test_with_work_budget(
                Duration::ZERO,
                crate::maintenance::acquire_retention_write_connection(&pool, "expired_pool_test"),
            ),
        )
        .await
        .expect("expired work is safely deferred");
    assert!(result.is_none());
    let snapshot = coordinator.snapshot().await;
    assert_eq!(snapshot.active_write_class, None);
    assert_eq!(snapshot.maintenance_waiter_count, 0);
    assert_eq!(
        snapshot
            .write_admission
            .maintenance_retention
            .admission_count,
        0
    );
    let p1 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    drop(p1);
    drop(held);
    pool.close().await;
}

#[tokio::test]
async fn retention_recovery_admission_deadline_cancels_waiter_without_preempting_p1() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-admission-deadline").await;
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let p1 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let result = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            crate::maintenance::retention_test_with_work_budget(
                Duration::from_millis(100),
                crate::maintenance::reconcile_legacy_retention_archive_segments(&pool, &config),
            ),
        )
        .await;
    assert!(result.is_err(), "deadline must defer the unstarted cleanup");
    let snapshot = coordinator.snapshot().await;
    assert_eq!(snapshot.maintenance_waiter_count, 0, "no leaked waiter");
    assert_eq!(
        snapshot
            .write_admission
            .maintenance_retention
            .admission_count,
        0,
        "P1 retains priority"
    );
    let retry: Option<String> = sqlx::query_scalar(
        "SELECT next_retry_at FROM retention_recovery_cursors WHERE scope='legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("cursor fact");
    assert_eq!(retry, None, "no metadata write after admission timeout");
    drop(p1);
    let expired = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            crate::maintenance::retention_test_with_work_budget(
                Duration::ZERO,
                crate::maintenance::reconcile_legacy_retention_archive_segments(&pool, &config),
            ),
        )
        .await;
    assert!(
        expired.is_err(),
        "expired work cannot take an immediately free permit"
    );
    assert_eq!(
        coordinator
            .snapshot()
            .await
            .write_admission
            .maintenance_retention
            .admission_count,
        0
    );
    crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            crate::maintenance::reconcile_legacy_retention_archive_segments(&pool, &config),
        )
        .await
        .expect("fresh task may retry after foreground release");
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
