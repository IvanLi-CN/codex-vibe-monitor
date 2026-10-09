use super::*;

const SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT: usize = 8;

#[tokio::test]
async fn ensure_schema_adds_pending_summary_rollup_partial_index_idempotently() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema test pool");
    ensure_schema(&pool).await.expect("ensure schema");

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM archive_batches \
         WHERE dataset = ?1 AND status = ?2 \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
         ORDER BY month_key, created_at, id LIMIT ?3",
    )
    .bind("codex_invocations")
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(128_i64)
    .fetch_all(&pool)
    .await
    .expect("explain pending Summary rollup query");
    assert!(
        plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_archive_batches_pending_summary_rollup_order")
        }),
        "pending Summary rollup query must use the partial index: {plan:?}"
    );

    ensure_schema(&pool).await.expect("repeat schema migration");
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'index' AND name = 'idx_archive_batches_pending_summary_rollup_order'",
    )
    .fetch_one(&pool)
    .await
    .expect("count pending Summary rollup indexes");
    assert_eq!(
        index_count, 1,
        "re-entry must not duplicate the partial index"
    );
    pool.close().await;
}

#[tokio::test]
async fn stats_serves_last_good_snapshot_for_terminal_gap_but_rejects_source_gap() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let base_occurred_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('stats-pending-proof-base', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(base_occurred_at)
    .execute(&state.pool)
    .await
    .expect("seed published stats snapshot");
    hydrate_stats_snapshot_for_test(&state).await;

    let mut terminal = summary_projection_test_invocation();
    terminal.id = 913_101;
    terminal.invoke_id = "stats-pending-proof-overlay".to_string();
    terminal.occurred_at = db_occurred_at_lower_bound(Utc::now());
    terminal.source = SOURCE_PROXY.to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(23);
    terminal.output_tokens = Some(11);
    terminal.cost = Some(2.5);
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost, payload, raw_response, detail_level) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}', '', 'full')",
    )
    .bind(terminal.id)
    .bind(&terminal.invoke_id)
    .bind(&terminal.occurred_at)
    .bind(&terminal.source)
    .bind(terminal.status.as_deref())
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .execute(&state.pool)
    .await
    .expect("persist stats overlay row");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("materialize stats overlay delta");
    state
        .subscription_hub
        .acknowledge_summary_delta(delta)
        .await;
    state
        .subscription_hub
        .record_summary_terminal_sequence_gap(913_102)
        .await;
    state.pool.close().await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("pending proof should serve the last-good summary snapshot");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.total_tokens, 40);
    assert_eq!(
        summary.data_quality,
        Some(StatsDataQualityResponse::summary_delta_journal_pending())
    );

    let current_error = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(current_error, Err(ApiError::Unavailable(_))),
        "a terminal gap affecting current rank must remain unavailable"
    );

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("legacy stats should share the degraded memory-only path");
    assert_eq!(stats.total_count, 2);
    assert_eq!(stats.total_tokens, 40);
    assert_eq!(
        stats.data_quality,
        Some(StatsDataQualityResponse::summary_delta_journal_pending())
    );

    state
        .subscription_hub
        .record_summary_source_change_gap(913_103)
        .await;
    let summary_error = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(summary_error, Err(ApiError::Unavailable(_))),
        "durable source gaps must remain unavailable for Summary HTTP reads"
    );

    let stats_error = fetch_stats(State(state)).await;
    assert!(
        matches!(stats_error, Err(ApiError::Unavailable(_))),
        "durable source gaps must remain unavailable for legacy stats reads"
    );
}

#[tokio::test]
async fn stats_keeps_terminal_gap_degraded_after_gap_proof_budget_overflow() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('stats-proof-budget-base', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2)))
    .execute(&state.pool)
    .await
    .expect("seed proof-budget stats snapshot");
    hydrate_stats_snapshot_for_test(&state).await;

    for sequence in 1..=(SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS as u64 + 1) {
        state
            .subscription_hub
            .record_summary_terminal_sequence_gap(sequence)
            .await;
    }
    state.pool.close().await;

    let Json(stats) = fetch_stats(State(state))
        .await
        .expect("terminal-only proof overflow should remain degraded");
    assert_eq!(
        stats.data_quality,
        Some(StatsDataQualityResponse::summary_delta_journal_pending())
    );
}

