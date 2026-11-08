use super::*;

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
    let recovery = tokio::spawn(async move {
        SummaryCoverageRecoverySupervisor::run(state_for_recovery.as_ref()).await
    });
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

    SummaryCoverageRecoverySupervisor::run(state.as_ref())
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

#[tokio::test]
async fn summary_snapshot_refresh_does_not_run_durable_coverage_recovery() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-snapshot-owner-live', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("seed rolling Summary source");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling Summary projection");

    let archive_bucket_start = Utc
        .timestamp_opt(
            align_bucket_epoch((Utc::now() - ChronoDuration::days(2)).timestamp(), 3_600, 0),
            0,
        )
        .single()
        .expect("valid snapshot owner archive bucket");
    let coverage_start = crate::stats::db_occurred_at_lower_bound(archive_bucket_start);
    let coverage_end =
        crate::db_occurred_at_upper_bound(archive_bucket_start + ChronoDuration::hours(1));
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
         VALUES ('codex_invocations', '2026-09', \
                 '/definitely/missing/summary-snapshot-owner.sqlite.gz', 'summary-snapshot-owner', 1, \
                 'completed', ?1, ?2)",
    )
    .bind(coverage_start)
    .bind(coverage_end)
    .execute(&state.pool)
    .await
    .expect("seed historical Summary manifest");
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;

    let (result, v2_window_calls) = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
        .scope(Cell::new(0), async {
            let result = refresh_summary_snapshots(state.as_ref()).await;
            let calls = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
                .try_with(Cell::get)
                .expect("V2 window counter scope");
            (result, calls)
        })
        .await;
    result.expect("Summary owner refresh must keep rolling projection available");
    assert_eq!(
        v2_window_calls, 0,
        "snapshot owner refresh must not invoke durable historical recovery"
    );

    let outcome_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM summary_archive_snapshot_backfill_outcome \
         WHERE manifest_sha256 = 'summary-snapshot-owner'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("inspect historical backfill outcomes");
    assert_eq!(
        outcome_count, 0,
        "snapshot owner refresh must not persist a durable recovery outcome"
    );
    state.pool.close().await;

    let Json(current) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("rolling projection remains available from memory");
    assert_eq!(current.total_count, 1);
    assert_eq!(current.total_tokens, 17);
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

#[test]
fn summary_coverage_recovery_scheduler_cadence_waits_ten_seconds() {
    assert_eq!(
        summary_coverage_recovery_scheduler_delay(
            SummaryCoverageRecoveryNextTurn::Immediate,
            Duration::from_secs(10),
        ),
        Duration::from_secs(1),
        "committed recovery pages must yield for one second before the next admission"
    );
    assert_eq!(
        summary_coverage_recovery_scheduler_delay(
            SummaryCoverageRecoveryNextTurn::Idle,
            Duration::from_secs(10),
        ),
        SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL,
        "idle durable recovery must use the ten-second floor"
    );
    assert_eq!(
        SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL,
        Duration::from_secs(10)
    );
}

#[test]
fn summary_coverage_recovery_follow_up_remains_enabled_when_work_exists() {
    for result in [
        SummaryArchiveSnapshotBackfillWindowResult {
            candidate_count: 1,
            ..SummaryArchiveSnapshotBackfillWindowResult::default()
        },
        SummaryArchiveSnapshotBackfillWindowResult {
            pending_obligation_count: 1,
            ..SummaryArchiveSnapshotBackfillWindowResult::default()
        },
        SummaryArchiveSnapshotBackfillWindowResult {
            hit_budget: true,
            ..SummaryArchiveSnapshotBackfillWindowResult::default()
        },
        SummaryArchiveSnapshotBackfillWindowResult {
            materialized_archive_batches: 1,
            ..SummaryArchiveSnapshotBackfillWindowResult::default()
        },
        SummaryArchiveSnapshotBackfillWindowResult {
            unavailable_archive_batches: 1,
            ..SummaryArchiveSnapshotBackfillWindowResult::default()
        },
    ] {
        assert!(summary_coverage_recovery_requires_second_v2_turn(&result));
    }
    assert!(!summary_coverage_recovery_requires_second_v2_turn(
        &SummaryArchiveSnapshotBackfillWindowResult::default()
    ));
}

#[tokio::test]
async fn summary_coverage_supervisor_idle_pass_skips_second_v2_window() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let (result, v2_window_calls) = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
        .scope(Cell::new(0), async {
            let result = SummaryCoverageRecoverySupervisor::run(state.as_ref()).await;
            let calls = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
                .try_with(Cell::get)
                .expect("V2 window counter scope");
            (result, calls)
        })
        .await;
    result.expect("idle recovery pass");
    assert_eq!(
        v2_window_calls, 1,
        "an idle recovery pass must issue only its initial V2 probe"
    );
    state.pool.close().await;
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

