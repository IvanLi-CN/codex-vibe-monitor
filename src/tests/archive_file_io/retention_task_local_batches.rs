use super::*;

#[tokio::test]
async fn retention_archive_file_operations_stop_at_the_operation_budget() {
    let temp_dir = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-retention-archive-file-operation-budget-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).expect("create archive file operation budget test directory");
    let source_path = temp_dir.join("archive.sqlite");
    let compressed_path = temp_dir.join("archive.sqlite.gz");
    let budgeted_compressed_path = temp_dir.join("archive-budgeted.sqlite.gz");
    let inflated_path = temp_dir.join("archive-inflated.sqlite");
    fs::write(&source_path, vec![b'x'; 4 * 1024 * 1024]).expect("write archive source");
    deflate_sqlite_file_to_gzip(&source_path, &compressed_path)
        .expect("create compressed archive source");

    let deflate_error = crate::maintenance::retention_test_with_archive_file_operation_budget(
        Duration::ZERO,
        deflate_sqlite_file_to_gzip_with_operation_budget(&source_path, &budgeted_compressed_path),
    )
    .await
    .expect_err("deflate should stop when its operation budget is exhausted");
    assert!(crate::is_retention_archive_file_operation_budget_exhausted(
        &deflate_error
    ));

    let inflate_error = crate::maintenance::retention_test_with_archive_file_operation_budget(
        Duration::ZERO,
        inflate_gzip_sqlite_file_with_operation_budget(&compressed_path, &inflated_path),
    )
    .await
    .expect_err("inflate should stop when its operation budget is exhausted");
    assert!(crate::is_retention_archive_file_operation_budget_exhausted(
        &inflate_error
    ));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archive_exports_transaction_and_complete_write_hold_separately() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("archive-observation-boundaries").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 3).await;
    let runtime = crate::ObservabilityRuntime::new(true);
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let result = crate::observability::contention::with_performance(
        runtime.clone(),
        RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator.clone(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                Arc::new(crate::db_pressure::DbPressureGate::new(
                    1,
                    Duration::from_secs(30),
                )),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ),
        ),
    )
    .await
    .expect("archive three source records");
    assert_eq!(result.0, 3);
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    let text = runtime.render();
    // The fixture selects two rows per batch, then archives the remaining row.
    for phase in ["transaction", "commit", "write_hold"] {
        assert!(
            text.lines().any(|line| {
                line.starts_with("cvm_retention_phase_duration_seconds_count{")
                    && line.contains("operation=\"invocation_archive\"")
                    && line.contains(&format!("phase=\"{phase}\""))
                    && line.ends_with(" 2")
            }),
            "both committed archive chunks must report {phase}: {}",
            text.lines()
                .filter(|line| line.starts_with("cvm_retention_phase_duration_seconds_count{"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(
        text.lines().any(|line| {
            line.starts_with("cvm_retention_phase_duration_seconds_count{")
                && line.contains("operation=\"invocation_archive\"")
                && line.contains("phase=\"file_verify\"")
                && !line.ends_with(" 0")
        }),
        "archive chunks must report file_verify: {}",
        text.lines()
            .filter(|line| line.starts_with("cvm_retention_phase_duration_seconds_count{"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_source_change_after_archive_artifact_proof() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-artifact-source-change").await;
    config.retention_batch_rows = 1;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;
    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let runtime = crate::ObservabilityRuntime::new(true);
    let work = crate::observability::contention::with_performance(
        runtime.clone(),
        RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator,
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate,
                crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                    probe.clone(),
                    archive_old_invocations(&pool, &config, config.database_path.parent(), false),
                ),
            ),
        ),
    );
    let mutate_source = async {
        probe.verified.notified().await;
        sqlx::query("UPDATE codex_invocations SET payload = '{\"changed\":true}' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("mutate source after artifact verification");
        probe.resume.notify_one();
    };
    let (result, ()) = tokio::join!(work, mutate_source);
    let error = result.expect_err("artifact-bound source proof must reject changed live rows");
    assert!(
        error
            .to_string()
            .contains("retention task-local archive source changed before conversion")
    );
    let rendered_metrics = runtime.render();
    let proof_diagnostics = rendered_metrics
        .lines()
        .filter(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("operation=\"retention\"")
                && line.contains("reason=\"proof_unavailable\"")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        proof_diagnostics.len(),
        1,
        "source changes must record one proof diagnostic: {}",
        proof_diagnostics.join("\n")
    );
    assert!(
        proof_diagnostics[0].ends_with(" 1"),
        "source changes must not emit duplicate proof diagnostics: {}",
        proof_diagnostics[0]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source remains after proof mismatch"),
        1
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_reuses_proven_month_archive_after_source_budget_stop() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reuse-proven-month").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let archive_path =
        archive_batch_file_path(&config, "codex_invocations", &month).expect("archive path");
    let staged = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, row_count FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("published staged manifest");
    assert_eq!(staged.0, ARCHIVE_STATUS_MATERIALIZING);
    assert_eq!(staged.1, 401);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof",)
            .fetch_one(&pool)
            .await
            .expect("durable Summary proof"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after first budget stop"),
        401
    );

    // The old path would enter the injected writer pause and exhaust this budget before it could
    // delete a source row. A proven artifact must be reused without touching the writer.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await;
    let resumed = resumed.expect("resume proven archive");
    assert_eq!(resumed.0, 401);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after proven archive reuse"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM archive_batches
             WHERE dataset='codex_invocations' AND file_path=?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .fetch_one(&pool)
        .await
        .expect("completed reused manifest"),
        ARCHIVE_STATUS_COMPLETED
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_persisted_proof_after_source_change() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reject-persisted-proof").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    sqlx::query("UPDATE codex_invocations SET payload = '{\"changed\":true}' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mutate source after durable proof");

    // A persisted proof is bound to the live source identity. Once that identity changes, the
    // safe fallback rebuild is deliberately paused so this test cannot pass by reusing the file.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await
    .expect("source-change fallback run");
    assert_eq!(resumed.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after rejected reuse"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_persisted_proof_when_archive_id_set_is_not_exact() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reject-archive-id-set").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let archive_path =
        archive_batch_file_path(&config, "codex_invocations", &month).expect("archive path");
    let mutation_path = temp_dir.join("archive-id-set-mutation.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &mutation_path)
        .expect("inflate archive for ID-set mutation");
    let mutation_pool = SqlitePool::connect(&test_sqlite_url_for_path(&mutation_path))
        .await
        .expect("open archive ID-set mutation database");
    sqlx::query(
        "UPDATE codex_invocations SET id = 999999
         WHERE id = (SELECT MAX(id) FROM codex_invocations)",
    )
    .execute(&mutation_pool)
    .await
    .expect("mutate one archived source ID");
    mutation_pool.close().await;
    deflate_sqlite_file_to_gzip(&mutation_path, &archive_path)
        .expect("rewrite archive after ID-set mutation");
    fs::remove_file(&mutation_path).expect("remove archive ID-set mutation database");
    retag_staged_archive_proof(&pool, &archive_path).await;

    // The final gzip still contains 401 rows, but its selected source ID set contains only 400
    // live IDs. The fallback writer is paused so reuse must inspect the archive itself.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await
    .expect("archive-id-set fallback run");
    assert_eq!(resumed.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after rejected archive-id-set reuse"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_persisted_proof_after_archive_metadata_change() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reject-archive-metadata").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let archive_path =
        archive_batch_file_path(&config, "codex_invocations", &month).expect("archive path");
    let mutation_path = temp_dir.join("archive-metadata-mutation.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &mutation_path)
        .expect("inflate archive for metadata mutation");
    let mutation_pool = SqlitePool::connect(&test_sqlite_url_for_path(&mutation_path))
        .await
        .expect("open archive metadata mutation database");
    sqlx::query(
        "UPDATE codex_invocations SET payload = '{\"archive_changed\":true}'
         WHERE id = 1",
    )
    .execute(&mutation_pool)
    .await
    .expect("mutate archived source metadata");
    mutation_pool.close().await;
    deflate_sqlite_file_to_gzip(&mutation_path, &archive_path)
        .expect("rewrite archive after metadata mutation");
    fs::remove_file(&mutation_path).expect("remove archive metadata mutation database");
    retag_staged_archive_proof(&pool, &archive_path).await;

    // The archive keeps the same row count and IDs, but its content identity no longer matches
    // the live source. The paused fallback proves reuse rejects that mismatch before deletion.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await
    .expect("archive-metadata fallback run");
    assert_eq!(resumed.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after rejected archive metadata reuse"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_budget_stop_during_proof_reuse_defers_without_deletion() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reuse-proof-budget").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let first_probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                first_probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = first_probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    first_probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    let proof_probe = Arc::new(crate::maintenance::ArchiveIdentityProofProbe::default());
    let second_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(
                crate::maintenance::RETENTION_TEST_ARCHIVE_IDENTITY_PROOF.scope(
                    proof_probe.clone(),
                    archive_old_invocations(&pool, &config, config.database_path.parent(), false),
                ),
            );
            tokio::select! {
                result = &mut archive => result,
                _ = proof_probe.started.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    proof_probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let second = tokio::time::timeout(Duration::from_secs(30), second_work)
        .await
        .expect("proof reuse reached the budget stop");
    assert_eq!(second.expect("proof reuse budget-limited run").0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after proof reuse budget stop"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_persisted_proof_when_final_marker_is_missing() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reject-missing-final-proof").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    sqlx::query("DELETE FROM summary_archive_snapshot_v2_proof")
        .execute(&pool)
        .await
        .expect("remove final Summary proof marker");

    // Valid pages without the final marker are recovery data, not a reusable proof. The paused
    // fallback prevents this test from succeeding through a fresh archive rebuild.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await
    .expect("missing-final-proof fallback run");
    assert_eq!(resumed.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after rejected missing-proof reuse"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rejects_persisted_proof_when_proof_coverage_changes() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-reject-coverage-change").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;

    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let first_work =
        crate::maintenance::retention_test_with_work_budget(Duration::from_secs(60), async {
            let mut archive = Box::pin(crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ));
            tokio::select! {
                result = &mut archive => result,
                _ = probe.verified.notified() => {
                    crate::maintenance::retention_test_expire_work_budget();
                    probe.resume.notify_one();
                    archive.await
                }
            }
        });
    let first = tokio::time::timeout(Duration::from_secs(30), first_work)
        .await
        .expect("first retention run reached the budget stop");
    assert_eq!(first.expect("first budget-limited run").0, 0);

    sqlx::query(
        "UPDATE summary_archive_snapshot_v2_proof
         SET coverage_start = '2099-01-01 00:00:00'",
    )
    .execute(&pool)
    .await
    .expect("corrupt staged proof coverage");

    // The proof marker remains present, but its coverage no longer matches the manifest. The
    // paused fallback proves that reuse is rejected before source conversion can report a late
    // error.
    let resumed = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(15),
        RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
            Duration::from_secs(60),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    )
    .await
    .expect("coverage-mismatch fallback run");
    assert_eq!(resumed.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows after rejected coverage reuse"),
        401
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archive_selection_defers_large_payload_materialization() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("archive-selection-metadata").await;
    let occurred_at = "2020-01-15 12:00:00";
    let payload = "x".repeat(48 * 1024);
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000)
         INSERT INTO codex_invocations(
             invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
         )
         SELECT 'metadata-selection-' || i, ?1, 'proxy', 'success', 7, 0.07, ?2, '{}'
         FROM n",
    )
    .bind(occurred_at)
    .bind(&payload)
    .execute(&pool)
    .await
    .expect("seed large invocation archive candidates");

    let (candidates, selected) =
        crate::maintenance::invocation_archive_candidate_selection_for_test(
            &pool,
            "9999-12-31 00:00:00",
            None,
            1_000,
        )
        .await
        .expect("select and materialize bounded invocation archive candidates");
    assert_eq!(candidates.len(), 1_000);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.payload.is_none()),
        "candidate selection must not materialize payloads before the size selector"
    );
    assert!(
        !selected.is_empty(),
        "the bounded selector must retain a minimum unit"
    );
    assert!(
        selected.len() < candidates.len(),
        "large payloads must constrain the selected batch before full materialization"
    );
    let materialized_payload_bytes: usize = selected
        .iter()
        .filter_map(|candidate| candidate.payload.as_deref())
        .map(str::len)
        .sum();
    assert!(
        materialized_payload_bytes <= 16 * 1024 * 1024,
        "selected payload materialization must remain within the 16 MiB selector budget: {materialized_payload_bytes}"
    );
    let selected_ids = selected
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    assert!(
        selected_ids.windows(2).all(|ids| ids[0] < ids[1]),
        "selected candidates must retain the source ordering"
    );
    assert!(
        selected
            .iter()
            .all(|candidate| candidate.payload.as_deref() == Some(payload.as_str())),
        "selected candidates must retain their full payloads for archive conversion"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_commit_reader_lock_is_pressure_deferred_before_durable_commit() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-commit-reader-lock").await;
    config.retention_batch_rows = 1;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;

    // A rollback-journal reader keeps a SHARED lock while the retention writer reaches COMMIT.
    // The production connection is switched to a zero busy timeout only by the test seam immediately
    // before the real COMMIT, so this exercises SQLite's reversible BUSY outcome.
    let mut journal_setup = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("open journal-mode setup connection");
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode=DELETE")
        .fetch_one(&mut journal_setup)
        .await
        .expect("switch test database to rollback journal");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut journal_setup)
        .await
        .expect("disable setup connection busy timeout");
    journal_setup
        .close()
        .await
        .expect("close journal-mode setup connection");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let transaction_probe = Arc::new(crate::maintenance::TransactionBeginProbe::default());
    let commit_probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
    commit_probe
        .force_commit_busy_timeout_zero
        .store(true, Ordering::Release);
    let runtime = crate::ObservabilityRuntime::new(true);
    let work = crate::observability::contention::with_performance(
        runtime.clone(),
        RETENTION_TEST_WRITE_COORDINATOR.scope(
            coordinator,
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                gate,
                crate::maintenance::RETENTION_TEST_TRANSACTION_BEGIN.scope(
                    transaction_probe.clone(),
                    crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
                        commit_probe,
                        crate::maintenance::retention_test_with_work_budget(
                            Duration::from_secs(60),
                            archive_old_invocations(
                                &pool,
                                &config,
                                config.database_path.parent(),
                                false,
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    let mut work = Box::pin(work);
    tokio::select! {
        result = &mut work => panic!("retention reached a terminal result before the reader lock: {result:?}"),
        notification = tokio::time::timeout(
            Duration::from_secs(5),
            transaction_probe.begun.notified(),
        ) => {
            notification.expect("retention transaction began");
        }
    }

    let mut reader = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("open rollback-journal reader");
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut reader)
        .await
        .expect("disable reader busy timeout");
    sqlx::query("BEGIN")
        .execute(&mut reader)
        .await
        .expect("begin reader transaction");
    let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&mut reader)
        .await
        .expect("establish reader shared lock");
    transaction_probe.resume.notify_one();

    let error = work
        .await
        .expect_err("reader lock must fail the real retention COMMIT");
    assert!(
        crate::is_sqlite_lock_error(&error),
        "expected SQLite BUSY/LOCKED from COMMIT, got: {error:#}"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut reader)
        .await
        .expect("release reader shared lock");
    reader.close().await.expect("close rollback-journal reader");

    let rendered = runtime.render();
    assert!(
        rendered.lines().any(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("operation=\"retention\"")
                && line.contains("reason=\"pressure_deferred\"")
                && line.ends_with(" 1")
        }),
        "known COMMIT locks must use the existing pressure classification: {rendered}"
    );
    assert!(
        !rendered.lines().any(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("operation=\"retention\"")
                && line.contains("reason=\"failed\"")
                && line.ends_with(" 1")
        }),
        "a reversible COMMIT lock must not be reported as generic failure: {rendered}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source remains after reversible commit failure"),
        1
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_p2_queue_does_not_hold_file_io_admission() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("task-local-p2-file-io-admission").await;
    let archive_path = config.archive_dir.join("fairness-probe.sqlite.gz");
    fs::write(&archive_path, b"retention fairness probe").expect("fairness probe file");
    let expected_sha256 = sha256_hex_file(&archive_path).expect("fairness probe digest");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let p1 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::ArchiveFileVerifyProbe::default());
    let p2_acquired = Arc::new(Notify::new());
    let p2_resume = Arc::new(Notify::new());
    let p2_task = {
        let coordinator = coordinator.clone();
        let p2_acquired = Arc::clone(&p2_acquired);
        let p2_resume = Arc::clone(&p2_resume);
        tokio::spawn(async move {
            let permit = coordinator
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
                .await;
            p2_acquired.notify_one();
            p2_resume.notified().await;
            drop(permit);
        })
    };
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_ARCHIVE_FILE_VERIFY.scope(
                probe.clone(),
                crate::maintenance::retention_test_with_work_budget(
                    Duration::from_secs(60),
                    crate::maintenance::acquire_retention_batch_write_connection_with_archive_fence_observed(
                        &pool,
                        "retention_fairness_probe",
                        &archive_path,
                        Some(&expected_sha256),
                        || None,
                    ),
                ),
            ),
        ),
    );
    let mut work = Box::pin(work);
    let coordinate = async {
        tokio::time::timeout(Duration::from_secs(1), probe.started.notified())
            .await
            .expect("archive file verification probe started");
        let snapshot = coordinator.snapshot().await;
        assert_eq!(snapshot.active_write_class.as_deref(), Some("p1_terminal"));
        assert_eq!(snapshot.maintenance_fairness_admission_count, 0);
        assert_eq!(
            snapshot
                .write_admission
                .maintenance_retention
                .admission_count,
            0,
            "file verification must not claim maintenance admission"
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if coordinator.snapshot().await.p2_waiter_count == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("P2 writer queued behind the active P1 writer");

        probe.resume.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = coordinator.snapshot().await;
                if snapshot.p2_waiter_count == 1 && snapshot.maintenance_waiter_count == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("archive waits for eligibility without holding the file fence");
        drop(p1);
        tokio::time::timeout(Duration::from_secs(1), p2_acquired.notified())
            .await
            .expect("P2 writer admitted before the archive");
        let snapshot = coordinator.snapshot().await;
        assert_eq!(snapshot.active_write_class.as_deref(), Some("p2_derived"));
        assert_eq!(snapshot.maintenance_fairness_admission_count, 0);
        p2_resume.notify_one();
    };
    let (result, ()) = tokio::join!(&mut work, coordinate);
    let (mut connection, admission, archive_lock) = result
        .expect("archive fence helper admitted after P2")
        .expect("archive fence helper returned resources");
    connection.return_to_pool().await;
    drop(archive_lock);
    drop(admission);
    p2_task.await.expect("P2 task joined");
    assert_eq!(coordinator.snapshot().await.active_write_class, None);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_transaction_cancel_rolls_back_before_releasing_admission() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-transaction-cancel").await;
    config.retention_batch_rows = 1;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::TransactionBeginProbe::default());
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_TRANSACTION_BEGIN.scope(
                probe.clone(),
                crate::maintenance::retention_test_with_work_budget(
                    Duration::from_secs(60),
                    archive_old_invocations(&pool, &config, config.database_path.parent(), false),
                ),
            ),
        ),
    );
    let mut work = Box::pin(work);
    tokio::select! {
        result = &mut work => panic!("transaction cancellation unexpectedly completed: {result:?}"),
        _ = probe.begun.notified() => drop(work),
    }

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.snapshot().await.active_write_class.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancel cleanup releases admission after native drain");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source remains after transaction cancellation"),
        1
    );
    wait_for_no_task_work_files(&config.archive_dir).await;
    let mut writer = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("independent writer after cancellation");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut writer)
        .await
        .expect("rollback drained before releasing the pool connection");
    sqlx::query("ROLLBACK")
        .execute(&mut writer)
        .await
        .expect("rollback probe transaction");
    writer
        .close()
        .await
        .expect("close cancellation probe writer");
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_commit_counters_survive_cancel_during_native_drain() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-commit-counter-drain").await;
    config.retention_batch_rows = 1;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::RetentionReturnToPoolProbe {
        operation: Some("invocation_archive"),
        ..Default::default()
    });
    probe
        .pause_at_return
        .store(1, std::sync::atomic::Ordering::Release);
    let observation = crate::TaskExecutionObservation::begin(
        "retention_archive",
        "Retention commit counter drain",
        "manual",
        Some("maintenance_retention"),
        "processing",
    );
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_RETURN_TO_POOL.scope(
                probe.clone(),
                run_data_retention_maintenance_with_circuit_and_prompt_cache(
                    &pool,
                    &config,
                    Some(false),
                    None,
                    Arc::new(RawCaptureCircuitBreaker::new(config.archive_dir.clone())),
                    None,
                    Some(observation.clone()),
                ),
            ),
        ),
    );
    let mut work = Box::pin(work);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = probe.returned.notified() => {}
            _ = &mut work => panic!("retention task completed before invocation COMMIT native drain pause"),
        }
    })
        .await
        .expect("pause after invocation COMMIT native drain");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("committed source deletion visible during native drain"),
        0
    );
    let workload = crate::task_runtime_observation::workload_sample("retention_archive")
        .expect("committed workload sample");
    assert_eq!(
        workload.processed.as_ref().and_then(|metric| metric.value),
        Some(1)
    );

    drop(work);
    probe.resume.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.snapshot().await.active_write_class.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native drain completes before admission release");
    observation.finish_with_status("cancelled");
    let workload = crate::task_runtime_observation::workload_sample("retention_archive")
        .expect("workload sample after cancellation");
    assert_eq!(
        workload.processed.as_ref().and_then(|metric| metric.value),
        Some(1)
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_summary_page_rejects_stale_manifest_without_overwriting_page() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-stale-summary-manifest").await;
    config.retention_batch_rows = 1;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::RetentionReturnToPoolProbe {
        operation: Some("archive_snapshot_manifest"),
        ..Default::default()
    });
    probe
        .pause_at_return
        .store(1, std::sync::atomic::Ordering::Release);
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator,
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_RETURN_TO_POOL.scope(
                probe.clone(),
                run_data_retention_maintenance_with_circuit_and_prompt_cache(
                    &pool,
                    &config,
                    Some(false),
                    None,
                    Arc::new(RawCaptureCircuitBreaker::new(config.archive_dir.clone())),
                    None,
                    None,
                ),
            ),
        ),
    );
    let mut work = Box::pin(work);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = probe.returned.notified() => {}
            _ = &mut work => panic!("retention task completed before manifest staging native drain pause"),
        }
    })
        .await
        .expect("pause after manifest staging native drain");

    let mut writer = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("open independent manifest replacement writer");
    assert_eq!(
        sqlx::query(
            "UPDATE archive_batches
             SET sha256='replacement-digest'
             WHERE dataset='codex_invocations'",
        )
        .execute(&mut writer)
        .await
        .expect("replace staged manifest digest")
        .rows_affected(),
        1
    );
    writer
        .close()
        .await
        .expect("close manifest replacement writer");
    probe.resume.notify_one();

    let summary = tokio::time::timeout(Duration::from_secs(5), &mut work)
        .await
        .expect("stale manifest page staging completes")
        .expect("structured stale manifest failure summary");
    assert_eq!(summary.completion(), "failed");
    assert!(summary.fatal_error.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot")
            .fetch_one(&pool)
            .await
            .expect("stale manifest leaves the durable page intact"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT sha256 FROM archive_batches WHERE dataset='codex_invocations'",
        )
        .fetch_one(&pool)
        .await
        .expect("replacement manifest remains current"),
        "replacement-digest"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("stale manifest leaves source live"),
        1
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn raw_orphan_dry_run_reports_due_release_without_mutating() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-dry-run-release").await;
    let raw_path = config.proxy_raw_dir.join("dry-run-orphan.bin");
    let payload = b"dry-run-orphan";
    fs::write(&raw_path, payload).expect("write dry-run orphan");
    let mut initial = RetentionRawDirectoryTraversal::default();
    let first = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, false, &mut initial)
        .await
        .expect("quarantine raw orphan");
    assert_eq!(first.quarantined, 1);
    sqlx::query("UPDATE retention_raw_reconciliation SET quarantined_at=?1 WHERE raw_path=?2")
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS as i64 + 1),
        ))
        .bind(raw_path.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .expect("age quarantine");
    let mut preview = RetentionRawDirectoryTraversal::default();
    let pass = sweep_orphan_proxy_raw_files_slice(&pool, &config, None, true, &mut preview)
        .await
        .expect("preview raw release");
    assert_eq!(pass.removed, 1);
    assert_eq!(pass.removed_bytes, payload.len() as u64);
    assert!(raw_path.exists());
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_raw_reconciliation WHERE raw_path=?1")
            .bind(raw_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count preview ledger");
    assert_eq!(rows, 1);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
use crate::maintenance::{RETENTION_TEST_DB_PRESSURE_GATE, RETENTION_TEST_WRITE_COORDINATOR};

#[tokio::test]
async fn retention_quota_candidate_cancellation_closes_connection_and_preserves_source() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("quota-query-cancel").await;
    let captured_at = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<256)
         INSERT INTO codex_quota_snapshots(captured_at) SELECT ?1 FROM n",
    )
    .bind(&captured_at)
    .execute(&pool)
    .await
    .expect("quota ranking fixture");
    let probe = Arc::new(crate::maintenance::RetentionSqliteMaintenanceTestProbe::default());
    let result = crate::maintenance::RETENTION_TEST_SQLITE_MAINTENANCE_PROBE
        .scope(
            probe.clone(),
            crate::maintenance::retention_test_with_work_budget(
                Duration::from_secs(60),
                crate::maintenance::compact_old_quota_snapshots(&pool, &config, false),
            ),
        )
        .await
        .expect("candidate cancellation is a safe stop");
    assert_eq!(result, (0, 0));
    assert_eq!(probe.progress_callbacks.load(Ordering::Acquire), 1);
    assert!(probe.connection_closed.load(Ordering::Acquire));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_quota_snapshots")
            .fetch_one(&pool)
            .await
            .expect("unmodified quota source"),
        256,
    );
    assert_no_task_work_files(&config.archive_dir);
    let mut writer = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("independent writer");
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut writer)
        .await
        .expect("nonblocking probe");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut writer)
        .await
        .expect("query connection released");
    sqlx::query("UPDATE codex_quota_snapshots SET used_amount=1")
        .execute(&mut writer)
        .await
        .expect("source remains writable");
    sqlx::query("COMMIT")
        .execute(&mut writer)
        .await
        .expect("no lingering read transaction");
    writer.close().await.expect("close probe writer");
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_quota_prepare_timeout_discards_work_and_reselects_live_source() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("quota-prepare-timeout").await;
    let early = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
    let late = utc_naive_from_shanghai_local_days_ago(40, 23, 0, 0);
    seed_quota_snapshot(&pool, &early).await;
    seed_quota_snapshot(&pool, &late).await;
    RETENTION_TEST_WRITE_COORDINATOR.scope(
        crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            Arc::new(crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30))),
            async {
    // Force the preparation timeout independently of CPU speed. The injected pause
    // guarantees pending work; no elapsed-time threshold is an assertion.
    let result = crate::maintenance::RETENTION_TEST_QUOTA_ARCHIVE_PREPARE_BUDGET
        .scope(
            Duration::ZERO,
            RETENTION_TEST_TASK_ARCHIVE_PAUSE.scope(
                Duration::from_secs(60),
                crate::maintenance::compact_old_quota_snapshots(&pool, &config, false),
            ),
        )
        .await
        .expect("prepare timeout preserves source");
    assert_eq!(result, (0, 0));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_quota_snapshots")
            .fetch_one(&pool).await.expect("source after timeout"),
        2,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM archive_batches WHERE dataset='codex_quota_snapshots'")
            .fetch_one(&pool).await.expect("no committed quota manifest"),
        0,
    );
    wait_for_no_task_work_files(&config.archive_dir).await;
    let next = crate::maintenance::compact_old_quota_snapshots(&pool, &config, false)
        .await.expect("fresh live-row selection");
    assert_eq!(next, (1, 1));
    let remaining: Vec<String> = sqlx::query_scalar("SELECT captured_at FROM codex_quota_snapshots")
        .fetch_all(&pool).await.expect("daily representative remains");
    assert_eq!(remaining, vec![late]);
    wait_for_no_task_work_files(&config.archive_dir).await;
            },
        ),
    ).await;
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_admission_reason_is_current_and_cleared_next_run() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-defer-reason").await;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1_000).await;
    let pressure = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(pressure.clone(), async {
                let owner = pressure
                    .try_begin_background("test_prompt_owner")
                    .expect("hold the competing owner's background slot");
                let deferred = run_data_retention_maintenance(&pool, &config, Some(false), None)
                    .await
                    .expect("safe admission deferral");
                assert!(deferred.deferred);
                assert_eq!(deferred.wait_reason.as_deref(), Some("background_busy"));
                assert_eq!(deferred.invocation_rows_archived, 0);
                assert_eq!(
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
                        .fetch_one(&pool)
                        .await
                        .expect("deferred source remains"),
                    1_000,
                );
                assert_no_task_work_files(&config.archive_dir);
                drop(owner);
                let completed = run_data_retention_maintenance(&pool, &config, Some(false), None)
                    .await
                    .expect("fresh run after admission is available");
                assert_eq!(completed.invocation_rows_archived, 1_000);
                assert!(!completed.deferred);
                assert_eq!(completed.wait_reason, None);
                assert_no_task_work_files(&config.archive_dir);
            }),
        )
        .await;
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_optional_sqlite_maintenance_stops_and_releases_writer() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("task-local-pragma-budget").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;
    let original: i64 = sqlx::query_scalar("SELECT total_tokens FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("original source value");
    let probe = Arc::new(crate::maintenance::RetentionSqliteMaintenanceTestProbe::default());
    RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                Arc::new(crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30))),
                crate::maintenance::RETENTION_TEST_SQLITE_MAINTENANCE_PROBE.scope(
                    probe.clone(),
                    run_best_effort_retention_pragma(
                        &pool,
                        "UPDATE codex_invocations SET total_tokens = (WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<10000) SELECT SUM(i) FROM n)",
                        "test expensive optional SQLite maintenance",
                    ),
                ),
            ),
        )
        .await
        .expect("optional maintenance cancellation is nonfatal");
    assert_eq!(probe.progress_callbacks.load(Ordering::Acquire), 1);
    assert!(probe.connection_closed.load(Ordering::Acquire));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT total_tokens FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("uncommitted statement rolled back"),
        original,
    );
    let mut writer = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("independent writer");
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut writer)
        .await
        .expect("nonblocking lock probe");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut writer)
        .await
        .expect("cancelled maintenance released SQLite write lock");
    sqlx::query("ROLLBACK")
        .execute(&mut writer)
        .await
        .expect("rollback probe");
    writer.close().await.expect("close writer");
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_cancel_closes_attached_connection_and_cleans_work() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("task-local-cancel-attach").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 3).await;
    let ids = sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("IDs");
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let result = RETENTION_TEST_TASK_ARCHIVE_PAUSE
        .scope(Duration::from_secs(1), async {
            tokio::time::timeout(
                Duration::from_millis(100),
                archive_rows_into_month_batch(
                    &pool,
                    &config,
                    archive_table_spec("codex_invocations"),
                    &month,
                    &ids,
                ),
            )
            .await
        })
        .await;
    assert!(result.is_err(), "cancel during attached task-local work");
    wait_for_no_task_work_files(&config.archive_dir).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source remains"),
        3
    );
    let mut connection = pool.acquire().await.expect("pool usable after cancel");
    let databases = sqlx::query("PRAGMA database_list")
        .fetch_all(&mut *connection)
        .await
        .expect("database list");
    assert!(
        databases
            .iter()
            .all(|row| row.get::<String, _>("name") != "archive_db")
    );
    drop(connection);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_task_batch(pool: &SqlitePool, occurred_at: &str, start: i64, rows: i64) {
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT ?1 UNION ALL SELECT i+1 FROM n WHERE i < ?1+?2-1)
         INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response)
         SELECT 'task-local-'||i,?3,'proxy','success',7,0.07,'{}','{}' FROM n",
    ).bind(start).bind(rows).bind(occurred_at).execute(pool).await.expect("seed task-local batch");
}

