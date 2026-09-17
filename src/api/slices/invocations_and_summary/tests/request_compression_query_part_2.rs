#[tokio::test]
async fn summary_projection_hydration_is_single_flight_per_hub() {
    let first = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let second = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;

    tokio::join!(
        hydrate_summary_projection_fixture(&first),
        hydrate_summary_projection_fixture(&second),
    );

    assert!(first.subscription_hub.summary_projection().await.is_some());
    assert!(second.subscription_hub.summary_projection().await.is_some());
}

#[tokio::test]
async fn summary_handler_serves_hydrated_projection_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    let query = SummaryQuery {
        window: Some("1d".to_string()),
        limit: None,
        time_zone: Some("America/Los_Angeles".to_string()),
        upstream_account_id: Some(42),
    };
    state.pool.close().await;

    let Json(actual) = fetch_summary(State(state), Query(query))
        .await
        .expect("serve hydrated projection");

    assert_eq!(actual.total_count, 1);
    assert_eq!(actual.total_tokens, 17);
    assert_eq!(actual.total_cost, 1.25);
}

#[tokio::test]
async fn summary_handler_previous7d_localizes_missing_archive_usage_replay() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let query = SummaryQuery {
        window: Some("previous7d".to_string()),
        limit: None,
        time_zone: Some("UTC".to_string()),
        upstream_account_id: None,
    };
    let occurred_at = Utc::now() - ChronoDuration::days(2);
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-previous7d-last-good', ?1, 'proxy', 'success', 29, 3.5, \
                     '{\"upstreamAccountId\":42,\"responseModel\":\"gpt-5\",\"reasoningEffort\":\"high\"}', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(occurred_at))
        .execute(&state.pool)
        .await
        .expect("insert previous7d last-good row");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate previous7d last-good projection");

    let Json(last_good) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("previous7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve initial previous7d projection");
    assert_eq!(last_good.total_count, 1);
    assert_eq!(last_good.total_tokens, 29);
    assert_eq!(last_good.total_cost, 3.5);
    let last_good_usage = last_good
        .usage_breakdown
        .as_ref()
        .expect("hydrate the previous7d model and reasoning usage");
    assert_eq!(last_good_usage.models.len(), 1);
    assert_eq!(last_good_usage.models[0].model, "gpt-5");
    assert_eq!(
        last_good_usage.models[0].reasoning_effort.as_deref(),
        Some("high")
    );

    let (range_start, _) =
        previous_full_days_range_bounds(7, Utc::now(), chrono_tz::UTC).expect("previous7d range");
    let archive_start = range_start + ChronoDuration::hours(2);
    let archive_end = archive_start + ChronoDuration::hours(1);
    let archive_path = "/definitely/missing/summary-previous7d-usage-replay.sqlite.gz";
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
              historical_rollups_materialized_at, created_at) \
             VALUES ('codex_invocations', '2026-01', ?1, 'summary-previous7d-last-good', 1, 'completed', \
                     ?2, ?3, datetime('now'), datetime('now'))",
        )
        .bind(archive_path)
        .bind(db_occurred_at_lower_bound(archive_start))
        .bind(db_occurred_at_upper_bound(archive_end))
        .execute(&state.pool)
        .await
        .expect("insert materialized archive with missing usage replay");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', ?2, 'summary-previous7d-last-good')",
        )
        .bind(target)
        .bind(archive_path)
        .execute(&state.pool)
        .await
        .expect("record non-usage archive replay coverage");
    }

    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("refresh publishes independent exact selections with the archive gap localized");
    state.pool.close().await;

    let response = fetch_summary(State(state), Query(query)).await;
    assert!(matches!(response, Err(ApiError::Unavailable(_))));
}

#[tokio::test]
async fn summary_handler_cold_key_and_invalid_window_do_not_require_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("Pacific/Auckland".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve an unrendered but valid projection selection");
    assert_eq!(response.total_count, 1);

    let invalid = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("not-a-summary-window".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(invalid, Err(ApiError::BadRequest(_))));
}

#[tokio::test]
async fn summary_handler_unknown_all_time_account_uses_fresh_memory_zero_response() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_projection_fixture(&state).await;
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Pacific/Auckland".to_string()),
            upstream_account_id: Some(9_999_999),
        }),
    )
    .await
    .expect("fresh projection preserves the legacy zero response for an unknown account");

    assert_eq!(response.total_count, 0);
    assert_eq!(response.total_tokens, 0);
    assert_eq!(response.total_cost, 0.0);
    assert_eq!(response.non_success_cost, Some(0.0));
    assert_eq!(response.usage_breakdown, None);
    assert_eq!(response.in_progress_conversation_count, Some(0));
    assert_eq!(response.in_progress_retry_conversation_count, Some(0));
    assert!(response.in_progress_phase_counts.is_some());
    assert!(response.maintenance.is_some());
}