#[tokio::test]
async fn summary_v2_final_proof_covers_materialized_archive_boundary() {
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
async fn summary_coverage_supervisor_idle_ready_checkpoint_skips_checkpoint_advance() {
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
    let (result, v2_window_calls, checkpoint_advance_calls) =
        SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
            .scope(Cell::new(0), async {
                SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
                    .scope(Cell::new(0), async {
                        let result =
                            SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                                state.as_ref(),
                                None,
                            )
                            .await;
                        let v2_window_calls = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS
                            .try_with(Cell::get)
                            .expect("V2 window counter scope");
                        let checkpoint_advance_calls =
                            SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
                                .try_with(Cell::get)
                                .expect("checkpoint advance counter scope");
                        (result, v2_window_calls, checkpoint_advance_calls)
                    })
                    .await
            })
            .await;
    let next_turn = result.expect("idle recovery pass");
    assert_eq!(next_turn, SummaryCoverageRecoveryNextTurn::Idle);
    assert_eq!(v2_window_calls, 1, "idle recovery must issue one V2 probe");
    assert_eq!(
        checkpoint_advance_calls, 0,
        "idle recovery must not advance an unchanged ready checkpoint"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_coverage_supervisor_idle_settled_unavailable_checkpoint_skips_advance() {
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
    sqlx::query(
        "UPDATE summary_all_time_projection_checkpoint SET account_unavailable = 1, account_rollup_complete = 1, account_manifest_complete = 1, usage_rollup_complete = 1 WHERE scope = ?1",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .execute(&state.pool)
    .await
    .expect("mark account scope terminal unavailable");

    let (result, checkpoint_advance_calls) = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
        .scope(Cell::new(0), async {
            let result = SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await;
            let checkpoint_advance_calls = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
                .try_with(Cell::get)
                .expect("checkpoint advance counter scope");
            (result, checkpoint_advance_calls)
        })
        .await;
    assert_eq!(
        result.expect("terminal unavailable recovery pass"),
        SummaryCoverageRecoveryNextTurn::Idle
    );
    assert_eq!(
        checkpoint_advance_calls, 0,
        "terminal unavailable scope must not restart a settled checkpoint"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_coverage_supervisor_idle_checkpoint_republishes_when_required() {
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

    let mut projection = Arc::unwrap_or_clone(
        state
            .subscription_hub
            .summary_projection()
            .await
            .expect("load published Summary projection"),
    );
    projection.all_time_refreshed_at =
        Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state
        .subscription_hub
        .store_summary_projection(projection)
        .await;

    let (result, checkpoint_advance_calls) = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
        .scope(Cell::new(0), async {
            let result = SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await;
            let checkpoint_advance_calls = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS
                .try_with(Cell::get)
                .expect("checkpoint advance counter scope");
            (result, checkpoint_advance_calls)
        })
        .await;
    assert_eq!(
        result.expect("stale all-time publication pass"),
        SummaryCoverageRecoveryNextTurn::Idle
    );
    assert_eq!(
        checkpoint_advance_calls, 0,
        "republishing a ready checkpoint must not restart checkpoint advancement"
    );
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("load republished Summary projection");
    assert!(
        projection
            .all_time_refreshed_at
            .is_some_and(|refreshed_at| refreshed_at.elapsed() < SUMMARY_SNAPSHOT_MAX_STALE),
        "stale all-time projection must be refreshed before the idle short circuit"
    );
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

#[tokio::test]
async fn summary_coverage_supervisor_publishes_verified_30d_proof_without_http_interest() {
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
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &last_page.expect("last V2 proof page"))
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
    assert_eq!(response.total_count, MANIFEST_COUNT as i64);
    assert_eq!(response.total_tokens, (MANIFEST_COUNT * 2) as i64);
    state.pool.close().await;
}

#[tokio::test]
async fn summary_all_time_recovery_never_enters_live_exact_admission() {
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
        .expect("valid recovery archive bucket");
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
        .bind(format!("/definitely/missing/summary-recovery-{index}.sqlite.gz"))
        .bind(format!("summary-recovery-{index}"))
        .bind(&coverage_start)
        .bind(&coverage_end)
        .execute(&state.pool)
        .await
        .expect("seed recovery manifest");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling projection before all-time recovery");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create recovery interleave gate");
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    );
    let state_for_recovery = state.clone();
    let recovery = tokio::spawn(async move {
        SummaryCoverageRecoverySupervisor::run(state_for_recovery.as_ref()).await
    });
    tokio::pin!(recovery);
    tokio::select! {
        result = &mut recovery => {
            clear_summary_projection_test_interleave();
            result.expect("run summary coverage recovery").expect("recovery should not fail");
        }
        _ = interleave.wait_for_writer() => {
            interleave.resume_build();
            let _ = recovery.await;
            clear_summary_projection_test_interleave();
            assert_eq!(
                interleave.build_attempts(),
                0,
                "AllTime recovery must not enter generic paged raw hydration",
            );
        }
    }
    assert_eq!(
        interleave.build_attempts(),
        0,
        "AllTime recovery must not enter generic paged raw hydration",
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_projection_bootstrap_never_runs_paged_boundary_raw_hydration() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    for index in 0..51 {
        sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES (?1, datetime('now', ?2), 'proxy', 'success', 7, 0.7, '{}', '', 'full')",
        )
        .bind(format!("summary-boundary-overflow-live-{index}"))
        .bind(format!("-{index} seconds"))
        .execute(&state.pool)
        .await
        .expect("seed current live row");
    }

    let archive_bucket_start = Utc
        .timestamp_opt(
            align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(2) + ChronoDuration::hours(1)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("valid boundary archive bucket");
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
        .bind(format!("/definitely/missing/summary-boundary-overflow-{index}.sqlite.gz"))
        .bind(format!("summary-boundary-overflow-{index}"))
        .bind(&coverage_start)
        .bind(&coverage_end)
        .execute(&state.pool)
        .await
        .expect("seed bounded boundary manifest");
    }

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create paged boundary interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    );
    let state_for_hydration = state.clone();
    let hydration = tokio::spawn(async move {
        hydrate_summary_snapshots_with_deadline(
            state_for_hydration.as_ref(),
            Duration::from_secs(5),
        )
        .await
    });
    tokio::pin!(hydration);
    tokio::select! {
        result = &mut hydration => {
            clear_summary_projection_test_interleave();
            result
                .expect("run bootstrap hydration")
                .expect("bounded archive admission gaps must not abort projection hydration");
        }
        _ = interleave.wait_for_writer() => {
            interleave.resume_build();
            let _ = hydration.await;
            clear_summary_projection_test_interleave();
            panic!("bootstrap must not enter paged boundary raw archive hydration");
        }
    }
    assert_eq!(
        interleave.build_attempts(),
        0,
        "bootstrap must publish before the paged raw archive stage",
    );
    let rolling_interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    );
    let state_for_rolling = state.clone();
    let rolling = tokio::spawn(async move {
        refresh_summary_snapshots_with_mode(
            state_for_rolling.as_ref(),
            SummaryProjectionBuildMode::Rolling,
        )
        .await
    });
    tokio::pin!(rolling);
    tokio::select! {
        result = &mut rolling => {
            clear_summary_projection_test_interleave();
            result
                .expect("run rolling hydration")
                .expect("rolling reconciliation must not open paged raw archives");
        }
        _ = rolling_interleave.wait_for_writer() => {
            rolling_interleave.resume_build();
            let _ = rolling.await;
            clear_summary_projection_test_interleave();
            panic!("rolling reconciliation must not enter paged boundary raw archive hydration");
        }
    }
    assert_eq!(
        rolling_interleave.build_attempts(),
        0,
        "rolling reconciliation must publish without the paged raw archive stage",
    );
    // The externally observable contract below is the proof: disjoint current/1d reads
    // remain exact after SQLite closes, while the affected 7d selection is unavailable.
    let all_time_interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforePagedBoundaryArchiveHydration,
    );
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("coverage supervisor should defer missing raw archives");
    clear_summary_projection_test_interleave();
    assert_eq!(
        all_time_interleave.build_attempts(),
        0,
        "AllTime supervisor must not enter the generic paged raw projection builder",
    );
    state.pool.close().await;

    for window in ["current", "1d"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("disjoint summary selection remains exact without SQLite");
        let expected_count = if window == "current" { 50 } else { 51 };
        assert_eq!(response.total_count, expected_count, "window {window}");
        assert_eq!(response.total_tokens, expected_count * 7, "window {window}");
        assert_f64_close(response.total_cost, expected_count as f64 * 0.7);
    }

    let affected = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(affected, Err(ApiError::Unavailable(_))),
        "a rolling range intersecting the unadmitted archive must fail closed"
    );
}

