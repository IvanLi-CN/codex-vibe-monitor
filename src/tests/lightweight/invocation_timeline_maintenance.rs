use super::*;

#[tokio::test]
async fn invocation_timeline_release_is_idempotent_and_deletes_rows_in_background() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let natural_day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let natural_day_end = natural_day_start + chrono::Duration::days(1);
    let range_start = now - chrono::Duration::minutes(30);
    let range_end = now;

    let make_snapshot = || {
        fetch_timeline(
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
    };
    let Json(released) = make_snapshot()
        .await
        .expect("create timeline snapshot to release");
    let Json(active) = make_snapshot()
        .await
        .expect("create active timeline snapshot");
    for token in [&released.as_of, &active.as_of] {
        sqlx::query(
            "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, ?2, ?3, 1, 0, 0, '{}')",
        )
        .bind(token)
        .bind(format!("fixture-{token}"))
        .bind(format_utc_iso(range_start))
        .execute(&state.pool)
        .await
        .expect("insert snapshot row for lifecycle verification");
    }

    let app = build_stats_routes(Router::new()).with_state(state.clone());
    let release_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/stats/invocation-timeline/{}", released.as_of))
                .body(Body::empty())
                .expect("build release request"),
        )
        .await
        .expect("dispatch release request");
    assert_eq!(release_response.status(), StatusCode::NO_CONTENT);

    let released_rows_before_cleanup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = ?1",
    )
    .bind(&released.as_of)
    .fetch_one(&state.pool)
    .await
    .expect("count released rows before background cleanup");
    assert_eq!(
        released_rows_before_cleanup, 1,
        "the DELETE handler must not delete SQLite rows in the HTTP request path"
    );

    for token in [&released.as_of, "expired-or-unknown-token"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/stats/invocation-timeline/{token}"))
                    .body(Body::empty())
                    .expect("build idempotent release request"),
            )
            .await
            .expect("dispatch idempotent release request");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    let mut cleanup = None;
    for _ in 0..8 {
        let result = cleanup_timeline_snapshot_rows_once(&state.pool)
            .await
            .expect("run pressure-gated background cleanup");
        if result.skipped.is_none() {
            cleanup = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cleanup = cleanup.expect("background cleanup should be admitted");
    assert!(cleanup.scanned_tokens <= 64);

    let released_rows_after_cleanup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = ?1",
    )
    .bind(&released.as_of)
    .fetch_one(&state.pool)
    .await
    .expect("count released rows after background cleanup");
    let active_rows_after_cleanup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = ?1",
    )
    .bind(&active.as_of)
    .fetch_one(&state.pool)
    .await
    .expect("count active rows after background cleanup");
    assert_eq!(released_rows_after_cleanup, 0);
    assert_eq!(active_rows_after_cleanup, 1);
    let active_release = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/stats/invocation-timeline/{}", active.as_of))
                .body(Body::empty())
                .expect("build active release request"),
        )
        .await
        .expect("dispatch active release request");
    assert_eq!(active_release.status(), StatusCode::NO_CONTENT);
    let _ = cleanup_timeline_snapshot_rows_once(&state.pool)
        .await
        .expect("reclaim final active test snapshot");
    state.pool.close().await;
}

