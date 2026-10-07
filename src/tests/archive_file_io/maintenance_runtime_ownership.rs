use super::*;

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
    owner.publish_role("service:ownership-v1:ready").unwrap();
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
    assert!(matches!(
        crate::maintenance::MaintenanceRuntimeLock::route(&config, true).unwrap(),
        crate::maintenance::MaintenanceRuntimeRoute::Online
    ));
    assert!(crate::maintenance::MaintenanceRuntimeLock::route(&config, false).is_err());
    let alias = directory.join("alias");
    fs::create_dir_all(&alias).unwrap();
    let mut alias_config = config.clone();
    alias_config.database_path = alias.join("..").join("business.sqlite");
    assert!(matches!(
        crate::maintenance::MaintenanceRuntimeLock::route(&alias_config, true).unwrap(),
        crate::maintenance::MaintenanceRuntimeRoute::Online
    ));
    let hardlink_dir = directory.join("hardlinks");
    fs::create_dir_all(&hardlink_dir).unwrap();
    fs::write(&config.database_path, b"business").unwrap();
    let maintenance_path = config.maintenance_database_path();
    fs::write(&maintenance_path, b"maintenance").unwrap();
    let hard_business = hardlink_dir.join("business.sqlite");
    let hard_maintenance = hardlink_dir.join("business.maintenance.sqlite");
    fs::hard_link(&config.database_path, &hard_business).unwrap();
    fs::hard_link(&maintenance_path, &hard_maintenance).unwrap();
    let mut hardlink_config = config.clone();
    hardlink_config.database_path = hard_business;
    unsafe { std::env::set_var("MAINTENANCE_DATABASE_PATH", &hard_maintenance) };
    assert!(matches!(
        crate::maintenance::MaintenanceRuntimeLock::route(&hardlink_config, true).unwrap(),
        crate::maintenance::MaintenanceRuntimeRoute::Online
    ));
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
    let route = crate::maintenance::MaintenanceRuntimeLock::route(&config, true).unwrap();
    assert!(matches!(
        route,
        crate::maintenance::MaintenanceRuntimeRoute::Offline(_)
    ));
    drop(route);
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
