async fn warm_timeseries_selection(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    upstream_account_id: Option<i64>,
    label: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) {
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        upstream_account_id,
        state.terminal_projection_hub.as_ref(),
        label,
        coordinator,
    )
    .await
    .expect("warm timeseries selection");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
}

async fn assert_same_cursor_exact_fallback(state: Arc<AppState>, now: DateTime<Utc>) {
    for upstream_account_id in [None, Some(17)] {
        assert!(
            crate::api::timeseries_minute_projection_has_uncovered_terminal_delta(
                state.terminal_projection_hub.as_ref(),
                InvocationSourceScope::All,
                upstream_account_id,
                now - ChronoDuration::hours(1),
                now,
            )
        );
        let Json(response) = crate::api::fetch_timeseries(
            axum::extract::State(state.clone()),
            axum::extract::Query(TimeseriesQuery {
                range: "1h".to_string(),
                bucket: Some("1m".to_string()),
                settlement_hour: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await
        .expect("build HTTP exact-fallback timeseries response");
        let payload = serde_json::to_value(response).expect("serialize HTTP timeseries response");
        let point = payload["points"]
            .as_array()
            .expect("HTTP timeseries points")
            .iter()
            .find(|point| point["totalCount"] == 1)
            .expect("replacement minute point");
        assert_eq!(point["successCount"], 0);
        assert_eq!(point["failureCount"], 1);
    }
    let query = TimeseriesQuery {
        range: "1h".to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        time_zone: Some("UTC".to_string()),
        upstream_account_id: None,
    };
    let base = crate::api::TimeseriesTopicMaterializedBase::build(state.as_ref(), &query)
        .await
        .expect("build materialized timeseries base");
    let payload: serde_json::Value = serde_json::from_slice(
        &base
            .serialize(&[])
            .expect("serialize exact-fallback materialized response"),
    )
    .expect("materialized timeseries JSON");
    let point = payload["points"]
        .as_array()
        .expect("timeseries points")
        .iter()
        .find(|point| point["totalCount"] == 1)
        .expect("replacement minute point");
    assert_eq!(point["successCount"], 0);
    assert_eq!(point["failureCount"], 1);
}

async fn seed_summary_manifest_admission_fixture() -> (Arc<AppState>, DateTime<Utc>, i64) {
    const MANIFEST_COUNT: i64 = 4_097;
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
    sqlx::query("INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', ?2, ?2, 0, ?3, ?4, 0)").bind(bucket).bind(MANIFEST_COUNT).bind(MANIFEST_COUNT * 17).bind(MANIFEST_COUNT as f64 * 1.25).execute(&state.pool).await.expect("insert durable summary rollup");
    sqlx::query("INSERT INTO upstream_account_stats_hourly (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', 42, ?2, ?2, 0, ?3, ?4, 0)").bind(bucket).bind(MANIFEST_COUNT).bind(MANIFEST_COUNT * 17).bind(MANIFEST_COUNT as f64 * 1.25).execute(&state.pool).await.expect("insert durable account summary rollup");
    sqlx::query("WITH RECURSIVE manifests(ordinal) AS (SELECT 1 UNION ALL SELECT ordinal + 1 FROM manifests WHERE ordinal < ?3) INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) SELECT 'codex_invocations', '2026-01', '/tmp/summary-manifest-admission-' || ordinal || '.sqlite.gz', 'summary-manifest-admission-' || ordinal, 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now') FROM manifests").bind(crate::stats::db_occurred_at_lower_bound(archive_start)).bind(crate::stats::db_occurred_at_lower_bound(archive_start)).bind(MANIFEST_COUNT).execute(&state.pool).await.expect("insert archive manifests beyond admission");
    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
        sqlx::query("INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) SELECT ?1, dataset, file_path, sha256 FROM archive_batches WHERE file_path GLOB '/tmp/summary-manifest-admission-*.sqlite.gz'").bind(target).execute(&state.pool).await.expect("record complete rollup replay coverage");
    }
    let live_id = sqlx::query("INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) VALUES ('summary-manifest-admission-live', datetime('now'), 'proxy', 'success', 23, 2.5, '{\"upstreamAccountId\":42}', '', 'full')").execute(&state.pool).await.expect("insert exact live-tail row").last_insert_rowid();
    for dataset in [
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        "invocation_account_activity_v2_repair_live_cursor",
    ] {
        sqlx::query("INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = excluded.updated_at").bind(dataset).bind(live_id).execute(&state.pool).await.expect("mark compact live cursor");
    }
    (state, archive_start, bucket)
}

async fn open_paged_boundary_state() -> (Arc<AppState>, PathBuf) {
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
    (state, temp_dir)
}

async fn seed_readable_boundary_archive(state: &AppState) -> (PathBuf, String, DateTime<Utc>) {
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
            occurred_at: &archived_at,
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
    sqlx::query("UPDATE archive_batches SET coverage_start_at = ?1, coverage_end_at = ?2, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?3").bind(crate::stats::db_occurred_at_lower_bound(archived_bucket_start)).bind(crate::db_occurred_at_upper_bound(archived_bucket_start + ChronoDuration::hours(1))).bind(archive_path.to_string_lossy().to_string()).execute(&state.pool).await.expect("mark the readable partial-hour archive materialized");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load readable boundary archive manifest id");
    sqlx::query("INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) VALUES (?1, 42, ?2)").bind(archive_batch_id).bind(crate::stats::db_occurred_at_lower_bound(archived_at_utc)).execute(&state.pool).await.expect("seed unrefreshed account manifest");
    (archive_path, archived_at, archived_at_utc)
}

async fn seed_readable_boundary_rollup(state: &AppState, archived_at: &str) {
    let mut tx = state.pool.begin().await.expect("begin boundary rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 70_001,
            occurred_at: archived_at.to_string(),
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
    .expect("seed complete compact rollups");
    tx.commit().await.expect("commit boundary rollup tx");
}

async fn seed_boundary_compact_manifests(state: &AppState) {
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
    sqlx::query("INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', 4096, 4096, 0, 40960, 409.6, 0)").bind(archive_start.timestamp()).execute(&state.pool).await.expect("seed compact global rollup");
    sqlx::query("INSERT INTO upstream_account_usage_breakdown_hourly (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, request_count, output_tokens, cost_output, has_cost) VALUES (?1, 'proxy', '-1', 'gpt-5', 'high', 4096, 40960, 409.6, 1)").bind(archive_start.timestamp()).execute(&state.pool).await.expect("seed compact usage rollup");
    sqlx::query("WITH RECURSIVE manifests(ordinal) AS (SELECT 1 UNION ALL SELECT ordinal + 1 FROM manifests WHERE ordinal < 4097) INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) SELECT 'codex_invocations', '2026-01', '/tmp/summary-boundary-manifest-admission-' || ordinal || '.sqlite.gz', 'summary-boundary-manifest-admission-' || ordinal, 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now') FROM manifests").bind(crate::stats::db_occurred_at_lower_bound(archive_start)).bind(crate::stats::db_occurred_at_lower_bound(archive_end)).execute(&state.pool).await.expect("insert exact-horizon manifests");
    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
        sqlx::query("INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) SELECT ?1, dataset, file_path, sha256 FROM archive_batches WHERE file_path GLOB '/tmp/summary-boundary-manifest-admission-*.sqlite.gz'").bind(target).execute(&state.pool).await.expect("record complete durable replay coverage");
    }
}

async fn prepare_boundary_replacement_archive(
    state: &AppState,
    archive_path: &Path,
    archived_at: &str,
) -> (PathBuf, String) {
    let db_path = state
        .config
        .archive_dir
        .join("summary-paged-boundary-replacement.sqlite");
    inflate_gzip_sqlite_file(archive_path, &db_path).expect("inflate replacement archive source");
    let pool = SqlitePool::connect(&test_sqlite_url_for_path(&db_path))
        .await
        .expect("open replacement archive source");
    sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) VALUES (70002, 'summary-paged-boundary-replacement', ?1, 'proxy', 'success', 99, 9.9, 'full', '{\"upstreamAccountId\":42,\"responseModel\":\"gpt-5\",\"reasoningEffort\":\"high\"}', '{}', ?1)").bind(archived_at).execute(&pool).await.expect("seed replacement archive row");
    pool.close().await;
    let replacement_path = state
        .config
        .archive_dir
        .join("summary-paged-boundary-replacement.sqlite.gz");
    deflate_sqlite_file_to_gzip(&db_path, &replacement_path).expect("compress replacement archive");
    let sha = sha256_hex_file(&replacement_path).expect("hash replacement archive");
    fs::remove_file(db_path).expect("remove replacement archive source");
    (replacement_path, sha)
}

async fn run_boundary_interleaved_refresh(
    state: Arc<AppState>,
    archive_path: PathBuf,
    replacement_path: PathBuf,
    replacement_sha: String,
    archived_at: String,
) {
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create summary projection interleave gate");
    let interleave = install_summary_projection_test_interleave();
    let replacement_path_text = archive_path.to_string_lossy().to_string();
    let replacement_pool = state.pool.clone();
    let replacement_interleave = interleave.clone();
    let replacement = tokio::spawn(async move {
        replacement_interleave.wait_for_writer().await;
        fs::rename(replacement_path, &archive_path)
            .expect("atomically publish replacement archive file");
        let mut tx = replacement_pool
            .begin()
            .await
            .expect("begin legacy manifest replacement");
        sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2").bind(&replacement_sha).bind(&replacement_path_text).execute(tx.as_mut()).await.expect("replace readable manifest SHA");
        sqlx::query("UPDATE hourly_rollup_archive_replay SET archive_sha256 = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2").bind(&replacement_sha).bind(&replacement_path_text).execute(tx.as_mut()).await.expect("refresh replacement replay markers");
        sqlx::query("INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response) VALUES ('summary-paged-boundary-concurrent-terminal', ?1, 'proxy', 'success', 7, 0.07, 'full', '{\"upstreamAccountId\":42}', '')").bind(&archived_at).execute(tx.as_mut()).await.expect("commit terminal write after snapshot establishment");
        let commit = tokio::time::timeout(Duration::from_secs(2), tx.commit()).await;
        replacement_interleave.resume_build();
        commit
            .expect("manifest replacement must not block snapshot reader")
            .expect("commit legacy manifest replacement");
    });
    let hydration =
        refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::Rolling)
            .await;
    replacement.await.expect("run legacy manifest replacement");
    clear_summary_projection_test_interleave();
    let error = hydration.expect_err("reject mixed archive and SQLite snapshot");
    assert!(
        error
            .to_string()
            .contains("summary projection archive changed during hydration")
    );
    assert_eq!(interleave.build_attempts(), 1);
}

