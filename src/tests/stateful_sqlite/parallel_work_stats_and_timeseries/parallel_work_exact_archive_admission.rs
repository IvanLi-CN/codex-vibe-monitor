use super::*;

#[tokio::test]
async fn summary_projection_hydrates_rolling_windows_beyond_archive_manifest_admission() {
    // Exercise the paged-overflow proof with the smallest manifest set that exceeds admission.
    const MANIFEST_COUNT: i64 = 4_097;

    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let projection_fixture_started_at = std::time::Instant::now();
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(1_000)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align archive fixture to a full hour");
    let bucket = crate::stats::align_bucket_epoch(archive_start.timestamp(), 3_600, 0);

    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', ?2, ?2, 0, ?3, ?4, 0)",
    )
    .bind(bucket)
    .bind(MANIFEST_COUNT)
    .bind(MANIFEST_COUNT * 17)
    .bind(MANIFEST_COUNT as f64 * 1.25)
    .execute(&state.pool)
    .await
    .expect("insert durable summary rollup");
    sqlx::query(
        "INSERT INTO upstream_account_stats_hourly \
         (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 42, ?2, ?2, 0, ?3, ?4, 0)",
    )
    .bind(bucket)
    .bind(MANIFEST_COUNT)
    .bind(MANIFEST_COUNT * 17)
    .bind(MANIFEST_COUNT as f64 * 1.25)
    .execute(&state.pool)
    .await
    .expect("insert durable account summary rollup");
    sqlx::query(
        "WITH RECURSIVE manifests(ordinal) AS ( \
            SELECT 1 UNION ALL SELECT ordinal + 1 FROM manifests WHERE ordinal < ?3 \
         ) \
         INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         SELECT 'codex_invocations', '2026-01', \
                '/tmp/summary-manifest-admission-' || ordinal || '.sqlite.gz', \
                'summary-manifest-admission-' || ordinal, 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now') \
         FROM manifests",
    )
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(MANIFEST_COUNT)
    .execute(&state.pool)
    .await
    .expect("insert archive manifests beyond the exact-record admission");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             SELECT ?1, dataset, file_path, sha256 FROM archive_batches \
             WHERE file_path GLOB '/tmp/summary-manifest-admission-*.sqlite.gz'",
        )
        .bind(target)
        .execute(&state.pool)
        .await
        .expect("record complete rollup replay coverage");
    }
    // These historical manifests model identities already quarantined by a prior bounded
    // recovery pass. Keep the fixture focused on the admission/checkpoint path instead of
    // issuing thousands of individual terminal outcome writes while the stateful shards run.
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_outcome \
         (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at) \
         SELECT id, sha256, 'unavailable', 'verification_failed', datetime('now', '+365 days') \
         FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND file_path GLOB '/tmp/summary-manifest-admission-*.sqlite.gz'",
    )
    .execute(&state.pool)
    .await
    .expect("seed terminal outcomes for quarantined historical manifests");
    let live_id = sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-manifest-admission-live', datetime('now'), 'proxy', 'success', 23, 2.5, \
                 '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert exact live-tail row")
    .last_insert_rowid();
    for dataset in [
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        "invocation_account_activity_v2_repair_live_cursor",
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
             VALUES (?1, ?2, datetime('now')) \
             ON CONFLICT(dataset) DO UPDATE SET \
                 cursor_id = excluded.cursor_id, updated_at = excluded.updated_at",
        )
        .bind(dataset)
        .bind(live_id)
        .execute(&state.pool)
        .await
        .expect("mark the compact live cursor as caught up");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate rolling summary projection beyond manifest admission");
    assert!(
        projection_fixture_started_at.elapsed() < std::time::Duration::from_secs(30),
        "summary projection Bootstrap exceeded its startup readiness budget"
    );

    let Json(current) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("bootstrap must publish the exact current prefix before all-time reconciliation");
    assert_eq!(current.total_count, 1);

    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    SummaryCoverageRecoverySupervisor::run(state.as_ref())
        .await
        .expect("advance the first all-time staged checkpoint page");
    let first_checkpoint = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT COALESCE(manifest_high_watermark_id, 0), global_manifest_next_id, global_rollup_next_rowid \
         FROM summary_all_time_projection_checkpoint WHERE scope = 'all'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("persist first staged all-time checkpoint");
    assert!(
        first_checkpoint.1 > 0,
        "first manifest page commits its seek cursor"
    );
    assert!(
        first_checkpoint.2 > 0,
        "first global rollup page commits its seek cursor"
    );

    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         VALUES ('codex_invocations', '2026-01', '/tmp/summary-manifest-admission-generation.sqlite.gz', \
                 'summary-manifest-admission-generation', 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now'))",
    )
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("append a later all-time manifest generation");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', '/tmp/summary-manifest-admission-generation.sqlite.gz', \
                     'summary-manifest-admission-generation')",
        )
        .bind(target)
        .execute(&state.pool)
        .await
        .expect("record replay proof for the next manifest generation");
    }
    sqlx::query(
        "UPDATE invocation_rollup_hourly SET total_count = total_count + 1, success_count = success_count + 1, \
         total_tokens = total_tokens + 17, total_cost = total_cost + 1.25 \
         WHERE bucket_start_epoch = ?1 AND source = 'proxy'",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("extend the global compact rollup for the next generation");
    sqlx::query(
        "UPDATE upstream_account_stats_hourly SET total_count = total_count + 1, success_count = success_count + 1, \
         total_tokens = total_tokens + 17, total_cost = total_cost + 1.25 \
         WHERE bucket_start_epoch = ?1 AND source = 'proxy' AND upstream_account_id = 42",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("extend the account compact rollup for the next generation");
    // The first staged recovery pass can exceed the 15-second all-time interest freshness
    // window under the full stateful profile. Re-register the owner before asking the
    // maintenance path to observe the later manifest generation.
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    SummaryCoverageRecoverySupervisor::run(state.as_ref())
        .await
        .expect("restart staged checkpoint at the later manifest generation");
    let reset_checkpoint = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT COALESCE(manifest_high_watermark_id, 0), global_manifest_next_id, global_rollup_next_rowid \
         FROM summary_all_time_projection_checkpoint WHERE scope = 'all'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load reset staged all-time checkpoint");
    assert!(reset_checkpoint.0 > first_checkpoint.0);
    assert!(
        reset_checkpoint.1 <= first_checkpoint.1 && reset_checkpoint.2 <= first_checkpoint.2,
        "a newer generation clears stale manifest and rollup cursors before resuming"
    );
    let mut completed = false;
    // Staged proof pages contain only a bounded set of manifests, so convergence requires one
    // refresh per page rather than a single monolithic pass over the full archive set.
    for _ in 0..200 {
        state
            .subscription_hub
            .note_summary_http_interest(true)
            .await;
        SummaryCoverageRecoverySupervisor::run(state.as_ref())
            .await
            .expect("resume bounded staged all-time recovery");
        let checkpoint = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            "SELECT global_manifest_complete, account_manifest_complete, global_rollup_complete, account_rollup_complete \
             FROM summary_all_time_projection_checkpoint WHERE scope = 'all'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load staged all-time completion state");
        completed = checkpoint == (1, 1, 1, 1);
        if completed {
            break;
        }
    }
    assert!(
        completed,
        "staged all-time recovery must converge from committed manifest and rollup cursors"
    );
    let Json(all_time) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("publish exact all-time projection after staged reconciliation");
    assert_eq!(all_time.total_count, MANIFEST_COUNT + 2);
    assert_eq!(all_time.total_tokens, (MANIFEST_COUNT + 1) * 17 + 23);

    // A rolling-only refresh must retain the fully hydrated all-time snapshot without reopening
    // its large manifest set.
    sqlx::query(
        "DELETE FROM archive_batches \
         WHERE file_path GLOB '/tmp/summary-manifest-admission-*.sqlite.gz'",
    )
    .execute(&state.pool)
    .await
    .expect("remove the admitted manifest fixture before rolling-only refresh");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::Rolling)
        .await
        .expect("rolling refresh retains the exact all-time snapshot");
    state.pool.close().await;

    for (window, upstream_account_id) in [("current", None), ("1d", None), ("1d", Some(42))] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await
        .expect("serve the hydrated rolling response without SQLite");
        assert_eq!(response.total_count, 1, "window {window}");
        assert_eq!(response.total_tokens, 23, "window {window}");
        assert_eq!(response.total_cost, 2.5, "window {window}");
    }

    for upstream_account_id in [None, Some(42)] {
        let response = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("all".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await;
        assert!(
            matches!(response, Err(ApiError::Unavailable(_))),
            "deleting an authority archive must revoke the corresponding all-time snapshot"
        );
    }
}

