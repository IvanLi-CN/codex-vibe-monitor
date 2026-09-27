use super::*;

#[tokio::test]
async fn invocation_timeline_snapshot_cleanup_runs_off_the_first_page_path_in_bounded_batches() {
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
