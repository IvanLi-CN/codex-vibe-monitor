use super::*;

#[cfg(unix)]
#[test]
fn ownership_runtime_sqlite_open_supports_concurrent_vfs_registration() {
    let directory = make_temp_test_dir("maintenance-runtime-concurrent-vfs");
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let children: Vec<_> = (0..4)
        .map(|index| {
            let mut config = test_config();
            config.database_path = directory.join(format!("business-{index}.sqlite"));
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
                    crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
                else {
                    panic!("offline owner required");
                };
                barrier.wait();
                let options = owner
                    .sqlite_connect_options(
                        &config.database_path,
                        SqliteConnectOptions::new().journal_mode(SqliteJournalMode::Wal),
                    )
                    .unwrap();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
                    sqlx::query("CREATE TABLE proof (id INTEGER)")
                        .execute(&mut connection)
                        .await
                        .unwrap();
                    connection.close().await.unwrap();
                });
            })
        })
        .collect();
    for child in children {
        child.join().unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_sqlite_open_rejects_replacement_before_initialization() {
    for maintenance in [false, true] {
        let directory = make_temp_test_dir("maintenance-runtime-pre-open");
        let mut config = test_config();
        config.database_path = directory.join("business.sqlite");
        fs::write(&config.database_path, b"").unwrap();
        fs::write(config.maintenance_database_path(), b"").unwrap();
        let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
            crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
        else {
            panic!("offline owner required");
        };
        let target = if maintenance {
            config.maintenance_database_path()
        } else {
            config.database_path.clone()
        };
        let options = owner
            .sqlite_connect_options(
                &target,
                SqliteConnectOptions::new().journal_mode(SqliteJournalMode::Wal),
            )
            .unwrap();
        let replacement = directory.join("replacement.sqlite");
        fs::write(&replacement, b"").unwrap();
        fs::rename(&target, directory.join("displaced.sqlite")).unwrap();
        fs::rename(replacement, &target).unwrap();
        assert!(owner.refresh_inode_pair_lock().is_err());
        assert!(owner.publish_role("service:ownership-v1:ready").is_err());
        assert!(SqliteConnection::connect_with(&options).await.is_err());
        assert!(
            fs::read(&target).unwrap().is_empty(),
            "no header or schema write before rejection"
        );
        assert!(!target.with_extension("sqlite-wal").exists());
        assert!(!target.with_extension("sqlite-shm").exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_sqlite_open_checks_native_handle_after_path_restoration() {
    for maintenance in [false, true] {
        let directory = make_temp_test_dir("maintenance-runtime-native-open");
        let mut config = test_config();
        config.database_path = directory.join("business.sqlite");
        let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
            crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
        else {
            panic!("offline owner required");
        };
        let target = if maintenance {
            config.maintenance_database_path()
        } else {
            config.database_path.clone()
        };
        let options = owner
            .sqlite_connect_options(
                &target,
                SqliteConnectOptions::new().journal_mode(SqliteJournalMode::Wal),
            )
            .unwrap();
        let replacement = directory.join("replacement.sqlite");
        fs::write(&replacement, b"").unwrap();
        crate::maintenance::replace_during_next_open(target.clone(), replacement.clone());
        assert!(
            SqliteConnection::connect_with(&options).await.is_err(),
            "native SQLite inode differs even after the held path is restored"
        );
        owner.refresh_inode_pair_lock().unwrap();
        assert!(fs::read(&target).unwrap().is_empty());
        assert!(
            fs::read(replacement).unwrap().is_empty(),
            "no journal/header initialization on the unowned inode"
        );
        let connection = SqliteConnection::connect_with(&options).await.unwrap();
        connection.close().await.unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_sqlite_open_guards_new_pool_connections_and_lease_end() {
    let directory = make_temp_test_dir("maintenance-runtime-pool-open");
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
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    let mut original = pool.acquire().await.unwrap();
    let displaced = directory.join("displaced.sqlite");
    fs::rename(&config.database_path, &displaced).unwrap();
    fs::write(&config.database_path, b"").unwrap();
    assert!(
        sqlx::query("CREATE TABLE replaced_connection_guard (id INTEGER)")
            .execute(&mut *original)
            .await
            .is_err(),
        "an already-open connection must fail closed after its database path is replaced"
    );
    assert!(
        SqliteConnection::connect_with(&pool.connect_options())
            .await
            .is_err()
    );
    assert!(fs::read(&config.database_path).unwrap().is_empty());
    fs::remove_file(&config.database_path).unwrap();
    fs::rename(&displaced, &config.database_path).unwrap();
    let second = SqliteConnection::connect_with(&pool.connect_options())
        .await
        .unwrap();
    second.close().await.unwrap();
    drop(original);
    pool.close().await;
    drop(owner);
    assert!(
        SqliteConnection::connect_with(&options).await.is_err(),
        "cached options cannot outlive runtime ownership"
    );
    assert!(
        matches!(
            crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap(),
            crate::maintenance::MaintenanceRuntimeRoute::Offline(_)
        ),
        "weak VFS registration must not retain database locks"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_sqlite_open_preserves_archive_attach_and_rejects_new_hardlinks() {
    let directory = make_temp_test_dir("maintenance-runtime-attach-open");
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
            SqliteConnectOptions::new()
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .unwrap();
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    let archive = directory.join("archive.sqlite");
    sqlx::query("ATTACH DATABASE ? AS archive_db")
        .bind(archive.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE archive_db.proof (id INTEGER)")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DETACH DATABASE archive_db")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    fs::hard_link(&config.database_path, directory.join("hardlink.sqlite")).unwrap();
    assert!(owner.refresh_inode_pair_lock().is_err());
    assert!(SqliteConnection::connect_with(&options).await.is_err());
}

#[cfg(unix)]
struct RuntimeOwnerChild(std::process::Child);

#[cfg(unix)]
impl Drop for RuntimeOwnerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
#[test]
#[ignore = "child fixture for the maintenance runtime process contract"]
fn ownership_runtime_lock_child_fixture() {
    let mut config = test_config();
    config.database_path = PathBuf::from(std::env::var_os("CVM_TEST_RUNTIME_DATABASE").unwrap());
    if std::env::var("CVM_TEST_RUNTIME_ROUTE_ASSERTION").as_deref() == Ok("unavailable") {
        assert!(crate::maintenance::MaintenanceRuntimeLock::route(&config, true).is_err());
        return;
    }
    let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut owner) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, false).unwrap()
    else {
        panic!("child must own the runtime");
    };
    owner
        .publish_role("service:ownership-v1:initializing")
        .unwrap();
    fs::write(&config.database_path, b"business").unwrap();
    fs::write(config.maintenance_database_path(), b"maintenance").unwrap();
    owner.refresh_inode_pair_lock().unwrap();
    owner.publish_role("service:ownership-v1:ready").unwrap();
    fs::write(config.database_path.with_extension("ready"), b"ready").unwrap();
    // stdin closure ends the fixture normally; the parent guard also handles failure.
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
}

#[cfg(unix)]
#[tokio::test]
async fn ownership_runtime_lock_routes_online_and_recovers_process_death_without_unlinking() {
    let directory = make_temp_test_dir("maintenance-runtime-process");
    let mut config = test_config();
    config.database_path = directory.join("business.sqlite");
    let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut offline) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, true).unwrap()
    else {
        panic!("fresh runtime must be offline");
    };
    offline.publish_role("cli:ownership-v1").unwrap();
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&config, true).is_err());
    offline
        .publish_role("service:ownership-v1:initializing")
        .unwrap();
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&config, true).is_err());
    drop(offline);
    let mut child = RuntimeOwnerChild(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::archive_file_io::maintenance_runtime_ownership::ownership_runtime_lock_child_fixture", "--ignored"])
        .env("CVM_TEST_RUNTIME_DATABASE", &config.database_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !config.database_path.with_extension("ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "runtime owner died before readiness"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let maintenance_path = config.maintenance_database_path();
    let displaced_maintenance = directory.join("displaced-maintenance.sqlite");
    let crate::maintenance::MaintenanceRuntimeRoute::Online(mut online) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, true).unwrap()
    else {
        panic!("ready service must route online");
    };
    online
        .sqlite_connect_options(
            &config.maintenance_database_path(),
            SqliteConnectOptions::new().create_if_missing(false),
        )
        .unwrap();
    fs::rename(&maintenance_path, &displaced_maintenance).unwrap();
    fs::write(&maintenance_path, b"replacement").unwrap();
    assert!(
        online
            .sqlite_connect_options(
                &config.maintenance_database_path(),
                SqliteConnectOptions::new().create_if_missing(false),
            )
            .is_err(),
        "online clients must reject a maintenance inode replaced after routing"
    );
    fs::remove_file(&maintenance_path).unwrap();
    fs::rename(&displaced_maintenance, &maintenance_path).unwrap();
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&config, false).is_err());
    fs::rename(&maintenance_path, &displaced_maintenance).unwrap();
    fs::write(&maintenance_path, b"replacement").unwrap();
    assert!(
        crate::maintenance::MaintenanceRuntimeLock::route(&config, true).is_err(),
        "ready path markers must not admit a replaced database inode"
    );
    fs::remove_file(&maintenance_path).unwrap();
    fs::rename(&displaced_maintenance, &maintenance_path).unwrap();
    let alias = directory.join("alias");
    fs::create_dir_all(&alias).unwrap();
    let mut alias_config = config.clone();
    alias_config.database_path = alias.join("..").join("business.sqlite");
    assert!(matches!(
        crate::maintenance::MaintenanceRuntimeLock::route(&alias_config, true).unwrap(),
        crate::maintenance::MaintenanceRuntimeRoute::Online(_)
    ));
    let hardlink_dir = directory.join("hardlinks");
    fs::create_dir_all(&hardlink_dir).unwrap();
    fs::write(&config.database_path, b"business").unwrap();
    fs::write(&maintenance_path, b"maintenance").unwrap();
    let hard_business = hardlink_dir.join("business.sqlite");
    let hard_maintenance = hardlink_dir.join("business.maintenance.sqlite");
    fs::hard_link(&config.database_path, &hard_business).unwrap();
    fs::hard_link(&maintenance_path, &hard_maintenance).unwrap();
    let mut hardlink_config = config.clone();
    hardlink_config.database_path = hard_business;
    unsafe { std::env::set_var("MAINTENANCE_DATABASE_PATH", &hard_maintenance) };
    assert!(
        crate::maintenance::MaintenanceRuntimeLock::route(&hardlink_config, true).is_err(),
        "hard-link aliases must not open a separate SQLite journal while online"
    );
    unsafe { std::env::remove_var("MAINTENANCE_DATABASE_PATH") };
    let other_database = directory.join("other.sqlite");
    let other_maintenance = directory.join("other.maintenance.sqlite");
    let mut other = RuntimeOwnerChild(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::archive_file_io::maintenance_runtime_ownership::ownership_runtime_lock_child_fixture", "--ignored"])
        .env("CVM_TEST_RUNTIME_DATABASE", &other_database)
        .env("MAINTENANCE_DATABASE_PATH", &other_maintenance)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !other_database.with_extension("ready").exists() {
            assert!(other.0.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let hard_other_maintenance = hardlink_dir.join("other.maintenance.sqlite");
    fs::hard_link(&other_maintenance, &hard_other_maintenance).unwrap();
    unsafe { std::env::set_var("MAINTENANCE_DATABASE_PATH", &hard_other_maintenance) };
    assert!(
        crate::maintenance::MaintenanceRuntimeLock::route(&hardlink_config, true).is_err(),
        "a mixed hard-link database pair must not acquire offline ownership"
    );
    unsafe { std::env::remove_var("MAINTENANCE_DATABASE_PATH") };
    let mismatched = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::archive_file_io::maintenance_runtime_ownership::ownership_runtime_lock_child_fixture", "--ignored"])
        .env("CVM_TEST_RUNTIME_DATABASE", &config.database_path)
        .env("MAINTENANCE_DATABASE_PATH", &other_maintenance)
        .env("CVM_TEST_RUNTIME_ROUTE_ASSERTION", "unavailable")
        .output().unwrap();
    assert!(
        mismatched.status.success(),
        "mixed database owners were accepted: {}",
        String::from_utf8_lossy(&mismatched.stderr)
    );
    drop(other);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(
        crate::maintenance::MaintenanceRuntimeLock::route(&hardlink_config, true).is_err(),
        "offline recovery must reject ambiguous hard-link journal ownership"
    );
    fs::remove_file(&hardlink_config.database_path).unwrap();
    fs::remove_file(&hard_maintenance).unwrap();
    fs::remove_file(&hard_other_maintenance).unwrap();
    let crate::maintenance::MaintenanceRuntimeRoute::Offline(mut recovered) =
        crate::maintenance::MaintenanceRuntimeLock::route(&config, true).unwrap()
    else {
        panic!("dead service must release the runtime");
    };
    let mut canonical_config = config.clone();
    canonical_config.database_path = alias.join("..").join("business.sqlite");
    assert!(
        crate::maintenance::MaintenanceRuntimeLock::route(&canonical_config, true).is_err(),
        "a new offline owner must clear a crashed service's ready markers before returning"
    );
    recovered
        .publish_role("service:ownership-v1:initializing")
        .unwrap();
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&canonical_config, true).is_err());
    recovered.publish_role("cli:ownership-v1").unwrap();
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&canonical_config, true).is_err());
    recovered
        .publish_role("service:ownership-v1:ready")
        .unwrap();
    assert!(matches!(
        crate::maintenance::MaintenanceRuntimeLock::route(&canonical_config, true).unwrap(),
        crate::maintenance::MaintenanceRuntimeRoute::Online(_)
    ));
    unsafe { std::env::remove_var("MAINTENANCE_DATABASE_PATH") };
    drop(recovered);
    assert!(directory.join("business.sqlite.runtime.lock").exists());
    cleanup_temp_test_dir(&directory);
}

