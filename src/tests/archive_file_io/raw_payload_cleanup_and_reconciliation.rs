use super::*;

#[tokio::test]
async fn legacy_retention_reconciliation_skips_archive_io_when_pressure_gate_is_busy() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-pressure-preflight").await;
    let archive_dir = config
        .archive_dir
        .join("codex_invocations")
        .join("2026")
        .join("01")
        .join("01");
    fs::create_dir_all(&archive_dir).expect("create legacy archive directory");
    fs::write(
        archive_dir.join("part-0000000000000000-0000000000000000-legacy.sqlite.gz"),
        b"not a gzip archive",
    )
    .expect("write legacy archive candidate");

    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        std::time::Duration::from_secs(60),
    ));
    let _busy_permit = pressure_gate
        .try_begin_background("retention_legacy_pressure_preflight")
        .expect("occupy the test background pressure slot");
    let directory_traversal = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let directory_entries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let archive_io = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let result = crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE
        .scope(
            pressure_gate,
            crate::maintenance::RETENTION_TEST_LEGACY_DIRECTORY_TRAVERSAL.scope(
                directory_traversal.clone(),
                crate::maintenance::RETENTION_TEST_LEGACY_DIRECTORY_ENTRIES.scope(
                    directory_entries.clone(),
                    crate::maintenance::RETENTION_TEST_LEGACY_ARCHIVE_IO.scope(
                        archive_io.clone(),
                        crate::maintenance::reconcile_legacy_retention_archive_segments(
                            &pool, &config,
                        ),
                    ),
                ),
            ),
        )
        .await;

    let error = result.expect_err("busy pressure must defer legacy reconciliation");
    assert!(error.to_string().contains("retention write deferred"));
    assert_eq!(
        directory_traversal.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        directory_entries.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(archive_io.load(std::sync::atomic::Ordering::Relaxed), 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_retention_reconciliation_processes_at_most_32_files_per_pass() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-scan-bound").await;
    let archive_dir = config
        .archive_dir
        .join("codex_invocations")
        .join("2026")
        .join("01")
        .join("01");
    fs::create_dir_all(&archive_dir).expect("create legacy segment directory");
    let archive_paths = (0..33)
        .map(|index| archive_dir.join(format!("part-{index:016x}-{index:016x}-legacy.sqlite.gz")))
        .collect::<Vec<_>>();
    for path in archive_paths.iter().rev() {
        fs::write(path, b"not a gzip archive").expect("write unverified legacy segment");
    }
    let selected_forward = crate::maintenance::retention_test_select_bounded_archive_paths(
        archive_paths.clone(),
        "",
        32,
    );
    let mut archive_paths_reversed = archive_paths.clone();
    archive_paths_reversed.reverse();
    let selected_reversed = crate::maintenance::retention_test_select_bounded_archive_paths(
        archive_paths_reversed,
        "",
        32,
    );
    assert_eq!(selected_forward, selected_reversed);

    let heap_peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let heap_live = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    crate::maintenance::RETENTION_TEST_LEGACY_DIRECTORY_ENTRIES
        .scope(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            crate::maintenance::RETENTION_TEST_LEGACY_DIRECTORY_HEAP_PEAK.scope(
                heap_peak.clone(),
                crate::maintenance::RETENTION_TEST_LEGACY_DIRECTORY_HEAP_LIVE.scope(
                    heap_live.clone(),
                    run_data_retention_maintenance(&pool, &config, Some(false), None),
                ),
            ),
        )
        .await
        .expect("run bounded legacy reconciliation pass");
    assert!(
        heap_peak.load(std::sync::atomic::Ordering::Relaxed) <= 32,
        "legacy discovery must retain no more than the 32-candidate heap bound"
    );
    assert_eq!(
        heap_peak.load(std::sync::atomic::Ordering::Relaxed),
        32,
        "legacy discovery should exercise the full bounded selection heap"
    );
    assert_eq!(heap_live.load(std::sync::atomic::Ordering::Relaxed), 0);
    let first_pass_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE state = 'quarantined'",
    )
    .fetch_one(&pool)
    .await
    .expect("count first-pass quarantined archives");
    assert_eq!(first_pass_count, 32);
    let first_pass_cursor: String = sqlx::query_scalar(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("load first-pass legacy cursor");
    assert!(first_pass_cursor.ends_with("part-000000000000001f-000000000000001f-legacy.sqlite.gz"));

    sqlx::query(
        "UPDATE retention_recovery_cursors SET next_retry_at = datetime('now', '-1 second') \
         WHERE scope = 'legacy_archive_segments'",
    )
    .execute(&pool)
    .await
    .expect("make legacy recovery cursor due for the next bounded pass");
    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("resume bounded legacy reconciliation pass");
    let second_pass_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE state = 'quarantined'",
    )
    .fetch_one(&pool)
    .await
    .expect("count resumed quarantined archives");
    assert_eq!(second_pass_count, 33);

    let late_file = archive_dir.join("aaa-late.sqlite.gz");
    fs::write(&late_file, b"late unverified legacy segment")
        .expect("write lexically earlier segment after the cursor advanced");
    sqlx::query(
        "UPDATE retention_recovery_cursors SET next_retry_at = datetime('now', '-1 second') \
         WHERE scope = 'legacy_archive_segments'",
    )
    .execute(&pool)
    .await
    .expect("make legacy recovery cursor due before wrapping");
    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("tail scan should wrap the cursor after exhaustion");
    let cursor_after_wrap: String = sqlx::query_scalar(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("load wrapped legacy cursor");
    assert!(cursor_after_wrap.is_empty());

    sqlx::query(
        "UPDATE retention_recovery_cursors SET next_retry_at = datetime('now', '-1 second') \
         WHERE scope = 'legacy_archive_segments'",
    )
    .execute(&pool)
    .await
    .expect("make legacy recovery cursor due after wrapping");
    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("scan again from the beginning after cursor wrap");
    let late_file_is_tracked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE file_path = ?1 AND state = 'quarantined'",
    )
    .bind(late_file.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("check the late legacy file was quarantined");
    assert_eq!(late_file_is_tracked, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_retention_reconciliation_does_not_starve_after_a_truncated_referenced_window() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-truncated-referenced-window")
            .await;
    let archive_dir = config
        .archive_dir
        .join("codex_invocations")
        .join("2026")
        .join("01")
        .join("01");
    fs::create_dir_all(&archive_dir).expect("create legacy segment directory");
    for index in 0..31 {
        let path = archive_dir.join(format!(
            "part-{index:016x}-{index:016x}-referenced.sqlite.gz"
        ));
        fs::write(&path, b"referenced legacy archive").expect("write referenced archive");
        sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, created_at) \
             VALUES ('codex_invocations', '2026-01', ?1, 'referenced', 1, ?2, datetime('now'))",
        )
        .bind(path.to_string_lossy().to_string())
        .bind(ARCHIVE_STATUS_COMPLETED)
        .execute(&pool)
        .await
        .expect("seed referenced archive manifest");
    }
    fs::write(
        archive_dir.join("part-000000000000001f-noise.txt"),
        b"not an archive candidate",
    )
    .expect("write non-candidate directory entry");
    let unreferenced =
        archive_dir.join("part-ffffffffffffffff-ffffffffffffffff-unreferenced.sqlite.gz");
    fs::write(&unreferenced, b"unreferenced legacy archive").expect("write unreferenced archive");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run first truncated legacy reconciliation pass");
    let first_pass_unreferenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_prepared_archives WHERE file_path = ?1")
            .bind(unreferenced.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("count unreferenced archive after first pass");
    assert_eq!(first_pass_unreferenced, 0);

    sqlx::query(
        "UPDATE retention_recovery_cursors SET next_retry_at = datetime('now', '-1 second') \
         WHERE scope = 'legacy_archive_segments'",
    )
    .execute(&pool)
    .await
    .expect("make legacy recovery cursor due before the resumed pass");
    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("resume after truncated legacy reconciliation pass");
    let second_pass_unreferenced: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives \
         WHERE file_path = ?1 AND state = 'quarantined'",
    )
    .bind(unreferenced.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count quarantined unreferenced archive after resume");
    assert_eq!(second_pass_unreferenced, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_retention_reconciliation_reaches_siblings_after_a_truncated_directory() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-truncated-directory-sibling")
            .await;
    let archive_dir = config
        .archive_dir
        .join("codex_invocations")
        .join("2026")
        .join("01")
        .join("01");
    fs::create_dir_all(archive_dir.join("a")).expect("create first empty legacy directory");
    fs::create_dir_all(archive_dir.join("z")).expect("create later legacy directory");
    for index in 0..31 {
        fs::write(
            archive_dir.join(format!("c{index:02}-noise.txt")),
            b"not an archive candidate",
        )
        .expect("write sibling selection noise");
    }
    let sibling_archive = archive_dir
        .join("z")
        .join("part-0000000000000000-0000000000000000-sibling.sqlite.gz");
    fs::write(&sibling_archive, b"unreferenced sibling archive")
        .expect("write later sibling archive");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run truncated parent-directory reconciliation pass");
    let sibling_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM retention_prepared_archives WHERE file_path = ?1")
            .bind(sibling_archive.to_string_lossy().to_string())
            .fetch_optional(&pool)
            .await
            .expect("load later sibling archive state");
    assert_eq!(sibling_state.as_deref(), Some("quarantined"));

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_retention_reconciliation_keeps_same_prefix_files_after_directory_traversal() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-same-prefix-file").await;
    let archive_dir = config
        .archive_dir
        .join("codex_invocations")
        .join("2026")
        .join("01")
        .join("01");
    fs::create_dir_all(archive_dir.join("a")).expect("create same-prefix legacy directory");
    for index in 0..32 {
        fs::write(
            archive_dir
                .join("a")
                .join(format!("part-{index:016x}-{index:016x}-legacy.sqlite.gz")),
            b"unreferenced nested archive",
        )
        .expect("write nested legacy archive");
    }
    let same_prefix_file = archive_dir.join("a.sqlite.gz");
    fs::write(&same_prefix_file, b"unreferenced same-prefix archive")
        .expect("write same-prefix legacy archive");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run same-prefix legacy reconciliation pass");
    let tracked_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM retention_prepared_archives WHERE file_path = ?1")
            .bind(same_prefix_file.to_string_lossy().to_string())
            .fetch_optional(&pool)
            .await
            .expect("load same-prefix legacy archive state");
    assert_eq!(tracked_state.as_deref(), Some("quarantined"));

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_retention_cursor_advance_is_monotonic_for_a_stale_writer() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-legacy-cursor-monotonic").await;
    let high = temp_dir.join("archives/codex_invocations/2026/01/01/part-z.sqlite.gz");
    let low = temp_dir.join("archives/codex_invocations/2026/01/01/part-a.sqlite.gz");
    sqlx::query(
        "UPDATE retention_recovery_cursors \
         SET cursor = ?1, updated_at = datetime('now') \
         WHERE scope = 'legacy_archive_segments'",
    )
    .bind(high.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed advanced legacy cursor");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator,
            crate::maintenance::advance_retention_recovery_cursor(
                &pool,
                &high.to_string_lossy(),
                &low.to_string_lossy(),
            ),
        )
        .await
        .expect("stale cursor writer should complete");
    let stored: String = sqlx::query_scalar(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("load monotonic legacy cursor");
    assert_eq!(stored, high.to_string_lossy());

    sqlx::query(
        "UPDATE retention_recovery_cursors SET cursor = '' \
         WHERE scope = 'legacy_archive_segments' AND cursor = ?1",
    )
    .bind(high.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("simulate a completed cursor wrap");
    let later_progress = temp_dir.join("archives/codex_invocations/2026/01/01/part-z2.sqlite.gz");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator,
            crate::maintenance::advance_retention_recovery_cursor(
                &pool,
                &high.to_string_lossy(),
                &later_progress.to_string_lossy(),
            ),
        )
        .await
        .expect("stale pre-wrap cursor writer should complete");
    let after_wrap_stale_write: String = sqlx::query_scalar(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("load cursor after stale pre-wrap write");
    assert!(after_wrap_stale_write.is_empty());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test(flavor = "current_thread")]
async fn retention_orphan_sweep_is_root_anchored_and_bounded() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("retention-orphan-db-parent");
    let db_root = temp_dir.join("db-root");
    let cwd_root = temp_dir.join("cwd-root");
    fs::create_dir_all(&db_root).expect("create db root");
    fs::create_dir_all(&cwd_root).expect("create cwd root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd_root);

    let db_path = db_root.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create sqlite file");
    let pool = SqlitePool::connect(&test_sqlite_url_for_path(&db_path))
        .await
        .expect("connect retention sqlite");
    ensure_schema(&pool).await.expect("ensure retention schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let anchored_dir = config.resolved_proxy_raw_dir();
    fs::create_dir_all(&anchored_dir).expect("create anchored raw dir");
    let anchored_orphan = anchored_dir.join("anchored-orphan.bin");
    fs::write(&anchored_orphan, b"anchored-orphan").expect("write anchored orphan");
    set_file_mtime_seconds_ago(&anchored_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let cwd_raw_dir = cwd_root.join("proxy_raw_payloads");
    fs::create_dir_all(&cwd_raw_dir).expect("create cwd raw dir");
    let cwd_orphan = cwd_raw_dir.join("cwd-orphan.bin");
    fs::write(&cwd_orphan, b"cwd-orphan").expect("write cwd orphan");
    set_file_mtime_seconds_ago(&cwd_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("run orphan sweep");

    assert_eq!(removed, 0);
    assert!(
        anchored_orphan.exists(),
        "unlinked raw residuals remain in place"
    );
    assert!(
        cwd_orphan.exists(),
        "orphan sweep should stop scanning cwd-relative stray files"
    );
    let quarantined: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(anchored_orphan.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count anchored raw quarantine row");
    assert_eq!(quarantined, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_raw_reconciliation_survives_reopen_and_releases_unassociated_file() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-reconcile-restart").await;
    let raw_path = config.proxy_raw_dir.join("restart-orphan.bin");
    fs::write(&raw_path, b"restart-orphan").expect("write restart orphan");

    let first = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("record raw quarantine");
    assert_eq!(first, 0);
    let raw_path_string = raw_path.to_string_lossy().to_string();
    let first_identity: String = sqlx::query_scalar(
        "SELECT file_identity FROM retention_raw_reconciliation WHERE raw_path = ?1",
    )
    .bind(&raw_path_string)
    .fetch_one(&pool)
    .await
    .expect("load durable raw identity");
    pool.close().await;

    let reopened = SqlitePool::connect(&test_sqlite_url_for_path(&config.database_path))
        .await
        .expect("reopen retention sqlite");
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS as i64 + 1),
        ))
        .bind(&raw_path_string)
        .execute(&reopened)
        .await
        .expect("age durable quarantine row");
    sqlx::query(
        "UPDATE retention_recovery_cursors SET cursor = '' WHERE scope = 'raw_payload_files'",
    )
    .execute(&reopened)
    .await
    .expect("rewind raw cursor for interrupted pass");

    let removed = sweep_orphan_proxy_raw_files(&reopened, &config, None, false)
        .await
        .expect("resume raw reconciliation after reopen");
    assert_eq!(removed, 1);
    assert!(!raw_path.exists());
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(&raw_path_string)
            .fetch_one(&reopened)
            .await
            .expect("count released raw quarantine row");
    assert_eq!(remaining, 0);
    let (inventory_state, inventory_recheck_active): (String, i64) = sqlx::query_as(
        "SELECT inventory_state, inventory_recheck_active FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&reopened)
    .await
    .expect("load raw inventory reset state after unlink");
    assert_eq!(inventory_state, "preparing");
    assert_eq!(inventory_recheck_active, 1);
    assert!(!first_identity.is_empty());

    reopened.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_reset_intent_survives_unlink_failure_before_file_release() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-reset-before-unlink").await;
    let raw_path = config.proxy_raw_dir.join("unlink-failure-orphan.bin");
    fs::write(&raw_path, b"unlink-failure-orphan").expect("write raw orphan");
    let mut traversal = RetentionRawDirectoryTraversal::default();
    let observed = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("record raw quarantine");
    assert_eq!(observed.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("expire raw quarantine");

    let fail_unlink = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut retry_traversal = RetentionRawDirectoryTraversal::default();
    let pass = crate::maintenance::RETENTION_TEST_RAW_UNLINK_FAILURE
        .scope(
            fail_unlink,
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut retry_traversal),
        )
        .await
        .expect("retain orphan after simulated unlink failure");

    assert_eq!(pass.removed, 0);
    assert_eq!(pass.failures, 1);
    assert!(!pass.complete);
    assert!(raw_path.exists());
    let (inventory_state, inventory_recheck_active): (String, i64) = sqlx::query_as(
        "SELECT inventory_state, inventory_recheck_active FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("read durable inventory reset intent");
    assert_eq!(inventory_state, "preparing");
    assert_eq!(inventory_recheck_active, 1);
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("retain raw quarantine ledger after failed unlink");
    assert_eq!(ledger_rows, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_persists_removal_evidence_before_ledger_cleanup() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-removal-evidence").await;
    let raw_path = config
        .proxy_raw_dir
        .join("ledger-cleanup-failure-orphan.bin");
    let payload = b"ledger-cleanup-failure-orphan";
    fs::write(&raw_path, payload).expect("write raw orphan");
    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let initial =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial_traversal)
            .await
            .expect("record raw quarantine");
    assert_eq!(initial.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("expire raw quarantine");

    let fail_cleanup = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut release_traversal = RetentionRawDirectoryTraversal::default();
    let pass = crate::maintenance::RETENTION_TEST_RAW_LEDGER_CLEANUP_FAILURE
        .scope(
            fail_cleanup,
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut release_traversal),
        )
        .await
        .expect("retain removal evidence after ledger cleanup failure");

    assert_eq!(pass.removed, 1);
    assert_eq!(pass.removed_bytes, payload.len() as u64);
    assert_eq!(pass.failures, 1);
    assert!(!pass.complete);
    assert!(!raw_path.exists());
    let (last_removed, last_removed_bytes, last_removed_at): (
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT last_nonzero_removal, last_nonzero_removal_bytes, last_nonzero_removal_at \
             FROM retention_recovery_cursors WHERE scope = 'raw_payload_files'",
    )
    .fetch_one(&pool)
    .await
    .expect("load durable removal evidence");
    assert_eq!(last_removed, Some(1));
    assert_eq!(last_removed_bytes, Some(payload.len() as i64));
    assert!(last_removed_at.is_some());
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("retain raw quarantine ledger after cleanup failure");
    assert_eq!(ledger_rows, 1);

    let mut retry_traversal = RetentionRawDirectoryTraversal::default();
    let retry =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut retry_traversal)
            .await
            .expect("clear pending raw quarantine ledger without duplicate evidence");
    assert_eq!(retry.removed, 0);
    let (retry_last_removed, retry_last_removed_bytes, retry_last_removed_at): (
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT last_nonzero_removal, last_nonzero_removal_bytes, last_nonzero_removal_at \
             FROM retention_recovery_cursors WHERE scope = 'raw_payload_files'",
    )
    .fetch_one(&pool)
    .await
    .expect("reload durable removal evidence after ledger retry");
    assert_eq!(retry_last_removed, last_removed);
    assert_eq!(retry_last_removed_bytes, last_removed_bytes);
    assert_eq!(retry_last_removed_at, last_removed_at);
    let retry_ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("clear raw quarantine after durable evidence retry");
    assert_eq!(retry_ledger_rows, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_recovers_when_removal_evidence_write_fails() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-evidence-retry").await;
    let raw_path = config
        .proxy_raw_dir
        .join("evidence-write-failure-orphan.bin");
    let payload = b"evidence-write-failure-orphan";
    fs::write(&raw_path, payload).expect("write raw orphan");
    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let initial =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial_traversal)
            .await
            .expect("record raw quarantine");
    assert_eq!(initial.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("expire raw quarantine");

    let fail_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut failed_traversal = RetentionRawDirectoryTraversal::default();
    let failed = crate::maintenance::RETENTION_TEST_RAW_REMOVAL_EVIDENCE_FAILURE
        .scope(
            fail_once,
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut failed_traversal),
        )
        .await
        .expect("retain pending release after evidence failure");
    assert_eq!(failed.removed, 1);
    assert_eq!(failed.failures, 1);
    assert!(!failed.complete);
    assert!(!raw_path.exists());
    let release_pending: i64 = sqlx::query_scalar(
        "SELECT release_pending FROM retention_raw_reconciliation WHERE raw_path = ?1",
    )
    .bind(raw_path.to_string_lossy().as_ref())
    .fetch_one(&pool)
    .await
    .expect("retain pending release marker");
    assert_eq!(release_pending, 1);

    let mut retry_traversal = RetentionRawDirectoryTraversal::default();
    let recovered =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut retry_traversal)
            .await
            .expect("recover removal evidence from pending release");
    assert_eq!(recovered.failures, 0);
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("clear recovered raw quarantine ledger");
    assert_eq!(ledger_rows, 0);
    let (last_removed, last_removed_bytes): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT last_nonzero_removal, last_nonzero_removal_bytes \
         FROM retention_recovery_cursors WHERE scope = 'raw_payload_files'",
    )
    .fetch_one(&pool)
    .await
    .expect("load recovered removal evidence");
    assert_eq!(last_removed, Some(1));
    assert_eq!(last_removed_bytes, Some(payload.len() as i64));

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_does_not_claim_pending_removal_after_identity_replacement() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pending-replacement").await;
    let raw_path = config
        .proxy_raw_dir
        .join("pending-identity-replacement-orphan.bin");
    let old_payload = b"pending-old";
    let new_payload = b"pending-new-payload";
    fs::write(&raw_path, old_payload).expect("write old raw orphan");
    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let initial =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial_traversal)
            .await
            .expect("record raw quarantine");
    assert_eq!(initial.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("expire raw quarantine");

    let fail_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut failed_traversal = RetentionRawDirectoryTraversal::default();
    let failed = crate::maintenance::RETENTION_TEST_RAW_UNLINK_FAILURE
        .scope(
            fail_once,
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut failed_traversal),
        )
        .await
        .expect("retain pending release after unlink failure");
    assert_eq!(failed.removed, 0);
    assert_eq!(failed.failures, 1);
    assert!(raw_path.exists());

    fs::remove_file(&raw_path).expect("replace pending raw file");
    fs::write(&raw_path, new_payload).expect("write replacement raw orphan");
    let mut replacement_traversal = RetentionRawDirectoryTraversal::default();
    let replacement =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut replacement_traversal)
            .await
            .expect("reset pending ledger after replacing identity");
    assert_eq!(replacement.quarantined, 1);
    let (last_removed, last_removed_bytes): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT last_nonzero_removal, last_nonzero_removal_bytes \
         FROM retention_recovery_cursors WHERE scope = 'raw_payload_files'",
    )
    .fetch_one(&pool)
    .await
    .expect("load replacement removal evidence");
    assert_eq!(last_removed, None);
    assert_eq!(last_removed_bytes, None);
    let (release_pending, evidence_persisted, byte_size): (i64, i64, i64) = sqlx::query_as(
        "SELECT release_pending, removal_evidence_persisted, byte_size \
         FROM retention_raw_reconciliation WHERE raw_path = ?1",
    )
    .bind(raw_path.to_string_lossy().as_ref())
    .fetch_one(&pool)
    .await
    .expect("load replacement reconciliation row");
    assert_eq!(release_pending, 0);
    assert_eq!(evidence_persisted, 0);
    assert_eq!(byte_size, new_payload.len() as i64);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test]
async fn raw_orphan_sweep_does_not_claim_pending_removal_for_non_regular_replacement() {
    use std::os::unix::fs::symlink;

    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pending-symlink").await;
    let raw_path = config
        .proxy_raw_dir
        .join("pending-non-regular-replacement.bin");
    let target_path = temp_dir.join("pending-symlink-target.bin");
    fs::write(&target_path, b"replacement-target").expect("write symlink target");
    symlink(&target_path, &raw_path).expect("replace pending raw file with symlink");
    sqlx::query(
        "INSERT INTO retention_raw_reconciliation
         (raw_path, file_identity, byte_size, quarantined_at, release_pending,
          release_pending_at, removal_evidence_persisted, updated_at)
         VALUES (?1, 'old-identity', 17, ?2, 1, ?2, 0, '2026-01-01 00:00:00')",
    )
    .bind(raw_path.to_string_lossy().as_ref())
    .bind(format_utc_iso(Utc::now() - ChronoDuration::days(2)))
    .execute(&pool)
    .await
    .expect("insert pending non-regular row");

    let mut traversal = RetentionRawDirectoryTraversal::default();
    let pass = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("clear non-regular pending row");
    assert_eq!(pass.failures, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1",
        )
        .bind(raw_path.to_string_lossy().as_ref())
        .fetch_one(&pool)
        .await
        .expect("count cleared non-regular row"),
        0
    );
    let last_removed: Option<i64> = sqlx::query_scalar(
        "SELECT last_nonzero_removal FROM retention_recovery_cursors WHERE scope = 'raw_payload_files'",
    )
    .fetch_one(&pool)
    .await
    .expect("load non-regular removal evidence");
    assert_eq!(last_removed, None);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_rotates_present_pending_rows_before_missing_rows() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pending-rotation").await;
    let old_quarantine = format_utc_iso(Utc::now() - ChronoDuration::days(2));
    let outside_root = (0..8)
        .map(|index| temp_dir.join(format!("pending-present-{index}.bin")))
        .collect::<Vec<_>>();
    for path in &outside_root {
        fs::write(path, b"pending-present").expect("write present pending raw file");
    }
    let missing_path = temp_dir.join("pending-missing.bin");
    for (index, path) in outside_root.iter().enumerate() {
        sqlx::query(
            "INSERT INTO retention_raw_reconciliation
             (raw_path, file_identity, byte_size, quarantined_at, release_pending,
              release_pending_at, removal_evidence_persisted, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, 0, ?6)",
        )
        .bind(path.to_string_lossy().as_ref())
        .bind(format!("present-identity-{index}"))
        .bind(15_i64)
        .bind(&old_quarantine)
        .bind(&old_quarantine)
        .bind("2026-01-01 00:00:00")
        .execute(&pool)
        .await
        .expect("insert present pending raw row");
    }
    sqlx::query(
        "INSERT INTO retention_raw_reconciliation
         (raw_path, file_identity, byte_size, quarantined_at, release_pending,
          release_pending_at, removal_evidence_persisted, updated_at)
         VALUES (?1, 'missing-identity', 17, ?2, 1, ?2, 0, '2026-01-02 00:00:00')",
    )
    .bind(missing_path.to_string_lossy().as_ref())
    .bind(&old_quarantine)
    .execute(&pool)
    .await
    .expect("insert missing pending raw row");

    let mut first_traversal = RetentionRawDirectoryTraversal::default();
    let first =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut first_traversal)
            .await
            .expect("rotate first pending batch");
    assert_eq!(first.reconciliation_rows_checked, 8);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1",
        )
        .bind(missing_path.to_string_lossy().as_ref())
        .fetch_one(&pool)
        .await
        .expect("keep missing pending row for next batch"),
        1
    );
    let rotated_present: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_raw_reconciliation
         WHERE release_pending != 0 AND updated_at > '2026-01-02 00:00:00'",
    )
    .fetch_one(&pool)
    .await
    .expect("count rotated pending rows");
    assert_eq!(rotated_present, 8);

    let mut second_traversal = RetentionRawDirectoryTraversal::default();
    let second =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut second_traversal)
            .await
            .expect("service missing pending row after rotation");
    assert_eq!(second.reconciliation_rows_checked, 8);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1",
        )
        .bind(missing_path.to_string_lossy().as_ref())
        .fetch_one(&pool)
        .await
        .expect("clear missing pending row"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM retention_raw_reconciliation WHERE release_pending != 0",
        )
        .fetch_one(&pool)
        .await
        .expect("retain present pending rows"),
        8
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_continues_after_one_unlink_failure() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-sweep-item-failure").await;
    let raw_paths = [
        config.proxy_raw_dir.join("item-failure-a.bin"),
        config.proxy_raw_dir.join("item-failure-b.bin"),
    ];
    for path in &raw_paths {
        fs::write(path, b"orphan").expect("write orphan candidate");
    }

    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let initial =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial_traversal)
            .await
            .expect("quarantine both orphan candidates");
    assert_eq!(initial.quarantined, 2);
    sqlx::query(
        "UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path IN (?2, ?3)",
    )
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
    ))
    .bind(raw_paths[0].to_string_lossy().as_ref())
    .bind(raw_paths[1].to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("expire both orphan quarantines");

    let fail_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut release_traversal = RetentionRawDirectoryTraversal::default();
    let pass = crate::maintenance::RETENTION_TEST_RAW_UNLINK_FAILURE
        .scope(
            fail_once,
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut release_traversal),
        )
        .await
        .expect("continue after a per-item unlink failure");

    assert_eq!(pass.failures, 1);
    assert_eq!(pass.removed, 1);
    assert!(!pass.complete);
    assert_eq!(raw_paths.iter().filter(|path| path.exists()).count(), 1);
    let remaining_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation")
            .fetch_one(&pool)
            .await
            .expect("count retained failure ledger");
    assert_eq!(remaining_rows, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_writer_fences_allow_concurrent_writers_and_block_sweep_lock() {
    let temp_dir = make_temp_test_dir("retention-raw-writer-fence");
    let raw_dir = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&raw_dir).expect("create raw directory");
    let raw_path = raw_dir.join("writer-fence.bin");

    let first_fence = crate::maintenance::acquire_retention_raw_write_fence(&raw_path)
        .expect("acquire first raw writer fence");
    let second_fence = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        tokio::task::spawn_blocking({
            let raw_path = raw_path.clone();
            move || crate::maintenance::acquire_retention_raw_write_fence(&raw_path)
        }),
    )
    .await
    .expect("shared raw writer fences must not deadlock")
    .expect("join second raw writer fence")
    .expect("acquire second raw writer fence");

    let exclusive_while_writers_active =
        crate::maintenance::retention_try_archive_locks_scope(async {
            crate::maintenance::retention_archive_file_lock(&raw_path)
        })
        .await;
    assert!(
        exclusive_while_writers_active.is_err(),
        "sweep lock must defer while raw writers retain their fences"
    );

    drop(second_fence);
    drop(first_fence);
    let exclusive_after_writers_finish =
        crate::maintenance::retention_try_archive_locks_scope(async {
            crate::maintenance::retention_archive_file_lock(&raw_path)
        })
        .await;
    assert!(exclusive_after_writers_finish.is_ok());

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_raw_deletion_plan_defers_while_writer_holds_fence() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-delete-fence").await;
    let raw_path = config.proxy_raw_dir.join("delete-fence.bin");
    fs::write(&raw_path, b"old").expect("write raw candidate");
    let _writer_fence = crate::maintenance::acquire_retention_raw_write_fence(&raw_path)
        .expect("hold raw writer fence");
    let mut connection = pool.acquire().await.expect("acquire retention connection");
    let plan = crate::maintenance::filter_unreferenced_proxy_raw_paths(
        &mut connection,
        &[Some(raw_path.to_string_lossy().into_owned())],
        None,
    )
    .await
    .expect("build deferred raw deletion plan");
    assert_eq!(
        plan.paths,
        vec![Some(raw_path.to_string_lossy().into_owned())]
    );
    assert!(plan.identities.is_empty());
    let removed =
        crate::maintenance::delete_proxy_raw_paths(plan, None).expect("skip deferred raw deletion");
    assert_eq!(removed, 0);
    assert!(raw_path.exists());

    drop(_writer_fence);
    drop(connection);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_raw_deletion_plan_preserves_replaced_identity() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-delete-identity").await;
    let raw_path = config.proxy_raw_dir.join("delete-identity.bin");
    fs::write(&raw_path, b"old").expect("write raw candidate");
    let mut connection = pool.acquire().await.expect("acquire retention connection");
    let plan = crate::maintenance::filter_unreferenced_proxy_raw_paths(
        &mut connection,
        &[Some(raw_path.to_string_lossy().into_owned())],
        None,
    )
    .await
    .expect("build identity-bound raw deletion plan");
    fs::write(&raw_path, b"replacement-with-new-identity")
        .expect("replace raw candidate after identity check");
    let removed =
        crate::maintenance::delete_proxy_raw_paths(plan, None).expect("skip replaced raw deletion");
    assert_eq!(removed, 0);
    assert_eq!(
        fs::read(&raw_path).expect("read replacement raw file"),
        b"replacement-with-new-identity"
    );

    drop(connection);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_orphan_sweep_defers_a_busy_directory_lock_without_holding_write_admission() {
    use crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass;

    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-sweep-lock-defer").await;
    let raw_path = config.proxy_raw_dir.join("directory-lock-orphan.bin");
    fs::write(&raw_path, b"orphan").expect("write orphan candidate");
    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let initial =
        sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial_traversal)
            .await
            .expect("quarantine orphan candidate");
    assert_eq!(initial.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("expire orphan quarantine");

    let held_lock = crate::maintenance::retention_archive_file_lock(&raw_path)
        .expect("hold raw directory lock");
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_thread = std::thread::spawn(move || {
        let _ = release_rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(held_lock);
    });
    let mut sweep = {
        let coordinator = coordinator.clone();
        let pool = pool.clone();
        let config = config.clone();
        tokio::spawn(async move {
            let mut traversal = RetentionRawDirectoryTraversal::default();
            crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
                .scope(
                    coordinator,
                    crate::maintenance::retention_try_archive_locks_scope(
                        sweep_orphan_proxy_raw_files_slice(
                            &pool,
                            &config,
                            None,
                            false,
                            &mut traversal,
                        ),
                    ),
                )
                .await
        })
    };
    let completed = tokio::time::timeout(std::time::Duration::from_millis(250), &mut sweep).await;
    let completed_before_lock_release = completed.is_ok();
    let _ = release_tx.send(());
    release_thread.join().expect("release held directory lock");
    let pass = match completed {
        Ok(result) => result.expect("join completed bounded sweep task"),
        Err(_) => sweep.await.expect("join deferred bounded sweep task"),
    }
    .expect("defer the busy directory lock");

    assert!(
        completed_before_lock_release,
        "a busy cross-process lock must defer without blocking"
    );
    assert_eq!(pass.failures, 1);
    assert_eq!(pass.removed, 0);
    assert!(raw_path.exists());
    let _p1_permit = coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await;

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_raw_reconciliation_resets_wrong_identity_without_release() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-reconcile-identity").await;
    let raw_path = config.proxy_raw_dir.join("identity-replacement.bin");
    fs::write(&raw_path, b"old-identity").expect("write original raw file");
    sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("record original identity");
    let raw_path_string = raw_path.to_string_lossy().to_string();
    let original_identity: String = sqlx::query_scalar(
        "SELECT file_identity FROM retention_raw_reconciliation WHERE raw_path = ?1",
    )
    .bind(&raw_path_string)
    .fetch_one(&pool)
    .await
    .expect("load original identity");
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS as i64 + 1),
        ))
        .bind(&raw_path_string)
        .execute(&pool)
        .await
        .expect("age original quarantine row");
    let replacement_path = config.proxy_raw_dir.join("identity-replacement.tmp");
    fs::write(&replacement_path, b"replacement-identity").expect("write replacement inode");
    // Timestamp resolution must not decide whether this fixture changes identity.
    fs::rename(&replacement_path, &raw_path).expect("atomically replace raw file identity");
    sqlx::query(
        "UPDATE retention_recovery_cursors SET cursor = '' WHERE scope = 'raw_payload_files'",
    )
    .execute(&pool)
    .await
    .expect("rewind raw cursor after replacement");

    let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("reconcile replacement identity");
    assert_eq!(removed, 0);
    assert!(raw_path.exists());
    let replacement_identity: String = sqlx::query_scalar(
        "SELECT file_identity FROM retention_raw_reconciliation WHERE raw_path = ?1",
    )
    .bind(&raw_path_string)
    .fetch_one(&pool)
    .await
    .expect("load replacement identity");
    assert_ne!(replacement_identity, original_identity);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_raw_reconciliation_keeps_referenced_fallback_owner() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-reconcile-reference").await;
    let raw_path = config.proxy_raw_dir.join("referenced-owner.bin");
    fs::write(&raw_path, b"referenced-owner").expect("write referenced raw file");
    insert_retention_invocation(
        &pool,
        "referenced-owner",
        &shanghai_local_days_ago(1, 12, 0, 0),
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        Some(&raw_path),
        Some(1),
        Some(0.0),
    )
    .await;

    let first = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("scan referenced raw file");
    assert_eq!(first, 0);
    assert!(raw_path.exists());
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count referenced raw ledger rows");
    assert_eq!(ledger_rows, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_raw_reconciliation_respects_scan_batch_boundary() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-reconcile-batch").await;
    for index in 0..33 {
        fs::write(
            config
                .proxy_raw_dir
                .join(format!("batch-boundary-{index:02}.bin")),
            b"batch-boundary",
        )
        .expect("write batch boundary raw file");
    }

    let mut traversal = RetentionRawDirectoryTraversal::default();
    let first = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("run first bounded raw scan");
    assert_eq!(first.removed, 0);
    assert!(first.inspected_entries <= 128);
    let first_ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation")
            .fetch_one(&pool)
            .await
            .expect("count first bounded raw ledger rows");
    assert_eq!(first_ledger_rows, 32);

    let second = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("run second bounded raw scan");
    assert_eq!(second.removed, 0);
    assert!(second.inspected_entries <= 128);
    let second_ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation")
            .fetch_one(&pool)
            .await
            .expect("count second bounded raw ledger rows");
    assert_eq!(second_ledger_rows, 33);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_raw_directory_iteration_is_bounded_and_eventually_reaches_eof() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-directory-slice").await;
    for index in 0..300 {
        fs::write(
            config
                .proxy_raw_dir
                .join(format!("untracked-{index:04}.tmp")),
            b"ignored",
        )
        .expect("write ignored directory entry");
    }

    let mut traversal = RetentionRawDirectoryTraversal::default();
    let mut passes = 0;
    let mut total_inspected = 0;
    loop {
        let pass = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
            .await
            .expect("run bounded raw directory pass");
        assert!(pass.inspected_entries <= 128);
        total_inspected += pass.inspected_entries;
        passes += 1;
        if pass.reached_end {
            break;
        }
        assert!(
            passes < 8,
            "directory iterator should make bounded progress"
        );
    }

    assert_eq!(total_inspected, 300);
    assert_eq!(passes, 3);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_pressure_rejection_performs_no_directory_io() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pressure-preflight").await;
    fs::write(
        config.proxy_raw_dir.join("pressure-candidate.bin"),
        b"candidate",
    )
    .expect("write raw candidate");
    let ledger_path = config.proxy_raw_dir.join("pressure-ledger.bin");
    fs::write(&ledger_path, b"ledger-candidate").expect("write ledger candidate");
    sqlx::query(
        "INSERT INTO retention_raw_reconciliation (raw_path, file_identity, byte_size, quarantined_at) VALUES (?1, 'pressure-ledger', 16, datetime('now'))",
    )
    .bind(ledger_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("seed raw reconciliation ledger row");
    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        std::time::Duration::from_secs(60),
    ));
    let _busy_permit = pressure_gate
        .try_begin_background("raw_orphan_pressure_preflight")
        .expect("occupy the test background pressure slot");
    let opens = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let entries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let metadata_checks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut traversal = RetentionRawDirectoryTraversal::default();
    let result = crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE
        .scope(
            pressure_gate,
            crate::maintenance::RETENTION_TEST_RAW_DIRECTORY_OPENS.scope(
                opens.clone(),
                crate::maintenance::RETENTION_TEST_RAW_DIRECTORY_ENTRIES.scope(
                    entries.clone(),
                    crate::maintenance::RETENTION_TEST_RAW_FILE_METADATA_CHECKS.scope(
                        metadata_checks.clone(),
                        sweep_orphan_proxy_raw_files_slice(
                            &pool,
                            &config,
                            None,
                            false,
                            &mut traversal,
                        ),
                    ),
                ),
            ),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(opens.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(entries.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        metadata_checks.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "pressure rejection must not inspect raw files referenced by the ledger"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_retains_unprocessed_batch_after_midpass_admission_defer() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-midpass-defer").await;
    for name in ["midpass-a.bin", "midpass-b.bin"] {
        fs::write(config.proxy_raw_dir.join(name), b"midpass-candidate")
            .expect("write midpass candidate");
    }

    let deny = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut traversal = RetentionRawDirectoryTraversal::default();
    let first = crate::maintenance::RETENTION_TEST_RAW_MIDPASS_WRITE_DENY
        .scope(
            deny.clone(),
            sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal),
        )
        .await
        .expect("settle a partial pass after admission defer");
    assert!(first.deferred);
    assert_eq!(first.admission_stage.as_deref(), Some("maintenance_write"));
    assert_eq!(first.admission_cause.as_deref(), Some("coordinator_wait"));
    assert_eq!(first.quarantined, 0);

    deny.store(false, std::sync::atomic::Ordering::Relaxed);
    let second = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("retry retained midpass candidates");
    assert_eq!(second.quarantined, 2);
    assert_eq!(second.removed, 0);

    let ledger_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation")
        .fetch_one(&pool)
        .await
        .expect("count retained midpass ledger rows");
    assert_eq!(ledger_rows, 2);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_holds_pressure_slot_but_releases_write_admission_during_directory_io() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pressure-slot").await;
    fs::write(
        config.proxy_raw_dir.join("pressure-slot-candidate.bin"),
        b"candidate",
    )
    .expect("write raw candidate");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        std::time::Duration::from_secs(60),
    ));
    let pressure_probes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let p1_probes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut traversal = RetentionRawDirectoryTraversal::default();
    let result = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator,
            crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                pressure_gate,
                crate::maintenance::RETENTION_TEST_RAW_DIRECTORY_PRESSURE_PROBES.scope(
                    pressure_probes.clone(),
                    crate::maintenance::RETENTION_TEST_RAW_DIRECTORY_P1_PROBES.scope(
                        p1_probes.clone(),
                        sweep_orphan_proxy_raw_files_slice(
                            &pool,
                            &config,
                            None,
                            false,
                            &mut traversal,
                        ),
                    ),
                ),
            ),
        )
        .await
        .expect("scan bounded raw directory");

    assert!(result.inspected_entries > 0);
    assert!(
        pressure_probes.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "another background task must not take the pressure slot during directory I/O"
    );
    assert!(
        p1_probes.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "the coordinator write permit must be released before directory I/O"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_schedule_persists_pressure_backoff_and_progress() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-sweep-schedule").await;
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        2,
        std::time::Duration::from_secs(60),
    ));
    let schedule_test = async {
        sqlx::query(
            "UPDATE retention_recovery_cursors SET next_retry_at = NULL, consecutive_failure_count = 0, last_failure_fingerprint = NULL, defer_reason = NULL, last_progress_at = NULL, last_admission_stage = NULL, last_admission_cause = NULL, last_settled_pass_at = NULL, last_settled_pass_complete = NULL, last_settled_pass_inspected_entries = NULL, last_settled_pass_referenced_skipped = NULL, last_settled_pass_quarantined = NULL, last_settled_pass_removed = NULL, last_settled_pass_removed_bytes = NULL, last_nonzero_removal_at = NULL, last_nonzero_removal = NULL, last_nonzero_removal_bytes = NULL WHERE scope = 'raw_payload_files'",
        )
        .execute(&pool)
        .await
        .expect("reset raw sweep schedule");

        for (failure_index, expected_delay) in
            [300, 600, 1_200, 2_400, 3_600].into_iter().enumerate()
        {
            assert_eq!(
                raw_orphan_sweep_failure_retry_secs(failure_index as u32),
                expected_delay
            );
            let persisted = persist_raw_orphan_sweep_schedule(
                &pool,
                expected_delay,
                RawOrphanSweepScheduleTransition::Failure {
                    fingerprint: format!("{:016x}", failure_index + 1),
                    progressed: failure_index == 0,
                },
            )
            .await
            .expect("persist failure schedule");
            assert_eq!(persisted, Some(failure_index as u32 + 1));
            let updated = load_raw_orphan_sweep_schedule(&pool)
                .await
                .expect("reload raw sweep failure schedule");
            assert_eq!(updated.consecutive_failure_count, failure_index as u32 + 1);
            assert_eq!(updated.defer_reason.as_deref(), Some("retry_backoff"));
            assert_eq!(
                updated.last_failure_fingerprint.as_deref(),
                Some(format!("{:016x}", failure_index + 1).as_str())
            );
            assert!(updated.next_retry_at.is_some());
            assert!(updated.retry_after_secs <= expected_delay);
            assert!(updated.retry_after_secs >= expected_delay - 1);
            if failure_index == 0 {
                assert!(updated.last_progress_at.is_some());
            }
        }
        assert_eq!(raw_orphan_sweep_failure_retry_secs(5), 3_600);

        persist_raw_orphan_sweep_schedule(&pool, 300, RawOrphanSweepScheduleTransition::Pressure)
            .await
            .expect("persist pressure schedule");
        let deferred = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("load deferred raw sweep schedule");
        assert_eq!(deferred.consecutive_failure_count, 5);
        assert_eq!(deferred.defer_reason.as_deref(), Some("sqlite_pressure"));
        assert_eq!(deferred.last_failure_fingerprint, None);
        assert!(deferred.retry_after_secs >= 299);

        persist_raw_orphan_sweep_schedule(
            &pool,
            1,
            RawOrphanSweepScheduleTransition::Success { progressed: true },
        )
        .await
        .expect("persist successful progress");
        let recovered = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("load recovered raw sweep schedule");
        assert_eq!(recovered.consecutive_failure_count, 0);
        assert_eq!(recovered.defer_reason, None);
        assert!(recovered.last_progress_at.is_some());
        assert!(recovered.next_retry_at.is_some());

        let evidence_pass = RawOrphanSweepPassResult {
            inspected_entries: 12,
            referenced_skipped: 3,
            quarantined: 2,
            removed: 1,
            removed_bytes: 4_096,
            reached_end: false,
            complete: true,
            ..Default::default()
        };
        persist_raw_orphan_sweep_schedule_with_evidence(
            &pool,
            1,
            RawOrphanSweepScheduleTransition::Success { progressed: true },
            Some(&evidence_pass),
            None,
        )
        .await
        .expect("persist settled raw sweep evidence");
        let settled = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("load settled raw sweep evidence");
        let settled_pass = settled
            .last_settled_pass
            .as_ref()
            .expect("settled pass evidence");
        assert!(settled_pass.complete);
        assert_eq!(settled_pass.inspected_entries, 12);
        assert_eq!(settled_pass.removed_bytes, 4_096);
        let last_removal = settled
            .last_nonzero_removal
            .as_ref()
            .expect("last nonzero removal evidence");
        assert_eq!(last_removal.removed, 1);
        assert_eq!(last_removal.removed_bytes, 4_096);

        persist_raw_orphan_sweep_schedule_with_evidence(
            &pool,
            300,
            RawOrphanSweepScheduleTransition::Pressure,
            None,
            Some(("background_slot", "background_busy")),
        )
        .await
        .expect("persist admission defer evidence");
        let deferred_with_evidence = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("reload deferred evidence");
        assert_eq!(
            deferred_with_evidence.admission_stage.as_deref(),
            Some("background_slot")
        );
        assert_eq!(
            deferred_with_evidence.admission_cause.as_deref(),
            Some("background_busy")
        );
        assert!(deferred_with_evidence.last_settled_pass.is_some());

        let progressing_pass = RawOrphanSweepPassResult {
            inspected_entries: 128,
            reached_end: false,
            ..Default::default()
        };
        let progress_delay = raw_orphan_sweep_next_retry_secs(&progressing_pass, 0);
        assert_eq!(progress_delay, 1);
        persist_raw_orphan_sweep_schedule(
            &pool,
            progress_delay,
            RawOrphanSweepScheduleTransition::Success { progressed: true },
        )
        .await
        .expect("persist bounded progress cadence");
        let progress_deadline = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("load bounded progress deadline");
        assert!(progress_deadline.retry_after_secs <= 1);

        let empty_pass = RawOrphanSweepPassResult {
            reached_end: true,
            complete: true,
            ..Default::default()
        };
        let empty_delay = raw_orphan_sweep_next_retry_secs(&empty_pass, 0);
        assert_eq!(empty_delay, 300);
        let budget_exhausted_at_eof = RawOrphanSweepPassResult {
            reached_end: true,
            complete: false,
            ..Default::default()
        };
        assert_eq!(
            raw_orphan_sweep_next_retry_secs(&budget_exhausted_at_eof, 0),
            1
        );
        persist_raw_orphan_sweep_schedule(
            &pool,
            empty_delay,
            RawOrphanSweepScheduleTransition::Success { progressed: false },
        )
        .await
        .expect("persist EOF cadence");
        let empty_deadline = load_raw_orphan_sweep_schedule(&pool)
            .await
            .expect("load EOF deadline");
        assert!(empty_deadline.retry_after_secs <= 300);
        assert!(empty_deadline.retry_after_secs >= 299);
    };
    crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator,
            crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(pressure_gate, schedule_test),
        )
        .await;

    pool.close().await;
    let reopened = SqlitePool::connect(&test_sqlite_url_for_path(&config.database_path))
        .await
        .expect("reopen raw sweep schedule database");
    let persisted = load_raw_orphan_sweep_schedule(&reopened)
        .await
        .expect("load schedule after reopen");
    assert_eq!(persisted.consecutive_failure_count, 0);
    assert!(persisted.last_progress_at.is_some());
    assert!(persisted.next_retry_at.is_some());
    assert!(persisted.last_settled_pass.is_some());
    assert!(persisted.last_nonzero_removal.is_some());

    reopened.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bounded_raw_sweep_yields_to_concurrent_p1_and_interactive_writes() {
    use crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-sweep-foreground-yield").await;
    for index in 0..1_024 {
        fs::write(
            config.proxy_raw_dir.join(format!("filler-{index:04}.tmp")),
            b"untracked",
        )
        .expect("write filler directory entry");
    }
    let raw_path = config.proxy_raw_dir.join("foreground-orphan.bin");
    fs::write(&raw_path, b"foreground-orphan").expect("write raw orphan");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        4,
        std::time::Duration::from_secs(60),
    ));
    let mut initial_traversal = RetentionRawDirectoryTraversal::default();
    let mut observed = false;
    for _ in 0..12 {
        let pass = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
            .scope(
                coordinator.clone(),
                crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                    pressure_gate.clone(),
                    sweep_orphan_proxy_raw_files_slice(
                        &pool,
                        &config,
                        None,
                        false,
                        &mut initial_traversal,
                    ),
                ),
            )
            .await
            .expect("scan bounded large directory");
        assert!(pass.inspected_entries <= 128);
        if pass.quarantined > 0 {
            observed = true;
            break;
        }
    }
    assert!(
        observed,
        "bounded passes should eventually observe the candidate"
    );
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(24 * 60 * 60 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("age candidate quarantine");

    let p1_errors = std::sync::Arc::new(AtomicUsize::new(0));
    let interactive_errors = std::sync::Arc::new(AtomicUsize::new(0));
    let p1 = {
        let pool = pool.clone();
        let coordinator = coordinator.clone();
        let errors = p1_errors.clone();
        async move {
            for _ in 0..40 {
                let _permit = coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await;
                if sqlx::query(
                    "UPDATE system_raw_payload_metrics SET updated_at = datetime('now') WHERE singleton = 1",
                )
                .execute(&pool)
                .await
                .is_err()
                {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    };
    let interactive = {
        let pool = pool.clone();
        let coordinator = coordinator.clone();
        let errors = interactive_errors.clone();
        async move {
            for _ in 0..40 {
                let _permit = coordinator
                    .acquire(ProxySqliteWriteClass::InteractiveProxy)
                    .await;
                if sqlx::query(
                    "UPDATE system_raw_payload_metrics SET updated_at = datetime('now') WHERE singleton = 1",
                )
                .execute(&pool)
                .await
                .is_err()
                {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    };
    let sweep = {
        let pool = pool.clone();
        let config = config.clone();
        let coordinator = coordinator.clone();
        let pressure_gate = pressure_gate.clone();
        let raw_path = raw_path.clone();
        async move {
            let mut traversal = RetentionRawDirectoryTraversal::default();
            let mut total_inspected = 0;
            let mut removed = 0;
            for _ in 0..16 {
                let pass = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
                    .scope(
                        coordinator.clone(),
                        crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
                            pressure_gate.clone(),
                            sweep_orphan_proxy_raw_files_slice(
                                &pool,
                                &config,
                                None,
                                false,
                                &mut traversal,
                            ),
                        ),
                    )
                    .await
                    .expect("continue bounded sweep during foreground writes");
                assert!(pass.inspected_entries <= 128);
                total_inspected += pass.inspected_entries;
                removed += pass.removed;
                if !raw_path.exists() {
                    break;
                }
            }
            (total_inspected, removed)
        }
    };

    let (_, _, (total_inspected, removed)) = tokio::join!(p1, interactive, sweep);
    assert_eq!(p1_errors.load(Ordering::Relaxed), 0);
    assert_eq!(interactive_errors.load(Ordering::Relaxed), 0);
    assert!(total_inspected > 0);
    assert!(removed >= 1);
    assert!(
        !raw_path.exists(),
        "expired orphan should be physically released"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_link_reference_query_uses_path_index_and_requires_completed_seed() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-link-index").await;
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
        "EXPLAIN QUERY PLAN SELECT DISTINCT raw_path FROM proxy_raw_payload_blob_links WHERE raw_path IN (?1, ?2)",
    )
    .bind("/tmp/raw-a.bin")
    .bind("/tmp/raw-b.bin")
    .fetch_all(&pool)
    .await
    .expect("explain indexed raw reference lookup")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>()
    .join(" ");
    assert!(plan.contains("idx_proxy_raw_payload_blob_links_path"));

    let raw_path = config.proxy_raw_dir.join("seed-incomplete.bin");
    fs::write(&raw_path, b"seed-incomplete").expect("write raw candidate");
    sqlx::query(
        "DELETE FROM proxy_raw_payload_blob_link_migrations WHERE migration_name = 'seed_existing_raw_blob_links_v1'",
    )
    .execute(&pool)
    .await
    .expect("remove legacy seed completion marker");

    let mut traversal = RetentionRawDirectoryTraversal::default();
    let pass = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut traversal)
        .await
        .expect("scan fails closed with incomplete seed");
    assert!(raw_path.exists());
    assert_eq!(pass.removed, 0);
    assert_eq!(pass.quarantined, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_dry_run_does_not_mutate_database_or_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-dry-run").await;
    let response_raw = config.proxy_raw_dir.join("dry-run-response.bin");
    let orphan = config.proxy_raw_dir.join("dry-run-orphan.bin");
    fs::write(&response_raw, b"dry-run-response").expect("write dry-run response raw");
    fs::write(&orphan, b"dry-run-orphan").expect("write dry-run orphan");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(91, 7, 0, 0);
    insert_retention_invocation(
        &pool,
        "dry-run-old",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&response_raw),
        Some(111),
        Some(0.9),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(true), None)
        .await
        .expect("run dry-run retention");
    assert!(summary.dry_run);
    assert_eq!(summary.invocation_rows_archived, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 0);
    assert!(response_raw.exists());
    assert!(orphan.exists());

    let row = sqlx::query(
        "SELECT detail_level, payload, raw_response FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("dry-run-old")
    .fetch_one(&pool)
    .await
    .expect("load dry-run invocation");
    assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert!(row.get::<Option<String>, _>("payload").is_some());
    assert_eq!(row.get::<String, _>("raw_response"), "{\"ok\":true}");

    let archive_batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count dry-run archive batches");
    assert_eq!(archive_batch_count, 0);

    let archive_files = fs::read_dir(&config.archive_dir)
        .expect("read archive dir")
        .count();
    assert_eq!(archive_files, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_sweep_dry_run_does_not_report_quarantine_work() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-dry-run-quarantine").await;
    let orphan = config.proxy_raw_dir.join("dry-run-quarantine.bin");
    fs::write(&orphan, b"dry-run-orphan").expect("write dry-run orphan");
    let mut traversal = RetentionRawDirectoryTraversal::default();

    let pass = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, true, &mut traversal)
        .await
        .expect("run raw sweep dry-run");

    assert_eq!(pass.quarantined, 0);
    assert!(orphan.exists());
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path = ?1")
            .bind(orphan.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count raw quarantine ledger rows");
    assert_eq!(ledger_rows, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
