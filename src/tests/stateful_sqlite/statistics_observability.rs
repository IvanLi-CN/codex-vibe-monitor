use super::*;
use crate::observability::ObservabilityRuntime;
use crate::observability::contention::with_performance;
use sqlx::sqlite::SqlitePoolOptions;

fn has_series(text: &str, name: &str, labels: &[&str], value: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with(&format!("{name}{{"))
            && labels.iter().all(|label| line.contains(label))
            && line.ends_with(value)
    })
}

#[tokio::test]
async fn long_term_state_query_does_not_count_as_an_overlap_query() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    sqlx::query(
        "CREATE TABLE long_term_stats_state (id INTEGER PRIMARY KEY, status TEXT NOT NULL, statistics_start_date TEXT, integrity_source_start_date TEXT, processed_rows INTEGER NOT NULL, total_rows INTEGER NOT NULL, last_error TEXT)",
    )
    .execute(&pool)
    .await
    .expect("long-term state schema");
    sqlx::query(
        "INSERT INTO long_term_stats_state (id, status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error) VALUES (1, 'ready', '2026-01-01', '2026-01-01', 4, 4, NULL)",
    )
    .execute(&pool)
    .await
    .expect("long-term state fixture");

    let runtime = ObservabilityRuntime::new(true);
    let row = with_performance(runtime.clone(), load_long_term_state(&pool))
        .await
        .expect("long-term state query");
    assert_eq!(row.status, "ready");
    assert_eq!(row.processed_rows, 4);

    assert!(!runtime.render().contains("query=\"long_term_overlap\""));
}

#[tokio::test]
async fn long_term_overlap_query_preserves_results_and_exports_actual_sql_outcomes() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("fixed date");
    let (start, end) = long_term_day_epoch_bounds(date).expect("day bounds");
    for (id, from, to) in [
        (1, start * 1_000 + 1_000, start * 1_000 + 3_000),
        (2, end * 1_000, end * 1_000 + 4_000),
    ] {
        sqlx::query("INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (?, 'model:fixture', 'account:fixture', ?, ?)")
            .bind(id).bind(from).bind(to).execute(&pool).await.expect("interval fixture");
    }
    let dates = HashSet::from([date.to_string()]);
    let baseline = load_long_term_projection_interval_index(&pool, &dates)
        .await
        .expect("unobserved overlap query");
    let runtime = ObservabilityRuntime::new(true);
    let observed = with_performance(
        runtime.clone(),
        load_long_term_projection_interval_index(&pool, &dates),
    )
    .await
    .expect("observed overlap query");
    let daily_key = projection_interval_key(
        "daily",
        date.to_string(),
        "model".to_string(),
        "model:fixture".to_string(),
    );
    assert_eq!(observed.len(), baseline.len());
    assert_eq!(observed[&daily_key].duration_ms, 2_000);
    assert_eq!(
        observed[&daily_key].intervals,
        baseline[&daily_key].intervals
    );
    let text = runtime.render();
    assert!(has_series(
        &text,
        "cvm_stats_queries_total",
        &["outcome=\"success\"", "query=\"long_term_overlap\""],
        " 1"
    ));
    assert!(has_series(
        &text,
        "cvm_stats_query_duration_seconds_count",
        &["outcome=\"success\"", "query=\"long_term_overlap\""],
        " 1"
    ));

    sqlx::query("DROP TABLE long_term_projection_interval_state")
        .execute(&pool)
        .await
        .expect("remove actual overlap source");
    let result = with_performance(
        runtime.clone(),
        load_long_term_projection_interval_index(&pool, &dates),
    )
    .await;
    assert!(result.is_err());
    assert!(has_series(
        &runtime.render(),
        "cvm_stats_queries_total",
        &["outcome=\"error\"", "query=\"long_term_overlap\""],
        " 1"
    ));
}
