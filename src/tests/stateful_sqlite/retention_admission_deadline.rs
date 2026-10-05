use super::*;

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
