use super::*;

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
    let failed = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("structured source failure");
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
