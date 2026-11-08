use super::*;

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

#[tokio::test]
async fn summary_projection_uses_materialized_hourly_rollup_when_archive_is_unavailable() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
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

#[tokio::test]
async fn summary_projection_all_time_rejects_archive_account_with_missing_compact_scope() {
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

    // A durable bucket key is not proof of a completed archive replay. This archive is
    // outside the finite raw horizon, so projection hydration remains available while the
    // all-time selection itself preserves the established unavailable contract.
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with incomplete archive replay");
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

    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
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

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate all-time projection from durable global rollup");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile all-time projection from durable global rollup");

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

    // The archive manifest lists only account 42. Until its refresh marker is committed,
    // account 43 could still be present in the archive and therefore cannot receive a
    // freshly rebuilt zero-valued all-time summary.
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

    // Account stats replay alone does not prove the usage/cost dimensions of an all-time
    // response. Removing that marker must invalidate the account scope instead of publishing
    // a structurally incomplete success response.
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
    let admission = query_summary_projection_live_rows_with_budget(
        &state.pool,
        InvocationSourceScope::All,
        ExactUtcRange {
            start: Utc::now() - ChronoDuration::minutes(1),
            end: Utc::now() + ChronoDuration::minutes(1),
        },
        high_watermark,
        None,
        None,
        2,
        false,
        SUMMARY_PROJECTION_MAX_PREVIEW_BYTES,
        &mut HashMap::new(),
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "summary_projection_test",
            builder: "narrow_preview",
            purpose: "narrow_preview_fixture",
        },
    )
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

#[tokio::test]
async fn summary_projection_current_archive_cutoff_is_request_specific() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES \
         ('summary-current-newest-live', ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full'), \
         ('summary-current-older-live', ?2, 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(now))
    .bind(db_occurred_at_lower_bound(now - ChronoDuration::hours(2)))
    .execute(&state.pool)
    .await
    .expect("seed ordered resident current rows");
    let archive_at = format_naive(
        (now - ChronoDuration::hours(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-current-archive-cutoff",
        &[(
            91_i64,
            "summary-current-archive-boundary",
            archive_at.as_str(),
            SOURCE_PROXY,
            "success",
            1_i64,
            0.1_f64,
            None,
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2 \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind((now - ChronoDuration::hours(1)).to_rfc3339())
    .bind((now - ChronoDuration::minutes(59)).to_rfc3339())
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("record current archive coverage bounds");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate resident current projection with an archived boundary");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("resident current projection");
    assert!(
        !projection.current_source_unavailable,
        "small fixture must not exhaust the current resident budget"
    );
    assert_eq!(projection.recent_indexes[&None].len(), 2);
    assert!(
        !projection.current_archive_may_affect_global_current(1),
        "archive coverage before the newest cutoff must not reject limit=1: latest={:?} cutoff={:?} exceeded={} unknown={}",
        projection.current_archive_latest_coverage_end,
        projection.current_selection_cutoff(1),
        projection.current_archive_admission_exceeded,
        projection.current_archive_has_unknown_coverage,
    );
    assert!(
        projection.current_archive_may_affect_global_current(2),
        "archive coverage reaching the second newest cutoff must reject limit=2"
    );
    state.pool.close().await;

    let Json(small) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("an archive below the newest one-row cutoff must not reject current");
    assert_eq!(small.total_count, 1);

    let large = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(2),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(large, Err(ApiError::Unavailable(_))));

    let account = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(matches!(account, Err(ApiError::Unavailable(_))));
}

#[tokio::test]
async fn summary_projection_current_uses_exact_unreadable_archive_endpoint_before_cutoff() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let current_hour = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("valid current hour");
    let archive_start = current_hour - ChronoDuration::hours(1) + ChronoDuration::minutes(10);
    let archive_end = current_hour - ChronoDuration::hours(1) + ChronoDuration::minutes(20);
    let selected_current = current_hour - ChronoDuration::hours(1) + ChronoDuration::minutes(50);
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-current-after-unreadable-archive', ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(selected_current))
    .execute(&state.pool)
    .await
    .expect("seed selected current record after unreadable archive endpoint");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at) \
         VALUES ('codex_invocations', '2026-01', '/definitely/missing/summary-current-same-hour.sqlite.gz', \
                 'summary-current-same-hour', 1, 'completed', ?1, ?2, datetime('now'))",
    )
    .bind(db_occurred_at_lower_bound(archive_start))
    .bind(db_occurred_at_lower_bound(archive_end))
    .execute(&state.pool)
    .await
    .expect("seed unreadable archive before selected current cutoff in the same hour");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with same-hour unreadable archive metadata");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert!(
        !projection.unavailable_archive_may_affect_global_current(1),
        "the precise archive endpoint must remain below the selected current cutoff"
    );
    state.pool.close().await;

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("closed SQLite current request must use the in-memory exact cutoff proof");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 1);
}

