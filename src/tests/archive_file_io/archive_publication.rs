use super::*;
use crate::maintenance::{
    PublicationTestPhase, PublicationTestProbe, RETENTION_PUBLICATION_TEST_PROBE,
    replace_legacy_archive_file_for_test,
};

struct PublicationFixture {
    pool: SqlitePool,
    root: PathBuf,
    final_path: PathBuf,
    temporary_path: PathBuf,
    previous_sha: Option<String>,
    next_sha: String,
}

impl PublicationFixture {
    async fn new(existing: bool) -> Self {
        let root = std::env::temp_dir().join(format!("archive-publication-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create owned publication directory");
        let final_path = root.join("archive.sqlite.gz");
        let temporary_path = root.join("prepared.tmp");
        let previous_sha = existing.then(|| {
            fs::write(&final_path, b"previous archive").expect("previous artifact");
            sha256_hex_file(&final_path).expect("previous digest")
        });
        fs::write(&temporary_path, b"replacement archive").expect("prepared artifact");
        let next_sha = sha256_hex_file(&temporary_path).expect("replacement digest");
        let database_path = root.join("publication.sqlite");
        fs::File::create(&database_path).expect("create durable publication database");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                build_sqlite_connect_options(
                    &test_sqlite_url_for_path(&database_path),
                    Duration::from_millis(20),
                )
                .expect("publication database connect options"),
            )
            .await
            .expect("owned publication database");
        sqlx::raw_sql(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY, dataset TEXT, month_key TEXT, file_path TEXT,
                sha256 TEXT, cleanup_state TEXT, cleanup_source_safe_start_date TEXT,
                replacement_staged_path TEXT);
             CREATE TABLE retention_prepared_archives (
                dataset TEXT, file_path TEXT, state TEXT, staged_file_path TEXT,
                updated_at TEXT);",
        )
        .execute(&pool)
        .await
        .expect("publication and recovery schemas");
        sqlx::query(
            "INSERT INTO archive_batches(id,dataset,month_key,file_path,sha256,cleanup_state)
             VALUES(1,'codex_invocations','2020-01',?1,?2,'active')",
        )
        .bind(final_path.to_string_lossy().to_string())
        .bind(&previous_sha)
        .execute(&pool)
        .await
        .expect("seed manifest");
        sqlx::query(
            "INSERT INTO retention_prepared_archives(dataset,file_path,state)
             VALUES('codex_invocations',?1,'preparing')",
        )
        .bind(final_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("seed journal");
        Self {
            pool,
            root,
            final_path,
            temporary_path,
            previous_sha,
            next_sha,
        }
    }

    async fn publish(&self) -> Result<()> {
        crate::maintenance::retention_test_with_work_budget(
            Duration::from_secs(3),
            replace_legacy_archive_file_for_test(
                &self.pool,
                "codex_invocations",
                "2020-01",
                &self.temporary_path,
                &self.final_path,
                self.previous_sha.as_deref(),
                &self.next_sha,
            ),
        )
        .await
    }

    async fn pointers(&self) -> (Option<String>, Option<String>) {
        let manifest =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id=1")
                .fetch_one(&self.pool)
                .await
                .expect("manifest recovery pointer");
        let prepared =
            sqlx::query_scalar("SELECT staged_file_path FROM retention_prepared_archives")
                .fetch_one(&self.pool)
                .await
                .expect("prepared recovery pointer");
        (manifest, prepared)
    }

    async fn cleanup(self) {
        self.pool.close().await;
        cleanup_temp_test_dir(&self.root);
    }
}

fn publication_probe(phase: PublicationTestPhase, fail: bool) -> Arc<PublicationTestProbe> {
    Arc::new(PublicationTestProbe {
        phase,
        fail,
        reached: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    })
}

