use super::*;

fn breakdown_archive_row<'a>(
    id: i64,
    invoke_id: &'a str,
    occurred_at: &'a str,
) -> SeedInvocationArchiveBatchRow<'a> {
    SeedInvocationArchiveBatchRow {
        id,
        invoke_id,
        occurred_at,
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
    }
}

async fn seed_overlap_archive_fixture(pool: &SqlitePool, config: &AppConfig) -> (PathBuf, i64) {
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let first_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let overlap_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid overlapping archived occurred_at"),
    );
    let extended_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(20)))
            .expect("valid extended archived occurred_at"),
    );
    let first_path = seed_invocation_archive_batch_with_details(
        pool,
        config,
        "stale-replay-sha-overlap-first",
        &[breakdown_archive_row(
            1,
            "stale-replay-sha-overlap-first",
            &first_at,
        )],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        pool,
        config,
        "stale-replay-sha-overlap-extended",
        &[
            breakdown_archive_row(1, "stale-replay-sha-overlap-extended-overlap", &overlap_at),
            breakdown_archive_row(2, "stale-replay-sha-overlap-extended-tail", &extended_at),
        ],
    )
    .await;
    let initial = materialize_historical_rollups(pool, config, false)
        .await
        .expect("materialize overlapping archives");
    assert_eq!(initial.materialized_invocation_batches, 2);
    let before: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(pool)
            .await
            .expect("capture original overlapping rollup total");
    assert_eq!(before, 3);
    (first_path, before)
}

async fn seed_stale_archive_without_coverage(
    pool: &SqlitePool,
    config: &AppConfig,
) -> (String, String) {
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 45) as i64, 8, 10, 0);
    let archive_path = seed_invocation_archive_batch_with_details(
        pool,
        config,
        "breakdown-replay-stale-replay-sha",
        &[breakdown_archive_row(
            1,
            "breakdown-replay-stale-replay-sha",
            &occurred_at,
        )],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();
    let original_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(pool)
    .await
    .expect("load original archive SHA");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
         FROM archive_batches AS batches
         WHERE batches.dataset = ?2 AND batches.status = 'completed' AND batches.file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(pool)
    .await
    .expect("seed replay marker for original archive SHA");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-archive-sha'
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(pool)
    .await
    .expect("publish replacement archive manifest SHA");
    let pending = load_invocation_archive_files_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    )
    .await
    .expect("list replacement archive as pending usage breakdown replay");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].file_path, archive_file_path);
    (archive_file_path, original_sha)
}