#[test]
fn summary_projection_known_all_time_account_without_exact_snapshot_is_unavailable() {
    let projection = SummaryProjection {
        all_time_refreshed_at: Some(Instant::now()),
        known_account_ids: HashSet::from([42]),
        freshness: SummaryProjectionFreshness {
            global_all_time_eligible: true,
            ..SummaryProjectionFreshness::default()
        },
        ..SummaryProjection::default()
    };
    let error = projection
        .response_for_query(
            &SummaryQuery {
                window: Some("all".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: Some(42),
            },
            100,
        )
        .expect_err("known account without an exact all-time snapshot must not become zero");
    assert!(matches!(error, ApiError::Unavailable(_)));
}

#[tokio::test]
async fn summary_handler_serves_legal_duration_bound_from_memory_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_bucket = align_bucket_epoch(
        (Utc::now() - ChronoDuration::days(29)).timestamp(),
        3_600,
        0,
    );
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) VALUES (?1, 'proxy', 1, 1, 0, 23, 0.75, 0)",
        )
        .bind(historical_bucket)
        .execute(&state.pool)
        .await
        .expect("insert historical exact summary rollup");
    sqlx::query(
            "INSERT INTO upstream_account_usage_breakdown_hourly (bucket_start_epoch, source, upstream_account_key, upstream_account_id, normalized_model, normalized_reasoning_effort, request_count, cache_write_tokens, cache_read_tokens, output_tokens, cost_input, has_cost) VALUES (?1, 'proxy', 'none', NULL, 'gpt-5', 'high', 1, 0, 0, 23, 0.75, 1)",
        )
        .bind(historical_bucket)
        .execute(&state.pool)
        .await
        .expect("insert historical exact usage rollup");
    hydrate_summary_projection_fixture(&state).await;
    state.pool.close().await;

    for (window, expected_count, expected_tokens) in [("30d", 2, 40), ("48h", 1, 17)] {
        let result = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await;
        let Json(summary) = result.expect("legal duration {window} must use memory only");
        assert_eq!(
            summary.total_count, expected_count,
            "duration {window} must stay exact"
        );
        assert_eq!(
            summary.total_tokens, expected_tokens,
            "duration {window} must stay exact"
        );
    }

    for window in ["31d", "49h", "60d", "-1d"] {
        let invalid = fetch_summary(
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
            matches!(invalid, Err(ApiError::BadRequest(_))),
            "invalid duration {window} must fail before the memory-only handler path"
        );
    }
}

#[test]
fn summary_projection_boundary_buckets_cover_closed_windows_for_every_timezone() {
    let end = Utc
        .with_ymd_and_hms(2026, 8, 31, 23, 59, 55)
        .single()
        .expect("valid deterministic projection boundary time");
    let buckets = summary_projection_boundary_buckets(end);

    for duration in [ChronoDuration::days(30), ChronoDuration::hours(48)] {
        assert!(
            buckets.contains(&align_bucket_epoch((end - duration).timestamp(), 3_600, 0)),
            "legal duration boundary {duration:?} must remain exact"
        );
    }

    for boundary_now in [
        end,
        end + ChronoDuration::seconds(SUMMARY_SNAPSHOT_MAX_STALE.as_secs() as i64),
    ] {
        for timezone in [
            "Asia/Kathmandu"
                .parse::<Tz>()
                .expect("valid Kathmandu timezone"),
            "Pacific/Kiritimati"
                .parse::<Tz>()
                .expect("valid Kiritimati timezone"),
        ] {
            for spec in ["today", "yesterday", "thisWeek", "thisMonth"] {
                let (start, end) = named_range_bounds(spec, boundary_now, timezone)
                    .expect("supported closed summary window");
                assert!(buckets.contains(&align_bucket_epoch(start.timestamp(), 3_600, 0)));
                assert!(buckets.contains(&align_bucket_epoch(end.timestamp(), 3_600, 0)));
            }
            let (start, end) = previous_full_days_range_bounds(7, boundary_now, timezone)
                .expect("previous7d range");
            assert!(buckets.contains(&align_bucket_epoch(start.timestamp(), 3_600, 0)));
            assert!(buckets.contains(&align_bucket_epoch(end.timestamp(), 3_600, 0)));
        }
    }
}

#[test]
fn summary_snapshot_marks_a_lagged_mutation_bus_dirty() {
    assert!(summary_snapshot_runtime_mutation_is_dirty(Err(
        broadcast::error::RecvError::Lagged(1)
    )));
    assert!(!summary_snapshot_runtime_mutation_is_dirty(Err(
        broadcast::error::RecvError::Closed
    )));
}

#[test]
fn summary_snapshot_marks_cadence_dirty_only_for_active_owners() {
    assert!(summary_snapshot_trigger_marks_dirty(
        SummarySnapshotTrigger::Cadence,
        true,
    ));
    assert!(!summary_snapshot_trigger_marks_dirty(
        SummarySnapshotTrigger::Cadence,
        false,
    ));
    assert!(summary_snapshot_trigger_marks_dirty(
        SummarySnapshotTrigger::Mutation,
        false,
    ));
}

