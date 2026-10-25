use super::*;

#[tokio::test]
async fn retention_compacts_old_quota_snapshots_by_shanghai_day() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-quota").await;
    let same_day_early = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
    let same_day_late = utc_naive_from_shanghai_local_days_ago(40, 23, 0, 0);
    let next_day = utc_naive_from_shanghai_local_days_ago(39, 9, 0, 0);
    seed_quota_snapshot(&pool, &same_day_early).await;
    seed_quota_snapshot(&pool, &same_day_late).await;
    seed_quota_snapshot(&pool, &next_day).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run quota compaction");
    assert_eq!(summary.quota_snapshot_rows_archived, 1);

    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT captured_at FROM codex_quota_snapshots ORDER BY captured_at ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load remaining quota snapshots");
    assert_eq!(remaining, vec![same_day_late.clone(), next_day.clone()]);

    let quota_batch_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_quota_snapshots'",
    )
    .fetch_one(&pool)
    .await
    .expect("count quota archive batches");
    assert_eq!(quota_batch_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_orphan_sweep_skips_fresh_raw_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-orphan-grace").await;
    let orphan = config.proxy_raw_dir.join("fresh-orphan.bin");
    fs::write(&orphan, b"fresh-orphan").expect("write fresh orphan");

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention with fresh orphan");
    assert_eq!(summary.orphan_raw_files_removed, 0);
    assert!(orphan.exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_recovery_persistence_failure_does_not_scan_orphan_raw_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-recovery-orphan-independence").await;
    let orphan = config.proxy_raw_dir.join("aged-orphan.bin");
    fs::write(&orphan, b"safe-to-sweep").expect("write aged orphan");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let old_attempt_at = shanghai_local_days_ago(
        config.forward_proxy_attempts_retention_days as i64 + 2,
        12,
        0,
        0,
    );
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe
        ) VALUES ('retention-recovery-independence', ?1, 1, 12.0, NULL, 0)
        "#,
    )
    .bind(old_attempt_at)
    .execute(&pool)
    .await
    .expect("seed independent forward proxy archive candidate");

    sqlx::query("DROP TABLE retention_prepared_archives")
        .execute(&pool)
        .await
        .expect("break recovery failure persistence after schema setup");

    let result = run_data_retention_maintenance(&pool, &config, Some(false), None).await;
    assert!(
        result.is_ok(),
        "recovery status refresh failure must not abort independent stages: {result:?}"
    );

    assert!(
        orphan.exists(),
        "unlinked raw residuals remain untouched while recovery fails"
    );
    let archived_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'forward_proxy_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count independent forward proxy archive batches");
    assert_eq!(archived_attempts, 1);
    let live_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = 'retention-recovery-independence'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining forward proxy attempts");
    assert_eq!(live_attempts, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archive_finalization_failure_keeps_live_rows_and_reselects_without_journal() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-archive-recovery").await;
    let occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
    let raw_path = config.proxy_raw_dir.join("retention-recovery-response.bin");
    fs::write(&raw_path, b"still-owned-until-publication").expect("write raw response");
    let orphan_path = config.proxy_raw_dir.join("retention-recovery-orphan.bin");
    fs::write(&orphan_path, b"safe-to-sweep").expect("write unreferenced raw file");
    set_file_mtime_seconds_ago(&orphan_path, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    insert_retention_invocation(
        &pool,
        "retention-recovery-row",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"error\":true}",
        None,
        Some(&raw_path),
        Some(12),
        Some(0.12),
    )
    .await;
    let query_plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM proxy_raw_payload_blob_links WHERE raw_path = ?1)",
    )
    .bind(raw_path.to_string_lossy().as_ref())
    .fetch_all(&pool)
    .await
    .expect("explain raw ownership lookup");
    assert!(
        query_plan
            .iter()
            .any(|entry| entry.3.contains("idx_proxy_raw_payload_blob_links_path")),
        "raw ownership lookup must use the path index: {query_plan:?}"
    );
    sqlx::query(
        r#"
        CREATE TRIGGER retention_test_abort_archive_finalize
        BEFORE UPDATE OF status ON archive_batches
        WHEN NEW.status = 'completed'
        BEGIN
            SELECT RAISE(
                ABORT,
                'injected archive finalization failure retention-test-secret /private/retention-test/raw.bin account-id-secret SELECT_SECRET'
            );
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install finalization failure trigger");

    let error = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect_err("injected finalization failure should stop the archive batch");
    assert!(
        error
            .to_string()
            .contains("injected archive finalization failure")
    );
    let live_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'retention-recovery-row'",
    )
    .fetch_one(&pool)
    .await
    .expect("check live invocation after rollback");
    assert_eq!(live_count, 1);
    assert!(
        raw_path.exists(),
        "raw payload remains source-owned before publication"
    );
    let journal_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM retention_prepared_archives")
        .fetch_one(&pool)
        .await
        .expect("no task continuation journal");
    assert_eq!(journal_count, 0);
    let (published_path, status): (String, String) = sqlx::query_as(
        "SELECT file_path,status FROM archive_batches WHERE dataset='codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("existing manifest fact");
    assert!(Path::new(&published_path).is_file());
    assert_eq!(status, ARCHIVE_STATUS_MATERIALIZING);
    let failed = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("structured task failure");
    assert!(failed.fatal_error.is_some());
    assert_eq!(failed.invocation_rows_archived, 0);
    assert!(raw_path.exists());
    let fingerprint = failed.fatal_error.expect("sanitized fatal result");
    for sensitive in [
        "retention-test-secret",
        "/private/retention-test/raw.bin",
        "account-id-secret",
        "SELECT_SECRET",
    ] {
        assert!(!fingerprint.contains(sensitive));
    }
    sqlx::query("DROP TRIGGER retention_test_abort_archive_finalize")
        .execute(&pool)
        .await
        .expect("remove finalization failure trigger");
    let recovered = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("retry published archive finalization after removing injected failure");
    assert_eq!(recovered.orphan_raw_files_removed, 0);
    assert!(
        !raw_path.exists(),
        "raw payload is released only after publication"
    );
    let remaining_journal_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_prepared_archives")
            .fetch_one(&pool)
            .await
            .expect("count recovery journal after successful retry");
    assert_eq!(remaining_journal_count, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_retires_expired_quarantined_archive_without_digest() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-quarantine-retirement").await;
    let archive_root = config.archive_dir.join("codex_invocations");
    fs::create_dir_all(&archive_root).expect("create archive root");
    let archive_path = archive_root.join("stale-quarantined.sqlite.gz");
    fs::write(&archive_path, b"unverified archive artifact").expect("write stale artifact");
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, artifact_sha256, quarantined_at, updated_at
        ) VALUES ('stale-quarantine', 'codex_invocations', '2026-01', ?1, '[]',
                  'source-identity', 'quarantined', NULL, datetime('now', '-2 days'), datetime('now', '-2 days'))
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed stale quarantined archive");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("retention should retire stale quarantined archive");

    assert!(
        !archive_path.exists(),
        "expired owned quarantine artifact is removed"
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE prepared_key = 'stale-quarantine'",
    )
    .fetch_one(&pool)
    .await
    .expect("count retired quarantine journal");
    assert_eq!(remaining, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_quarantines_invalid_publication_kind_instead_of_retrying() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-invalid-publication-kind").await;
    let archive_root = config.archive_dir.join("codex_invocations");
    fs::create_dir_all(&archive_root).expect("create archive root");
    let archive_path = archive_root.join("legacy-unknown-kind.sqlite.gz");
    fs::write(&archive_path, b"unverified legacy artifact").expect("write legacy artifact");
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, publication_kind, updated_at
        ) VALUES ('invalid-publication-kind', 'codex_invocations', '2026-01', ?1, '[]',
                  'source-identity', 'preparing', 'unsupported_kind', datetime('now'))
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed invalid publication kind");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("retention should isolate invalid publication kind");

    let (state, next_retry_at, last_failure_stage): (String, Option<String>, Option<String>) =
        sqlx::query_as(
        "SELECT state, next_retry_at, last_failure_stage FROM retention_prepared_archives WHERE prepared_key = 'invalid-publication-kind'",
    )
    .fetch_one(&pool)
    .await
    .expect("load invalid publication kind journal");
    assert_eq!(state, "quarantined");
    assert_eq!(next_retry_at, None);
    assert_eq!(last_failure_stage.as_deref(), Some("prepared_reconcile"));
    let (failure_count, defer_reason): (i64, Option<String>) = sqlx::query_as(
        "SELECT consecutive_failure_count, defer_reason FROM retention_recovery_cursors WHERE scope = 'prepared_archives'",
    )
    .fetch_one(&pool)
    .await
    .expect("load durable invalid-kind failure cursor");
    assert_eq!(failure_count, 0);
    assert_eq!(defer_reason, None);
    assert!(
        archive_path.exists(),
        "quarantine retains the artifact for evidence"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_quarantine_clears_existing_recovery_backoff() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-quarantine-clears-backoff").await;
    let archive_root = config.archive_dir.join("codex_invocations");
    fs::create_dir_all(&archive_root).expect("create archive root");
    let archive_path = archive_root.join("legacy-unknown-kind.sqlite.gz");
    fs::write(&archive_path, b"unverified legacy artifact").expect("write legacy artifact");
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, publication_kind, updated_at
        ) VALUES ('quarantine-clears-backoff', 'codex_invocations', '2026-01', ?1, '[]',
                  'source-identity', 'preparing', 'unsupported_kind', datetime('now'))
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed invalid publication kind");
    sqlx::query(
        "UPDATE retention_recovery_cursors
         SET next_retry_at = datetime('now', '+300 seconds'),
             consecutive_failure_count = 4,
             last_failure_fingerprint = 'previous-failure',
             defer_reason = 'retry_backoff'
         WHERE scope = 'prepared_archives'",
    )
    .execute(&pool)
    .await
    .expect("seed existing recovery backoff");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let persisted = crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            coordinator,
            crate::maintenance::retention_recovery_persist_prepared_failure_best_effort(
                &pool,
                "quarantine-clears-backoff",
                &anyhow::anyhow!("prepared archive publication kind is invalid"),
            ),
        )
        .await;
    assert!(persisted, "quarantine failure state must persist");

    let (state, next_retry_at, last_failure_stage): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT state, next_retry_at, last_failure_stage
             FROM retention_prepared_archives WHERE prepared_key = 'quarantine-clears-backoff'",
        )
        .fetch_one(&pool)
        .await
        .expect("load quarantined archive");
    assert_eq!(state, "quarantined");
    assert_eq!(next_retry_at, None);
    assert_eq!(last_failure_stage.as_deref(), Some("prepared_reconcile"));

    let cursor: (Option<String>, i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT next_retry_at, consecutive_failure_count, last_failure_fingerprint, defer_reason
         FROM retention_recovery_cursors WHERE scope = 'prepared_archives'",
    )
    .fetch_one(&pool)
    .await
    .expect("load cleared recovery backoff");
    assert_eq!(cursor, (None, 0, None, None));
    assert!(
        archive_path.exists(),
        "quarantine retains the artifact for evidence"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_quarantines_missing_published_archive_instead_of_retrying() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-missing-published-archive").await;
    let archive_path = config
        .archive_dir
        .join("codex_invocations")
        .join("missing-published.sqlite.gz");
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, artifact_sha256, updated_at
        ) VALUES ('missing-published-archive', 'codex_invocations', '2026-01', ?1, '[]',
                  'source-identity', 'published', 'expected-sha', datetime('now'))
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed missing published archive");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("retention should quarantine missing published archive");

    let (state, next_retry_at, last_failure_stage): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT state, next_retry_at, last_failure_stage
             FROM retention_prepared_archives
             WHERE prepared_key = 'missing-published-archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("load missing published archive journal");
    assert_eq!(state, "quarantined");
    assert_eq!(next_retry_at, None);
    assert_eq!(last_failure_stage.as_deref(), Some("prepared_reconcile"));

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_pressure_defer_keeps_legacy_prepared_rows_unconsumed() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-prepared-pressure-cursor").await;
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, updated_at
        ) VALUES ('pressure-cursor', 'codex_invocations', '2026-01', ?1, '[]',
                  'source-identity', 'preparing', datetime('now'))
        "#,
    )
    .bind(
        config
            .archive_dir
            .join("codex_invocations")
            .join("missing.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .execute(&pool)
    .await
    .expect("seed prepared pressure cursor");
    sqlx::query(
        "UPDATE retention_recovery_cursors SET next_retry_at = NULL, defer_reason = NULL WHERE scope = 'prepared_archives'",
    )
    .execute(&pool)
    .await
    .expect("make prepared cursor due");

    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        std::time::Duration::from_secs(60),
    ));
    let _busy_permit = pressure_gate
        .try_begin_background("retention_prepared_pressure_cursor")
        .expect("occupy the test background pressure slot");
    let summary = crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE
        .scope(
            pressure_gate,
            run_data_retention_maintenance(&pool, &config, Some(false), None),
        )
        .await
        .expect("pressure defer should keep independent retention stages alive");

    assert!(summary.deferred);
    assert_eq!(summary.invocation_rows_archived, 0);
    let state: String = sqlx::query_scalar(
        "SELECT state FROM retention_prepared_archives WHERE prepared_key='pressure-cursor'",
    )
    .fetch_one(&pool)
    .await
    .expect("pressure does not consume old source IDs");
    assert_eq!(state, "preparing");

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_finalization_rejects_source_content_changed_after_archive_copy() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-source-content-identity").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    let occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
    let raw_path = config.proxy_raw_dir.join("source-content-identity.bin");
    fs::write(&raw_path, b"owned-until-verified-publication").expect("write raw response");
    insert_retention_invocation(
        &pool,
        "retention-source-content-identity",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"error\":true}",
        None,
        Some(&raw_path),
        Some(12),
        Some(0.12),
    )
    .await;
    sqlx::query(
        r#"
        CREATE TRIGGER retention_test_mutate_archived_source
        AFTER INSERT ON archive_batches
        WHEN NEW.dataset = 'codex_invocations' AND NEW.status = 'materializing'
        BEGIN
            UPDATE codex_invocations
            SET payload = '{"error":"changed-during-finalization"}'
                , input_tokens = NULL
                , raw_response = CAST('changed-storage-class' AS BLOB)
            WHERE invoke_id = 'retention-source-content-identity';
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install source-content mutation trigger");

    let error = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect_err("changed archive-covered source fields must prevent publication");
    assert!(
        error
            .to_string()
            .contains("source changed before conversion")
    );
    let source = sqlx::query(
        "SELECT payload FROM codex_invocations WHERE invoke_id = 'retention-source-content-identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("source row remains after verification rollback");
    assert_eq!(
        source.get::<Option<String>, _>("payload").as_deref(),
        Some("{\"error\":\"changed-during-finalization\"}")
    );
    assert!(
        raw_path.exists(),
        "raw data remains source-owned after rollback"
    );

    sqlx::query("DROP TRIGGER retention_test_mutate_archived_source")
        .execute(&pool)
        .await
        .expect("remove source-content mutation trigger");
    let retry = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("retry publication after the source identity is stable");
    assert_eq!(retry.0, 1);
    assert!(
        !raw_path.exists(),
        "raw data is released after verified publication"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn persisted_recovery_failure_keeps_its_prepared_key_for_outer_error_handling() {
    let error = crate::maintenance::retention_recovery_failure_persisted(
        "codex_invocations:/tmp/archive-a:sha-a",
        anyhow::anyhow!("source identity verification failed"),
    );

    assert!(crate::maintenance::is_retention_recovery_failure_persisted(
        &error
    ));
    assert!(
        error
            .to_string()
            .contains("source identity verification failed")
    );
}