#[tokio::test]
async fn invocation_timeline_http_release_reuses_capacity_for_two_clients() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let natural_day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let natural_day_end = natural_day_start + chrono::Duration::days(1);
    let range_start = natural_day_start + chrono::Duration::hours(12);
    let range_end = range_start + chrono::Duration::hours(1);
    for (invoke_id, offset_seconds) in [("capacity-fixture-a", 10), ("capacity-fixture-b", 20)] {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 1000, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(invoke_id)
        .bind(db_occurred_at_lower_bound(
            range_start + chrono::Duration::seconds(offset_seconds),
        ))
        .execute(&state.pool)
        .await
        .expect("insert populated timeline capacity fixture");
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("naturalDayStart", &format_utc_iso(natural_day_start))
        .append_pair("naturalDayEnd", &format_utc_iso(natural_day_end))
        .append_pair("from", &format_utc_iso(range_start))
        .append_pair("to", &format_utc_iso(range_end))
        .append_pair("includeLive", "false")
        .append_pair("limit", "1")
        .finish();
    let app = build_stats_routes(Router::new()).with_state(state.clone());
    let run_client = |client_id: usize, app: Router, query: String| async move {
        for traversal in 0..150 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(format!("/api/stats/invocation-timeline?{query}"))
                        .body(Body::empty())
                        .expect("build snapshot page request"),
                )
                .await
                .expect("dispatch snapshot page request");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "client {client_id} traversal {traversal} should not hit snapshot capacity"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read snapshot page response");
            let first_page: serde_json::Value =
                serde_json::from_slice(&body).expect("decode snapshot page response");
            assert_eq!(first_page["hasMore"], true);
            assert_eq!(first_page["total"], 2);
            assert_eq!(first_page["records"].as_array().map(Vec::len), Some(1));
            let as_of = first_page["asOf"]
                .as_str()
                .expect("snapshot page should return asOf")
                .to_string();
            let cursor = first_page["nextCursor"]
                .as_str()
                .expect("first snapshot page should return a cursor");
            let second_page_query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("naturalDayStart", &format_utc_iso(natural_day_start))
                .append_pair("naturalDayEnd", &format_utc_iso(natural_day_end))
                .append_pair("from", &format_utc_iso(range_start))
                .append_pair("to", &format_utc_iso(range_end))
                .append_pair("includeLive", "false")
                .append_pair("limit", "1")
                .append_pair("asOf", &as_of)
                .append_pair("cursor", cursor)
                .finish();
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(format!(
                            "/api/stats/invocation-timeline?{second_page_query}"
                        ))
                        .body(Body::empty())
                        .expect("build second snapshot page request"),
                )
                .await
                .expect("dispatch second snapshot page request");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "client {client_id} traversal {traversal} should complete its second page"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read second snapshot page response");
            let second_page: serde_json::Value =
                serde_json::from_slice(&body).expect("decode second snapshot page response");
            assert_eq!(second_page["asOf"], as_of);
            assert_eq!(second_page["hasMore"], false);
            assert_eq!(second_page["total"], 2);
            assert_eq!(second_page["records"].as_array().map(Vec::len), Some(1));

            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::DELETE)
                        .uri(format!("/api/stats/invocation-timeline/{as_of}"))
                        .body(Body::empty())
                        .expect("build completed snapshot release request"),
                )
                .await
                .expect("dispatch completed snapshot release request");
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
    };

    let ((), ()) = tokio::join!(
        run_client(1, app.clone(), query.clone()),
        run_client(2, app, query),
    );

    for _ in 0..32 {
        let cleanup = cleanup_timeline_snapshot_rows_once(&state.pool)
            .await
            .expect("drain bounded snapshot release batches");
        assert!(cleanup.scanned_tokens <= 64);
    }
    let remaining_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_timeline_snapshot_rows")
            .fetch_one(&state.pool)
            .await
            .expect("count rows after released populated snapshots are reclaimed");
    assert_eq!(remaining_rows, 0);
    state.pool.close().await;
}

