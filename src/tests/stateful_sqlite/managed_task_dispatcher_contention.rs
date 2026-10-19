use crate::maintenance_store::MaintenanceStore;

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

#[tokio::test]
async fn dispatcher_probe_ignores_due_task_with_active_run() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");

    sqlx::query(
        "UPDATE managed_tasks
         SET next_trigger_at='2000-01-01T00:00:00.000Z'
         WHERE task_key='invocation_timeline_snapshot'",
    )
    .execute(&store.pool)
    .await
    .expect("seed due task");
    sqlx::query(
        "INSERT INTO managed_task_runs(task_key,trigger_kind,started_at,status)
         VALUES('invocation_timeline_snapshot','schedule','2000-01-01T00:00:00.000Z','running')",
    )
    .execute(&store.pool)
    .await
    .expect("seed active run");

    assert!(
        !store
            .probe_due_or_requested_runs()
            .await
            .expect("probe active dispatcher")
    );
    assert_eq!(
        store
            .enqueue_due_runs()
            .await
            .expect("enqueue active dispatcher"),
        0
    );
}

#[tokio::test]
async fn dispatcher_requested_probe_uses_status_index() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");

    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
        "EXPLAIN QUERY PLAN
         SELECT 1 FROM managed_task_runs WHERE status='requested' LIMIT 1",
    )
    .fetch_all(&store.pool)
    .await
    .expect("explain requested probe");
    let details = plan
        .into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect::<Vec<_>>();
    assert!(
        details
            .iter()
            .any(|detail| { detail.contains("idx_managed_task_runs_requested_status") }),
        "requested probe must use the partial status index: {details:?}"
    );
}

#[tokio::test]
async fn events_history_query_uses_bounded_history_indexes() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");

    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
        "EXPLAIN QUERY PLAN
         SELECT id,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,
                duration_ms,actual_duration_ms,status,error_detail,execution_uid
         FROM managed_task_runs INDEXED BY idx_managed_task_runs_task_actual_started
         WHERE task_key=? AND status NOT IN ('requested','queued')
           AND NOT EXISTS (
               SELECT 1 FROM managed_task_work_runs wr
               WHERE wr.managed_run_id=managed_task_runs.id
           )
         ORDER BY COALESCE(actual_started_at,started_at) DESC,id DESC LIMIT 200",
    )
    .bind("invocation_timeline_snapshot")
    .fetch_all(&store.pool)
    .await
    .expect("explain events history query");
    let details = plan
        .into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect::<Vec<_>>();
    assert!(
        details
            .iter()
            .any(|detail| { detail.contains("idx_managed_task_work_runs_managed_run_id") }),
        "work history anti-join must use managed run index: {details:?}"
    );
    assert!(
        details
            .iter()
            .any(|detail| { detail.contains("idx_managed_task_runs_task_actual_started") }),
        "events history ordering must use the expression index: {details:?}"
    );
}

#[tokio::test]
async fn events_history_query_with_time_window_uses_bounded_history_indexes() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");

    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
        "EXPLAIN QUERY PLAN
         SELECT id,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,
                duration_ms,actual_duration_ms,status,error_detail,execution_uid
         FROM managed_task_runs INDEXED BY idx_managed_task_runs_task_actual_started
         WHERE task_key=? AND status NOT IN ('requested','queued')
           AND started_at>=? AND started_at<=?
           AND NOT EXISTS (
               SELECT 1 FROM managed_task_work_runs wr
               WHERE wr.managed_run_id=managed_task_runs.id
           )
         ORDER BY COALESCE(actual_started_at,started_at) DESC,id DESC LIMIT 200",
    )
    .bind("invocation_timeline_snapshot")
    .bind("2026-01-01T00:00:00.000Z")
    .bind("2026-12-31T23:59:59.999Z")
    .fetch_all(&store.pool)
    .await
    .expect("explain windowed events history query");
    let details = plan
        .into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect::<Vec<_>>();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("idx_managed_task_work_runs_managed_run_id")),
        "windowed work history anti-join must use managed run index: {details:?}"
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("idx_managed_task_runs_task_actual_started")),
        "windowed events history ordering must use the expression index: {details:?}"
    );
}