#[tokio::test]
async fn summary_projection_rolling_never_runs_historical_live_coverage() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-coverage-before', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical live coverage");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection with historical coverage");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-coverage-current', datetime('now'), 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("advance only the current live source");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create historical coverage interleave gate");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling projection before AllTime coverage recovery");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
    );
    let refresh_state = state.clone();
    let rolling = tokio::spawn(async move {
        refresh_summary_snapshots_with_mode(
            refresh_state.as_ref(),
            SummaryProjectionBuildMode::Rolling,
        )
        .await
    });
    tokio::pin!(rolling);
    tokio::select! {
        result = &mut rolling => {
            clear_summary_projection_test_interleave();
            result
                .expect("join rolling refresh")
                .expect("rolling refresh must publish without a historical full scan");
        }
        _ = interleave.wait_for_writer() => {
            interleave.resume_build();
            let _ = rolling.await;
            clear_summary_projection_test_interleave();
            panic!("rolling refresh must not enter historical live coverage");
        }
    }
    assert_eq!(
        interleave.build_attempts(),
        0,
        "only Bootstrap or background recovery may scan historical live coverage",
    );
    state.pool.close().await;
    let Json(current) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("rolling projection remains memory-only after SQLite closes");
    assert_eq!(current.total_count, 2);
    assert_eq!(current.total_tokens, 40);
}