#[tokio::test]
async fn invocation_timeline_cleanup_scans_database_while_release_queue_is_full() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let natural_day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let range_start = natural_day_start + chrono::Duration::hours(12);
    let range_end = range_start + chrono::Duration::hours(1);
    let _ = fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            natural_day_start: Some(format_utc_iso(natural_day_start)),
            natural_day_end: Some(format_utc_iso(
                natural_day_start + chrono::Duration::days(1),
            )),
            from: format_utc_iso(range_start),
            to: format_utc_iso(range_end),
            include_live: Some(false),
            ..Default::default()
        }),
    )
    .await
    .expect("initialize snapshot table before testing cleanup fairness");
    for index in 0..20 {
        sqlx::query(
            "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, ?2, ?3, 1, 0, 0, '{}')",
        )
        .bind(format!("orphan-{index:03}"))
        .bind(format!("invoke-{index:03}"))
        .bind(format_utc_iso(range_start))
        .execute(&state.pool)
        .await
        .expect("insert orphan snapshot row");
    }
    sqlx::query(
        "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES ('overflowed-token', 'overflowed-invocation', ?1, 1, 0, 0, '{}')",
    )
    .bind(format_utc_iso(range_start))
    .execute(&state.pool)
    .await
    .expect("insert snapshot row for a release that will overflow the queue");
    let queue_limit = timeline_snapshot_release_queue_limit_for_test();
    let queued = seed_timeline_snapshot_release_queue_for_test(
        (0..queue_limit).map(|index| format!("queued-{index:04}")),
    );
    assert_eq!(queued, queue_limit);
    assert_eq!(
        seed_timeline_snapshot_release_queue_for_test(["overflowed-token".to_string()]),
        0,
        "a full release queue must leave the overflowed row to periodic scanning"
    );

    let cleanup = cleanup_timeline_snapshot_rows_once(&state.pool)
        .await
        .expect("run bounded cleanup with a saturated explicit-release queue");
    assert_eq!(cleanup.scanned_tokens, 64);
    assert_eq!(cleanup.deleted_rows, 16);
    let followup_cleanup = cleanup_timeline_snapshot_rows_once(&state.pool)
        .await
        .expect("continue scanning overflowed rows while the release queue remains full");
    assert_eq!(followup_cleanup.deleted_rows, 5);
    let remaining_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM invocation_timeline_snapshot_rows")
            .fetch_one(&state.pool)
            .await
            .expect("count stale rows after scanning the overflowed release");
    assert_eq!(remaining_rows, 0);
    assert_eq!(
        timeline_snapshot_release_queue_depth_for_test(),
        queue_limit - 96
    );

    reset_timeline_snapshot_release_queue_for_test();
    reset_timeline_snapshot_cleanup_cursor_for_test();
    state.pool.close().await;
}

#[tokio::test]
async fn invocation_timeline_snapshot_cleanup_runs_off_the_first_page_path_in_bounded_batches() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
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
async fn invocation_timeline_snapshot_cleanup_wraps_after_a_partial_tail_batch() {
    let _cleanup_test_guard = lock_timeline_snapshot_cleanup_tests().await;
    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
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

    for token in ["cursor-z-1", "cursor-z-2"] {
        sqlx::query(
            "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, ?2, ?3, 1, 0, 0, '{}')",
        )
        .bind(token)
        .bind(format!("fixture-{token}"))
        .bind(format_utc_iso(range_start))
        .execute(&state.pool)
        .await
        .expect("insert tail snapshot row");
    }

    let first_cleanup = {
        let mut admitted = None;
        for _ in 0..8 {
            let result = cleanup_timeline_snapshot_rows_once(&state.pool)
                .await
                .expect("run first bounded cleanup batch");
            if result.skipped.is_none() {
                admitted = Some(result);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        admitted.expect("first cleanup batch should be admitted")
    };
    assert_eq!(first_cleanup.scanned_tokens, 2);

    sqlx::query(
        "INSERT INTO invocation_timeline_snapshot_rows (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES ('cursor-a-late', 'late-row', ?1, 1, 0, 0, '{}')",
    )
    .bind(format_utc_iso(range_start))
    .execute(&state.pool)
    .await
    .expect("insert token before the prior cursor");

    let followup_cleanup = {
        let mut admitted = None;
        for _ in 0..8 {
            let result = cleanup_timeline_snapshot_rows_once(&state.pool)
                .await
                .expect("run wrapped cleanup batch");
            if result.skipped.is_none() {
                admitted = Some(result);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        admitted.expect("follow-up cleanup batch should be admitted")
    };
    assert_eq!(followup_cleanup.scanned_tokens, 1);
    assert_eq!(followup_cleanup.deleted_tokens, 1);

    let remaining_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invocation_timeline_snapshot_rows WHERE snapshot_token = 'cursor-a-late'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count wrapped snapshot row");
    assert_eq!(remaining_rows, 0);

    reset_timeline_snapshot_cleanup_cursor_for_test();
    reset_timeline_snapshot_release_queue_for_test();
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
