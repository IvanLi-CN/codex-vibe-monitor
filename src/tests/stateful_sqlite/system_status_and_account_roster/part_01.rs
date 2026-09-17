pub(crate) const STATEFUL_SCHEMA_TEMPLATE_PATH_ENV: &str =
    "CODEX_VIBE_MONITOR_STATEFUL_SCHEMA_TEMPLATE_PATH";
pub(crate) const ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV: &str =
    "CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH";

pub(crate) static SYSTEM_TASK_RUN_RETENTION_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

pub(crate) fn write_backfill_response_payload_with_service_tier(
    path: &Path,
    service_tier: Option<&str>,
) {
    let mut response = json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 88,
                "output_tokens": 22,
                "total_tokens": 110,
                "input_tokens_details": { "cached_tokens": 9 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }
    });
    if let Some(service_tier) = service_tier {
        response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }
    let raw = [
        "event: response.completed".to_string(),
        format!("data: {response}"),
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(path, compressed).expect("write response payload");
}

fn seed_system_status_files(state: &Arc<AppState>, temp_dir: &Path) -> (PathBuf, PathBuf) {
    let archive_root = resolved_archive_dir(&state.config)
        .join("codex_invocations")
        .join("2026");
    fs::create_dir_all(&archive_root).expect("create archive tree");
    let archive_file = archive_root.join("codex_invocations-2026-06.sqlite.gz");
    fs::write(&archive_file, vec![b'a'; 17]).expect("write archive payload");
    let other_archive_root = resolved_archive_dir(&state.config)
        .join("pool_upstream_request_attempts")
        .join("2026");
    fs::create_dir_all(&other_archive_root).expect("create non-invocation archive tree");
    let other_archive_file =
        other_archive_root.join("pool-upstream-request-attempts-2026-06.sqlite.gz");
    fs::write(&other_archive_file, vec![b'o'; 23]).expect("write non-invocation archive payload");

    let raw_dir = state.config.resolved_proxy_raw_dir();
    fs::create_dir_all(&raw_dir).expect("create raw dir");
    let request_only_file = raw_dir.join("request-only.bin");
    fs::write(&request_only_file, vec![b'r'; 7]).expect("write request raw payload");
    let response_only_file = raw_dir.join("response-only.bin.gz");
    fs::write(&response_only_file, vec![b's'; 11]).expect("write response raw payload");
    let request_both_file = raw_dir.join("request-both.bin");
    fs::write(&request_both_file, vec![b't'; 13]).expect("write shared request raw payload");
    let response_both_file = raw_dir.join("response-both.bin.gz");
    fs::write(&response_both_file, vec![b'u'; 17]).expect("write shared response raw payload");

    let xray_runtime_dir = &state.config.xray_runtime_dir;
    let runtime_state_dir = xray_runtime_dir.join("state");
    fs::create_dir_all(&runtime_state_dir).expect("create runtime state dir");
    fs::write(xray_runtime_dir.join("config.json"), vec![b'x'; 11]).expect("write runtime file");
    fs::write(runtime_state_dir.join("access.log"), vec![b'y'; 13])
        .expect("write nested runtime file");

    let unrelated_file = temp_dir.join("notes.txt");
    fs::write(&unrelated_file, vec![b'n'; 19]).expect("write unrelated file");
    (archive_file, other_archive_file)
}

