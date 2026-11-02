use super::super::*;

static SYSTEM_TASK_RUN_RETENTION_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

#[tokio::test]
async fn system_task_runs_filter_and_routes_serve_json() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let startup_handle = begin_system_task_run(
        &state.pool,
        SystemTaskKind::StartupBackfill,
        "startup",
        Some("startup cycle".to_string()),
    )
    .await
    .expect("insert startup task");
    tokio::time::sleep(Duration::from_millis(5)).await;
    finish_system_task_run(
        &state.pool,
        &startup_handle,
        SystemTaskStatus::Success,
        Some("startup ok".to_string()),
        None,
    )
    .await;
    sqlx::query("UPDATE system_task_runs SET started_at = ?1, finished_at = ?2 WHERE id = ?3")
        .bind("2026-06-22T08:45:00.000Z")
        .bind("2026-06-22T08:45:01.000Z")
        .bind(startup_handle.id)
        .execute(&state.pool)
        .await
        .expect("pin startup task timestamps");

    let retention_handle = begin_system_task_run(
        &state.pool,
        SystemTaskKind::RetentionArchive,
        "manual",
        Some("retention cycle".to_string()),
    )
    .await
    .expect("insert retention task");
    finish_system_task_run(
        &state.pool,
        &retention_handle,
        SystemTaskStatus::Failed,
        Some("retention failed".to_string()),
        Some("Authorization: Basic old-secret".to_string()),
    )
    .await;
    sqlx::query("UPDATE system_task_runs SET started_at = ?1, finished_at = ?2 WHERE id = ?3")
        .bind("2026-06-22T09:15:00.000Z")
        .bind("2026-06-22T09:15:02.000Z")
        .bind(retention_handle.id)
        .execute(&state.pool)
        .await
        .expect("pin retention task timestamps");
    hydrate_system_status_snapshot(state.as_ref())
        .await
        .expect("hydrate system status snapshot");

    let tied_handle = begin_system_task_run(
        &state.pool,
        SystemTaskKind::StartupBackfill,
        "manual",
        Some("same timestamp cursor tie".to_string()),
    )
    .await
    .expect("insert tied cursor task");
    finish_system_task_run(
        &state.pool,
        &tied_handle,
        SystemTaskStatus::Success,
        Some("tied cursor task completed".to_string()),
        None,
    )
    .await;
    sqlx::query("UPDATE system_task_runs SET started_at = ?1, finished_at = ?2 WHERE id = ?3")
        .bind("2026-06-22T09:15:00.000Z")
        .bind("2026-06-22T09:15:03.000Z")
        .bind(tied_handle.id)
        .execute(&state.pool)
        .await
        .expect("pin tied cursor task timestamps");

    let filtered = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: Some(SystemTaskKind::RetentionArchive.as_str().to_string()),
            status: Some(SystemTaskStatus::Failed.as_str().to_string()),
            started_at_from: None,
            started_at_to: None,
            limit: Some(10),
            page: None,
            page_size: None,
            cursor: None,
        }),
    )
    .await
    .expect("list filtered system tasks")
    .0;

    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.page, 1);
    assert_eq!(filtered.page_size, 10);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].task_kind, "retention_archive");
    assert_eq!(filtered.items[0].status, "failed");
    assert_eq!(
        filtered.items[0].summary.as_deref(),
        Some("retention failed")
    );
    assert!(
        !filtered.items[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("old-secret")
    );
    assert!(filtered.items[0].duration_ms.unwrap_or_default() >= 0);

    let all = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: None,
            started_at_to: None,
            limit: Some(10),
            page: None,
            page_size: None,
            cursor: None,
        }),
    )
    .await
    .expect("list all system tasks")
    .0;

    assert_eq!(all.total, 3);
    assert_eq!(all.page, 1);
    assert_eq!(all.page_size, 10);
    assert_eq!(all.items.len(), 3);
    assert_eq!(all.items[0].id, tied_handle.id);
    assert_eq!(all.items[1].id, retention_handle.id);
    assert_eq!(all.items[2].id, startup_handle.id);

    let paged = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: None,
            started_at_to: None,
            limit: None,
            page: Some(2),
            page_size: Some(1),
            cursor: None,
        }),
    )
    .await
    .expect("list paged system tasks")
    .0;

    assert_eq!(paged.total, 3);
    assert_eq!(paged.page, 2);
    assert_eq!(paged.page_size, 1);
    assert_eq!(paged.items.len(), 1);
    assert_eq!(paged.items[0].id, retention_handle.id);

    let ranged = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: Some("2026-06-22T09:00:00Z".to_string()),
            started_at_to: Some("2026-06-22T09:30:00Z".to_string()),
            limit: Some(10),
            page: None,
            page_size: None,
            cursor: None,
        }),
    )
    .await
    .expect("list ranged system tasks")
    .0;

    assert_eq!(ranged.total, 2);
    assert_eq!(ranged.items.len(), 2);
    assert_eq!(ranged.items[0].id, tied_handle.id);
    assert_eq!(ranged.items[1].id, retention_handle.id);

    let first_cursor_page = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: None,
            started_at_to: None,
            limit: None,
            page: None,
            page_size: Some(1),
            cursor: None,
        }),
    )
    .await
    .expect("list first cursor page")
    .0;
    let cursor = first_cursor_page
        .next_cursor
        .clone()
        .expect("first cursor page should expose a continuation");
    let second_cursor_page = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: None,
            started_at_to: None,
            limit: None,
            page: None,
            page_size: Some(1),
            cursor: Some(cursor),
        }),
    )
    .await
    .expect("list second cursor page")
    .0;
    let third_cursor_page = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            task_kind: None,
            status: None,
            started_at_from: None,
            started_at_to: None,
            limit: None,
            page: None,
            page_size: Some(1),
            cursor: second_cursor_page.next_cursor.clone(),
        }),
    )
    .await
    .expect("list third cursor page")
    .0;
    assert_eq!(first_cursor_page.total, paged.total);
    assert_eq!(first_cursor_page.items[0].id, tied_handle.id);
    assert_eq!(second_cursor_page.items[0].id, retention_handle.id);
    assert_eq!(third_cursor_page.items[0].id, startup_handle.id);
    assert!(third_cursor_page.next_cursor.is_none());

    let app = build_app_router(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/system/tasks?taskKind=retention_archive&status=failed&startedAtFrom=2026-06-22T09:00:00Z&startedAtTo=2026-06-22T09:30:00Z&page=1&pageSize=1")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("serve system tasks route");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode tasks payload");
    assert_eq!(payload["total"].as_u64(), Some(1));
    assert_eq!(payload["page"].as_u64(), Some(1));
    assert_eq!(payload["pageSize"].as_u64(), Some(1));
    assert_eq!(
        payload["items"][0]["taskKind"].as_str(),
        Some("retention_archive")
    );

    let cursor_with_page_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/system/tasks?cursor=eyJzdGFydGVkQXQiOiIyMDI2LTA2LTIyVDA5OjE1OjAwWiIsImlkIjoyfQ&page=2")
                .body(Body::empty())
                .expect("build invalid cursor and page request"),
        )
        .await
        .expect("serve invalid cursor and page request");
    assert_eq!(cursor_with_page_response.status(), StatusCode::BAD_REQUEST);

    let invalid_range_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/system/tasks?startedAtFrom=not-a-date")
                .body(Body::empty())
                .expect("build invalid request"),
        )
        .await
        .expect("serve invalid system tasks route");
    assert_eq!(invalid_range_response.status(), StatusCode::BAD_REQUEST);

    let app = build_app_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/status")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("serve system status route");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read status body");
    let payload: Value = serde_json::from_slice(&body).expect("decode status payload");
    assert!(payload.get("liveInvocationsCount").is_some());
    assert!(payload.get("successCount").is_some());
    assert!(payload.get("completedArchiveBatchesCount").is_some());
    assert!(payload.get("databaseBytes").is_some());
    assert!(payload.get("refreshedAt").is_some());
}

