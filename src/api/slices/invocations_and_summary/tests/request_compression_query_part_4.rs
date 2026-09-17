#[tokio::test]
async fn summary_projection_localizes_historical_live_record_count_overflow() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_occurred_at =
        crate::stats::db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(21));
    // The old per-record preflight treated aggregate cardinality across the full historical
    // horizon as one global admission failure. Keep the source confined to a disjoint hour:
    // current and 7d must still be exact without SQLite after publication.
    for batch in 0..51 {
        sqlx::query(
                "WITH RECURSIVE rows(value) AS ( \
                     SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1 \
                 ) \
                 INSERT INTO codex_invocations \
                 (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
                 SELECT ?2 || value, ?3, 'proxy', 'success', 1, 0.1, '{}', '', 'full' FROM rows",
            )
            .bind(1_000_i64)
            .bind(format!("summary-historical-overflow-{batch}-"))
            .bind(&historical_occurred_at)
            .execute(&state.pool)
            .await
            .expect("seed historical terminal batch");
    }
    let current_occurred_at = db_occurred_at_lower_bound(Utc::now());
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-historical-overflow-current', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .bind(current_occurred_at)
        .execute(&state.pool)
        .await
        .expect("seed current terminal");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("disjoint historical cardinality must not abort bootstrap publication");
    state.pool.close().await;

    for window in ["current", "1d", "7d", "today"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("disjoint selection remains exact from the in-memory projection");
        let expected_count = if window == "current" { 50 } else { 1 };
        let expected_tokens = if window == "current" { 66 } else { 17 };
        assert_eq!(response.total_count, expected_count, "{window} total count");
        assert_eq!(
            response.total_tokens, expected_tokens,
            "{window} total tokens"
        );
    }

    let affected = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("30d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(affected, Err(ApiError::Unavailable(_))));
}