#[tokio::test]
async fn summary_projection_current_ignores_unreadable_archive_ending_at_cutoff() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let current_hour = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("valid current hour");
    // Keep the selected row before hydration's `Utc::now()` upper bound so this endpoint
    // proof does not depend on which minute of the current hour the test starts.
    let selected_current = current_hour - ChronoDuration::minutes(20);
    let archive_start = selected_current - ChronoDuration::minutes(10);
    // Manifest coverage is inclusive, so this becomes the exact half-open endpoint used by
    // the resident-current proof.
    let archive_end_inclusive = selected_current - ChronoDuration::seconds(1);
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-current-after-adjacent-unreadable-archive', ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(selected_current))
    .execute(&state.pool)
    .await
    .expect("seed selected current record at the half-open archive endpoint");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at) \
         VALUES ('codex_invocations', '2026-01', '/definitely/missing/summary-current-adjacent.sqlite.gz', \
                 'summary-current-adjacent', 1, 'completed', ?1, ?2, datetime('now'))",
    )
    .bind(db_occurred_at_lower_bound(archive_start))
    .bind(db_occurred_at_lower_bound(archive_end_inclusive))
    .execute(&state.pool)
    .await
    .expect("seed unreadable archive adjacent to the selected current cutoff");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate current projection with adjacent unreadable archive metadata");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert!(
        !projection.unavailable_archive_may_affect_global_current(1),
        "a half-open archive ending at the cutoff cannot affect the selected rank"
    );
    assert!(
        !projection.current_archive_may_affect_global_current(1),
        "an unrepresented archive whose exclusive end equals the cutoff cannot reject current"
    );
    state.pool.close().await;

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("closed-pool current request must use the half-open cutoff proof");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 1);
}

#[tokio::test]
async fn summary_projection_current_fails_closed_when_historical_materialized_archive_can_fill_missing_rank()
 {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-current-resident-only', ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(now))
    .execute(&state.pool)
    .await
    .expect("seed one resident current row");
    let archive_endpoint = now - ChronoDuration::days(3);
    let archive_at = format_naive(archive_endpoint.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-current-historical-materialized",
        &[(
            101_i64,
            "summary-current-historical-materialized",
            archive_at.as_str(),
            SOURCE_PROXY,
            "success",
            7_i64,
            0.7_f64,
            None,
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2, \
             historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(db_occurred_at_lower_bound(
        archive_endpoint - ChronoDuration::minutes(1),
    ))
    .bind(db_occurred_at_lower_bound(archive_endpoint))
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark historical archive materialized with bounded coverage");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with historical materialized archive");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert_eq!(projection.recent_indexes[&None].len(), 1);
    assert!(
        projection.current_archive_latest_coverage_end.is_some(),
        "an archive outside the current admission horizon remains unrepresented"
    );
    state.pool.close().await;

    let response = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(2),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(response, Err(ApiError::Unavailable(_))),
        "one resident candidate cannot prove that a historical materialized archive does not fill rank two"
    );
}

#[tokio::test]
async fn summary_projection_current_fails_closed_when_unreadable_archive_can_fill_missing_rank() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-current-unreadable-resident', ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(now))
    .execute(&state.pool)
    .await
    .expect("seed one resident current row");
    let archive_endpoint = now - ChronoDuration::days(3);
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at) \
         VALUES ('codex_invocations', '2026-01', '/definitely/missing/summary-current-unreadable.sqlite.gz', \
                 'summary-current-unreadable', 1, 'completed', ?1, ?2, datetime('now'))",
    )
    .bind(db_occurred_at_lower_bound(
        archive_endpoint - ChronoDuration::minutes(1),
    ))
    .bind(db_occurred_at_lower_bound(archive_endpoint))
    .execute(&state.pool)
    .await
    .expect("seed unreadable historical archive manifest");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish projection with unreadable historical archive metadata");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert_eq!(projection.recent_indexes[&None].len(), 1);
    assert!(
        !projection
            .unavailable_unmaterialized_archive_ranges
            .is_empty(),
        "unreadable archive must retain an in-memory unavailability proof"
    );
    state.pool.close().await;

    let response = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(2),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(response, Err(ApiError::Unavailable(_))),
        "one resident candidate cannot prove that an unreadable archive does not fill rank two"
    );
}