#[tokio::test]
async fn summary_rollup_repair_reads_a_bounded_archive_batch() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
        r#"WITH RECURSIVE rows(value) AS (
               SELECT 1
               UNION ALL
               SELECT value + 1 FROM rows WHERE value < 129
           )
           INSERT INTO archive_batches (
               dataset, month_key, file_path, sha256, row_count, status, summary_source_kind
           )
           SELECT
               'codex_invocations',
               '2026-01',
               'stats-bounded-archive-' || value,
               'hash-' || value,
               1,
               'completed',
               'unknown'
           FROM rows"#,
    )
    .execute(&state.pool)
    .await
    .expect("insert archive repair batch fixture");

    let rows = load_invocation_archives_missing_summary_rollup_markers(&state.pool)
        .await
        .expect("load bounded archive repair batch");
    assert_eq!(rows.len(), 128);
}

#[tokio::test]
async fn summary_rollup_repair_reopens_stale_archive_sha_markers() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let archive_path = "stats-stale-summary-archive.sqlite.gz";
    sqlx::query(
        "INSERT INTO archive_batches (
             dataset, month_key, file_path, sha256, row_count, status, summary_source_kind
         ) VALUES ('codex_invocations', '2026-01', ?1, 'current-hash', 1, 'completed', 'unknown')",
    )
    .bind(archive_path)
    .execute(&state.pool)
    .await
    .expect("insert stale Summary archive manifest");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (
                 target, dataset, file_path, archive_sha256
             ) VALUES (?1, 'codex_invocations', ?2, 'stale-hash')",
        )
        .bind(target)
        .bind(archive_path)
        .execute(&state.pool)
        .await
        .expect("insert stale Summary replay marker");
    }

    let rows = load_invocation_archives_missing_summary_rollup_markers(&state.pool)
        .await
        .expect("load stale Summary archive marker");
    assert_eq!(rows.len(), 1);
    state.pool.close().await;
}

