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
    let recovery =
        tokio::spawn(async move { refresh_summary_snapshots(state_for_recovery.as_ref()).await });
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

async fn seed_summary_projection_bootstrap_boundary_fixture(state: &Arc<AppState>) {
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
}

async fn assert_summary_projection_published_memory_windows(state: Arc<AppState>) {
    for window in ["current", "1d", "7d", "30d", "today"] {
        let _ = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .unwrap_or_else(|error| {
            panic!("{window} must be served from the published memory Projection: {error:?}")
        });
    }
}

fn assert_summary_projection_bootstrap_retry_modes(interleave: &SummaryProjectionTestInterleave) {
    assert_eq!(
        interleave.build_attempts(),
        2,
        "the cold retry must reach Bootstrap coverage and publication in order",
    );
    assert_eq!(
        interleave.build_modes(),
        vec![
            SummaryProjectionBuildMode::Bootstrap,
            SummaryProjectionBuildMode::Bootstrap,
        ],
        "a cold retry must remain Bootstrap through the publication tail",
    );
}

async fn assert_summary_projection_bootstrap_boundary_selections(state: Arc<AppState>) {
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
async fn summary_projection_bootstrap_never_runs_paged_boundary_raw_hydration() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    seed_summary_projection_bootstrap_boundary_fixture(&state).await;

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

    assert_summary_projection_bootstrap_boundary_selections(state).await;
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
async fn summary_projection_cold_retry_uses_bootstrap_deadline() {
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
        hydrate_summary_snapshots_with_deadline(initial_state.as_ref(), Duration::from_secs(1))
            .await
    });
    tokio::pin!(initial);
    tokio::select! {
        _ = initial_interleave.wait_for_writer() => {}
        result = &mut initial => {
            clear_summary_projection_test_interleave();
            result
                .expect("join cold Bootstrap timeout")
                .expect_err("cold Bootstrap must time out while its historical stage is paused");
            panic!("cold Bootstrap must reach the deterministic timeout stage");
        }
    }
    initial
        .await
        .expect("join timed-out Bootstrap")
        .expect_err("timed-out Bootstrap must not publish");
    clear_summary_projection_test_interleave();
    assert!(
        state.subscription_hub.summary_projection().await.is_none(),
        "a timed-out Bootstrap must leave the hub without a Projection"
    );

    let retry_interleave = install_summary_projection_test_interleave_for_stages(&[
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    ]);
    let retry_state = state.clone();
    let retry = tokio::spawn(async move { refresh_summary_snapshots(retry_state.as_ref()).await });
    tokio::pin!(retry);
    tokio::select! {
        _ = retry_interleave.wait_for_writer() => {}
        result = &mut retry => {
            clear_summary_projection_test_interleave();
            result
                .expect("join cold retry")
                .expect("cold retry must not fail before selecting a build mode");
            panic!("a cold maintenance retry must select Bootstrap, not Rolling");
        }
    }
    retry_interleave.resume_build();
    tokio::select! {
        _ = retry_interleave.wait_for_writer() => {}
        result = &mut retry => {
            clear_summary_projection_test_interleave();
            result
                .expect("join cold retry")
                .expect("cold retry must not fail before Projection publication");
            panic!("cold Bootstrap retry must reach the publication tail");
        }
    }
    retry_interleave.resume_build();
    retry
        .await
        .expect("join cold Bootstrap retry")
        .expect("cold Bootstrap retry must publish a Projection");
    clear_summary_projection_test_interleave();
    assert_summary_projection_bootstrap_retry_modes(&retry_interleave);

    state.pool.close().await;
    assert_summary_projection_published_memory_windows(state).await;
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
async fn summary_projection_refresh_keeps_historical_gap_local_until_supervisor_proof() {
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
        "the localized gap remains fail-closed until background coverage completes",
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
        SummaryProjectionHistoricalLiveCoverageInput {
            pool: &state.pool,
            range,
            high_watermark_id: row_id,
            rollup_live_cursor: row_id,
            account_rollup_live_cursor: Some(row_id),
            hourly_rollup_totals: &HashMap::from([
                ((bucket, None), totals),
                ((bucket, Some(42)), totals),
            ]),
            hourly_rollup_usage: &HashMap::from([
                ((bucket, None), usage.clone()),
                ((bucket, Some(42)), usage),
            ]),
            fully_admitted_live_buckets: &BTreeSet::new(),
            pending_terminal_identities: &HashSet::new(),
            unavailable_global_buckets: &mut unavailable_global_buckets,
            unavailable_account_buckets: &mut unavailable_account_buckets,
            global_covered_terminal_invoke_ids: &mut covered_terminal_ids,
        },
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

async fn assert_summary_projection_historical_terminal_uncovered() {
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
}

async fn prepare_summary_projection_historical_terminal_account_gap() -> (Arc<AppState>, String) {
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

    (state, occurred_at)
}

async fn assert_summary_projection_historical_terminal_account_gap(state: Arc<AppState>) {
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
async fn summary_projection_scopes_historical_terminal_coverage_before_releasing_sse_identity() {
    assert_summary_projection_historical_terminal_uncovered().await;
    let (state, _) = prepare_summary_projection_historical_terminal_account_gap().await;
    assert_summary_projection_historical_terminal_account_gap(state).await;
}