#[tokio::test]
async fn summary_projection_current_quiet_account_with_historical_archive_is_unavailable_without_sqlite()
 {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let archive_endpoint = Utc::now() - ChronoDuration::days(3);
    let archive_at = format_naive(archive_endpoint.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-current-quiet-account-history",
        &[SeedInvocationArchiveBatchRow {
            id: 102,
            invoke_id: "summary-current-quiet-account-history",
            occurred_at: archive_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 11,
            cost: 1.1,
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
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2 \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(db_occurred_at_lower_bound(
        archive_endpoint - ChronoDuration::minutes(1),
    ))
    .bind(db_occurred_at_lower_bound(archive_endpoint))
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("record quiet account archive coverage");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate projection with quiet account history");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    assert!(projection.current_account_source_unavailable);
    state.pool.close().await;

    let response = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(
        matches!(response, Err(ApiError::Unavailable(_))),
        "account current must not become an empty memory-only response when its only candidate is archived"
    );
}

#[tokio::test]
async fn summary_projection_current_includes_materialized_archive_endpoint_row() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let endpoint = Utc::now() - ChronoDuration::minutes(1);
    let older = endpoint - ChronoDuration::minutes(1);
    let older_at = format_naive(older.with_timezone(&Shanghai).naive_local());
    let endpoint_at = format_naive(endpoint.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-current-inclusive-endpoint",
        &[
            (
                103_i64,
                "summary-current-inclusive-endpoint-older",
                older_at.as_str(),
                SOURCE_PROXY,
                "success",
                5_i64,
                0.5_f64,
                None,
            ),
            (
                104_i64,
                "summary-current-inclusive-endpoint-newer",
                endpoint_at.as_str(),
                SOURCE_PROXY,
                "success",
                17_i64,
                1.7_f64,
                None,
            ),
        ],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2, \
             historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(db_occurred_at_lower_bound(older))
    .bind(db_occurred_at_lower_bound(endpoint))
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("record inclusive materialized archive bounds");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate materialized archive current candidates");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve inclusive endpoint current candidate without SQLite");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
    assert_eq!(response.total_cost, 1.7);
}

#[tokio::test]
async fn summary_projection_archive_merge_prefers_resident_persisted_live_id() {
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
         cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT, \
         failure_class TEXT)",
    )
    .execute(&archive_pool)
    .await
    .expect("create archive fixture table");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, payload, total_tokens, cost, status) \
         VALUES (7, 'archive-replay-copy', datetime('now', '-1 minute'), 'proxy', '{}', 9, 1.0, 'success')",
    )
    .execute(&archive_pool)
    .await
    .expect("insert archive replay copy");

    let live_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory live pool");
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&live_pool)
        .await
        .expect("create live fixture table");
    sqlx::query("INSERT INTO codex_invocations (id) VALUES (7)")
        .execute(&live_pool)
        .await
        .expect("insert richer persisted live id");

    let now = Utc::now();
    let archive_range = ExactUtcRange {
        start: now - ChronoDuration::hours(2),
        end: now + ChronoDuration::minutes(1),
    };
    let mut records = HashMap::new();
    records.insert(
        "live-authoritative-copy".to_string(),
        SummaryProjectionRecord {
            row: UpstreamAccountInvocationPreviewRow {
                upstream_account_id: None,
                id: 7,
                invoke_id: "live-authoritative-copy".to_string(),
                prompt_cache_key: None,
                occurred_at: now.to_rfc3339(),
                conversation_created_at: None,
                status: "success".to_string(),
                live_phase: None,
                failure_class: None,
                route_mode: None,
                model: None,
                request_model: None,
                response_model: None,
                total_tokens: 17,
                cost: Some(1.7),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                source: Some(SOURCE_PROXY.to_string()),
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                reasoning_effort: None,
                error_message: None,
                downstream_status_code: None,
                downstream_error_message: None,
                failure_kind: None,
                is_actionable: None,
                proxy_display_name: None,
                upstream_account_name: None,
                upstream_account_plan_type: None,
                response_content_encoding: None,
                request_compression_algorithm: None,
                transport: None,
                requested_service_tier: None,
                service_tier: None,
                billing_service_tier: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
                t_total_ms: None,
                endpoint: None,
                compaction_request_kind: None,
                compaction_response_kind: None,
                image_intent: None,
            },
            occurred_at: now,
            global_rollup_covered: false,
            account_rollup_covered: false,
            usage_global_rollup_covered: false,
            usage_account_rollup_covered: false,
            is_persisted_live_record: true,
            is_archive_record: false,
            archive_has_materialized_rollups: false,
            account_archive_totals_fallback_included: false,
        },
    );
    let mut budget = 0;
    let mut bytes = 0;
    merge_summary_projection_archive_records_with_coverage(
        &archive_pool,
        &live_pool,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        false,
        None,
        None,
        None,
        archive_range,
        &HashSet::new(),
        None,
        &HashSet::new(),
        &mut records,
        &mut budget,
        &mut bytes,
        0,
        None,
    )
    .await
    .expect("bounded persisted-id lookup");
    assert_eq!(records.len(), 1);
    assert!(records.contains_key("live-authoritative-copy"));
}

