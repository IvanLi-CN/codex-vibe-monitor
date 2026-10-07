use super::*;
use crate::maintenance::{
    RETENTION_TEST_DB_PRESSURE_GATE, RETENTION_TEST_WRITE_CONNECTION_POOL_READY,
    RETENTION_TEST_WRITE_COORDINATOR, acquire_retention_batch_write_connection,
    retention_test_with_shutdown, retention_test_with_work_budget,
};

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