async fn seed_summary_projection_account_cursor_fixture() -> (Arc<AppState>, i64) {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, input_tokens, output_tokens, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES (1, 'summary-live-scope-coverage', datetime('now', '-2 hours', '-10 minutes'), 'proxy', 'success', 3, 14, 17, 1.25, '{\"upstreamAccountId\":42,\"responseModel\":\"gpt-5\",\"reasoningEffort\":\"high\"}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert persisted live row");
    let occurred_at: String = sqlx::query_scalar(
        "SELECT occurred_at FROM codex_invocations WHERE invoke_id = 'summary-live-scope-coverage'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load live row timestamp");
    let bucket = align_bucket_epoch(
        parse_to_utc_datetime(&occurred_at)
            .expect("parse live row timestamp")
            .timestamp(),
        3_600,
        0,
    );
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert global live rollup");
    sqlx::query(
            "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) VALUES ('codex_invocations_summary_rollup_v2_live_cursor', 1, datetime('now')) ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = datetime('now')",
        )
        .execute(&state.pool)
        .await
        .expect("advance global live rollup cursor");
    (state, bucket)
}

async fn assert_summary_projection_account_cursor_before_sqlite_close(state: &Arc<AppState>) {
    let Json(global_before) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve global projection");
    assert_eq!(global_before.total_count, 1);
    assert_eq!(global_before.total_tokens, 17);

    let Json(account_before) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve account exact fallback");
    assert_eq!(account_before.total_count, 1);
    assert_eq!(account_before.total_tokens, 17);
    let account_usage = account_before
        .usage_breakdown
        .as_ref()
        .expect("account exact usage");
    assert_eq!(account_usage.models[0].model, "gpt-5");
    assert_eq!(account_usage.models[0].output_tokens, 14);

    let Json(account_all_time_before) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve live-only account all-time projection");
    assert_eq!(account_all_time_before.total_count, 1);
    assert_eq!(account_all_time_before.total_tokens, 17);
}

async fn seed_summary_projection_account_rollups(state: &Arc<AppState>, bucket: i64) {
    sqlx::query(
            "INSERT INTO upstream_account_stats_hourly (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', 42, 0, 0, 0, 0, 0, 0)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert account totals rollup");
    sqlx::query(
            "INSERT INTO upstream_account_usage_breakdown_hourly (bucket_start_epoch, source, upstream_account_key, upstream_account_id, normalized_model, normalized_reasoning_effort, request_count, cache_write_tokens, cache_read_tokens, output_tokens, cost_input, has_cost) VALUES (?1, 'proxy', '42', 42, 'gpt-5', 'high', 0, 0, 0, 0, 0, 1)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert account usage rollup");
}

async fn assert_summary_projection_account_cursor_after_sqlite_close(state: Arc<AppState>) {
    let Json(account_after) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve account exact projection without sqlite");
    assert_eq!(account_after.total_count, 1);
    assert_eq!(account_after.total_tokens, 17);
    let account_usage = account_after
        .usage_breakdown
        .as_ref()
        .expect("account exact usage without an account cursor");
    assert_eq!(account_usage.models.len(), 1);
    assert_eq!(account_usage.models[0].model, "gpt-5");
    assert_eq!(account_usage.models[0].output_tokens, 14);
}

#[tokio::test]
async fn summary_projection_requires_account_cursor_for_persisted_live_coverage() {
    let (state, bucket) = seed_summary_projection_account_cursor_fixture().await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate scope-aware projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile the live-only account all-time projection");
    assert_summary_projection_account_cursor_before_sqlite_close(&state).await;

    seed_summary_projection_account_rollups(&state, bucket).await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("rehydrate without account cursor coverage");
    state.pool.close().await;
    assert_summary_projection_account_cursor_after_sqlite_close(state).await;
}

#[tokio::test]
async fn summary_handler_rejects_expired_projection_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let query = SummaryQuery {
        window: Some("1d".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    Arc::make_mut(&mut projection).refreshed_at =
        Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state
        .subscription_hub
        .store_summary_projection(Arc::unwrap_or_clone(projection))
        .await;
    state.pool.close().await;

    let response = fetch_summary(State(state), Query(query)).await;
    assert!(matches!(response, Err(ApiError::Unavailable(_))));
}

#[tokio::test]
async fn summary_handler_rejects_expired_all_time_snapshot_after_rolling_refresh() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let query = SummaryQuery {
        window: Some("all".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    let projection_ref = Arc::make_mut(&mut projection);
    projection_ref.refreshed_at = Some(Instant::now());
    projection_ref.all_time_refreshed_at =
        Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state
        .subscription_hub
        .store_summary_projection(Arc::unwrap_or_clone(projection))
        .await;
    state.pool.close().await;

    let response = fetch_summary(State(state), Query(query)).await;
    assert!(matches!(response, Err(ApiError::Unavailable(_))));
}

#[tokio::test]
async fn summary_handler_preserves_all_time_last_good_across_rolling_only_refresh() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::Rolling)
        .await
        .expect("rolling-only refresh preserves prior all-time projection");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("exact all-time last-good remains available after rolling refresh");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
}

#[tokio::test]
async fn summary_handler_keeps_global_all_time_freshness_separate_from_accounts() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    let projection_ref = Arc::make_mut(&mut projection);
    projection_ref.all_time_refreshed_at = Some(Instant::now());
    projection_ref.all_time_account_refreshed_at.insert(
        42,
        Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1),
    );
    state
        .subscription_hub
        .store_summary_projection(Arc::unwrap_or_clone(projection))
        .await;
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
    .expect("fresh global all-time response");
    assert_eq!(global.total_count, 1);

    let account = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(matches!(account, Err(ApiError::Unavailable(_))));
}

#[test]
fn summary_projection_exact_horizon_covers_longest_named_calendar_window() {
    let end = Utc
        .with_ymd_and_hms(2026, 8, 31, 23, 59, 55)
        .single()
        .expect("valid deterministic projection boundary time");
    let horizon_start = end - summary_projection_exact_horizon(0);

    for timezone in TZ_VARIANTS {
        let (start, _) = named_range_bounds("thisMonth", end, timezone)
            .expect("thisMonth range must be valid for every IANA timezone");
        assert!(
            start >= horizon_start,
            "{timezone} thisMonth start must remain inside the bounded exact horizon"
        );
    }
}

