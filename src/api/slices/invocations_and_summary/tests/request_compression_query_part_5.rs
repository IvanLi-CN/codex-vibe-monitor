async fn seed_summary_projection_all_time_account_manifest_gap() -> (Arc<AppState>, String) {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let bucket = align_bucket_epoch(
        (Utc::now() - ChronoDuration::days(180)).timestamp(),
        3_600,
        0,
    );
    let coverage_start = Utc
        .timestamp_opt(bucket, 0)
        .single()
        .expect("valid archive coverage start")
        .to_rfc3339();
    let coverage_end = Utc
        .timestamp_opt(bucket.saturating_add(3_599), 0)
        .single()
        .expect("valid archive coverage end")
        .to_rfc3339();
    let archive_path = "/definitely/missing/summary-account-coverage.sqlite.gz";
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, 'proxy', 3, 2, 1, 91, 4.5, 1.5)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert durable global rollup");
    sqlx::query(
            "INSERT INTO upstream_account_stats_hourly \
             (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, 'proxy', 42, 3, 2, 1, 91, 4.5, 1.5)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert observed archive account rollup");
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
            "INSERT INTO pool_upstream_accounts \
             (id, kind, provider, display_name, status, enabled, created_at, updated_at) \
             VALUES (43, 'api_key_codex', 'codex', 'missing manifest account', 'active', 1, ?1, ?1)",
        )
        .bind(&created_at)
        .execute(&state.pool)
        .await
        .expect("register account omitted by the interrupted archive manifest");
    sqlx::query(
        "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, \
              coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at) \
             VALUES ('codex_invocations', '2026-01', ?1, 'summary-account-coverage', 3, \
                     'completed', ?2, ?3, datetime('now'), datetime('now'))",
    )
    .bind(archive_path)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&state.pool)
    .await
    .expect("insert materialized archive manifest");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path)
    .fetch_one(&state.pool)
    .await
    .expect("load materialized archive id");
    sqlx::query(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
             VALUES (?1, 42, ?2)",
        )
        .bind(archive_batch_id)
        .bind(&coverage_start)
        .execute(&state.pool)
        .await
        .expect("record archive account manifest");
    (state, archive_path.to_string())
}

async fn assert_summary_projection_all_time_replay_is_missing(state: &Arc<AppState>) {
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
            "missing replay marker must fail closed for {upstream_account_id:?}"
        );
    }
}

async fn mark_summary_projection_archive_replay_targets(
    state: &Arc<AppState>,
    archive_path: &str,
    targets: &[&str],
) {
    for target in targets {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', ?2, 'summary-account-coverage')",
        )
        .bind(target)
        .bind(archive_path)
        .execute(&state.pool)
        .await
        .expect("mark compact archive target replayed");
    }
}

async fn assert_summary_projection_account_manifest_completion(
    state: &Arc<AppState>,
    archive_path: &str,
) {
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
    .expect("global compact coverage remains exact");
    assert_eq!(global.total_count, 3);
    assert_eq!(global.total_tokens, 91);

    let incomplete_manifest_account = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(43),
        }),
    )
    .await;
    assert!(matches!(
        incomplete_manifest_account,
        Err(ApiError::Unavailable(_))
    ));

    sqlx::query(
        "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') \
             WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path)
    .execute(&state.pool)
    .await
    .expect("complete archive account manifest");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate all-time projection after account manifest completion");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile all-time projection after account manifest completion");
    let Json(complete_manifest_account) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(43),
        }),
    )
    .await
    .expect("completed archive manifest proves the zero-valued account scope");
    assert_eq!(complete_manifest_account.total_count, 0);
}

async fn assert_summary_projection_replay_identity_revocation(
    state: &Arc<AppState>,
    archive_path: &str,
) {
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL \
             WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(archive_path)
    .execute(&state.pool)
    .await
    .expect("remove global replay marker identity");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("identityless replay refresh publishes a bounded recent projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile identityless replay while revoking global all-time authority");

    let missing_global = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(missing_global, Err(ApiError::Unavailable(_))));

    let Json(account) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("independently proven account coverage remains exact");
    assert_eq!(account.total_count, 3);
    assert_eq!(account.total_tokens, 91);

    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay \
             WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(archive_path)
    .execute(&state.pool)
    .await
    .expect("remove account usage replay marker");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("refresh account coverage after usage proof removal");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile account coverage after usage proof removal");
    let missing_account_usage = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(matches!(
        missing_account_usage,
        Err(ApiError::Unavailable(_))
    ));
}