#[tokio::test]
async fn system_task_runs_cursor_preserves_invalid_legacy_timestamp() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for (task_kind, started_at) in [
        ("invalid_legacy_timestamp", "2026-02-30 08:45:00"),
        ("older_canonical_timestamp", "2025-01-01T00:00:00.000Z"),
    ] {
        sqlx::query(
            "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES (?1, 'fixture', 'success', ?2)",
        )
        .bind(task_kind)
        .bind(started_at)
        .execute(&state.pool)
        .await
        .expect("seed cursor timestamp fixture");
    }

    let first_page = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            page_size: Some(1),
            ..SystemTaskRunsQuery::default()
        }),
    )
    .await
    .expect("list invalid timestamp cursor page")
    .0;
    assert_eq!(first_page.items[0].task_kind, "invalid_legacy_timestamp");
    let cursor = first_page
        .next_cursor
        .expect("invalid legacy timestamp still yields a cursor");

    let second_page = list_system_task_runs(
        State(state.clone()),
        Query(SystemTaskRunsQuery {
            page_size: Some(1),
            cursor: Some(cursor),
            ..SystemTaskRunsQuery::default()
        }),
    )
    .await
    .expect("continue after invalid timestamp cursor")
    .0;
    assert_eq!(second_page.items[0].task_kind, "older_canonical_timestamp");
    assert!(second_page.next_cursor.is_none());

    let ranged_page = list_system_task_runs(
        State(state),
        Query(SystemTaskRunsQuery {
            started_at_from: Some("2026-02-01T00:00:00Z".to_string()),
            started_at_to: Some("2026-03-01T00:00:00Z".to_string()),
            ..SystemTaskRunsQuery::default()
        }),
    )
    .await
    .expect("exclude invalid legacy timestamp from time range")
    .0;
    assert_eq!(ranged_page.total, 0);
    assert!(ranged_page.items.is_empty());
}

