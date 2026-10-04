use super::*;
#[cfg(target_os = "linux")]
use std::process::Command;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio_util::sync::CancellationToken;

static TEMP_DIR_ID: AtomicU64 = AtomicU64::new(0);

struct StorageFixture(PathBuf);

impl StorageFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cvm-storage-fixture-{}-{}",
            std::process::id(),
            TEMP_DIR_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create storage fixture directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for StorageFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn system_storage_matches_gnu_du_for_configured_union_and_file_identity_deduplication() {
    use std::os::unix::fs::symlink;
    use std::process::Command;

    let fixture = StorageFixture::new();
    let data = fixture.path().join("data");
    let raw = fixture.path().join("external-raw");
    let raw_alias = fixture.path().join("raw-alias");
    let archive = raw.join("archive-subtree");
    let xray = fixture.path().join("external-xray");
    let external_db_dir = fixture.path().join("external-db");
    let performance_db = external_db_dir.join("performance.sqlite");
    let maintenance_db = external_db_dir.join("maintenance.sqlite");
    fs::create_dir_all(&data).expect("create data directory");
    fs::create_dir_all(&archive).expect("create archive directory");
    fs::create_dir_all(&xray).expect("create Xray runtime directory");
    fs::create_dir_all(&external_db_dir).expect("create external database directory");
    symlink(&raw, &raw_alias).expect("create configured raw root symlink");

    let database = data.join("main.sqlite");
    fs::write(&database, vec![1_u8; 4097]).expect("write main database");
    for suffix in ["-wal", "-shm", "-journal"] {
        fs::write(format!("{}{suffix}", database.display()), vec![2_u8; 2049])
            .expect("write main database sidecar");
    }
    let shared = data.join("shared-payload.bin");
    fs::write(&shared, vec![3_u8; 8193]).expect("write shared payload");
    fs::hard_link(&shared, raw.join("shared-payload-hard-link.bin"))
        .expect("create cross-root hard link");
    fs::write(raw.join("independent-copy.bin"), vec![4_u8; 8193]).expect("write independent copy");
    fs::write(archive.join("segment.sqlite.gz"), vec![5_u8; 4097]).expect("write archive segment");
    symlink(&data, raw.join("internal-data-link")).expect("create internal directory symlink");
    symlink(&raw, raw.join("internal-cycle")).expect("create internal cycle symlink");
    let sparse = data.join("sparse.bin");
    fs::File::create(&sparse)
        .and_then(|file| file.set_len(64 * 1024 * 1024))
        .expect("create sparse file");
    fs::write(xray.join("config.json"), vec![6_u8; 1024]).expect("write Xray config");
    fs::write(&performance_db, vec![7_u8; 4097]).expect("write external performance database");
    fs::write(&maintenance_db, vec![12_u8; 4097]).expect("write external maintenance database");
    for suffix in ["-wal", "-shm", "-journal"] {
        fs::write(
            format!("{}{suffix}", performance_db.display()),
            vec![8_u8; 1024],
        )
        .expect("write external database sidecar");
        fs::write(
            format!("{}{suffix}", maintenance_db.display()),
            vec![13_u8; 1024],
        )
        .expect("write maintenance database sidecar");
    }
    fs::write(
        external_db_dir.join("unrelated.bin"),
        vec![9_u8; 256 * 1024],
    )
    .expect("write unrelated external file");

    let mut config = test_config();
    config.database_path = database.clone();
    config.performance_database_path = performance_db.clone();
    config.proxy_raw_dir = raw_alias;
    config.archive_dir = archive.clone();
    config.xray_runtime_dir = xray.clone();

    let runtime =
        SystemStorageRuntime::new_with_maintenance_path_for_test(&config, &maintenance_db);
    runtime
        .sample_once_for_test(&CancellationToken::new())
        .await;
    let snapshot = runtime.snapshot().await;
    assert_eq!(snapshot.state, SystemStorageState::Ready);
    assert!(snapshot.sampled_at.is_some());

    let mut oracle_paths = vec![
        data,
        raw,
        archive,
        xray,
        performance_db.clone(),
        maintenance_db.clone(),
    ];
    for database in [&performance_db, &maintenance_db] {
        for suffix in ["-wal", "-shm", "-journal"] {
            oracle_paths.push(PathBuf::from(format!("{}{suffix}", database.display())));
        }
    }
    let output = Command::new("du")
        .args(["-s", "-c", "-B1"])
        .args(&oracle_paths)
        .output()
        .expect("run GNU du oracle");
    assert!(
        output.status.success(),
        "GNU du failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = String::from_utf8(output.stdout)
        .expect("decode GNU du output")
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().next())
        .and_then(|bytes| bytes.parse::<u64>().ok())
        .expect("parse GNU du total");
    assert_eq!(snapshot.total_bytes, Some(expected));
}