#[tokio::test]
async fn ownership_identity_budget_closes_sqlite_worker_before_releasing_owner_fence() {
    use crate::prompt_cache_conversations::invocation_ranges::Owner;
    let directory = make_temp_test_dir("identity-budget-worker-drain");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::from_str(&test_sqlite_url_for_path(
                &directory.join("business.sqlite"),
            ))
            .unwrap()
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(1)),
        )
        .await
        .unwrap();
    ensure_schema(&pool).await.unwrap();
    sqlx::query("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES('budget-owner','KZYYYY','2020-01-01','2020-01-01')")
        .execute(&pool).await.unwrap();
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let manager = cache.lock().await.identity_cache.range_manager.clone();
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let result = with_identity_cleanup_work_budget(
        Duration::from_millis(250),
        cleanup_invocation_identities(&pool, false, &cache),
    )
    .await;
    assert!(
        result
            .expect_err("cleanup must time out")
            .to_string()
            .contains("work budget")
    );
    assert!(
        manager
            .freeze_owner(Owner::Conversation("budget-owner".to_string()), "KZYYYY")
            .is_none(),
        "SQLite worker still owns the lifecycle fence"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(fence) =
                manager.freeze_owner(Owner::Conversation("budget-owner".to_string()), "KZYYYY")
            {
                drop(fence);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='budget-owner'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    pool.close().await;
    cleanup_temp_test_dir(&directory);
}