#[tokio::test]
async fn summary_projection_all_time_rejects_archive_account_with_missing_compact_scope() {
    let (state, archive_path) = seed_summary_projection_all_time_account_manifest_gap().await;

    // A durable bucket key is not proof of a completed archive replay. This archive is
    // outside the finite raw horizon, so projection hydration remains available while the
    // all-time selection itself preserves the established unavailable contract.
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with incomplete archive replay");
    assert_summary_projection_all_time_replay_is_missing(&state).await;
    mark_summary_projection_archive_replay_targets(
        &state,
        &archive_path,
        &[
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        ],
    )
    .await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate all-time projection from durable global rollup");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile all-time projection from durable global rollup");
    assert_summary_projection_account_manifest_completion(&state, &archive_path).await;
    assert_summary_projection_replay_identity_revocation(&state, &archive_path).await;
    state.pool.close().await;
}

#[tokio::test]
async fn summary_projection_all_time_replays_unmaterialized_archive_when_replay_sha_is_stale() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(180))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-all-time-stale-replay-sha",
        &[SeedInvocationArchiveBatchRow {
            id: 9_001,
            invoke_id: "summary-all-time-stale-replay-sha",
            occurred_at: &archived_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 17,
            cost: 1.7,
            ttfb_ms: None,
            payload: Some(r#"{"upstreamAccountId":42}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_path = archive_path.to_string_lossy().to_string();
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .fetch_one(&state.pool)
    .await
    .expect("load unmaterialized archive manifest");
    sqlx::query(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
             VALUES (?1, 42, ?2)",
        )
        .bind(archive_batch_id)
        .bind(&archived_at)
        .execute(&state.pool)
        .await
        .expect("record archived account activity");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', ?2, 'stale-replaced-archive-sha')",
        )
        .bind(target)
        .bind(&archive_path)
        .execute(&state.pool)
        .await
        .expect("seed stale replay marker for replaced archive");
    }

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("stale replay markers must admit exact all-time raw fallback");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile stale replay markers through the exact all-time raw fallback");
    state.pool.close().await;

    for upstream_account_id in [None, Some(42)] {
        let Json(summary) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("all".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await
        .expect("closed-pool all-time Summary must use the hydrated raw fallback");
        assert_eq!(summary.total_count, 1, "scope {upstream_account_id:?}");
        assert_eq!(summary.total_tokens, 17, "scope {upstream_account_id:?}");
        assert_eq!(summary.total_cost, 1.7, "scope {upstream_account_id:?}");
    }
}

#[tokio::test]
async fn summary_projection_fails_closed_for_unmaterialized_archive_with_global_only_replay() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    // An unrelated durable rollup must not mask an unreadable unmaterialized archive in
    // the all-time view.
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, 'proxy', 3, 3, 0, 30, 3.0, 0.0)",
        )
        .bind(align_bucket_epoch(
            (Utc::now() - ChronoDuration::days(8)).timestamp(),
            3_600,
            0,
        ))
        .execute(&state.pool)
        .await
        .expect("insert unrelated durable global rollup");
    let start = Utc::now() - ChronoDuration::hours(2);
    let end = start + ChronoDuration::hours(1);
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at) \
             VALUES ('codex_invocations', '2026-01', '/definitely/missing/unmaterialized-summary.sqlite.gz', \
                     'summary-unavailable-test', 1, 'completed', ?1, ?2, datetime('now'))",
        )
        .bind(db_occurred_at_lower_bound(start))
        .bind(db_occurred_at_upper_bound(end))
        .execute(&state.pool)
        .await
        .expect("insert unavailable unmaterialized archive manifest");
    sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', '/definitely/missing/unmaterialized-summary.sqlite.gz', \
                     'summary-unavailable-test')",
        )
        .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
        .execute(&state.pool)
        .await
        .expect("mark only global replay coverage");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish projection with unavailable archive metadata");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert_eq!(
        projection.unavailable_unmaterialized_archive_ranges.len(),
        1,
        "global-only replay cannot prove account or usage dimensions for a missing archive"
    );
    state.pool.close().await;

    for window in ["current", "1d", "all"] {
        let response = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await;
        assert!(
            matches!(response, Err(ApiError::Unavailable(_))),
            "{window} must fail closed without touching the closed SQLite pool"
        );
    }
}