#[tokio::test]
async fn summary_projection_archive_merge_keeps_exact_global_row_without_overall_replay() {
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
         cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT, \
         failure_class TEXT)",
    )
    .execute(&archive_pool)
    .await
    .expect("create archive fixture table");
    let bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 3_600;
    let occurred_at = db_occurred_at_lower_bound(
        Utc.timestamp_opt(bucket + 300, 0)
            .single()
            .expect("valid archive row time"),
    );
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, payload, total_tokens, cost, status) \
         VALUES (8, 'archive-global-replay-gap', ?1, 'proxy', '{}', 9, 1.0, 'success')",
    )
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert archive replay row");
    let live_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory live pool");
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&live_pool)
        .await
        .expect("create live fixture table");
    let archive_range = ExactUtcRange {
        start: Utc
            .timestamp_opt(bucket, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .timestamp_opt(bucket + 3_600, 0)
            .single()
            .expect("valid range end"),
    };
    let mut totals = HashMap::new();
    totals.insert((bucket, None), StatsTotals::default());
    let mut usage = HashMap::new();
    usage.insert(
        (bucket, None),
        UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        },
    );
    let mut records = HashMap::new();
    let mut budget = 0;
    let mut bytes = 0;
    merge_summary_projection_archive_records_with_coverage(
        &archive_pool,
        &live_pool,
        &HashSet::new(),
        &totals,
        &usage,
        true,
        Some(false),
        Some(true),
        Some(true),
        archive_range,
        &HashSet::new(),
        None,
        &HashSet::new(),
        &mut records,
        &mut budget,
        &mut bytes,
        0,
        None,
    )
    .await
    .expect("merge exact archive row when overall replay is missing");
    let record = records
        .get("archive-global-replay-gap")
        .expect("missing overall replay keeps the exact archive row");
    assert!(!record.global_rollup_covered);
    assert_eq!(record.row.total_tokens, 9);
    assert!(
        summary_projection_all_time_uses_global_exact_record(record, 100),
        "a materialized raw fallback contributes when its global compact bucket is absent"
    );
    let mut account_record = record.clone();
    account_record.row.upstream_account_id = Some(42);
    account_record.account_rollup_covered = false;
    assert!(
        summary_projection_all_time_uses_account_exact_record(&account_record, Some(100)),
        "a missing account compact bucket retains the exact materialized source"
    );
    account_record.account_rollup_covered = true;
    assert!(
        !summary_projection_all_time_uses_account_exact_record(&account_record, Some(100)),
        "a complete account compact bucket must not double count the raw fallback"
    );

    // Replacing a partial compact aggregate requires every co-located live contribution,
    // not only the archive repair row. This models a retained terminal failure whose source
    // archive prunes successful detail rows after materialization.
    let mut retained_live_record = record.clone();
    retained_live_record.row.id = 9;
    retained_live_record.row.invoke_id = "retained-live-tail".to_string();
    retained_live_record.row.total_tokens = 3;
    retained_live_record.row.cost = Some(0.2);
    retained_live_record.row.status = "failed".to_string();
    retained_live_record.row.failure_class = Some("service_failure".to_string());
    retained_live_record.is_archive_record = false;
    retained_live_record.is_persisted_live_record = true;
    retained_live_record.global_rollup_covered = false;

    // The exact bucket replaces the partial compact aggregate, then adds both the archive
    // and retained live records from the canonical projection.
    let projection = SummaryProjection {
        records: vec![record.clone(), retained_live_record],
        hourly_buckets: BTreeMap::from([(bucket, vec![0, 1])]),
        hourly_rollup_totals: HashMap::from([(
            (bucket, None),
            StatsTotals {
                total_count: 1,
                success_count: 1,
                total_tokens: 4,
                total_cost: 0.5,
                ..StatsTotals::default()
            },
        )]),
        exact_global_total_rollup_buckets: HashSet::from([bucket]),
        refreshed_at: Some(Instant::now()),
        ..SummaryProjection::default()
    };
    let response = projection
        .response_for_query(
            &SummaryQuery {
                window: Some("2h".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            },
            100,
        )
        .expect("fresh in-memory range response");
    assert_eq!(response.total_count, 2);
    assert_eq!(response.success_count, 1);
    assert_eq!(response.failure_count, 1);
    assert_eq!(response.total_tokens, 12);
    assert_eq!(response.total_cost, 1.2);
    assert_eq!(response.non_success_cost, Some(0.2));
}