#[tokio::test]
async fn summary_projection_all_time_never_runs_historical_live_coverage() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-all-time-historical-coverage', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical live source");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create historical coverage interleave gate");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling projection before AllTime coverage recovery");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
    );
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("AllTime recovery must use coverage proof, not a historical live scan");
    clear_summary_projection_test_interleave();
    assert_eq!(
        interleave.build_attempts(),
        0,
        "only Bootstrap or the dedicated historical recovery may scan historical live data",
    );
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
    .expect("AllTime live-only aggregate remains exact");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
}

#[tokio::test]
async fn summary_projection_live_rollup_change_does_not_restart_historical_coverage() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let before = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load initial coverage revision");
    let account_before = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load initial account coverage revision");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) \
         VALUES (1, 'proxy', 1, 1, 0, 5, 0.25)",
    )
    .execute(&state.pool)
    .await
    .expect("insert changed rollup proof");
    let after = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load updated coverage revision");
    let account_after = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load unchanged account coverage revision");
    assert_eq!(
        after, before,
        "live rollup progress belongs to the live-tail cursor, not the historical coverage fence"
    );
    assert_eq!(
        account_after, account_before,
        "a global-only rollup change must not restart account coverage"
    );
}

#[tokio::test]
async fn summary_projection_live_account_rollup_does_not_restart_historical_coverage() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let global_before = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load initial global coverage revision");
    let account_before = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load initial account coverage revision");
    sqlx::query(
        "INSERT INTO upstream_account_stats_hourly \
         (bucket_start_epoch, source, upstream_account_id, total_count, success_count, \
          failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (1, 'proxy', 42, 1, 1, 0, 5, 0.25, 0)",
    )
    .execute(&state.pool)
    .await
    .expect("insert account-scoped rollup proof");
    let global_after = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load unchanged global coverage revision");
    let account_after = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load updated account coverage revision");
    assert_eq!(
        global_after, global_before,
        "an account-only rollup change must not restart global coverage"
    );
    assert_eq!(
        account_after, account_before,
        "live account rollup progress belongs to the account live-tail cursor"
    );
}

#[tokio::test]
async fn summary_projection_archive_replay_advances_historical_coverage_fence() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let global_before = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load initial global coverage revision");
    let account_before = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load initial account coverage revision");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay \
         (target, dataset, file_path, archive_sha256) \
         VALUES (?1, 'codex_invocations', '/archive/coverage-fence.sqlite.gz', 'coverage-fence')",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .execute(&state.pool)
    .await
    .expect("insert archive replay proof");
    let global_after = summary_projection_coverage_revision(&state.pool)
        .await
        .expect("load updated global coverage revision");
    let account_after = summary_projection_account_coverage_revision(&state.pool)
        .await
        .expect("load updated account coverage revision");
    assert!(
        global_after > global_before,
        "archive replay proof must advance global historical coverage"
    );
    assert!(
        account_after > account_before,
        "archive replay proof must advance account historical coverage"
    );
}

#[tokio::test]
async fn summary_projection_bootstrap_timeout_publishes_recent_exact_projection_and_local_gap() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-cold-retry-historical', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical source that Bootstrap must cover");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create cold retry interleave gate");

    let initial_interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
    );
    let initial_state = state.clone();
    let initial = tokio::spawn(async move {
        summary_projection_lifecycle::observe_bootstrap_timeout(
            hydrate_summary_snapshots_with_deadline(initial_state.as_ref(), Duration::from_secs(1)),
        )
        .await
    });
    tokio::pin!(initial);
    let initial_result = tokio::select! {
        _ = initial_interleave.wait_for_writer() => initial.await,
        result = &mut initial => result,
    };
    clear_summary_projection_test_interleave();
    let (initial_result, timed_out) = initial_result.expect("join timed-out Bootstrap");
    initial_result.expect("timed-out Bootstrap must publish its bounded fallback");
    assert!(
        timed_out,
        "cold Bootstrap must hit its actual build deadline"
    );
    let fallback_fence = state
        .subscription_hub
        .summary_bootstrap_fallback_fence()
        .await;
    assert!(
        fallback_fence.is_some(),
        "cold Bootstrap must have published its bounded fallback"
    );
    assert!(
        state.subscription_hub.summary_projection().await.is_some(),
        "a timed-out Bootstrap must publish a bounded Projection"
    );

    state.pool.close().await;
    let Json(current) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("current must be served from the fallback Projection");
    assert_eq!(
        current.total_count, 1,
        "current remains exact from the bounded live source"
    );
    let Json(recent) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("recent rolling selection must be served from memory");
    assert_eq!(
        recent.total_count, 0,
        "recent rolling selection remains exact"
    );
    let historical = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("the timed-out historical range must remain unavailable");
    assert!(
        matches!(historical, ApiError::Unavailable(_)),
        "historical fallback error must be explicit: {historical:?}"
    );
}

