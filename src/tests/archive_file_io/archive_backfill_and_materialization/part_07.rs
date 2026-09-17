use super::*;
use serde_json::json;

async fn assert_rebuilt_account_stats(pool: &SqlitePool, account_id: i64) {
    let hourly_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT total_count, success_count, first_response_byte_total_sample_count
         FROM upstream_account_stats_hourly WHERE upstream_account_id = ?1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load rebuilt account stats hourly row");
    let minute_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT total_count, success_count, first_response_byte_total_sample_count
         FROM upstream_account_stats_minute WHERE upstream_account_id = ?1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load rebuilt account stats minute row");
    let live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_one(pool)
            .await
            .expect("load recreated live progress cursor");
    assert_eq!(hourly_counts, (1, 1, 1));
    assert_eq!(minute_counts, (1, 1, 1));
    assert_eq!(live_cursor, 1);
}

async fn rebuild_legacy_invocation_rollup_table(pool: &SqlitePool) {
    sqlx::query("ALTER TABLE invocation_rollup_hourly RENAME TO invocation_rollup_hourly_current")
        .execute(pool)
        .await
        .expect("rename current invocation rollup table");
    sqlx::query(
        "CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL, source TEXT NOT NULL,
            total_count INTEGER NOT NULL, success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL, total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL, first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0, first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy invocation rollup table");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly
         SELECT bucket_start_epoch, source, total_count, success_count, failure_count,
                total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
                first_byte_max_ms, first_byte_histogram, updated_at
         FROM invocation_rollup_hourly_current",
    )
    .execute(pool)
    .await
    .expect("copy legacy invocation rollup rows");
    sqlx::query("DROP TABLE invocation_rollup_hourly_current")
        .execute(pool)
        .await
        .expect("drop current invocation rollup table copy");
}