#[test]
fn summary_cadence_refreshes_stale_all_time_only_for_an_all_time_owner() {
    let projection = SummaryProjection {
        refreshed_at: Some(Instant::now()),
        all_time_refreshed_at: Some(
            Instant::now() - SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL - Duration::from_secs(1),
        ),
        ..SummaryProjection::default()
    };
    assert!(!projection.needs_cadence_refresh(false));
    assert!(projection.needs_cadence_refresh(true));
}

#[test]
fn summary_manifest_admission_retries_only_after_the_controlled_backoff() {
    let blocked_at = Instant::now();
    assert!(!summary_projection_manifest_admission_retry_is_due(
        Some(blocked_at),
        blocked_at + SUMMARY_PROJECTION_MANIFEST_ADMISSION_RETRY_BACKOFF - Duration::from_secs(1),
    ));
    assert!(summary_projection_manifest_admission_retry_is_due(
        Some(blocked_at),
        blocked_at + SUMMARY_PROJECTION_MANIFEST_ADMISSION_RETRY_BACKOFF,
    ));
}

#[test]
fn summary_cadence_does_not_rebuild_all_time_during_manifest_admission_backoff() {
    let projection = SummaryProjection {
        refreshed_at: Some(Instant::now()),
        all_time_manifest_admission_blocked_at: Some(Instant::now()),
        ..SummaryProjection::default()
    };
    assert!(
        !projection.needs_cadence_refresh(true),
        "a blocked all-time admission must not turn an active owner into a retry loop"
    );
}

#[test]
fn summary_all_time_account_manifest_backoff_retains_fresh_last_good() {
    let account_id = 42;
    let response = StatsTotals {
        total_count: 3,
        success_count: 3,
        total_tokens: 91,
        total_cost: 4.5,
        ..StatsTotals::default()
    }
    .into_response();
    let projection = SummaryProjection {
        all_time_by_account: HashMap::from([(Some(account_id), response)]),
        all_time_refreshed_at: Some(Instant::now()),
        all_time_account_refreshed_at: HashMap::from([(account_id, Instant::now())]),
        all_time_account_manifest_admission_blocked_at: Some(Instant::now()),
        all_time_account_ids_with_projection_data: HashSet::from([account_id]),
        refreshed_at: Some(Instant::now()),
        freshness: SummaryProjectionFreshness {
            global_all_time_eligible: true,
            account_all_time_eligible: HashSet::from([account_id]),
        },
        ..SummaryProjection::default()
    };
    let response = projection
        .response_for_query(
            &SummaryQuery {
                window: Some("all".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: Some(account_id),
            },
            50,
        )
        .expect("fresh last-good account snapshot remains memory-readable during backoff");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.total_tokens, 91);
}

#[tokio::test]
async fn summary_all_time_v2_marker_without_pages_does_not_complete_manifest_scan() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    for (id, sha256) in [(1_i64, "summary-v2-proof-1"), (2, "summary-v2-proof-2")] {
        sqlx::query(
            "INSERT INTO archive_batches
                 (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
                 VALUES (?1, 'codex_invocations', '2026-09', ?2, ?3, 1, 'completed', 'unknown')",
        )
        .bind(id)
        .bind(format!("/archive/{sha256}.sqlite.gz"))
        .bind(sha256)
        .execute(&pool)
        .await
        .expect("seed archive manifest");
        sqlx::query(
            "INSERT INTO summary_archive_snapshot_v2_proof
                 (archive_batch_id, manifest_sha256, page_count, row_count, semantic_sha256)
                 VALUES (?1, ?2, 1, 1, 'semantic-proof')",
        )
        .bind(id)
        .bind(sha256)
        .execute(&pool)
        .await
        .expect("seed V2 proof");
    }

    assert!(
        !summary_all_time_manifest_v2_coverage_complete(&pool, 2)
            .await
            .expect("check incomplete V2 proof set"),
        "a marker without verified V2 pages must not complete coverage"
    );

    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
             VALUES (3, 'codex_invocations', '2026-09', '/archive/unproven.sqlite.gz',
                     'summary-v2-unproven', 1, 'completed', 'unknown')",
    )
    .execute(&pool)
    .await
    .expect("seed unproven archive manifest");
    assert!(
        !summary_all_time_manifest_v2_coverage_complete(&pool, 3)
            .await
            .expect("check incomplete V2 proof set"),
        "a missing V2 proof must retain the bounded legacy proof path"
    );
}

#[test]
fn summary_paged_boundary_raw_archive_admission_is_bounded() {
    let mut admitted_archives = 0usize;
    for _ in 0..SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        summary_projection_admit_paged_boundary_raw_archive(&mut admitted_archives)
            .expect("each archive within the bounded admission is accepted");
    }
    let error = summary_projection_admit_paged_boundary_raw_archive(&mut admitted_archives)
        .expect_err("the next raw archive must fail closed before it is opened");
    assert!(
        error
            .to_string()
            .contains("paged boundary raw archive admission exceeded")
    );
}

async fn summary_projection_overflowed_boundary_pool() -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    pool
}