#[tokio::test]
async fn summary_projection_unexpected_bootstrap_error_is_not_suppressed() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query("DROP TABLE upstream_account_stats_hourly")
        .execute(&state.pool)
        .await
        .expect("drop rollup table to force an unexpected Bootstrap error");

    let error = hydrate_summary_snapshots_with_deadline(&state, Duration::from_secs(1))
        .await
        .expect_err("schema errors must propagate instead of publishing a fallback");
    assert!(
        error.to_string().contains("summary projection")
            || error.to_string().contains("no such table"),
        "unexpected Bootstrap error must remain visible: {error:?}"
    );
    assert!(
        state.subscription_hub.summary_projection().await.is_none(),
        "unexpected Bootstrap errors must not publish a Projection"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_projection_rolling_localizes_late_historical_live_source_gap() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-gap-before', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("seed initial historical source");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection with historical proof");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-gap-late', ?1, 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("insert a late historical source after bootstrap proof");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::Rolling)
        .await
        .expect("late historical source must not abort rolling publication");
    state.pool.close().await;

    for window in ["current", "1d"] {
        let _ = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("disjoint {window} selection remains exact from memory");
    }
    let affected = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(affected, Err(ApiError::Unavailable(_))),
        "only a selection intersecting the late historical source gap may fail closed",
    );
}

#[tokio::test]
async fn summary_projection_refresh_rejects_legacy_source_gap() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-recovery-before', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical live coverage");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection with historical proof");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-recovery-late', ?1, 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("insert late historical source");
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("rolling refresh must not enter the historical full scan");
    let Json(current) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("rolling projection is published before historical recovery completes");
    assert_eq!(current.total_count, 2);
    let affected = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(affected, Err(ApiError::Unavailable(_))),
        "a scoped legacy source gap must remain unavailable for its affected historical range"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn summary_projection_historical_rollup_clears_stale_live_admission_gap() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = Utc::now() - ChronoDuration::days(3);
    let occurred_at_db = db_occurred_at_lower_bound(occurred_at);
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-stale-live-gap', ?1, 'proxy', 'success', 7, 0.7, '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .bind(&occurred_at_db)
    .execute(&state.pool)
    .await
    .expect("insert historical live row");
    let row_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id = 'summary-stale-live-gap'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load historical live row id");
    let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
    let range = ExactUtcRange {
        start: occurred_at - ChronoDuration::minutes(1),
        end: occurred_at + ChronoDuration::minutes(1),
    };
    let mut unavailable_global_buckets = BTreeSet::from([bucket]);
    let mut unavailable_account_buckets = HashMap::from([(42_i64, BTreeSet::from([bucket]))]);
    let mut covered_terminal_ids = HashSet::new();
    let totals = StatsTotals {
        total_count: 1,
        success_count: 1,
        total_tokens: 7,
        total_cost: 0.7,
        ..StatsTotals::default()
    };
    let usage = UsageBreakdownResponse {
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        output_tokens: 7,
        costs: None,
        models: Vec::new(),
    };

    mark_summary_projection_uncovered_historical_live_ranges(
        &state.pool,
        range,
        row_id,
        row_id,
        Some(row_id),
        &HashMap::from([((bucket, None), totals), ((bucket, Some(42)), totals)]),
        &HashMap::from([((bucket, None), usage.clone()), ((bucket, Some(42)), usage)]),
        &BTreeSet::new(),
        &HashSet::new(),
        &mut unavailable_global_buckets,
        &mut unavailable_account_buckets,
        &mut covered_terminal_ids,
    )
    .await
    .expect("durable rollup proof should be accepted");

    assert!(
        !unavailable_global_buckets.contains(&bucket),
        "a committed global rollup must clear a stale resident-budget gap"
    );
    assert!(
        unavailable_account_buckets
            .get(&42)
            .is_none_or(|buckets| !buckets.contains(&bucket)),
        "a committed account rollup must clear its scoped stale gap"
    );
}

#[tokio::test]
async fn summary_projection_legacy_historical_gap_stays_selection_local() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-retry-before', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical live coverage");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection with historical proof");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-historical-retry-late', ?1, 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
    )
    .bind(&historical_at)
    .execute(&state.pool)
    .await
    .expect("insert late historical source");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::Rolling)
        .await
        .expect("publish localized historical gap");
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("legacy historical gap is handled without generic rebuild");
    state.pool.close().await;
    let affected = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: Some(50),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(affected, Err(ApiError::Unavailable(_))),
        "a legacy live row without durable identity must remain unavailable only for its affected range"
    );
}