#[tokio::test]
async fn system_task_runs_time_ranges_exclude_canonical_shaped_invalid_dates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('invalid_canonical_timestamp', 'fixture', 'success', '2026-02-30T08:45:00.000Z')",
    )
    .execute(&state.pool)
    .await
    .expect("seed canonical-shaped invalid timestamp");

    let ranged_page = list_system_task_runs(
        State(state),
        Query(SystemTaskRunsQuery {
            started_at_from: Some("2026-02-01T00:00:00Z".to_string()),
            started_at_to: Some("2026-03-01T00:00:00Z".to_string()),
            ..SystemTaskRunsQuery::default()
        }),
    )
    .await
    .expect("exclude canonical-shaped invalid timestamp from time range")
    .0;
    assert_eq!(ranged_page.total, 0);
    assert!(ranged_page.items.is_empty());
}

#[tokio::test]
async fn system_task_runs_schema_indexes_support_time_queries() {
    let pool = test_current_schema_pool().await;
    ensure_schema(&pool)
        .await
        .expect("apply current system task run schema");
    sqlx::query(
        r#"
        WITH RECURSIVE
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 999
            ),
            blocks(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM blocks WHERE value < 399
            )
        INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at)
        SELECT
            CASE WHEN (hundreds.value + blocks.value) % 2 = 0
                THEN 'retention_archive' ELSE 'startup_backfill' END,
            'fixture',
            CASE WHEN hundreds.value % 3 = 0 THEN 'success' ELSE 'failed' END,
            printf(
                '2026-06-%02dT%02d:%02d:%02d.000Z',
                1 + ((hundreds.value * 400 + blocks.value) / 86400),
                ((hundreds.value * 400 + blocks.value) / 3600) % 24,
                ((hundreds.value * 400 + blocks.value) / 60) % 60,
                (hundreds.value * 400 + blocks.value) % 60
            )
        FROM hundreds CROSS JOIN blocks
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed 400k system task runs");

    let default_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id FROM system_task_runs ORDER BY started_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&pool)
    .await
    .expect("explain default system task query")
    .into_iter()
    .map(|row| row.get("detail"))
    .collect();
    assert!(
        default_plan
            .iter()
            .any(|detail| detail.contains("idx_system_task_runs_started_at_id")),
        "default time query must use its ordering index: {default_plan:?}"
    );

    let filtered_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success' AND started_at >= '2026-06-02T00:00:00Z' ORDER BY started_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&pool)
    .await
    .expect("explain combined system task query")
    .into_iter()
    .map(|row| row.get("detail"))
    .collect();
    assert!(
        filtered_plan
            .iter()
            .any(|detail| detail.contains("idx_system_task_runs_task_status_time")),
        "combined filter and sort query must use its index: {filtered_plan:?}"
    );
}