#[tokio::test]
async fn summary_projection_rollups_sum_sources_without_cross_account_suppression() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    for (source, count, tokens, cost) in [("proxy", 2_i64, 20_i64, 2.0_f64), ("xy", 3, 30, 3.0)] {
        sqlx::query(
                "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, ?2, ?3, ?3, 0, ?4, ?5, 0)",
            )
            .bind(bucket)
            .bind(source)
            .bind(count)
            .bind(tokens)
            .bind(cost)
            .execute(&state.pool)
            .await
            .expect("insert source-partitioned overall rollup");
        sqlx::query(
                "INSERT INTO upstream_account_stats_hourly (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, ?2, 42, ?3, ?3, 0, ?4, ?5, 0)",
            )
            .bind(bucket)
            .bind(source)
            .bind(count)
            .bind(tokens)
            .bind(cost)
            .execute(&state.pool)
            .await
            .expect("insert source-partitioned account rollup");
    }
    let (totals, _) = load_summary_projection_rollup_totals(&state.pool)
        .await
        .expect("load source-partitioned rollups");
    assert_eq!(totals[&(bucket, None)].total_count, 5);
    assert_eq!(totals[&(bucket, None)].total_tokens, 50);
    assert_eq!(totals[&(bucket, None)].total_cost, 5.0);
    assert_eq!(totals[&(bucket, Some(42))].total_count, 5);
    assert_eq!(totals[&(bucket, Some(42))].total_tokens, 50);
}

#[tokio::test]
async fn summary_projection_usage_rollups_sum_sources_once_per_scope() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let bucket = align_bucket_epoch(
        (Utc::now() - ChronoDuration::hours(2)).timestamp(),
        3_600,
        0,
    );
    for (source, count, input_cost) in [("proxy", 2_i64, 0.5_f64), ("xy", 3_i64, 0.75_f64)] {
        sqlx::query(
            "INSERT INTO upstream_account_usage_breakdown_hourly \
                 (bucket_start_epoch, source, upstream_account_key, upstream_account_id, \
                  normalized_model, normalized_reasoning_effort, request_count, \
                  cache_write_tokens, cache_read_tokens, output_tokens, cost_input, has_cost) \
                 VALUES (?1, ?2, '42', 42, 'gpt-5', 'high', ?3, 10, 20, 30, ?4, 1)",
        )
        .bind(bucket)
        .bind(source)
        .bind(count)
        .bind(input_cost)
        .execute(&state.pool)
        .await
        .expect("insert source-partitioned usage rollup");
    }
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
             (bucket_start_epoch, source, upstream_account_key, upstream_account_id, \
              normalized_model, normalized_reasoning_effort, request_count, \
              cache_write_tokens, cache_read_tokens, output_tokens, cost_input, has_cost) \
             VALUES (?1, 'proxy', 'none', NULL, 'gpt-5', 'high', 1, 4, 5, 6, 0.25, 1)",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert account-less usage rollup");

    let usage = load_summary_projection_rollup_usage(&state.pool)
        .await
        .expect("load source-partitioned usage rollups");
    let global = usage.get(&(bucket, None)).expect("global usage rollup");
    let account = usage
        .get(&(bucket, Some(42)))
        .expect("account usage rollup");
    assert_eq!(global.cache_write_tokens, 24);
    assert_eq!(global.cache_read_tokens, 45);
    assert_eq!(global.output_tokens, 66);
    assert_eq!(global.models.len(), 1);
    assert_eq!(global.models[0].model, "gpt-5");
    assert_eq!(global.models[0].reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        global.models[0]
            .costs
            .as_ref()
            .expect("global rollup costs")
            .input,
        1.5
    );
    assert_eq!(account.cache_write_tokens, 20);
    assert_eq!(
        account.models[0]
            .costs
            .as_ref()
            .expect("rollup costs")
            .input,
        1.25
    );
}

