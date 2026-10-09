use super::*;

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
    assert_no_task_work_files(&config.archive_dir);
    let next = crate::maintenance::compact_old_quota_snapshots(&pool, &config, false)
        .await.expect("fresh live-row selection");
    assert_eq!(next, (1, 1));
    let remaining: Vec<String> = sqlx::query_scalar("SELECT captured_at FROM codex_quota_snapshots")
        .fetch_all(&pool).await.expect("daily representative remains");
    assert_eq!(remaining, vec![late]);
    assert_no_task_work_files(&config.archive_dir);
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
    assert_no_task_work_files(&config.archive_dir);
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
async fn retention_task_local_rebuilds_published_live_copy_and_cleans_failed_work() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("task-local-failed-reselection").await;
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
    let (pool, config, temp_dir) = retention_test_pool_and_config("task-local-corrupt-file").await;
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