async fn hydrate_summary_and_capture_first_checkpoint(
    state: Arc<AppState>,
    started_at: std::time::Instant,
) -> (i64, i64, i64) {
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate rolling summary projection beyond manifest admission");
    assert!(
        started_at.elapsed() < std::time::Duration::from_secs(30),
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
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("advance the first all-time staged checkpoint page");
    sqlx::query_as::<_, (i64, i64, i64)>("SELECT COALESCE(manifest_high_watermark_id, 0), global_manifest_next_id, global_rollup_next_rowid FROM summary_all_time_projection_checkpoint WHERE scope = 'all'").fetch_one(&state.pool).await.expect("persist first staged all-time checkpoint")
}

async fn append_summary_manifest_generation(
    state: &AppState,
    archive_start: DateTime<Utc>,
    bucket: i64,
) {
    sqlx::query("INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) VALUES ('codex_invocations', '2026-01', '/tmp/summary-manifest-admission-generation.sqlite.gz', 'summary-manifest-admission-generation', 1, 'completed', ?1, ?2, datetime('now'), datetime('now'), datetime('now'))").bind(crate::stats::db_occurred_at_lower_bound(archive_start)).bind(crate::stats::db_occurred_at_lower_bound(archive_start)).execute(&state.pool).await.expect("append a later all-time manifest generation");
    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
        sqlx::query("INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', '/tmp/summary-manifest-admission-generation.sqlite.gz', 'summary-manifest-admission-generation')").bind(target).execute(&state.pool).await.expect("record replay proof for next manifest generation");
    }
    sqlx::query("UPDATE invocation_rollup_hourly SET total_count = total_count + 1, success_count = success_count + 1, total_tokens = total_tokens + 17, total_cost = total_cost + 1.25 WHERE bucket_start_epoch = ?1 AND source = 'proxy'").bind(bucket).execute(&state.pool).await.expect("extend the global compact rollup");
    sqlx::query("UPDATE upstream_account_stats_hourly SET total_count = total_count + 1, success_count = success_count + 1, total_tokens = total_tokens + 17, total_cost = total_cost + 1.25 WHERE bucket_start_epoch = ?1 AND source = 'proxy' AND upstream_account_id = 42").bind(bucket).execute(&state.pool).await.expect("extend the account compact rollup");
}