#[test]
fn summary_unavailable_archive_ranges_compact_many_boundary_fragments() {
    let start = Utc
        .timestamp_opt(align_bucket_epoch(1_700_000_000, 3_600, 0), 0)
        .single()
        .expect("valid fixed test timestamp");
    let mut buckets = BTreeSet::new();
    for fragment in 0..50_001_i64 {
        let fragment_start = start + ChronoDuration::milliseconds(fragment);
        summary_projection_mark_unavailable_archive_ranges(
            &mut buckets,
            [ExactUtcRange {
                start: fragment_start,
                end: fragment_start + ChronoDuration::milliseconds(1),
            }],
        )
        .expect("many fragments in one boundary bucket remain bounded");
    }
    assert_eq!(buckets.len(), 1);
    let ranges = summary_projection_unavailable_bucket_ranges(buckets);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].end - ranges[0].start, ChronoDuration::hours(1));

    let mut distinct_buckets = BTreeSet::new();
    for offset in 0..SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
        let bucket_start = start + ChronoDuration::hours(offset as i64);
        summary_projection_mark_unavailable_archive_ranges(
            &mut distinct_buckets,
            [ExactUtcRange {
                start: bucket_start,
                end: bucket_start + ChronoDuration::hours(1),
            }],
        )
        .expect("the configured unavailable-bucket budget is accepted exactly");
    }
    let next_bucket = start + ChronoDuration::hours(SUMMARY_PROJECTION_MAX_EXACT_BUCKETS as i64);
    let error = summary_projection_mark_unavailable_archive_ranges(
        &mut distinct_buckets,
        [ExactUtcRange {
            start: next_bucket,
            end: next_bucket + ChronoDuration::hours(1),
        }],
    )
    .expect_err("a distinct bucket beyond the unavailable-range budget must fail closed");
    assert!(
        error
            .to_string()
            .contains("unavailable archive range budget")
    );
}

#[test]
fn summary_projection_rolling_refresh_keeps_all_time_terminal_watermark() {
    assert_eq!(
        summary_projection_all_time_sequence_watermark(false, true, 12, 7),
        7,
        "rolling-only refreshes must not claim new all-time terminal coverage",
    );
    assert_eq!(
        summary_projection_all_time_sequence_watermark(true, false, 12, 7),
        7,
        "incomplete all-time rebuilds must retain the prior coverage proof",
    );
    assert_eq!(
        summary_projection_all_time_sequence_watermark(true, true, 12, 7),
        12,
        "only complete all-time hydration may advance its terminal watermark",
    );
}

#[test]
fn summary_projection_all_time_terminal_coverage_is_scope_specific() {
    assert!(summary_projection_all_time_scope_covered(true, 7, 7, false));
    assert!(!summary_projection_all_time_scope_covered(
        false, 7, 7, false
    ));
    assert!(summary_projection_all_time_scope_covered(true, 7, 12, true));
    assert!(!summary_projection_all_time_scope_covered(
        true, 7, 12, false
    ));

    let mut projection = SummaryProjection {
        all_time_terminal_coverage_complete: true,
        all_time_terminal_sequence_watermark: 7,
        ..SummaryProjection::default()
    };
    projection
        .all_time_persisted_live_terminal_invoke_ids
        .insert("race\0timestamp".to_string());
    assert!(projection.all_time_terminal_scope_covers(None, "race", "timestamp", 12));
    assert!(
        !projection.all_time_terminal_scope_covers(Some(42), "race", "timestamp", 7),
        "global all-time proof must never cover an account scope"
    );
    projection
        .all_time_account_terminal_sequence_watermarks
        .insert(42, 7);
    assert!(projection.all_time_terminal_scope_covers(Some(42), "race", "timestamp", 7));
}

#[test]
fn summary_snapshot_failure_backoff_coalesces_retries() {
    let now = Instant::now();
    let retry_at = summary_snapshot_retry_not_before(now);
    assert!(!summary_snapshot_refresh_is_due(
        Some(now - SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL),
        Some(retry_at),
        now,
    ));
    assert!(summary_snapshot_refresh_is_due(
        Some(now - SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL),
        Some(retry_at),
        retry_at,
    ));
}

#[test]
fn summary_snapshot_cadence_respects_refresh_floor() {
    let now = Instant::now();
    assert!(!summary_snapshot_refresh_is_due(
        Some(now - Duration::from_secs(4)),
        None,
        now,
    ));
    assert!(summary_snapshot_refresh_is_due(
        Some(now - SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL),
        None,
        now,
    ));
}

#[test]
fn summary_snapshot_refresh_budget_completes_before_freshness_ceiling() {
    assert!(
        SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL
            + SUMMARY_SNAPSHOT_REFRESH_INTERVAL
            + SUMMARY_SNAPSHOT_COORDINATION_SLACK
            + SUMMARY_PROJECTION_BUILD_DEADLINE
            < SUMMARY_SNAPSHOT_MAX_STALE
    );
}

