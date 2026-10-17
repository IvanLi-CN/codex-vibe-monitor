use super::*;
use crate::prompt_cache_conversations::invocation_ranges::transaction::{
    RANGE_FINISH_OWNER_READY, RangeTransaction,
};
use crate::proxy_sqlite_write_coordinator::{
    ProxySqliteWriteClass, test_proxy_sqlite_write_coordinator,
};

#[derive(Default)]
struct NativeGate {
    entered: Notify,
    armed: AtomicBool,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

struct ReleaseNative(Arc<NativeGate>);
impl Drop for ReleaseNative {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.changed.notify_all();
    }
}

#[tokio::test]
async fn cancelled_reservation_and_finish_waiter_keep_native_writer_ownership() {
    for cancel_finish in [false, true] {
        let (pool, _config, root) = retention_test_pool_and_config("range-native-recovery").await;
        let coordinator = test_proxy_sqlite_write_coordinator();
        let mut transaction = RangeTransaction::begin(
            &pool,
            coordinator
                .acquire(ProxySqliteWriteClass::InteractiveProxy)
                .await,
        )
        .await
        .unwrap();
        let gate = Arc::new(NativeGate {
            armed: AtomicBool::new(true),
            ..Default::default()
        });
        let release = ReleaseNative(gate.clone());
        {
            let mut native = transaction.connection().lock_handle().await.unwrap();
            let gate = gate.clone();
            native.set_progress_handler(1, move || {
                if !gate.armed.swap(false, Ordering::AcqRel) {
                    return true;
                }
                gate.entered.notify_one();
                let released = gate.released.lock().unwrap();
                let _ = gate
                    .changed
                    .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
                    .unwrap();
                false
            });
        }
        {
            let native = sqlx::query("SELECT 1").execute(transaction.connection());
            tokio::pin!(native);
            tokio::select! {
                result = &mut native => panic!("native gate bypassed: {result:?}"),
                _ = gate.entered.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("native gate not entered"),
            }
        }
        if cancel_finish {
            let ready = Arc::new(Notify::new());
            let finish = tokio::spawn(
                RANGE_FINISH_OWNER_READY.scope(ready.clone(), transaction.finish(false)),
            );
            tokio::time::timeout(Duration::from_secs(2), ready.notified())
                .await
                .unwrap();
            finish.abort();
            assert!(finish.await.unwrap_err().is_cancelled());
        } else {
            drop(transaction);
        }
        assert!(
            coordinator
                .try_acquire(ProxySqliteWriteClass::P1Terminal)
                .is_none()
        );
        let mut physical = SqliteConnection::connect_with(
            &pool
                .connect_options()
                .as_ref()
                .clone()
                .busy_timeout(Duration::from_millis(20)),
        )
        .await
        .unwrap();
        assert!(
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut physical)
                .await
                .is_err()
        );
        drop(release);
        let foreground = tokio::time::timeout(
            Duration::from_secs(2),
            coordinator.acquire(ProxySqliteWriteClass::P1Terminal),
        )
        .await
        .expect("native reservation owner must drain before admission");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut physical)
            .await
            .unwrap();
        sqlx::query("ROLLBACK")
            .execute(&mut physical)
            .await
            .unwrap();
        physical.close().await.unwrap();
        drop(foreground);
        pool.close().await;
        cleanup_temp_test_dir(&root);
    }
}