#[test]
fn summary_projection_reuses_cached_legacy_archive_coverage_for_all_time() {
    let end = Utc
        .with_ymd_and_hms(2026, 8, 31, 23, 59, 55)
        .single()
        .expect("valid deterministic projection boundary time");
    let horizon = ExactUtcRange {
        start: end - ChronoDuration::days(31),
        end,
    };
    let archive = crate::stats::ArchiveBatchPathRow::from_file_path("legacy-summary.sqlite");
    let cached = HashMap::from([(
        "legacy-summary.sqlite".to_string(),
        ExactUtcRange {
            start: end - ChronoDuration::days(2),
            end: end - ChronoDuration::days(1),
        },
    )]);

    assert!(!summary_projection_archive_has_exact_all_time_source(
        &archive,
        &HashMap::new(),
        horizon,
    ));
    assert!(summary_projection_archive_has_exact_all_time_source(
        &archive, &cached, horizon,
    ));
}

#[test]
fn summary_projection_overflowed_recent_account_index_is_unavailable() {
    let projection = SummaryProjection {
        refreshed_at: Some(Instant::now()),
        recent_index_complete: false,
        ..SummaryProjection::default()
    };
    let error = projection
        .response_for_query(
            &SummaryQuery {
                window: Some("current".to_string()),
                limit: Some(1),
                time_zone: Some("UTC".to_string()),
                upstream_account_id: Some(42),
            },
            100,
        )
        .expect_err("bounded current index cannot silently become an empty account response");
    assert!(matches!(error, ApiError::Unavailable(_)));
}

#[tokio::test]
async fn summary_handler_after_refresh_failure_does_not_require_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let query = SummaryQuery {
        window: Some("1d".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    state.pool.close().await;

    assert!(refresh_summary_snapshots(state.as_ref()).await.is_err());
    let Json(response) = fetch_summary(State(state), Query(query))
        .await
        .expect("serve in-memory summary response after refresh failure");

    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
}

#[tokio::test]
async fn summary_live_tail_aggregate_stays_within_its_admitted_id_snapshot() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    for id in [1_i64] {
        sqlx::query(
                "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
                 VALUES (?1, ?2, datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
            )
            .bind(id)
            .bind(format!("bounded-live-tail-{id}"))
            .execute(&state.pool)
            .await
            .expect("insert admitted live tail row");
    }
    let admitted = crate::stats::load_live_invocation_ids_after_id_bounded_snapshot(
        &state.pool,
        InvocationSourceScope::All,
        0,
        summary_projection_exact_record_limit(),
    )
    .await
    .expect("admit bounded live tail");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES (2, 'post-admission-live-tail', datetime('now'), 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert concurrent post-admission row");

    let totals = crate::stats::query_live_invocation_totals_after_id(
        &state.pool,
        InvocationSourceScope::All,
        0,
        admitted.upper_bound_id,
    )
    .await
    .expect("aggregate only admitted live tail ids");
    assert_eq!(admitted.ids, HashSet::from([1]));
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.total_tokens, 17);
}

#[tokio::test]
async fn summary_account_live_tail_aggregate_stays_within_its_admitted_id_snapshot() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            r#"INSERT INTO codex_invocations
               (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
               VALUES (1, 'bounded-account-live-tail-1', datetime('now'), 'proxy', 'success', 17, 1.25, '{"upstreamAccountId":42}', '', 'full')"#,
        )
        .execute(&state.pool)
        .await
        .expect("insert admitted account live tail row");

    let admitted = admit_summary_projection_live_tail_account_ids(&state.pool, Some(0))
        .await
        .expect("admit bounded account live tail")
        .expect("account cursor admits a live tail");
    sqlx::query(
            r#"INSERT INTO codex_invocations
               (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
               VALUES (2, 'post-admission-account-live-tail', datetime('now'), 'proxy', 'success', 23, 2.5, '{"upstreamAccountId":42}', '', 'full')"#,
        )
        .execute(&state.pool)
        .await
        .expect("insert concurrent post-admission account row");

    let totals = load_summary_projection_live_tail_account_totals(
        &state.pool,
        Some(0),
        Some(admitted.upper_bound_id),
    )
    .await
    .expect("aggregate only admitted account live tail rows");
    assert_eq!(totals[&42].total_count, 1);
    assert_eq!(totals[&42].total_tokens, 17);
    assert_eq!(totals[&42].total_cost, 1.25);
}

#[tokio::test]
async fn summary_live_only_all_time_does_not_add_account_tail_twice() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            r#"INSERT INTO codex_invocations
               (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
               VALUES (1, 'live-only-account-tail', datetime('now'), 'proxy', 'success', 17, 1.25, '{"upstreamAccountId":42}', '', 'full')"#,
        )
        .execute(&state.pool)
        .await
        .expect("insert live-only account row");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
             VALUES ('invocation_account_activity_v2_repair_live_cursor', 0, datetime('now'))",
    )
    .execute(&state.pool)
    .await
    .expect("record independent account cursor");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate live-only all-time projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("finalize exact live-only all-time projection");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve live-only account projection without SQLite");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
    assert_eq!(response.total_cost, 1.25);
}