#[tokio::test]
async fn summary_projection_usage_rollup_rejects_oversized_text_before_fetch() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let bucket = align_bucket_epoch(
        (Utc::now() - ChronoDuration::hours(2)).timestamp(),
        3_600,
        0,
    );
    let oversized_model = "🙂".repeat((SUMMARY_PROJECTION_MAX_ROLLUP_BYTES / 4) + 1);
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
             (bucket_start_epoch, source, upstream_account_key, upstream_account_id, \
              normalized_model, normalized_reasoning_effort, request_count, \
              cache_write_tokens, cache_read_tokens, output_tokens, cost_input, has_cost) \
             VALUES (?1, 'proxy', 'none', NULL, ?2, 'high', 1, 0, 0, 0, 0.0, 1)",
    )
    .bind(bucket)
    .bind(oversized_model)
    .execute(&state.pool)
    .await
    .expect("insert oversized usage rollup");

    let error = load_summary_projection_rollup_usage(&state.pool)
        .await
        .expect_err("oversized usage text must fail closed before fetch_all");
    assert!(
        error
            .to_string()
            .contains("usage rollup byte budget exceeded"),
        "oversized usage rows must be rejected by the preflight budget: {error:#}"
    );
}

#[tokio::test]
async fn summary_projection_pages_live_source_text_beyond_resident_preview_budget() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let payload_value = "x".repeat(SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES / 2 - 1_024);
    let payload = format!(r#"{{"model":"{payload_value}"}}"#);
    let occurred_at = crate::stats::db_occurred_at_lower_bound(Utc::now());
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('paged-summary-live-1', ?1, 'proxy', 'success', 1, 0.1, ?3, '', 'full'),
                    ('paged-summary-live-2', ?2, 'proxy', 'success', 2, 0.2, ?3, '', 'full')",
        )
        .bind(&occurred_at)
        .bind(&occurred_at)
        .bind(&payload)
        .execute(&state.pool)
        .await
        .expect("insert paged live payloads");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("source pages may exceed one resident preview budget");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("source pages may reconcile all-time coverage after bootstrap");
    assert!(state.subscription_hub.summary_projection().await.is_some());
    state.pool.close().await;

    for (window, limit) in [
        ("current", Some(50)),
        ("1d", None),
        ("7d", None),
        ("30d", None),
        ("today", None),
        ("all", None),
    ] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit,
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("summary must remain exact from the published projection");
        assert_eq!(response.total_count, 2, "{window} total count");
        assert_eq!(response.total_tokens, 3, "{window} total tokens");
    }
}

#[tokio::test]
async fn summary_projection_localizes_oversized_live_source_record() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let now = format_utc_iso(Utc::now());
    let oversized_plan_type = "p".repeat(SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES + 1);
    sqlx::query(
            "INSERT INTO pool_upstream_accounts \
             (id, kind, provider, display_name, status, enabled, plan_type, created_at, updated_at) \
             VALUES (987654, 'api_key_codex', 'codex', 'oversized plan account', 'active', 1, ?1, ?2, ?2)",
        )
        .bind(oversized_plan_type)
        .bind(&now)
        .execute(&state.pool)
        .await
        .expect("insert oversized plan type account");
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('oversized-summary-plan', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{\"upstreamAccountId\":987654}', '', 'full'),
                    ('safe-summary-plan', datetime('now', '-1 minute'), 'proxy', 'success', 2, 0.2, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert oversized plan type live row");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("oversized source record must become a local admission gap");
    assert!(state.subscription_hub.summary_projection().await.is_some());
    state.pool.close().await;

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
    .expect("current prefix below the unproven rank remains exact");
    assert_eq!(current.total_count, 1);
    let account_error = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(987654),
        }),
    )
    .await
    .expect_err("the account current prefix reaches its first unproven rank");
    assert!(matches!(account_error, ApiError::Unavailable(_)));
    let error = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("a rolling range intersecting the admission gap must be unavailable");
    assert!(matches!(error, ApiError::Unavailable(_)));
}

