use super::*;
use serde_json::json;
use std::time::{Duration, Instant};

const FIXTURE_CONTRACT_VERSION: &str = "summary-representative-scale-v2";
const FIXTURE_ROWS: i64 = 321;
const FIXTURE_PAYLOAD_BYTES: usize = 700_000;
const MIN_RAW_SOURCE_BYTES: usize = 214 * 1024 * 1024;
const BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(30);
const ALL_TIME_DEADLINE: Duration = Duration::from_secs(1_800);

#[tokio::test]
async fn summary_representative_scale_acceptance() {
    let state =
        test_state_with_openai_base(url::Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let payload = format!(
        r#"{{"fixtureContract":"{FIXTURE_CONTRACT_VERSION}","padding":"{}"}}"#,
        "x".repeat(FIXTURE_PAYLOAD_BYTES)
    );
    let occurred_at = crate::stats::db_occurred_at_lower_bound(chrono::Utc::now());
    sqlx::query(
        r#"WITH RECURSIVE rows(value) AS (
               SELECT 1
               UNION ALL
               SELECT value + 1 FROM rows WHERE value < ?3
           )
           INSERT INTO codex_invocations
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level, model)
           SELECT 'representative-scale-' || value, ?1, 'proxy', 'success', 7, 0.25, ?2, '', 'full', NULL
           FROM rows"#,
    )
    .bind(&occurred_at)
    .bind(&payload)
    .bind(FIXTURE_ROWS)
    .execute(&state.pool)
    .await
    .expect("insert deterministic representative-scale fixture");

    assert!(
        FIXTURE_ROWS as usize * payload.len() >= MIN_RAW_SOURCE_BYTES,
        "fixture must contain at least 214 MiB of raw source text"
    );

    let bootstrap_started = Instant::now();
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("current and rolling bootstrap must publish");
    let bootstrap_elapsed = bootstrap_started.elapsed();
    assert!(
        bootstrap_elapsed <= BOOTSTRAP_DEADLINE,
        "bootstrap exceeded {}s: {:?}",
        BOOTSTRAP_DEADLINE.as_secs(),
        bootstrap_elapsed
    );

    let expected = json!({
        "totalCount": FIXTURE_ROWS,
        "successCount": FIXTURE_ROWS,
        "failureCount": 0,
        "totalCost": FIXTURE_ROWS as f64 * 0.25,
        "totalTokens": FIXTURE_ROWS * 7,
        "inProgressConversationCount": 0,
        "inProgressRetryConversationCount": 0,
        "inProgressPhaseCounts": {
            "queued": 0,
            "requesting": 0,
            "responding": 0,
        },
        "nonSuccessCost": 0.0,
        "maintenance": {
            "historicalRollupBackfill": {
                "alertLevel": "none",
                "legacyArchivePending": 0,
                "pendingBuckets": 0,
            },
            "rawCompressionBacklog": {
                "alertLevel": "ok",
                "oldestUncompressedAgeSecs": 0,
                "uncompressedBytes": 0,
                "uncompressedCount": 0,
            },
            "startupBackfill": {
                "nextRunAfter": null,
                "upstreamActivityArchivePendingAccounts": 0,
                "zeroUpdateStreak": 0,
            },
        },
    });
    for window in ["current", "1d", "7d", "30d", "today"] {
        let mut expected_for_window = expected.clone();
        if window != "current" {
            expected_for_window["nonSuccessTokens"] = json!(0);
            expected_for_window["usageBreakdown"] = json!({
                "cacheReadTokens": 0,
                "cacheWriteTokens": 0,
                "outputTokens": 0,
                "costs": {
                    "cacheRead": 0.0,
                    "cacheWrite": 0.0,
                    "input": 0.0,
                    "output": 0.0,
                    "reasoning": 0.0,
                    "unknown": FIXTURE_ROWS as f64 * 0.25,
                },
                "models": [{
                    "model": "unknown",
                    "outputTokens": 0,
                    "cacheWriteTokens": 0,
                    "cacheReadTokens": 0,
                    "costs": {
                        "cacheRead": 0.0,
                        "cacheWrite": 0.0,
                        "input": 0.0,
                        "output": 0.0,
                        "reasoning": 0.0,
                        "unknown": FIXTURE_ROWS as f64 * 0.25,
                    },
                }],
            });
        }
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(200),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("rolling/calendar projection must be exact");
        assert_eq!(
            serde_json::to_value(response).expect("serialize rolling response"),
            expected_for_window,
            "independent oracle mismatch for {window}"
        );
    }

    let all_time_started = Instant::now();
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("all-time checkpoint must complete");
    let all_time_elapsed = all_time_started.elapsed();
    assert!(
        all_time_elapsed <= ALL_TIME_DEADLINE,
        "all-time reconciliation exceeded {}s: {:?}",
        ALL_TIME_DEADLINE.as_secs(),
        all_time_elapsed
    );
    let Json(response) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: Some(200),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("all-time projection must be exact");
    assert_eq!(
        serde_json::to_value(response).expect("serialize all-time response"),
        expected,
        "independent oracle mismatch for all-time"
    );

    // Keep the fixture contract in the test binary so the selected acceptance remains reproducible.
    assert_eq!(FIXTURE_CONTRACT_VERSION, "summary-representative-scale-v2");
}