#[tokio::test]
async fn summary_projection_keeps_30d_memory_exact_when_unreachable_rollups_exceed_admission() {
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse("http://127.0.0.1:9").expect("valid test URL");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let unreachable_bucket = crate::stats::align_bucket_epoch(
        (Utc::now() - ChronoDuration::days(60)).timestamp(),
        3_600,
        0,
    );

    sqlx::query(
        "WITH RECURSIVE sources(ordinal) AS ( \
            SELECT 1 UNION ALL SELECT ordinal + 1 FROM sources WHERE ordinal < 200001 \
         ) \
         INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         SELECT ?1, 'summary-unreachable-rollup-' || ordinal, 1, 1, 0, 1, 0.1, 0 \
         FROM sources",
    )
    .bind(unreachable_bucket)
    .execute(&state.pool)
    .await
    .expect("seed rollups outside the legal summary horizon beyond admission");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-30d-exact-after-admission', datetime('now', '-1 hour'), 'proxy', 'success', 23, 2.5, \
                 '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert legal-horizon exact row");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("unreachable all-time rollup admission must not block rolling hydration");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("30d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve the exact 30d response from the hydrated projection without SQLite");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 23);
    assert_eq!(response.total_cost, 2.5);
}

#[tokio::test]
async fn summary_projection_paged_missing_compact_bucket_is_strictly_unavailable() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let bucket_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(12)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align compact coverage fixture to a full hour");
    let coverage_start = crate::stats::db_occurred_at_lower_bound(bucket_start);
    let coverage_end = crate::db_occurred_at_upper_bound(bucket_start + ChronoDuration::hours(1));

    sqlx::query(
        "WITH RECURSIVE manifests(ordinal) AS ( \
            SELECT 1 UNION ALL SELECT ordinal + 1 FROM manifests WHERE ordinal < 4097 \
         ) \
         INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         SELECT 'codex_invocations', '2026-01', \
                '/tmp/summary-paged-missing-compact-' || ordinal || '.sqlite.gz', \
                'summary-paged-missing-compact-' || ordinal, 1, 'completed', ?1, ?2, \
                datetime('now'), datetime('now'), datetime('now') \
         FROM manifests",
    )
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&state.pool)
    .await
    .expect("insert paged manifests with a missing compact bucket");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             SELECT ?1, dataset, file_path, sha256 FROM archive_batches \
             WHERE file_path GLOB '/tmp/summary-paged-missing-compact-*.sqlite.gz'",
        )
        .bind(target)
        .execute(&state.pool)
        .await
        .expect("record complete replay markers without inventing a compact rollup key");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish recent projection with a local compact-coverage gap");
    state.pool.close().await;

    let response = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(response, Err(ApiError::Unavailable(_))),
        "a missing compact rollup key in a full internal hour must not return an undercounted 200"
    );
}