async fn seed_summary_projection_authoritative_boundary_proof(pool: &sqlx::SqlitePool) -> String {
    let source_path = "/authoritative/complete-summary-proof.sqlite.gz";
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
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                ?1,
                'complete-summary-proof',
                1,
                'completed',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
    )
    .bind(source_path)
    .execute(pool)
    .await
    .expect("seed legacy source before proof promotion");
    for target in SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', ?2, 'complete-summary-proof')",
        )
        .bind(target)
        .bind(source_path)
        .execute(pool)
        .await
        .expect("seed authoritative Summary proof");
    }
    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = 'authoritative' WHERE file_path = ?1",
    )
    .bind(source_path)
    .execute(pool)
    .await
    .expect("promote proof-complete archive to authoritative source");
    source_path.to_string()
}

async fn seed_summary_projection_boundary_mirrors(pool: &sqlx::SqlitePool) {
    let mut tx = pool
        .begin()
        .await
        .expect("begin mirror fixture transaction");
    for index in 0..=SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        sqlx::query(
            r#"
                INSERT INTO archive_batches (
                    dataset,
                    month_key,
                    file_path,
                    sha256,
                    row_count,
                    status,
                    summary_source_kind,
                    coverage_start_at,
                    coverage_end_at,
                    historical_rollups_materialized_at
                )
                VALUES (
                    'codex_invocations',
                    '2026-08',
                    ?1,
                    ?2,
                    1,
                    'completed',
                    'live_mirror',
                    '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00',
                    datetime('now')
                )
                "#,
        )
        .bind(format!("/mirror/summary-detail-{index}.sqlite.gz"))
        .bind(format!("mirror-{index}"))
        .execute(tx.as_mut())
        .await
        .expect("seed non-Summary detail mirror");
    }
    tx.commit().await.expect("commit mirror fixture");
}

#[tokio::test]
async fn summary_projection_overflowed_boundary_mirrors_do_not_block_proof_paging() {
    let pool = summary_projection_overflowed_boundary_pool().await;
    let source_path = seed_summary_projection_authoritative_boundary_proof(&pool).await;
    seed_summary_projection_boundary_mirrors(&pool).await;

    let admitted = crate::stats::load_completed_invocation_archive_paths_in_range_bounded(
        &pool,
        None,
        SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES,
    )
    .await
    .expect("load Summary archive admission");
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].file_path(), source_path);
    let exact_horizon = ExactUtcRange {
        start: Utc
            .with_ymd_and_hms(2026, 8, 1, 0, 0, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .with_ymd_and_hms(2026, 9, 1, 0, 0, 0)
            .single()
            .expect("valid range end"),
    };

    assert_eq!(
        summary_projection_overflowed_boundary_manifest_coverage(&pool, exact_horizon)
            .await
            .expect("load bounded source proof"),
        Some(SummaryProjectionOverflowedBoundaryCoverage {
            high_watermark_id: 1,
            unknown_coverage_ranges: Vec::new(),
        }),
    );
}

#[tokio::test]
async fn summary_projection_overflowed_bounded_boundary_gap_remains_pageable() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    let range = ExactUtcRange {
        start: Utc
            .with_ymd_and_hms(2026, 8, 1, 0, 0, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .with_ymd_and_hms(2026, 8, 2, 0, 0, 0)
            .single()
            .expect("valid range end"),
    };
    for index in 0..=SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        sqlx::query(
                "INSERT INTO archive_batches \
                 (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
                  historical_rollups_materialized_at) \
                 VALUES ('codex_invocations', '2026-08', ?1, ?2, 1, 'completed', ?3, ?4, datetime('now'))",
            )
            .bind(format!("/archive/summary-boundary-gap-{index}.sqlite.gz"))
            .bind(format!("boundary-gap-{index}"))
            .bind(crate::stats::db_occurred_at_lower_bound(range.start))
            .bind(crate::stats::db_occurred_at_lower_bound(range.start + ChronoDuration::hours(1)))
            .execute(&pool)
            .await
            .expect("seed overflowed boundary manifest");
    }

    assert_eq!(
        summary_projection_overflowed_boundary_manifest_coverage(&pool, range)
            .await
            .expect("load overflowed boundary coverage"),
        Some(SummaryProjectionOverflowedBoundaryCoverage {
            high_watermark_id: (SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64,
            unknown_coverage_ranges: Vec::new(),
        }),
        "bounded incomplete boundaries must remain eligible for range-local admission"
    );
}

#[tokio::test]
async fn summary_projection_overflowed_boundary_unknown_coverage_is_broad() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    let range = ExactUtcRange {
        start: Utc
            .with_ymd_and_hms(2026, 8, 1, 0, 0, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .with_ymd_and_hms(2026, 9, 1, 0, 0, 0)
            .single()
            .expect("valid range end"),
    };
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status) \
             VALUES ('codex_invocations', '2026-08', '/archive/summary-boundary-unknown.sqlite.gz', \
                     'boundary-unknown', 1, 'completed')",
        )
        .execute(&pool)
        .await
        .expect("seed unknown boundary manifest");

    assert_eq!(
        summary_projection_overflowed_boundary_manifest_coverage(&pool, range)
            .await
            .expect("load unknown boundary coverage"),
        Some(SummaryProjectionOverflowedBoundaryCoverage {
            high_watermark_id: 1,
            unknown_coverage_ranges: vec![ExactUtcRange {
                start: range.start,
                end: Utc
                    .with_ymd_and_hms(2026, 8, 31, 16, 0, 0)
                    .single()
                    .expect("valid Shanghai month boundary"),
            }],
        }),
        "an unknown current-month manifest must remain unavailable across that partition"
    );
}