async fn retag_staged_archive_proof(pool: &SqlitePool, archive_path: &Path) {
    let archive_path = archive_path.to_string_lossy().to_string();
    let sha256 = crate::maintenance::sha256_hex_file(Path::new(&archive_path))
        .expect("hash mutated staged archive");
    let (
        archive_batch_id,
        _old_manifest_sha256,
        page_count,
        row_count,
        coverage_start,
        coverage_end,
        semantic_sha256,
        verified_at,
    ) = sqlx::query_as::<
        _,
        (
            i64,
            String,
            i64,
            i64,
            Option<String>,
            Option<String>,
            String,
            String,
        ),
    >(
        "SELECT proof.archive_batch_id, proof.manifest_sha256, proof.page_count, proof.row_count,
                proof.coverage_start, proof.coverage_end, proof.semantic_sha256, proof.verified_at
         FROM summary_archive_snapshot_v2_proof AS proof
         JOIN archive_batches AS manifest ON manifest.id = proof.archive_batch_id
         WHERE manifest.dataset='codex_invocations' AND manifest.file_path=?1
           AND manifest.status='materializing'
         LIMIT 1",
    )
    .bind(&archive_path)
    .fetch_one(pool)
    .await
    .expect("load staged archive proof before retag");
    sqlx::query(
        "UPDATE archive_batches SET sha256=?1
         WHERE dataset='codex_invocations' AND file_path=?2 AND status='materializing'",
    )
    .bind(&sha256)
    .bind(&archive_path)
    .execute(pool)
    .await
    .expect("retag staged archive manifest");
    sqlx::query(
        "UPDATE summary_archive_snapshot SET manifest_sha256=?1
         WHERE archive_batch_id IN (
             SELECT id FROM archive_batches
             WHERE dataset='codex_invocations' AND file_path=?2 AND status='materializing'
         )",
    )
    .bind(&sha256)
    .bind(&archive_path)
    .execute(pool)
    .await
    .expect("retag staged Summary pages");
    sqlx::query(
        "UPDATE summary_archive_snapshot_v2_proof SET manifest_sha256=?1
         WHERE archive_batch_id IN (
             SELECT id FROM archive_batches
             WHERE dataset='codex_invocations' AND file_path=?2 AND status='materializing'
         )",
    )
    .bind(&sha256)
    .bind(&archive_path)
    .execute(pool)
    .await
    .expect("retag staged Summary proof");
    let inserted = sqlx::query(
        "INSERT INTO summary_archive_snapshot_v2_proof
            (archive_batch_id, manifest_sha256, page_count, row_count, coverage_start,
             coverage_end, semantic_sha256, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(archive_batch_id)
    .bind(&sha256)
    .bind(page_count)
    .bind(row_count)
    .bind(coverage_start)
    .bind(coverage_end)
    .bind(semantic_sha256)
    .bind(verified_at)
    .execute(pool)
    .await
    .expect("restore staged Summary proof after retag");
    assert_eq!(inserted.rows_affected(), 1);
}

fn assert_no_task_work_files(directory: &Path) {
    for entry in fs::read_dir(directory).expect("list archive directory") {
        let entry = entry.expect("archive entry");
        if entry.file_type().expect("archive entry type").is_dir() {
            assert_no_task_work_files(&entry.path());
        } else {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains(".task-") && !name.ends_with(".restore"),
                "unfinished file: {name}"
            );
        }
    }
}

fn has_task_work_files(directory: &Path) -> bool {
    fs::read_dir(directory)
        .expect("list archive directory")
        .any(|entry| {
            let entry = entry.expect("archive entry");
            if entry.file_type().expect("archive entry type").is_dir() {
                has_task_work_files(&entry.path())
            } else {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains(".task-") || name.ends_with(".restore")
            }
        })
}

async fn wait_for_no_task_work_files(directory: &Path) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if !has_task_work_files(directory) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "unfinished task work files did not drain before the bounded wait"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn retention_task_local_older_same_month_append_preserves_archive_expiry() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("task-local-late-expiry").await;
    let select_sql = "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2";
    let mut first_expiry = None;
    let mut first_path = None;
    for (invoke_id, occurred_at, expected_rows) in [
        ("expiry-newer", "2020-01-20 12:00:00", 1i64),
        ("expiry-older", "2020-01-05 12:00:00", 2i64),
    ] {
        sqlx::query("INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status) VALUES(?1,?2,?3,'/v1/responses','pool',0,0,0,'success')")
            .bind(expected_rows).bind(invoke_id).bind(occurred_at).execute(&pool).await.expect("late same-month attempt");
        let archived = crate::maintenance::retention_test_with_work_budget(
            Duration::from_secs(60),
            archive_timestamped_dataset(
                &pool,
                &config,
                archive_table_spec("pool_upstream_request_attempts"),
                select_sql,
                "2020-02-01 00:00:00".to_string(),
                false,
                None,
            ),
        )
        .await
        .expect("close task-local attempt batch");
        assert_eq!(archived.0, 1);
        let manifest = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT file_path,coverage_end_at,archive_expires_at,row_count FROM archive_batches WHERE dataset='pool_upstream_request_attempts'",
        )
        .fetch_one(&pool)
        .await
        .expect("same-month completed manifest");
        assert_eq!(manifest.1, "2020-01-20 12:00:00");
        assert_eq!(manifest.3, expected_rows);
        if let Some(expiry) = &first_expiry {
            assert_eq!(
                &manifest.2, expiry,
                "older rows cannot shorten the monthly TTL"
            );
            assert_eq!(Some(&manifest.0), first_path.as_ref());
        } else {
            first_expiry = Some(manifest.2);
            first_path = Some(manifest.0);
        }
        assert_no_task_work_files(&config.archive_dir);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(&pool)
            .await
            .expect("all source transitions committed"),
        0,
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_batch_reselects_live_rows_without_main_schema_changes() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-month-batches").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1_200).await;
    let schema_before = sqlx::query_as::<_, (String, String)>(
        "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("source schema before retention");
    let first = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("first task");
    assert_eq!(first.invocation_rows_archived, 1_000);
    assert_eq!(
        first
            .batches
            .iter()
            .filter(|batch| batch.dataset == "codex_invocations")
            .map(|batch| batch.batch_rows)
            .collect::<Vec<_>>(),
        vec![1_000]
    );
    let second = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("second task");
    assert_eq!(second.invocation_rows_archived, 200);
    let manifests = sqlx::query_as::<_, (String, i64, String)>("SELECT file_path,row_count,status FROM archive_batches WHERE dataset='codex_invocations' AND summary_source_kind='authoritative'")
        .fetch_all(&pool).await.expect("monthly manifests");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].1, 1_200);
    assert_eq!(manifests[0].2, ARCHIVE_STATUS_COMPLETED);
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    assert_eq!(
        Path::new(&manifests[0].0),
        archive_batch_file_path(&config, "codex_invocations", &month).expect("canonical target")
    );
    let prepared: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM retention_prepared_archives")
        .fetch_one(&pool)
        .await
        .expect("prepared count");
    assert_eq!(prepared, 0);
    assert_no_task_work_files(&config.archive_dir);
    let schema_after = sqlx::query_as::<_, (String, String)>(
        "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("source schema after retention");
    let changed_names = schema_before
        .iter()
        .chain(&schema_after)
        .filter(|entry| !schema_before.contains(entry) || !schema_after.contains(entry))
        .map(|entry| &entry.0)
        .collect::<Vec<_>>();
    assert!(
        changed_names.is_empty(),
        "business schema changed: {changed_names:?}"
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_complete_work_unit_proves_797_rows_once() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-complete-work-unit-797").await;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 797).await;
    let page_stores = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let summary = RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_STORES
        .scope(
            page_stores.clone(),
            run_data_retention_maintenance(&pool, &config, Some(false), None),
        )
        .await
        .expect("complete the selected retention work unit");

    assert_eq!(summary.invocation_rows_archived, 797);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.work_budget_ms, None);
    assert_eq!(summary.timeout_count, None);
    assert!(!summary.budget_exhausted);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source row count after complete work unit"),
        0
    );

    let manifest_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM archive_batches
         WHERE dataset='codex_invocations' AND summary_source_kind='authoritative'",
    )
    .fetch_one(&pool)
    .await
    .expect("authoritative manifest count");
    assert_eq!(manifest_count, 1);

    let (batch_id, manifest_rows, status, manifest_sha) =
        sqlx::query_as::<_, (i64, i64, String, String)>(
            "SELECT id, row_count, status, sha256
             FROM archive_batches
             WHERE dataset='codex_invocations' AND summary_source_kind='authoritative'",
        )
        .fetch_one(&pool)
        .await
        .expect("one authoritative monthly manifest");
    assert_eq!(manifest_rows, 797);
    assert_eq!(status, ARCHIVE_STATUS_COMPLETED);

    let (page_count, page_rows) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(row_count), 0)
         FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2",
    )
    .bind(batch_id)
    .bind(&manifest_sha)
    .fetch_one(&pool)
    .await
    .expect("Summary page counts");
    assert_eq!((page_count, page_rows), (2, 797));
    let (proof_pages, proof_rows) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT page_count, row_count
         FROM summary_archive_snapshot_v2_proof
         WHERE archive_batch_id=?1 AND manifest_sha256=?2",
    )
    .bind(batch_id)
    .bind(&manifest_sha)
    .fetch_one(&pool)
    .await
    .expect("Summary proof counts");
    assert_eq!((proof_pages, proof_rows), (2, 797));
    assert_eq!(
        page_stores.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "each newly scanned Summary page is stored once"
    );

    let batch = summary
        .batches
        .iter()
        .find(|batch| batch.dataset == "codex_invocations")
        .expect("invocation batch metrics");
    assert_eq!(batch.batch_rows, 797);
    assert_eq!(batch.committed_rows, 797);
    assert!(batch.file_prepare_ms.is_some());
    assert!(batch.source_identity_ms.is_some());
    assert!(batch.archive_writer_ms.is_some());
    assert!(batch.summary_proof_ms.is_some());
    let serialized_batch = serde_json::to_value(batch).expect("serialize invocation metrics");
    assert!(serialized_batch.get("lockWaitMs").is_some());

    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_snapshot_pages_survive_cancel_and_resume() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-summary-page-resume").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month)
        .expect("canonical archive target");
    let first_page_committed = Arc::new(tokio::sync::Notify::new());
    let mut first_work = Box::pin(RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_PAUSE.scope(
        Duration::from_secs(60),
        RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_COMMITTED.scope(
            first_page_committed.clone(),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = first_page_committed.notified() => {}
            result = &mut first_work => panic!("archive completed before the injected page pause: {result:?}"),
        }
    })
    .await
    .expect("first Summary Snapshot page is durably committed");
    drop(first_work);

    assert!(
        target.exists(),
        "published archive must survive cancellation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1",
        )
        .bind(shanghai_local_cutoff_string(config.invocation_max_days))
        .fetch_one(&pool)
        .await
        .expect("live rows remain after cancellation"),
        401
    );
    let staged = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT status, row_count, (SELECT COUNT(*) FROM summary_archive_snapshot)
         FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1",
    )
    .bind(target.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("staged archive manifest");
    assert_eq!(staged.0, ARCHIVE_STATUS_MATERIALIZING);
    assert_eq!(staged.1, 401);
    assert_eq!(staged.2, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT row_count FROM summary_archive_snapshot
             WHERE archive_batch_id=(SELECT id FROM archive_batches WHERE file_path=?1)
             AND page_index=0",
        )
        .bind(target.to_string_lossy().to_string())
        .fetch_one(&pool)
        .await
        .expect("first durable Summary Snapshot page"),
        400
    );
    let page_prefix_reused = Arc::new(tokio::sync::Notify::new());
    let mut resumed_work = Box::pin(RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_REUSED.scope(
        page_prefix_reused.clone(),
        archive_old_invocations(&pool, &config, config.database_path.parent(), false),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = page_prefix_reused.notified() => {}
            result = &mut resumed_work => panic!("resume completed without loading the durable Summary Snapshot page: {result:?}"),
        }
    })
    .await
    .expect("resume loads the durable Summary Snapshot page");
    let resumed = resumed_work
        .await
        .expect("resume from the durable Summary Snapshot page");
    assert_eq!(resumed.0, 401);
    let settled = sqlx::query_as::<_, (String, i64, i64, i64)>(
        "SELECT status, row_count,
                (SELECT COUNT(*) FROM summary_archive_snapshot),
                (SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof)
         FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1",
    )
    .bind(target.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("completed archive manifest");
    assert_eq!(settled.0, ARCHIVE_STATUS_COMPLETED);
    assert_eq!(settled.1, 401);
    assert_eq!(settled.2, 2);
    assert_eq!(settled.3, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("all source rows converted after resume"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_first_manifest_insert_preserves_predecessor_identity() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-summary-predecessor-identity").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 4).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM codex_invocations ORDER BY id LIMIT 3")
        .fetch_all(&pool)
        .await
        .expect("legacy archive source IDs");
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month)
        .expect("canonical archive target");
    let legacy = archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("codex_invocations"),
        &month,
        &ids,
    )
    .await
    .expect("publish legacy archive without a Summary manifest");
    assert_eq!(legacy.row_count, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM archive_batches
             WHERE dataset='codex_invocations' AND file_path=?1",
        )
        .bind(target.to_string_lossy().to_string())
        .fetch_one(&pool)
        .await
        .expect("legacy manifest count"),
        0
    );

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let probe = Arc::new(crate::maintenance::RetentionReturnToPoolProbe {
        operation: Some("archive_snapshot_manifest_stage"),
        ..Default::default()
    });
    probe
        .pause_at_return
        .store(1, std::sync::atomic::Ordering::Release);
    let work = RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator.clone(),
        RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_RETURN_TO_POOL.scope(
                probe.clone(),
                archive_old_invocations(&pool, &config, config.database_path.parent(), false),
            ),
        ),
    );
    let mut work = Box::pin(work);
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = probe.returned.notified() => {}
            result = &mut work => panic!("archive completed before predecessor manifest pause: {result:?}"),
        }
    })
    .await
    .expect("pause after predecessor manifest staging");
    drop(work);
    probe.resume.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if coordinator.snapshot().await.active_write_class.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("manifest cancellation cleanup releases admission");

    let staged = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT status, sha256, row_count
         FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1",
    )
    .bind(target.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("staged predecessor manifest");
    assert_eq!(staged.0, ARCHIVE_STATUS_MATERIALIZING);
    assert_eq!(staged.1, legacy.sha256);
    assert_eq!(staged.2, legacy.row_count);
    assert_eq!(
        sha256_hex_file(&target).expect("predecessor file digest"),
        legacy.sha256
    );

    let resumed = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("retry replaces the predecessor archive");
    assert_eq!(resumed.0, 4);
    let settled = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT status, sha256, row_count
         FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1",
    )
    .bind(target.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("completed replacement manifest");
    assert_eq!(settled.0, ARCHIVE_STATUS_COMPLETED);
    assert_ne!(settled.1, legacy.sha256);
    assert_eq!(settled.2, 4);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("all source rows convert after predecessor retry"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_same_sha_page_replay_preserves_final_proof() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-summary-page-proof-replay").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let completed = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("complete monthly archive before same-SHA replay");
    assert_eq!(completed.0, 401);

    let (batch_id, file_path, sha256, row_count, coverage_start_at, coverage_end_at): (
        i64,
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT id, file_path, sha256, row_count, coverage_start_at, coverage_end_at
         FROM archive_batches
         WHERE dataset='codex_invocations' AND month_key=?1",
    )
    .bind(&month)
    .fetch_one(&pool)
    .await
    .expect("load completed Summary archive manifest");
    let pages = sqlx::query_as::<_, (i64, String, String, i64, Vec<u8>)>(
        "SELECT page_index, coverage_start, coverage_end, row_count, payload
         FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2
         ORDER BY page_index",
    )
    .bind(batch_id)
    .bind(&sha256)
    .fetch_all(&pool)
    .await
    .expect("load completed Summary archive pages")
    .into_iter()
    .map(
        |(page_index, coverage_start, coverage_end, row_count, payload)| {
            (
                u32::try_from(page_index).expect("page index fits test type"),
                TaskArchiveSnapshotPage {
                    coverage_start,
                    coverage_end,
                    row_count: u32::try_from(row_count).expect("page row count fits test type"),
                    payload,
                },
            )
        },
    )
    .collect::<Vec<_>>();
    let outcome = ArchiveBatchOutcome {
        dataset: HOURLY_ROLLUP_DATASET_INVOCATIONS,
        month_key: month,
        day_key: None,
        part_key: None,
        file_path,
        sha256,
        source_identity_sha256: None,
        row_count,
        upstream_last_activity: Vec::new(),
        coverage_start_at,
        coverage_end_at,
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    };
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof")
            .fetch_one(&pool)
            .await
            .expect("existing final Summary proof"),
        1
    );
    let original_page_payload = pages
        .first()
        .map(|(_, page)| page.payload.clone())
        .expect("completed Summary archive has a page");
    let original_manifest_fields: (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT day_key, part_key, archive_expires_at
             FROM archive_batches WHERE id=?1",
        )
        .bind(batch_id)
        .fetch_one(&pool)
        .await
        .expect("load original proven manifest fields");
    let mut stale_manifest_outcome = outcome.clone();
    stale_manifest_outcome.day_key = Some("stale-day".to_string());
    stale_manifest_outcome.part_key = Some("stale-part".to_string());
    stale_manifest_outcome.archive_expires_at = Some("2099-01-01T00:00:00".to_string());
    assert_eq!(
        crate::maintenance::stage_task_summary_archive_manifest(
            &pool,
            &stale_manifest_outcome,
            Some(&outcome.sha256),
            Some(outcome.row_count),
        )
        .await
        .expect("same-SHA manifest replay"),
        Some(batch_id)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof")
            .fetch_one(&pool)
            .await
            .expect("final Summary proof after same-SHA manifest replay"),
        1
    );
    let persisted_manifest_fields: (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT day_key, part_key, archive_expires_at
             FROM archive_batches WHERE id=?1",
        )
        .bind(batch_id)
        .fetch_one(&pool)
        .await
        .expect("load preserved proven manifest");
    assert_eq!(persisted_manifest_fields, original_manifest_fields);
    for (page_index, mut page) in pages {
        if page_index == 0 {
            // Simulate a stale retry selected before proof publication. A valid final proof is an
            // authority fence, so the retry must leave the proven page set untouched even when
            // the replacement payload is not decodable.
            page.payload = b"stale-retry-page".to_vec();
        }
        crate::maintenance::store_task_summary_snapshot_page(
            &pool, &outcome, batch_id, page_index, page,
        )
        .await
        .expect("same-SHA page replay")
        .expect("same-SHA page replay admission");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof")
            .fetch_one(&pool)
            .await
            .expect("final Summary proof after same-SHA replay"),
        1
    );
    let retained_page_payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=0",
    )
    .bind(batch_id)
    .bind(&outcome.sha256)
    .fetch_one(&pool)
    .await
    .expect("load retained proven Summary page");
    assert_eq!(retained_page_payload, original_page_payload);
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_snapshot_rebuilds_semantically_stale_page() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-summary-page-semantic-rebuild").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;
    let first_page_committed = Arc::new(tokio::sync::Notify::new());
    let mut first_work = Box::pin(RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_PAUSE.scope(
        Duration::from_secs(60),
        RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_COMMITTED.scope(
            first_page_committed.clone(),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = first_page_committed.notified() => {}
            result = &mut first_work => panic!("archive completed before the injected page pause: {result:?}"),
        }
    })
    .await
    .expect("first Summary Snapshot page is durably committed");
    drop(first_work);

    let (batch_id, manifest_sha256): (i64, String) = sqlx::query_as(
        "SELECT id, sha256 FROM archive_batches
         WHERE dataset='codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load staged Summary archive identity");
    let payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=0",
    )
    .bind(batch_id)
    .bind(&manifest_sha256)
    .fetch_one(&pool)
    .await
    .expect("load committed Summary page payload");
    let mut records =
        crate::summary_source_change::decode_summary_archive_snapshot_v2_payload(&payload)
            .expect("decode committed Summary page payload");
    records[0].status = "semantically-corrupted".to_string();
    let normalized = serde_json::to_vec(&records).expect("encode corrupted Summary page payload");
    let corrupted_payload = zstd::stream::encode_all(normalized.as_slice(), 1)
        .expect("compress corrupted Summary page");
    sqlx::query(
        "UPDATE summary_archive_snapshot
         SET payload=?1, payload_bytes=?2
         WHERE archive_batch_id=?3 AND manifest_sha256=?4 AND page_index=0",
    )
    .bind(&corrupted_payload)
    .bind(corrupted_payload.len() as i64)
    .bind(batch_id)
    .bind(&manifest_sha256)
    .execute(&pool)
    .await
    .expect("mutate decodable Summary page payload");

    let resumed = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("resume rebuilds the semantically stale Summary page");
    assert_eq!(resumed.0, 1);
    let rebuilt_payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=0",
    )
    .bind(batch_id)
    .bind(&manifest_sha256)
    .fetch_one(&pool)
    .await
    .expect("load rebuilt Summary page payload");
    let rebuilt_records =
        crate::summary_source_change::decode_summary_archive_snapshot_v2_payload(&rebuilt_payload)
            .expect("decode rebuilt Summary page payload");
    assert_eq!(rebuilt_records[0].status, "success");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows convert after semantic page rebuild"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_snapshot_rebuilds_non_prefix_page() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-summary-page-prefix-rebuild").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 401).await;
    let first_page_committed = Arc::new(tokio::sync::Notify::new());
    let mut first_work = Box::pin(RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_PAUSE.scope(
        Duration::from_secs(60),
        RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_COMMITTED.scope(
            first_page_committed.clone(),
            archive_old_invocations(&pool, &config, config.database_path.parent(), false),
        ),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = first_page_committed.notified() => {}
            result = &mut first_work => panic!("archive completed before the injected page pause: {result:?}"),
        }
    })
    .await
    .expect("first Summary Snapshot page is durably committed");
    drop(first_work);

    let (batch_id, manifest_sha256): (i64, String) = sqlx::query_as(
        "SELECT id, sha256 FROM archive_batches
         WHERE dataset='codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load staged Summary archive identity");
    let payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=0",
    )
    .bind(batch_id)
    .bind(&manifest_sha256)
    .fetch_one(&pool)
    .await
    .expect("load committed Summary page payload");
    let mut records =
        crate::summary_source_change::decode_summary_archive_snapshot_v2_payload(&payload)
            .expect("decode committed Summary page payload");
    assert_eq!(records.len(), 400);
    records.remove(0);
    let normalized = serde_json::to_vec(&records).expect("encode non-prefix Summary page payload");
    let corrupted_payload = zstd::stream::encode_all(normalized.as_slice(), 1)
        .expect("compress non-prefix Summary page");
    sqlx::query(
        "UPDATE summary_archive_snapshot
         SET payload=?1, payload_bytes=?2, row_count=?3
         WHERE archive_batch_id=?4 AND manifest_sha256=?5 AND page_index=0",
    )
    .bind(&corrupted_payload)
    .bind(corrupted_payload.len() as i64)
    .bind(records.len() as i64)
    .bind(batch_id)
    .bind(&manifest_sha256)
    .execute(&pool)
    .await
    .expect("mutate Summary page into a non-prefix subset");

    let resumed = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("resume rebuilds the non-prefix Summary page");
    assert_eq!(resumed.0, 401);
    let settled = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(row_count), 0)
         FROM summary_archive_snapshot
         WHERE archive_batch_id=?1 AND manifest_sha256=?2",
    )
    .bind(batch_id)
    .bind(&manifest_sha256)
    .fetch_one(&pool)
    .await
    .expect("load rebuilt Summary pages");
    assert_eq!(settled, (2, 401));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source rows convert after non-prefix page rebuild"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_rebuilds_published_live_copy_and_cleans_failed_work() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-failed-reselection").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 3).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM codex_invocations ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("source IDs");
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target =
        archive_batch_file_path(&config, "codex_invocations", &month).expect("canonical target");
    // Publication without source conversion is not an input checkpoint for the next task.
    archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("codex_invocations"),
        &month,
        &ids,
    )
    .await
    .expect("publish source copy");
    sqlx::query("UPDATE codex_invocations SET payload='{\"changed\":true}' WHERE id=?1")
        .bind(ids[0])
        .execute(&pool)
        .await
        .expect("live mutation");
    let stale = PathBuf::from(format!("{}.task-2000000000-1-0.sqlite", target.display()));
    fs::write(&stale, b"discard, never resume").expect("abandoned task work");
    let result = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("fresh source selection");
    assert_eq!(result.0, 3);
    assert!(!stale.exists());
    let decoded = temp_dir.join("published.sqlite");
    inflate_gzip_sqlite_file(&target, &decoded).expect("decode final file");
    let mut connection = open_archive_sqlite_connection(&decoded)
        .await
        .expect("open final file");
    let payload: String = sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE id=?1")
        .bind(ids[0])
        .fetch_one(&mut connection)
        .await
        .expect("new source payload");
    assert_eq!(payload, "{\"changed\":true}");
    connection.close().await.expect("close decoded file");
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_bad_month_file_keeps_source_and_removes_temporary_files() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-corrupt-file").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 3).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month).expect("target");
    fs::create_dir_all(target.parent().expect("parent")).expect("archive directory");
    fs::write(&target, b"corrupt monthly archive").expect("corrupt file fixture");
    assert!(
        archive_old_invocations(&pool, &config, config.database_path.parent(), false)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("live count"),
        3
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_attempt_failure_reports_prior_committed_invocations() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-partial-failure").await;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago(
        (config
            .invocation_max_days
            .max(config.pool_upstream_request_attempts_retention_days)
            + 2) as i64,
        12,
        0,
        0,
    );
    seed_task_batch(&pool, &occurred_at, 0, 512).await;
    sqlx::query("INSERT INTO pool_upstream_request_attempts(invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status) VALUES('task-local-0',?1,'/v1/responses','pool',0,0,0,'success')")
        .bind(&occurred_at).execute(&pool).await.expect("attempt fixture");
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "pool_upstream_request_attempts", &month)
        .expect("attempt target");
    fs::create_dir_all(target.parent().expect("target directory")).expect("archive directory");
    fs::write(&target, b"invalid gzip fixture").expect("corrupt attempt archive");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("structured failure result");
    assert_eq!(summary.invocation_rows_archived, 512);
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 0);
    assert_eq!(summary.completion(), "failed");
    assert!(
        summary
            .fatal_error
            .as_deref()
            .is_some_and(|error| error.contains("attempt archive failed"))
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(&pool)
            .await
            .expect("unproved attempts remain"),
        1
    );
    assert_eq!(
        summary
            .batches
            .iter()
            .filter(|batch| batch.dataset == "codex_invocations")
            .map(|batch| batch.committed_rows)
            .sum::<usize>(),
        512
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_isolates_old_prepared_rows_without_resuming_their_ids() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-old-prepared").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 2).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month).expect("target");
    sqlx::query("INSERT INTO retention_prepared_archives(prepared_key,dataset,month_key,file_path,source_ids_json,source_identity_sha256,state) VALUES('abandoned','codex_invocations',?1,?2,'not-a-continuation','invalid','preparing')")
        .bind(month).bind(target.to_string_lossy().as_ref()).execute(&pool).await.expect("old prepared fixture");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("fresh task");
    assert_eq!(summary.invocation_rows_archived, 2);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM retention_prepared_archives WHERE prepared_key='abandoned'"
        )
        .fetch_one(&pool)
        .await
        .expect("isolated old row"),
        "quarantined"
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