#[tokio::test]
async fn summary_projection_rejects_oversized_archive_payload_before_preview_fetch() {
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory archive pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (\
             id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
             source TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, \
             cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, \
             cost_input REAL, cost_cache_write REAL, cost_cache_read REAL, cost_output REAL, \
             cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT)",
        )
        .execute(&archive_pool)
        .await
        .expect("create archive payload fixture");
    let oversized_payload = "x".repeat(SUMMARY_PROJECTION_MAX_PREVIEW_BYTES + 1);
    sqlx::query(
            "INSERT INTO codex_invocations \
             (id, invoke_id, occurred_at, source, payload, total_tokens, cost, status) \
             VALUES (1, 'oversized-summary-archive', datetime('now', '+8 hours', '-1 minute'), 'proxy', ?1, 1, 0.1, 'success')",
        )
        .bind(oversized_payload)
        .execute(&archive_pool)
        .await
        .expect("insert oversized archive payload");
    let columns = [
        ("source", true),
        ("model", true),
        ("payload", true),
        ("status", true),
        ("error_message", true),
        ("failure_kind", true),
        ("failure_class", true),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let range = ExactUtcRange {
        start: Utc::now() - ChronoDuration::hours(2),
        end: Utc::now() + ChronoDuration::minutes(1),
    };
    let error = ensure_summary_projection_archive_text_budget(
        &archive_pool,
        &[range],
        &columns,
        SUMMARY_PROJECTION_MAX_PREVIEW_BYTES,
    )
    .await
    .expect_err("oversized archive payload must fail before preview materialization");
    assert!(
        error
            .to_string()
            .contains("archive byte budget exceeded before preview fetch"),
        "archive byte preflight must reject oversized payloads: {error:#}"
    );
}

#[tokio::test]
async fn summary_projection_live_aggregate_admission_fails_closed_at_its_bound() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES \
             ('aggregate-budget-summary-1', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{}', '', 'full'), \
             ('aggregate-budget-summary-2', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{}', '', 'full'), \
             ('aggregate-budget-summary-3', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert live aggregate budget fixture");

    let admission =
        summary_projection_live_history_within_aggregate_budget_with_limit(&state.pool, 2)
            .await
            .expect("aggregate admission probe must complete");
    assert!(admission.is_none(), "aggregate overflow must fail closed");
}

#[tokio::test]
async fn summary_projection_live_aggregate_fence_excludes_post_admission_rows() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('aggregate-fence-before', datetime('now', '-3 days'), 'proxy', 'success', 3, 0.3, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert pre-admission live row");
    let admission = summary_projection_live_history_within_aggregate_budget(&state.pool)
        .await
        .expect("admission must succeed")
        .expect("small live history must be admitted");

    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('aggregate-fence-after', datetime('now', '-3 days'), 'proxy', 'success', 7, 0.7, '{\"upstreamAccountId\":43}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert post-admission live row");

    let global = StatsTotals::from(
        crate::stats::query_stats_row_through_id(
            &state.pool,
            InvocationSourceScope::All,
            admission.upper_bound_id,
        )
        .await
        .expect("bounded global aggregate"),
    );
    assert_eq!(global.total_count, 1);
    assert_eq!(global.total_tokens, 3);
    let accounts =
        load_summary_projection_live_account_totals(&state.pool, admission.upper_bound_id)
            .await
            .expect("bounded account aggregate");
    assert_eq!(accounts.get(&42).map(|totals| totals.total_count), Some(1));
    assert_eq!(accounts.get(&43).map(|totals| totals.total_count), None);
}

#[test]
fn summary_projection_resident_record_views_share_the_canonical_byte_budget() {
    let half_budget = SUMMARY_PROJECTION_MAX_PREVIEW_BYTES / 2;
    ensure_summary_projection_resident_record_bytes(half_budget, half_budget)
        .expect("the two resident views may exactly fill their shared budget");

    let error = ensure_summary_projection_resident_record_bytes(half_budget + 1, half_budget)
        .expect_err("two independently sized resident views must not exceed one budget");
    assert!(
        error
            .to_string()
            .contains("resident record bytes exceeded bounded budget"),
        "the shared resident budget must reject combined rolling/current ownership: {error:#}"
    );
}

#[test]
fn summary_projection_live_source_admission_packs_bounded_pages_and_gaps_records() {
    let candidate =
        |id: i64, source_bytes: usize, global_rank: usize| SummaryProjectionLiveCandidate {
            id,
            occurred_at: format!("2026-01-01 00:00:{id:02}"),
            upstream_account_id: Some(42),
            source_bytes,
            global_rank,
            account_rank: global_rank,
        };
    let (pages, gaps, overflow) = pack_summary_projection_live_candidates(
        vec![
            candidate(1, SUMMARY_PROJECTION_MAX_SOURCE_PAGE_BYTES / 2 + 1, 1),
            candidate(2, SUMMARY_PROJECTION_MAX_SOURCE_PAGE_BYTES / 2 + 1, 2),
            candidate(3, SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES + 1, 3),
        ],
        4,
    );
    assert_eq!(
        pages.len(),
        2,
        "two admitted records require two source pages"
    );
    assert_eq!(pages.iter().map(Vec::len).sum::<usize>(), 2);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].global_rank, 3);
    assert!(overflow.is_none());
}

