async fn seed_upstream_activity_account(pool: &SqlitePool, account_id: i64, display_name: &str) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(account_id)
    .bind("api_key_codex")
    .bind("codex")
    .bind(display_name)
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(pool)
    .await
    .expect("insert upstream account");
}

async fn write_full_activity_archive(
    temp_dir: &Path,
    archive_path: &Path,
    invoke_id: &str,
    occurred_at: &str,
    account_id: i64,
) {
    fs::create_dir_all(
        archive_path
            .parent()
            .expect("archived invocation batch should have parent"),
    )
    .expect("create archived invocation batch dir");

    let archive_db_path = temp_dir.join("upstream-last-activity-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, created_at, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind(invoke_id)
    .bind(occurred_at)
    .bind("{}")
    .bind(occurred_at)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert archived invocation");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, archive_path)
        .expect("compress archived invocation batch");
}

async fn register_activity_archive_manifest(
    pool: &SqlitePool,
    month_key: &str,
    archive_path: &Path,
) {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(archive_path).expect("archive sha256"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(pool)
    .await
    .expect("insert archive batch manifest");
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_backfill_reads_archived_batches() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("upstream-last-activity-archive-backfill").await;
    let account_id = 501_i64;
    seed_upstream_activity_account(&pool, account_id, "Archived-only account").await;

    let occurred_at = shanghai_local_days_ago(120, 9, 30, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve archived invocation batch");
    write_full_activity_archive(
        &temp_dir,
        &archive_path,
        "archived-upstream-activity",
        &occurred_at,
        account_id,
    )
    .await;
    register_activity_archive_manifest(&pool, &month_key, &archive_path).await;

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("rebuild archive upstream activity manifest");
    assert_eq!(refresh.refreshed_batches, 1);
    assert_eq!(refresh.account_rows_written, 1);

    backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill upstream last activity from archives");

    let last_activity_at: Option<String> =
        sqlx::query_scalar("SELECT last_activity_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load persisted last activity");
    assert_eq!(last_activity_at.as_deref(), Some(occurred_at.as_str()));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_archive_backfill_retries_after_failed_progress() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let pool = state.pool.clone();

    let task_name = STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES;
    let retry_due = format_utc_iso(Utc::now() - ChronoDuration::seconds(1));
    mark_startup_backfill_running(&pool, task_name, 0)
        .await
        .expect("seed running startup progress");
    save_startup_backfill_progress(
        &pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &retry_due,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed failed startup progress");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("retry failed archive backfill progress");

    let progress = load_startup_backfill_progress(&pool, task_name)
        .await
        .expect("load startup backfill progress");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(progress.last_finished_at.is_some());
    assert!(!progress.is_due(Utc::now()));
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_archive_backfill_marks_exhausted_accounts_complete() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityArchives,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    let account_id = 902_i64;

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(account_id)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Never used account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("run archive activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load archive backfill progress");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_scanned, 0);

    let completed: i64 = sqlx::query_scalar(
        r#"
        SELECT last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load archive completion flag");
    assert_eq!(completed, 1);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force archive task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("rerun archive activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("reload archive backfill progress");
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_live_backfill_marks_unmatched_rows_complete() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityLive,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(903_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("No live invocation")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityLive)
        .await
        .expect("run live activity backfill");

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_live_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(903_i64)
    .fetch_one(&state.pool)
    .await
    .expect("load live backfill row");
    assert!(row.0.is_none());
    assert_eq!(row.1, 1);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force live task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityLive)
        .await
        .expect("rerun live activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load live backfill progress");
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_archive_backfill_keeps_pending_when_archive_missing() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityArchives,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(904_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Missing archive account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind("/tmp/definitely-missing-upstream-activity.sqlite.gz")
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert missing archive manifest");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("run archive activity backfill with missing file");

    let completed: i64 = sqlx::query_scalar(
        r#"
        SELECT last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(904_i64)
    .fetch_one(&state.pool)
    .await
    .expect("load archive completion flag");
    assert_eq!(completed, 0);

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load archive backfill progress");
    assert_eq!(progress.last_updated, 0);
}

async fn write_activity_archive_batch(
    temp_dir: &Path,
    month_key: &str,
    suffix: &str,
    occurred_at: &str,
    account_id: i64,
) -> ArchiveBatchOutcome {
    let archive_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite.gz"));
    let archive_db_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite"));
    let archive_url = format!("sqlite://{}", archive_db_path.to_string_lossy());
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            build_sqlite_connect_options(
                &archive_url,
                Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
            )
            .expect("build archive sqlite options"),
        )
        .await
        .expect("open archive sqlite");

    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            requester TEXT,
            occurred_at TEXT NOT NULL,
            request_method TEXT,
            payload TEXT
        )
        "#,
    )
    .execute(&archive_pool)
    .await
    .expect("create archive codex_invocations");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, requester, occurred_at, request_method, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(format!("archive-{suffix}"))
    .bind("archived-upstream-activity")
    .bind(occurred_at)
    .bind("{}")
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert archived invocation");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress archived invocation batch");

    ArchiveBatchOutcome {
        dataset: "codex_invocations",
        month_key: month_key.to_string(),
        day_key: None,
        part_key: None,
        file_path: archive_path.to_string_lossy().to_string(),
        sha256: sha256_hex_file(&archive_path).expect("archive sha256"),
        row_count: 1,
        upstream_last_activity: vec![(account_id, occurred_at.to_string())],
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    }
}

async fn persist_activity_archive_batch(pool: &SqlitePool, batch: &ArchiveBatchOutcome) {
    let mut tx = pool.begin().await.expect("begin archive batch tx");
    upsert_archive_batch_manifest(tx.as_mut(), batch)
        .await
        .expect("upsert archive batch manifest");
    tx.commit().await.expect("commit archive batch manifest");
}

async fn load_archive_activity_backfill_row(
    pool: &SqlitePool,
    account_id: i64,
) -> (Option<String>, i64) {
    sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load archive backfill row")
}

#[tokio::test]
pub(crate) async fn upstream_last_activity_archive_backfill_refreshes_existing_activity_when_new_archive_arrives()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let pool = state.pool.clone();
    let temp_dir = make_temp_test_dir("upstream-archive-activity-refresh");
    let account_id = 905_i64;
    seed_upstream_activity_account(&pool, account_id, "Archive refresh account").await;

    let first_activity_at = format_utc_iso(Utc::now() - ChronoDuration::days(14));
    let first_batch = write_activity_archive_batch(
        &temp_dir,
        "2025-01",
        "first",
        &first_activity_at,
        account_id,
    )
    .await;
    persist_activity_archive_batch(&pool, &first_batch).await;
    backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill first archive activity");
    let first_row = load_archive_activity_backfill_row(&pool, account_id).await;
    assert_eq!(first_row.0.as_deref(), Some(first_activity_at.as_str()));
    assert_eq!(first_row.1, 0);

    let second_activity_at = format_utc_iso(Utc::now() - ChronoDuration::days(1));
    let second_batch = write_activity_archive_batch(
        &temp_dir,
        "2025-02",
        "second",
        &second_activity_at,
        account_id,
    )
    .await;
    persist_activity_archive_batch(&pool, &second_batch).await;
    let refreshed_row = load_archive_activity_backfill_row(&pool, account_id).await;
    assert_eq!(
        refreshed_row.0.as_deref(),
        Some(second_activity_at.as_str())
    );
    assert_eq!(refreshed_row.1, 0);

    cleanup_temp_test_dir(&temp_dir);
}

async fn register_manifest_rebuild_archive(
    pool: &SqlitePool,
    month_key: &str,
    archive_path: &Path,
    occurred_at: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(archive_path).expect("manifest backlog archive sha"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("insert manifest backlog batch");
}

#[tokio::test]
pub(crate) async fn archive_backfill_waits_for_manifest_until_rebuilt() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-rebuild").await;
    let account_id = 991_i64;
    seed_upstream_activity_account(&pool, account_id, "Manifest backlog account").await;
    let occurred_at = shanghai_local_days_ago(120, 9, 45, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve manifest backlog archive path");
    write_full_activity_archive(
        &temp_dir,
        &archive_path,
        "manifest-backlog-row",
        &occurred_at,
        account_id,
    )
    .await;
    register_manifest_rebuild_archive(&pool, &month_key, &archive_path, &occurred_at).await;

    let waiting = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("run archive backfill before manifest rebuild");
    assert!(waiting.waiting_for_manifest_backfill);
    assert_eq!(waiting.updated_accounts, 0);

    let dry_run = refresh_archive_upstream_activity_manifest(&pool, &config, true)
        .await
        .expect("dry-run manifest rebuild");
    assert_eq!(dry_run.pending_batches, 1);
    assert_eq!(dry_run.refreshed_batches, 1);
    assert_eq!(dry_run.account_rows_written, 1);

    let rebuild = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("live manifest rebuild");
    assert_eq!(rebuild.pending_batches, 1);
    assert_eq!(rebuild.refreshed_batches, 1);
    assert_eq!(rebuild.account_rows_written, 1);

    let summary = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("run archive backfill after manifest rebuild");
    assert!(!summary.waiting_for_manifest_backfill);
    assert_eq!(summary.updated_accounts, 1);

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load manifest backlog account row");
    assert_eq!(row.0.as_deref(), Some(occurred_at.as_str()));
    assert_eq!(row.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_manifest_refresh_leaves_missing_batches_pending_for_retry() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("manifest-missing-terminal").await;
    let account_id = 993_i64;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(account_id)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Missing manifest archive account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert missing manifest account");

    let occurred_at = shanghai_local_days_ago(90, 10, 15, 0);
    let month_key = occurred_at[..7].to_string();
    let missing_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve missing archive batch path");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(missing_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing manifest batch");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh manifest with missing archive file");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.refreshed_batches, 0);
    assert_eq!(refresh.missing_files, 1);

    let refreshed_at: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load missing batch retry marker");
    assert!(refreshed_at.is_none());

    let summary = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill upstream activity while waiting for missing batch retry");
    assert!(summary.waiting_for_manifest_backfill);
    assert_eq!(summary.updated_accounts, 0);

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load missing manifest account row");
    assert!(row.0.is_none());
    assert_eq!(row.1, 0);

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_duplicate_archive_rows(
    pool: &SqlitePool,
    config: &AppConfig,
    account_id: i64,
    row_count: usize,
) -> String {
    let base_occurred_at = parse_shanghai_local_naive(&shanghai_local_days_ago(120, 9, 0, 0))
        .expect("valid shanghai local");
    let mut newest_occurred_at = String::new();
    for idx in 0..row_count {
        let occurred_at = format_naive(base_occurred_at + ChronoDuration::seconds(idx as i64));
        newest_occurred_at = occurred_at.clone();
        let response_raw = config
            .proxy_raw_dir
            .join(format!("duplicate-account-{idx}.bin.gz"));
        write_gzip_test_file(
            &response_raw,
            format!("{{\"index\":{idx},\"accountId\":{account_id}}}").as_bytes(),
        );
        insert_retention_invocation_with_fixture(
            pool,
            RetentionInvocationFixture {
                invoke_id: &format!("duplicate-account-{idx}"),
                occurred_at: &occurred_at,
                source: SOURCE_PROXY,
                status: "success",
                payload: Some(
                    &json!({ "endpoint": "/v1/responses", "upstreamAccountId": account_id })
                        .to_string(),
                ),
                raw_response: "{\"ok\":true}",
                request_raw_path: None,
                response_raw_path: Some(&response_raw),
                total_tokens: Some(42),
                cost: Some(0.42),
            },
        )
        .await;
    }
    newest_occurred_at
}

#[tokio::test]
pub(crate) async fn retention_archives_duplicate_upstream_activity_across_chunks() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-archive-manifest-dedupe").await;
    config.retention_batch_rows = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;

    let account_id = 995_i64;
    seed_upstream_activity_account(&pool, account_id, "Duplicate archive account").await;
    let row_count = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;
    let newest_occurred_at =
        seed_duplicate_archive_rows(&pool, &config, account_id, row_count).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention archive for duplicate account rows");
    assert_eq!(summary.invocation_rows_archived, row_count);
    assert!(summary.raw_files_removed >= row_count);

    let manifest_rows = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, MAX(last_activity_at) AS last_activity_at
        FROM archive_batch_upstream_activity
        GROUP BY account_id
        ORDER BY account_id ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load latest account activity across archive segments");
    assert_eq!(
        manifest_rows,
        vec![(account_id, newest_occurred_at.clone())]
    );

    let last_activity_at: Option<String> =
        sqlx::query_scalar("SELECT last_activity_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load updated account activity");
    assert_eq!(
        last_activity_at.as_deref(),
        Some(newest_occurred_at.as_str())
    );

    let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("count remaining live invocations");
    assert_eq!(live_count, 0);
    assert_eq!(
        fs::read_dir(&config.proxy_raw_dir)
            .expect("read raw dir after archive cleanup")
            .count(),
        0
    );

    cleanup_temp_test_dir(&temp_dir);
}

use super::*;