#[tokio::test]
async fn summary_projection_overflowed_boundary_unknown_old_month_is_disjoint() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status) \
             VALUES ('codex_invocations', '2025-01', '/archive/summary-boundary-unknown-old.sqlite.gz', \
                     'boundary-unknown-old', 1, 'completed')",
        )
        .execute(&pool)
        .await
        .expect("seed old unknown boundary manifest");
    let range = ExactUtcRange {
        start: Utc
            .with_ymd_and_hms(2026, 8, 1, 0, 0, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .with_ymd_and_hms(2026, 9, 1, 0, 0, 0)
            .single()
            .expect("valid range end"),
    };

    assert_eq!(
        summary_projection_overflowed_boundary_manifest_coverage(&pool, range)
            .await
            .expect("load old unknown boundary coverage"),
        Some(SummaryProjectionOverflowedBoundaryCoverage {
            high_watermark_id: 1,
            unknown_coverage_ranges: Vec::new(),
        }),
        "an old unknown archive partition must not poison a disjoint supported horizon"
    );
}

#[tokio::test]
async fn summary_projection_staged_recovery_defers_paged_raw_to_supervisor() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let archive_bucket_start = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(3)).timestamp(), 3_600, 0),
            0,
        )
        .single()
        .expect("valid staged recovery archive bucket");
    let coverage_start = crate::stats::db_occurred_at_lower_bound(archive_bucket_start);
    let coverage_end =
        crate::db_occurred_at_upper_bound(archive_bucket_start + ChronoDuration::hours(1));
    for index in 0..=SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        sqlx::query(
                "INSERT INTO archive_batches \
                 (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
                  historical_rollups_materialized_at) \
                 VALUES ('codex_invocations', '2026-08', ?1, ?2, 1, 'completed', ?3, ?4, datetime('now'))",
            )
            .bind(format!(
                "/definitely/missing/summary-staged-recovery-{index}.sqlite.gz"
            ))
            .bind(format!("summary-staged-recovery-{index}"))
            .bind(&coverage_start)
            .bind(&coverage_end)
            .execute(&state.pool)
            .await
            .expect("seed paged staged-recovery manifest");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling projection before all-time recovery");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create staged-recovery interleave gate");
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    );
    let state_for_recovery = state.clone();
    let recovery =
        tokio::spawn(async move { refresh_summary_snapshots(state_for_recovery.as_ref()).await });
    tokio::pin!(recovery);
    tokio::time::timeout(Duration::from_secs(5), &mut recovery)
        .await
        .expect("staged coverage supervisor should not wait on generic raw hydration")
        .expect("join staged coverage recovery")
        .expect("defer unavailable raw source without withdrawing rolling projection");
    clear_summary_projection_test_interleave();
    assert_eq!(
        interleave.build_attempts(),
        0,
        "staged coverage recovery must defer paged raw hydration to its own page worker",
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_coverage_supervisor_runs_without_http_interest() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");
    let archive_bucket_start = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(2)).timestamp(), 3_600, 0),
            0,
        )
        .single()
        .expect("valid recent recovery archive bucket");
    let coverage_start = crate::stats::db_occurred_at_lower_bound(archive_bucket_start);
    let coverage_end =
        crate::db_occurred_at_upper_bound(archive_bucket_start + ChronoDuration::hours(1));
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
             VALUES ('codex_invocations', '2026-08', \
                     '/definitely/missing/summary-no-interest.sqlite.gz', 'summary-no-interest', 1, \
                     'completed', ?1, ?2)",
        )
        .bind(coverage_start)
        .bind(coverage_end)
        .execute(&state.pool)
        .await
        .expect("seed recent legacy archive manifest");

    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("run recovery without an all-time HTTP or SSE owner");

    let outcome: String = sqlx::query_scalar(
        "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome \
             WHERE manifest_sha256 = 'summary-no-interest'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("supervisor must record its independent recovery outcome");
    assert_eq!(outcome, "missing_source");
    state.pool.close().await;
}

#[test]
fn summary_coverage_overlay_keeps_recovery_incremental_until_final_pass() {
    assert!(
        !summary_coverage_overlay_requires_full_reduction(false, true, false),
        "a verified page must extend an existing overlay while obligations remain"
    );
    assert!(
        summary_coverage_overlay_requires_full_reduction(true, true, false),
        "the no-pending final pass must rebuild from all verified proofs"
    );
    assert!(
        summary_coverage_overlay_requires_full_reduction(false, true, true),
        "proof revocation must rebuild from the remaining verified proofs"
    );
    assert!(
        !summary_coverage_overlay_requires_full_reduction(false, true, false),
        "coverage fence changes extend same-identity proof contributions incrementally"
    );
    assert!(
        summary_coverage_overlay_requires_full_reduction(false, false, false),
        "the first overlay publication has no incremental base"
    );
}

