use super::*;
use serde_json::json;

async fn insert_summary_archive_snapshot_proof(
    pool: &SqlitePool,
    archive_batch_id: i64,
    manifest_sha256: &str,
    coverage_start: &str,
    coverage_end: &str,
    row_count: u32,
) {
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id,
        manifest_sha256: manifest_sha256.to_string(),
        page_index: 0,
        coverage_start: coverage_start.to_string(),
        coverage_end: coverage_end.to_string(),
        row_count,
        payload: b"normalized-summary-test-fixture".to_vec(),
    };
    let mut tx = pool
        .begin()
        .await
        .expect("begin Summary Snapshot proof transaction");
    store_summary_archive_snapshot_page_tx(tx.as_mut(), &page)
        .await
        .expect("store Summary Snapshot proof");
    tx.commit().await.expect("commit Summary Snapshot proof");
}

#[tokio::test]
async fn archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-dedupe").await;
    let primary_account_id = 996_i64;
    let secondary_account_id = 997_i64;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [
        (primary_account_id, "Manifest duplicate primary"),
        (secondary_account_id, "Manifest duplicate secondary"),
    ] {
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
        .execute(&pool)
        .await
        .expect("insert manifest refresh account");
    }

    let base_occurred_at = parse_shanghai_local_naive(&shanghai_local_days_ago(120, 8, 0, 0))
        .expect("valid shanghai local");
    let month_key = format_naive(base_occurred_at)[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve archive manifest refresh path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create archive manifest refresh parent");

    let archive_db_path = temp_dir.join("archive-manifest-refresh-dedupe.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive schema");

    let repeated_rows = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;
    let mut primary_latest = String::new();
    let mut secondary_latest = String::new();
    for idx in 0..repeated_rows {
        let occurred_at = format_naive(base_occurred_at + ChronoDuration::seconds(idx as i64));
        primary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(idx as i64 + 1)
        .bind(format!("manifest-refresh-primary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": primary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated primary manifest row");
    }
    for idx in 0..2 {
        let occurred_at = format_naive(
            base_occurred_at + ChronoDuration::seconds(repeated_rows as i64 + idx as i64 + 1),
        );
        secondary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(repeated_rows as i64 + idx as i64 + 1)
        .bind(format!("manifest-refresh-secondary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": secondary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated secondary manifest row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress manifest refresh archive");

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
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha"))
    .bind((repeated_rows + 2) as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(format_naive(base_occurred_at))
    .bind(secondary_latest.clone())
    .execute(&pool)
    .await
    .expect("insert manifest refresh batch");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh manifest rows for duplicate accounts");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.refreshed_batches, 1);
    assert_eq!(refresh.account_rows_written, 2);

    let manifest_rows = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        ORDER BY account_id ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load refreshed manifest rows");
    assert_eq!(
        manifest_rows,
        vec![
            (primary_account_id, primary_latest),
            (secondary_account_id, secondary_latest),
        ]
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_refresh_respects_the_retention_candidate_budget() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-budget").await;
    config.retention_batch_rows = 1;
    let created_at = format_utc_iso(Utc::now());

    for id in 1_i64..=3 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, 'missing', 1, ?3, ?4)
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("missing-manifest-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert missing manifest candidate");
    }

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh bounded manifest candidates");
    let pending_after_refresh: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE upstream_activity_manifest_refreshed_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count pending manifest candidates");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.missing_files, 1);
    assert_eq!(pending_after_refresh, 3);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_activity_replacement_clears_and_writes_in_micro_batches() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-activity-micro-batches").await;
    config.retention_batch_rows = 2;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id, dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES (99, 'codex_invocations', '2026-01', 'memory://manifest-activity', 'manifest', 5, ?1, ?2)
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert archive manifest batch");
    for account_id in 10_i64..15 {
        sqlx::query(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) VALUES (99, ?1, ?2)",
        )
        .bind(account_id)
        .bind("2026-01-01 00:00:00")
        .execute(&pool)
        .await
        .expect("seed stale manifest activity");
    }

    let next_values = (20_i64..25)
        .map(|account_id| (account_id, format!("2026-01-01 00:00:{account_id:02}")))
        .collect::<Vec<_>>();
    let written = replace_archive_batch_upstream_activity_in_micro_batches(
        &pool,
        &config,
        99,
        &next_values,
        0,
    )
    .await
    .expect("replace manifest activity in micro batches");
    let actual = sqlx::query_as::<_, (i64, String)>(
        "SELECT account_id, last_activity_at FROM archive_batch_upstream_activity WHERE archive_batch_id = 99 ORDER BY account_id ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load rebuilt manifest activity");
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE id = 99",
    )
    .fetch_one(&pool)
    .await
    .expect("load manifest refresh marker");
    assert_eq!(written, Some(5));
    assert_eq!(actual, next_values);
    assert!(marker.is_some());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_persistent_prep_skips_mutations_for_dry_run_commands() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("startup-prep-dry-run").await;
    let occurred_at = shanghai_local_days_ago(45, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();

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
    .bind(
        temp_dir
            .join("pending-manifest.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest batch");

    let cli = CliArgs {
        command: Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: true }),
        })),
        ..Default::default()
    };

    let summary = run_startup_persistent_prep(&pool, &config, &cli)
        .await
        .expect("run startup prep for dry-run maintenance command");
    assert_eq!(summary.refreshed_manifest_batches, 0);
    assert_eq!(summary.backfilled_archive_expiries, 0);
    assert!(!summary.bootstrapped_hourly_rollups);

    let refreshed_at: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load manifest refreshed flag");
    assert!(refreshed_at.is_none());

    let archive_expires_at: Option<String> = sqlx::query_scalar(
        "SELECT archive_expires_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive expiry");
    assert!(archive_expires_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_persistent_prep_keeps_manifest_backlog_pending_after_a_bounded_pass() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("startup-prep-bounded-manifest-backlog").await;
    config.retention_batch_rows = 1;
    let occurred_at = shanghai_local_days_ago(45, 9, 0, 0);

    for (index, month_key) in ["2025-01", "2025-02"].into_iter().enumerate() {
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
        .bind(
            temp_dir
                .join(format!("pending-manifest-{index}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("deadbeef-{index}"))
        .bind(1_i64)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert pending manifest batch");
    }

    let summary = run_startup_persistent_prep_inner(&pool, &config, &CliArgs::default(), false)
        .await
        .expect("run bounded startup prep");

    assert_eq!(summary.refreshed_manifest_batches, 0);
    assert_eq!(summary.pending_manifest_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn startup_rollup_bootstrap_runs_in_background_only_for_normal_server_start() {
    let default_cli = CliArgs::default();
    assert!(should_spawn_background_startup_hourly_rollup_bootstrap(
        &default_cli
    ));
    assert!(!should_run_blocking_startup_persistent_prep(&default_cli));

    let retention_cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };
    assert!(!should_spawn_background_startup_hourly_rollup_bootstrap(
        &retention_cli
    ));
    assert!(should_run_blocking_startup_persistent_prep(&retention_cli));

    let maintenance_cli = CliArgs {
        command: Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
        })),
        ..CliArgs::default()
    };
    assert!(!should_spawn_background_startup_hourly_rollup_bootstrap(
        &maintenance_cli
    ));
    assert!(!should_run_blocking_startup_persistent_prep(
        &maintenance_cli
    ));
}

#[tokio::test]
async fn startup_persistent_prep_rebuilds_manifest_before_archive_backfill() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("startup-prep-manifest").await;
    let account_id = 992_i64;
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
    .bind("Startup prep manifest account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert startup prep manifest account");

    let occurred_at = shanghai_local_days_ago(90, 10, 15, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve startup prep archive path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create startup prep archive parent");
    let archive_db_path = temp_dir.join("startup-prep-manifest.sqlite");
    fs::File::create(&archive_db_path).expect("create startup prep sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open startup prep sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create startup prep archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, created_at, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind("startup-prep-manifest-row")
    .bind(&occurred_at)
    .bind("{}")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert startup prep archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress startup prep archive");

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
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("startup prep archive sha"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert startup prep batch");

    let summary = run_startup_persistent_prep(&pool, &config, &CliArgs::default())
        .await
        .expect("run startup persistent prep");
    assert_eq!(summary.refreshed_manifest_batches, 1);
    assert_eq!(summary.refreshed_manifest_account_rows, 1);
    assert_eq!(summary.missing_manifest_files, 0);
    assert!(summary.bootstrapped_hourly_rollups);

    let backfill = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill upstream activity after startup prep");
    assert!(!backfill.waiting_for_manifest_backfill);
    assert_eq!(backfill.updated_accounts, 1);

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
    .expect("load startup prep account row");
    assert_eq!(row.0.as_deref(), Some(occurred_at.as_str()));
    assert_eq!(row.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_backfill_respects_scan_limit_budget() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("archive-backfill-budget").await;
    let created_at = format_utc_iso(Utc::now());
    for account_id in [993_i64, 994_i64] {
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
        .bind(format!("Archive budget account {account_id}"))
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert archive budget account");
    }

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            upstream_activity_manifest_refreshed_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(
        temp_dir
            .join("budget.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert archive budget batch");

    let first_activity_at = shanghai_local_days_ago(20, 8, 0, 0);
    let second_activity_at = shanghai_local_days_ago(19, 9, 0, 0);
    sqlx::query(
        r#"
        INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at)
        VALUES (?1, ?2, ?3), (?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind(993_i64)
    .bind(&first_activity_at)
    .bind(1_i64)
    .bind(994_i64)
    .bind(&second_activity_at)
    .execute(&pool)
    .await
    .expect("insert archive budget manifest rows");

    let first_pass = backfill_upstream_account_last_activity_from_archives(
        &pool,
        Some(1),
        Some(Duration::from_secs(60)),
    )
    .await
    .expect("run first archive budget pass");
    assert_eq!(first_pass.updated_accounts, 1);
    assert!(first_pass.hit_budget);

    let remaining_pending: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining archive backfill accounts");
    assert_eq!(remaining_pending, 1);

    let second_pass = backfill_upstream_account_last_activity_from_archives(
        &pool,
        Some(1),
        Some(Duration::from_secs(60)),
    )
    .await
    .expect("run second archive budget pass");
    assert_eq!(second_pass.updated_accounts, 1);
    assert!(!second_pass.hit_budget);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_invocation_archive_batches_removes_manifest_rows() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup").await;
    config.invocation_archive_ttl_days = 0;

    let archive_path = temp_dir.join("expired-archive.sqlite.gz");
    let archive_db_path = temp_dir.join("expired-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create expired archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open expired invocation archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create expired invocation archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, t_total_ms, created_at
        )
        VALUES (1, 'expired-long-wall-time', '2025-01-01 23:00:00', '{}', ?1, '2025-01-01 23:00:00')
        "#,
    )
    .bind(48.0_f64 * 60.0 * 60.0 * 1000.0)
    .execute(&archive_pool)
    .await
    .expect("insert multi-day expired invocation archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress expired invocation archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash expired invocation archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), ?10)
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert expired invocation archive batch");
    sqlx::query(
        r#"
        INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(1_i64)
    .bind(7_i64)
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert expired invocation archive manifest row");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("mark long-term archive replay complete");
    insert_summary_archive_snapshot_proof(
        &pool,
        1,
        &archive_sha256,
        "2025-01-01 00:00:00",
        "2025-01-01 00:00:00",
        1,
    )
    .await;

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup expired invocation archive batches");
    assert_eq!(deleted, 1);
    assert!(!archive_path.exists());

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 0);
    let remaining_manifest_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batch_upstream_activity")
            .fetch_one(&pool)
            .await
            .expect("count remaining archive manifest rows");
    assert_eq!(remaining_manifest_rows, 0);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load long-term integrity source boundary");
    assert_eq!(
        integrity_source_start.as_deref(),
        Some("2025-01-04"),
        "archive cleanup must exclude every Shanghai day touched by a multi-day wall-time interval"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_archive_keeps_a_missing_materialized_invocation_manifest_after_source_loss()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("cleanup-missing-invocation-manifest-finalize").await;
    let missing_archive_path = temp_dir.join("missing-finalizable-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '2000-01-01 00:00:00', datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-cleanup-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing replayed invocation archive metadata");
    sqlx::query(
        "UPDATE long_term_stats_state SET status = 'error', last_error = 'terminal integrity proof reconciliation is incomplete' WHERE id = 1",
    )
        .execute(&pool)
        .await
        .expect("mark long-term statistics unavailable after source loss");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-cleanup-sha")
    .execute(&pool)
    .await
    .expect("insert long-term replay marker");

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing source manifest during expiry cleanup");
    assert_eq!(deleted, 0);
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count retained replay markers");
    assert_eq!(remaining_markers, 1);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved long-term source boundary");
    assert!(integrity_source_start.is_none());
    let status: String =
        sqlx::query_scalar("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load durable source-loss status");
    assert_eq!(status, "error");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_invocation_archive_requires_every_source_timestamp_to_be_parseable() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-unparseable-source").await;
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "archive-ttl-cleanup-unparseable-source",
        &[
            (
                1_i64,
                "archive-ttl-cleanup-parseable-source",
                "2025-01-01 09:00:00",
                SOURCE_PROXY,
                "success",
                42_i64,
                0.42_f64,
                Some(120.0),
            ),
            (
                2_i64,
                "archive-ttl-cleanup-unparseable-source",
                "invalid-timestamp",
                SOURCE_PROXY,
                "success",
                42_i64,
                0.42_f64,
                Some(120.0),
            ),
        ],
    )
    .await;
    let archive_sha256: String =
        sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = '2025-01-01 00:00:00',
            coverage_end_at = '2025-01-01 00:00:00',
            archive_expires_at = '2000-01-01 00:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?1
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark unparseable archive eligible for cleanup");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark long-term archive replay complete");

    let boundary =
        crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path.to_string_lossy(),
            Some("2025-01-01 00:00:00"),
        )
        .await;
    assert!(
        boundary.is_err(),
        "manifest coverage must not substitute for an unparseable source timestamp"
    );
    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("unparseable source should retain the archive instead of failing cleanup");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());
    let cleanup_state: String =
        sqlx::query_scalar("SELECT cleanup_state FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load retained archive cleanup state");
    assert_eq!(cleanup_state, ARCHIVE_CLEANUP_STATE_ACTIVE);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved source boundary");
    assert!(integrity_source_start.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_invocation_archive_preserves_source_when_metadata_transaction_fails() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-transaction-failure").await;
    config.invocation_archive_ttl_days = 0;
    let occurred_at = "2025-01-01 09:00:00";
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "archive-ttl-cleanup-transaction-failure",
        &[(
            1_i64,
            "archive-ttl-cleanup-transaction-failure",
            occurred_at,
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    let archive_sha256: String =
        sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = ?1,
            coverage_end_at = ?1,
            archive_expires_at = '2025-01-02 00:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?2
        "#,
    )
    .bind(occurred_at)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark archive eligible for cleanup");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("mark long-term archive replay complete");
    let archive_batch_id: i64 =
        sqlx::query_scalar("SELECT id FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive batch id for Snapshot proof");
    insert_summary_archive_snapshot_proof(
        &pool,
        archive_batch_id,
        &archive_sha256,
        occurred_at,
        occurred_at,
        1,
    )
    .await;
    sqlx::query(
        r#"
        CREATE TRIGGER reject_archive_cleanup
        BEFORE UPDATE OF cleanup_state ON archive_batches
        WHEN NEW.cleanup_state = 'delete_pending'
        BEGIN
            SELECT RAISE(ABORT, 'forced archive metadata transaction failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install archive cleanup failure trigger");

    cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect_err("metadata transaction failure should abort archive cleanup");
    assert!(
        archive_path.exists(),
        "the readable archive source must remain when its metadata transaction rolls back"
    );
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained archive batch");
    assert_eq!(remaining_batches, 1);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load rolled-back long-term integrity source boundary");
    assert!(
        integrity_source_start.is_none(),
        "a rolled-back metadata transaction must not publish a source boundary"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_archive_retries_pending_file_deletion_without_advancing_source_boundary_early()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-pending-delete").await;
    let pending_path = temp_dir.join("pending-delete-directory");
    fs::create_dir_all(&pending_path).expect("create pending-delete directory");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            cleanup_state,
            cleanup_source_safe_start_date,
            archive_expires_at,
            created_at
        )
        VALUES ('codex_quota_snapshots', '2025-01', ?1, 'pending-delete-sha', 1, ?2, 'delete_pending', '2025-01-04', '2000-01-01 00:00:00', datetime('now'))
        "#,
    )
    .bind(pending_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert pending archive deletion record");

    let first_attempt = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("failed file deletion should remain retryable");
    assert_eq!(first_attempt, 0);
    let pending_state: (String, String) =
        sqlx::query_as("SELECT status, cleanup_state FROM archive_batches WHERE file_path = ?1")
            .bind(pending_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load retained pending deletion record");
    assert_eq!(pending_state.0, ARCHIVE_STATUS_COMPLETED);
    assert_eq!(pending_state.1, ARCHIVE_CLEANUP_STATE_DELETE_PENDING);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after failed pending cleanup");
    assert!(
        integrity_source_start.is_none(),
        "a failed pending file cleanup must not publish its staged source boundary"
    );

    fs::remove_dir(&pending_path).expect("remove directory that blocks file deletion");
    fs::write(&pending_path, b"retryable archive file").expect("restore removable archive file");
    let retryable_sha256 = sha256_hex_file(&pending_path).expect("hash retryable archive file");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(retryable_sha256)
        .bind(pending_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("update retryable archive identity");
    let second_attempt = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("pending archive deletion should retry successfully");
    assert_eq!(second_attempt, 1);
    assert!(!pending_path.exists());
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retired pending deletion record");
    assert_eq!(remaining_batches, 0);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after finalized pending cleanup");
    assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_attempt_archive_preserves_the_long_term_source_boundary_and_retains_unverifiable_mapping()
 {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("attempt-archive-long-term-boundary").await;
    let occurred_at = "2025-01-01 23:00:00";
    insert_retention_invocation(
        &pool,
        "attempt-archive-long-term-boundary",
        occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{}"),
        "{}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET t_total_ms = ?1 WHERE invoke_id = ?2")
        .bind(48.0_f64 * 60.0 * 60.0 * 1000.0)
        .bind("attempt-archive-long-term-boundary")
        .execute(&pool)
        .await
        .expect("add multi-day timing to matching live invocation");

    let archive_path = temp_dir.join("expired-attempt-archive.sqlite.gz");
    let archive_db_path = temp_dir.join("expired-attempt-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create expired attempt archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open expired attempt archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create expired attempt archive schema");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            upstream_account_id,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            created_at
        )
        VALUES (1, ?1, ?2, '/v1/responses', 'pool', 7, 0, 0, 0, 'succeeded', ?2)
        "#,
    )
    .bind("attempt-archive-long-term-boundary")
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert attempt account mapping");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress expired attempt archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash expired attempt archive");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (1, 'pool_upstream_request_attempts', '2025-01', ?1, ?4, 1, ?2, ?3, ?3, '2025-01-02 00:00:00', datetime('now'), ?3)
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("insert expired attempt archive manifest");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for attempt cleanup");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
        )
        .bind(target)
        .bind(archive_path.to_string_lossy().to_string())
        .bind(&archive_sha256)
        .execute(&pool)
        .await
        .expect("mark attempt archive replay complete");
    }

    let verified_safe_start =
        crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            "pool_upstream_request_attempts",
            &archive_path.to_string_lossy(),
            Some(occurred_at),
        )
        .await
        .expect("resolve the live invocation for the archived attempt mapping");
    assert_eq!(
        verified_safe_start,
        Some(NaiveDate::from_ymd_opt(2025, 1, 4).expect("fixed safe date"))
    );

    let deleted = cleanup_expired_archive_batches(&pool, &_config, false)
        .await
        .expect("cleanup expired attempt archive");
    assert_eq!(deleted, 1);
    assert!(!archive_path.exists());
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load long-term integrity source boundary after attempt cleanup");
    assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));

    let unverifiable_archive_path = temp_dir.join("unverifiable-attempt-archive.sqlite.gz");
    let unverifiable_archive_db_path = temp_dir.join("unverifiable-attempt-archive.sqlite");
    fs::File::create(&unverifiable_archive_db_path)
        .expect("create unverifiable attempt archive sqlite file");
    let archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&unverifiable_archive_db_path))
            .await
            .expect("open unverifiable attempt archive sqlite");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create unverifiable attempt archive schema");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            upstream_account_id,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            created_at
        )
        VALUES (1, 'missing-invocation-source', ?1, '/v1/responses', 'pool', 7, 0, 0, 0, 'succeeded', ?1)
        "#,
    )
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert unverifiable attempt account mapping");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&unverifiable_archive_db_path, &unverifiable_archive_path)
        .expect("compress unverifiable attempt archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (2, 'pool_upstream_request_attempts', '2025-01', ?1, 'unverifiable-attempt-sha', 1, ?2, ?3, ?3, '2025-01-02 00:00:00', datetime('now'), ?3)
        "#,
    )
    .bind(unverifiable_archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .execute(&pool)
    .await
    .expect("insert unverifiable attempt archive manifest");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, 'unverifiable-attempt-sha')",
        )
        .bind(target)
        .bind(unverifiable_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark unverifiable attempt archive replay complete");
    }

    let deleted = cleanup_expired_archive_batches(&pool, &_config, false)
        .await
        .expect("attempt cleanup should retain unverifiable mapping");
    assert_eq!(deleted, 0);
    assert!(unverifiable_archive_path.exists());
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained unverifiable archive batch");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_attempt_archive_keeps_a_missing_manifest_as_long_term_source_evidence() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("cleanup-missing-attempt-manifest").await;
    let missing_archive_path = temp_dir.join("missing-attempt-source.sqlite.gz");
    let coverage_end_at = shanghai_local_days_ago(400, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (1, 'pool_upstream_request_attempts', ?1, ?2, 'missing-attempt-sha', 1, ?3, ?4, ?4, '2000-01-01 00:00:00', datetime('now'), datetime('now'))
        "#,
    )
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing expired attempt archive manifest");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term statistics ready for attempt cleanup");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, 'missing-attempt-sha')",
        )
        .bind(target)
        .bind(missing_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark missing attempt archive replay complete");
    }

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing attempt source manifest during expiry cleanup");
    assert_eq!(deleted, 0);
    let remaining_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count retained missing attempt manifest");
    assert_eq!(remaining_batches, 1);
    let remaining_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(missing_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count retained missing attempt markers");
    assert_eq!(remaining_marker_count, 3);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn backfill_invocation_archive_expiries_uses_coverage_end_at() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-expiry-backfill").await;
    let coverage_end_at = shanghai_local_days_ago(45, 18, 30, 0);
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), ?10)
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(
        temp_dir
            .join("expiry.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert archive batch for expiry backfill");

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill archive expiry");
    assert_eq!(updated, 1);

    let expected = shanghai_archive_expiry_from_reference_timestamp(
        &coverage_end_at,
        config.invocation_archive_ttl_days,
    )
    .expect("compute expected archive expiry");
    let actual: Option<String> =
        sqlx::query_scalar("SELECT archive_expires_at FROM archive_batches WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load archive expiry");
    assert_eq!(actual.as_deref(), Some(expected.as_str()));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_expiry_backfill_respects_the_retention_candidate_budget() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-expiry-backfill-budget").await;
    config.retention_batch_rows = 2;
    let coverage_end_at = shanghai_local_days_ago(45, 18, 30, 0);
    let created_at = format_utc_iso(Utc::now());

    for id in 1_i64..=3 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', ?2, ?3, 'expiry-budget', 1, ?4,
                    ?5, ?5, datetime('now'), ?6)
            "#,
        )
        .bind(id)
        .bind(&coverage_end_at[..7])
        .bind(
            temp_dir
                .join(format!("expiry-budget-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&coverage_end_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert expiry backfill candidate");
    }

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill bounded archive expiries");
    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches WHERE archive_expires_at IS NULL")
            .fetch_one(&pool)
            .await
            .expect("count remaining archive expiry candidates");
    assert_eq!(updated, 2);
    assert_eq!(pending, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pending_legacy_invocation_archives_do_not_expire_before_materialization() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("archive-expiry-pending-legacy").await;
    config.invocation_archive_ttl_days = 0;
    let coverage_end_at = shanghai_local_days_ago(120, 8, 0, 0);
    let archive_path = temp_dir.join("pending-legacy.sqlite.gz");
    write_gzip_test_file(&archive_path, b"pending-legacy");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert pending legacy archive batch");

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill should skip pending legacy archive");
    assert_eq!(updated, 0);

    let dry_run_deleted = cleanup_expired_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run should also skip pending legacy archive");
    assert_eq!(dry_run_deleted, 0);

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep pending legacy archive");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining pending legacy batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pending_legacy_forward_proxy_archives_do_not_expire_before_materialization() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("archive-expiry-pending-forward-proxy").await;
    let coverage_end_at = format_utc_iso(Utc::now() - ChronoDuration::days(120));
    let archive_path = temp_dir.join("pending-forward-proxy.sqlite.gz");
    write_gzip_test_file(&archive_path, b"pending-forward-proxy");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("forward_proxy_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert pending forward-proxy archive batch");

    let dry_run_deleted = cleanup_expired_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run should skip pending forward-proxy archive");
    assert_eq!(dry_run_deleted, 0);

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep pending forward-proxy archive");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining pending forward-proxy batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn prune_legacy_archive_batches_keeps_missing_invocation_manifest_while_backfill_pending() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("prune-missing-invocation-manifest-pending").await;
    let missing_archive_path = temp_dir.join("missing-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(771_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pending manifest account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest account");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing invocation archive metadata");
    sqlx::query(
        r#"
        INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(1_i64)
    .bind(771_i64)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest row");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("prune should keep missing invocation manifest metadata while pending");
    assert_eq!(prune_summary.deleted_archive_batches, 0);
    assert_eq!(prune_summary.skipped_unmaterialized_batches, 1);

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_manifest_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batch_upstream_activity")
            .fetch_one(&pool)
            .await
            .expect("count remaining manifest rows");
    assert_eq!(remaining_manifest_rows, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn prune_legacy_archive_batches_keeps_a_materialized_missing_manifest_after_source_loss() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("prune-missing-invocation-manifest-finalize").await;
    let missing_archive_path = temp_dir.join("missing-finalizable-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-finalizable-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing replayed invocation archive metadata");
    sqlx::query(
        "UPDATE long_term_stats_state SET status = 'error', last_error = 'terminal integrity proof reconciliation is incomplete' WHERE id = 1",
    )
        .execute(&pool)
        .await
        .expect("mark long-term statistics unavailable after source loss");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-finalizable-sha")
    .execute(&pool)
    .await
    .expect("insert long-term replay marker");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing source manifest");
    assert_eq!(prune_summary.deleted_archive_batches, 0);
    assert_eq!(prune_summary.skipped_unmaterialized_batches, 1);
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count retained replay markers");
    assert_eq!(remaining_markers, 1);

    let status: String =
        sqlx::query_scalar("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load durable source-loss status");
    assert_eq!(status, "error");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_prune_preserves_upstream_account_id_for_archive_manifest() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("prune-preserve-upstream-account").await;
    config.invocation_archive_ttl_days = 365;
    let occurred_at = shanghai_local_days_ago(31, 14, 0, 0);

    insert_retention_invocation(
        &pool,
        "prune-preserve-upstream-account",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\",\"upstreamAccountId\":771}"),
        "{\"ok\":true}",
        None,
        None,
        Some(111),
        Some(0.5),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("prune invocation details while preserving upstream account id");

    let pruned_payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("prune-preserve-upstream-account")
            .fetch_one(&pool)
            .await
            .expect("load pruned payload");
    let pruned_payload = serde_json::from_str::<Value>(
        pruned_payload
            .as_deref()
            .expect("payload should keep upstream account id"),
    )
    .expect("parse pruned payload");
    assert_eq!(pruned_payload, json!({ "upstreamAccountId": 771 }));

    let archived_occurred_at = shanghai_local_days_ago(91, 14, 0, 0);
    sqlx::query("UPDATE codex_invocations SET occurred_at = ?1 WHERE invoke_id = ?2")
        .bind(&archived_occurred_at)
        .bind("prune-preserve-upstream-account")
        .execute(&pool)
        .await
        .expect("age pruned invocation into archive window");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive pruned invocation");

    let manifest_row = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        WHERE account_id = ?1
        LIMIT 1
        "#,
    )
    .bind(771_i64)
    .fetch_one(&pool)
    .await
    .expect("load archive upstream activity manifest row");
    assert_eq!(manifest_row.0, 771);
    assert_eq!(manifest_row.1, archived_occurred_at);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_marks_batches_and_prune_removes_files() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-materialize-prune").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-materialize-prune",
        &[(
            1_i64,
            "historical-rollup-materialize-prune",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog before materialization");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert!(snapshot_before.pending_buckets >= 1);

    let dry_run_summary = materialize_historical_rollups(&pool, &config, true)
        .await
        .expect("dry-run materialize historical rollups");
    assert_eq!(dry_run_summary.scanned_archive_batches, 1);
    assert_eq!(dry_run_summary.materialized_archive_batches, 1);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize historical rollups");
    assert_eq!(summary.materialized_invocation_batches, 1);
    assert_eq!(summary.materialized_forward_proxy_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load materialized invocation hourly total count");
    assert_eq!(total_count, 1);

    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive batch materialized timestamp");
    assert!(materialized_at.is_some());

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog after materialization");
    assert_eq!(snapshot_after.legacy_archive_pending, 0);

    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive checksum for long-term replay marker");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for archive pruning");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark archive replay complete for long-term stats");

    let prune_dry_run = prune_legacy_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run prune legacy archive batches");
    assert_eq!(prune_dry_run.deleted_archive_batches, 1);
    assert!(archive_path.exists(), "dry-run should keep archive file");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("prune legacy archive batches");
    assert_eq!(prune_summary.deleted_archive_batches, 1);
    assert!(
        !archive_path.exists(),
        "pruned legacy archive file should be removed"
    );
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load long-term integrity source boundary after legacy prune");
    assert_eq!(
        integrity_source_start.as_deref(),
        Some(
            archived_hour_local
                .date()
                .succ_opt()
                .expect("archived date has successor")
                .to_string()
                .as_str()
        ),
        "legacy archive prune must persist the exact source boundary before deleting the source"
    );

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches after prune");
    assert_eq!(remaining_batches, 0);

    let rerun_summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("rerun materialize historical rollups after prune");
    assert_eq!(rerun_summary.materialized_invocation_batches, 0);
    let rerun_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load hourly totals after rerun");
    assert_eq!(rerun_total_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_skips_already_materialized_batches() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-skip-materialized").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_occurred_at).expect("invocation bucket epoch");

    seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-skip-materialized",
        &[(
            1_i64,
            "historical-rollup-skip-materialized",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .execute(&pool)
    .await
    .expect("mark archive batch already materialized");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&pool)
    .await
    .expect("seed already materialized invocation rollup");

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot for already materialized batch");
    assert_eq!(snapshot.legacy_archive_pending, 0);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should skip already materialized archive batch");
    assert_eq!(summary.materialized_invocation_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation rollup total count after skipped replay");
    assert_eq!(total_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_replays_breakdown_for_pruned_detail_archives() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-pruned-detail-pending").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-pruned-detail-pending",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-pruned-detail-pending",
            occurred_at: archived_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize historical rollups with pruned detail archive");
    assert_eq!(summary.materialized_invocation_batches, 0);
    assert_eq!(summary.blocked_archive_batches, 1);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after partial materialization");
    assert_eq!(total_count, 1);

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown replay marker for pruned archive");
    assert_eq!(breakdown_replay_markers, 1);
    let replayed_archive_sha256: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown replay identity");
    let manifest_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load replayed archive manifest identity");
    assert_eq!(
        replayed_archive_sha256.as_deref(),
        Some(manifest_sha256.as_str())
    );

    let missing_breakdown_archives =
        crate::stats::load_invocation_archives_missing_effective_rollup_target(
            &pool,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            None,
        )
        .await
        .expect("load archives missing usage breakdown replay marker");
    assert!(
        missing_breakdown_archives.is_empty(),
        "replayed usage breakdown target should not trigger read-side archive fallback"
    );

    let breakdown_row: (String, Option<i64>, String, String, i64, i64, i64, f64) = sqlx::query_as(
        r#"
            SELECT
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
                request_count,
                success_count,
                performance_total_tokens,
                cost_unknown
            FROM upstream_account_usage_breakdown_hourly
            "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown rollup row for pruned archive");
    assert_eq!(breakdown_row.0, "upstream:17");
    assert_eq!(breakdown_row.1, Some(17));
    assert_eq!(breakdown_row.2, "unknown");
    assert_eq!(breakdown_row.3, "");
    assert_eq!(breakdown_row.4, 1);
    assert_eq!(breakdown_row.5, 1);
    assert_eq!(breakdown_row.6, 12);
    assert_f64_close(breakdown_row.7, 0.12);

    let keyed_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4)
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE)
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(HOURLY_ROLLUP_TARGET_STICKY_KEYS)
    .fetch_one(&pool)
    .await
    .expect("load keyed replay markers");
    assert_eq!(keyed_replay_markers, 0);

    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive batch materialized timestamp for pruned detail archive");
    assert!(materialized_at.is_none());

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup snapshot after pruned detail materialization");
    assert_eq!(snapshot.legacy_archive_pending, 1);
    assert_eq!(
        snapshot.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_counts_partially_blocked_archive_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-partial-budget").await;
    let first_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 62) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived local hour");
    let second_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 31) as i64))
    .and_hms_opt(9, 0, 0)
    .expect("valid second archived local hour");
    let first_occurred_at = format_naive(
        first_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        second_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid second archived occurred_at"),
    );

    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-pruned-first",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-pruned-first",
            occurred_at: first_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let second_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-full-second",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-full-second",
            occurred_at: second_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 21,
            cost: 0.21,
            ttfb_ms: Some(210.0),
            payload: Some(r#"{"upstreamAccountId":18}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let first_summary =
        materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
            .await
            .expect("materialize one partially blocked archive within budget");
    assert_eq!(first_summary.materialized_invocation_batches, 0);

    let total_count_after_first: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after first bounded pass");
    assert_eq!(total_count_after_first, 1);

    let second_archive_materialized_at_after_first: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load second archive materialized timestamp after first bounded pass");
    assert!(
        second_archive_materialized_at_after_first.is_none(),
        "the second archive should not be materialized once the first partially blocked archive consumes the one-archive budget"
    );

    let second_summary =
        materialize_historical_rollups_bounded_from_skip(
            &pool,
            &config,
            false,
            Some(1),
            None,
            first_summary.scanned_archive_batches,
        )
        .await
        .expect("materialize second archive on the next bounded pass after skipping the previously scanned backlog");
    assert_eq!(second_summary.materialized_invocation_batches, 1);

    let total_count_after_second: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after second bounded pass");
    assert_eq!(total_count_after_second, 2);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_counts_fully_blocked_archive_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-fully-blocked-budget").await;
    let first_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 62) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived local hour");
    let second_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 31) as i64))
    .and_hms_opt(9, 0, 0)
    .expect("valid second archived local hour");
    let first_occurred_at = format_naive(
        first_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        second_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid second archived occurred_at"),
    );

    let first_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-fully-blocked-first",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-fully-blocked-first",
            occurred_at: first_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let second_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-fully-blocked-second",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-fully-blocked-second",
            occurred_at: second_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 21,
            cost: 0.21,
            ttfb_ms: Some(210.0),
            payload: Some(r#"{"upstreamAccountId":18}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target,
                dataset,
                file_path,
                archive_sha256,
                replayed_at
            )
            SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
            FROM archive_batches AS batches
            WHERE batches.dataset = ?2
              AND batches.status = 'completed'
              AND batches.file_path = ?3
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(first_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert replay marker for non-keyed invocation target");
    }

    let first_summary =
        materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
            .await
            .expect("fully blocked archive should still consume the one-archive budget");
    assert_eq!(first_summary.materialized_invocation_batches, 0);
    assert_eq!(first_summary.blocked_archive_batches, 1);

    let second_archive_materialized_at_after_first: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load second archive materialized timestamp after fully blocked bounded pass");
    assert!(
        second_archive_materialized_at_after_first.is_none(),
        "a fully blocked first archive should still consume the one-archive budget"
    );

    let second_summary = materialize_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        false,
        Some(1),
        None,
        first_summary.scanned_archive_batches,
    )
    .await
    .expect("second bounded pass should reach the next archive after skipping the blocked one");
    assert_eq!(second_summary.materialized_invocation_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn replay_invocation_archives_into_hourly_rollups_respects_caller_elapsed_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-shared-elapsed-budget").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );

    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-shared-elapsed-budget",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-shared-elapsed-budget",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let mut tx = pool.begin().await.expect("begin transaction");
    let summary = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now() - Duration::from_secs(1),
        Some(1),
        Some(Duration::from_millis(1)),
        0,
    )
    .await
    .expect("replay bounded invocation archives");
    tx.rollback().await.expect("rollback replay transaction");

    assert_eq!(summary.scanned_batches, 0);
    assert_eq!(summary.materialized_batches, 0);
    assert_eq!(summary.budget_consumed_batches, 0);

    let still_pending: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load archive materialized timestamp after bounded replay");
    assert!(
        still_pending.is_none(),
        "expired caller elapsed budget should leave the archive pending for a later pass"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_resumes_invocation_archive_from_saved_progress_cursor() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-resume-progress-cursor").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start_epoch = align_bucket_epoch(
        local_naive_to_utc(archive_hour_local, Shanghai).timestamp(),
        3_600,
        0,
    );
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid second archived occurred_at"),
    );

    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-resume-progress-cursor",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "historical-rollup-resume-progress-cursor-1",
                occurred_at: first_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "historical-rollup-resume-progress-cursor-2",
                occurred_at: second_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 21,
                cost: 0.21,
                ttfb_ms: Some(210.0),
                payload: Some(r#"{"upstreamAccountId":18}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(1_i64)
    .bind(120.0_f64)
    .bind(120.0_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]")
    .execute(&pool)
    .await
    .expect("seed previously materialized first row rollup");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (
            dataset,
            file_path,
            cursor_id,
            updated_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("seed historical rollup archive cursor");

    let summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
        .await
        .expect("resume archive replay from saved cursor");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let (total_count, total_tokens): (i64, i64) = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(total_tokens), 0) AS total_tokens
        FROM invocation_rollup_hourly
        WHERE source = ?1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load resumed invocation rollup totals");
    assert_eq!(
        total_count, 2,
        "resume should only apply the remaining archived row instead of replaying the whole archive"
    );
    assert_eq!(total_tokens, 33);

    let progress_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_optional(&pool)
    .await
    .expect("load archive progress after resumed materialization");
    assert_eq!(progress_cursor, None);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_backfills_usage_breakdown_prefix_behind_shared_cursor() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-breakdown-prefix-catchup").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start_epoch = align_bucket_epoch(
        local_naive_to_utc(archive_hour_local, Shanghai).timestamp(),
        3_600,
        0,
    );
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid second archived occurred_at"),
    );

    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-breakdown-prefix-catchup",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "historical-rollup-breakdown-prefix-catchup-1",
                occurred_at: first_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "historical-rollup-breakdown-prefix-catchup-2",
                occurred_at: second_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 21,
                cost: 0.21,
                ttfb_ms: Some(210.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;
    let archive_db_path = config
        .archive_dir
        .join("historical-rollup-breakdown-prefix-catchup.sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open mutable invocation archive sqlite");
    for (id, cost_input, cost_output) in [(1_i64, 0.05_f64, 0.07_f64), (2_i64, 0.08_f64, 0.13_f64)]
    {
        sqlx::query(
            r#"
            UPDATE codex_invocations
            SET model = 'gpt-5',
                cost_input = ?2,
                cost_cache_write = 0.0,
                cost_cache_read = 0.0,
                cost_output = ?3,
                cost_reasoning = 0.0
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(cost_input)
        .bind(cost_output)
        .execute(&archive_pool)
        .await
        .expect("write archive usage breakdown detail fields");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("refresh archive gzip with usage breakdown detail fields");

    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(1_i64)
    .bind(120.0_f64)
    .bind(120.0_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]")
    .execute(&pool)
    .await
    .expect("seed historical invocation rollup prefix");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_hourly (
            bucket_start_epoch,
            upstream_account_id,
            request_count,
            total_tokens,
            total_cost,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            success_count,
            failure_count,
            first_seen_at,
            last_seen_at
        )
        VALUES (?1, ?2, 1, 12, 0.12, 0, 12, 0, 1, 0, ?3, ?4)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(17_i64)
    .bind(&first_occurred_at)
    .bind(&first_occurred_at)
    .execute(&pool)
    .await
    .expect("seed historical account usage prefix");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (
            dataset,
            file_path,
            cursor_id,
            updated_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("seed shared replay cursor from legacy targets");

    let summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
        .await
        .expect("materialize historical rollups with breakdown prefix catch-up");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let (request_count, cost_input, cost_output, cost_unknown): (i64, f64, f64, f64) =
        sqlx::query_as::<_, (i64, f64, f64, f64)>(
            r#"
        SELECT
            COALESCE(SUM(request_count), 0) AS request_count,
            COALESCE(SUM(cost_input), 0) AS cost_input,
            COALESCE(SUM(cost_output), 0) AS cost_output,
            COALESCE(SUM(cost_unknown), 0) AS cost_unknown
        FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = ?1
          AND normalized_model = 'gpt-5'
        "#,
        )
        .bind(17_i64)
        .fetch_one(&pool)
        .await
        .expect("load historical usage breakdown totals after catch-up");
    assert_eq!(request_count, 2);
    assert_f64_close(cost_input, 0.13_f64);
    assert_f64_close(cost_output, 0.20_f64);
    assert_f64_close(cost_unknown, 0.0_f64);

    let breakdown_progress_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_optional(&pool)
    .await
    .expect("load breakdown-specific archive progress after materialization");
    assert_eq!(breakdown_progress_cursor, None);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_skips_live_replay_when_elapsed_budget_is_zero() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-live-budget-zero").await;
    let recent_invocation = shanghai_local_days_ago(0, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "historical-rollup-bounded-live-budget-zero",
        &recent_invocation,
        SOURCE_PROXY,
        "success",
        Some(r#"{"promptCacheKey":"live-budget-zero"}"#),
        "{\"ok\":true}",
        None,
        None,
        Some(8),
        Some(0.08),
    )
    .await;

    let summary =
        materialize_historical_rollups_bounded(&pool, &config, false, None, Some(Duration::ZERO))
            .await
            .expect("bounded materialization with zero elapsed budget");
    assert_eq!(summary.materialized_archive_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after zero-budget bounded pass");
    assert_eq!(
        total_count, 0,
        "zero elapsed budget should leave live rollup catch-up for a later pass"
    );

    let live_cursor: Option<i64> =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_optional(&pool)
            .await
            .expect("load invocation live cursor after zero-budget bounded pass");
    assert_eq!(
        live_cursor, None,
        "zero elapsed budget should not advance the shared live replay cursor"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_marks_replayed_batches_as_materialized_after_usage_breakdown_replay()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-mark-replayed").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let old_attempt = parse_to_utc_datetime(&utc_naive_from_shanghai_local_days_ago(
        (config.forward_proxy_attempts_retention_days + 2) as i64,
        7,
        0,
        0,
    ))
    .expect("parse old forward proxy attempt timestamp");

    insert_retention_invocation(
        &pool,
        "historical-rollup-mark-replayed",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"replayed\",\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\",\"stickyKey\":\"sticky-replayed\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    seed_forward_proxy_attempt_at(&pool, "proxy-replayed", old_attempt, true).await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before materialize");
    assert_eq!(retention.invocation_rows_archived, 1);
    assert_eq!(retention.forward_proxy_attempt_rows_archived, 1);

    sqlx::query("UPDATE archive_batches SET historical_rollups_materialized_at = NULL")
        .execute(&pool)
        .await
        .expect("clear materialized markers to mimic pre-upgrade replay state");

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2")
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("clear atomic publication proofs to mimic pre-upgrade replay state");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target,
                dataset,
                file_path,
                archive_sha256,
                replayed_at
            )
            SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
            FROM archive_batches AS batches
            WHERE batches.dataset = ?2
              AND batches.status = 'completed'
              AND batches.file_path = ?3
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("insert invocation replay marker");
    }

    let forward_proxy_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'forward_proxy_attempts' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load forward-proxy archive path");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
        FROM archive_batches AS batches
        WHERE batches.dataset = ?2
          AND batches.status = 'completed'
          AND batches.file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&forward_proxy_archive_path)
    .execute(&pool)
    .await
    .expect("insert forward-proxy replay marker");

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot before marking replayed batches");
    assert_eq!(snapshot_before.legacy_archive_pending, 2);

    let materialize = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should only mark replayed batches");
    assert_eq!(materialize.materialized_invocation_batches, 1);
    assert_eq!(materialize.materialized_forward_proxy_batches, 0);

    let materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count replayed batches marked materialized");
    assert_eq!(materialized_batches, 2);

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot after marking replayed batches");
    assert_eq!(snapshot_after.legacy_archive_pending, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_replays_usage_breakdown_when_account_targets_are_pending() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-account-target-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "historical-rollup-account-target-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before materialize");
    assert_eq!(retention.invocation_rows_archived, 1);

    sqlx::query("UPDATE archive_batches SET historical_rollups_materialized_at = NULL")
        .execute(&pool)
        .await
        .expect("clear materialized markers to mimic pre-upgrade replay state");

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2")
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("clear automatic publication proofs to mimic pre-upgrade replay state");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target,
                dataset,
                file_path,
                archive_sha256,
                replayed_at
            )
            SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
            FROM archive_batches AS batches
            WHERE batches.dataset = ?2
              AND batches.status = 'completed'
              AND batches.file_path = ?3
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("insert invocation replay marker");
    }

    let materialize = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should replay pending usage breakdown target");
    assert_eq!(materialize.materialized_invocation_batches, 1);

    let account_target_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4, ?5)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_one(&pool)
    .await
    .expect("count account replay markers");
    assert_eq!(account_target_markers, 4);

    let breakdown_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly")
            .fetch_one(&pool)
            .await
            .expect("count usage breakdown rollup rows after replay");
    assert!(breakdown_row_count > 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_priority_materialization_drains_backlog_without_clearing_blocked_batches()
{
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("usage-breakdown-priority-materialize").await;
    let archived_occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let payload = json!({
        "upstreamAccountId": 17,
        "upstreamAccountName": "Replay",
        "promptCacheKey": "pck-priority-materialize",
        "stickyKey": "sticky-priority-materialize",
        "responseModel": "gpt-5.4",
        "reasoningEffort": "high",
    })
    .to_string();
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "usage-breakdown-priority-materialize",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "usage-breakdown-priority-materialize",
            occurred_at: &archived_occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42_i64,
            cost: 0.42_f64,
            ttfb_ms: Some(120.0),
            payload: Some(payload.as_str()),
            detail_level: "summary",
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog before priority materialization");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert_eq!(snapshot_before.pending_usage_breakdown_batches, 1);

    let summary = materialize_usage_breakdown_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        Some(2),
        Some(Duration::from_secs(6)),
        0,
    )
    .await
    .expect("priority materialization should drain usage breakdown backlog");
    assert_eq!(summary.scanned_archive_batches, 1);
    assert_eq!(summary.materialized_invocation_batches, 0);
    assert_eq!(summary.blocked_archive_batches, 1);

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after priority materialization");
    assert_eq!(snapshot_after.pending_usage_breakdown_batches, 0);
    assert_eq!(snapshot_after.legacy_archive_pending, 1);

    let usage_breakdown_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND target = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count usage breakdown replay markers after priority materialization");
    assert_eq!(usage_breakdown_marker_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_reopens_materialized_batches_missing_usage_breakdown_backfill() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-repairs-account-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "bootstrap-repairs-account-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before bootstrap repair");
    assert_eq!(retention.invocation_rows_archived, 1);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4, ?5)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .execute(&pool)
    .await
    .expect("drop account replay markers to mimic old materialized state");

    let snapshot_before_bootstrap = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup snapshot before bootstrap repair");
    assert_eq!(snapshot_before_bootstrap.legacy_archive_pending, 0);
    assert_eq!(snapshot_before_bootstrap.pending_usage_breakdown_batches, 1);
    assert_eq!(
        snapshot_before_bootstrap.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should reopen missing usage breakdown backfill");

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count remaining breakdown replay markers");
    assert_eq!(breakdown_replay_markers, 0);

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(&invocation_archive_path)
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp after bootstrap");
    assert!(materialized_at.is_none());

    let backlog_snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after bootstrap repair");
    assert_eq!(backlog_snapshot.legacy_archive_pending, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_repairs_pre_upgrade_breakdown_rows_before_syncing_new_live_rows()
{
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-live-breakdown-repair-before-sync").await;
    let first_local = shanghai_local_days_ago(0, 9, 10, 0);
    let second_local = shanghai_local_days_ago(0, 9, 20, 0);

    for (id, invoke_id, occurred_at, total_tokens, cost, upstream_account_id) in [
        (
            41_i64,
            "bootstrap-live-breakdown-old-cursor",
            first_local.as_str(),
            30_i64,
            0.01_f64,
            17_i64,
        ),
        (
            42_i64,
            "bootstrap-live-breakdown-new-row",
            second_local.as_str(),
            40_i64,
            0.02_f64,
            17_i64,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                total_tokens,
                cost,
                payload,
                raw_response,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(12_i64)
        .bind(6_i64)
        .bind(total_tokens)
        .bind(cost)
        .bind(
            json!({
                "upstreamAccountId": upstream_account_id,
                "responseModel": "gpt-5",
            })
            .to_string(),
        )
        .bind("{}")
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation for breakdown bootstrap repair");
    }

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
        VALUES (?1, ?2)
        ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(41_i64)
    .execute(&pool)
    .await
    .expect("seed pre-upgrade shared invocation cursor");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should repair old breakdown rows before syncing new live rows");

    let breakdown = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT request_count, success_count, performance_total_tokens
        FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = ?1
          AND normalized_model = 'gpt-5'
        "#,
    )
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load repaired breakdown rollup row");
    assert_eq!(breakdown.0, 2);
    assert_eq!(breakdown.1, 2);
    assert_eq!(breakdown.2, 70);

    let shared_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_one(&pool)
            .await
            .expect("load shared invocation cursor after bootstrap");
    assert_eq!(shared_cursor, 42);

    let repair_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET)
            .fetch_one(&pool)
            .await
            .expect("load breakdown repair cursor after bootstrap");
    assert_eq!(repair_cursor, 41);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_repairs_legacy_account_replay_markers_when_breakdown_is_healthy()
{
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-repairs-legacy-account-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "bootstrap-repairs-legacy-account-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before bootstrap repair");
    assert_eq!(retention.invocation_rows_archived, 1);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    let bucket_epoch: i64 = sqlx::query_scalar(
        "SELECT bucket_start_epoch FROM upstream_account_usage_hourly ORDER BY bucket_start_epoch DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load usage rollup bucket epoch");

    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_breakdown_hourly (
            bucket_start_epoch,
            source,
            upstream_account_key,
            upstream_account_id,
            normalized_model,
            normalized_reasoning_effort,
            request_count,
            success_count,
            failure_count,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            cost_unknown,
            has_cost,
            performance_total_tokens,
            performance_stream_output_tokens,
            performance_stream_duration_ms,
            performance_response_sample_count,
            performance_response_sum_ms,
            performance_first_byte_sample_count,
            performance_first_byte_sum_ms,
            performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms,
            updated_at
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, '', 1, 1, 0, 0, 0, 42,
            0.42, 0.0, 0.0, 0.0, 0.0, 0.0, 1, 42, 42, 120.0, 1, 120.0, 1, 60.0, 1, 120.0,
            datetime('now')
        )
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .bind("upstream:17")
    .bind(17_i64)
    .bind("gpt-5")
    .execute(&pool)
    .await
    .expect("seed healthy breakdown rollup row");

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .execute(&pool)
    .await
    .expect("drop legacy account replay markers");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should repair legacy account replay markers");

    let account_target_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4, ?5)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_one(&pool)
    .await
    .expect("count repaired legacy account replay markers");
    assert_eq!(account_target_markers, 4);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_reopens_a_replaced_archive_with_a_stale_replay_sha() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-stale-replay-sha").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("breakdown-repair-stale-replay-sha.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let archive_file = PathBuf::from(&archive_path);
    fs::create_dir_all(
        archive_file
            .parent()
            .expect("archive fixture has a parent directory"),
    )
    .expect("create archive fixture directory");
    fs::write(&archive_file, b"initial archive bytes").expect("write initial archive fixture");
    let initial_sha256 = sha256_hex_file(&archive_file).expect("hash initial archive fixture");

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
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (
            ?1,
            '2026-01',
            ?2,
            ?3,
            1,
            ?4,
            '2026-01-15 08:00:00',
            '2026-01-15 08:30:00',
            datetime('now'),
            datetime('now')
        )
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&initial_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("seed materialized archive manifest at SHA A");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&initial_sha256)
    .execute(&pool)
    .await
    .expect("seed matching usage breakdown replay marker");

    let untouched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("matching marker must preserve materialized archive state");
    assert_eq!(untouched, 0);

    fs::write(&archive_file, b"replacement archive bytes").expect("replace archive fixture bytes");
    let replacement_sha256 =
        sha256_hex_file(&archive_file).expect("hash replacement archive fixture");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1
        WHERE dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(&replacement_sha256)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("replace completed archive manifest at the same path with SHA B");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("stale marker must reopen the replacement archive");
    assert_eq!(touched, 1);

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load reopened replacement archive state");
    assert!(materialized_at.is_none());

    let replay_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("count stale usage breakdown replay markers");
    assert_eq!(replay_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_rebuilds_transitive_boundary_overlap_closure() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-transitive-boundary-closure").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived hour");
    let transitive_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid transitive overlap timestamp"),
    );
    let boundary_start_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(30)))
            .expect("valid boundary archive start timestamp"),
    );
    let boundary_end_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .expect("valid boundary archive end timestamp"),
    );
    let stale_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid stale archive timestamp"),
    );

    let stale_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-stale-root",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "breakdown-closure-stale-root",
            occurred_at: stale_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-boundary-peer",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "breakdown-closure-boundary-peer-start",
                occurred_at: boundary_start_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "breakdown-closure-boundary-peer-end",
                occurred_at: boundary_end_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-transitive-peer",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "breakdown-closure-transitive-peer",
            occurred_at: transitive_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize the overlap closure archives");
    assert_eq!(initial.materialized_invocation_batches, 3);
    let before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly",
    )
    .fetch_one(&pool)
    .await
    .expect("capture initial usage breakdown total");
    assert_eq!(before, 4);

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(stale_archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("remove only the stale root usage breakdown marker");

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("reopen the full transitive overlap closure"),
        3
    );
    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("replay the full transitive overlap closure");
    assert_eq!(rebuilt.materialized_invocation_batches, 3);
    let after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly",
    )
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown total after closure replay");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_repair_quarantines_unverified_legacy_archive_without_replaying_rollups() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("startup-repair-quarantines-unverified-legacy").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "startup-repair-quarantines-unverified-legacy",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old invocation before legacy marker quarantine");
    assert_eq!(retention.invocation_rows_archived, 1);

    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archived invocation path");
    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture materialized rollup totals before legacy marker quarantine");
    assert_eq!(before, (1, 1, 0));

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("restore legacy null replay markers");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = '   ' WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("remove the completed archive manifest proof");

    for _ in 0..2 {
        bootstrap_hourly_rollups(&pool)
            .await
            .expect("startup repair must quarantine the unverified legacy archive");
        let summary = materialize_historical_rollups(&pool, &config, false)
            .await
            .expect("materializer must leave an unverified legacy archive quarantined");
        assert_eq!(summary.materialized_invocation_batches, 0);
    }

    let after_quarantine = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rollup totals after legacy marker quarantine");
    assert_eq!(after_quarantine, before);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load archive materialization state after quarantine");
    assert!(materialized_at.is_some());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_replay_markers_do_not_upgrade_from_sha_a_to_sha_b() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("legacy-replay-marker-upgrade").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "legacy-replay-marker-a-to-b",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "legacy-replay-marker-a-to-b",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"legacy-replay-marker-a-to-b"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize archive before legacy marker upgrade");
    assert_eq!(initial.materialized_invocation_batches, 1);

    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture materialized global and account totals");
    assert_eq!(before, (1, 1, 1));

    sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET archive_sha256 = NULL
        WHERE dataset = ?1 AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed legacy null replay markers");
    sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET archive_sha256 = ''
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed legacy blank replay marker");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-archive-sha-b' WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("publish replacement manifest SHA B");

    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        let replay_sha256: Option<String> = sqlx::query_scalar(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_file_path)
        .fetch_one(&pool)
        .await
        .expect("load quarantined replay marker");
        let expected = if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
            Some("")
        } else {
            None
        };
        assert_eq!(replay_sha256.as_deref(), expected);
    }

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("legacy marker quarantine must not reopen the archive"),
        0
    );
    let mut tx = pool.begin().await.expect("begin legacy proof check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file_path,
        )
        .await
        .expect("legacy NULL marker must fail closed"),
    );
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file_path,
        )
        .await
        .expect("legacy blank marker must fail closed"),
    );
    tx.commit().await.expect("commit legacy proof check");
    let after = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load totals after legacy marker upgrade");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn stale_replay_sha_rebuilds_all_rollup_targets_without_double_counting() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("stale-replay-sha-full-rebuild").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "stale-replay-sha-full-rebuild",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "stale-replay-sha-full-rebuild",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"stale-replay-sha-full-rebuild"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture original global and account totals");
    assert_eq!(before, (1, 1, 1, 1, 1, 1, 1, 1, 1, 1));

    let replacement_db_path = temp_dir.join("stale-replay-sha-full-rebuild.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_db_path)
        .expect("inflate replacement invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &archive_path)
        .expect("compress replacement invocation archive");
    let replacement_sha =
        sha256_hex_file(&archive_path).expect("hash replacement invocation archive");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE dataset = ?2 AND file_path = ?3")
        .bind(&replacement_sha)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("publish replacement archive manifest SHA");
    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = 17
        "#,
    )
    .execute(&pool)
    .await
    .expect("clear only the stale usage breakdown rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("clear only the stale usage breakdown marker");

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("rebuild stale archive contributions before replay"),
        1
    );
    let stale_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count cleared stale replay markers");
    assert_eq!(stale_marker_count, 0);

    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize the replacement archive after clearing old contributions");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rebuilt global and account totals");
    assert_eq!(after, before);

    let replacement_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2 AND archive_sha256 = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .fetch_one(&pool)
    .await
    .expect("count replacement replay markers");
    assert_eq!(
        replacement_marker_count,
        INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_summary_proof_recovery_reopens_materialized_archive_and_replays_exactly() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("summary-proof-recovery").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "summary-proof-recovery",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "summary-proof-recovery",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"summary-proof-recovery"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture original Summary rollup totals");
    assert_eq!(before, (1, 1, 1));

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("remove only the global Summary proof");

    let recovered = materialize_historical_rollups_startup_window(&pool, 0, Duration::from_secs(6))
        .await
        .expect("startup recovery must reopen and replay the verified archive");
    assert_eq!(recovered.summary.materialized_invocation_batches, 1);
    assert_eq!(recovered.changed_path_count, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load Summary totals after startup recovery replay");
    assert_eq!(after, before);

    let summary_proof_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND target IN (?3, ?4, ?5)
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count complete Summary proofs after automatic replay");
    assert_eq!(
        summary_proof_count,
        SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS.len() as i64
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_recovery_classifies_sparse_legacy_detail_mirror_by_archive_identity() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("sparse-legacy-detail-mirror").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_rows = [
        SeedInvocationArchiveBatchRow {
            id: 41,
            invoke_id: "sparse-legacy-detail-mirror-41",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some("{}"),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        },
        SeedInvocationArchiveBatchRow {
            id: 43,
            invoke_id: "sparse-legacy-detail-mirror-43",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some("{}"),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        },
    ];
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "sparse-legacy-detail-mirror",
        &archive_rows,
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    for row in archive_rows {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost,
                t_upstream_ttfb_ms, payload, detail_level, failure_class, is_actionable,
                raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '{}', ?3)
            "#,
        )
        .bind(row.id)
        .bind(row.invoke_id)
        .bind(&occurred_at)
        .bind(row.source)
        .bind(row.status)
        .bind(row.total_tokens)
        .bind(row.cost)
        .bind(row.ttfb_ms)
        .bind(row.payload)
        .bind(row.detail_level)
        .bind(row.failure_class)
        .bind(row.is_actionable)
        .execute(&pool)
        .await
        .expect("retain canonical live mirror record");
    }
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET layout = ?1,
            part_key = ?2,
            coverage_start_at = ?3,
            coverage_end_at = ?4
        WHERE dataset = ?5 AND file_path = ?6
        "#,
    )
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind("part-0000000000000029-000000000000002b-0123456789abcdef")
    .bind(&occurred_at)
    .bind(&occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("record sparse legacy segment metadata");

    let mirror_recovery =
        reconcile_legacy_detail_mirrors_startup_window(&pool, 0, Duration::from_secs(6))
            .await
            .expect("classify sparse legacy detail mirror before rollup replay");
    let recovered = materialize_historical_rollups_startup_window(&pool, 0, Duration::from_secs(6))
        .await
        .expect("recover sparse legacy detail mirror without replaying it");
    let source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load reconciled source role");

    assert_eq!(mirror_recovery.changed_path_count, 1);
    assert_eq!(source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR);
    assert_eq!(recovered.summary.materialized_invocation_batches, 0);

    let recovered_archive_path = temp_dir.join("sparse-legacy-detail-mirror-recovered.sqlite.gz");
    fs::copy(&archive_path, &recovered_archive_path)
        .expect("copy an independently provable legacy detail mirror");
    let recovered_archive_file_path = recovered_archive_path.to_string_lossy().to_string();
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at, layout, part_key, created_at
        )
        SELECT
            dataset, month_key, ?1, sha256, row_count, status,
            coverage_start_at, coverage_end_at, layout, part_key, datetime('now')
        FROM archive_batches
        WHERE dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(&recovered_archive_file_path)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed later independently provable legacy mirror manifest");
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 \
         WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("restore the first source role for startup recovery");
    fs::remove_file(&archive_path).expect("make the first legacy source unavailable");

    let high_watermark = summary_startup_legacy_detail_mirror_high_watermark(&pool)
        .await
        .expect("capture a stable startup mirror high-watermark")
        .expect("find legacy mirror candidates");
    let summary_startup_recovery = reconcile_legacy_detail_mirrors_for_summary_startup_window(
        &pool,
        0,
        high_watermark,
        Duration::from_secs(6),
    )
    .await
    .expect("a missing legacy source must not block later identity proofs");
    let source_kinds = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, summary_source_kind FROM archive_batches \
         WHERE dataset = ?1 AND file_path IN (?2, ?3) ORDER BY file_path ASC",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(&recovered_archive_file_path)
    .fetch_all(&pool)
    .await
    .expect("load summary startup recovery source roles");

    assert!(summary_startup_recovery.completed);
    assert_eq!(summary_startup_recovery.unavailable_path_count, 1);
    assert_eq!(summary_startup_recovery.changed_path_count, 1);
    assert_eq!(
        source_kinds,
        vec![
            (
                archive_file_path.clone(),
                SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN.to_string(),
            ),
            (
                recovered_archive_file_path.clone(),
                SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR.to_string(),
            ),
        ],
        "the unreadable source stays fail-closed while the independent mirror leaves Summary admission"
    );

    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("exclude the unavailable source from the generic mismatch probe");
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&recovered_archive_file_path)
    .execute(&pool)
    .await
    .expect("restore the independently proved source role for mismatch probing");
    let budget_exhausted = reconcile_legacy_detail_mirrors_startup_window(&pool, 0, Duration::ZERO)
        .await
        .expect("defer an unstarted mirror proof at its elapsed budget");
    assert!(budget_exhausted.hit_budget);
    assert_eq!(budget_exhausted.inspected_path_count, 0);
    sqlx::query("UPDATE codex_invocations SET invoke_id = ?1 WHERE id = ?2")
        .bind("sparse-legacy-detail-mirror-mismatch")
        .bind(43_i64)
        .execute(&pool)
        .await
        .expect("mismatch one retained live identity");
    let rejected = reconcile_legacy_detail_mirrors_startup_window(&pool, 0, Duration::from_secs(6))
        .await
        .expect("reject non-mirror archive identity");
    let rejected_source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&recovered_archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load rejected source role");

    assert_eq!(rejected.changed_path_count, 0);
    assert_eq!(rejected_source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);

    fs::remove_file(&recovered_archive_path).expect("make archived source unavailable");
    let unreadable =
        reconcile_legacy_detail_mirrors_startup_window(&pool, 0, Duration::from_secs(6))
            .await
            .expect_err("surface an unreadable archive to the startup task");
    let unreadable_source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&recovered_archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load unreadable source role");

    assert!(
        unreadable
            .to_string()
            .contains("failed to open legacy detail archive for sha256")
    );
    assert_eq!(unreadable_source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn partial_materialization_rebuilds_stale_sha_before_any_additive_replay() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("partial-materialization-stale-sha").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "partial-materialization-stale-sha",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "partial-materialization-stale-sha",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"partial-materialization-stale-sha"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture original rollup totals");
    assert_eq!(before, (1, 1, 1));

    let replacement_db_path = temp_dir.join("partial-materialization-stale-sha.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_db_path)
        .expect("inflate replacement invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &archive_path)
        .expect("compress replacement invocation archive");
    let replacement_sha =
        sha256_hex_file(&archive_path).expect("hash replacement invocation archive");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1,
            historical_rollups_materialized_at = NULL
        WHERE dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(&replacement_sha)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("publish replacement manifest while preserving stale target markers");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("leave a partially materialized replacement archive");

    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("rebuild stale partial archive before replay");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rollup totals after stale partial rebuild");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn stale_archive_rebuild_clears_full_coverage_of_reopened_overlaps() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("stale-replay-sha-overlap-coverage").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let overlap_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid overlapping archived occurred_at"),
    );
    let extended_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(20)))
            .expect("valid extended archived occurred_at"),
    );
    let first_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "stale-replay-sha-overlap-first",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "stale-replay-sha-overlap-first",
            occurred_at: first_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "stale-replay-sha-overlap-extended",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "stale-replay-sha-overlap-extended-overlap",
                occurred_at: overlap_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "stale-replay-sha-overlap-extended-tail",
                occurred_at: extended_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize overlapping archives");
    assert_eq!(initial.materialized_invocation_batches, 2);
    let before: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&pool)
            .await
            .expect("capture original overlapping rollup total");
    assert_eq!(before, 3);

    let replacement_db_path = temp_dir.join("stale-replay-sha-overlap-first.sqlite");
    inflate_gzip_sqlite_file(&first_archive_path, &replacement_db_path)
        .expect("inflate replacement overlapping invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement overlapping invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement overlapping invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &first_archive_path)
        .expect("compress replacement overlapping invocation archive");
    let replacement_sha = sha256_hex_file(&first_archive_path)
        .expect("hash replacement overlapping invocation archive");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1,
            historical_rollups_materialized_at = NULL
        WHERE dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(&replacement_sha)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("replace the short archive manifest");

    let first_replay = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("reopen stale archive and its overlapping peer");
    assert_eq!(first_replay.materialized_invocation_batches, 1);
    let second_replay = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("replay the reopened extended overlap archive");
    assert_eq!(second_replay.materialized_invocation_batches, 1);
    let after: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&pool)
            .await
            .expect("load overlapping rollup total after rebuild");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn forward_proxy_stale_replay_sha_rebuilds_same_path_without_double_counting() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("forward-proxy-stale-replay-sha").await;
    let first_occurred_at = (Utc::now() - ChronoDuration::days(120))
        .with_minute(10)
        .expect("set first minute")
        .with_second(0)
        .expect("set first second");
    let second_occurred_at = first_occurred_at + ChronoDuration::hours(1);
    let replacement_occurred_at = first_occurred_at + ChronoDuration::hours(2);
    let first_occurred_at = first_occurred_at.format("%Y-%m-%d %H:%M:%S").to_string();
    let second_occurred_at = second_occurred_at.format("%Y-%m-%d %H:%M:%S").to_string();
    let replacement_occurred_at = replacement_occurred_at
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let month_key = first_occurred_at[..7].to_string();
    let archive_path = temp_dir.join("forward-proxy-stale-replay.sqlite.gz");
    let archive_db_path = temp_dir.join("forward-proxy-stale-replay.sqlite");

    fs::File::create(&archive_db_path).expect("create forward proxy archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open forward proxy archive sqlite");
    let create_sql = FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create forward proxy archive schema");
    for (id, occurred_at, is_success, latency_ms) in [
        (1_i64, first_occurred_at.as_str(), 1_i64, Some(120.0_f64)),
        (2_i64, second_occurred_at.as_str(), 0_i64, None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_attempts (
                id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe
            )
            VALUES (?1, 'proxy-stale-replay', ?2, ?3, ?4, NULL, 0)
            "#,
        )
        .bind(id)
        .bind(occurred_at)
        .bind(is_success)
        .bind(latency_ms)
        .execute(&archive_pool)
        .await
        .expect("insert initial forward proxy archive row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress initial forward proxy archive");
    let initial_sha = sha256_hex_file(&archive_path).expect("hash initial forward proxy archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, 2, ?5, ?6, ?7, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&initial_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&first_occurred_at)
    .bind(&second_occurred_at)
    .execute(&pool)
    .await
    .expect("insert initial forward proxy archive manifest");

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize initial forward proxy archive");
    assert_eq!(initial.materialized_forward_proxy_batches, 1);
    let before: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(attempts), 0),
            COALESCE(SUM(success_count), 0),
            COALESCE(SUM(failure_count), 0)
        FROM forward_proxy_attempt_hourly
        WHERE proxy_key = 'proxy-stale-replay'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load initial forward proxy rollups");
    assert_eq!(before, (2, 1, 1));

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("remove forward proxy replay proof from an already materialized archive");
    let markerless = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("leave a markerless materialized forward proxy archive unchanged");
    assert_eq!(markerless.materialized_forward_proxy_batches, 0);
    let markerless_total: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(attempts), 0),
            COALESCE(SUM(success_count), 0),
            COALESCE(SUM(failure_count), 0)
        FROM forward_proxy_attempt_hourly
        WHERE proxy_key = 'proxy-stale-replay'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load markerless forward proxy rollups");
    assert_eq!(markerless_total, before);
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target, dataset, file_path, archive_sha256, replayed_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&initial_sha)
    .execute(&pool)
    .await
    .expect("restore original forward proxy replay proof");

    inflate_gzip_sqlite_file(&archive_path, &archive_db_path)
        .expect("inflate replacement forward proxy archive");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open replacement forward proxy archive sqlite");
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe
        )
        VALUES (3, 'proxy-stale-replay', ?1, 1, 80.0, NULL, 0)
        "#,
    )
    .bind(&replacement_occurred_at)
    .execute(&archive_pool)
    .await
    .expect("append replacement forward proxy archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress replacement forward proxy archive");
    let replacement_sha =
        sha256_hex_file(&archive_path).expect("hash replacement forward proxy archive");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1,
            row_count = 3,
            coverage_end_at = ?2
        WHERE dataset = ?3
          AND file_path = ?4
        "#,
    )
    .bind(&replacement_sha)
    .bind(&replacement_occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("publish replacement forward proxy manifest SHA");

    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("rebuild stale forward proxy archive");
    assert_eq!(rebuilt.materialized_forward_proxy_batches, 1);
    let after: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(attempts), 0),
            COALESCE(SUM(success_count), 0),
            COALESCE(SUM(failure_count), 0)
        FROM forward_proxy_attempt_hourly
        WHERE proxy_key = 'proxy-stale-replay'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rebuilt forward proxy rollups");
    assert_eq!(after, (3, 2, 1));

    let replay_sha: String = sqlx::query_scalar(
        r#"
        SELECT archive_sha256
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load replacement forward proxy replay marker");
    assert_eq!(replay_sha, replacement_sha);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_account_marker_repair_requires_a_nonblank_completed_manifest_sha() {
    // Existing installations can retain the pre-upgrade nullable SHA column because
    // CREATE TABLE IF NOT EXISTS never tightens the historical table definition.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open legacy nullable-manifest test database");
    sqlx::query(
        r#"
        CREATE TABLE archive_batches (
            id INTEGER PRIMARY KEY,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            file_path TEXT NOT NULL,
            sha256 TEXT,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            historical_rollups_materialized_at TEXT,
            summary_source_kind TEXT NOT NULL DEFAULT 'unknown',
            created_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create nullable legacy archive manifest table");
    sqlx::query(
        r#"
        CREATE TABLE hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            archive_sha256 TEXT,
            replayed_at TEXT NOT NULL,
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create archive replay marker table");

    let null_manifest_path = "legacy-null-manifest.sqlite.gz";
    let blank_manifest_path = "legacy-blank-manifest.sqlite.gz";
    for (month_key, file_path, sha256) in [
        ("2026-01", null_manifest_path, None),
        ("2026-02", blank_manifest_path, Some("   ")),
    ] {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                historical_rollups_materialized_at, created_at
            )
            VALUES (?1, ?2, ?3, ?4, 1, ?5, datetime('now'), datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(month_key)
        .bind(file_path)
        .bind(sha256)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .execute(&pool)
        .await
        .expect("seed unverified materialized invocation archive");
    }

    assert_eq!(
        repair_materialized_upstream_account_archive_markers(&pool)
            .await
            .expect("unverified manifests must remain quarantined"),
        0
    );
    for file_path in [&null_manifest_path, &blank_manifest_path] {
        let account_marker_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM hourly_rollup_archive_replay
            WHERE dataset = ?1
              AND file_path = ?2
              AND target IN (?3, ?4, ?5)
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
        .fetch_one(&pool)
        .await
        .expect("count quarantined account replay markers");
        assert_eq!(account_marker_count, 0);
    }

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target, dataset, file_path, archive_sha256, replayed_at
        )
        VALUES (?1, ?2, ?3, '', datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(blank_manifest_path)
    .execute(&pool)
    .await
    .expect("seed blank legacy replay marker");
    let mut tx = pool.begin().await.expect("begin blank marker read check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            blank_manifest_path,
        )
        .await
        .expect("blank manifest marker read must fail closed")
    );
    assert!(
        mark_hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            blank_manifest_path,
        )
        .await
        .is_err(),
        "a blank manifest SHA must not be written as replay proof"
    );
    tx.commit().await.expect("commit blank marker read check");

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = 'recovered-account-manifest-sha'
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(null_manifest_path)
    .execute(&pool)
    .await
    .expect("publish recovered invocation manifest SHA");
    assert_eq!(
        repair_materialized_upstream_account_archive_markers(&pool)
            .await
            .expect("valid manifest recovery must repair account markers"),
        1
    );
    let repaired_marker_sha: String = sqlx::query_scalar(
        r#"
        SELECT archive_sha256
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(null_manifest_path)
    .fetch_one(&pool)
    .await
    .expect("load repaired account replay marker");
    assert_eq!(repaired_marker_sha, "recovered-account-manifest-sha");
}

#[tokio::test]
async fn same_path_invocation_archive_append_preserves_coverage_for_stale_rebuild() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("same-path-invocation-append-coverage").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid second archived occurred_at"),
    );
    let appended_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid appended archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "same-path-invocation-append-coverage",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "same-path-invocation-append-first",
                occurred_at: first_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(
                    r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#,
                ),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "same-path-invocation-append-second",
                occurred_at: second_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(
                    r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#,
                ),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = ?1,
            coverage_end_at = ?2
        WHERE dataset = ?3
          AND file_path = ?4
        "#,
    )
    .bind(&first_occurred_at)
    .bind(&second_occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("record initial archive coverage");

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize initial same-path archive");
    assert_eq!(initial.materialized_invocation_batches, 1);

    let replacement_db_path = temp_dir.join("same-path-invocation-append-replacement.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_db_path)
        .expect("inflate append archive source");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open append archive sqlite");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost,
            t_upstream_ttfb_ms, payload, detail_level, error_message, failure_kind,
            failure_class, is_actionable, raw_response, created_at
        )
        VALUES (
            3, 'same-path-invocation-append-third', ?1, ?2, 'success', 12, 0.12,
            120.0, ?3, ?4, NULL, NULL, 'none', 0, '{}', ?1
        )
        "#,
    )
    .bind(&appended_occurred_at)
    .bind(SOURCE_PROXY)
    .bind(
        r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#,
    )
    .bind(DETAIL_LEVEL_FULL)
    .execute(&replacement_pool)
    .await
    .expect("append invocation archive row");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &archive_path)
        .expect("compress appended invocation archive");
    let replacement_sha = sha256_hex_file(&archive_path).expect("hash appended invocation archive");
    let replacement_manifest = ArchiveBatchOutcome {
        dataset: HOURLY_ROLLUP_DATASET_INVOCATIONS,
        month_key: first_occurred_at[..7].to_string(),
        day_key: None,
        part_key: None,
        file_path: archive_file_path.clone(),
        sha256: replacement_sha.clone(),
        row_count: 3,
        upstream_last_activity: Vec::new(),
        coverage_start_at: Some(appended_occurred_at.clone()),
        coverage_end_at: Some(appended_occurred_at.clone()),
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    };
    let mut tx = pool
        .begin()
        .await
        .expect("begin append manifest transaction");
    upsert_archive_batch_manifest(tx.as_mut(), &replacement_manifest)
        .await
        .expect("upsert appended same-path manifest");
    tx.commit()
        .await
        .expect("commit appended same-path manifest");

    let coverage: (String, String) = sqlx::query_as(
        r#"
        SELECT coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load appended archive coverage");
    assert_eq!(
        coverage,
        (first_occurred_at.clone(), appended_occurred_at.clone())
    );

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("reopen stale same-path archive over full preserved coverage"),
        1
    );
    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("rebuild appended same-path archive");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rebuilt same-path rollup totals");
    assert_eq!(after, (3, 3, 3, 3, 3, 3, 3, 3, 3, 3));

    let replacement_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND archive_sha256 = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .fetch_one(&pool)
    .await
    .expect("count appended archive replacement markers");
    assert_eq!(
        replacement_marker_count,
        INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_replay_quarantines_a_stale_archive_without_coverage_bounds() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-replay-stale-replay-sha").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-replay-stale-replay-sha",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "breakdown-replay-stale-replay-sha",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let original_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load original archive SHA");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
        FROM archive_batches AS batches
        WHERE batches.dataset = ?2
          AND batches.status = 'completed'
          AND batches.file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed replay marker for original archive SHA");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = 'replacement-archive-sha'
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("publish replacement archive manifest SHA");

    let pending = load_invocation_archive_files_missing_rollup_target(
        &pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    )
    .await
    .expect("list replacement archive as pending usage breakdown replay");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].file_path, archive_file_path);

    let mut tx = pool
        .begin()
        .await
        .expect("begin replacement archive replay");
    let summary = replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(1),
        None,
        0,
    )
    .await
    .expect("quarantine replacement archive with unknown coverage");
    tx.commit()
        .await
        .expect("commit replacement archive replay");
    assert_eq!(summary.materialized_batches, 0);
    assert_eq!(summary.blocked_batches, 1);

    let replay_sha256: Option<String> = sqlx::query_scalar(
        r#"
        SELECT archive_sha256
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load quarantined usage breakdown replay marker");
    assert_eq!(replay_sha256.as_deref(), Some(original_sha.as_str()));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_replay_marker_requires_a_matching_completed_manifest() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-replay-manifest-identity").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("breakdown-replay-manifest-identity.sqlite.gz")
        .to_string_lossy()
        .to_string();

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        VALUES (?1, ?2, ?3, NULL, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("seed replay marker without a manifest");

    crate::schema::ensure_schema(&pool)
        .await
        .expect("upgrade legacy marker without a manifest");
    let missing_manifest_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load missing-manifest legacy marker");
    assert!(
        missing_manifest_sha.is_none(),
        "a legacy marker without a completed manifest must remain unverified"
    );

    let mut tx = pool.begin().await.expect("begin missing-manifest check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path,
        )
        .await
        .expect("check marker without manifest"),
        "a marker without a completed manifest must fail closed"
    );
    tx.commit().await.expect("commit missing-manifest check");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            created_at
        )
        VALUES (?1, '2026-01', ?2, 'archive-sha-a', 1, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("seed matching completed archive manifest");

    crate::schema::ensure_schema(&pool)
        .await
        .expect("reapply schema without upgrading a legacy marker");
    let completed_manifest_legacy_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load legacy marker after completed manifest");
    assert!(
        completed_manifest_legacy_sha.is_none(),
        "a completed manifest alone must not upgrade a legacy replay marker"
    );

    let mut tx = pool
        .begin()
        .await
        .expect("begin unverified completed-manifest check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path,
        )
        .await
        .expect("check legacy marker with a completed manifest"),
        "an unverified legacy marker must fail closed even with a completed manifest"
    );
    tx.commit()
        .await
        .expect("commit unverified completed-manifest check");

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = 'archive-sha-a' WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("record a verified replay marker matching the completed manifest");

    let mut tx = pool.begin().await.expect("begin matching-manifest check");
    assert!(
        hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path,
        )
        .await
        .expect("check matching marker"),
        "a marker matching the completed manifest must remain accepted"
    );
    tx.commit().await.expect("commit matching-manifest check");

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET status = 'writing'
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("make manifest incomplete");

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("restore legacy marker before incomplete-manifest check");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("upgrade legacy marker with an incomplete manifest");
    let incomplete_manifest_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load incomplete-manifest legacy marker");
    assert!(
        incomplete_manifest_sha.is_none(),
        "a legacy marker with an incomplete manifest must remain unverified"
    );

    let mut tx = pool.begin().await.expect("begin incomplete-manifest check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path,
        )
        .await
        .expect("check marker against incomplete manifest"),
        "a marker without a completed manifest must fail closed"
    );
    tx.commit().await.expect("commit incomplete-manifest check");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_reopens_partially_populated_usage_breakdown_history() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-reopens-partial-breakdown-history").await;
    let first_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 10, 0);
    let second_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 10, 20, 0);

    for occurred_at in [&first_local, &second_local] {
        insert_retention_invocation(
            &pool,
            "bootstrap-reopens-partial-breakdown-history",
            occurred_at,
            SOURCE_PROXY,
            "success",
            Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
            "{\"ok\":true}",
            None,
            None,
            Some(42),
            Some(0.42),
        )
        .await;
    }

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before partial breakdown repair");
    assert_eq!(retention.invocation_rows_archived, 2);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    let first_bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&first_local).expect("valid first local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    let second_bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&second_local).expect("valid second local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;

    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
        "#,
    )
    .bind(second_bucket_epoch)
    .execute(&pool)
    .await
    .expect("drop second breakdown bucket to mimic partial backfill");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&invocation_archive_path)
    .execute(&pool)
    .await
    .expect("invalidate the usage breakdown proof after removing its rollup data");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO NOTHING
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(first_bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed first invocation materialized bucket");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO NOTHING
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(second_bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed second invocation materialized bucket");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should reopen partial usage breakdown history");

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(&invocation_archive_path)
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp after partial repair");
    assert!(materialized_at.is_none());

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count breakdown replay markers after partial repair");
    assert_eq!(breakdown_replay_markers, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_reopens_same_bucket_partial_usage_breakdown_groups() {
    let (pool, config, temp_dir) = retention_memory_test_pool_and_config(
        "bootstrap-reopens-same-bucket-partial-breakdown-groups",
    )
    .await;
    let first_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 10, 0);
    let second_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 20, 0);

    insert_retention_invocation(
        &pool,
        "bootstrap-reopens-same-bucket-group-a",
        &first_local,
        SOURCE_PROXY,
        "success",
        Some(
            "{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay A\",\"responseModel\":\"gpt-5\"}",
        ),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "bootstrap-reopens-same-bucket-group-b",
        &second_local,
        SOURCE_PROXY,
        "success",
        Some(
            "{\"upstreamAccountId\":18,\"upstreamAccountName\":\"Replay B\",\"responseModel\":\"gpt-4o\"}",
        ),
        "{\"ok\":true}",
        None,
        None,
        Some(64),
        Some(0.64),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before same-bucket repair");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before same-bucket partial breakdown repair");
    assert_eq!(retention.invocation_rows_archived, 2);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    let bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&first_local).expect("valid same-bucket local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;

    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
          AND upstream_account_id = ?2
        "#,
    )
    .bind(bucket_epoch)
    .bind(18_i64)
    .execute(&pool)
    .await
    .expect("drop one same-bucket breakdown group to mimic partial backfill");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&invocation_archive_path)
    .execute(&pool)
    .await
    .expect("invalidate the usage breakdown proof after removing one breakdown group");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO NOTHING
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed same-bucket invocation materialized marker");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should reopen same-bucket partial breakdown groups");

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(&invocation_archive_path)
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp after same-bucket repair");
    assert!(materialized_at.is_none());

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count breakdown replay markers after same-bucket repair");
    assert_eq!(breakdown_replay_markers, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_preserves_retained_live_rows_in_reopened_archive_bucket() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-preserves-live-bucket").await;
    let live_local = shanghai_local_days_ago(0, 9, 50, 0);
    let coverage_start = shanghai_local_days_ago(0, 9, 5, 0);
    let coverage_end = shanghai_local_days_ago(0, 9, 20, 0);

    insert_retention_invocation(
        &pool,
        "breakdown-repair-retained-live",
        &live_local,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Retained Live\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live breakdown rollup before repair");

    let bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&live_local).expect("valid live local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    let seeded_live_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(request_count), 0)
        FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
          AND upstream_account_id = ?2
        "#,
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load seeded live breakdown rows");
    assert_eq!(seeded_live_rows, 1);

    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("partial-retained-live.sqlite.gz");
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
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_start[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("partial-retained-live-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&pool)
    .await
    .expect("seed materialized archive batch overlapping retained live bucket");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair missing usage breakdown replay state");
    assert_eq!(touched, 1);

    let retained_live_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(request_count), 0)
        FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
          AND upstream_account_id = ?2
        "#,
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load retained live breakdown rows after repair");
    assert_eq!(retained_live_rows, 1);

    let retained_live_cost: f64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(cost_unknown), 0.0)
        FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
          AND upstream_account_id = ?2
        "#,
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load retained live breakdown cost after repair");
    assert_f64_close(retained_live_cost, 0.42);

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp after repair");
    assert!(materialized_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn historical_rollup_backfill_stays_critical_until_legacy_invocations_materialized() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("historical-rollup-backfill-critical").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-backfill-critical",
        &[(
            1_i64,
            "historical-rollup-backfill-critical",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0))
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed unrelated materialized bucket marker");

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog snapshot");
    assert_eq!(snapshot.legacy_archive_pending, 1);
    assert!(snapshot.last_materialized_hour.is_none());
    assert_eq!(
        snapshot.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn prune_legacy_archive_batches_keeps_detail_prune_backups_within_live_window() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-prune-detail-backup").await;
    let pruned_occurred_at = shanghai_local_days_ago(
        (config.invocation_success_full_days + 5)
            .try_into()
            .expect("detail prune age should fit in i64"),
        14,
        0,
        0,
    );
    insert_retention_invocation(
        &pool,
        "historical-rollup-prune-detail-backup",
        &pruned_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"detail-backup\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(11),
        Some(0.11),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention detail prune");

    let (archive_path, materialized_at, detail_level): (String, Option<String>, String) =
        sqlx::query_as(
            r#"
            SELECT b.file_path, b.historical_rollups_materialized_at, i.detail_level
            FROM archive_batches AS b
            JOIN codex_invocations AS i
              ON i.invoke_id = ?1
            WHERE b.dataset = 'codex_invocations'
            ORDER BY b.id DESC
            LIMIT 1
            "#,
        )
        .bind("historical-rollup-prune-detail-backup")
        .fetch_one(&pool)
        .await
        .expect("load detail prune archive metadata");
    assert_eq!(detail_level, DETAIL_LEVEL_STRUCTURED_ONLY);
    assert!(
        materialized_at.is_some(),
        "detail-prune archive still participates in rollup materialization"
    );
    assert!(
        Path::new(&archive_path).exists(),
        "detail backup archive should exist"
    );

    let prune_dry_run = prune_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run prune should retain detail backup archive");
    assert_eq!(prune_dry_run.expired_archive_batches_deleted, 0);
    assert_eq!(prune_dry_run.legacy_archive_batches_deleted, 0);
    assert!(
        Path::new(&archive_path).exists(),
        "dry-run should not remove archive"
    );

    let prune_summary = prune_archive_batches(&pool, &config, false)
        .await
        .expect("prune should keep detail backup archive");
    assert_eq!(prune_summary.expired_archive_batches_deleted, 0);
    assert_eq!(prune_summary.legacy_archive_batches_deleted, 0);
    assert!(
        Path::new(&archive_path).exists(),
        "detail backup archive must remain"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_skips_missing_archives_and_preserves_existing_rollups() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-materialize-atomic").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_occurred_at).expect("invocation bucket epoch");
    let missing_archive_path =
        archive_batch_file_path(&config, "codex_invocations", &archived_occurred_at[..7])
            .expect("resolve missing archive path");

    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(7_i64)
    .bind(6_i64)
    .bind(1_i64)
    .bind(77_i64)
    .bind(7.7_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&pool)
    .await
    .expect("seed existing invocation rollup");
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
    .bind(&archived_occurred_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&archived_occurred_at)
    .bind(&archived_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing archive manifest");

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot before skipping missing archive");
    assert_eq!(snapshot_before.legacy_archive_pending, 0);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialization should skip missing archive file");
    assert_eq!(summary.materialized_invocation_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load retained invocation rollup total");
    assert_eq!(
        total_count, 7,
        "skipped materialization must keep prior rollups"
    );

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("prune should retain stale metadata without long-term replay");
    assert_eq!(prune_summary.deleted_archive_batches, 0);

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches after pruning missing metadata");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn recompute_invocation_hourly_rollups_ignores_archive_manifests_for_live_buckets() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-recompute-live-only").await;
    let live_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_success_full_days + 5) as i64))
    .and_hms_opt(9, 20, 0)
    .expect("valid live historical local hour");
    let live_occurred_at = format_naive(live_local);
    insert_retention_invocation(
        &pool,
        "historical-rollup-recompute-live-only",
        &live_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"live-key\"}"),
        "{}",
        None,
        None,
        Some(5),
        Some(0.5),
    )
    .await;

    let live_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("historical-rollup-recompute-live-only")
            .fetch_one(&pool)
            .await
            .expect("load live invocation id");
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&live_occurred_at).expect("invocation bucket epoch");
    let missing_archive_path =
        archive_batch_file_path(&config, "codex_invocations", &live_occurred_at[..7])
            .expect("resolve missing archive path");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&live_occurred_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&live_occurred_at)
    .bind(&live_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing archive manifest for same live month");

    let mut tx = pool.begin().await.expect("begin recompute tx");
    sqlx::query("UPDATE codex_invocations SET total_tokens = ?1, cost = ?2 WHERE id = ?3")
        .bind(15_i64)
        .bind(1.5_f64)
        .bind(live_id)
        .execute(tx.as_mut())
        .await
        .expect("update live invocation before recompute");
    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[live_id])
        .await
        .expect("recompute invocation hourly rollups without archive dependency");
    tx.commit().await.expect("commit recompute tx");

    let after = sqlx::query_as::<_, StatsRow>(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load post-recompute bucket totals");
    assert_eq!(after.total_count, 1);
    assert_eq!(after.success_count, Some(1));
    assert_eq!(after.failure_count, Some(0));
    assert_eq!(after.total_tokens, 15);
    assert_f64_close(after.total_cost, 1.5);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_and_cleans_up_pool_upstream_request_attempts() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-attempts").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    config.retention_batch_rows = 10;

    let old_occurred_at = shanghai_local_days_ago(10, 9, 30, 0);
    let recent_occurred_at = shanghai_local_days_ago(1, 9, 30, 0);
    insert_retention_pool_upstream_request_attempt(
        &pool,
        "retention-pool-attempts-old",
        &old_occurred_at,
        Some(7),
        1,
        1,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(200),
        None,
        Some(&old_occurred_at),
        Some(&old_occurred_at),
    )
    .await;
    insert_retention_pool_upstream_request_attempt(
        &pool,
        "retention-pool-attempts-recent",
        &recent_occurred_at,
        Some(8),
        1,
        1,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(200),
        None,
        Some(&recent_occurred_at),
        Some(&recent_occurred_at),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "retention-pool-attempts-old",
        &old_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":7}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt retention");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 1);
    assert_eq!(summary.archive_batches_deleted, 0);

    let remaining_old_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE occurred_at < ?1",
    )
    .bind(shanghai_local_cutoff_string(
        config.pool_upstream_request_attempts_retention_days,
    ))
    .fetch_one(&pool)
    .await
    .expect("count old pool attempt rows");
    assert_eq!(remaining_old_rows, 0);

    let recent_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind("retention-pool-attempts-recent")
    .fetch_one(&pool)
    .await
    .expect("count recent pool attempt rows");
    assert_eq!(recent_rows, 1);

    let archive_batch = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT file_path, archive_expires_at
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load pool attempt archive batch");
    let archive_path = PathBuf::from(&archive_batch.0);
    assert!(archive_path.exists(), "archive file should exist");
    assert!(
        archive_batch.1.is_some(),
        "archive batch should carry expiry"
    );

    sqlx::query(
        "UPDATE archive_batches SET archive_expires_at = ?1 WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .bind("2000-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("expire archive batch");

    sqlx::query("UPDATE long_term_stats_state SET status = 'empty' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats empty before attempt archive cleanup");
    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load pool attempt archive checksum");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_batch.0)
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark pool attempt archive replay complete");

    let cleanup_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt archive ttl cleanup");
    assert_eq!(cleanup_summary.archive_batches_deleted, 1);
    assert!(
        !archive_path.exists(),
        "expired pool attempt archive file should be removed"
    );

    let remaining_archive_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining pool attempt archive batches");
    assert_eq!(remaining_archive_batches, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_accumulates_pool_upstream_hourly_history_when_reusing_monthly_batch_file() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-hourly-accumulates").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 365;
    config.retention_batch_rows = 1;

    let binding_key = "fpn-retention-hourly-accumulates";
    let archive_month_prefix = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(31))
    .format("%Y-%m")
    .to_string();
    let first_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-10 09:30:00"))
        .expect("first attempt timestamp should parse");
    let second_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-11 09:30:00"))
        .expect("second attempt timestamp should parse");
    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-accumulates-success",
        first_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let first_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run first pool upstream retention pass");
    assert_eq!(first_summary.pool_upstream_request_attempt_rows_archived, 1);

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-accumulates-failure",
        second_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let second_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run second pool upstream retention pass");
    assert_eq!(
        second_summary.pool_upstream_request_attempt_rows_archived,
        1
    );

    let archive_batches =
        sqlx::query_as::<_, (i64, String, String, Option<String>, Option<String>, i64)>(
            r#"
        SELECT id, month_key, file_path, day_key, part_key, row_count
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id
        "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load retained pool upstream archive batches");
    let archive_batch_count = archive_batches.len() as i64;
    assert_eq!(
        archive_batch_count, 1,
        "both retention passes should append into the same monthly archive batch: {:?}",
        archive_batches
    );

    let materialized = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(binding_key)
    .fetch_one(&pool)
    .await
    .expect("load materialized pool upstream hourly history");
    assert_eq!(materialized.0, 1);
    assert_eq!(materialized.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_keeps_preexisting_pool_node_health_month_archives_pending_after_append() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-hourly-preexisting-pending").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 365;
    config.retention_batch_rows = 1;

    let binding_key = "fpn-retention-hourly-preexisting-pending";
    let archive_month_prefix = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(31))
    .format("%Y-%m")
    .to_string();
    let first_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-12 09:30:00"))
        .expect("first attempt timestamp should parse");
    let second_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-13 09:30:00"))
        .expect("second attempt timestamp should parse");

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-preexisting-pending-success",
        first_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let first_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run first pool upstream retention pass");
    assert_eq!(first_summary.pool_upstream_request_attempt_rows_archived, 1);

    let archive_path = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load existing pool upstream month archive path");

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached pool node health archive rows");
    sqlx::query("DELETE FROM pool_upstream_node_health_hourly_archive")
        .execute(&pool)
        .await
        .expect("clear cached pool node health hourly archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND target IN (?1, ?2)
          AND file_path = ?3
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("clear pool node health replay markers for existing archive");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("clear historical rollup materialized timestamp");

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-preexisting-pending-failure",
        second_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let second_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run second pool upstream retention pass");
    assert_eq!(
        second_summary.pool_upstream_request_attempt_rows_archived,
        1
    );

    let pending_cache_batches = pending_pool_upstream_node_health_archive_batches(&pool)
        .await
        .expect("count pending cached pool node health archives");
    let pending_hourly_batches = pending_pool_upstream_node_health_hourly_archive_batches(&pool)
        .await
        .expect("count pending hourly pool node health archives");
    assert_eq!(pending_cache_batches, 1);
    assert_eq!(pending_hourly_batches, 1);

    let replay_markers = sqlx::query_as::<_, (String, i64)>(
        r#"
        SELECT target, COUNT(*) AS replay_count
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
          AND target IN (?2, ?3)
        GROUP BY target
        ORDER BY target
        "#,
    )
    .bind(&archive_path)
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .fetch_all(&pool)
    .await
    .expect("load replay markers for existing pool node health archive");
    assert!(
        replay_markers.is_empty(),
        "preexisting month archives should stay pending after append until full backfill runs: {:?}",
        replay_markers
    );

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load historical rollup materialized timestamp after append");
    assert!(materialized_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn refreshing_pool_node_health_hourly_cache_from_row_cache_is_idempotent() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-hourly-cache-idempotent").await;
    let archive_batch_id = 42_i64;
    let archive_file_path = temp_dir
        .join("hourly-cache-idempotent.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-hourly-cache-idempotent";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(40))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(15)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("insert cached pool node health row");
    }

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_hourly_archive (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(pool_upstream_node_health_archive_identity_for_batch_id(
        archive_batch_id,
    ))
    .bind(archive_batch_id)
    .bind(&archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .bind(9_i64)
    .bind(4_i64)
    .execute(&pool)
    .await
    .expect("insert stale cached hourly row");

    for _ in 0..2 {
        let mut tx = pool.begin().await.expect("begin hourly refresh tx");
        refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
            tx.as_mut(),
            archive_batch_id,
            &archive_file_path,
        )
        .await
        .expect("refresh hourly rows from cached rows");
        tx.commit().await.expect("commit hourly refresh tx");
    }

    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
          AND proxy_binding_key_snapshot = ?2
          AND bucket_start_epoch = ?3
        "#,
    )
    .bind(archive_batch_id)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .fetch_one(&pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed.0, 1);
    assert_eq!(refreshed.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_node_health_hourly_backfill_waits_for_cache_replay_and_refreshes_from_cache() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-hourly-cache-replay-gate").await;
    let archive_file_path = temp_dir
        .join("hourly-cache-replay-gate.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-hourly-cache-replay-gate";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let month_key = archive_month_key_from_day_key(&occurred_at[..10])
        .expect("derive archive month key from occurred_at");
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&month_key)
    .bind(&archive_file_path)
    .bind("hourly-cache-replay-gate")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert pool upstream archive batch");
    let archive_batch_id: i64 = sqlx::query_scalar(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        LIMIT 1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load pool upstream archive batch id");

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("insert cached pool node health row");
    }

    let first = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("hourly backfill should wait for cache replay");
    assert_eq!(first.materialized_batches, 0);
    assert_eq!(first.pending_batches, 1);
    let hourly_rows_before: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .fetch_one(&pool)
    .await
    .expect("count hourly rows before cache replay completes");
    assert_eq!(hourly_rows_before, 0);

    let archive_sha256: String = sqlx::query_scalar(
        r#"
        SELECT sha256
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load pool upstream archive checksum");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind("pool_upstream_request_attempts")
    .bind(&archive_file_path)
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("mark cached pool node health replay complete");

    let second = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("hourly backfill should refresh from cached pool node health rows");
    assert_eq!(second.materialized_batches, 1);
    assert_eq!(second.pending_batches, 0);

    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
          AND proxy_binding_key_snapshot = ?2
          AND bucket_start_epoch = ?3
        "#,
    )
    .bind(archive_batch_id)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .fetch_one(&pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed.0, 1);
    assert_eq!(refreshed.1, 1);

    let hourly_replayed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count hourly replay markers after cache refresh");
    assert_eq!(hourly_replayed, 1);

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = 'replaced-hourly-cache-replay-gate'
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("replace pool upstream archive checksum");

    assert_eq!(
        pending_pool_upstream_node_health_archive_batches(&pool)
            .await
            .expect("load cache archives pending after checksum replacement"),
        1,
        "a stale cache replay marker must reopen the archive"
    );
    assert_eq!(
        pending_pool_upstream_node_health_hourly_archive_batches(&pool)
            .await
            .expect("load hourly archives pending after checksum replacement"),
        1,
        "a stale hourly replay marker must reopen the archive"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn node_health_cache_replacement_discards_removed_archive_attempts_before_marking_sha_b() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("node-health-cache-replacement").await;
    let archive_path = temp_dir.join("node-health-cache-replacement.sqlite.gz");
    let archive_db_path = temp_dir.join("node-health-cache-replacement.sqlite");
    let archive_file_path = archive_path.to_string_lossy().to_string();
    let binding_key = "node-health-cache-replacement";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );

    fs::File::create(&archive_db_path).expect("create replacement archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open replacement archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create replacement archive schema");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            proxy_binding_key_snapshot,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            finished_at,
            status,
            first_byte_latency_ms,
            created_at
        )
        VALUES (1, 'node-health-cache-replacement-keep', ?1, '/v1/responses', 'pool', ?2, 0, 0, 0, ?1, ?3, 120.0, ?1)
        "#,
    )
    .bind(&occurred_at)
    .bind(binding_key)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .execute(&archive_pool)
    .await
    .expect("write surviving replacement archive attempt");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress replacement archive");
    let replacement_sha = sha256_hex_file(&archive_path).expect("hash replacement archive");

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
        VALUES ('pool_upstream_request_attempts', ?1, ?2, ?3, 1, ?4, ?5, ?5, datetime('now'))
        "#,
    )
    .bind(&occurred_at[..7])
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("publish replacement archive manifest");

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("seed stale node health cache row");
    }
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target,
                dataset,
                file_path,
                archive_sha256,
                replayed_at
            )
            VALUES (?1, 'pool_upstream_request_attempts', ?2, 'archive-sha-a', datetime('now'))
            "#,
        )
        .bind(target)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("seed stale node health replay marker");
    }

    let cache_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("replace stale node health cache from archive SHA B");
    assert_eq!(cache_summary.materialized_batches, 1);
    assert_eq!(cache_summary.pending_batches, 0);
    let cached = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            COUNT(*),
            COALESCE(SUM(is_success), 0),
            COALESCE(SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END), 0)
        FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load replaced node health cache rows");
    assert_eq!(cached, (1, 1, 0));

    let hourly_summary = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("rebuild node health hourly rows from replaced cache");
    assert_eq!(hourly_summary.materialized_batches, 1);
    assert_eq!(hourly_summary.pending_batches, 0);
    let hourly = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load node health hourly rows after cache replacement");
    assert_eq!(hourly, (1, 0));
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        let marked_sha: String = sqlx::query_scalar(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' AND file_path = ?2",
        )
        .bind(target)
        .bind(&archive_file_path)
        .fetch_one(&pool)
        .await
        .expect("load replacement replay marker");
        assert_eq!(marked_sha, replacement_sha);
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn node_health_archives_with_blank_manifest_sha_stay_quarantined() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("node-health-blank-manifest-quarantine").await;
    let archive_file_path = temp_dir
        .join("node-health-blank-manifest-quarantine.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "node-health-blank-manifest-quarantine";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let month_key = archive_month_key_from_day_key(&occurred_at[..10])
        .expect("derive archive month key from occurred_at");
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

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
        VALUES ('pool_upstream_request_attempts', ?1, ?2, '', 1, ?3, ?4, ?4, datetime('now'))
        "#,
    )
    .bind(&month_key)
    .bind(&archive_file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert blank-manifest pool upstream archive batch");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load blank-manifest archive batch id");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_archive (
            archive_file_path,
            archived_row_id,
            occurred_at,
            proxy_binding_key_snapshot,
            is_success,
            latency_ms
        )
        VALUES (?1, 1, ?2, ?3, 0, NULL)
        "#,
    )
    .bind(&archive_file_path)
    .bind(&occurred_at)
    .bind(binding_key)
    .execute(&pool)
    .await
    .expect("seed node health cache protected by blank manifest");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_hourly_archive (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 0, 1)
        "#,
    )
    .bind(pool_upstream_node_health_archive_identity_for_batch_id(
        archive_batch_id,
    ))
    .bind(archive_batch_id)
    .bind(&archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .execute(&pool)
    .await
    .expect("seed node health hourly cache protected by blank manifest");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, '')",
        )
        .bind(target)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("seed blank node health replay marker");
    }

    for _ in 0..2 {
        let cache = backfill_pool_upstream_node_health_archives(&pool, None, None)
            .await
            .expect("blank-manifest cache archive must remain quarantined");
        assert_eq!(cache.materialized_batches, 0);
        assert_eq!(cache.pending_batches, 1);
        let hourly = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
            .await
            .expect("blank-manifest hourly archive must remain quarantined");
        assert_eq!(hourly.materialized_batches, 0);
        assert_eq!(hourly.pending_batches, 1);
    }

    let cached: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(is_success), 0) FROM pool_upstream_node_health_archive WHERE archive_file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load quarantined node health cache");
    assert_eq!(cached, (1, 0));
    let hourly: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0) FROM pool_upstream_node_health_hourly_archive WHERE archive_file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load quarantined node health hourly cache");
    assert_eq!(hourly, (1, 0, 1));
    let replay_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1 AND archive_sha256 = ''",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count blank replay markers left quarantined");
    assert_eq!(replay_marker_count, 2);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn missing_pool_node_health_archives_clear_stale_cached_rows_before_marking_replayed() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-missing-archive-clears-cache").await;
    let archive_file_path = temp_dir
        .join("missing-node-health-archive.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-missing-archive-clears-cache";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(0)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );

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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&occurred_at[..7])
    .bind(&archive_file_path)
    .bind("missing-node-health-archive")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing pool upstream archive manifest");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_archive (
            archive_file_path,
            archived_row_id,
            occurred_at,
            proxy_binding_key_snapshot,
            is_success,
            latency_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(&archive_file_path)
    .bind(1_i64)
    .bind(&occurred_at)
    .bind(binding_key)
    .bind(1_i64)
    .bind(120.0_f64)
    .execute(&pool)
    .await
    .expect("seed stale cached node health row");

    let summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("backfill should clear stale cached rows for missing archives");
    assert_eq!(summary.pending_batches, 0);

    let cached_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count cached rows after missing archive replay");
    assert_eq!(cached_rows, 0);

    let replayed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count replay marker for missing archive");
    assert_eq!(replayed, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_pool_upstream_archives_waits_for_cache_replay_completion() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("pool-node-health-cleanup-cache-replay-gate").await;
    let coverage_end_at = shanghai_local_days_ago(14, 9, 0, 0);
    let archive_file_path = archive_batch_file_path(
        &config,
        "pool_upstream_request_attempts",
        &coverage_end_at[..7],
    )
    .expect("resolve expired pool upstream archive path");
    fs::create_dir_all(archive_file_path.parent().expect("archive parent"))
        .expect("create archive parent for cleanup gate");
    fs::write(&archive_file_path, b"placeholder-archive")
        .expect("seed raw archive file for cleanup gate");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            archive_expires_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_file_path.to_string_lossy().to_string())
    .bind("cache-replay-gate")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(shanghai_local_days_ago(1, 0, 0, 0))
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert expired pool upstream archive batch");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind("pool_upstream_request_attempts")
    .bind(archive_file_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark hourly replay complete while cache replay stays pending");

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should respect pending cache replay");
    assert_eq!(deleted, 0);
    assert!(
        archive_file_path.exists(),
        "raw archive must remain until cache replay completes"
    );

    let remaining_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining pool upstream archive batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_pool_upstream_archives_preserves_recent_exact_window_history() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("pool-node-health-cleanup-window-gate").await;
    let coverage_end_at = shanghai_local_days_ago(2, 9, 0, 0);
    let archive_file_path = archive_batch_file_path(
        &config,
        "pool_upstream_request_attempts",
        &coverage_end_at[..7],
    )
    .expect("resolve recent pool upstream archive path");
    fs::create_dir_all(archive_file_path.parent().expect("archive parent"))
        .expect("create archive parent for window gate");
    fs::write(&archive_file_path, b"placeholder-archive")
        .expect("seed raw archive file for window gate");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            archive_expires_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_file_path.to_string_lossy().to_string())
    .bind("recent-window-gate")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(shanghai_local_days_ago(1, 0, 0, 0))
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert recent expired pool upstream archive batch");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(target)
        .bind("pool_upstream_request_attempts")
        .bind(archive_file_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark node health replay complete");
    }

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep recent exact node health history");
    assert_eq!(deleted, 0);
    assert!(
        archive_file_path.exists(),
        "raw archive should remain while Live/Settings windows can still overlap it"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_reuses_stable_temp_db_when_budget_is_hit() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-temp-reuse").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;

    let old_occurred_at = shanghai_local_days_ago(10, 9, 30, 0);
    let old_base = parse_shanghai_local_naive(&old_occurred_at).expect("valid shanghai local");
    for idx in 0..row_count {
        let occurred_at = format_naive(old_base + ChronoDuration::seconds(idx as i64));
        insert_retention_pool_upstream_request_attempt(
            &pool,
            &format!("pool-node-health-archive-temp-reuse-old-{idx}"),
            &occurred_at,
            Some(7),
            1,
            1,
            1,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            Some(200),
            None,
            Some(&occurred_at),
            Some(&occurred_at),
        )
        .await;
    }

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt retention");
    assert_eq!(
        summary.pool_upstream_request_attempt_rows_archived,
        row_count
    );

    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(&pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .execute(&pool)
    .await
    .expect("clear node health archive replay progress");

    let first = backfill_pool_upstream_node_health_archives(
        &pool,
        Some(1),
        Some(std::time::Duration::from_millis(1)),
    )
    .await
    .expect("run budget-limited node health archive backfill");
    assert!(first.hit_budget);
    assert!(
        temp_path.exists(),
        "budget-limited backfill should keep a stable temp sqlite for reuse"
    );

    let second = backfill_pool_upstream_node_health_archives(
        &pool,
        Some(1),
        Some(std::time::Duration::from_millis(1)),
    )
    .await
    .expect("rerun budget-limited node health archive backfill");
    let matching_temp_files = fs::read_dir(
        temp_path
            .parent()
            .expect("stable temp sqlite should have a parent directory"),
    )
    .expect("list archive temp directory")
    .filter_map(|entry| entry.ok())
    .filter(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .ends_with(".pool-upstream-node-health.sqlite")
    })
    .count();
    assert!(
        matching_temp_files <= 1,
        "budget-limited retries must not leak multiple temp sqlite files"
    );
    if second.hit_budget {
        assert!(
            temp_path.exists(),
            "stable temp sqlite should be reused across passes"
        );
        let final_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
            .await
            .expect("finish node health archive backfill");
        assert!(!final_summary.hit_budget);
        assert_eq!(final_summary.pending_batches, 0);
        assert!(
            !temp_path.exists(),
            "completed node health archive replay should clean up the stable temp sqlite"
        );
    } else {
        assert_eq!(second.pending_batches, 0);
        assert!(
            !temp_path.exists(),
            "a completed retry should clean up the stable temp sqlite instead of leaking it"
        );
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_marks_missing_archives_replayed() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-missing-archive").await;
    let missing_occurred_at = shanghai_local_days_ago(45, 9, 0, 0);
    let month_key = missing_occurred_at[..7].to_string();
    let missing_archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &month_key)
            .expect("resolve missing pool node health archive path");

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
    .bind("pool_upstream_request_attempts")
    .bind(&month_key)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&missing_occurred_at)
    .bind(&missing_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing pool node health archive manifest");

    let pending_before = pending_pool_upstream_node_health_archive_batches(&pool)
        .await
        .expect("count pending pool node health archive batches before replay");
    assert_eq!(pending_before, 1);

    let summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("missing pool node health archive should be marked replayed");
    assert!(!summary.hit_budget);
    assert_eq!(summary.pending_batches, 0);

    let replay_marked: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count replay markers for missing pool node health archive");
    assert_eq!(replay_marked, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_refreshes_stale_temp_after_archive_rewrite() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-refresh-temp").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;
    let binding_key = "stale-temp-rewrite-binding";

    let old_occurred_at = shanghai_local_days_ago(10, 11, 0, 0);
    let old_base = parse_shanghai_local_naive(&old_occurred_at).expect("valid shanghai local");
    for idx in 0..row_count {
        let occurred_at = Shanghai
            .from_local_datetime(&(old_base + ChronoDuration::seconds(idx as i64)))
            .single()
            .expect("valid shanghai local old pool attempt")
            .with_timezone(&Utc);
        seed_pool_upstream_attempt_at(
            &pool,
            &format!("pool-node-health-archive-refresh-temp-old-{idx}"),
            occurred_at,
            Some(binding_key),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )
        .await;
    }

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run initial pool attempt retention");
    assert_eq!(
        summary.pool_upstream_request_attempt_rows_archived,
        row_count
    );

    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load rewritten pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(&pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .execute(&pool)
    .await
    .expect("clear node health archive replay progress");

    let mut first = None;
    let mut partial_cursor = 0_i64;
    for budget_ms in [1_u64, 5, 10, 20, 40] {
        let summary = backfill_pool_upstream_node_health_archives(
            &pool,
            Some(1),
            Some(std::time::Duration::from_millis(budget_ms)),
        )
        .await
        .expect("run budget-limited node health archive backfill before archive rewrite");
        partial_cursor = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .fetch_optional(&pool)
        .await
        .expect("load partial node health archive replay cursor")
        .unwrap_or(0);
        first = Some(summary);
        if partial_cursor > 0 {
            break;
        }
    }
    let first = first.expect("expected at least one budget-limited replay attempt");
    assert!(first.hit_budget);
    assert!(
        partial_cursor > 0,
        "stale-temp rewrite coverage requires a partial replay cursor before the month archive is rewritten"
    );
    assert!(
        temp_path.exists(),
        "budget-limited replay should keep a temp sqlite"
    );

    std::thread::sleep(std::time::Duration::from_millis(10));
    let appended_occurred_at = Shanghai
        .from_local_datetime(&(old_base + ChronoDuration::seconds(row_count as i64 + 60)))
        .single()
        .expect("valid shanghai local appended pool attempt")
        .with_timezone(&Utc);
    seed_pool_upstream_attempt_at(
        &pool,
        "pool-node-health-archive-refresh-temp-appended",
        appended_occurred_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let second_retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("rerun retention so the month archive gets rewritten with a new appended row");
    assert_eq!(
        second_retention.pool_upstream_request_attempt_rows_archived,
        1
    );

    let final_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("finish node health archive replay after archive rewrite");
    assert!(!final_summary.hit_budget);
    assert_eq!(final_summary.pending_batches, 0);

    let cached_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_node_health_archive")
            .fetch_one(&pool)
            .await
            .expect("count cached node health archive rows after archive rewrite");
    assert_eq!(
        cached_rows,
        (row_count + 1) as i64,
        "rewritten month archives must invalidate stale temp sqlite copies so appended rows are replayed"
    );
    assert!(
        !temp_path.exists(),
        "completed replay after archive rewrite should clean up the temp sqlite"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn cleanup_stale_archive_temp_files_removes_only_old_archive_residue() {
    let temp_dir = make_temp_test_dir("archive-temp-janitor");
    let mut config = test_config();
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    let nested_dir = archive_root.join("codex_invocations/2026/03/25");
    fs::create_dir_all(&nested_dir).expect("create nested archive dir");

    let stale_temp = nested_dir.join("part-000001.sqlite.gz.1.partial.sqlite");
    let fresh_temp = nested_dir.join("part-000002.sqlite.gz.1.partial.sqlite");
    let official = nested_dir.join("part-000003.sqlite.gz");
    fs::write(&stale_temp, b"stale temp").expect("write stale temp");
    fs::write(&fresh_temp, b"fresh temp").expect("write fresh temp");
    fs::write(&official, b"official archive").expect("write official archive");
    set_file_mtime_seconds_ago(&stale_temp, DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS + 60);
    set_file_mtime_seconds_ago(&fresh_temp, 60);

    let summary = cleanup_stale_archive_temp_files(&config, false).expect("run archive janitor");
    assert_eq!(summary.stale_temp_files_removed, 1);
    assert!(summary.stale_temp_bytes_removed > 0);
    assert!(!stale_temp.exists(), "stale temp should be deleted");
    assert!(fresh_temp.exists(), "fresh temp should be kept");
    assert!(official.exists(), "official archive should be kept");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn verify_archive_storage_reports_missing_orphan_and_temp_files() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("verify-archive-storage").await;
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    fs::create_dir_all(&archive_root).expect("create archive root");

    let day_key = "2025-01-01";
    let missing_path = archive_segment_file_path(
        &config,
        "codex_invocations",
        day_key,
        "part-000001",
        ArchiveFileCodec::Gzip,
    )
    .expect("resolve missing segment path");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, day_key, part_key, file_path, sha256, row_count, status, layout, codec, writer_version, cleanup_state, coverage_start_at, coverage_end_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(day_key)
    .bind("part-000001")
    .bind(missing_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert missing segment manifest");

    let orphan_path = archive_root.join("codex_invocations/2025/01/01/orphan.sqlite.gz");
    fs::create_dir_all(orphan_path.parent().expect("orphan parent")).expect("create orphan parent");
    fs::write(&orphan_path, b"orphan archive").expect("write orphan archive");
    let stale_temp =
        archive_root.join("codex_invocations/2025/01/01/part-000009.sqlite.gz.1.partial.sqlite");
    fs::write(&stale_temp, b"stale temp").expect("write stale temp");
    set_file_mtime_seconds_ago(&stale_temp, DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS + 60);

    let summary = verify_archive_storage(&pool, &config)
        .await
        .expect("verify archive storage");
    assert_eq!(summary.manifest_rows, 1);
    assert_eq!(summary.missing_files, 1);
    assert_eq!(summary.orphan_files, 1);
    assert_eq!(summary.stale_temp_files, 1);
    assert!(summary.stale_temp_bytes > 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn prune_archive_batches_removes_expired_segments_and_legacy_batches() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("prune-archive-batches").await;
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    fs::create_dir_all(&archive_root).expect("create archive root");

    let segment_path = archive_segment_file_path(
        &config,
        "codex_invocations",
        "2025-01-02",
        "part-000001",
        ArchiveFileCodec::Gzip,
    )
    .expect("resolve segment path");
    fs::create_dir_all(segment_path.parent().expect("segment parent"))
        .expect("create segment parent");
    let segment_source_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "prune-segment-source",
        &[(
            1_i64,
            "prune-segment-source",
            "2025-01-02 09:00:00",
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    fs::rename(&segment_source_path, &segment_path)
        .expect("move segment archive into fixture path");
    let segment_sha256 = sha256_hex_file(&segment_path).expect("load segment archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET day_key = '2025-01-02',
            part_key = 'part-000001',
            file_path = ?1,
            sha256 = ?2,
            layout = ?3,
            codec = ?4,
            writer_version = ?5,
            cleanup_state = ?6,
            coverage_start_at = '2025-01-02 09:00:00',
            coverage_end_at = '2025-01-02 09:00:00',
            archive_expires_at = '2000-01-01 00:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?7
        "#,
    )
    .bind(segment_path.to_string_lossy().to_string())
    .bind(segment_sha256.clone())
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind(segment_source_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("adapt expired segment manifest to fixture path");
    sqlx::query("UPDATE long_term_stats_state SET status = 'empty' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats as ready for legacy prune fixture");

    let legacy_path = archive_batch_file_path(&config, "codex_invocations", "2024-12")
        .expect("resolve legacy batch path");
    fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("create legacy parent");
    let legacy_source_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "prune-legacy-source",
        &[(
            2_i64,
            "prune-legacy-source",
            "2024-12-01 09:00:00",
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    fs::rename(&legacy_source_path, &legacy_path).expect("move legacy archive into fixture path");
    let legacy_sha256 = sha256_hex_file(&legacy_path).expect("load legacy archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET file_path = ?1,
            sha256 = ?2,
            layout = ?3,
            codec = ?4,
            writer_version = ?5,
            cleanup_state = ?6,
            coverage_start_at = '2024-12-01 09:00:00',
            coverage_end_at = '2024-12-01 09:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?7
        "#,
    )
    .bind(legacy_path.to_string_lossy().to_string())
    .bind(legacy_sha256.clone())
    .bind(ARCHIVE_LAYOUT_LEGACY_MONTH)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind(legacy_source_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("adapt legacy archive manifest to fixture path");
    for (archive_path, archive_sha256) in [
        (segment_path.to_string_lossy().to_string(), segment_sha256),
        (legacy_path.to_string_lossy().to_string(), legacy_sha256),
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(archive_path)
        .bind(archive_sha256)
        .execute(&pool)
        .await
        .expect("mark legacy prune archive replay complete");
    }

    let summary = prune_archive_batches(&pool, &config, false)
        .await
        .expect("prune archive batches");
    assert_eq!(
        summary.expired_archive_batches_deleted + summary.legacy_archive_batches_deleted,
        2
    );
    assert!(!segment_path.exists(), "expired segment should be removed");
    assert!(!legacy_path.exists(), "legacy archive should be removed");

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_keeps_retention_materialized_totals_unchanged_while_reopening_missing_breakdown_backfill()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-retention-accounted").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let bucket_start = local_naive_to_utc(
        parse_shanghai_local_naive(&old_invocation).expect("valid old invocation time"),
        Shanghai,
    );
    insert_invocation_hourly_rollup_bucket(&pool, bucket_start, SOURCE_PROXY, 1, 1, 0, 42, 0.42)
        .await;
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempt_hourly (
            proxy_key,
            bucket_start_epoch,
            attempts,
            success_count,
            failure_count,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
        "#,
    )
    .bind("proxy-retention-accounted")
    .bind(bucket_start.timestamp())
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .execute(&pool)
    .await
    .expect("seed materialized forward proxy hourly rollup");
    for table in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        let statement = format!(
            r#"
            INSERT INTO {table} (
                bucket_start_epoch,
                source,
                upstream_account_id,
                total_count,
                success_count,
                failure_count,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            "#
        );
        sqlx::query(&statement)
            .bind(bucket_start.timestamp())
            .bind(SOURCE_PROXY)
            .bind(17_i64)
            .bind(1_i64)
            .bind(1_i64)
            .bind(0_i64)
            .execute(&pool)
            .await
            .expect("seed account stats rollup guard");
    }

    let invocation_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly totals before retention");
    let forward_proxy_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
    )
    .bind("proxy-retention-accounted")
    .fetch_one(&pool)
    .await
    .expect("load forward proxy hourly totals before retention");

    for (dataset, file_name) in [
        ("codex_invocations", "materialized-invocation.sqlite.gz"),
        (
            "forward_proxy_attempts",
            "materialized-forward-proxy.sqlite.gz",
        ),
    ] {
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
                historical_rollups_materialized_at,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))
            "#,
        )
        .bind(dataset)
        .bind(&old_invocation[..7])
        .bind(temp_dir.join(file_name).to_string_lossy().to_string())
        .bind(format!("{dataset}-sha"))
        .bind(1_i64)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&old_invocation)
        .bind(&old_invocation)
        .execute(&pool)
        .await
        .expect("seed materialized archive batch");
    }

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("replay hourly rollups after retention");
    let invocation_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly totals after bootstrap");
    let forward_proxy_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
    )
    .bind("proxy-retention-accounted")
    .fetch_one(&pool)
    .await
    .expect("load forward proxy hourly totals after bootstrap");

    assert_eq!(invocation_total_before, 1);
    assert_eq!(invocation_total_after, invocation_total_before);
    assert_eq!(forward_proxy_total_before, 1);
    assert_eq!(forward_proxy_total_after, forward_proxy_total_before);

    let invocation_materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_invocations' AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count materialized invocation archive batches");
    assert_eq!(invocation_materialized_batches, 0);

    let forward_proxy_materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'forward_proxy_attempts' AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count materialized forward proxy archive batches");
    assert_eq!(forward_proxy_materialized_batches, 1);

    let replay_marker_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hourly_rollup_archive_replay")
            .fetch_one(&pool)
            .await
            .expect("count hourly rollup archive replay markers");
    assert_eq!(replay_marker_count, 0);

    let historical_backfill_snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after reopening missing breakdown backfill");
    assert_eq!(historical_backfill_snapshot.legacy_archive_pending, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_ignores_missing_replay_markers() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-missing-invocation-target").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let payload = r#"{"endpoint":"/v1/responses","promptCacheKey":"cache-replay","upstreamAccountId":17,"upstreamAccountName":"Replay Account","stickyKey":"sticky-replay"}"#;
    insert_retention_invocation(
        &pool,
        "hourly-rollup-missing-invocation-target",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some(payload),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention before bootstrap replay");
    assert_eq!(summary.invocation_rows_archived, 1);

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap hourly rollups after retention");

    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archived codex_invocations batch path");
    let invocation_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation totals before marker repair");

    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("delete one invocation replay marker");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should replay only the missing target");

    let invocation_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation totals after marker repair");
    let repaired_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load repaired replay marker count");

    assert_eq!(invocation_total_before, 1);
    assert_eq!(invocation_total_after, invocation_total_before);
    assert_eq!(repaired_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_backfills_legacy_invocation_rollup_aggregate_columns() {
    let (pool, config, temp_dir) = retention_fresh_schema_test_pool_and_config(
        "legacy-rollup-first-response-byte-total-backfill",
    )
    .await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-rollup-first-response-byte-total-backfill",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_req_read_ms = ?1,
            t_req_parse_ms = ?2,
            t_upstream_connect_ms = ?3,
            t_upstream_ttfb_ms = ?4
        WHERE invoke_id = ?5
        "#,
    )
    .bind(120.0_f64)
    .bind(80.0_f64)
    .bind(43_000.0_f64)
    .bind(690.0_f64)
    .bind("legacy-rollup-first-response-byte-total-backfill")
    .execute(&pool)
    .await
    .expect("seed staged latency fields");

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old invocation before schema migration");
    assert_eq!(summary.invocation_rows_archived, 1);

    sqlx::query("ALTER TABLE invocation_rollup_hourly RENAME TO invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("rename current invocation rollup table");
    sqlx::query(
        r#"
        CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy invocation rollup table");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        )
        SELECT
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        FROM invocation_rollup_hourly_current
        "#,
    )
    .execute(&pool)
    .await
    .expect("copy legacy invocation rollup rows");
    sqlx::query("DROP TABLE invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("drop current invocation rollup table copy");

    ensure_schema(&pool)
        .await
        .expect("ensure schema should backfill first-response-byte totals");

    let row = sqlx::query_as::<_, (i64, f64, f64, String)>(
        r#"
        SELECT
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load backfilled invocation rollup row");

    assert_eq!(
        row.0, 1,
        "legacy rollup row should gain one first-response sample"
    );
    assert_eq!(row.1, 43_890.0);
    assert_eq!(row.2, 43_890.0);
    assert_ne!(
        row.3, "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]",
        "backfill should write a non-empty histogram"
    );

    let terminal = sqlx::query_as::<_, (i64, i64, f64, i64)>(
        r#"
        SELECT terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load terminal proof backfill");
    assert_eq!(terminal.0, 1);
    assert_eq!(terminal.1, 42);
    assert_eq!(terminal.2, 0.42);
    assert_eq!(terminal.3, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_reconciles_legacy_rollups_when_sources_are_complete() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("legacy-rollup-terminal-proof-partial-source")
            .await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-rollup-terminal-proof-partial-source",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollup");
    sqlx::query(
        "UPDATE invocation_rollup_hourly SET total_count = 2, total_tokens = 84, total_cost = 0.84 WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed canonical totals that cannot be reconstructed from the remaining source");

    sqlx::query("ALTER TABLE invocation_rollup_hourly RENAME TO invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("rename current invocation rollup table");
    sqlx::query(
        r#"
        CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy invocation rollup table");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        )
        SELECT
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        FROM invocation_rollup_hourly_current
        "#,
    )
    .execute(&pool)
    .await
    .expect("copy legacy invocation rollup row");
    sqlx::query("DROP TABLE invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("drop current invocation rollup table copy");

    ensure_schema(&pool)
        .await
        .expect("ensure schema should reconcile a legacy canonical row against complete sources");

    let row = sqlx::query_as::<_, (i64, i64, f64, i64)>(
        r#"
        SELECT total_count, total_tokens, total_cost, terminal_proof_complete
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load reconciled legacy rollup");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, 42);
    assert_eq!(row.2, 0.42);
    assert_eq!(
        row.3, 1,
        "complete sources should restore the terminal integrity proof after replacing legacy totals"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn terminal_proof_reconciliation_repairs_a_contradictory_canonical_bucket() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-incremental-reconciliation").await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-incremental-reconciliation",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    let proof_before_reconciliation: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load unproved incremental rollup");
    assert_eq!(
        proof_before_reconciliation, 0,
        "bounded incremental writes cannot certify complete historical source coverage"
    );

    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("reconcile canonical rollup against all available sources");
    sqlx::query(
        "UPDATE invocation_rollup_hourly SET total_count = 9, terminal_count = 9, terminal_tokens = 999, terminal_cost = 9.99, total_tokens = 999, total_cost = 9.99, terminal_proof_complete = 0 WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("corrupt canonical hourly rollup");

    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("complete reconciliation should repair canonical totals");
    let repaired = sqlx::query_as::<_, (i64, i64, i64, f64, i64)>(
        "SELECT total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load repaired canonical rollup");
    assert!(reconciliation.source_complete);
    assert_eq!(repaired, (1, 1, 42, 0.42, 1));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn terminal_proof_is_revoked_when_a_completed_invocation_archive_is_missing() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-missing-archive").await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-missing-archive",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("initial source reconciliation should certify the live-only fixture");
    let proof_before_missing_archive: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load initially certified terminal proof");
    assert_eq!(proof_before_missing_archive, 1);

    let missing_archive_path = temp_dir.join("missing-terminal-proof-source.sqlite.gz");
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
        VALUES ('codex_invocations', '2026-01', ?1, 'missing-terminal-proof-sha', 1, ?2, '2026-01-01 00:00:00', '2026-01-01 00:00:00', datetime('now'))
        "#,
    )
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert missing completed invocation archive manifest");

    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("incomplete reconciliation should not certify a partial source set");
    assert_eq!(reconciliation.applied_rollups, 0);
    assert!(
        !reconciliation.source_complete,
        "a missing completed archive must make source reconciliation incomplete"
    );
    let proof_after_missing_archive: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load terminal proof after missing archive detection");
    assert_eq!(
        proof_after_missing_archive, 0,
        "a completed but unreadable archive makes every canonical terminal proof unavailable"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn terminal_proof_is_revoked_when_a_readable_invocation_archive_hash_mismatches() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-archive-hash-mismatch").await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-archive-hash-mismatch-live",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("initial source reconciliation should certify the live bucket");

    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "terminal-proof-archive-hash-mismatch",
        &[(
            1_i64,
            "terminal-proof-archive-hash-mismatch-archive",
            &shanghai_local_days_ago(20, 9, 0, 0),
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            None,
        )],
    )
    .await;
    sqlx::query("UPDATE archive_batches SET sha256 = 'tampered-archive-sha' WHERE file_path = ?1")
        .bind(archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("corrupt archive manifest identity");

    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("hash mismatch should make the source unavailable without failing refresh");
    let proof_after_hash_mismatch: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load terminal proof after archive hash mismatch");
    assert!(!reconciliation.source_complete);
    assert_eq!(reconciliation.applied_rollups, 0);
    assert_eq!(
        reconciliation.unavailable_archive_file_paths,
        vec![archive_path.to_string_lossy().to_string()]
    );
    assert_eq!(proof_after_hash_mismatch, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn unreadable_archive_does_not_revoke_retired_terminal_proofs() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-retired-source-unavailable-archive").await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-retired-source-unavailable-archive",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("initial source reconciliation should certify the historical bucket");

    let source_start = shanghai_local_days_ago(2, 0, 0, 0)[..10].to_string();
    sqlx::query("UPDATE long_term_stats_state SET integrity_source_start_date = ?1 WHERE id = 1")
        .bind(&source_start)
        .execute(&pool)
        .await
        .expect("record durable source boundary after archive retirement");
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-retired-source-unavailable-archive")
        .execute(&pool)
        .await
        .expect("simulate intentionally retired source rows");
    let missing_archive_path = temp_dir.join("missing-active-terminal-proof-source.sqlite.gz");
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
        VALUES ('codex_invocations', '2026-01', ?1, 'missing-active-terminal-proof-sha', 1, ?2, '2026-01-01 00:00:00', '2026-01-01 00:00:00', datetime('now'))
        "#,
    )
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert missing active invocation archive manifest");

    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("unreadable active archive should retain retired proofs");
    assert!(
        !reconciliation.source_complete,
        "the unavailable active archive must still keep the refresh in error"
    );
    let retired_proof: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load proof for intentionally retired bucket");
    assert_eq!(retired_proof, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn complete_reconciliation_removes_stale_missing_canonical_buckets() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-reconciliation-drops-bucket").await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    for (invoke_id, source) in [
        ("terminal-proof-reconciliation-proxy", SOURCE_PROXY),
        ("terminal-proof-reconciliation-xy", SOURCE_XY),
    ] {
        insert_retention_invocation(
            &pool,
            invoke_id,
            &occurred_at,
            source,
            "success",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":true}",
            None,
            None,
            Some(42),
            Some(0.42),
        )
        .await;
    }
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("initial source reconciliation should certify both buckets");

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-reconciliation-proxy")
        .execute(&pool)
        .await
        .expect("remove one source row from the reconciled source set");
    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("reconciliation should remove a missing canonical bucket");
    let proxy_rollups: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_rollup_hourly WHERE source = ?1")
            .bind(SOURCE_PROXY)
            .fetch_one(&pool)
            .await
            .expect("count removed proxy rollup");
    let xy_proof: i64 = sqlx::query_scalar(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 LIMIT 1",
    )
    .bind(SOURCE_XY)
    .fetch_one(&pool)
    .await
    .expect("load still-reconstructed xy proof");
    assert!(reconciliation.source_complete);
    assert_eq!(reconciliation.invalidated_bucket_start_epochs.len(), 1);
    assert_eq!(proxy_rollups, 0);
    assert_eq!(xy_proof, 1);

    // A prior incomplete run may already have marked the missing bucket untrusted. A complete
    // scan must remove that stale row too; otherwise audits skip the date forever.
    let proxy_bucket_start =
        invocation_bucket_start_epoch(&occurred_at).expect("valid Shanghai source timestamp");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 0, 1, 42, 0.42, 0, 42, 0.42)",
    )
    .bind(proxy_bucket_start)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("insert stale untrusted proxy rollup");
    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("complete reconciliation should remove stale untrusted bucket");
    let proxy_rollups: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_rollup_hourly WHERE source = ?1")
            .bind(SOURCE_PROXY)
            .fetch_one(&pool)
            .await
            .expect("count stale untrusted proxy rollup");
    assert!(reconciliation.source_complete);
    assert_eq!(reconciliation.invalidated_bucket_start_epochs.len(), 1);
    assert_eq!(proxy_rollups, 0);

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-reconciliation-xy")
        .execute(&pool)
        .await
        .expect("remove the final source row from the reconciled source set");
    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("empty reconciliation should remove every remaining active canonical bucket");
    let xy_rollups_after_empty_reconciliation: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_rollup_hourly WHERE source = ?1")
            .bind(SOURCE_XY)
            .fetch_one(&pool)
            .await
            .expect("count rollups after empty reconciliation");
    assert!(reconciliation.source_complete);
    assert_eq!(xy_rollups_after_empty_reconciliation, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn schema_upgrade_preserves_legacy_canonical_history_outside_the_new_source_window() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");
    sqlx::query("DROP TABLE invocation_rollup_hourly")
        .execute(&pool)
        .await
        .expect("replace current canonical table with origin/main shape");
    sqlx::query(
        r#"
        CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy canonical table");
    let legacy_occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    let legacy_bucket =
        invocation_bucket_start_epoch(&legacy_occurred_at).expect("resolve legacy bucket");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 0, 42, 0.42)",
    )
    .bind(legacy_bucket)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed canonical history whose source was retired before upgrade");
    sqlx::query(
        "UPDATE long_term_stats_state SET integrity_source_start_date = NULL, integrity_source_pending_start_date = NULL WHERE id = 1",
    )
    .execute(&pool)
    .await
    .expect("clear new boundary to simulate origin/main state");

    let expected_source_start = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    ensure_schema(&pool)
        .await
        .expect("upgrade legacy canonical rollup schema");

    let preserved: Option<(i64, i64)> = sqlx::query_as(
        "SELECT total_tokens, terminal_proof_complete FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(legacy_bucket)
    .bind(SOURCE_PROXY)
    .fetch_optional(&pool)
    .await
    .expect("load preserved legacy canonical bucket");
    assert_eq!(preserved, Some((42, 0)));
    let source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load migration source boundary");
    assert_eq!(
        source_start.as_deref(),
        Some(expected_source_start.as_str())
    );
}

#[tokio::test]
async fn schema_upgrade_resumes_boundary_bootstrap_after_rollup_columns_are_present() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");
    let legacy_occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    let legacy_bucket =
        invocation_bucket_start_epoch(&legacy_occurred_at).expect("resolve legacy bucket");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 0, 42, 0.42)",
    )
    .bind(legacy_bucket)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed canonical history after every rollup ALTER TABLE completed");
    sqlx::query(
        "UPDATE long_term_stats_state SET integrity_source_start_date = NULL, integrity_source_pending_start_date = NULL WHERE id = 1",
    )
    .execute(&pool)
    .await
    .expect("simulate interruption before durable boundary bootstrap");

    let expected_source_start = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    ensure_schema(&pool)
        .await
        .expect("resume schema migration after completed rollup ALTER TABLE statements");

    let preserved: Option<(i64, i64)> = sqlx::query_as(
        "SELECT total_tokens, terminal_proof_complete FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(legacy_bucket)
    .bind(SOURCE_PROXY)
    .fetch_optional(&pool)
    .await
    .expect("load preserved canonical bucket");
    assert_eq!(preserved, Some((42, 0)));
    let source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load resumed migration source boundary");
    assert_eq!(
        source_start.as_deref(),
        Some(expected_source_start.as_str())
    );
}

#[tokio::test]
async fn reconciliation_keeps_a_pre_retention_live_snapshot_from_deleting_canonical_history() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-consistent-source-snapshot").await;
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await
        .expect("enable concurrent reader and writer fixture");
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-consistent-source-snapshot",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed canonical rollup before reconciliation");

    let snapshot_open = std::sync::Arc::new(tokio::sync::Notify::new());
    let resume_live_scan = std::sync::Arc::new(tokio::sync::Notify::new());
    let reconciliation_pool = pool.clone();
    let snapshot_open_for_task = snapshot_open.clone();
    let resume_live_scan_for_task = resume_live_scan.clone();
    let reconciliation = tokio::spawn(async move {
        backfill_invocation_rollup_hourly_from_sources_with_snapshot_hook(
            &reconciliation_pool,
            move || async move {
                snapshot_open_for_task.notify_one();
                resume_live_scan_for_task.notified().await;
            },
        )
        .await
    });
    snapshot_open.notified().await;

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-consistent-source-snapshot")
        .execute(&pool)
        .await
        .expect("simulate retention removing the live row after snapshot creation");
    resume_live_scan.notify_one();
    let reconciliation = reconciliation
        .await
        .expect("join source reconciliation task");

    if let Err(error) = reconciliation {
        let message = error.to_string().to_ascii_lowercase();
        assert!(
            message.contains("locked") || message.contains("busy"),
            "only a retryable snapshot-write conflict is allowed, got {error}"
        );
    }
    let canonical_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_rollup_hourly WHERE source = ?1")
            .bind(SOURCE_PROXY)
            .fetch_one(&pool)
            .await
            .expect("count canonical row after concurrent retention");
    assert_eq!(
        canonical_rows, 1,
        "a source snapshot that predated retention must not delete its canonical rollup"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn terminal_proof_keeps_retired_buckets_but_removes_missing_boundary_buckets() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("terminal-proof-retired-source-boundary").await;
    let retired_occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    let boundary_occurred_at = shanghai_local_days_ago(2, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "terminal-proof-retired-source-boundary",
        &retired_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "terminal-proof-missing-boundary-bucket",
        &boundary_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("write bounded live rollup increment");
    backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("initial source reconciliation should certify the bucket");

    let source_start = shanghai_local_days_ago(2, 0, 0, 0)[..10].to_string();
    sqlx::query("UPDATE long_term_stats_state SET integrity_source_start_date = ?1 WHERE id = 1")
        .bind(&source_start)
        .execute(&pool)
        .await
        .expect("record durable source boundary after archive retirement");
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-retired-source-boundary")
        .execute(&pool)
        .await
        .expect("simulate intentionally retired source rows");
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("terminal-proof-missing-boundary-bucket")
        .execute(&pool)
        .await
        .expect("simulate a missing source that remains inside the source window");

    let reconciliation = backfill_invocation_rollup_hourly_from_sources(&pool)
        .await
        .expect("reconciliation should distinguish retired and active source buckets");
    assert!(
        reconciliation.source_complete,
        "a complete source scan should remove a missing active-window bucket"
    );
    assert_eq!(reconciliation.invalidated_bucket_start_epochs.len(), 1);
    let proofs_after_reconciliation = sqlx::query_as::<_, (i64, i64)>(
        "SELECT bucket_start_epoch, terminal_proof_complete FROM invocation_rollup_hourly WHERE source = ?1 ORDER BY bucket_start_epoch ASC",
    )
    .bind(SOURCE_PROXY)
    .fetch_all(&pool)
    .await
    .expect("load terminal proofs after source reconciliation");
    assert_eq!(proofs_after_reconciliation.len(), 1);
    assert_eq!(proofs_after_reconciliation[0].1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_backfills_account_usage_status_counts_and_reopens_archive_rollups() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");

    sqlx::query("DROP TABLE upstream_account_usage_hourly")
        .execute(&pool)
        .await
        .expect("drop current account usage hourly table");
    sqlx::query(
        r#"
        CREATE TABLE upstream_account_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            request_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cache_input_tokens INTEGER NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy account usage hourly table");

    let account_id = 42_i64;
    let live_bucket = Utc::now().timestamp() / 3600 * 3600;
    let live_occurred_at = Utc
        .timestamp_opt(live_bucket + 60, 0)
        .single()
        .expect("valid live timestamp")
        .with_timezone(&Shanghai)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    for (idx, status, failure_kind, reasoning_tokens) in [
        (1_i64, "success", None, 3_i64),
        (2_i64, "http_500", Some("upstream_response_failed"), 4_i64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, reasoning_tokens,
                total_tokens, cost, status, error_message, failure_kind, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(idx)
        .bind(format!("legacy-account-status-backfill-{idx}"))
        .bind(&live_occurred_at)
        .bind(SOURCE_PROXY)
        .bind("gpt-5")
        .bind(10_i64)
        .bind(20_i64)
        .bind(reasoning_tokens)
        .bind(30_i64)
        .bind(0.01_f64)
        .bind(status)
        .bind("")
        .bind(failure_kind)
        .bind(json!({ "upstreamAccountId": account_id }).to_string())
        .bind("{}")
        .bind(&live_occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation for legacy account backfill");
    }
    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_hourly (
            bucket_start_epoch, upstream_account_id, request_count, total_tokens, total_cost,
            input_tokens, output_tokens, cache_input_tokens, first_seen_at, last_seen_at
        )
        VALUES (?1, ?2, 2, 60, 0.02, 20, 40, 0, ?3, ?3)
        "#,
    )
    .bind(live_bucket)
    .bind(account_id)
    .bind(&live_occurred_at)
    .execute(&pool)
    .await
    .expect("insert legacy live account usage rollup");

    let archive_bucket = live_bucket - 7 * 24 * 3600;
    let archive_start = Utc
        .timestamp_opt(archive_bucket + 30, 0)
        .single()
        .expect("valid archive start")
        .with_timezone(&Shanghai)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let archive_end = Utc
        .timestamp_opt(archive_bucket + 90, 0)
        .single()
        .expect("valid archive end")
        .with_timezone(&Shanghai)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_hourly (
            bucket_start_epoch, upstream_account_id, request_count, total_tokens, total_cost,
            input_tokens, output_tokens, cache_input_tokens, first_seen_at, last_seen_at
        )
        VALUES (?1, ?2, 1, 10, 0.01, 4, 6, 0, ?3, ?4)
        "#,
    )
    .bind(archive_bucket)
    .bind(account_id)
    .bind(&archive_start)
    .bind(&archive_end)
    .execute(&pool)
    .await
    .expect("insert stale archived account usage rollup");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, status, row_count,
            coverage_start_at, coverage_end_at, historical_rollups_materialized_at
        )
        VALUES ('codex_invocations', '2026-01', '/tmp/account-usage-backfill.sqlite.gz',
                'account-usage-backfill-sha', 'completed', 1, ?1, ?2, datetime('now'))
        "#,
    )
    .bind(&archive_start)
    .bind(&archive_end)
    .execute(&pool)
    .await
    .expect("insert completed archive batch");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES ('upstream_account_usage_hourly', 'codex_invocations',
                '/tmp/account-usage-backfill.sqlite.gz', datetime('now'))
        "#,
    )
    .execute(&pool)
    .await
    .expect("insert stale replay marker");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id)
        VALUES ('codex_invocations', '/tmp/account-usage-backfill.sqlite.gz', 10)
        "#,
    )
    .execute(&pool)
    .await
    .expect("insert stale replay progress");

    ensure_schema(&pool)
        .await
        .expect("migrate account usage status counts");

    let live_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT success_count, failure_count, reasoning_tokens FROM upstream_account_usage_hourly WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(live_bucket)
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load live account status counts");
    let archived_row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upstream_account_usage_hourly WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(archive_bucket)
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load archived stale row count");
    let archive_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = '/tmp/account-usage-backfill.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive materialized marker");
    let stale_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = 'upstream_account_usage_hourly'",
    )
    .fetch_one(&pool)
    .await
    .expect("load stale replay marker count");
    let stale_progress: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_progress WHERE dataset = 'codex_invocations' AND file_path = '/tmp/account-usage-backfill.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load stale replay progress count");

    assert_eq!(live_counts, (1, 1, 7));
    assert_eq!(archived_row_count, 0);
    assert!(archive_materialized_at.is_none());
    assert_eq!(stale_markers, 0);
    assert_eq!(stale_progress, 0);
}

#[tokio::test]
async fn ensure_schema_rebuilds_account_stats_when_live_progress_table_is_missing() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");

    sqlx::query("DROP TABLE upstream_account_stats_hourly")
        .execute(&pool)
        .await
        .expect("drop current account stats hourly table");
    sqlx::query("DROP TABLE upstream_account_stats_minute")
        .execute(&pool)
        .await
        .expect("drop current account stats minute table");
    sqlx::query("DROP TABLE hourly_rollup_live_progress")
        .execute(&pool)
        .await
        .expect("drop hourly rollup live progress table");

    let account_id = 64_i64;
    let occurred_at = Utc::now()
        .with_timezone(&Shanghai)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, model, input_tokens, output_tokens,
            cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message,
            failure_kind, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
    )
    .bind(1_i64)
    .bind("legacy-account-stats-rebuild")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("gpt-5")
    .bind(120_i64)
    .bind(45_i64)
    .bind(0_i64)
    .bind(0_i64)
    .bind(165_i64)
    .bind(0.42_f64)
    .bind("success")
    .bind("")
    .bind(Option::<&str>::None)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .bind("{}")
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert live invocation for account stats rebuild");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_req_read_ms = ?1,
            t_req_parse_ms = ?2,
            t_upstream_connect_ms = ?3,
            t_upstream_ttfb_ms = ?4
        WHERE invoke_id = ?5
        "#,
    )
    .bind(120.0_f64)
    .bind(80.0_f64)
    .bind(43_000.0_f64)
    .bind(690.0_f64)
    .bind("legacy-account-stats-rebuild")
    .execute(&pool)
    .await
    .expect("seed latency fields for account stats rebuild");

    ensure_schema(&pool)
        .await
        .expect("rebuild account stats should recreate live progress table first");

    let hourly_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT total_count, success_count, first_response_byte_total_sample_count
        FROM upstream_account_stats_hourly
        WHERE upstream_account_id = ?1
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load rebuilt account stats hourly row");
    let minute_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT total_count, success_count, first_response_byte_total_sample_count
        FROM upstream_account_stats_minute
        WHERE upstream_account_id = ?1
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load rebuilt account stats minute row");
    let live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_one(&pool)
            .await
            .expect("load recreated live progress cursor");

    assert_eq!(hourly_counts, (1, 1, 1));
    assert_eq!(minute_counts, (1, 1, 1));
    assert_eq!(live_cursor, 1);
}

#[tokio::test]
async fn ensure_schema_backfill_deduplicates_detail_prune_archives() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("legacy-rollup-detail-prune-dedup").await;
    let prune_invocation =
        shanghai_local_days_ago((config.invocation_success_full_days + 2) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-rollup-detail-prune-dedup",
        &prune_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_req_read_ms = ?1,
            t_req_parse_ms = ?2,
            t_upstream_connect_ms = ?3,
            t_upstream_ttfb_ms = ?4
        WHERE invoke_id = ?5
        "#,
    )
    .bind(120.0_f64)
    .bind(80.0_f64)
    .bind(43_000.0_f64)
    .bind(690.0_f64)
    .bind("legacy-rollup-detail-prune-dedup")
    .execute(&pool)
    .await
    .expect("seed staged latency fields");

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before detail prune");
    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention to produce detail-prune archive");

    let live_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1")
            .bind("legacy-rollup-detail-prune-dedup")
            .fetch_one(&pool)
            .await
            .expect("load detail-pruned live invocation count");
    assert_eq!(
        live_row_count, 1,
        "detail-pruned invocation should remain live"
    );

    sqlx::query("ALTER TABLE invocation_rollup_hourly RENAME TO invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("rename current invocation rollup table");
    sqlx::query(
        r#"
        CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy invocation rollup table");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        )
        SELECT
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            updated_at
        FROM invocation_rollup_hourly_current
        "#,
    )
    .execute(&pool)
    .await
    .expect("copy legacy invocation rollup rows");
    sqlx::query("DROP TABLE invocation_rollup_hourly_current")
        .execute(&pool)
        .await
        .expect("drop current invocation rollup table copy");

    ensure_schema(&pool)
        .await
        .expect("ensure schema should deduplicate detail-prune archive rows");

    let row = sqlx::query_as::<_, (i64, i64, f64)>(
        r#"
        SELECT
            total_count,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load deduplicated invocation rollup row");

    assert_eq!(
        row.0, 1,
        "detail-prune archive and live row should not double-count"
    );
    assert_eq!(
        row.1, 1,
        "detail-prune archive should still contribute latency sample"
    );
    assert_eq!(row.2, 43_890.0);

    cleanup_temp_test_dir(&temp_dir);
}