#[tokio::test]
async fn ensure_schema_normalizes_legacy_system_task_run_timestamps() {
    let pool = test_current_schema_pool().await;
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at, finished_at) VALUES ('startup_backfill', 'fixture', 'success', '2026-06-22 08:45:00.125', '2026-06-22T17:15:00+08:00')",
    )
    .execute(&pool)
    .await
    .expect("seed legacy system task timestamp");

    ensure_schema(&pool)
        .await
        .expect("normalize legacy system task timestamps");
    let (started_at, finished_at): (String, Option<String>) = sqlx::query_as(
        "SELECT started_at, finished_at FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&pool)
    .await
    .expect("load normalized task timestamps");
    assert_eq!(started_at, "2026-06-22T00:45:00.125Z");
    assert_eq!(finished_at.as_deref(), Some("2026-06-22T09:15:00.000Z"));

    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at, finished_at) VALUES ('startup_backfill_with_offset', 'fixture', 'success', '2026-06-22 08:45:00.125+08:00', '2026-06-22 17:15:00+08:00')",
    )
    .execute(&pool)
    .await
    .expect("seed space timestamps with explicit offsets");
    ensure_schema(&pool)
        .await
        .expect("normalize offset timestamps");
    let (offset_started_at, offset_finished_at): (String, Option<String>) = sqlx::query_as(
        "SELECT started_at, finished_at FROM system_task_runs WHERE task_kind = 'startup_backfill_with_offset'",
    )
    .fetch_one(&pool)
    .await
    .expect("load normalized offset timestamps");
    assert_eq!(offset_started_at, "2026-06-22T00:45:00.125Z");
    assert_eq!(
        offset_finished_at.as_deref(),
        Some("2026-06-22T09:15:00.000Z")
    );

    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('legacy_rfc3339_without_millis', 'fixture', 'success', '2026-06-22T08:45:00Z')",
    )
    .execute(&pool)
    .await
    .expect("seed legacy RFC3339 timestamp without milliseconds");
    ensure_schema(&pool)
        .await
        .expect("normalize legacy RFC3339 timestamp without milliseconds");
    let legacy_rfc3339_started_at: String = sqlx::query_scalar(
        "SELECT started_at FROM system_task_runs WHERE task_kind = 'legacy_rfc3339_without_millis'",
    )
    .fetch_one(&pool)
    .await
    .expect("load normalized legacy RFC3339 timestamp");
    assert_eq!(legacy_rfc3339_started_at, "2026-06-22T08:45:00.000Z");

    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at, finished_at) VALUES ('invalid_legacy_task_timestamp', 'fixture', 'success', '2026-02-30 08:45:00', '2026-02-30 17:15:00+08:00')",
    )
    .execute(&pool)
    .await
    .expect("seed invalid legacy task timestamps");
    ensure_schema(&pool)
        .await
        .expect("preserve invalid task timestamps");
    let (invalid_started_at, invalid_finished_at): (String, Option<String>) = sqlx::query_as(
        "SELECT started_at, finished_at FROM system_task_runs WHERE task_kind = 'invalid_legacy_task_timestamp'",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved invalid timestamps");
    assert_eq!(invalid_started_at, "2026-02-30 08:45:00");
    assert_eq!(
        invalid_finished_at.as_deref(),
        Some("2026-02-30 17:15:00+08:00")
    );

    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at, finished_at) VALUES ('invalid_legacy_task_timestamp_suffix', 'fixture', 'success', '2026-06-22 08:45:00foo', '2026-06-22 17:15:00+08:00foo')",
    )
    .execute(&pool)
    .await
    .expect("seed invalid legacy task timestamp suffixes");
    ensure_schema(&pool)
        .await
        .expect("preserve invalid task timestamp suffixes");
    let (invalid_suffix_started_at, invalid_suffix_finished_at): (String, Option<String>) =
        sqlx::query_as(
            "SELECT started_at, finished_at FROM system_task_runs WHERE task_kind = 'invalid_legacy_task_timestamp_suffix'",
        )
        .fetch_one(&pool)
        .await
        .expect("load preserved invalid timestamp suffixes");
    assert_eq!(invalid_suffix_started_at, "2026-06-22 08:45:00foo");
    assert_eq!(
        invalid_suffix_finished_at.as_deref(),
        Some("2026-06-22 17:15:00+08:00foo")
    );
}

