use super::*;
use crate::observability::ObservabilityRuntime;
use crate::observability::contention::{StatsQuery, observe_query, with_performance};
use sqlx::sqlite::SqlitePoolOptions;

fn has_series(text: &str, name: &str, labels: &[&str], value: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with(&format!("{name}{{"))
            && labels.iter().all(|label| line.contains(label))
            && line.ends_with(value)
    })
}

#[tokio::test]
async fn long_term_state_query_preserves_result_and_exports_query_observation() {
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

    let result = with_performance(
        runtime.clone(),
        observe_query(StatsQuery::LongTermOverlap, async {
            Err::<(), _>("synthetic failure")
        }),
    )
    .await;
    assert_eq!(result, Err("synthetic failure"));
}
