#[tokio::test]
async fn archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("archive-manifest-refresh-dedupe").await;
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
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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

    let refresh = refresh_archive_upstream_activity_manifest(&pool, false)
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

    let mut cli = CliArgs::default();
    cli.command = Some(CliCommand::Maintenance(MaintenanceCliArgs {
        command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: true }),
    }));

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

#[test]
fn startup_rollup_bootstrap_stays_blocking_for_normal_server_start() {
    let default_cli = CliArgs::default();
    assert!(should_run_blocking_startup_hourly_rollup_bootstrap(
        &default_cli
    ));
    assert!(!should_run_blocking_startup_persistent_prep(&default_cli));

    let retention_cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };
    assert!(!should_run_blocking_startup_hourly_rollup_bootstrap(
        &retention_cli
    ));
    assert!(should_run_blocking_startup_persistent_prep(&retention_cli));

    let maintenance_cli = CliArgs {
        command: Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
        })),
        ..CliArgs::default()
    };
    assert!(!should_run_blocking_startup_hourly_rollup_bootstrap(
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
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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
    let (pool, _config, temp_dir) = retention_test_pool_and_config("archive-backfill-budget").await;
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
    let (pool, mut config, temp_dir) = retention_test_pool_and_config("archive-ttl-cleanup").await;
    config.invocation_archive_ttl_days = 0;

    let archive_path = temp_dir.join("expired-archive.sqlite.gz");
    write_gzip_test_file(&archive_path, b"expired");
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
    .bind("expired-sha")
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

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn backfill_invocation_archive_expiries_uses_coverage_end_at() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("archive-expiry-backfill").await;
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
async fn retention_archives_forward_proxy_attempts_and_stats_snapshots() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-timestamped").await;
    let old_attempt = Utc::now() - ChronoDuration::days(35);
    let recent_attempt = Utc::now() - ChronoDuration::days(1);
    seed_forward_proxy_attempt_at(&pool, "proxy-old", old_attempt, true).await;
    seed_forward_proxy_attempt_at(&pool, "proxy-new", recent_attempt, true).await;

    let old_captured_at = utc_naive_from_shanghai_local_days_ago(35, 8, 0, 0);
    let recent_captured_at = utc_naive_from_shanghai_local_days_ago(1, 8, 0, 0);
    insert_stats_source_snapshot_row(&pool, &old_captured_at, &old_captured_at[..10]).await;
    insert_stats_source_snapshot_row(&pool, &recent_captured_at, &recent_captured_at[..10]).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run timestamped retention");
    assert_eq!(summary.forward_proxy_attempt_rows_archived, 1);
    assert_eq!(summary.stats_source_snapshot_rows_archived, 1);

    let remaining_old_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM forward_proxy_attempts WHERE occurred_at < ?1")
            .bind(shanghai_utc_cutoff_string(
                config.forward_proxy_attempts_retention_days,
            ))
            .fetch_one(&pool)
            .await
            .expect("count old forward proxy attempts");
    assert_eq!(remaining_old_attempts, 0);

    let remaining_old_snapshots: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stats_source_snapshots WHERE captured_at < ?1")
            .bind(shanghai_utc_cutoff_string(
                config.stats_source_snapshots_retention_days,
            ))
            .fetch_one(&pool)
            .await
            .expect("count old stats snapshots");
    assert_eq!(remaining_old_snapshots, 0);

    let datasets: HashSet<String> = sqlx::query_scalar(
        r#"
        SELECT dataset
        FROM archive_batches
        WHERE dataset IN ('forward_proxy_attempts', 'stats_source_snapshots')
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load timestamped archive batch datasets")
    .into_iter()
    .collect();
    assert!(datasets.contains("forward_proxy_attempts"));
    assert!(datasets.contains("stats_source_snapshots"));

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
async fn materialize_historical_rollups_keeps_pruned_detail_archives_pending() {
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

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after partial materialization");
    assert_eq!(total_count, 1);

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
            INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(first_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert replay marker for non-keyed invocation target");
    }

    let first_summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
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

    let summary = materialize_historical_rollups_bounded(
        &pool,
        &config,
        false,
        None,
        Some(Duration::ZERO),
    )
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

    let live_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
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
async fn materialize_historical_rollups_marks_replayed_batches_as_materialized() {
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
            INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
            VALUES (?1, ?2, ?3, datetime('now'))
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
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
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
    assert_eq!(materialize.materialized_invocation_batches, 0);
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
async fn historical_rollup_backfill_stays_critical_until_legacy_invocations_materialized() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-backfill-critical").await;
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
        .expect("prune should remove stale missing archive metadata");
    assert_eq!(prune_summary.deleted_archive_batches, 1);

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches after pruning missing metadata");
    assert_eq!(remaining_batches, 0);

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
            COALESCE(SUM(total_tokens), 0) AS total_tokens
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
    fs::write(&segment_path, b"expired segment").expect("write segment archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, day_key, part_key, file_path, sha256, row_count, status, layout, codec, writer_version, cleanup_state, coverage_start_at, coverage_end_at, archive_expires_at, historical_rollups_materialized_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'), datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind("2025-01-02")
    .bind("part-000001")
    .bind(segment_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind("2025-01-02 00:00:00")
    .bind("2025-01-02 00:00:00")
    .bind("2000-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert expired segment manifest");

    let legacy_path = archive_batch_file_path(&config, "codex_invocations", "2024-12")
        .expect("resolve legacy batch path");
    fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("create legacy parent");
    fs::write(&legacy_path, b"legacy archive").expect("write legacy archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, layout, codec, writer_version, cleanup_state, coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'), datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2024-12")
    .bind(legacy_path.to_string_lossy().to_string())
    .bind("feedface")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_LAYOUT_LEGACY_MONTH)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind("2024-12-01 00:00:00")
    .bind("2024-12-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert legacy archive manifest");

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
async fn bootstrap_hourly_rollups_keeps_retention_materialized_totals_unchanged() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("hourly-rollup-retention-accounted").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "hourly-rollup-retention-accounted",
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
    let old_attempt = Utc::now()
        - ChronoDuration::days((config.forward_proxy_attempts_retention_days + 2) as i64);
    seed_forward_proxy_attempt_at(&pool, "proxy-retention-accounted", old_attempt, true).await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
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

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention before bootstrap replay");
    assert_eq!(summary.invocation_rows_archived, 1);
    assert_eq!(summary.forward_proxy_attempt_rows_archived, 1);

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
    assert_eq!(invocation_materialized_batches, 1);

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

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_ignores_missing_replay_markers() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("hourly-rollup-missing-invocation-target").await;
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
async fn ensure_schema_backfills_first_response_byte_totals_for_legacy_invocation_rollups() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("legacy-rollup-first-response-byte-total-backfill").await;
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

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_backfill_deduplicates_detail_prune_archives() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("legacy-rollup-detail-prune-dedup").await;
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