#[tokio::test]
async fn system_task_run_retention_preserves_recent_rows_and_bounds_deletes() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_TEST_LOCK.lock().await;
    reset_system_task_run_retention_schedule();
    let pool = test_current_schema_pool().await;
    ensure_schema(&pool)
        .await
        .expect("apply current system task run schema");
    sqlx::query(
        r#"
        WITH RECURSIVE
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 999
            ),
            blocks(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM blocks WHERE value < 5
            )
        INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at)
        SELECT 'retention_archive', 'fixture', 'success', '2020-01-01T00:00:00.000Z'
        FROM hundreds CROSS JOIN blocks
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed expired terminal task runs");
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('retention_archive', 'fixture', 'running', '2019-01-01T00:00:00.000Z')",
    )
    .execute(&pool)
    .await
    .expect("seed running task run");
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('retention_archive', 'fixture', 'success', '2026-02-30T08:45:00.000Z')",
    )
    .execute(&pool)
    .await
    .expect("seed canonical-shaped invalid retention timestamp");

    let pruned = prune_system_task_runs(&pool, false)
        .await
        .expect("prune expired task runs");
    assert_eq!(pruned, 5_000, "one retention pass has a hard delete cap");
    let terminal_remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining terminal task runs");
    assert_eq!(terminal_remaining, 1_001);
    let oldest_retained_id: i64 = sqlx::query_scalar(
        "SELECT MIN(id) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("find oldest retained terminal task run");
    assert_eq!(
        oldest_retained_id, 5_001,
        "the pass deletes the oldest records before newer equal-timestamp ties"
    );
    let invalid_timestamp_remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success' AND started_at = '2026-02-30T08:45:00.000Z'",
    )
    .fetch_one(&pool)
    .await
    .expect("count preserved invalid retention timestamp");
    assert_eq!(invalid_timestamp_remaining, 1);
    let running_remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM system_task_runs WHERE status = 'running'")
            .fetch_one(&pool)
            .await
            .expect("count running task runs");
    assert_eq!(running_remaining, 1, "running task runs are never pruned");
    assert_eq!(
        prune_system_task_runs(&pool, false)
            .await
            .expect("immediate retention pass should be throttled"),
        0,
        "retention passes are spaced by at least 15 seconds"
    );
    reset_system_task_run_retention_schedule();
    assert_eq!(
        prune_system_task_runs(&pool, true)
            .await
            .expect("count remaining retention candidates"),
        800,
        "dry run counts each eligible row once and respects the per-pass cap"
    );
    reset_system_task_run_retention_schedule();
}

#[tokio::test]
async fn system_task_run_retention_pressure_marks_the_maintenance_pass_deferred() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_TEST_LOCK.lock().await;
    let generation_before = retention_defer_generation();
    let pressure_error = anyhow::anyhow!("pool timed out while waiting for an open connection");
    assert!(system_task_run_retention_handle_pressure(&pressure_error));
    assert!(
        retention_defer_generation() > generation_before,
        "pressure must wake the five-minute maintenance retry path"
    );
    reset_system_task_run_retention_schedule();
}