async fn seed_materialized_invocation_bucket(pool: &SqlitePool, bucket_epoch: i64) {
    sqlx::query(
        "INSERT INTO hourly_rollup_materialized_buckets
         (target, bucket_start_epoch, source, materialized_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(target, bucket_start_epoch, source) DO NOTHING",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(pool)
    .await
    .expect("seed invocation materialized bucket");
}

async fn seed_partial_breakdown_history(
    pool: &SqlitePool,
    config: &AppConfig,
) -> (String, i64, i64) {
    let first_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 10, 0);
    let second_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 10, 20, 0);
    for occurred_at in [&first_local, &second_local] {
        insert_retention_invocation(
            pool,
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
    sync_hourly_rollups_from_live_tables(pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(pool, config, Some(false), None)
        .await
        .expect("archive old rows before partial breakdown repair");
    assert_eq!(retention.invocation_rows_archived, 2);
    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load invocation archive path");
    let first_bucket = local_naive_to_utc(
        parse_shanghai_local_naive(&first_local).expect("valid first local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    let second_bucket = local_naive_to_utc(
        parse_shanghai_local_naive(&second_local).expect("valid second local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    sqlx::query(
        "DELETE FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(second_bucket)
    .execute(pool)
    .await
    .expect("drop second breakdown bucket to mimic partial backfill");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(pool)
    .await
    .expect("invalidate the usage breakdown proof after removing its rollup data");
    seed_materialized_invocation_bucket(pool, first_bucket).await;
    seed_materialized_invocation_bucket(pool, second_bucket).await;
    (archive_path, first_bucket, second_bucket)
}

async fn rewrite_archive_and_publish_sha(
    pool: &SqlitePool,
    temp_dir: &Path,
    archive_path: &Path,
    archive_file_path: &str,
    replacement_name: &str,
) -> String {
    let replacement_db_path = temp_dir.join(replacement_name);
    inflate_gzip_sqlite_file(archive_path, &replacement_db_path)
        .expect("inflate replacement invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, archive_path)
        .expect("compress replacement invocation archive");
    let replacement_sha =
        sha256_hex_file(archive_path).expect("hash replacement invocation archive");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = ?1, historical_rollups_materialized_at = NULL
         WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(&replacement_sha)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_file_path)
    .execute(pool)
    .await
    .expect("publish replacement archive manifest SHA");
    replacement_sha
}

async fn assert_three_account_rollup_total(pool: &SqlitePool, expected: i64) {
    let total: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(pool)
            .await
            .expect("load invocation rollup total");
    assert_eq!(total, expected);
}

async fn assert_unverified_account_markers_are_quarantined(pool: &SqlitePool, paths: &[&str]) {
    assert_eq!(
        repair_materialized_upstream_account_archive_markers(pool)
            .await
            .expect("unverified manifests must remain quarantined"),
        0
    );
    for file_path in paths {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay
             WHERE dataset = ?1 AND file_path = ?2 AND target IN (?3, ?4, ?5)",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
        .fetch_one(pool)
        .await
        .expect("count quarantined account replay markers");
        assert_eq!(count, 0);
    }
}

async fn assert_blank_manifest_marker_fails_closed(pool: &SqlitePool, path: &str) {
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         VALUES (?1, ?2, ?3, '', datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(path)
    .execute(pool)
    .await
    .expect("seed blank legacy replay marker");
    let mut tx = pool.begin().await.expect("begin blank marker read check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            path,
        )
        .await
        .expect("blank manifest marker read must fail closed")
    );
    assert!(
        mark_hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            path,
        )
        .await
        .is_err()
    );
    tx.commit().await.expect("commit blank marker read check");
}

async fn assert_recovered_account_marker(pool: &SqlitePool, path: &str) {
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'recovered-account-manifest-sha'
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(path)
    .execute(pool)
    .await
    .expect("publish recovered invocation manifest SHA");
    assert_eq!(
        repair_materialized_upstream_account_archive_markers(pool)
            .await
            .expect("valid manifest recovery must repair account markers"),
        1
    );
    let repaired_marker_sha: String = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(path)
    .fetch_one(pool)
    .await
    .expect("load repaired account replay marker");
    assert_eq!(repaired_marker_sha, "recovered-account-manifest-sha");
}

struct SparseLegacyDetailMirrorFixture {
    pool: SqlitePool,
    config: AppConfig,
    temp_dir: PathBuf,
    archive_path: PathBuf,
    archive_file_path: String,
    occurred_at: String,
}
async fn create_sparse_legacy_detail_mirror_fixture() -> SparseLegacyDetailMirrorFixture {
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
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "sparse-legacy-detail-mirror",
        &[
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
        ],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();
    SparseLegacyDetailMirrorFixture {
        pool,
        config,
        temp_dir,
        archive_path,
        archive_file_path,
        occurred_at,
    }
}

async fn seed_sparse_legacy_detail_mirror_live_rows(fixture: &SparseLegacyDetailMirrorFixture) {
    for (id, invoke_id) in [
        (41_i64, "sparse-legacy-detail-mirror-41"),
        (43_i64, "sparse-legacy-detail-mirror-43"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost,
                t_upstream_ttfb_ms, payload, detail_level, failure_class, is_actionable,
                raw_response, created_at
            )
            VALUES (?1, ?2, ?3, 'proxy', 'success', 12, 0.12, 120.0, '{}', 'full', 'none', 0, '{}', ?3)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&fixture.occurred_at)
        .execute(&fixture.pool)
        .await
        .expect("retain canonical live mirror record");
    }
}

async fn mark_sparse_legacy_detail_mirror_manifest(fixture: &SparseLegacyDetailMirrorFixture) {
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
    .bind(&fixture.occurred_at)
    .bind(&fixture.occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("record sparse legacy segment metadata");
}

async fn assert_sparse_legacy_detail_mirror_initial_recovery(
    fixture: &SparseLegacyDetailMirrorFixture,
) {
    let mirror_recovery =
        reconcile_legacy_detail_mirrors_startup_window(&fixture.pool, 0, Duration::from_secs(6))
            .await
            .expect("classify sparse legacy detail mirror before rollup replay");
    let recovered =
        materialize_historical_rollups_startup_window(&fixture.pool, 0, Duration::from_secs(6))
            .await
            .expect("recover sparse legacy detail mirror without replaying it");
    let source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load reconciled source role");

    assert_eq!(mirror_recovery.changed_path_count, 1);
    assert_eq!(source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR);
    assert_eq!(recovered.summary.materialized_invocation_batches, 0);
}

async fn seed_sparse_independent_mirror(
    fixture: &SparseLegacyDetailMirrorFixture,
) -> (PathBuf, String) {
    let recovered_archive_path = fixture
        .temp_dir
        .join("sparse-legacy-detail-mirror-recovered.sqlite.gz");
    fs::copy(&fixture.archive_path, &recovered_archive_path)
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
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("seed later independently provable legacy mirror manifest");
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("restore the first source role for startup recovery");
    fs::remove_file(&fixture.archive_path).expect("make the first legacy source unavailable");
    (recovered_archive_path, recovered_archive_file_path)
}

async fn assert_sparse_independent_mirror_recovery(
    fixture: &SparseLegacyDetailMirrorFixture,
    recovered_archive_file_path: &str,
) {
    let high_watermark = summary_startup_legacy_detail_mirror_high_watermark(&fixture.pool)
        .await
        .expect("capture a stable startup mirror high-watermark")
        .expect("find legacy mirror candidates");
    let summary_startup_recovery = reconcile_legacy_detail_mirrors_for_summary_startup_window(
        &fixture.pool,
        0,
        high_watermark,
        Duration::from_secs(6),
    )
    .await
    .expect("a missing legacy source must not block later identity proofs");
    let source_kinds = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, summary_source_kind FROM archive_batches          WHERE dataset = ?1 AND file_path IN (?2, ?3) ORDER BY file_path ASC",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .bind(recovered_archive_file_path)
    .fetch_all(&fixture.pool)
    .await
    .expect("load summary startup recovery source roles");

    assert!(summary_startup_recovery.completed);
    assert_eq!(summary_startup_recovery.unavailable_path_count, 1);
    assert_eq!(summary_startup_recovery.changed_path_count, 1);
    assert_eq!(
        source_kinds,
        vec![
            (
                fixture.archive_file_path.clone(),
                SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN.to_string(),
            ),
            (
                recovered_archive_file_path.to_string(),
                SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR.to_string(),
            ),
        ],
        "the unreadable source stays fail-closed while the independent mirror leaves Summary admission"
    );
}

async fn assert_sparse_legacy_detail_mirror_rejection(
    fixture: &SparseLegacyDetailMirrorFixture,
    recovered_archive_path: &Path,
    recovered_archive_file_path: &str,
) {
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("exclude the unavailable source from the generic mismatch probe");
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1 WHERE dataset = ?2 AND file_path = ?3",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(recovered_archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("restore the independently proved source role for mismatch probing");
    let budget_exhausted =
        reconcile_legacy_detail_mirrors_startup_window(&fixture.pool, 0, Duration::ZERO)
            .await
            .expect("defer an unstarted mirror proof at its elapsed budget");
    assert!(budget_exhausted.hit_budget);
    assert_eq!(budget_exhausted.inspected_path_count, 0);
    sqlx::query("UPDATE codex_invocations SET invoke_id = ?1 WHERE id = ?2")
        .bind("sparse-legacy-detail-mirror-mismatch")
        .bind(43_i64)
        .execute(&fixture.pool)
        .await
        .expect("mismatch one retained live identity");
    let rejected =
        reconcile_legacy_detail_mirrors_startup_window(&fixture.pool, 0, Duration::from_secs(6))
            .await
            .expect("reject non-mirror archive identity");
    let rejected_source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(recovered_archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load rejected source role");
    assert_eq!(rejected.changed_path_count, 0);
    assert_eq!(rejected_source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);

    fs::remove_file(recovered_archive_path).expect("make archived source unavailable");
    let unreadable =
        reconcile_legacy_detail_mirrors_startup_window(&fixture.pool, 0, Duration::from_secs(6))
            .await
            .expect_err("surface an unreadable archive to the startup task");
    assert!(
        unreadable
            .to_string()
            .contains("failed to open legacy detail archive for sha256")
    );
    let unreadable_source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(recovered_archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load unreadable source role");
    assert_eq!(unreadable_source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);
}

#[tokio::test]
pub(crate) async fn startup_recovery_classifies_sparse_legacy_detail_mirror_by_archive_identity() {
    let fixture = create_sparse_legacy_detail_mirror_fixture().await;
    seed_sparse_legacy_detail_mirror_live_rows(&fixture).await;
    mark_sparse_legacy_detail_mirror_manifest(&fixture).await;
    assert_sparse_legacy_detail_mirror_initial_recovery(&fixture).await;
    let (recovered_archive_path, recovered_archive_file_path) =
        seed_sparse_independent_mirror(&fixture).await;
    assert_sparse_independent_mirror_recovery(&fixture, &recovered_archive_file_path).await;
    assert_sparse_legacy_detail_mirror_rejection(
        &fixture,
        &recovered_archive_path,
        &recovered_archive_file_path,
    )
    .await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}

#[tokio::test]
pub(crate) async fn partial_materialization_rebuilds_stale_sha_before_any_additive_replay() {
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

    let _replacement_sha = rewrite_archive_and_publish_sha(
        &pool,
        &temp_dir,
        &archive_path,
        &archive_file_path,
        "partial-materialization-stale-sha.sqlite",
    )
    .await;
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
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)",
    )
    .fetch_one(&pool)
    .await
    .expect("load rollup totals after stale partial rebuild");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn stale_archive_rebuild_clears_full_coverage_of_reopened_overlaps() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("stale-replay-sha-overlap-coverage").await;
    let (first_archive_path, before) = seed_overlap_archive_fixture(&pool, &config).await;

    let first_archive_path_string = first_archive_path.to_string_lossy().to_string();
    let _replacement_sha = rewrite_archive_and_publish_sha(
        &pool,
        &temp_dir,
        &first_archive_path,
        &first_archive_path_string,
        "stale-replay-sha-overlap-first.sqlite",
    )
    .await;

    let first_replay = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("reopen stale archive and its overlapping peer");
    assert_eq!(first_replay.materialized_invocation_batches, 1);
    let second_replay = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("replay the reopened extended overlap archive");
    assert_eq!(second_replay.materialized_invocation_batches, 1);
    assert_three_account_rollup_total(&pool, before).await;

    cleanup_temp_test_dir(&temp_dir);
}

struct ForwardProxyReplayFixture {
    pool: SqlitePool,
    config: AppConfig,
    temp_dir: PathBuf,
    archive_path: PathBuf,
    archive_db_path: PathBuf,
    archive_file_path: String,
    first_occurred_at: String,
    second_occurred_at: String,
    replacement_occurred_at: String,
    initial_sha: String,
}

async fn seed_forward_proxy_replay_archive(
    _pool: &SqlitePool,
    archive_path: &Path,
    archive_db_path: &Path,
    first_occurred_at: &str,
    second_occurred_at: &str,
) -> String {
    fs::File::create(archive_db_path).expect("create forward proxy archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(archive_db_path))
        .await
        .expect("open forward proxy archive sqlite");
    let create_sql = FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create forward proxy archive schema");
    for (id, occurred_at, is_success, latency_ms) in [
        (1_i64, first_occurred_at, 1_i64, Some(120.0_f64)),
        (2_i64, second_occurred_at, 0_i64, None),
    ] {
        sqlx::query(
            "INSERT INTO forward_proxy_attempts
             (id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe)
             VALUES (?1, 'proxy-stale-replay', ?2, ?3, ?4, NULL, 0)",
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
    deflate_sqlite_file_to_gzip(archive_db_path, archive_path)
        .expect("compress initial forward proxy archive");
    sha256_hex_file(archive_path).expect("hash initial forward proxy archive")
}

async fn create_forward_proxy_replay_fixture() -> ForwardProxyReplayFixture {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("forward-proxy-stale-replay-sha").await;
    let first = Utc::now()
        .checked_sub_signed(ChronoDuration::days(120))
        .expect("set first archive time")
        .with_minute(10)
        .expect("set first minute")
        .with_second(0)
        .expect("set first second");
    let first_occurred_at = first.format("%Y-%m-%d %H:%M:%S").to_string();
    let second_occurred_at = (first + ChronoDuration::hours(1))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let replacement_occurred_at = (first + ChronoDuration::hours(2))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let archive_path = temp_dir.join("forward-proxy-stale-replay.sqlite.gz");
    let archive_db_path = temp_dir.join("forward-proxy-stale-replay.sqlite");
    let initial_sha = seed_forward_proxy_replay_archive(
        &pool,
        &archive_path,
        &archive_db_path,
        &first_occurred_at,
        &second_occurred_at,
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();
    sqlx::query(
        "INSERT INTO archive_batches
         (dataset, month_key, file_path, sha256, row_count, status,
          coverage_start_at, coverage_end_at, created_at)
         VALUES (?1, ?2, ?3, ?4, 2, ?5, ?6, ?7, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&first_occurred_at[..7])
    .bind(&archive_file_path)
    .bind(&initial_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&first_occurred_at)
    .bind(&second_occurred_at)
    .execute(&pool)
    .await
    .expect("insert initial forward proxy archive manifest");
    ForwardProxyReplayFixture {
        pool,
        config,
        temp_dir,
        archive_path,
        archive_db_path,
        archive_file_path,
        first_occurred_at,
        second_occurred_at,
        replacement_occurred_at,
        initial_sha,
    }
}

async fn assert_initial_forward_proxy_replay(
    fixture: &ForwardProxyReplayFixture,
) -> (i64, i64, i64) {
    let initial = materialize_historical_rollups(&fixture.pool, &fixture.config, false)
        .await
        .expect("materialize initial forward proxy archive");
    assert_eq!(initial.materialized_forward_proxy_batches, 1);
    let totals: (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(attempts), 0), COALESCE(SUM(success_count), 0),
                COALESCE(SUM(failure_count), 0)
         FROM forward_proxy_attempt_hourly WHERE proxy_key = 'proxy-stale-replay'",
    )
    .fetch_one(&fixture.pool)
    .await
    .expect("load initial forward proxy rollups");
    assert_eq!(totals, (2, 1, 1));
    totals
}

async fn assert_markerless_forward_proxy_replay(
    fixture: &ForwardProxyReplayFixture,
    before: (i64, i64, i64),
) {
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("remove forward proxy replay proof from materialized archive");
    let markerless = materialize_historical_rollups(&fixture.pool, &fixture.config, false)
        .await
        .expect("leave markerless materialized archive unchanged");
    assert_eq!(markerless.materialized_forward_proxy_batches, 0);
    let totals: (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(attempts), 0), COALESCE(SUM(success_count), 0),
                COALESCE(SUM(failure_count), 0)
         FROM forward_proxy_attempt_hourly WHERE proxy_key = 'proxy-stale-replay'",
    )
    .fetch_one(&fixture.pool)
    .await
    .expect("load markerless forward proxy rollups");
    assert_eq!(totals, before);
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&fixture.archive_file_path)
    .bind(&fixture.initial_sha)
    .execute(&fixture.pool)
    .await
    .expect("restore original forward proxy replay proof");
}

async fn replace_forward_proxy_replay_archive(fixture: &ForwardProxyReplayFixture) -> String {
    inflate_gzip_sqlite_file(&fixture.archive_path, &fixture.archive_db_path)
        .expect("inflate replacement forward proxy archive");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&fixture.archive_db_path))
        .await
        .expect("open replacement forward proxy archive sqlite");
    sqlx::query(
        "INSERT INTO forward_proxy_attempts
         (id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe)
         VALUES (3, 'proxy-stale-replay', ?1, 1, 80.0, NULL, 0)",
    )
    .bind(&fixture.replacement_occurred_at)
    .execute(&archive_pool)
    .await
    .expect("append replacement forward proxy archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&fixture.archive_db_path, &fixture.archive_path)
        .expect("compress replacement forward proxy archive");
    let replacement_sha =
        sha256_hex_file(&fixture.archive_path).expect("hash replacement forward proxy archive");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = ?1, row_count = 3, coverage_end_at = ?2
         WHERE dataset = ?3 AND file_path = ?4",
    )
    .bind(&replacement_sha)
    .bind(&fixture.replacement_occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("publish replacement forward proxy manifest SHA");
    replacement_sha
}

async fn assert_rebuilt_forward_proxy_replay(
    fixture: &ForwardProxyReplayFixture,
    replacement_sha: &str,
) {
    let rebuilt = materialize_historical_rollups(&fixture.pool, &fixture.config, false)
        .await
        .expect("rebuild stale forward proxy archive");
    assert_eq!(rebuilt.materialized_forward_proxy_batches, 1);
    let totals: (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(attempts), 0), COALESCE(SUM(success_count), 0),
                COALESCE(SUM(failure_count), 0)
         FROM forward_proxy_attempt_hourly WHERE proxy_key = 'proxy-stale-replay'",
    )
    .fetch_one(&fixture.pool)
    .await
    .expect("load rebuilt forward proxy rollups");
    assert_eq!(totals, (3, 2, 1));
    let replay_sha: String = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&fixture.archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load replacement forward proxy replay marker");
    assert_eq!(replay_sha, replacement_sha);
}

#[tokio::test]
pub(crate) async fn forward_proxy_stale_replay_sha_rebuilds_same_path_without_double_counting() {
    let fixture = create_forward_proxy_replay_fixture().await;
    let before = assert_initial_forward_proxy_replay(&fixture).await;
    assert_markerless_forward_proxy_replay(&fixture, before).await;
    let replacement_sha = replace_forward_proxy_replay_archive(&fixture).await;
    assert_rebuilt_forward_proxy_replay(&fixture, &replacement_sha).await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}
pub(crate) async fn startup_account_marker_repair_requires_a_nonblank_completed_manifest_sha() {
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

    assert_unverified_account_markers_are_quarantined(
        &pool,
        &[null_manifest_path, blank_manifest_path],
    )
    .await;
    assert_blank_manifest_marker_fails_closed(&pool, blank_manifest_path).await;
    assert_recovered_account_marker(&pool, null_manifest_path).await;
}

struct SamePathArchiveFixture {
    pool: SqlitePool,
    config: AppConfig,
    temp_dir: PathBuf,
    archive_path: PathBuf,
    archive_file_path: String,
    first_occurred_at: String,
    second_occurred_at: String,
    appended_occurred_at: String,
}

async fn create_same_path_archive_fixture() -> SamePathArchiveFixture {
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
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#),
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
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;
    SamePathArchiveFixture {
        pool,
        config,
        temp_dir,
        archive_file_path: archive_path.to_string_lossy().to_string(),
        archive_path,
        first_occurred_at,
        second_occurred_at,
        appended_occurred_at,
    }
}

async fn record_same_path_initial_materialization(fixture: &SamePathArchiveFixture) {
    sqlx::query(
        "UPDATE archive_batches SET coverage_start_at = ?1, coverage_end_at = ?2
         WHERE dataset = ?3 AND file_path = ?4",
    )
    .bind(&fixture.first_occurred_at)
    .bind(&fixture.second_occurred_at)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("record initial archive coverage");
    let initial = materialize_historical_rollups(&fixture.pool, &fixture.config, false)
        .await
        .expect("materialize initial same-path archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
}

async fn append_same_path_archive(fixture: &SamePathArchiveFixture) -> String {
    let replacement_db_path = fixture
        .temp_dir
        .join("same-path-invocation-append-replacement.sqlite");
    inflate_gzip_sqlite_file(&fixture.archive_path, &replacement_db_path)
        .expect("inflate append archive source");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open append archive sqlite");
    sqlx::query(
        "INSERT INTO codex_invocations
         (id, invoke_id, occurred_at, source, status, total_tokens, cost,
          t_upstream_ttfb_ms, payload, detail_level, failure_class, is_actionable, raw_response, created_at)
         VALUES (3, 'same-path-invocation-append-third', ?1, ?2, 'success', 12, 0.12,
                 120.0, ?3, ?4, 'none', 0, '{}', ?1)",
    )
    .bind(&fixture.appended_occurred_at)
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"same-path-append","stickyKey":"same-path-append"}"#)
    .bind(DETAIL_LEVEL_FULL)
    .execute(&replacement_pool)
    .await
    .expect("append invocation archive row");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &fixture.archive_path)
        .expect("compress appended invocation archive");
    let replacement_sha =
        sha256_hex_file(&fixture.archive_path).expect("hash appended invocation archive");
    let replacement_manifest = ArchiveBatchOutcome {
        dataset: HOURLY_ROLLUP_DATASET_INVOCATIONS,
        month_key: fixture.first_occurred_at[..7].to_string(),
        day_key: None,
        part_key: None,
        file_path: fixture.archive_file_path.clone(),
        sha256: replacement_sha.clone(),
        row_count: 3,
        upstream_last_activity: Vec::new(),
        coverage_start_at: Some(fixture.appended_occurred_at.clone()),
        coverage_end_at: Some(fixture.appended_occurred_at.clone()),
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    };
    let mut tx = fixture
        .pool
        .begin()
        .await
        .expect("begin append manifest transaction");
    upsert_archive_batch_manifest(tx.as_mut(), &replacement_manifest)
        .await
        .expect("upsert appended same-path manifest");
    tx.commit()
        .await
        .expect("commit appended same-path manifest");
    replacement_sha
}

async fn assert_same_path_archive_rebuild(fixture: &SamePathArchiveFixture, replacement_sha: &str) {
    let coverage: (String, String) = sqlx::query_as(
        "SELECT coverage_start_at, coverage_end_at FROM archive_batches
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load appended archive coverage");
    assert_eq!(
        coverage,
        (
            fixture.first_occurred_at.clone(),
            fixture.appended_occurred_at.clone()
        )
    );
    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&fixture.pool)
            .await
            .expect("reopen stale same-path archive over full preserved coverage"),
        1
    );
    let rebuilt = materialize_historical_rollups(&fixture.pool, &fixture.config, false)
        .await
        .expect("rebuild appended same-path archive");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        "SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)",
    )
    .fetch_one(&fixture.pool)
    .await
    .expect("load rebuilt same-path rollup totals");
    assert_eq!(after, (3, 3, 3, 3, 3, 3, 3, 3, 3, 3));
    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = ?1 AND file_path = ?2 AND archive_sha256 = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_file_path)
    .bind(replacement_sha)
    .fetch_one(&fixture.pool)
    .await
    .expect("count appended archive replacement markers");
    assert_eq!(marker_count, INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64);
}

#[tokio::test]
pub(crate) async fn same_path_invocation_archive_append_preserves_coverage_for_stale_rebuild() {
    let fixture = create_same_path_archive_fixture().await;
    record_same_path_initial_materialization(&fixture).await;
    let replacement_sha = append_same_path_archive(&fixture).await;
    assert_same_path_archive_rebuild(&fixture, &replacement_sha).await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_replay_quarantines_a_stale_archive_without_coverage_bounds() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-replay-stale-replay-sha").await;
    let (archive_file_path, original_sha) =
        seed_stale_archive_without_coverage(&pool, &config).await;

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

struct ReplayMarkerFixture {
    pool: SqlitePool,
    temp_dir: PathBuf,
    archive_path: String,
}

async fn create_replay_marker_fixture() -> ReplayMarkerFixture {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-replay-manifest-identity").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("breakdown-replay-manifest-identity.sqlite.gz")
        .to_string_lossy()
        .to_string();
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         VALUES (?1, ?2, ?3, NULL, datetime('now'))",
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
    let sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load missing-manifest legacy marker");
    assert!(sha.is_none());
    assert_replay_marker_rejected(&pool, &archive_path, "check marker without manifest").await;
    ReplayMarkerFixture {
        pool,
        temp_dir,
        archive_path,
    }
}

async fn assert_replay_marker_rejected(pool: &SqlitePool, path: &str, message: &str) {
    let mut tx = pool.begin().await.expect("begin replay marker check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            path,
        )
        .await
        .expect(message),
        "an unverified replay marker must fail closed"
    );
    tx.commit().await.expect("commit replay marker check");
}

async fn seed_completed_replay_manifest(fixture: &ReplayMarkerFixture) {
    sqlx::query(
        "INSERT INTO archive_batches
         (dataset, month_key, file_path, sha256, row_count, status, created_at)
         VALUES (?1, '2026-01', ?2, 'archive-sha-a', 1, ?3, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&fixture.pool)
    .await
    .expect("seed matching completed archive manifest");
    crate::schema::ensure_schema(&fixture.pool)
        .await
        .expect("reapply schema without upgrading a legacy marker");
    let sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load legacy marker after completed manifest");
    assert!(sha.is_none());
    assert_replay_marker_rejected(
        &fixture.pool,
        &fixture.archive_path,
        "check legacy marker with a completed manifest",
    )
    .await;
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = 'archive-sha-a'
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .execute(&fixture.pool)
    .await
    .expect("record a verified replay marker");
}

async fn assert_completed_replay_manifest(fixture: &ReplayMarkerFixture) {
    let mut tx = fixture
        .pool
        .begin()
        .await
        .expect("begin matching-manifest check");
    assert!(
        hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &fixture.archive_path,
        )
        .await
        .expect("check matching marker")
    );
    tx.commit().await.expect("commit matching-manifest check");
    sqlx::query(
        "UPDATE archive_batches SET status = 'writing'
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .execute(&fixture.pool)
    .await
    .expect("make manifest incomplete");
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .execute(&fixture.pool)
    .await
    .expect("restore legacy marker before incomplete-manifest check");
    crate::schema::ensure_schema(&fixture.pool)
        .await
        .expect("upgrade legacy marker with an incomplete manifest");
    let sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&fixture.archive_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load incomplete-manifest legacy marker");
    assert!(sha.is_none());
    assert_replay_marker_rejected(
        &fixture.pool,
        &fixture.archive_path,
        "check marker against incomplete manifest",
    )
    .await;
}

#[tokio::test]
pub(crate) async fn usage_breakdown_replay_marker_requires_a_matching_completed_manifest() {
    let fixture = create_replay_marker_fixture().await;
    seed_completed_replay_manifest(&fixture).await;
    assert_completed_replay_manifest(&fixture).await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}
pub(crate) async fn bootstrap_hourly_rollups_reopens_partially_populated_usage_breakdown_history() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-reopens-partial-breakdown-history").await;
    let (invocation_archive_path, _first_bucket_epoch, _second_bucket_epoch) =
        seed_partial_breakdown_history(&pool, &config).await;

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
