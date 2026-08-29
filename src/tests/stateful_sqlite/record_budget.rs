use super::*;

const SUMMARY_PROJECTION_TEST_EXACT_RECORD_LIMIT: usize = 8;

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