#[tokio::test]
async fn summary_handler_rejects_an_expired_projection_after_idle_cadence() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    Arc::make_mut(&mut projection).refreshed_at =
        Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state
        .subscription_hub
        .store_summary_projection(Arc::unwrap_or_clone(projection))
        .await;
    state.pool.close().await;

    let error = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect_err("expired projection must not be revived by idle cadence");
    assert!(matches!(error, ApiError::Unavailable(_)));
}

async fn seed_summary_projection_materialized_hourly_rollup_fixture(state: &Arc<AppState>) -> i64 {
    let bucket = align_bucket_epoch(
        (Utc::now() - ChronoDuration::days(31)).timestamp(),
        3_600,
        0,
    );
    let coverage_start = Utc
        .timestamp_opt(bucket, 0)
        .single()
        .expect("valid archive coverage start")
        .to_rfc3339();
    let coverage_end = coverage_start.clone();
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, 'proxy', 3, 2, 1, 91, 4.5, 1.5)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert materialized hourly rollup");
    sqlx::query(
            r#"INSERT INTO upstream_account_stats_hourly (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost)
               VALUES (?1, 'proxy', 42, 3, 2, 1, 91, 4.5, 1.5)"#,
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert account rollup for global dedup regression");
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
                cache_write_tokens,
                cache_read_tokens,
                output_tokens,
                cost_input,
                has_cost
            ) VALUES (?1, 'proxy', '42', 42, 'gpt-5', 'high', 3, 0, 0, 91, 4.5, 1)
            "#,
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert account usage rollup for unavailable archive");
    sqlx::query(
        "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, \
              coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at) \
             VALUES ('codex_invocations', '2026-01', '/definitely/missing/summary.sqlite.gz', \
                     'summary-test', 3, 'completed', ?1, ?2, datetime('now'), datetime('now'))",
    )
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&state.pool)
    .await
    .expect("insert unavailable materialized archive manifest");
    bucket
}

#[tokio::test]
async fn summary_projection_uses_materialized_hourly_rollup_when_archive_is_unavailable() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let bucket = seed_summary_projection_materialized_hourly_rollup_fixture(&state).await;
    let usage = load_summary_projection_rollup_usage(&state.pool)
        .await
        .expect("load materialized usage rollup");
    assert!(
        usage.contains_key(&(bucket, None)),
        "materialized account usage must contribute to the global projection scope"
    );
    assert!(
        usage.contains_key(&(bucket, Some(42))),
        "materialized account usage must remain available to the account projection scope"
    );
    sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', '/definitely/missing/summary.sqlite.gz', 'summary-test')",
        )
        .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
        .execute(&state.pool)
        .await
        .expect("mark unavailable archive global scope as replayed");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with incomplete materialized replay metadata");
    let incomplete = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection with incomplete replay metadata");
    assert_eq!(
        incomplete.unavailable_unmaterialized_archive_ranges.len(),
        1,
        "materialization alone cannot prove account or usage dimensions"
    );
    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
                "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', '/definitely/missing/summary.sqlite.gz', 'summary-test')",
            )
            .bind(target)
            .execute(&state.pool)
            .await
            .expect("mark unavailable archive account scope as replayed");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection from fully replayed materialized rollup");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile exact all-time projection from fully replayed materialized rollup");
    let expected = query_hourly_backed_summary_range(
        state.as_ref(),
        Utc::now() - ChronoDuration::days(40),
        Utc::now(),
        InvocationSourceScope::All,
    )
    .await
    .expect("load legacy rollup-backed summary");
    state.pool.close().await;

    let Json(actual) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve rollup-backed projection without sqlite");
    assert_eq!(actual.total_count, expected.total_count);
    assert_eq!(actual.success_count, expected.success_count);
    assert_eq!(actual.failure_count, expected.failure_count);
    assert_eq!(actual.total_tokens, expected.total_tokens);
    assert_eq!(actual.total_cost, expected.total_cost);
}