#[tokio::test]
async fn summary_projection_preview_hydration_uses_the_narrow_projection() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, model, input_tokens, output_tokens, \
              cache_input_tokens, total_tokens, cost, cost_input, cost_cache_write, \
              cost_cache_read, cost_output, cost_reasoning, payload, raw_response, detail_level) \
             VALUES \
             ('summary-narrow-preview', datetime('now', '+8 hours'), 'proxy', 'success', 'stored-model', \
              10, 20, 3, 30, 0.3, 0.1, 0.02, 0.03, 0.1, 0.05, \
              '{\"upstreamAccountId\":321,\"responseModel\":\"response-model\",\"reasoningEffort\":\"high\",\"failureKind\":\"upstream\",\"promptCacheKey\":\"not-needed\",\"routeMode\":\"sticky\",\"endpoint\":\"/not-needed\"}', \
              '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert summary preview fixture");
    let high_watermark = load_summary_projection_live_high_watermark(&state.pool)
        .await
        .expect("load summary preview high watermark");
    let admission =
        query_summary_projection_live_rows_with_budget(SummaryProjectionLiveRowsQuery {
            pool: &state.pool,
            source_scope: InvocationSourceScope::All,
            range: ExactUtcRange {
                start: Utc::now() - ChronoDuration::minutes(1),
                end: Utc::now() + ChronoDuration::minutes(1),
            },
            high_watermark_id: high_watermark,
            min_id_exclusive: None,
            upstream_account_id: None,
            limit: 2,
            in_progress_only: false,
            preview_cache: &mut HashMap::new(),
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "summary_projection_test",
                builder: "narrow_preview",
                purpose: "narrow_preview_fixture",
            },
        })
        .await
        .expect("hydrate summary preview");

    assert_eq!(admission.rows.len(), 1);
    let row = &admission.rows[0];
    assert_eq!(row.upstream_account_id, Some(321));
    assert_eq!(row.model.as_deref(), Some("stored-model"));
    assert_eq!(row.response_model.as_deref(), Some("response-model"));
    assert_eq!(row.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(row.failure_kind.as_deref(), Some("upstream"));
    assert_eq!(row.total_tokens, 30);
    assert_eq!(row.input_tokens, Some(10));
    assert_eq!(row.output_tokens, Some(20));
    assert_eq!(row.cache_input_tokens, Some(3));
    assert!(row.prompt_cache_key.is_none());
    assert!(row.route_mode.is_none());
    assert!(row.endpoint.is_none());
    assert!(row.upstream_account_name.is_none());
}

#[test]
fn summary_projection_resident_byte_overflow_is_local_to_the_affected_range() {
    let now = Utc::now();
    let affected_range = ExactUtcRange {
        start: now - ChronoDuration::days(3),
        end: now - ChronoDuration::days(3) + ChronoDuration::hours(1),
    };
    let error = ensure_summary_projection_resident_record_bytes(
        SUMMARY_PROJECTION_MAX_PREVIEW_BYTES / 2 + 1,
        SUMMARY_PROJECTION_MAX_PREVIEW_BYTES / 2,
    )
    .expect_err("combined resident views must reject the overflowing boundary");
    assert!(summary_projection_resident_record_budget_exceeded(&error));

    let mut unavailable_buckets = BTreeSet::new();
    summary_projection_mark_unavailable_archive_ranges(&mut unavailable_buckets, [affected_range])
        .expect("one overflowed boundary remains representable as a local unavailable range");
    let projection = SummaryProjection {
        refreshed_at: Some(Instant::now()),
        unavailable_exact_live_ranges: summary_projection_unavailable_bucket_ranges(
            unavailable_buckets,
        ),
        ..SummaryProjection::default()
    };

    let current = projection.response_for_query(
        &SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        },
        100,
    );
    assert!(
        current.is_ok(),
        "current must not inherit a rolling-only overflow"
    );

    let affected = projection.response_for_query(
        &SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        },
        100,
    );
    assert!(matches!(affected, Err(ApiError::Unavailable(_))));

    let unaffected = projection.response_for_query(
        &SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        },
        100,
    );
    assert!(
        unaffected.is_ok(),
        "an independent legal range remains available"
    );
}
