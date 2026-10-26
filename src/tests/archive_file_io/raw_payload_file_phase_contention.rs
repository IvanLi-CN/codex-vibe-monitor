use super::*;

#[tokio::test]
async fn raw_orphan_sweep_retains_directory_cursor_after_file_phase_defer() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-directory-defer").await;
    for index in 0..33 {
        fs::write(
            config
                .proxy_raw_dir
                .join(format!("directory-defer-{index:02}.bin")),
            b"directory-defer",
        )
        .expect("write directory defer candidate");
    }

    let mut traversal = RetentionRawDirectoryTraversal::default();
    let first = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, true, &mut traversal)
        .await
        .expect("run first bounded raw scan");
    assert_eq!(first.quarantined, 0);
    assert!(traversal.directory.is_some());

    let held_file_phase = crate::maintenance::acquire_file_phase_permit_for_cleanup().await;
    let deferred = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, true, &mut traversal)
        .await
        .expect("defer raw scan while file phase is busy");
    assert!(deferred.deferred);
    assert_eq!(deferred.admission_stage.as_deref(), Some("file_phase"));
    assert_eq!(deferred.admission_cause.as_deref(), Some("file_phase_busy"));
    assert!(
        traversal.directory.is_some(),
        "a deferred scan must retain its ReadDir cursor"
    );
    drop(held_file_phase);

    let second = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, true, &mut traversal)
        .await
        .expect("resume raw scan after file phase release");
    assert_eq!(second.quarantined, 0);
    assert!(second.reached_end);

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