#[tokio::test]
pub(crate) async fn terminal_proof_reconciliation_repairs_a_contradictory_canonical_bucket() {
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
pub(crate) async fn terminal_proof_is_revoked_when_a_completed_invocation_archive_is_missing() {
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
pub(crate) async fn terminal_proof_is_revoked_when_a_readable_invocation_archive_hash_mismatches() {
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
pub(crate) async fn unreadable_archive_does_not_revoke_retired_terminal_proofs() {
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
pub(crate) async fn complete_reconciliation_removes_stale_missing_canonical_buckets() {
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
pub(crate) async fn schema_upgrade_preserves_legacy_canonical_history_outside_the_new_source_window()
 {
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
pub(crate) async fn schema_upgrade_resumes_boundary_bootstrap_after_rollup_columns_are_present() {
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
pub(crate) async fn reconciliation_keeps_a_pre_retention_live_snapshot_from_deleting_canonical_history()
 {
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
pub(crate) async fn terminal_proof_keeps_retired_buckets_but_removes_missing_boundary_buckets() {
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

async fn create_legacy_account_usage_fixture() -> (SqlitePool, i64, i64) {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");
    sqlx::query("DROP TABLE upstream_account_usage_hourly")
        .execute(&pool)
        .await
        .expect("drop current account usage hourly table");
    sqlx::query(
        "CREATE TABLE upstream_account_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL, upstream_account_id INTEGER NOT NULL,
            request_count INTEGER NOT NULL, total_tokens INTEGER NOT NULL, total_cost REAL NOT NULL,
            input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_input_tokens INTEGER NOT NULL,
            first_seen_at TEXT NOT NULL, last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id)
        )",
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
            "INSERT INTO codex_invocations
             (id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, reasoning_tokens,
              total_tokens, cost, status, error_message, failure_kind, payload, raw_response, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
        "INSERT INTO upstream_account_usage_hourly
         (bucket_start_epoch, upstream_account_id, request_count, total_tokens, total_cost,
          input_tokens, output_tokens, cache_input_tokens, first_seen_at, last_seen_at)
         VALUES (?1, ?2, 2, 60, 0.02, 20, 40, 0, ?3, ?3)",
    )
    .bind(live_bucket)
    .bind(account_id)
    .bind(&live_occurred_at)
    .execute(&pool)
    .await
    .expect("insert legacy live account usage rollup");
    (pool, account_id, live_bucket)
}

async fn seed_legacy_account_usage_archive(pool: &SqlitePool, account_id: i64, live_bucket: i64) {
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
        "INSERT INTO upstream_account_usage_hourly
         (bucket_start_epoch, upstream_account_id, request_count, total_tokens, total_cost,
          input_tokens, output_tokens, cache_input_tokens, first_seen_at, last_seen_at)
         VALUES (?1, ?2, 1, 10, 0.01, 4, 6, 0, ?3, ?4)",
    )
    .bind(archive_bucket)
    .bind(account_id)
    .bind(&archive_start)
    .bind(&archive_end)
    .execute(pool)
    .await
    .expect("insert stale archived account usage rollup");
    sqlx::query(
        "INSERT INTO archive_batches
         (dataset, month_key, file_path, sha256, status, row_count,
          coverage_start_at, coverage_end_at, historical_rollups_materialized_at)
         VALUES ('codex_invocations', '2026-01', '/tmp/account-usage-backfill.sqlite.gz',
                 'account-usage-backfill-sha', 'completed', 1, ?1, ?2, datetime('now'))",
    )
    .bind(&archive_start)
    .bind(&archive_end)
    .execute(pool)
    .await
    .expect("insert completed archive batch");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, replayed_at)
         VALUES ('upstream_account_usage_hourly', 'codex_invocations',
                 '/tmp/account-usage-backfill.sqlite.gz', datetime('now'))",
    )
    .execute(pool)
    .await
    .expect("insert stale replay marker");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id)
         VALUES ('codex_invocations', '/tmp/account-usage-backfill.sqlite.gz', 10)",
    )
    .execute(pool)
    .await
    .expect("insert stale replay progress");
}

async fn assert_legacy_account_usage_backfill(
    pool: &SqlitePool,
    account_id: i64,
    live_bucket: i64,
) {
    ensure_schema(pool)
        .await
        .expect("migrate account usage status counts");
    let live_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT success_count, failure_count, reasoning_tokens
         FROM upstream_account_usage_hourly WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(live_bucket)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load live account status counts");
    let archived_row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upstream_account_usage_hourly
         WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(live_bucket - 7 * 24 * 3600)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load archived stale row count");
    let archive_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches
         WHERE file_path = '/tmp/account-usage-backfill.sqlite.gz'",
    )
    .fetch_one(pool)
    .await
    .expect("load archive materialized marker");
    let stale_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE target = 'upstream_account_usage_hourly'",
    )
    .fetch_one(pool)
    .await
    .expect("load stale replay marker count");
    let stale_progress: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_progress
         WHERE dataset = 'codex_invocations' AND file_path = '/tmp/account-usage-backfill.sqlite.gz'",
    )
    .fetch_one(pool)
    .await
    .expect("load stale replay progress count");
    assert_eq!(live_counts, (1, 1, 7));
    assert_eq!(archived_row_count, 0);
    assert!(archive_materialized_at.is_none());
    assert_eq!(stale_markers, 0);
    assert_eq!(stale_progress, 0);
}

#[tokio::test]
pub(crate) async fn ensure_schema_backfills_account_usage_status_counts_and_reopens_archive_rollups()
 {
    let (pool, account_id, live_bucket) = create_legacy_account_usage_fixture().await;
    seed_legacy_account_usage_archive(&pool, account_id, live_bucket).await;
    assert_legacy_account_usage_backfill(&pool, account_id, live_bucket).await;
}
pub(crate) async fn ensure_schema_rebuilds_account_stats_when_live_progress_table_is_missing() {
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

    assert_rebuilt_account_stats(&pool, account_id).await;
}

#[tokio::test]
pub(crate) async fn ensure_schema_backfill_deduplicates_detail_prune_archives() {
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

    rebuild_legacy_invocation_rollup_table(&pool).await;

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
