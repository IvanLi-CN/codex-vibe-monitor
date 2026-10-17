use super::*;
use crate::maintenance::{
    PERFORMANCE_INDEX_TEST_PROBE, PerformanceIndexNativeProbe, PerformanceIndexPreparationRequest,
    prepare_performance_indexes,
};

struct ReleaseNativeProbe(Arc<PerformanceIndexNativeProbe>);
impl Drop for ReleaseNativeProbe {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn performance_index_cancelled_client_retains_native_owner_until_rollback_and_close() {
    let directory = make_temp_test_dir("performance-index-native-drain");
    let options = SqliteConnectOptions::new()
        .filename(directory.join("business.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE TABLE proxy_raw_payload_blob_links(owner_kind TEXT, owner_id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE codex_invocations(id INTEGER PRIMARY KEY, payload TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let probe = Arc::new(PerformanceIndexNativeProbe::default());
    let _release_on_failure = ReleaseNativeProbe(probe.clone());
    let cancel = CancellationToken::new();
    let client = tokio::spawn({
        let coordinator = coordinator.clone();
        let probe = probe.clone();
        let cancel = cancel.clone();
        let options = options.clone();
        async move {
            PERFORMANCE_INDEX_TEST_PROBE
                .scope(
                    probe,
                    prepare_performance_indexes(
                        options,
                        PerformanceIndexPreparationRequest {
                            dry_run: false,
                            budget_ms: 2_000,
                        },
                        coordinator,
                        cancel,
                    ),
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), probe.entered.notified())
        .await
        .expect("DDL reached native callback");
    cancel.cancel();
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    assert_eq!(
        coordinator.snapshot().await.active_write_class.as_deref(),
        Some("maintenance_retention")
    );
    assert!(
        coordinator
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
            .is_none(),
        "cancelled waiter cannot release native ownership"
    );
    probe.release();
    timeout(Duration::from_secs(2), probe.closed.notified())
        .await
        .expect("native worker closed after cancellation");
    let permit = timeout(
        Duration::from_secs(2),
        coordinator
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal),
    )
    .await
    .expect("admission resumes after native drain");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='index'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "interrupted DDL rolled back"
    );
    drop(permit);
    let resumed = prepare_performance_indexes(
        options,
        PerformanceIndexPreparationRequest::default(),
        coordinator,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(resumed.is_complete());
    assert!(
        resumed
            .indexes
            .iter()
            .all(|index| index.outcome == "created")
    );
    pool.close().await;
}

#[tokio::test]
async fn performance_index_lock_budget_keeps_old_path_usable_and_allows_reentry() {
    let directory = make_temp_test_dir("performance-index-lock-budget");
    let options = SqliteConnectOptions::new()
        .filename(directory.join("business.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE TABLE proxy_raw_payload_blob_links(owner_kind TEXT, owner_id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE codex_invocations(id INTEGER PRIMARY KEY, payload TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let report = prepare_performance_indexes(
        options.clone(),
        PerformanceIndexPreparationRequest {
            dry_run: false,
            budget_ms: 20,
        },
        coordinator.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(matches!(report.outcome, "budget_exhausted" | "sqlite_busy"));
    assert!(!report.is_complete());
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    assert!(
        prepare_performance_indexes(
            options,
            PerformanceIndexPreparationRequest::default(),
            coordinator,
            CancellationToken::new()
        )
        .await
        .unwrap()
        .is_complete()
    );
    pool.close().await;
}