#[tokio::test]
async fn summary_projection_scopes_historical_terminal_coverage_before_releasing_sse_identity() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES (1, 'summary-historical-terminal-coverage', datetime('now', '-3 days'), 'proxy', 'success', 17, 1.25, '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert historical persisted terminal");
    sqlx::query(
        "WITH RECURSIVE rows(value) AS ( \
             SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1 \
         ) \
         INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         SELECT 'summary-current-prefix-' || value, datetime('now'), 'proxy', 'running', 0, 0, '{\"upstreamAccountId\":42}', '', 'full' \
         FROM rows",
    )
    .bind((state.config.list_limit_max + 1) as i64)
    .execute(&state.pool)
    .await
    .expect("fill the bounded current prefix ahead of the historical terminal");
    let occurred_at: String = sqlx::query_scalar(
        "SELECT occurred_at FROM codex_invocations WHERE invoke_id = 'summary-historical-terminal-coverage'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load historical terminal timestamp");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate with an uncovered historical terminal");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("projection with historical coverage gap");
    assert!(
        !projection.contains_persisted_live_terminal(
            "summary-historical-terminal-coverage",
            &occurred_at,
        ),
        "a global coverage gap must retain the Summary SSE terminal overlay",
    );
    for upstream_account_id in [None, Some(42)] {
        let response = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("7d".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id,
            }),
        )
        .await;
        assert!(
            matches!(response, Err(ApiError::Unavailable(_))),
            "an uncovered global terminal must fail closed for {upstream_account_id:?}",
        );
    }
    state.pool.close().await;

    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES (1, 'summary-historical-terminal-coverage', datetime('now', '-3 days'), 'proxy', 'success', 17, 1.25, '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert historical persisted terminal with global coverage");
    sqlx::query(
        "WITH RECURSIVE rows(value) AS ( \
             SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1 \
         ) \
         INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         SELECT 'summary-current-prefix-' || value, datetime('now'), 'proxy', 'running', 0, 0, '{\"upstreamAccountId\":42}', '', 'full' \
         FROM rows",
    )
    .bind((state.config.list_limit_max + 1) as i64)
    .execute(&state.pool)
    .await
    .expect("fill the bounded current prefix ahead of the covered historical terminal");
    let occurred_at: String = sqlx::query_scalar(
        "SELECT occurred_at FROM codex_invocations WHERE invoke_id = 'summary-historical-terminal-coverage'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load globally covered terminal timestamp");
    let bucket = align_bucket_epoch(
        parse_to_utc_datetime(&occurred_at)
            .expect("parse globally covered terminal timestamp")
            .timestamp(),
        3_600,
        0,
    );

    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert complete global compact bucket");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_output, has_cost) \
         VALUES (?1, 'proxy', '42', 'gpt-5', 'high', 1, 17, 1.25, 1)",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert complete global usage compact bucket");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES ('codex_invocations_summary_rollup_v2_live_cursor', 1, datetime('now')) \
         ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = datetime('now')",
    )
    .execute(&state.pool)
    .await
    .expect("advance global live cursor");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("rehydrate with only the account coverage gap");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("projection with account-only historical coverage gap");
    assert!(
        projection.contains_persisted_live_terminal(
            "summary-historical-terminal-coverage",
            &occurred_at,
        ),
        "an account-only gap must not keep a global Summary SSE overlay that would double count",
    );

    let Json(global) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("global compact coverage remains exact");
    assert_eq!(global.total_count, state.config.list_limit_max as i64 + 2);
    assert_eq!(global.total_tokens, 17);

    let account = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await;
    assert!(matches!(account, Err(ApiError::Unavailable(_))));
    state.pool.close().await;
}