#[tokio::test]
async fn summary_projection_pages_exact_boundary_manifests_beyond_admission() {
    let _identity_guard = SUMMARY_PROJECTION_ARCHIVE_IDENTITY_TEST_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("summary-paged-boundary-snapshot");
    let database_path = temp_dir.join("summary-projection.db");
    fs::File::create(&database_path).expect("create summary projection database");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&test_sqlite_url_for_path(&database_path))
        .await
        .expect("open summary projection database");
    let mut config = test_config();
    config.database_path = database_path;
    config.archive_dir = temp_dir.join("archives");
    config.proxy_raw_dir = temp_dir.join("proxy-raw");
    fs::create_dir_all(&config.archive_dir).expect("create summary projection archive directory");
    fs::create_dir_all(&config.proxy_raw_dir).expect("create summary projection raw directory");
    config.openai_upstream_base_url = Url::parse("http://127.0.0.1:9").expect("valid test URL");
    let state = test_state_from_existing_pool(pool, config, true).await;
    sqlx::query_scalar::<_, String>("PRAGMA journal_mode = WAL")
        .fetch_one(&state.pool)
        .await
        .expect("enable WAL for concurrent snapshot writer coverage");
    let archived_bucket_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(6)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align boundary fixture to a full hour");
    let archived_at_utc = archived_bucket_start + ChronoDuration::minutes(5);
    let archived_at = format_naive(archived_at_utc.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-paged-boundary-manifest",
        &[SeedInvocationArchiveBatchRow {
            id: 70_001,
            invoke_id: "summary-paged-boundary-manifest",
            occurred_at: archived_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20,
            cost: 0.20,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":42,"responseModel":"gpt-5","reasoningEffort":"high"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let coverage_end = crate::db_occurred_at_lower_bound(
        archived_bucket_start + ChronoDuration::hours(1) - ChronoDuration::seconds(1),
    );
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2, historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(crate::stats::db_occurred_at_lower_bound(archived_bucket_start))
    .bind(&coverage_end)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark the readable partial-hour archive materialized");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load readable boundary archive manifest id");
    // Keep the account manifest deliberately unrefreshed. The paged fallback must replace the
    // existing account compact bucket with this raw record rather than adding both sources.
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
         VALUES (?1, 42, ?2)",
    )
    .bind(archive_batch_id)
    .bind(crate::stats::db_occurred_at_lower_bound(archived_at_utc))
    .execute(&state.pool)
    .await
    .expect("seed unrefreshed account manifest for the readable boundary archive");
    let mut tx = state.pool.begin().await.expect("begin boundary rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 70_001,
            occurred_at: archived_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: None,
            output_tokens: Some(20),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(20),
            cost: Some(0.20),
            upstream_account_id: Some(42),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none".to_string()),
            is_actionable: Some(0),
            payload: Some(
                r#"{"upstreamAccountId":42,"responseModel":"gpt-5","reasoningEffort":"high"}"#
                    .to_string(),
            ),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(120.0),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await
    .expect("seed complete compact rollups for the readable boundary archive");
    tx.commit().await.expect("commit boundary rollup tx");
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    // Prepare a valid replacement archive that would add a second exact-boundary row if it
    // were combined with the snapshot's old durable rollup and manifest state.
    let replacement_archive_db_path = state
        .config
        .archive_dir
        .join("summary-paged-boundary-replacement.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_archive_db_path)
        .expect("inflate replacement archive source");
    let replacement_archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&replacement_archive_db_path))
            .await
            .expect("open replacement archive source");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) \
         VALUES (70002, 'summary-paged-boundary-replacement', ?1, 'proxy', 'success', 99, 9.9, 'full', \
                 '{\"upstreamAccountId\":42,\"responseModel\":\"gpt-5\",\"reasoningEffort\":\"high\"}', '{}', ?1)",
    )
    .bind(&archived_at)
    .execute(&replacement_archive_pool)
    .await
    .expect("seed replacement archive row");
    replacement_archive_pool.close().await;
    let replacement_archive_path = state
        .config
        .archive_dir
        .join("summary-paged-boundary-replacement.sqlite.gz");
    deflate_sqlite_file_to_gzip(&replacement_archive_db_path, &replacement_archive_path)
        .expect("compress replacement archive");
    let replacement_archive_sha256 =
        sha256_hex_file(&replacement_archive_path).expect("hash replacement archive");
    fs::remove_file(&replacement_archive_db_path).expect("remove replacement archive source");

    // These fully materialized older manifests force paging without needing raw records for a
    // legal current/1d selection. Their compact aggregate belongs outside the exact boundary.
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(100)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align compact manifest fixture to a full hour");
    let archive_end = archive_start + ChronoDuration::hours(1);
    let compact_bucket = archive_start.timestamp();
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 4096, 4096, 0, 40960, 409.6, 0)",
    )
    .bind(compact_bucket)
    .execute(&state.pool)
    .await
    .expect("seed compact global rollup for paged manifests");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_output, has_cost) \
         VALUES (?1, 'proxy', '-1', 'gpt-5', 'high', 4096, 40960, 409.6, 1)",
    )
    .bind(compact_bucket)
    .execute(&state.pool)
    .await
    .expect("seed compact global usage rollup for paged manifests");
    sqlx::query(
        "WITH RECURSIVE manifests(ordinal) AS ( \
            SELECT 1 UNION ALL SELECT ordinal + 1 FROM manifests WHERE ordinal < 4097 \
         ) \
         INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         SELECT 'codex_invocations', '2026-01', \
                '/tmp/summary-boundary-manifest-admission-' || ordinal || '.sqlite.gz', \
                'summary-boundary-manifest-admission-' || ordinal, 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now') \
         FROM manifests",
    )
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(crate::stats::db_occurred_at_lower_bound(archive_end))
    .execute(&state.pool)
    .await
    .expect("insert exact-horizon manifests beyond bounded admission");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             SELECT ?1, dataset, file_path, sha256 FROM archive_batches \
             WHERE file_path GLOB '/tmp/summary-boundary-manifest-admission-*.sqlite.gz'",
        )
        .bind(target)
        .execute(&state.pool)
        .await
        .expect("record complete durable replay coverage");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish initial exact paged-boundary snapshot");

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create summary projection interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeArchiveIdentityRevalidation,
    );
    let stage_reached = Arc::new(Notify::new());
    let replacement_cancelled = Arc::new(Notify::new());
    let replacement_path = archive_path.to_string_lossy().to_string();
    let concurrent_terminal_at = archived_at.clone();
    let replacement_pool = state.pool.clone();
    let replacement_interleave = interleave.clone();
    let replacement_stage_reached = stage_reached.clone();
    let replacement_cancelled_for_writer = replacement_cancelled.clone();
    let replacement_archive_path_for_writer = replacement_archive_path.clone();
    let archive_path_for_writer = archive_path.clone();
    let replacement = tokio::spawn(async move {
        tokio::select! {
            _ = replacement_stage_reached.notified() => {}
            _ = replacement_cancelled_for_writer.notified() => return,
        }
        fs::rename(
            &replacement_archive_path_for_writer,
            &archive_path_for_writer,
        )
        .expect("atomically publish replacement archive file");
        let mut tx = replacement_pool
            .begin()
            .await
            .expect("begin legacy manifest replacement");
        sqlx::query(
            "UPDATE archive_batches SET sha256 = ?1 \
             WHERE dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(&replacement_archive_sha256)
        .bind(&replacement_path)
        .execute(tx.as_mut())
        .await
        .expect("replace readable manifest SHA");
        sqlx::query(
            "UPDATE hourly_rollup_archive_replay SET archive_sha256 = ?1 \
             WHERE dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(&replacement_archive_sha256)
        .bind(&replacement_path)
        .execute(tx.as_mut())
        .await
        .expect("refresh replacement replay markers");
        sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response) \
             VALUES ('summary-paged-boundary-concurrent-terminal', ?1, 'proxy', 'success', 7, 0.07, 'full', '{\"upstreamAccountId\":42}', '')",
        )
        .bind(&concurrent_terminal_at)
        .execute(tx.as_mut())
        .await
        .expect("commit terminal write after snapshot establishment");
        let commit = tokio::time::timeout(Duration::from_secs(2), tx.commit()).await;
        replacement_interleave.resume_build();
        commit
            .expect("legacy manifest replacement must not block the snapshot reader")
            .expect("commit legacy manifest replacement");
    });
    // This test exercises archive identity revalidation after a complete boundary admission.
    let hydration = refresh_summary_snapshots_with_test_setup_deadline(
        state.as_ref(),
        SummaryProjectionBuildMode::Rolling,
        Duration::from_secs(30),
        interleave.clone(),
        stage_reached,
    )
    .await;
    replacement_cancelled.notify_one();
    let replacement_result = replacement.await;
    clear_summary_projection_test_interleave();
    replacement_result.expect("run legacy manifest replacement");
    let hydration_error = hydration.expect_err("reject the mixed archive and SQLite snapshot");
    assert!(
        hydration_error
            .to_string()
            .contains("summary projection archive changed during hydration"),
        "unexpected hydration failure: {hydration_error:?}"
    );
    assert_eq!(
        interleave.build_attempts(),
        1,
        "the archive mismatch must fail closed without retrying under normal terminal writes"
    );
    state.pool.close().await;

    for upstream_account_id in [None, Some(42)] {
        let response = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("current".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await;
        assert!(
            matches!(response, Err(ApiError::Unavailable(_))),
            "current {upstream_account_id:?} must not clamp an incomplete resident prefix"
        );
    }

    let Json(response) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve the exact global paged-boundary response without SQLite");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 20);
    assert_f64_close(response.total_cost, 0.20);
    let model = response
        .usage_breakdown
        .expect("paged boundary global usage breakdown")
        .models
        .into_iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("paged boundary model/reasoning detail");
    let model_cost = model.costs.expect("paged boundary model costs").unknown;
    assert!(
        (model_cost - 0.20).abs() < 1e-6,
        "expected model cost 0.20, got {model_cost}",
    );

    let account_response = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(
        matches!(account_response, Err(ApiError::Unavailable(_))),
        "a stale paged account manifest must retain an account-local unavailable proof"
    );
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_projection_rejects_replaced_unmaterialized_all_time_archive() {
    let _identity_guard = SUMMARY_PROJECTION_ARCHIVE_IDENTITY_TEST_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("summary-all-time-archive-identity");
    let database_path = temp_dir.join("summary-projection.db");
    fs::File::create(&database_path).expect("create summary projection database");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&test_sqlite_url_for_path(&database_path))
        .await
        .expect("open summary projection database");
    let mut config = test_config();
    config.database_path = database_path;
    config.archive_dir = temp_dir.join("archives");
    config.proxy_raw_dir = temp_dir.join("proxy-raw");
    config.invocation_max_days = 7;
    config.openai_upstream_base_url = Url::parse("http://127.0.0.1:9").expect("valid test URL");
    fs::create_dir_all(&config.archive_dir).expect("create summary projection archive directory");
    fs::create_dir_all(&config.proxy_raw_dir).expect("create summary projection raw directory");
    let state = test_state_from_existing_pool(pool, config, true).await;
    sqlx::query_scalar::<_, String>("PRAGMA journal_mode = WAL")
        .fetch_one(&state.pool)
        .await
        .expect("enable WAL for concurrent all-time snapshot writer coverage");

    // Keep this beyond the rolling exact horizon (retention plus the 31-day grace window) so
    // only the all-time generic archive aggregation can observe the replacement.
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(45))
    .and_hms_opt(6, 0, 0)
    .expect("valid archived local hour");
    let archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived invocation time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-time-archive-identity",
        &[(
            80_001,
            "summary-all-time-archive-identity",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10,
            0.10,
            Some(100.0),
        )],
    )
    .await;
    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        &archive_path,
    )
    .await;

    let replacement_archive_db_path = state
        .config
        .archive_dir
        .join("summary-all-time-archive-identity-replacement.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_archive_db_path)
        .expect("inflate replacement all-time archive");
    let replacement_archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&replacement_archive_db_path))
            .await
            .expect("open replacement all-time archive");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) \
         VALUES (80002, 'summary-all-time-archive-identity-replacement', ?1, 'proxy', 'success', 99, 9.9, 'full', '{}', '{}', ?1)",
    )
    .bind(&archived_at)
    .execute(&replacement_archive_pool)
    .await
    .expect("seed replacement all-time archive row");
    replacement_archive_pool.close().await;
    let replacement_archive_path = state
        .config
        .archive_dir
        .join("summary-all-time-archive-identity-replacement.sqlite.gz");
    deflate_sqlite_file_to_gzip(&replacement_archive_db_path, &replacement_archive_path)
        .expect("compress replacement all-time archive");
    let replacement_archive_sha256 =
        sha256_hex_file(&replacement_archive_path).expect("hash replacement all-time archive");
    fs::remove_file(&replacement_archive_db_path)
        .expect("remove replacement all-time archive source");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish the initial exact rolling projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("publish initial exact all-time archive snapshot after reconciliation");
    let replacement_path = archive_path.to_string_lossy().to_string();
    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("make all-time archive unavailable before recovery");
    fs::remove_file(&replacement_archive_path).expect("remove unused replacement archive");
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin all-time archive replacement");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = ?1 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(&replacement_archive_sha256)
    .bind(&replacement_path)
    .execute(tx.as_mut())
    .await
    .expect("replace all-time archive manifest SHA");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response) \
         VALUES ('summary-all-time-archive-identity-terminal', ?1, 'proxy', 'success', 7, 0.07, 'full', '{}', '')",
    )
    .bind(&archived_at)
    .execute(tx.as_mut())
    .await
    .expect("write terminal beside all-time archive replacement");
    tx.commit()
        .await
        .expect("commit all-time archive replacement");

    tokio::time::timeout(
        Duration::from_secs(2),
        refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime),
    )
    .await
    .expect("all-time recovery must not wait for an unchanged publication stage")
    .expect("retain the prior all-time aggregate without scanning a replaced archive");
    state.pool.close().await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve the prior all-time snapshot without SQLite");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 10);
    assert_f64_close(summary.total_cost, 0.10);
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_projection_keeps_global_all_exact_when_account_manifest_admission_exceeds_budget()
{
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(1_000)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align archive fixture to a full hour");
    let bucket = crate::stats::align_bucket_epoch(archive_start.timestamp(), 3_600, 0);
    let file_path = "/tmp/summary-account-manifest-admission.sqlite.gz";
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert durable global rollup");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, created_at) \
         VALUES ('codex_invocations', '2026-01', ?1, 'summary-account-manifest-admission', 1, 'completed', \
                 ?2, ?3, datetime('now'), datetime('now'))",
    )
    .bind(file_path)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("insert materialized archive manifest");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load archive manifest id");
    sqlx::query(
        "WITH RECURSIVE accounts(account_id) AS ( \
            SELECT 1 UNION ALL SELECT account_id + 1 FROM accounts WHERE account_id <= 50000 \
         ) \
         INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
         SELECT ?1, account_id, ?2 FROM accounts",
    )
    .bind(archive_batch_id)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("insert account activity beyond bounded admission");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', ?2, 'summary-account-manifest-admission')",
        )
        .bind(target)
        .bind(file_path)
        .execute(&state.pool)
        .await
        .expect("record durable replay coverage");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate exact global all-time response despite account admission overflow");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile exact global all-time response despite account admission overflow");
    state.pool.close().await;

    let Json(global) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve exact global all-time response without SQLite");
    assert_eq!(global.total_count, 1);
    assert_eq!(global.total_tokens, 17);
    assert_eq!(global.total_cost, 1.25);
    assert!(matches!(
        fetch_summary(
            State(state),
            Query(SummaryQuery {
                window: Some("all".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: Some(42),
            }),
        )
        .await,
        Err(ApiError::Unavailable(_))
    ));
}

