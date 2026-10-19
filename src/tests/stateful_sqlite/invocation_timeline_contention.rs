use super::*;
use axum::extract::{Query, State};
use chrono::{DateTime, TimeZone, Utc};
use std::time::Duration;
use url::Url;

use crate::api::slices::invocation_timeline::{
    InvocationTimelineQuery, fetch_timeline, install_timeline_source_page_pause_for_test,
};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .expect("valid timeline test timestamp")
}

#[tokio::test]
async fn timeline_source_page_releases_connection_before_next_page() {
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
    release.notify_one();
    task.await
        .expect("timeline fetch task should finish")
        .expect("timeline fetch should succeed");
    drop(held_connections);

    assert!(
        page_connection_available,
        "a completed source page must release its pool connection before the next page"
    );
    state.pool.close().await;
}