/// Backfill fixtures need all 1217 source rows in one monthly target. They use two closed
/// retention tasks rather than relying on a single run to exceed its selected file batch.
pub(super) async fn archive_two_attempt_tasks(
    pool: &SqlitePool,
    config: &AppConfig,
    expected: usize,
) {
    let first = run_data_retention_maintenance(pool, config, Some(false), None)
        .await
        .expect("first closed attempt task");
    assert_eq!(first.pool_upstream_request_attempt_rows_archived, 1_000);
    let tail = run_data_retention_maintenance(pool, config, Some(false), None)
        .await
        .expect("second task reselects live attempt tail");
    assert_eq!(
        first.pool_upstream_request_attempt_rows_archived
            + tail.pool_upstream_request_attempt_rows_archived,
        expected
    );
}

#[tokio::test]
async fn retention_task_local_partial_source_commit_preserves_exact_summary_and_reselects() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-partial-source").await;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1_000).await;
    let before = query_combined_totals(&pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("exact totals before source conversion");
    sqlx::query("CREATE TRIGGER task_local_stop_second_chunk BEFORE DELETE ON codex_invocations WHEN OLD.id=65 BEGIN SELECT RAISE(ABORT,'injected source conversion failure'); END")
        .execute(&pool).await.expect("stop after a committed source chunk");
    let observation = crate::TaskExecutionObservation::begin(
        "retention_archive",
        "Retention partial source commit",
        "manual",
        Some("maintenance_retention"),
        "processing",
    );
    let failed = run_data_retention_maintenance_with_circuit_and_prompt_cache(
        &pool,
        &config,
        Some(false),
        None,
        Arc::new(RawCaptureCircuitBreaker::new(config.archive_dir.clone())),
        None,
        Some(observation.clone()),
    )
    .await
    .expect("structured source failure");
    observation.finish_with_status("failed");
    let workload = crate::task_runtime_observation::workload_sample("retention_archive")
        .expect("failed run retains committed workload");
    assert_eq!(
        workload.pending.as_ref().and_then(|metric| metric.value),
        Some(1_000)
    );
    assert_eq!(
        workload.discovered.as_ref().and_then(|metric| metric.value),
        Some(1_000)
    );
    assert_eq!(
        workload.processed.as_ref().and_then(|metric| metric.value),
        Some(64)
    );
    assert_eq!(workload.status, "failed");
    assert!(failed.fatal_error.is_some());
    assert_eq!(failed.invocation_rows_archived, 64);
    assert_eq!(
        failed
            .batches
            .iter()
            .filter(|b| b.dataset == "codex_invocations")
            .map(|b| b.committed_rows)
            .sum::<usize>(),
        64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("uncommitted sources"),
        936
    );
    let day_key = shanghai_day_key_from_local_naive(&occurred_at).expect("archive day");
    let full_batch_ids = (1..=1_000).collect::<Vec<_>>();
    let part_key = archive_segment_part_key_for_ids(&full_batch_ids).expect("archive part");
    let archive_path = archive_segment_file_path(
        &config,
        "codex_invocations",
        &day_key,
        &part_key,
        config.invocation_archive_codec,
    )
    .expect("archive path");
    assert!(
        archive_path.is_file(),
        "partial archive artifact must remain recoverable"
    );
    let inflated_path = temp_dir.join("partial-source-archive.sqlite");
    inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &inflated_path)
        .await
        .expect("inflate repaired partial archive");
    let mut archive_connection = open_archive_sqlite_connection(&inflated_path)
        .await
        .expect("open repaired partial archive");
    let archive_row_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&mut archive_connection)
        .await
        .expect("count repaired partial archive rows");
    archive_connection
        .close()
        .await
        .expect("close repaired partial archive");
    assert_eq!(archive_row_count, 64);
    let archive_path = archive_path.to_string_lossy().to_string();
    let repaired_manifest = sqlx::query_as::<_, (i64, String)>(
        "SELECT row_count, sha256 FROM archive_batches
         WHERE dataset='codex_invocations' AND file_path=?1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load repaired partial archive manifest");
    assert_eq!(repaired_manifest.0, 64);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay
             WHERE dataset='codex_invocations' AND file_path=?1 AND archive_sha256=?2",
        )
        .bind(&archive_path)
        .bind(&repaired_manifest.1)
        .fetch_one(&pool)
        .await
        .expect("load replay markers for repaired partial archive"),
        INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64
    );
    let after = query_combined_totals(&pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("exact totals after partial source conversion");
    assert_eq!(before.total_count, after.total_count);
    assert_eq!(before.total_tokens, after.total_tokens);
    assert_f64_close(before.total_cost, after.total_cost);
    assert_no_task_work_files(&config.archive_dir);
    sqlx::query("DROP TRIGGER task_local_stop_second_chunk")
        .execute(&pool)
        .await
        .expect("remove source failure");
    let next = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("fresh task selects remaining live sources");
    assert_eq!(next.invocation_rows_archived, 936);
    let settled = query_combined_totals(&pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("settled exact totals");
    assert_eq!(before.total_count, settled.total_count);
    assert_eq!(before.total_tokens, settled.total_tokens);
    assert_f64_close(before.total_cost, settled.total_cost);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM retention_prepared_archives")
            .fetch_one(&pool)
            .await
            .expect("no continuation"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_partial_timestamped_archive_reselects_without_duplicates() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("task-local-partial-timestamped-archive").await;
    config.retention_batch_rows = 1_000;
    config.pool_upstream_request_attempts_retention_days = config.invocation_max_days;
    let occurred_at = shanghai_local_days_ago(
        (config.pool_upstream_request_attempts_retention_days + 2) as i64,
        12,
        0,
        0,
    );
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000)
         INSERT INTO pool_upstream_request_attempts
             (id, attempt_public_id, invoke_id, occurred_at, finished_at, endpoint, route_mode,
              attempt_index, distinct_account_index, same_account_retry_index, status)
         SELECT i, 'partial-attempt-'||i, 'partial-invocation-'||i, ?1, ?1,
                '/v1/responses', 'pool', 0, 0, 0, 'success'
         FROM n",
    )
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("timestamped archive candidates");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let probe = Arc::new(crate::maintenance::BatchCommitProbe::default());
    let archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &occurred_at[..7])
            .expect("timestamped archive path");
    let select_sql = "SELECT id, occurred_at AS timestamp_value
                      FROM pool_upstream_request_attempts
                      WHERE occurred_at < ?1
                      ORDER BY occurred_at ASC, id ASC LIMIT ?2";
    let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator,
        crate::maintenance::RETENTION_TEST_BATCH_COMMIT.scope(
            probe.clone(),
            crate::maintenance::retention_test_with_work_budget(
                Duration::from_secs(60),
                archive_timestamped_dataset(
                    &pool,
                    &config,
                    archive_table_spec("pool_upstream_request_attempts"),
                    select_sql,
                    "9999-01-01 00:00:00".to_string(),
                    false,
                    None,
                ),
            ),
        ),
    );
    let competitor = async {
        probe.committed.notified().await;
        sqlx::query(
            "UPDATE archive_batches
             SET cleanup_state=?1
             WHERE dataset=?2 AND month_key=?3 AND file_path=?4",
        )
        .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
        .bind("pool_upstream_request_attempts")
        .bind(&occurred_at[..7])
        .bind(archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark timestamped archive cleanup pending");
        probe.resume.notify_one();
    };
    let (first, ()) = tokio::join!(work, competitor);
    let first = first.expect("timestamped archive should defer after a committed prefix");
    assert_eq!(first.0, 64);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts",)
            .fetch_one(&pool)
            .await
            .expect("remaining live timestamped rows"),
        936
    );

    let manifest_row_count: i64 = sqlx::query_scalar(
        "SELECT row_count FROM archive_batches
         WHERE dataset='pool_upstream_request_attempts' AND file_path=?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("partial timestamped archive manifest");
    assert_eq!(manifest_row_count, 64);
    let inflated_path = temp_dir.join("partial-timestamped-archive.sqlite");
    inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &inflated_path)
        .await
        .expect("inflate repaired timestamped archive");
    let mut archive_connection = open_archive_sqlite_connection(&inflated_path)
        .await
        .expect("open repaired timestamped archive");
    let archive_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(&mut archive_connection)
            .await
            .expect("count repaired timestamped archive rows");
    archive_connection
        .close()
        .await
        .expect("close repaired timestamped archive");
    assert_eq!(archive_row_count, 64);

    let second = crate::maintenance::retention_test_with_work_budget(
        Duration::from_secs(60),
        archive_timestamped_dataset(
            &pool,
            &config,
            archive_table_spec("pool_upstream_request_attempts"),
            select_sql,
            "9999-01-01 00:00:00".to_string(),
            false,
            None,
        ),
    )
    .await
    .expect("reselect remaining timestamped rows");
    assert_eq!(second.0, 936);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_request_attempts",)
            .fetch_one(&pool)
            .await
            .expect("all timestamped source rows converted"),
        0
    );
    let final_manifest_row_count: i64 = sqlx::query_scalar(
        "SELECT row_count FROM archive_batches
         WHERE dataset='pool_upstream_request_attempts' AND file_path=?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("final timestamped archive manifest");
    assert_eq!(final_manifest_row_count, 1_000);
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_writer_reads_the_shared_memory_source() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("task-local-memory-source").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;
    let mut source = SqliteConnection::connect_with(pool.connect_options().as_ref())
        .await
        .expect("open independent source reader");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM main.codex_invocations")
            .fetch_one(&mut source)
            .await
            .expect("independent reader shares the source schema"),
        1
    );
    source.close().await.expect("close source reader");
    let result = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("archive from the live shared memory database");
    assert_eq!(result.0, 1);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
