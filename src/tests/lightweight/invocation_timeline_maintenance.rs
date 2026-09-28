use super::*;

#[tokio::test]
async fn invocation_timeline_snapshot_cleanup_runs_off_the_first_page_path_in_bounded_batches() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let natural_day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let natural_day_end = natural_day_start + chrono::Duration::days(1);
    let range_start = now - chrono::Duration::minutes(30);
    let range_end = now;

    let _ = fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            natural_day_start: Some(format_utc_iso(natural_day_start)),
            natural_day_end: Some(format_utc_iso(natural_day_end)),
            from: format_utc_iso(range_start),
            to: format_utc_iso(range_end),
            include_live: Some(false),
            ..Default::default()
        }),
    )
    .await
    .expect("initialize timeline snapshot table");

    sqlx::query(
        "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES ('legacy-stale', 'stale', ?1, 1, 0, 0, '{}')",
    )
    .bind(format_utc_iso(range_start))
    .execute(&state.pool)
    .await
    .expect("insert stale snapshot row before first page");

    let Json(response) = fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            natural_day_start: Some(format_utc_iso(natural_day_start)),
            natural_day_end: Some(format_utc_iso(natural_day_end)),
            from: format_utc_iso(range_start),
            to: format_utc_iso(range_end),
            include_live: Some(false),
            ..Default::default()
        }),
    )
    .await
    .expect("first timeline page should succeed without global cleanup");

    let stale_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = 'legacy-stale'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale snapshot rows after request");
    assert_eq!(
        stale_count, 1,
        "the HTTP request must leave stale rows alone"
    );

    sqlx::query(
        "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, 'active', ?2, 1, 0, 0, '{}')",
    )
    .bind(&response.as_of)
    .bind(format_utc_iso(range_start))
    .execute(&state.pool)
    .await
    .expect("insert active snapshot row");
    for index in 0..69 {
        sqlx::query(
            "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, ?2, ?3, 1, 0, 0, '{}')",
        )
        .bind(format!("legacy-stale-{index:03}"))
        .bind(format!("stale-{index}"))
        .bind(format_utc_iso(range_start))
        .execute(&state.pool)
        .await
        .expect("insert bounded cleanup fixture");
    }

    let mut first_cleanup = None;
    for _ in 0..8 {
        let result = cleanup_timeline_snapshot_rows_once(&state.pool)
            .await
            .expect("run bounded snapshot cleanup");
        if result.skipped.is_none() {
            first_cleanup = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let first_cleanup = first_cleanup.expect("cleanup should be admitted");
    assert!(first_cleanup.scanned_tokens <= 64);
    assert!(first_cleanup.deleted_tokens <= 64);

    for _ in 0..8 {
        let remaining_stale: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token LIKE 'legacy-stale%'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count stale rows during cleanup");
        if remaining_stale == 0 {
            break;
        }
        let _ = cleanup_timeline_snapshot_rows_once(&state.pool)
            .await
            .expect("continue bounded snapshot cleanup");
    }

    let remaining_stale: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token LIKE 'legacy-stale%'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale rows after cleanup");
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = ?1",
    )
    .bind(&response.as_of)
    .fetch_one(&state.pool)
    .await
    .expect("count active snapshot row");
    assert_eq!(
        remaining_stale, 0,
        "bounded passes should reclaim stale rows"
    );
    assert_eq!(active_count, 1, "cleanup must preserve a live cursor token");
    state.pool.close().await;
}

#[tokio::test]
async fn invocation_timeline_cleanup_preserves_snapshot_materialized_after_token_scan() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let natural_day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let natural_day_end = natural_day_start + chrono::Duration::days(1);
    let range_start = now - chrono::Duration::minutes(2);
    let range_end = now;

    let _ = fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            natural_day_start: Some(format_utc_iso(natural_day_start)),
            natural_day_end: Some(format_utc_iso(natural_day_end)),
            from: format_utc_iso(range_start),
            to: format_utc_iso(range_end),
            include_live: Some(false),
            ..Default::default()
        }),
    )
    .await
    .expect("initialize timeline snapshot table");

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('cleanup-race-request', ?1, 'proxy', 'success', 100, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(range_start))
    .execute(&state.pool)
    .await
    .expect("insert invocation used by the concurrent first-page request");
    sqlx::query(
        "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES ('~cleanup-race-trigger', 'stale-trigger', ?1, 1, 0, 0, '{}')",
    )
    .bind(format_utc_iso(range_start))
    .execute(&state.pool)
    .await
    .expect("insert a stale token so cleanup reaches the interleaving point");

    let mut cleanup_task = None;
    let mut cleanup_release = None;
    for _ in 0..8 {
        let (entered, release) = install_timeline_snapshot_cleanup_pause_for_test();
        let cleanup_pool = state.pool.clone();
        let mut task =
            tokio::spawn(async move { cleanup_timeline_snapshot_rows_once(&cleanup_pool).await });
        let reached_interleaving = tokio::select! {
            _ = entered.notified() => true,
            result = &mut task => {
                let result = result.expect("cleanup task should not panic").expect("cleanup should complete");
                assert!(result.skipped.is_some(), "an admitted cleanup should pause after scanning tokens");
                false
            }
            _ = tokio::time::sleep(Duration::from_secs(3)) => {
                panic!("cleanup did not reach the token-scan interleaving point");
            }
        };
        if reached_interleaving {
            cleanup_task = Some(task);
            cleanup_release = Some(release);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut cleanup_task = cleanup_task.expect("cleanup should be admitted");
    let cleanup_release = cleanup_release.expect("cleanup pause should be installed");

    let materialization_pause = TimelineMaterializationPause {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        upstream_account_id: None,
        include_live: false,
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
        token: Arc::new(StdMutex::new(None)),
    };
    *INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(materialization_pause.clone());
    let mut fetch_task = tokio::spawn(fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            natural_day_start: Some(format_utc_iso(natural_day_start)),
            natural_day_end: Some(format_utc_iso(natural_day_end)),
            from: format_utc_iso(range_start),
            to: format_utc_iso(range_end),
            include_live: Some(false),
            ..Default::default()
        }),
    ));
    tokio::time::timeout(
        Duration::from_secs(5),
        materialization_pause.entered.notified(),
    )
    .await
    .expect("first-page request should finish materializing while cleanup is paused");
    let token = materialization_pause
        .token
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("materialization pause should publish an active token");

    cleanup_release.notify_one();
    let cleanup_result = tokio::time::timeout(Duration::from_secs(5), &mut cleanup_task)
        .await
        .expect("cleanup should resume and finish")
        .expect("cleanup task should not panic")
        .expect("cleanup should succeed");
    assert!(cleanup_result.skipped.is_none());

    let remaining_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = ?1",
    )
    .bind(&token)
    .fetch_one(&state.pool)
    .await
    .expect("count rows for the active materialized snapshot");
    assert_eq!(
        remaining_rows, 1,
        "cleanup must not delete the new snapshot"
    );

    materialization_pause.release.notify_one();
    let Json(response) = tokio::time::timeout(Duration::from_secs(5), &mut fetch_task)
        .await
        .expect("first-page request should finish after cleanup")
        .expect("first-page request should not panic")
        .expect("first-page request should succeed");
    assert_eq!(response.as_of, token);
    assert_eq!(response.total, 1);
    assert_eq!(response.records.len(), 1);
    assert_eq!(response.records[0].invoke_id, "cleanup-race-request");
    *INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    state.pool.close().await;
}