#[tokio::test]
async fn archive_publication_present_and_absent_predecessors_commit_and_clear_recovery() {
    for existing in [false, true] {
        let fixture = PublicationFixture::new(existing).await;
        fixture
            .publish()
            .await
            .expect("publish exact prepared bytes");
        assert_eq!(
            fs::read(&fixture.final_path).expect("published bytes"),
            b"replacement archive"
        );
        let manifest: String = sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE id=1")
            .fetch_one(&fixture.pool)
            .await
            .expect("committed digest");
        assert_eq!(manifest, fixture.next_sha);
        assert_eq!(fixture.pointers().await, (None, None));
        assert!(!fixture.temporary_path.exists());
        fixture.cleanup().await;
    }
}

#[tokio::test]
async fn archive_publication_known_precommit_failure_restores_or_removes_only_its_artifact() {
    for existing in [false, true] {
        let fixture = PublicationFixture::new(existing).await;
        let probe = publication_probe(PublicationTestPhase::AfterRename, true);
        probe.resume.notify_one();
        let result = RETENTION_PUBLICATION_TEST_PROBE
            .scope(probe, fixture.publish())
            .await;
        assert!(result.is_err());
        if existing {
            assert_eq!(
                fs::read(&fixture.final_path).expect("restored bytes"),
                b"previous archive"
            );
        } else {
            assert!(
                !fixture.final_path.exists(),
                "uncommitted first artifact must be removed"
            );
        }
        let manifest: Option<String> =
            sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE id=1")
                .fetch_one(&fixture.pool)
                .await
                .expect("unchanged manifest");
        assert_eq!(manifest, fixture.previous_sha);
        assert_eq!(fixture.pointers().await, (None, None));
        fixture.cleanup().await;
    }
}