#[tokio::test]
async fn summary_coverage_supervisor_runs_independently_of_http_interest() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");
    state
        .subscription_hub
        .note_summary_http_interest(false)
        .await;
    let archive_bucket_start = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(2)).timestamp(), 3_600, 0),
            0,
        )
        .single()
        .expect("valid recent recovery archive bucket");
    let coverage_start = crate::stats::db_occurred_at_lower_bound(archive_bucket_start);
    let coverage_end =
        crate::db_occurred_at_upper_bound(archive_bucket_start + ChronoDuration::hours(1));
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
             VALUES ('codex_invocations', '2026-08', \
                     '/definitely/missing/summary-owner-cadence.sqlite.gz', 'summary-owner-cadence', 1, \
                     'completed', ?1, ?2)",
        )
        .bind(coverage_start)
        .bind(coverage_end)
        .execute(&state.pool)
        .await
        .expect("seed recent legacy archive manifest");

    SummaryCoverageRecoverySupervisor::run(state.as_ref())
        .await
        .expect("coverage worker must recover independently of HTTP interest");
    let trigger = summary_snapshot_cadence_trigger(state.as_ref()).await;
    assert!(
        trigger.is_none(),
        "a fresh owner must not force a rolling refresh for historical coverage"
    );

    let outcome: String = sqlx::query_scalar(
        "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome \
             WHERE manifest_sha256 = 'summary-owner-cadence'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("independent worker must recover coverage even while an HTTP owner is present");
    assert_eq!(outcome, "missing_source");
    state.pool.close().await;
}

#[tokio::test]
async fn summary_v2_exact_coverage_batches_unresolved_hour_ranges() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let middle_bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let middle = Utc
        .timestamp_opt(middle_bucket, 0)
        .single()
        .expect("valid middle bucket");
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
             VALUES ('codex_invocations', '2026-09', \
                     '/definitely/missing/batched-coverage.sqlite.gz', 'batched-coverage', 1, \
                     'completed', ?1, ?1)",
        )
        .bind(crate::stats::db_occurred_at_lower_bound(middle))
        .execute(&state.pool)
        .await
        .expect("seed unresolved archive coverage");
    let candidates = [
        middle_bucket.saturating_sub(3_600),
        middle_bucket,
        middle_bucket.saturating_add(3_600),
    ]
    .into_iter()
    .collect::<HashSet<_>>();

    let exact = summary_v2_exact_coverage_buckets(&state.pool, &candidates)
        .await
        .expect("batch coverage lookup");
    assert_eq!(
        exact,
        [
            middle_bucket.saturating_sub(3_600),
            middle_bucket.saturating_add(3_600),
        ]
        .into_iter()
        .collect(),
        "only the hour intersecting an unresolved authority may remain unavailable"
    );
    state.pool.close().await;
}

async fn summary_v2_final_proof_fixture() -> (Arc<AppState>, i64, String) {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = crate::stats::db_occurred_at_lower_bound(Utc::now());
    let archive_batch_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
              coverage_start_at, coverage_end_at, historical_rollups_materialized_at) \
             VALUES ('codex_invocations', '2026-09', \
                     '/definitely/missing/materialized-v2-boundary.sqlite.gz', \
                     'materialized-v2-boundary', 1, 'completed', 'unknown', ?1, ?1, datetime('now')) \
             RETURNING id",
        )
        .bind(&occurred_at)
        .fetch_one(&state.pool)
        .await
        .expect("seed materialized archive manifest");
    let record = SummaryArchiveSnapshotV2Record {
        id: 1,
        invoke_id: "materialized-v2-boundary".to_string(),
        occurred_at: occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        model: Some("gpt-5".to_string()),
        response_model: None,
        input_tokens: 1,
        output_tokens: 1,
        cache_input_tokens: 0,
        reasoning_tokens: 0,
        reasoning_effort: None,
        total_tokens: 2,
        cost: Some(0.01),
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        status: "success".to_string(),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: false,
        upstream_account_id: None,
    };
    let encoded = serde_json::to_vec(&vec![record]).expect("encode V2 proof record");
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id,
        manifest_sha256: "materialized-v2-boundary".to_string(),
        page_index: 0,
        coverage_start: occurred_at.clone(),
        coverage_end: occurred_at.clone(),
        row_count: 1,
        payload: zstd::stream::encode_all(encoded.as_slice(), 1).expect("compress V2 proof record"),
    };
    let mut transaction = state
        .pool
        .begin()
        .await
        .expect("begin V2 proof transaction");
    store_summary_archive_snapshot_page_v2_tx(transaction.as_mut(), &page)
        .await
        .expect("store materialized V2 proof");
    transaction
        .commit()
        .await
        .expect("commit materialized V2 proof");
    assert!(
        ensure_summary_archive_snapshot_v2_final_proof(
            &state.pool,
            archive_batch_id,
            "materialized-v2-boundary",
        )
        .await
        .expect("verify materialized V2 final proof"),
        "fixture V2 page must become a final proof"
    );
    (state, archive_batch_id, occurred_at)
}

