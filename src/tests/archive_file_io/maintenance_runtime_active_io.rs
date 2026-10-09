use super::*;

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_active_connection_rejects_database_and_sidecar_replacement() {
    for journal in [SqliteJournalMode::Wal, SqliteJournalMode::Delete] {
        for sidecar in [false, true] {
            let directory = make_temp_test_dir("maintenance-runtime-active-io");
            let mut config = test_config();
            config.database_path = directory.join("business.sqlite");
            let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
                crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
            else {
                panic!("offline owner required");
            };
            let options = owner
                .sqlite_connect_options(
                    &config.database_path,
                    SqliteConnectOptions::new().journal_mode(journal),
                )
                .unwrap();
            let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
            sqlx::query("CREATE TABLE proof (id INTEGER)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("PRAGMA wal_autocheckpoint=0")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO proof VALUES(1)")
                .execute(&mut connection)
                .await
                .unwrap();
            if sidecar && journal == SqliteJournalMode::Delete {
                sqlx::query("BEGIN IMMEDIATE")
                    .execute(&mut connection)
                    .await
                    .unwrap();
                sqlx::query("UPDATE proof SET id=2")
                    .execute(&mut connection)
                    .await
                    .unwrap();
            }
            let target = if sidecar {
                PathBuf::from(format!(
                    "{}-{}",
                    config.database_path.display(),
                    if journal == SqliteJournalMode::Wal {
                        "wal"
                    } else {
                        "journal"
                    }
                ))
            } else {
                config.database_path.clone()
            };
            assert!(
                target.exists(),
                "fixture must contain the native file being replaced"
            );
            fs::rename(&target, directory.join("displaced")).unwrap();
            fs::write(&target, b"replacement must remain untouched").unwrap();
            let outcome = if sidecar && journal == SqliteJournalMode::Delete {
                sqlx::query("COMMIT").execute(&mut connection).await
            } else {
                sqlx::query("INSERT INTO proof VALUES(3)")
                    .execute(&mut connection)
                    .await
            };
            assert!(
                outcome.is_err(),
                "existing connection must reject replaced file: {target:?}"
            );
            assert_eq!(
                fs::read(&target).unwrap(),
                b"replacement must remain untouched"
            );
            let _ = connection.close().await;
            assert_eq!(
                fs::read(&target).unwrap(),
                b"replacement must remain untouched",
                "failure cleanup must not delete or rewrite the replacement"
            );
            drop(owner);
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ownership_runtime_maintenance_initialization_failure_drains_workers() {
    use std::os::unix::fs::MetadataExt;
    let directory = make_temp_test_dir("maintenance-runtime-failed-open");
    let mut config = test_config();
    config.database_path = directory.join("business.sqlite");
    let path = config.maintenance_database_path();
    let seed = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::query("CREATE VIEW managed_tasks AS SELECT 'broken' AS task_key")
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;
    let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
    else {
        panic!("offline owner required");
    };
    let identity = fs::metadata(&path).unwrap();
    let descriptor_count = || {
        fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| fs::metadata(entry.path()).ok())
            .filter(|metadata| (metadata.dev(), metadata.ino()) == (identity.dev(), identity.ino()))
            .count()
    };
    let owned = descriptor_count();
    assert_eq!(
        owned, 1,
        "only the runtime fence holds the database before opening"
    );
    assert!(
        crate::maintenance_store::open_owned(&config, &mut owner)
            .await
            .is_err()
    );
    assert_eq!(
        descriptor_count(),
        owned,
        "failed initialization must drain SQLite workers before releasing its owner fence"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_active_connection_rejects_shared_memory_replacement() {
    let directory = make_temp_test_dir("maintenance-runtime-shm-replacement");
    let mut config = test_config();
    config.database_path = directory.join("business.sqlite");
    let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
    else {
        panic!("offline owner required");
    };
    let options = owner
        .sqlite_connect_options(
            &config.database_path,
            SqliteConnectOptions::new().journal_mode(SqliteJournalMode::Wal),
        )
        .unwrap();
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE proof (id INTEGER)")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO proof VALUES(1)")
        .execute(&mut connection)
        .await
        .unwrap();
    let shm = PathBuf::from(format!("{}-shm", config.database_path.display()));
    assert!(shm.exists());
    fs::rename(&shm, directory.join("displaced-shm")).unwrap();
    fs::write(&shm, b"replacement must remain untouched").unwrap();
    assert!(
        sqlx::query("INSERT INTO proof VALUES(2)")
            .execute(&mut connection)
            .await
            .is_err()
    );
    let _ = connection.close().await;
    assert_eq!(
        fs::read(&shm).unwrap(),
        b"replacement must remain untouched"
    );
}
