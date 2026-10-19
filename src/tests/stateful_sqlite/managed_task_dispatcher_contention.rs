use super::*;

#[tokio::test]
async fn dispatcher_probe_reports_only_due_or_requested_work() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");

    assert!(
        !store
            .probe_due_or_requested_runs()
            .await
            .expect("probe idle dispatcher")
    );

    sqlx::query(
        "UPDATE managed_tasks
         SET next_trigger_at='2000-01-01T00:00:00.000Z'
         WHERE task_key='invocation_timeline_snapshot'",
    )
    .execute(&store.pool)
    .await
    .expect("seed due task");
    assert!(
        store
            .probe_due_or_requested_runs()
            .await
            .expect("probe due dispatcher")
    );

    sqlx::query(
        "UPDATE managed_tasks
         SET next_trigger_at=NULL
         WHERE task_key='invocation_timeline_snapshot'",
    )
    .execute(&store.pool)
    .await
    .expect("clear due task");
    store
        .request_run("raw_compression")
        .await
        .expect("queue manual task");
    assert!(
        store
            .probe_due_or_requested_runs()
            .await
            .expect("probe requested dispatcher")
    );
}