#[tokio::test]
async fn summary_account_live_tail_admission_fails_closed_above_budget() {
    with_summary_projection_test_exact_record_limit(
        SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT,
        async {
            let state = crate::tests::test_state_with_openai_base(
                url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
            )
            .await;
            sqlx::query(
                r#"WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1)
                   INSERT INTO codex_invocations
                   (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
                   SELECT 'account-live-tail-' || value, datetime('now', '-1 minute'), 'proxy', 'success', 1, 0.1, '{"upstreamAccountId":42}', '', 'full'
                   FROM rows"#,
            )
            .bind((SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT + 1) as i64)
            .execute(&state.pool)
            .await
            .expect("insert account live-tail overflow fixture");

            let error = admit_summary_projection_live_tail_account_ids_for_test(&state.pool, Some(0))
                .await
                .expect_err("account live-tail overflow must fail closed");
            assert!(
                error
                    .to_string()
                    .contains("summary projection account live-tail id hydration failed"),
                "account live-tail overflow must report its bounded admission: {error:#}"
            );
            assert!(error.to_string().contains("budget exceeded"));
        },
    )
    .await;
}

#[tokio::test]
async fn summary_projection_hydrates_when_historical_live_rows_exceed_exact_budget() {
    with_summary_projection_test_exact_record_limit(
        SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT,
        async {
            let state = crate::tests::test_state_with_openai_base(
                url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
            )
            .await;
            sqlx::query(
                r#"WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1)
                   INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
                   SELECT 'historical-summary-' || value, datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{}', '', 'full' FROM rows"#,
            )
            .bind((SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT + 1) as i64)
            .execute(&state.pool)
            .await
            .expect("insert historical overflow fixture");
            hydrate_summary_snapshots(state.as_ref())
                .await
                .expect("hourly-backed projection should not retain epoch-zero history");
            assert!(state.subscription_hub.summary_projection().await.is_some());
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
            .expect("a quiet historical account must have an exact memory-only current response");
            assert_eq!(current.total_count, 1);
            assert_eq!(current.total_tokens, 1);
        },
    )
    .await;
}

#[tokio::test]
async fn summary_projection_live_horizon_overflow_publishes_local_unavailability() {
    with_summary_projection_test_exact_record_limit(
        SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT,
        async {
            let state = crate::tests::test_state_with_openai_base(
                url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
            )
            .await;
            sqlx::query(
                r#"WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1)
                   INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
                   SELECT 'horizon-summary-' || value, datetime('now', '-1 minute'), 'proxy', 'success', 1, 0.1, '{}', '', 'full' FROM rows"#,
            )
            .bind((SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT + 1) as i64)
            .execute(&state.pool)
            .await
            .expect("insert in-horizon overflow fixture");

            hydrate_summary_snapshots(state.as_ref())
                .await
                .expect("in-horizon overflow must publish a projection with a local gap");
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
            .expect("current prefix remains exact despite an older unproven rank");
            assert_eq!(current.total_count, 1);
            let error = fetch_summary(
                State(state),
                Query(SummaryQuery {
                    window: Some("1d".to_string()),
                    limit: None,
                    time_zone: Some("UTC".to_string()),
                    upstream_account_id: None,
                }),
            )
            .await
            .expect_err("the overflowing live hour must remain unavailable");
            assert!(matches!(error, ApiError::Unavailable(_)));
        },
    )
    .await;
}

#[tokio::test]
async fn summary_projection_fails_closed_for_mixed_recent_index_overflow() {
    with_summary_projection_test_exact_record_limit(
        SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT,
        async {
            let state = crate::tests::test_state_with_openai_base(
                url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
            )
            .await;
            sqlx::query(
                r#"WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1)
                   INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
                   SELECT 'mixed-overflow-current-' || value, datetime('now', '-1 minute'), 'proxy', 'success', 1, 0.1, '{"upstreamAccountId":42}', '', 'full' FROM rows"#,
            )
            .bind((SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT - 1) as i64)
            .execute(&state.pool)
            .await
            .expect("insert exact-horizon fixture rows");
            sqlx::query(
                r#"INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level)
                   VALUES ('mixed-overflow-older-1', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{"upstreamAccountId":42}', '', 'full'),
                          ('mixed-overflow-older-2', datetime('now', '-3 days'), 'proxy', 'success', 1, 0.1, '{"upstreamAccountId":42}', '', 'full')"#,
            )
            .execute(&state.pool)
            .await
            .expect("insert legal rolling rows outside the exact horizon");
            for dataset in [
                "codex_invocations_summary_rollup_v2_live_cursor",
                "invocation_account_activity_v2_repair_live_cursor",
            ] {
                sqlx::query(
                    r#"INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
                       VALUES (?1, ?2, datetime('now'))
                       ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = excluded.updated_at"#,
                )
                .bind(dataset)
                .bind((SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT - 1) as i64)
                .execute(&state.pool)
                .await
                .expect("record lagging rollup cursor");
            }

            let mut old_runtime_overlay = summary_projection_test_invocation();
            old_runtime_overlay.id = 0;
            old_runtime_overlay.invoke_id = "mixed-overflow-old-runtime-overlay".to_string();
            old_runtime_overlay.occurred_at = (Utc::now() - ChronoDuration::days(8))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            old_runtime_overlay.created_at = old_runtime_overlay.occurred_at.clone();
            old_runtime_overlay.source = SOURCE_PROXY.to_string();
            old_runtime_overlay.status = Some("success".to_string());
            old_runtime_overlay.upstream_account_id = Some(42);
            old_runtime_overlay.total_tokens = Some(1);
            old_runtime_overlay.cost = Some(0.1);
            state
                .proxy_runtime_invocations
                .upsert_terminal(old_runtime_overlay);

            hydrate_summary_snapshots(state.as_ref())
                .await
                .expect("hydrate bounded mixed-overflow projection with an older runtime overlay");
            state.pool.close().await;

            for upstream_account_id in [None, Some(42)] {
                let error = fetch_summary(
                    State(state.clone()),
                    Query(SummaryQuery {
                        window: Some("7d".to_string()),
                        limit: None,
                        time_zone: Some("UTC".to_string()),
                        upstream_account_id,
                    }),
                )
                .await
                .expect_err(
                    "an unretained unrolled live row must not produce a truncated rolling total",
                );
                assert!(
                    matches!(error, ApiError::Unavailable(_)),
                    "rolling overflow must fail closed for {upstream_account_id:?}: {error:?}"
                );
            }

            for upstream_account_id in [None, Some(42)] {
                let Json(response) = fetch_summary(
                    State(state.clone()),
                    Query(SummaryQuery {
                        window: Some("1d".to_string()),
                        limit: None,
                        time_zone: Some("UTC".to_string()),
                        upstream_account_id,
                    }),
                )
                .await
                .expect("a range newer than the strictest overflow boundary remains exact in memory");
                assert_eq!(
                    response.total_count,
                    (SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT - 1) as i64,
                    "safe range for {upstream_account_id:?}"
                );
            }
        },
    )
    .await;
}