#[tokio::test]
async fn archive_publication_ambiguous_commit_retains_new_bytes_and_recovery_evidence() {
    let fixture = PublicationFixture::new(true).await;
    let probe = publication_probe(PublicationTestPhase::AfterCommit, true);
    probe.resume.notify_one();
    let error = RETENTION_PUBLICATION_TEST_PROBE
        .scope(probe, fixture.publish())
        .await
        .expect_err("caller did not observe a successful commit");
    assert!(error.to_string().contains("outcome is unknown"));
    assert_eq!(
        fs::read(&fixture.final_path).expect("durable final bytes"),
        b"replacement archive"
    );
    let (manifest_pointer, prepared_pointer) = fixture.pointers().await;
    assert_eq!(manifest_pointer, prepared_pointer);
    let backup = PathBuf::from(manifest_pointer.expect("durable rollback reference"));
    assert_eq!(
        fs::read(&backup).expect("rollback bytes retained"),
        b"previous archive"
    );
    let digest: String = sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE id=1")
        .fetch_one(&fixture.pool)
        .await
        .expect("actual committed manifest");
    assert_eq!(
        digest, fixture.next_sha,
        "never compensate an ambiguous successful commit"
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn archive_publication_cleanup_retains_file_fence_after_releasing_database_admission() {
    let fixture = Arc::new(PublicationFixture::new(true).await);
    let probe = publication_probe(PublicationTestPhase::AfterNativeDrain, false);
    let publication = tokio::spawn({
        let fixture = fixture.clone();
        let probe = probe.clone();
        async move {
            RETENTION_PUBLICATION_TEST_PROBE
                .scope(probe, fixture.publish())
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), probe.reached.notified())
        .await
        .expect("publication reached post-drain cleanup");
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let foreground = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .expect("post-commit file cleanup cannot retain original writer admission");
    let mut connection = fixture
        .pool
        .acquire()
        .await
        .expect("native connection returned");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .expect("native writer is no longer held");
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .expect("release foreground writer");
    connection.return_to_pool().await;
    assert!(
        crate::maintenance::retention_archive_file_try_lock(&fixture.final_path).is_err(),
        "file cleanup still owns the directory fence"
    );
    drop(foreground);
    probe.resume.notify_one();
    publication
        .await
        .expect("publication owner")
        .expect("cleanup completed");
    let fixture = Arc::try_unwrap(fixture)
        .ok()
        .expect("fixture owner returned");
    assert_eq!(fixture.pointers().await, (None, None));
    fixture.cleanup().await;
}

#[tokio::test]
async fn archive_publication_failed_file_restore_keeps_both_recovery_pointers() {
    let fixture = Arc::new(PublicationFixture::new(true).await);
    let probe = publication_probe(PublicationTestPhase::AfterRename, true);
    let publication = tokio::spawn({
        let fixture = fixture.clone();
        let probe = probe.clone();
        async move {
            RETENTION_PUBLICATION_TEST_PROBE
                .scope(probe, fixture.publish())
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), probe.reached.notified())
        .await
        .expect("renamed publication is paused before COMMIT");
    let backup = fs::read_dir(&fixture.root)
        .expect("owned fixture entries")
        .map(|entry| entry.expect("owned fixture entry").path())
        .find(|path| path.to_string_lossy().ends_with(".restore"))
        .expect("the durable rollback link was created");
    // Remove only the test-owned link from the expected location, preserving its
    // bytes, to inject a deterministic failed restore without permission tricks.
    let displaced = fixture.root.join("displaced-rollback-for-fault");
    fs::rename(&backup, &displaced).expect("inject unavailable rollback location");
    probe.resume.notify_one();
    let error = publication
        .await
        .expect("publication owner")
        .expect_err("file compensation failed");
    assert!(error.to_string().contains("file compensation deferred"));
    let fixture = Arc::try_unwrap(fixture)
        .ok()
        .expect("fixture owner returned");
    let expected = Some(backup.to_string_lossy().to_string());
    assert_eq!(fixture.pointers().await, (expected.clone(), expected));
    assert_eq!(
        fs::read(&displaced).expect("preserved test predecessor"),
        b"previous archive"
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn archive_publication_cancellation_retains_recoverable_predecessor_until_worker_drain() {
    let fixture = Arc::new(PublicationFixture::new(true).await);
    let probe = publication_probe(PublicationTestPhase::AfterRename, false);
    let publication = tokio::spawn({
        let fixture = fixture.clone();
        let probe = probe.clone();
        async move {
            RETENTION_PUBLICATION_TEST_PROBE
                .scope(probe, fixture.publish())
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), probe.reached.notified())
        .await
        .expect("open transaction has published its replacement");
    publication.abort();
    assert!(
        publication
            .await
            .expect_err("publication cancelled")
            .is_cancelled()
    );
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let foreground = tokio::time::timeout(
        Duration::from_secs(2),
        coordinator
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal),
    )
    .await
    .expect("cancelled resource owner must finish native rollback");
    let mut connection = fixture
        .pool
        .acquire()
        .await
        .expect("pool usable after native drain");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .expect("a released admission cannot leave a native transaction behind");
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .expect("release foreground writer");
    connection.return_to_pool().await;
    drop(foreground);
    let fixture = Arc::try_unwrap(fixture)
        .ok()
        .expect("fixture owner returned");
    let (manifest_pointer, prepared_pointer) = fixture.pointers().await;
    assert_eq!(manifest_pointer, prepared_pointer);
    let backup =
        PathBuf::from(manifest_pointer.expect("cancelled publication remains recoverable"));
    assert_eq!(
        fs::read(&backup).expect("recoverable predecessor"),
        b"previous archive"
    );
    let digest: String = sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE id=1")
        .fetch_one(&fixture.pool)
        .await
        .expect("rolled-back manifest");
    assert_eq!(Some(digest), fixture.previous_sha);
    let fence = crate::maintenance::retention_archive_file_try_lock(&fixture.final_path)
        .expect("cancelled owner releases file fence after draining native work");
    drop(fence);
    crate::maintenance::reconcile_staged_archive_replacements_for_test(&fixture.pool, None)
        .await
        .expect("existing durable replacement recovery can reenter");
    assert_eq!(
        fs::read(&fixture.final_path).expect("recovered final bytes"),
        b"previous archive"
    );
    let (manifest_pointer, prepared_pointer) = fixture.pointers().await;
    assert!(manifest_pointer.is_none());
    // The prepared-archive owner clears its own pointer on the next pass. It
    // must not cause the manifest recovery to overwrite a newer publication.
    assert_eq!(prepared_pointer, Some(backup.to_string_lossy().to_string()));
    crate::maintenance::reconcile_staged_archive_replacements_for_test(&fixture.pool, None)
        .await
        .expect("repeated recovery is idempotent");
    assert_eq!(
        fs::read(&fixture.final_path).expect("stable recovered bytes"),
        b"previous archive"
    );
    fixture.cleanup().await;
}

struct PublicationNativeGate {
    entered: tokio::sync::Notify,
    armed: std::sync::atomic::AtomicBool,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

struct ReleasePublicationNativeGate(Arc<PublicationNativeGate>);

impl Drop for ReleasePublicationNativeGate {
    fn drop(&mut self) {
        *self.0.released.lock().expect("native fixture gate") = true;
        self.0.changed.notify_all();
    }
}

#[tokio::test]
async fn summary_live_tail_cancellation_retains_native_write_and_background_admission() {
    use crate::api::{SUMMARY_LIVE_TAIL_CLEANUP_READY, SummaryLiveTailTransaction};
    use crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass;

    for cancel_finish_waiter in [false, true] {
        let (pool, _config, root) = retention_test_pool_and_config("summary-native-drain").await;
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
        let pressure = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
        let background = Arc::new(
            pressure
                .try_begin_background("summary-native-fixture")
                .unwrap(),
        );
        let mut transaction = SummaryLiveTailTransaction::for_test(
            pool.acquire().await.expect("publication connection"),
            coordinator.acquire(ProxySqliteWriteClass::P2Derived).await,
            background,
        );
        transaction
            .begin()
            .await
            .expect("native Summary transaction");
        let gate = Arc::new(PublicationNativeGate {
            entered: tokio::sync::Notify::new(),
            armed: std::sync::atomic::AtomicBool::new(true),
            released: std::sync::Mutex::new(false),
            changed: std::sync::Condvar::new(),
        });
        let release_on_exit = ReleasePublicationNativeGate(gate.clone());
        {
            let mut native = transaction.connection_mut().lock_handle().await.unwrap();
            let gate = gate.clone();
            native.set_progress_handler(1, move || {
                if !gate.armed.swap(false, std::sync::atomic::Ordering::AcqRel) {
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
            let native = sqlx::query("SELECT 1").execute(transaction.connection_mut());
            tokio::pin!(native);
            tokio::select! {
                result = &mut native => panic!("native barrier was bypassed: {result:?}"),
                _ = gate.entered.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("native barrier not entered"),
            }
        }
        if cancel_finish_waiter {
            let ready = Arc::new(tokio::sync::Notify::new());
            let finish = tokio::spawn(
                SUMMARY_LIVE_TAIL_CLEANUP_READY.scope(ready.clone(), transaction.finish(false)),
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
        assert!(pressure.try_begin_background("other-background").is_err());
        let mut physical = SqliteConnection::connect_with(
            &pool
                .connect_options()
                .as_ref()
                .clone()
                .busy_timeout(Duration::from_millis(20)),
        )
        .await
        .expect("independent physical writer");
        assert!(
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut physical)
                .await
                .is_err()
        );
        drop(release_on_exit);
        let foreground = tokio::time::timeout(
            Duration::from_secs(2),
            coordinator.acquire(ProxySqliteWriteClass::P1Terminal),
        )
        .await
        .expect("Summary native cleanup drained");
        assert!(pressure.try_begin_background("other-background").is_ok());
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut physical)
            .await
            .expect("physical writer released");
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

#[tokio::test]
async fn retention_native_cleanup_keeps_ownership_when_its_waiter_is_cancelled() {
    for cancel_finish_waiter in [false, true] {
        let (pool, _config, root) = retention_test_pool_and_config("native-resource-drain").await;
        let path = root.join("resource.sqlite.gz");
        fs::write(&path, b"native resource fence").expect("owned archive fence target");
        let digest = sha256_hex_file(&path).expect("owned target digest");
        let coordinator =
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
        let pressure = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(30),
        ));
        let (connection, admission, fence) = RETENTION_TEST_WRITE_COORDINATOR
            .scope(
                coordinator.clone(),
                RETENTION_TEST_DB_PRESSURE_GATE.scope(
                    pressure,
                    crate::maintenance::acquire_retention_batch_write_connection_with_archive_fence(
                        &pool,
                        "native_resource_fixture",
                        &path,
                        Some(&digest),
                    ),
                ),
            )
            .await
            .expect("acquire fixture ownership")
            .expect("fixture admitted");
        let mut resources =
            crate::maintenance::RetentionTransactionResources::new(connection, admission, fence);
        resources.mark_transaction_started();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(resources.connection_mut())
            .await
            .expect("native writer transaction");
        let gate = Arc::new(PublicationNativeGate {
            entered: tokio::sync::Notify::new(),
            armed: std::sync::atomic::AtomicBool::new(true),
            released: std::sync::Mutex::new(false),
            changed: std::sync::Condvar::new(),
        });
        let release_on_exit = ReleasePublicationNativeGate(gate.clone());
        {
            let mut handle = resources
                .connection_mut()
                .lock_handle()
                .await
                .expect("native handle");
            let gate = gate.clone();
            handle.set_progress_handler(1, move || {
                if !gate.armed.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    return true;
                }
                gate.entered.notify_one();
                let released = gate.released.lock().expect("native fixture gate");
                // This is SQLite's dedicated native worker. A bounded fixture
                // barrier keeps real work alive after dropping the SQL future.
                let _ = gate
                    .changed
                    .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
                    .expect("native fixture barrier");
                false
            });
        }
        {
            let work = sqlx::query("SELECT 1").execute(resources.connection_mut());
            tokio::pin!(work);
            tokio::select! {
                result = &mut work => panic!("native work completed before its barrier: {result:?}"),
                _ = gate.entered.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("native worker did not enter fixture barrier"),
            }
        }
        if cancel_finish_waiter {
            let ready = Arc::new(tokio::sync::Notify::new());
            let finish = tokio::spawn(crate::maintenance::RETENTION_CLEANUP_WAITER_READY.scope(
                ready.clone(),
                resources.rollback_and_close_retaining_archive_fence(),
            ));
            tokio::time::timeout(Duration::from_secs(2), ready.notified())
                .await
                .expect("cleanup owner detached before cancelling its waiter");
            finish.abort();
            assert!(
                finish
                    .await
                    .expect_err("finish waiter cancelled")
                    .is_cancelled()
            );
        } else {
            drop(resources);
        }
        assert!(
            coordinator
                .try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal,
                )
                .is_none(),
            "native work must retain writer admission after cancellation"
        );
        assert!(
            crate::maintenance::retention_archive_file_try_lock(&path).is_err(),
            "native work must retain the file fence after cancellation"
        );
        let options = pool
            .connect_options()
            .as_ref()
            .clone()
            .busy_timeout(Duration::from_millis(20));
        let mut independent = SqliteConnection::connect_with(&options)
            .await
            .expect("independent physical writer connection");
        assert!(
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut independent)
                .await
                .is_err(),
            "the native SQLite writer is physically held during the barrier"
        );
        drop(release_on_exit);
        let foreground = tokio::time::timeout(
            Duration::from_secs(2),
            coordinator
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal),
        )
        .await
        .expect("native rollback and close complete before releasing admission");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut independent)
            .await
            .expect("physical writer available after native drain");
        sqlx::query("ROLLBACK")
            .execute(&mut independent)
            .await
            .expect("release physical writer");
        independent.close().await.expect("close physical writer");
        drop(foreground);
        let fence = crate::maintenance::retention_archive_file_try_lock(&path)
            .expect("file fence available after native drain");
        drop(fence);
        pool.close().await;
        cleanup_temp_test_dir(&root);
    }
}