async fn seed_system_status_invocations(state: &Arc<AppState>) {
    for (invoke_id, occurred_at, status, request_raw_path, response_raw_path) in [
        (
            "sys-request-1",
            "2026-06-22 12:00:00",
            "success",
            Some("proxy-raw/request-only.bin"),
            None,
        ),
        (
            "sys-request-dup",
            "2026-06-22 12:01:00",
            "failed",
            Some("proxy-raw/request-only.bin"),
            None,
        ),
        (
            "sys-response-1",
            "2026-06-22 12:02:00",
            "success",
            None,
            Some("proxy-raw/response-only.bin.gz"),
        ),
        (
            "sys-response-dup",
            "2026-06-22 12:03:00",
            "failed",
            None,
            Some("proxy-raw/response-only.bin.gz"),
        ),
        (
            "sys-both",
            "2026-06-22 12:04:00",
            "success",
            Some("proxy-raw/request-both.bin"),
            Some("proxy-raw/response-both.bin.gz"),
        ),
        (
            "sys-missing",
            "2026-06-22 12:05:00",
            "running",
            Some("proxy-raw/missing.bin"),
            None,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                raw_response,
                request_raw_path,
                response_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind("{}")
        .bind(request_raw_path)
        .bind(response_raw_path)
        .execute(&state.pool)
        .await
        .expect("insert raw invocation");
    }
}

async fn seed_system_status_archive_manifests(
    state: &Arc<AppState>,
    archive_file: &Path,
    other_archive_file: &Path,
) {
    let archive_root = archive_file
        .parent()
        .expect("archive file should have parent");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("codex_invocations")
    .bind("2026-06")
    .bind(archive_file.to_string_lossy().to_string())
    .bind("sha-success")
    .bind(4_i64)
    .bind("completed")
    .bind("2026-06-22 12:10:00")
    .execute(&state.pool)
    .await
    .expect("insert completed archive batch");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("codex_invocations")
    .bind("2026-06")
    .bind(
        archive_root
            .join("pending.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("sha-pending")
    .bind(99_i64)
    .bind("pending")
    .bind("2026-06-22 12:11:00")
    .execute(&state.pool)
    .await
    .expect("insert pending archive batch");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind("2026-06")
    .bind(other_archive_file.to_string_lossy().to_string())
    .bind("sha-other")
    .bind(8_i64)
    .bind("completed")
    .bind("2026-06-22 12:12:00")
    .execute(&state.pool)
    .await
    .expect("insert non-invocation completed archive batch");
}

#[tokio::test]
pub(crate) async fn system_status_aggregates_counts_and_file_sizes() {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "system-status-aggregation",
        StdDuration::from_millis(100),
    )
    .await;
    let (archive_file, other_archive_file) = seed_system_status_files(&state, &temp_dir);
    seed_system_status_invocations(&state).await;
    seed_system_status_archive_manifests(&state, &archive_file, &other_archive_file).await;

    refresh_system_raw_payload_metrics_inventory(state.as_ref())
        .await
        .expect("refresh persisted raw metrics snapshot");
    hydrate_system_status_snapshot(state.as_ref())
        .await
        .expect("hydrate system status snapshot");
    let response = load_system_status_cached(state.as_ref())
        .await
        .expect("load cached system status");

    assert_eq!(response.live_invocations_count, 6);
    assert_eq!(response.success_count, 3);
    assert_eq!(response.non_success_count, 3);
    assert_eq!(response.completed_archive_batches_count, 1);
    assert_eq!(response.archived_bodies.count, 4);
    assert_eq!(response.archived_bodies.bytes, 17);
    assert_eq!(response.raw_bodies.count, 4);
    assert_eq!(response.raw_bodies.bytes, 48);
    assert_eq!(response.request_raw_bodies.count, 2);
    assert_eq!(response.request_raw_bodies.bytes, 20);
    assert_eq!(response.response_raw_bodies.count, 2);
    assert_eq!(response.response_raw_bodies.bytes, 28);
    assert_eq!(response.raw_metrics_health.state, "ready");
    let response_json = serde_json::to_value(&response).expect("serialize system status");
    assert_eq!(
        response_json["runtimePressureHealth"]["writerAccounting"]["state"],
        "healthy"
    );
    assert!(response_json["runtimePressureHealth"]["process"]["rssAnonBytes"].is_u64());
    assert_eq!(
        response_json["runtimePressureHealth"]["requestPipeline"]["mode"],
        "projection"
    );
    assert!(
        response_json["runtimePressureHealth"]["dashboardProjection"]["sliceCounters"]["current"]
            ["buildCount"]
            .is_u64()
    );
    assert!(
        response_json["runtimePressureHealth"]["delivery"]["activity"]["serializationCount"]
            .is_u64()
    );
    assert!(
        response_json["runtimePressureHealth"]["requestPipeline"]["semanticParseCount"].is_u64()
    );
    assert_eq!(
        response_json["runtimePressureHealth"]["retentionWriteHealth"]["state"],
        "healthy"
    );
    assert!(
        response_json["runtimePressureHealth"]["retentionWriteHealth"]["budgetBreachCount"]
            .is_u64()
    );
    assert!(
        response.database_bytes > 0,
        "database bytes should include sqlite files"
    );
    assert_eq!(response.other_files_bytes, 24);
    assert!(
        parse_to_utc_datetime(&response.refreshed_at).is_some(),
        "refreshedAt should be an ISO UTC timestamp"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn system_raw_metrics_inventory_tracks_raw_attached_after_invocation_cursor_advanced()
 {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "system-status-late-raw-link",
        StdDuration::from_millis(100),
    )
    .await;
    let raw_dir = state.config.resolved_proxy_raw_dir();
    fs::create_dir_all(&raw_dir).expect("create raw dir");
    let request_path = raw_dir.join("request.zst");
    let response_path = raw_dir.join("response.zst");
    fs::write(&request_path, b"rq").expect("write request raw");
    fs::write(&response_path, b"rsp").expect("write response raw");

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, raw_response, request_raw_path, request_raw_size) VALUES (?1, ?2, 'proxy', '', ?3, 2)",
    )
    .bind("late-raw-link")
    .bind("2026-08-03T00:00:00.000Z")
    .bind(request_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert invocation with request raw");
    refresh_system_raw_payload_metrics_inventory(state.as_ref())
        .await
        .expect("advance legacy and link cursors");

    sqlx::query("UPDATE codex_invocations SET response_raw_path = ?2, response_raw_size = 3 WHERE invoke_id = ?1")
        .bind("late-raw-link")
        .bind(response_path.to_string_lossy().as_ref())
        .execute(&state.pool)
        .await
        .expect("attach response raw after invocation cursor advanced");
    refresh_system_raw_payload_metrics_inventory(state.as_ref())
        .await
        .expect("consume late response blob link");

    let response = load_system_status_uncached(state.as_ref())
        .await
        .expect("load raw metrics snapshot");
    assert_eq!(response.raw_bodies.count, 2);
    assert_eq!(response.raw_bodies.bytes, 5);
    assert_eq!(response.request_raw_bodies.count, 1);
    assert_eq!(response.response_raw_bodies.count, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn system_status_surfaces_runtime_raw_metrics_deferral_without_a_db_write() {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "system-status-raw-metrics-deferred",
        StdDuration::from_millis(100),
    )
    .await;
    sqlx::query(
        "UPDATE system_raw_payload_metrics SET inventory_state = 'ready' WHERE singleton = 1",
    )
    .execute(&state.pool)
    .await
    .expect("seed ready inventory state");

    set_system_raw_metrics_health_override(state.as_ref(), Some("deferred")).await;
    let deferred = load_system_status_uncached(state.as_ref())
        .await
        .expect("load deferred status");
    assert_eq!(deferred.raw_metrics_health.state, "deferred");

    set_system_raw_metrics_health_override(state.as_ref(), None).await;
    let recovered = load_system_status_uncached(state.as_ref())
        .await
        .expect("load recovered status");
    assert_eq!(recovered.raw_metrics_health.state, "ready");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn system_status_cached_snapshot_applies_raw_metrics_override_without_io() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    hydrate_system_status_snapshot(state.as_ref())
        .await
        .expect("hydrate system status snapshot");
    let baseline = load_system_status_cached(state.as_ref())
        .await
        .expect("load hydrated status snapshot");

    set_system_raw_metrics_health_override(state.as_ref(), Some("deferred")).await;
    state.pool.close().await;

    let deferred = load_system_status_cached(state.as_ref())
        .await
        .expect("serve patched status cache without SQLite");
    assert_eq!(deferred.raw_metrics_health.state, "deferred");

    set_system_raw_metrics_health_override(state.as_ref(), None).await;
    let restored = load_system_status_cached(state.as_ref())
        .await
        .expect("restore cached durable inventory state without SQLite");
    assert_eq!(
        restored.raw_metrics_health.state,
        baseline.raw_metrics_health.state
    );
}

#[tokio::test]
pub(crate) async fn runtime_pressure_health_serializes_without_sql() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state.proxy_runtime_invocations.record_request_pipeline(
        "file",
        1,
        0,
        REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES,
        Some("request_json_invalid"),
    );
    let event_bus = state.subscription_hub.runtime_mutation_bus();
    event_bus.record_router_batch(4, 2);
    event_bus.record_topic_work(2);
    event_bus.record_router_lag();
    event_bus.record_router_gap();
    event_bus.record_cursor_recovery();
    state.pool.close().await;

    let health = load_runtime_pressure_health(state.as_ref()).await;
    let payload = serde_json::to_value(health).expect("serialize runtime pressure health");

    assert_eq!(payload["state"], "degraded");
    assert!(payload["process"]["rssAnonBytes"].is_u64());
    assert!(payload["process"]["swapBytes"].is_u64());
    assert!(payload["process"]["managedBytes"].is_u64());
    assert!(payload["process"]["unattributedAnonBytes"].is_u64());
    assert!(payload["allocator"]["mallocArenaMax"].is_string());
    assert_eq!(payload["writerAccounting"]["state"], "healthy");
    assert!(payload["databasePressure"]["pressureEvents"].is_u64());
    assert!(payload["databasePressure"]["sqliteBusyEvents"].is_u64());
    assert!(payload["databasePressure"]["sqliteLockedEvents"].is_u64());
    assert!(payload["databasePressure"]["poolAcquireTimeoutEvents"].is_u64());
    assert_eq!(payload["dashboardProjection"]["mode"], "auto");
    assert_eq!(payload["dashboardProjection"]["livePathDbReadCount"], 0);
    assert!(payload["dashboardProjection"]["buildCount"].is_u64());
    assert!(payload["dashboardProjection"]["activeSubscriberCount"].is_u64());
    assert!(
        payload["dashboardProjection"]["sliceCounters"]["terminal"]["cadenceMissCount"].is_u64()
    );
    assert!(payload["delivery"]["summary"]["activeSubscriberCount"].is_u64());
    assert!(payload["delivery"]["summary"]["builderCount"].is_u64());
    assert!(payload["delivery"]["summary"]["frameBytesCount"].is_u64());
    assert!(payload["delivery"]["summary"]["frameReused"].is_u64());
    assert!(payload["delivery"]["summary"]["cursorAdvanced"].is_u64());
    assert_eq!(payload["dashboardHotTopics"]["state"], "healthy");
    for topic in [
        "activity",
        "summary",
        "networkTimeseries",
        "networkRecent",
        "workingConversations",
        "parallelWork",
        "timeseries",
    ] {
        assert_eq!(
            payload["dashboardHotTopics"][topic]["topicClass"],
            "hot_projection"
        );
        assert_eq!(payload["dashboardHotTopics"][topic]["state"], "healthy");
        assert!(payload["dashboardHotTopics"][topic]["builderCount"].is_u64());
        assert!(payload["dashboardHotTopics"][topic]["genericFallbackBuildCount"].is_u64());
        assert!(payload["dashboardHotTopics"][topic]["livePathDbReadCount"].is_u64());
        assert!(payload["dashboardHotTopics"][topic]["cadenceMissCount"].is_u64());
        assert!(payload["dashboardHotTopics"][topic]["reconnectChurnCount"].is_u64());
    }
    assert!(payload["promptCacheProjection"]["failedOrStaleTopicCount"].is_u64());
    assert_eq!(payload["requestPipeline"]["lastSnapshotKind"], "file");
    assert_eq!(payload["requestPipeline"]["semanticParseCount"], 1);
    assert_eq!(
        payload["requestPipeline"]["wholeBodyMaterializationCount"],
        0
    );
    assert_eq!(
        payload["requestPipeline"]["rewriteBufferPeakBytes"],
        REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
    );
    assert_eq!(
        payload["requestPipeline"]["lastFallbackReason"],
        "request_json_invalid"
    );
    assert_eq!(payload["eventBus"]["state"], "degraded");
    assert_eq!(payload["eventBus"]["processedEventCount"], 2);
    assert_eq!(payload["eventBus"]["coalescedEventCount"], 2);
    assert_eq!(payload["eventBus"]["businessPayloadCloneCount"], 0);
    assert_eq!(payload["eventBus"]["cursorRecoveryCount"], 1);
    assert!(payload["backfill"]["state"].is_string());
}

async fn seed_system_task_route_fixture(
    state: &Arc<AppState>,
) -> (
    SystemTaskRunHandle,
    SystemTaskRunHandle,
    SystemTaskRunHandle,
) {
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
        Some("disk full".to_string()),
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
    (startup_handle, retention_handle, tied_handle)
}

async fn assert_filtered_system_tasks(
    state: &Arc<AppState>,
    _retention_handle: &SystemTaskRunHandle,
) {
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
    assert_eq!(filtered.items[0].detail.as_deref(), Some("disk full"));
    assert!(filtered.items[0].duration_ms.unwrap_or_default() >= 0);
}

async fn assert_system_task_pagination(
    state: &Arc<AppState>,
    startup_handle: &SystemTaskRunHandle,
    retention_handle: &SystemTaskRunHandle,
    tied_handle: &SystemTaskRunHandle,
) {
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
}

async fn assert_system_task_cursor_pagination(
    state: &Arc<AppState>,
    startup_handle: &SystemTaskRunHandle,
    retention_handle: &SystemTaskRunHandle,
    tied_handle: &SystemTaskRunHandle,
) {
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
    assert_eq!(first_cursor_page.total, 3);
    assert_eq!(first_cursor_page.items[0].id, tied_handle.id);
    assert_eq!(second_cursor_page.items[0].id, retention_handle.id);
    assert_eq!(third_cursor_page.items[0].id, startup_handle.id);
    assert!(third_cursor_page.next_cursor.is_none());
}

async fn assert_system_task_routes(state: &Arc<AppState>) {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;

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

    let app = build_app_router(state.clone());
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
pub(crate) async fn system_task_runs_filter_and_routes_serve_json() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let (startup_handle, retention_handle, tied_handle) =
        seed_system_task_route_fixture(&state).await;
    assert_filtered_system_tasks(&state, &retention_handle).await;
    assert_system_task_pagination(&state, &startup_handle, &retention_handle, &tied_handle).await;
    assert_system_task_cursor_pagination(&state, &startup_handle, &retention_handle, &tied_handle)
        .await;
    assert_system_task_routes(&state).await;
}

#[tokio::test]
pub(crate) async fn system_task_runs_cursor_preserves_invalid_legacy_timestamp() {
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
pub(crate) async fn system_task_runs_time_ranges_exclude_canonical_shaped_invalid_dates() {
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

use super::*;