#[tokio::test]
async fn summary_projection_full_historical_identity_admission_uses_bounded_population() {
    with_summary_projection_test_exact_record_limit(8, async {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let projection_end = Utc::now() + ChronoDuration::seconds(1);
        let protected = summary_projection_boundary_buckets(projection_end);
        let (historical_at, bucket) = (0_i64..32)
            .find_map(|index| {
                let candidate = db_occurred_at_lower_bound(
                    projection_end
                        - ChronoDuration::hours(72 + index * 3)
                        - ChronoDuration::minutes(30),
                );
                let bucket = align_bucket_epoch(
                    parse_to_utc_datetime(&candidate)?.timestamp(),
                    3_600,
                    0,
                );
                (!(-1_i64..=1).any(|delta| protected.contains(&(bucket + delta * 3_600))))
                    .then_some((candidate, bucket))
            })
            .expect("bounded identity fixture must avoid every Summary boundary bucket");
        sqlx::query(
            "WITH RECURSIVE rows(value) AS (\
                 SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1\
             )\
             INSERT INTO codex_invocations\
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)\
             SELECT 'summary-bounded-identity-' || value, ?2, 'proxy', 'success', 1, 0.1, '{}', '', 'full'\
             FROM rows",
        )
        .bind(9_i64)
        .bind(&historical_at)
        .execute(&state.pool)
        .await
        .expect("seed historical terminal identity population");
        let high_watermark_id: i64 =
            sqlx::query_scalar("SELECT MAX(id) FROM codex_invocations")
                .fetch_one(&state.pool)
                .await
                .expect("load historical identity high watermark");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly\
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost)\
             VALUES (?1, 'proxy', 9, 9, 0, 9, 0.9, 0)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("seed complete global rollup");
        sqlx::query(
            "INSERT INTO upstream_account_usage_breakdown_hourly\
             (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort,\
              request_count, output_tokens, cost_output, has_cost)\
             VALUES (?1, 'proxy', 'none', 'gpt-5', 'high', 9, 9, 0.9, 1)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("seed complete global usage rollup");
        sqlx::query(
            "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)\
             VALUES ('codex_invocations_summary_rollup_v2_live_cursor', ?1, datetime('now'))\
             ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = excluded.updated_at",
        )
        .bind(high_watermark_id)
        .execute(&state.pool)
        .await
        .expect("advance global live cursor beyond historical identities");
        let (result, hydrated_rows) =
            SUMMARY_PROJECTION_TEST_HISTORICAL_IDENTITY_HYDRATION_ROWS
                .scope(Cell::new(0), async {
                    let result = hydrate_summary_snapshots(state.as_ref()).await;
                    let hydrated_rows = SUMMARY_PROJECTION_TEST_HISTORICAL_IDENTITY_HYDRATION_ROWS
                        .try_with(Cell::get)
                        .expect("historical identity hydration counter scope");
                    (result, hydrated_rows)
                })
                .await;
        result.expect("bounded historical identity admission must hydrate");
        assert_eq!(
            hydrated_rows, 0,
            "a zero tail must not admit the over-budget historical identity population"
        );
        state.pool.close().await;
        let Json(response) = fetch_summary(
            State(state),
            Query(SummaryQuery {
                window: Some("7d".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("compact rollup must keep the covered historical Summary exact");
        assert_eq!(response.total_count, 9);
        assert_eq!(response.total_tokens, 9);
    })
    .await;
}

#[tokio::test]
async fn summary_projection_overflow_pending_identity_uses_primary_key_lookup() {
    with_summary_projection_test_exact_record_limit(1, async {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
        for (invoke_id, status) in [
            ("summary-overflow-pending", "success"),
            ("summary-overflow-other", "success"),
            ("summary-overflow-running", "running"),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations \
                 (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
                 VALUES (?1, ?2, 'proxy', ?3, 1, 0.1, '{}', '', 'full')",
            )
            .bind(invoke_id)
            .bind(&occurred_at)
            .bind(status)
            .execute(&state.pool)
            .await
            .expect("seed overflow terminal identity");
        }
        let pending_row_id: i64 = sqlx::query_scalar(
            "SELECT id FROM codex_invocations WHERE invoke_id = 'summary-overflow-pending'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load pending terminal row id");
        let running_row_id: i64 = sqlx::query_scalar(
            "SELECT id FROM codex_invocations WHERE invoke_id = 'summary-overflow-running'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load running row id");
        let high_watermark_id: i64 =
            sqlx::query_scalar("SELECT MAX(id) FROM codex_invocations")
                .fetch_one(&state.pool)
                .await
                .expect("load terminal high watermark");
        let occurred_at_utc = parse_to_utc_datetime(&occurred_at).expect("parse occurred_at");
        let bucket = align_bucket_epoch(occurred_at_utc.timestamp(), 3_600, 0);
        let range = ExactUtcRange {
            start: occurred_at_utc - ChronoDuration::hours(1),
            end: occurred_at_utc + ChronoDuration::hours(1),
        };
        let totals = StatsTotals {
            total_count: 2,
            success_count: 2,
            total_tokens: 2,
            total_cost: 0.2,
            ..StatsTotals::default()
        };
        let usage = UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 2,
            costs: None,
            models: Vec::new(),
        };
        let mut unavailable_global_buckets = BTreeSet::from([bucket]);
        let mut unavailable_account_buckets = HashMap::new();
        let mut covered_terminal_ids = HashSet::new();
        let pending_terminal_identities = HashSet::from([
            format!(
                "{pending_row_id}\0{occurred_at}\0summary-overflow-pending"
            ),
            "summary-overflow-pending\0malformed".to_string(),
            format!("{pending_row_id}\0{occurred_at}\0changed-invoke-id"),
            format!("{running_row_id}\0{occurred_at}\0summary-overflow-running"),
        ]);

        mark_summary_projection_uncovered_historical_live_ranges(
            &state.pool,
            range,
            high_watermark_id,
            high_watermark_id,
            Some(high_watermark_id),
            &HashMap::from([((bucket, None), totals)]),
            &HashMap::from([((bucket, None), usage)]),
            &BTreeSet::new(),
            &pending_terminal_identities,
            &mut unavailable_global_buckets,
            &mut unavailable_account_buckets,
            &mut covered_terminal_ids,
        )
        .await
        .expect("overflow pending identity lookup should succeed");

        assert!(covered_terminal_ids.contains(&summary_projection_source_identity_key(
            pending_row_id,
            "summary-overflow-pending",
            &occurred_at,
        )));
        assert_eq!(covered_terminal_ids.len(), 1);
        state.pool.close().await;
    })
    .await;
}

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

#[tokio::test]
async fn summary_projection_requires_account_cursor_for_persisted_live_coverage() {
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

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate scope-aware projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile the live-only account all-time projection");
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
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("rehydrate without account cursor coverage");
    state.pool.close().await;

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
