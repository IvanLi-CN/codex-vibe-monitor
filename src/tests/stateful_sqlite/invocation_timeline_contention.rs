use super::*;
use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, TimeZone, Utc};
use sqlx::Row;
use std::time::Duration;
use url::Url;

use crate::api::slices::invocation_timeline::{
    InvocationTimelineQuery, InvocationTimelineRecord, fetch_timeline,
    install_timeline_source_page_pause_for_test,
};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .expect("valid timeline test timestamp")
}

#[tokio::test]
async fn timeline_bounded_source_capture_releases_connection_before_next_page() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_start = at(30_000);
    let range_end = at(31_000);
    for index in 0..501_i64 {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 100, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(format!("short-read-{index:03}"))
        .bind(crate::db_occurred_at_lower_bound(
            range_start + chrono::Duration::seconds(index),
        ))
        .execute(&state.pool)
        .await
        .expect("insert short-read timeline fixture");
    }

    let held_connections = vec![
        state
            .pool
            .acquire()
            .await
            .expect("hold pool connection one"),
        state
            .pool
            .acquire()
            .await
            .expect("hold pool connection two"),
        state
            .pool
            .acquire()
            .await
            .expect("hold pool connection three"),
    ];
    let (entered, release) = install_timeline_source_page_pause_for_test();
    let task = tokio::spawn(fetch_timeline(
        State(state.clone()),
        Query(InvocationTimelineQuery {
            from: crate::format_utc_iso(range_start),
            to: crate::format_utc_iso(range_end),
            upstream_account_id: Some(42),
            include_live: Some(false),
            limit: Some(1),
            cursor: None,
            as_of: None,
            ..Default::default()
        }),
    ));
    entered.notified().await;

    let page_connection_available =
        match tokio::time::timeout(Duration::from_millis(100), state.pool.acquire()).await {
            Ok(Ok(connection)) => {
                drop(connection);
                true
            }
            _ => false,
        };
    if page_connection_available {
        // The bounded source capture has completed before this pause, so this later update must
        // not alter the already-captured source row used by the remaining in-memory pages.
        sqlx::query(
            "UPDATE codex_invocations SET status='failed', t_total_ms=900, payload='{\"upstreamAccountId\":77}' WHERE invoke_id='short-read-500'",
        )
        .execute(&state.pool)
        .await
        .expect("mutate a later source row after capture");
    }
    release.notify_one();
    let Json(first) = task
        .await
        .expect("timeline fetch task should finish")
        .expect("timeline fetch should succeed");
    drop(held_connections);

    let captured_row = sqlx::query(
        "SELECT payload FROM invocation_timeline_snapshot_rows WHERE snapshot_token=?1 AND invoke_id='short-read-500'",
    )
    .bind(&first.as_of)
    .fetch_one(&state.pool)
    .await
    .expect("load captured later source row");
    let captured_payload: String = captured_row
        .try_get("payload")
        .expect("decode captured source payload");
    let captured: InvocationTimelineRecord =
        serde_json::from_str(&captured_payload).expect("decode captured timeline row");

    assert!(
        page_connection_available,
        "a completed source page must release its pool connection before the next page"
    );
    assert_eq!(captured.status.as_deref(), Some("success"));
    assert_eq!(captured.t_total_ms, Some(100.0));
    assert_eq!(captured.upstream_account_id, Some(42));
    state.pool.close().await;
}