#[tokio::test]
async fn summary_projection_rejects_materialized_endpoint_hour_missing_rollup_keys() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(1_000)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align archive fixture to a full hour");
    let endpoint_hour = archive_start + ChronoDuration::hours(1);
    let file_path = "/tmp/summary-missing-endpoint-rollup-key.sqlite.gz";
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         VALUES ('codex_invocations', '2026-01', ?1, 'summary-missing-endpoint-rollup-key', 2, 'completed', \
                 ?2, ?3, datetime('now'), datetime('now'), datetime('now'))",
    )
    .bind(file_path)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    // `coverage_end_at` is inclusive: this batch also contains a row in `endpoint_hour`.
    .bind(crate::stats::db_occurred_at_lower_bound(endpoint_hour))
    .execute(&state.pool)
    .await
    .expect("insert materialized archive manifest");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load endpoint archive manifest id");
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
         VALUES (?1, 42, ?2)",
    )
    .bind(archive_batch_id)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("record endpoint archive account manifest");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(archive_start.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed only the pre-endpoint global rollup key");
    sqlx::query(
        "INSERT INTO upstream_account_stats_hourly \
         (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 42, 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(archive_start.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed only the pre-endpoint account rollup key");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', ?2, 'summary-missing-rollup-key')",
        )
        .bind(target)
        .bind(file_path)
        .execute(&state.pool)
        .await
        .expect("record materialized replay marker");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate rolling projection with an incomplete all-time rollup");
    state.pool.close().await;

    let Json(rolling) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve the exact rolling zero response without SQLite");
    assert_eq!(rolling.total_count, 0);

    for upstream_account_id in [None, Some(42)] {
        assert!(
            matches!(
                fetch_summary(
                    State(state.clone()),
                    Query(SummaryQuery {
                        window: Some("all".to_string()),
                        limit: None,
                        time_zone: Some("UTC".to_string()),
                        upstream_account_id,
                    }),
                )
                .await,
                Err(ApiError::Unavailable(_))
            ),
            "the endpoint hour must be proven for {upstream_account_id:?}"
        );
    }
}