async fn converge_summary_manifest_generation(state: &AppState, first: (i64, i64, i64)) {
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    refresh_summary_snapshots(state)
        .await
        .expect("restart staged checkpoint at later manifest generation");
    let reset = sqlx::query_as::<_, (i64, i64, i64)>("SELECT COALESCE(manifest_high_watermark_id, 0), global_manifest_next_id, global_rollup_next_rowid FROM summary_all_time_projection_checkpoint WHERE scope = 'all'").fetch_one(&state.pool).await.expect("load reset staged all-time checkpoint");
    assert!(reset.0 > first.0 && reset.1 <= first.1 && reset.2 <= first.2);
    for _ in 0..200 {
        state
            .subscription_hub
            .note_summary_http_interest(true)
            .await;
        refresh_summary_snapshots(state)
            .await
            .expect("resume bounded staged all-time recovery");
        let complete = sqlx::query_as::<_, (i64, i64, i64, i64)>("SELECT global_manifest_complete, account_manifest_complete, global_rollup_complete, account_rollup_complete FROM summary_all_time_projection_checkpoint WHERE scope = 'all'").fetch_one(&state.pool).await.expect("load staged all-time completion state");
        if complete == (1, 1, 1, 1) {
            return;
        }
    }
    panic!("staged all-time recovery must converge from committed manifest and rollup cursors");
}

