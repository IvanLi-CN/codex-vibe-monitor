use super::*;
use crate::maintenance::{
    RETENTION_TEST_DB_PRESSURE_GATE, RETENTION_TEST_WRITE_CONNECTION_POOL_READY,
    RETENTION_TEST_WRITE_COORDINATOR, acquire_retention_batch_write_connection,
    retention_test_with_shutdown, retention_test_with_work_budget,
};

#[tokio::test]
async fn retention_batch_priority_reservation_wait_does_not_wake_itself() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let reservation = gate.reserve_priority_background();
    // This is the production mismatch: the preflight sees an idle slot, while
    // best-effort admission must yield to the recovery reservation.
    assert_eq!(gate.background_deny_reason(), None);
    assert_eq!(
        gate.try_begin_background("preflight_probe").unwrap_err(),
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy
    );
    let generation = gate.eligibility_generation();
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let shutdown = CancellationToken::new();
    let ready = Arc::new(Notify::new());
    // Couple permit release to the injected gate, as production does. Cancel
    // on an unexpected self-notification so a regression fails without spinning.
    let notification: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let gate = gate.clone();
        let shutdown = shutdown.clone();
        move || {
            gate.notify_background_eligibility();
            shutdown.cancel();
        }
    });
    let work = crate::proxy_sqlite_write_coordinator::TEST_BACKGROUND_ELIGIBILITY_NOTIFY.scope(
        notification,
        RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator.clone(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate.clone(),
                RETENTION_TEST_WRITE_CONNECTION_POOL_READY.scope(
                    ready.clone(),
                    retention_test_with_work_budget(
                        Duration::from_secs(86_400),
                        retention_test_with_shutdown(
                            shutdown.clone(),
                            acquire_retention_batch_write_connection(&pool, "reserved_batch_wait"),
                        ),
                    ),
                ),
            ),
        ),
    );
    let release = async {
        ready.notified().await;
        assert!(!shutdown.is_cancelled(), "rejected admission woke itself");
        assert_eq!(gate.eligibility_generation(), generation);
        let snapshot = coordinator.snapshot().await;
        assert_eq!(snapshot.active_write_class, None);
        assert_eq!(
            snapshot
                .write_admission
                .maintenance_retention
                .admission_count,
            1
        );
        let mut connection = pool.acquire().await.expect("no retained connection");
        connection.return_to_pool().await;
        drop(connection);
        drop(reservation);
    };
    let (result, ()) = tokio::join!(work, release);
    let (mut connection, admission) = result
        .expect("reservation wait")
        .expect("external wake resumes");
    assert!(!shutdown.is_cancelled());
    assert_eq!(
        coordinator
            .snapshot()
            .await
            .write_admission
            .maintenance_retention
            .admission_count,
        2
    );
    drop(admission);
    connection.return_to_pool().await;
    drop(connection);
    pool.close().await;
}

#[tokio::test]
async fn retention_batch_background_wait_cancels_without_owning_pool_or_coordinator() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let owner = gate.try_begin_background("owner").expect("occupied slot");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let shutdown = CancellationToken::new();
    let ready = Arc::new(Notify::new());
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate.clone(),
            RETENTION_TEST_WRITE_CONNECTION_POOL_READY.scope(
                ready.clone(),
                retention_test_with_work_budget(
                    Duration::from_secs(86_400),
                    retention_test_with_shutdown(
                        shutdown.clone(),
                        acquire_retention_batch_write_connection(&pool, "cancel_batch_wait"),
                    ),
                ),
            ),
        ),
    );
    let control = async {
        ready.notified().await;
        assert_eq!(coordinator.snapshot().await.active_write_class, None);
        assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
        let mut connection = pool.acquire().await.expect("no retained connection");
        connection.return_to_pool().await;
        drop(connection);
        shutdown.cancel();
    };
    let (result, ()) = tokio::join!(work, control);
    assert!(result.expect("cancelled wait").is_none());
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    assert_eq!(
        gate.try_begin_background("probe").unwrap_err(),
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy
    );
    drop(owner);
    pool.close().await;
}

#[tokio::test]
async fn retention_batch_wait_stops_on_real_pressure_and_expired_deadline() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let owner = gate.try_begin_background("owner").expect("occupied slot");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let ready = Arc::new(Notify::new());
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate.clone(),
            RETENTION_TEST_WRITE_CONNECTION_POOL_READY.scope(
                ready.clone(),
                retention_test_with_work_budget(
                    Duration::from_secs(86_400),
                    acquire_retention_batch_write_connection(&pool, "pressure_batch_wait"),
                ),
            ),
        ),
    );
    let pressure = async {
        ready.notified().await;
        gate.record_pressure("injected_lock_pressure", "sqlite_locked");
    };
    let (result, ()) = tokio::join!(work, pressure);
    assert!(result.expect("pressure stops wait").is_none());
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    drop(owner);
    let result = RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator.clone(),
            retention_test_with_work_budget(
                Duration::ZERO,
                acquire_retention_batch_write_connection(&pool, "expired_batch_wait"),
            ),
        )
        .await
        .expect("expired wait");
    assert!(result.is_none());
    assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
    pool.close().await;
}
