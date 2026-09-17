async fn seed_summary_projection_current_archive_cutoff_fixture() -> Arc<AppState> {
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
    state
}

#[tokio::test]
async fn summary_projection_current_archive_cutoff_is_request_specific() {
    let state = seed_summary_projection_current_archive_cutoff_fixture().await;
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