#[tokio::test]
async fn summary_v2_final_proof_covers_materialized_archive_boundary() {
    let (state, _archive_batch_id, occurred_at) = summary_v2_final_proof_fixture().await;

    let identities = load_summary_v2_archive_proof_identities(&state.pool)
        .await
        .expect("load materialized V2 proof identity");
    let totals = load_summary_v2_archive_totals_for_proof_identities(
        &state.pool,
        &identities,
        &HashSet::new(),
    )
    .await
    .expect("reduce materialized V2 proof");
    let bucket = align_bucket_epoch(
        parse_to_utc_datetime(&occurred_at)
            .expect("valid V2 coverage time")
            .timestamp(),
        3_600,
        0,
    );
    assert!(
        totals.global_coverage_buckets.contains(&bucket),
        "a verified materialized archive must remove its boundary unavailability proof"
    );
    assert_eq!(
        totals.global.total_count, 0,
        "materialized rollups remain the aggregate authority and must not be double counted"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_coverage_supervisor_does_not_republish_unchanged_ready_checkpoint() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");
    SummaryCoverageRecoverySupervisor::run(state.as_ref())
        .await
        .expect("publish ready all-time checkpoint");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create publication interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    );
    let recovery_state = state.clone();
    let recovery = tokio::spawn(async move {
        SummaryCoverageRecoverySupervisor::run(recovery_state.as_ref()).await
    });
    tokio::pin!(recovery);
    tokio::select! {
        result = &mut recovery => {
            result
                .expect("join unchanged recovery pass")
                .expect("unchanged recovery pass must remain a no-op");
        }
        _ = interleave.wait_for_writer() => {
            interleave.resume_build();
            let _ = recovery.await;
            clear_summary_projection_test_interleave();
            panic!("an unchanged ready checkpoint must not republish the Projection");
        }
    }
    clear_summary_projection_test_interleave();
    assert_eq!(interleave.build_attempts(), 0);
    state.pool.close().await;
}

#[tokio::test]
async fn summary_all_time_publication_uses_its_own_coverage_fence() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let stale_fence = SummaryCoverageFence {
        completed_manifest_high_watermark_id: None,
        coverage_revision: 1,
        account_coverage_revision: 1,
    };
    let current_fence = SummaryCoverageFence {
        coverage_revision: 2,
        account_coverage_revision: 2,
        ..stale_fence
    };
    let checkpoint = SummaryAllTimeProjectionCheckpointRow {
        coverage_revision: current_fence.coverage_revision,
        account_coverage_revision: current_fence.account_coverage_revision,
        global_manifest_complete: 1,
        account_manifest_complete: 1,
        global_rollup_complete: 1,
        account_rollup_complete: 1,
        usage_rollup_complete: 1,
        ..SummaryAllTimeProjectionCheckpointRow::default()
    };
    state
        .subscription_hub
        .store_summary_projection(SummaryProjection {
            global_all_time_coverage_fence: Some(stale_fence),
            account_all_time_coverage_fence: Some(stale_fence),
            generation_fence: checkpoint.generation_fence(),
            freshness: SummaryProjectionFreshness {
                global_all_time_eligible: true,
                ..SummaryProjectionFreshness::default()
            },
            ..SummaryProjection::default()
        })
        .await;
    assert!(
        summary_all_time_checkpoint_publication_required(state.as_ref(), &checkpoint)
            .await
            .expect("compare stale all-time coverage fence"),
        "a rolling generation update must not make a retained all-time aggregate look current"
    );

    state
        .subscription_hub
        .store_summary_projection(SummaryProjection {
            global_all_time_coverage_fence: Some(current_fence),
            account_all_time_coverage_fence: Some(current_fence),
            all_time_refreshed_at: Some(Instant::now()),
            generation_fence: checkpoint.generation_fence(),
            freshness: SummaryProjectionFreshness {
                global_all_time_eligible: true,
                ..SummaryProjectionFreshness::default()
            },
            ..SummaryProjection::default()
        })
        .await;
    assert!(
        !summary_all_time_checkpoint_publication_required(state.as_ref(), &checkpoint)
            .await
            .expect("compare current all-time coverage fence"),
        "an unchanged all-time coverage fence must not repeat finalization"
    );
    state
        .subscription_hub
        .store_summary_projection(SummaryProjection {
            global_all_time_coverage_fence: Some(current_fence),
            account_all_time_coverage_fence: Some(current_fence),
            all_time_refreshed_at: Some(
                Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1),
            ),
            generation_fence: checkpoint.generation_fence(),
            freshness: SummaryProjectionFreshness {
                global_all_time_eligible: true,
                ..SummaryProjectionFreshness::default()
            },
            ..SummaryProjection::default()
        })
        .await;
    assert!(
        summary_all_time_checkpoint_publication_required(state.as_ref(), &checkpoint)
            .await
            .expect("compare stale all-time snapshot freshness"),
        "a ready checkpoint with an expired all-time snapshot must re-enter bounded finalization"
    );
    state.pool.close().await;
}