#[cfg(unix)]
#[test]
fn system_storage_treats_a_child_removed_during_traversal_as_concurrent_deletion() {
    use std::os::unix::fs::MetadataExt;

    let fixture = StorageFixture::new();
    let data = fixture.path().join("data");
    fs::create_dir_all(&data).expect("create data directory");
    let removed = data.join("removed-during-scan.bin");
    let retained = data.join("retained.bin");
    fs::write(&removed, vec![1_u8; 8192]).expect("write removed file");
    fs::write(&retained, vec![2_u8; 4096]).expect("write retained file");
    let mut removed_during_walk = false;

    let total = scan_paths_with_limits_and_hook_for_test(
        std::slice::from_ref(&data),
        64 * 1024 * 1024,
        128,
        |path| {
            if path == removed && !removed_during_walk {
                fs::remove_file(path).expect("delete file after directory enumeration");
                removed_during_walk = true;
            }
        },
    )
    .expect("scan succeeds after treating a vanished file as deleted");
    let expected = [data, retained]
        .into_iter()
        .map(|path| {
            fs::symlink_metadata(path)
                .expect("remaining object metadata")
                .blocks()
                * 512
        })
        .sum::<u64>();

    assert!(removed_during_walk);
    assert_eq!(total, expected);
}

#[cfg(unix)]
#[tokio::test]
async fn system_storage_retains_last_good_value_and_marks_stale_after_failure() {
    let fixture = StorageFixture::new();
    let data = fixture.path().join("data");
    fs::create_dir_all(&data).expect("create required data directory");
    fs::write(data.join("payload.bin"), vec![1_u8; 8192]).expect("write payload");
    let mut config = test_config();
    config.database_path = data.join("main.sqlite");
    config.performance_database_path = fixture.path().join("performance.sqlite");
    config.proxy_raw_dir = fixture.path().join("missing-raw");
    config.archive_dir = fixture.path().join("missing-archive");
    config.xray_runtime_dir = fixture.path().join("missing-xray");
    let maintenance_db = data.join("maintenance.sqlite");
    let runtime =
        SystemStorageRuntime::new_with_maintenance_path_for_test(&config, &maintenance_db);

    runtime
        .sample_once_for_test(&CancellationToken::new())
        .await;
    let successful = runtime.snapshot().await;
    assert_eq!(successful.state, SystemStorageState::Ready);
    let bytes = successful.total_bytes.expect("successful byte total");
    let sampled_at = successful.sampled_at.clone().expect("sample timestamp");

    fs::remove_dir_all(&data).expect("remove required root to inject failure");
    runtime
        .sample_once_for_test(&CancellationToken::new())
        .await;
    let failed = runtime.snapshot().await;
    assert_eq!(failed.state, SystemStorageState::Error);
    assert_eq!(failed.reason.as_deref(), Some("not_found"));
    assert_eq!(failed.total_bytes, Some(bytes));
    assert_eq!(failed.sampled_at.as_deref(), Some(sampled_at.as_str()));

    runtime
        .set_last_good_for_test(bytes, Utc::now() - chrono::Duration::seconds(61))
        .await;
    assert!(runtime.snapshot().await.stale);
}

#[cfg(unix)]
#[test]
fn system_storage_resource_budget_defers_without_returning_a_partial_total() {
    let fixture = StorageFixture::new();
    fs::write(fixture.path().join("payload"), vec![1_u8; 4096]).expect("write payload");

    assert_eq!(
        scan_paths_with_limits_for_test(&[fixture.path().to_path_buf()], 1, 128)
            .expect_err("tiny memory budget must defer"),
        "resource_limit"
    );
}

#[cfg(unix)]
#[test]
fn system_storage_depth_limit_applies_to_files_and_directories() {
    let fixture = StorageFixture::new();
    fs::write(fixture.path().join("payload"), b"x").expect("write payload");

    assert_eq!(
        scan_paths_with_limits_for_test(&[fixture.path().to_path_buf()], 1024, 0)
            .expect_err("child file beyond the depth limit must defer"),
        "resource_limit"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn system_storage_shutdown_cancels_worker_and_keeps_single_flight_permit_until_exit() {
    let fixture = StorageFixture::new();
    let data = fixture.path().join("data");
    fs::create_dir_all(&data).expect("create data directory");
    for index in 0..10_000 {
        fs::write(data.join(format!("entry-{index}")), b"x").expect("create scan entry");
    }
    let mut config = test_config();
    config.database_path = data.join("main.sqlite");
    config.performance_database_path = fixture.path().join("performance.sqlite");
    config.proxy_raw_dir = fixture.path().join("missing-raw");
    config.archive_dir = fixture.path().join("missing-archive");
    config.xray_runtime_dir = fixture.path().join("missing-xray");
    let maintenance_db = data.join("maintenance.sqlite");
    let runtime = Arc::new(SystemStorageRuntime::new_with_maintenance_path_for_test(
        &config,
        &maintenance_db,
    ));
    let shutdown = CancellationToken::new();
    let worker_runtime = runtime.clone();
    let worker_shutdown = shutdown.clone();
    let worker = tokio::spawn(async move {
        worker_runtime.sample_once_for_test(&worker_shutdown).await;
    });

    while !runtime.snapshot().await.scan_in_progress {
        tokio::task::yield_now().await;
    }
    runtime
        .sample_once_for_test(&CancellationToken::new())
        .await;
    shutdown.cancel();
    worker.await.expect("join cancelled scanner");

    let snapshot = runtime.snapshot().await;
    assert!(!snapshot.scan_in_progress);
    assert_eq!(snapshot.reason.as_deref(), Some("cancelled"));
    assert_eq!(runtime.scan_available_permits_for_test(), 1);
}
