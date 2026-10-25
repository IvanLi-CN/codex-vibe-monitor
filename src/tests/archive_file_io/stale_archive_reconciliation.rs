use super::archive_backfill_test_support::write_valid_invocation_archive;
use super::*;
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

    // Older releases could leave a LegacyMonth detail mirror classified as authoritative after
    // reusing the deterministic month path. The bounded recovery pass must re-prove and repair
    // that role before Summary treats it as authority again.
    sqlx::query("DROP TRIGGER trg_update_authoritative_invocation_archive_requires_summary_proof")
        .execute(&pool)
        .await
        .expect("temporarily remove publication guard for legacy fixture");
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = ?1, layout = ?2 \
         WHERE dataset = ?3 AND file_path = ?4",
    )
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE)
    .bind(ARCHIVE_LAYOUT_LEGACY_MONTH)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed an older authoritative LegacyMonth detail mirror");
    let legacy_role_recovery =
        reconcile_legacy_detail_mirrors_startup_window(&pool, 0, Duration::from_secs(6))
            .await
            .expect("reconcile older authoritative LegacyMonth detail mirror");
    let repaired_source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load repaired legacy source role");
    assert_eq!(legacy_role_recovery.changed_path_count, 1);
    assert_eq!(
        repaired_source_kind,
        SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
    );
    ensure_schema(&pool)
        .await
        .expect("restore publication guard after legacy fixture");

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
        source_identity_sha256: None,
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
    write_valid_invocation_archive(&archive_path, "partial-retained-live").await;
    let archive_sha = sha256_hex_file(&archive_path).expect("hash retained-live archive");
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
    .bind(&archive_sha)
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