async fn summary_coverage_supervisor_30d_proof_fixture()
-> (Arc<AppState>, SummaryArchiveSnapshotPage, usize) {
    const MANIFEST_COUNT: usize = SUMMARY_PROJECTION_ALL_TIME_MANIFEST_PROOF_PAGE_SIZE * 3 + 1;

    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish empty bootstrap projection");

    let occurred_at =
        crate::stats::db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(2));
    let mut last_page = None;
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin archive proof fixture");
    for index in 0..MANIFEST_COUNT {
        let manifest_sha256 = format!("summary-ownerless-proof-{index}");
        let file_path = format!("/definitely/missing/summary-ownerless-proof-{index}.sqlite.gz");
        let archive_batch_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO archive_batches \
                 (dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
                  coverage_start_at, coverage_end_at) \
                 VALUES ('codex_invocations', '2026-09', ?1, ?2, 1, 'completed', \
                         'unknown', ?3, ?3) RETURNING id",
        )
        .bind(file_path)
        .bind(&manifest_sha256)
        .bind(&occurred_at)
        .fetch_one(tx.as_mut())
        .await
        .expect("insert archive manifest fixture");
        let record = SummaryArchiveSnapshotV2Record {
            id: index as i64 + 1,
            invoke_id: format!("summary-ownerless-proof-{index}"),
            occurred_at: occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            model: Some("gpt-5".to_string()),
            response_model: None,
            input_tokens: 1,
            output_tokens: 1,
            cache_input_tokens: 0,
            reasoning_tokens: 0,
            reasoning_effort: None,
            total_tokens: 2,
            cost: Some(0.01),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            status: "success".to_string(),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: false,
            upstream_account_id: None,
        };
        let encoded = serde_json::to_vec(&vec![record]).expect("encode V2 proof record");
        let page = SummaryArchiveSnapshotPage {
            archive_batch_id,
            manifest_sha256,
            page_index: 0,
            coverage_start: occurred_at.clone(),
            coverage_end: occurred_at.clone(),
            row_count: 1,
            payload: zstd::stream::encode_all(encoded.as_slice(), 1)
                .expect("compress V2 proof record"),
        };
        if index == MANIFEST_COUNT - 1 {
            last_page = Some(page);
        } else {
            store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
                .await
                .expect("store initial V2 proof page");
        }
    }
    tx.commit().await.expect("commit archive proof fixture");
    (
        state,
        last_page.expect("retain final V2 proof page for the test"),
        MANIFEST_COUNT,
    )
}

#[tokio::test]
async fn summary_coverage_supervisor_publishes_verified_30d_proof_without_http_interest() {
    let (state, last_page, manifest_count) = summary_coverage_supervisor_30d_proof_fixture().await;

    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::RollingDelta)
        .await
        .expect("publish the finite recent archive gap");
    let unavailable = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("30d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(unavailable, Err(ApiError::Unavailable(_))));

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create overlay publication interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::AfterRollupLoad,
    );
    let recovery_state = state.clone();
    let recovery = tokio::spawn(async move {
        SummaryCoverageRecoverySupervisor::run(recovery_state.as_ref()).await
    });
    tokio::pin!(recovery);
    tokio::select! {
        _ = interleave.wait_for_writer() => {
            let modes = interleave.build_modes();
            interleave.resume_build();
            recovery.await.expect("join ownerless historical recovery").expect("run ownerless historical recovery");
            clear_summary_projection_test_interleave();
            assert!(
                !modes.contains(&SummaryProjectionBuildMode::RollingDelta),
                "verified V2 coverage must publish an overlay without generic RollingDelta",
            );
        }
        result = &mut recovery => {
            clear_summary_projection_test_interleave();
            result.expect("join ownerless historical recovery").expect("run ownerless historical recovery");
            assert!(
                interleave.build_modes().is_empty(),
                "verified V2 coverage must not start a projection rebuild when publishing an overlay",
            );
        }
    }
    let partial = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("30d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(partial, Err(ApiError::Unavailable(_))),
        "a sibling archive without a final proof must keep the shared hour unavailable"
    );

    let mut tx = state.pool.begin().await.expect("begin final V2 proof");
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &last_page)
        .await
        .expect("store final recent V2 proof page");
    tx.commit().await.expect("commit final V2 proof page");

    SummaryCoverageRecoverySupervisor::run(state.as_ref())
        .await
        .expect("finish the remaining bounded V2 proof");
    let Json(response) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("30d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("all overlapping V2 proofs must publish an exact 30d response");
    assert_eq!(response.total_count, manifest_count as i64);
    assert_eq!(response.total_tokens, (manifest_count * 2) as i64);
    state.pool.close().await;
}