#[ignore = "shared-testbox 32 MiB filesystem acceptance"]
async fn retention_task_local_low_disk_space_keeps_live_sources() {
    let (pool, mut config, temp_dir) = retention_test_pool_and_config("task-local-low-disk").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    config.archive_dir = PathBuf::from(
        std::env::var_os("CVM_RETENTION_LIMITED_ARCHIVE_DIR")
            .expect("explicit limited test filesystem"),
    );
    assert!(crate::filesystem_available_bytes(&config.archive_dir).unwrap() < 64 * 1024 * 1024);
    config.retention_batch_rows = 1_000;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1_000).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month).expect("target");
    fs::create_dir_all(target.parent().expect("archive parent")).expect("archive parent");
    let abandoned = PathBuf::from(format!(
        "{}.task-{}-1-0.sqlite",
        target.display(),
        std::process::id()
    ));
    fs::write(&abandoned, b"abandoned work on a constrained filesystem")
        .expect("abandoned fixture");
    let result = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("low disk admission is a safe defer");
    assert_eq!(result.0, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("all unproved sources remain live"),
        1_000
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM archive_batches")
            .fetch_one(&pool)
            .await
            .expect("no archive publication"),
        0
    );
    assert_no_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_month_writer_reads_the_disk_source() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("task-local-disk-source").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_task_batch(&pool, &occurred_at, 0, 1).await;
    let result = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("archive from the live disk database");
    assert_eq!(result.0, 1);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