#[tokio::test]
pub(crate) async fn startup_minute_projection_recovery_keeps_the_durable_fence_when_cancelled_after_warming()
 {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for minute_start_epoch in 0_i64..=1_024 {
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state) VALUES (?1, 'all', -1, '{}', '[]', '[]', '[]', '[]', 0, 'ready')",
        )
        .bind(minute_start_epoch)
        .execute(&state.pool)
        .await
        .expect("seed coverage invalidation batches");
    }
    crate::api::mark_timeseries_minute_projection_startup_recovery(&state.pool)
        .await
        .expect("publish durable recovery marker");

    let cancellation = CancellationToken::new();
    let recovery_state = state.clone();
    let recovery_cancellation = cancellation.clone();
    let recovery = tokio::spawn(async move {
        crate::api::prepare_timeseries_minute_projection_after_restart(
            recovery_state.as_ref(),
            &recovery_cancellation,
        )
        .await
    });
    let mut saw_warming = false;
    for _ in 0..1_000 {
        let recovery_state = sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_error FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
        )
        .fetch_optional(&state.pool)
        .await
        .expect("load startup recovery state")
        .flatten();
        if recovery_state.as_deref() == Some("warming") {
            cancellation.cancel();
            saw_warming = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        saw_warming,
        "cancellation must occur after recovery publishes warming and before it can report ready"
    );
    let outcome = recovery
        .await
        .expect("recovery task should not panic")
        .expect("cancelled recovery should not report a database error");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Cancelled
    );
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load retained recovery marker");
    assert_eq!(recovery_pending, 1);
    let recovery_state = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_error FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_optional(&state.pool)
    .await
    .expect("load cancelled recovery state")
    .flatten();
    assert_eq!(recovery_state.as_deref(), Some("warming"));
}

#[tokio::test]
pub(crate) async fn live_update_trigger_rebuild_fences_source_changing_terminal_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query("DROP TABLE timeseries_minute_projection_v2_recovery")
        .execute(&state.pool)
        .await
        .expect("model an interrupted startup with a missing recovery table");
    crate::schema::rebuild_invocation_in_progress_live_triggers(&state.pool)
        .await
        .expect("rebuild must restore the recovery table with its live update trigger");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) VALUES ('rebuild-recovery-table', datetime('now'), 'proxy', 'running', '{}')",
    )
    .execute(&state.pool)
    .await
    .expect("insert an in-flight proxy invocation");
    sqlx::query("UPDATE codex_invocations SET source = 'cli', status = 'success' WHERE invoke_id = 'rebuild-recovery-table'")
        .execute(&state.pool)
        .await
    .expect("source-changing terminal write must not target a missing recovery table");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("live update trigger publishes the restored recovery marker");
    assert_eq!(recovery_pending, 1);
}