pub(crate) async fn test_state_with_openai_base(openai_base: Url) -> Arc<AppState> {
    test_state_with_openai_base_and_body_limit(
        openai_base,
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_and_runtime_projection_mode(
    openai_base: Url,
    runtime_projection_mode: RuntimeProjectionMode,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
        config,
        true,
        immediate_test_pool_no_available_wait_settings(),
        runtime_projection_mode,
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_and_body_limit(
    openai_base: Url,
    body_limit: usize,
) -> Arc<AppState> {
    test_state_with_openai_base_body_limit_and_read_timeout(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_body_limit_and_read_timeout(
    openai_base: Url,
    body_limit: usize,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    test_state_with_openai_base_and_proxy_timeouts(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS),
        request_read_timeout,
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_and_proxy_timeouts(
    openai_base: Url,
    body_limit: usize,
    handshake_timeout: Duration,
    compact_handshake_timeout: Duration,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    config.openai_proxy_max_request_body_bytes = body_limit;
    config.openai_proxy_handshake_timeout = handshake_timeout;
    config.openai_proxy_compact_handshake_timeout = compact_handshake_timeout;
    config.openai_proxy_request_read_timeout = request_read_timeout;
    test_state_from_config(config, true).await
}

pub(crate) async fn test_state_with_openai_base_and_pool_no_available_wait(
    openai_base: Url,
    timeout: Duration,
    _poll_interval: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout,
            poll_interval: _poll_interval,
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await
}

pub(crate) async fn test_state_from_config(
    config: AppConfig,
    startup_ready: bool,
) -> Arc<AppState> {
    test_state_from_config_with_pool_no_available_wait(
        config,
        startup_ready,
        immediate_test_pool_no_available_wait_settings(),
    )
    .await
}

pub(crate) async fn complete_prompt_cache_conversation_materialization_for_test(
    pool: &Pool<Sqlite>,
) {
    let _timing = test_timing_span("fixture_materialization");
    for _ in 0..32 {
        let outcome = run_prompt_cache_conversations_materialization(pool, 2_000, None)
            .await
            .expect("complete prompt-cache conversation materialization");
        if outcome.complete {
            return;
        }
    }
    panic!("prompt-cache conversation materialization did not complete in the test budget");
}

pub(crate) fn immediate_test_pool_no_available_wait_settings() -> PoolNoAvailableWaitSettings {
    PoolNoAvailableWaitSettings {
        timeout: Duration::ZERO,
        poll_interval: Duration::ZERO,
        retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
    }
}

fn isolate_default_test_runtime_path(path: &Path, default_root: &Path, db_id: u64) -> PathBuf {
    if path != default_root {
        return path.to_path_buf();
    }
    let isolated = default_root.join(format!("{}-{db_id}", std::process::id()));
    fs::create_dir_all(&isolated).expect("create isolated test runtime dir");
    isolated
}

fn isolate_stateful_test_config_runtime_paths(mut config: AppConfig, db_id: u64) -> AppConfig {
    config.archive_dir = isolate_default_test_runtime_path(
        &config.archive_dir,
        &test_runtime_path("archive-tests"),
        db_id,
    );
    config.proxy_raw_dir = isolate_default_test_runtime_path(
        &config.proxy_raw_dir,
        &test_runtime_path("proxy-raw-tests"),
        db_id,
    );
    config.xray_runtime_dir = isolate_default_test_runtime_path(
        &config.xray_runtime_dir,
        &test_runtime_path("xray-forward-tests"),
        db_id,
    );
    config
}

pub(crate) async fn test_state_from_config_with_pool_no_available_wait(
    config: AppConfig,
    startup_ready: bool,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
) -> Arc<AppState> {
    test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
        config,
        startup_ready,
        pool_no_available_wait,
        RuntimeProjectionMode::Auto,
    )
    .await
}

pub(crate) async fn test_current_schema_pool() -> SqlitePool {
    let _timing = test_timing_span("fixture_pool_connect");
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url =
        format!("sqlite:file:codex-vibe-monitor-test-pool-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        // Keep the shared in-memory schema available while a test acquires connections.
        .min_connections(1)
        .max_connections(4)
        .connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    restore_test_state_schema_template(&pool)
        .await
        .expect("initialize current-schema test pool");
    pool
}

async fn test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
    config: AppConfig,
    startup_ready: bool,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
    runtime_projection_mode: RuntimeProjectionMode,
) -> Arc<AppState> {
    let _timing = test_timing_span("fixture_setup");
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let config = isolate_stateful_test_config_runtime_paths(config, db_id);
    let db_url = format!("sqlite:file:codex-vibe-monitor-test-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        // A shared in-memory database is destroyed when its final connection closes.
        // Keep its schema alive while concurrent test tasks acquire and release connections.
        .min_connections(1)
        .max_connections(4)
        .connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    restore_test_state_schema_template(&pool)
        .await
        .expect("schema should initialize from the stateful template");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;
    // Ordinary stateful tests model a process that has already completed the one-time startup
    // warming pass. Dedicated recovery tests use `test_current_schema_pool` to observe the
    // migration's pending marker before it is consumed.
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&pool)
    .await
    .expect("complete the startup recovery baseline for ordinary stateful tests");

    let _state_timing = test_timing_span("fixture_state_build");
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");
    let prompt_cache_conversation_cache =
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let sqlite_batch_writer = SqliteBatchWriter::spawn_for_test_with_prompt_cache(
        prompt_cache_conversation_cache.clone(),
    );
    let proxy_runtime_invocations =
        Arc::new(ProxyRuntimeInvocationStore::new(runtime_projection_mode));
    sqlite_batch_writer.set_terminal_runtime_store(proxy_runtime_invocations.clone());

    Arc::new(AppState {
        system_storage: Arc::new(SystemStorageRuntime::new(&config)),
        config: config.clone(),
        sqlite_batch_writer,
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations,
        proxy_runtime_attempts: Arc::new(RuntimeAttemptStore::default()),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        subscription_hub: Arc::new(crate::SubscriptionHub::new()),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        process_started_at_utc: chrono::Utc::now(),
        observability: ObservabilityRuntime::disabled_for_tests(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        raw_capture_circuit: Arc::new(RawCaptureCircuitBreaker::new(
            config.resolved_proxy_raw_dir(),
        )),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
        prompt_cache_conversation_cache,
        dashboard_activity_snapshot_cache: Arc::new(Mutex::new(
            DashboardActivitySnapshotCacheState::default(),
        )),
        terminal_projection_hub: Arc::new(crate::TerminalProjectionHub::default()),
        long_term_projection_runtime: Arc::new(Mutex::new(
            crate::LongTermProjectionRuntime::default(),
        )),
        memory_diagnostics: Arc::new(crate::MemoryDiagnosticsRuntime::default()),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: Some(Duration::ZERO),
        pool_no_available_wait,
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sqlite_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn schema_object_signature(
    pool: &SqlitePool,
) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'table', 'trigger', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("load schema signature")
}

async fn schema_default_data_signature(pool: &SqlitePool) -> Vec<(String, Vec<String>)> {
    let table_names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("load schema table names");
    let mut table_rows = Vec::with_capacity(table_names.len());
    for table_name in table_names {
        let columns = sqlx::query_scalar::<_, String>(&format!(
            "SELECT name FROM pragma_table_info({}) ORDER BY cid",
            quote_sqlite_string(&table_name)
        ))
        .fetch_all(pool)
        .await
        .expect("load schema signature columns");
        let values = columns
            .iter()
            .map(|column| {
                let identifier = quote_sqlite_identifier(column);
                if column.ends_with("_at") {
                    format!(
                        "CASE WHEN {identifier} IS NULL THEN 'NULL' ELSE '<initialization-timestamp>' END"
                    )
                } else {
                    format!("quote({identifier})")
                }
            })
            .collect::<Vec<_>>()
            .join(" || char(31) || ");
        let rows = if values.is_empty() {
            Vec::new()
        } else {
            sqlx::query_scalar::<_, String>(&format!(
                "SELECT {values} FROM {} ORDER BY 1",
                quote_sqlite_identifier(&table_name)
            ))
            .fetch_all(pool)
            .await
            .expect("load schema default data signature")
        };
        table_rows.push((table_name, rows));
    }
    table_rows
}

#[tokio::test]
async fn stateful_schema_template_matches_fresh_schema_and_keeps_pooled_databases_isolated() {
    let temp_dir = make_temp_test_dir("stateful-schema-template-parity");
    let template_path = temp_dir.join("current-schema.db");
    write_current_schema_template(&template_path)
        .await
        .expect("write stateful schema template");

    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let template_url =
        format!("sqlite:file:codex-vibe-monitor-template-parity-{db_id}?mode=memory&cache=shared");
    let fresh_url =
        format!("sqlite:file:codex-vibe-monitor-fresh-parity-{db_id}?mode=memory&cache=shared");
    let isolated_url = format!(
        "sqlite:file:codex-vibe-monitor-template-isolated-{db_id}?mode=memory&cache=shared"
    );
    let template_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(&template_url)
        .await
        .expect("connect template schema parity sqlite");
    restore_current_schema_template_from_path(&template_pool, &template_path)
        .await
        .expect("restore template schema parity sqlite");
    let fresh_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(&fresh_url)
        .await
        .expect("connect fresh schema parity sqlite");
    ensure_schema(&fresh_pool)
        .await
        .expect("initialize fresh schema parity sqlite");

    assert_eq!(
        schema_object_signature(&template_pool).await,
        schema_object_signature(&fresh_pool).await,
        "template schema must match fresh ensure_schema output"
    );
    assert_eq!(
        schema_default_data_signature(&template_pool).await,
        schema_default_data_signature(&fresh_pool).await,
        "template default data must match fresh ensure_schema output"
    );

    let mut first_connection = template_pool
        .acquire()
        .await
        .expect("acquire first template connection");
    sqlx::query("CREATE TABLE template_pool_visibility (value INTEGER NOT NULL)")
        .execute(&mut *first_connection)
        .await
        .expect("create pooled visibility table");
    sqlx::query("INSERT INTO template_pool_visibility (value) VALUES (7)")
        .execute(&mut *first_connection)
        .await
        .expect("write through first template connection");

    let mut second_connection = template_pool
        .acquire()
        .await
        .expect("acquire second template connection");
    let visible_value: i64 = sqlx::query_scalar("SELECT value FROM template_pool_visibility")
        .fetch_one(&mut *second_connection)
        .await
        .expect("template write must be visible to a second pooled connection");
    assert_eq!(visible_value, 7);
    sqlx::query("INSERT INTO template_pool_visibility (value) VALUES (11)")
        .execute(&mut *second_connection)
        .await
        .expect("write through second template connection");
    let visible_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM template_pool_visibility")
        .fetch_one(&mut *first_connection)
        .await
        .expect("second pooled connection write must be visible to the first connection");
    assert_eq!(visible_count, 2);

    let isolated_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(&isolated_url)
        .await
        .expect("connect isolated template sqlite");
    restore_current_schema_template_from_path(&isolated_pool, &template_path)
        .await
        .expect("restore isolated template sqlite");
    let isolated_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'template_pool_visibility'",
    )
    .fetch_one(&isolated_pool)
    .await
    .expect("query isolated template schema");
    assert_eq!(
        isolated_count, 0,
        "template pools must not share test writes"
    );

    drop(second_connection);
    drop(first_connection);
    template_pool.close().await;
    fresh_pool.close().await;
    isolated_pool.close().await;
    fs::remove_dir_all(temp_dir).expect("remove stateful schema template parity directory");
}