#[tokio::test]
async fn summary_projection_fails_closed_when_unmaterialized_archive_exceeds_budget() {
    const TEST_EXACT_RECORD_LIMIT: usize = 8;

    with_summary_projection_test_exact_record_limit(TEST_EXACT_RECORD_LIMIT, async {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory archive pool");
    sqlx::query(
        "CREATE TABLE codex_invocations (\
         id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL,\
         source TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER,\
         cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL,\
         cost_input REAL, cost_cache_write REAL, cost_cache_read REAL, cost_output REAL,\
         cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT,\
         failure_class TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create archive fixture table");
    sqlx::query(
        "WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1) \
         INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model, payload, \
         input_tokens, output_tokens, total_tokens, cost, status) \
         SELECT value, 'archive-summary-' || value, datetime('now', '-1 minute'), 'proxy', \
         'gpt-5', '{}', 1, 1, 2, 0.1, 'success' FROM rows",
    )
    .bind((TEST_EXACT_RECORD_LIMIT + 1) as i64)
    .execute(&pool)
    .await
    .expect("insert unmaterialized archive fixture");

    // SQLite's `datetime('now')` is UTC while persisted naive timestamps are interpreted as
    // Asia/Shanghai.  Use the matching UTC range so this fixture proves the LIMIT/overflow
    // path instead of silently selecting an empty range.
    let now = Utc::now();
    let archive_range = ExactUtcRange {
        start: now - ChronoDuration::hours(9),
        end: now - ChronoDuration::hours(7),
    };
    let mut records = HashMap::new();
    let mut budget = 0;
    let error = merge_summary_projection_archive_records(
        &pool,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        false,
        archive_range,
        &HashSet::new(),
        None,
        &HashSet::new(),
        &mut records,
        &mut budget,
    )
    .await
    .expect_err("unmaterialized archive overflow must fail closed");
    assert!(error.to_string().contains("exact-record budget"));
    assert!(records.len() <= TEST_EXACT_RECORD_LIMIT);

    let mut materialized_records = HashMap::new();
    let mut materialized_budget = 0;
    merge_summary_projection_archive_records(
        &pool,
        &HashSet::new(),
        &HashMap::from([(
            (
                align_bucket_epoch((now - ChronoDuration::hours(8)).timestamp(), 3_600, 0),
                None,
            ),
            StatsTotals::default(),
        )]),
        &HashMap::from([(
            (
                align_bucket_epoch((now - ChronoDuration::hours(8)).timestamp(), 3_600, 0),
                None,
            ),
            UsageBreakdownResponse {
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                output_tokens: 0,
                costs: None,
                models: Vec::new(),
            },
        )]),
        true,
        archive_range,
        &HashSet::new(),
        None,
        &HashSet::new(),
        &mut materialized_records,
        &mut materialized_budget,
    )
    .await
    .expect("materialized archive outside protected boundaries must stay bounded");
    assert!(materialized_records.is_empty());
    })
    .await;
}

#[tokio::test]
async fn summary_projection_bounds_current_index_and_serves_recent_limit_from_it() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let fixture_now = Utc::now();
    for index in 0..80_i64 {
        let occurred_at =
            db_occurred_at_lower_bound(fixture_now - ChronoDuration::seconds(index + 2));
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES (?1, ?2, 'proxy', 'success', ?3, 1.0, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(format!("summary-current-{index}"))
        .bind(occurred_at)
        .bind(index + 1)
        .execute(&state.pool)
        .await
        .expect("insert current fixture row");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate bounded recent index");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    let max = state.config.list_limit_max as usize;
    assert!(projection.recent_indexes[&None].len() <= max);
    assert!(projection.recent_indexes[&Some(42)].len() <= max);
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(3),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve bounded current index from memory");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.total_tokens, 3 + 2 + 1);
}