#[tokio::test]
pub(crate) async fn materialized_timeseries_uses_exact_fallback_for_same_cursor_terminal_replacement()
 {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let invoke_id = "timeseries-materializer-same-cursor-replacement";
    let persisted = persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("persist initial terminal row")
    .expect("initial terminal row should be stored");
    assert_eq!(persisted.status.as_deref(), Some("success"));

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let warm_start = now - ChronoDuration::minutes(62);
    let warm_end = now + ChronoDuration::minutes(2);
    warm_timeseries_selection(
        &state,
        warm_start,
        warm_end,
        None,
        "stateful_same_cursor_materializer_seed",
        &coordinator,
    )
    .await;
    warm_timeseries_selection(
        &state,
        warm_start,
        warm_end,
        Some(17),
        "stateful_same_cursor_account_materializer_seed",
        &coordinator,
    )
    .await;

    let minute_start = parse_to_utc_datetime(&occurred_at)
        .expect("parse persisted occurrence")
        .timestamp()
        .div_euclid(60)
        * 60;
    let (coverage_state, projection_cursor): (String, i64) = sqlx::query_as(
        "SELECT coverage_state, max_row_id FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(minute_start)
    .fetch_one(&state.pool)
    .await
    .expect("load warm minute projection");
    assert_eq!(coverage_state, "ready");
    assert_eq!(projection_cursor, persisted.id);

    sqlx::query(
        "UPDATE codex_invocations SET status = 'failed', failure_kind = 'upstream_response_failed', failure_class = 'service_failure', is_actionable = 1 WHERE id = ?1",
    )
    .bind(persisted.id)
    .execute(&state.pool)
    .await
    .expect("replace the terminal row without advancing its cursor");

    let mut replacement =
        api_invocation_from_runtime_record(&test_proxy_capture_record(invoke_id, &occurred_at));
    replacement.status = Some("failed".to_string());
    replacement.failure_kind = Some("upstream_response_failed".to_string());
    replacement.failure_class = Some("service_failure".to_string());
    replacement.is_actionable = Some(true);
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);
    let event_id = state
        .terminal_projection_hub
        .register_pending(&replacement, None)
        .expect("replacement must fit in the terminal projection journal");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &replacement.invoke_id,
        &replacement.occurred_at,
        persisted.id,
    );

    assert_same_cursor_exact_fallback(state.clone(), now).await;
}

#[tokio::test]
pub(crate) async fn summary_projection_hydrates_rolling_windows_beyond_archive_manifest_admission()
{
    const MANIFEST_COUNT: i64 = 4_097;
    let projection_fixture_started_at = std::time::Instant::now();
    let (state, archive_start, bucket) = seed_summary_manifest_admission_fixture().await;

    let first_checkpoint =
        hydrate_summary_and_capture_first_checkpoint(state.clone(), projection_fixture_started_at)
            .await;
    assert!(first_checkpoint.1 > 0);
    assert!(first_checkpoint.2 > 0);

    append_summary_manifest_generation(state.as_ref(), archive_start, bucket).await;
    converge_summary_manifest_generation(state.as_ref(), first_checkpoint).await;

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
pub(crate) async fn summary_projection_keeps_30d_memory_exact_when_unreachable_rollups_exceed_admission()
 {
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
pub(crate) async fn summary_projection_paged_missing_compact_bucket_is_strictly_unavailable() {
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
pub(crate) async fn summary_projection_pages_exact_boundary_manifests_beyond_admission() {
    let _identity_guard = SUMMARY_PROJECTION_ARCHIVE_IDENTITY_TEST_LOCK.lock().await;
    let (state, temp_dir) = open_paged_boundary_state().await;
    let (archive_path, archived_at, _archived_at_utc) =
        seed_readable_boundary_archive(state.as_ref()).await;
    seed_readable_boundary_rollup(state.as_ref(), &archived_at).await;
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    let (replacement_archive_path, replacement_archive_sha256) =
        prepare_boundary_replacement_archive(&state, &archive_path, &archived_at).await;
    seed_boundary_compact_manifests(state.as_ref()).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish initial exact paged-boundary snapshot");

    run_boundary_interleaved_refresh(
        state.clone(),
        archive_path.clone(),
        replacement_archive_path,
        replacement_archive_sha256,
        archived_at.clone(),
    )
    .await;
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

use super::*;
