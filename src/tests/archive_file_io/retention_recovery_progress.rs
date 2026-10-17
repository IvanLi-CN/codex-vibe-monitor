use super::*;

#[tokio::test]
async fn retention_rejects_archive_replacement_after_writer_proof_before_admission() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-recovery-file-replacement").await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    seed_recovery_invocation(&pool, &occurred_at).await;
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let archive_path =
        archive_batch_file_path(&config, "codex_invocations", &month).expect("archive path");
    let probe = Arc::new(crate::maintenance::ArchiveArtifactProbe::default());
    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let work = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR.scope(
        coordinator,
        crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE.scope(
            gate,
            crate::maintenance::RETENTION_TEST_ARCHIVE_ARTIFACT.scope(
                probe.clone(),
                crate::maintenance::retention_test_with_work_budget(
                    Duration::from_secs(60),
                    archive_old_invocations(&pool, &config, config.database_path.parent(), false),
                ),
            ),
        ),
    );
    let replace_archive = async {
        tokio::time::timeout(Duration::from_secs(10), probe.verified.notified())
            .await
            .expect("archive artifact proof reached before replacement");
        let mut bytes = fs::read(&archive_path).expect("read published archive");
        assert!(bytes.len() > 8, "published archive must have mutable bytes");
        let replacement_index = bytes.len() / 2;
        bytes[replacement_index] ^= 1;
        fs::write(&archive_path, bytes).expect("replace published archive");
        probe.resume.notify_one();
    };
    let (result, ()) = tokio::join!(work, replace_archive);
    let error = result.expect_err("first admission must reject the replaced archive");
    assert!(
        error
            .to_string()
            .contains("archive artifact changed while awaiting retention admission")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("source remains after archive replacement"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM archive_batches")
            .fetch_one(&pool)
            .await
            .expect("archive manifest remains unpublished"),
        0
    );
    assert_no_recovery_task_work_files(&config.archive_dir);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_recovery_invocation(pool: &sqlx::SqlitePool, occurred_at: &str) {
    sqlx::query(
        "INSERT INTO codex_invocations
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response)
         VALUES ('retention-recovery-file-replacement', ?1, 'proxy', 'success', 7, 0.07, '{}', '{}')",
    )
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("seed recovery invocation");
}

fn assert_no_recovery_task_work_files(directory: &std::path::Path) {
    for entry in fs::read_dir(directory).expect("list archive directory") {
        let entry = entry.expect("archive entry");
        if entry.file_type().expect("archive entry type").is_dir() {
            assert_no_recovery_task_work_files(&entry.path());
        } else {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains(".task-") && !name.ends_with(".restore"),
                "unfinished file: {name}"
            );
        }
    }
}
