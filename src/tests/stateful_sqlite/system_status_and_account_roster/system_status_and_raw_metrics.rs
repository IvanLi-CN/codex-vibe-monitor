use super::super::*;
use serde_json::json;

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

#[tokio::test]
async fn system_status_aggregates_counts_and_file_sizes() {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "system-status-aggregation",
        StdDuration::from_millis(100),
    )
    .await;

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
    assert_eq!(response.archived_bodies.bytes, Some(17));
    assert_eq!(response.raw_bodies.count, 4);
    assert_eq!(response.raw_bodies.bytes, Some(48));
    assert_eq!(response.request_raw_bodies.count, 2);
    assert_eq!(response.request_raw_bodies.bytes, Some(20));
    assert_eq!(response.response_raw_bodies.count, 2);
    assert_eq!(response.response_raw_bodies.bytes, Some(28));
    assert_eq!(response.raw_metrics_health.state, "ready");
    assert_eq!(response.raw_metrics_health.physical_coverage, "partial");
    let response_json = serde_json::to_value(&response).expect("serialize system status");
    assert_eq!(response_json["rawBodies"]["bytes"], 48);
    assert_eq!(
        response_json["rawMetricsHealth"]["physicalCoverage"],
        "partial"
    );
    assert_eq!(
        response_json["runtimePressureHealth"]["writerAccounting"]["state"],
        "healthy"
    );
    assert!(response_json["runtimePressureHealth"]["process"]["rssAnonBytes"].is_u64());
    assert_eq!(
        response_json["runtimePressureHealth"]["requestPipeline"]["mode"],
        "projection"
    );
    assert!(response_json["runtimePressureHealth"]["dashboardProjection"]["sliceCounters"]["current"]["buildCount"].is_u64());
    assert!(
        response_json["runtimePressureHealth"]["delivery"]["activity"]["serializationCount"]
            .is_u64()
    );
    assert!(
        response_json["runtimePressureHealth"]["requestPipeline"]["semanticParseCount"].is_u64()
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
async fn system_storage_endpoint_reads_only_the_memory_snapshot_when_sqlite_is_closed() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:1/").expect("valid test upstream URL"),
    )
    .await;
    state.pool.close().await;

    let response = build_app_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/system/storage")
                .body(Body::empty())
                .expect("build storage request"),
        )
        .await
        .expect("serve storage route without SQLite");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read storage response");
    let payload: Value = serde_json::from_slice(&body).expect("decode storage response");
    assert!(payload["totalBytes"].is_null());
    assert!(payload["sampledAt"].is_null());
    assert_eq!(payload["state"], "unknown");
    assert_eq!(payload["scanInProgress"], false);
    assert_eq!(payload["stale"], false);
}

#[tokio::test]
async fn system_raw_metrics_inventory_tracks_raw_attached_after_invocation_cursor_advanced() {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
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
    assert_eq!(response.raw_bodies.bytes, Some(5));
    assert_eq!(response.request_raw_bodies.count, 1);
    assert_eq!(response.response_raw_bodies.count, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn system_raw_metrics_inventory_revalidates_existing_paths_after_hydration() {
    use std::time::Duration as StdDuration;

    let _target_timing = test_timing_target(
        "tests::stateful_sqlite::system_status_and_account_roster::system_status_and_raw_metrics::system_raw_metrics_inventory_revalidates_existing_paths_after_hydration",
    );
    let _whole_test = test_timing_span("whole_test");
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "system-status-raw-revalidation",
        StdDuration::from_millis(100),
    )
    .await;
    let raw_dir = state.config.resolved_proxy_raw_dir();
    fs::create_dir_all(&raw_dir).expect("create raw dir");
    let response_path = raw_dir.join("revalidated-response.bin");
    fs::write(&response_path, b"small").expect("write initial response raw");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, raw_response, response_raw_path, response_raw_size) VALUES (?1, ?2, 'proxy', '', ?3, 5)",
    )
    .bind("revalidation-invocation")
    .bind("2026-08-04T00:00:00.000Z")
    .bind(response_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert invocation with response raw");

    {
        let _operation = test_timing_span("production_operation");
        refresh_system_raw_payload_metrics_inventory(state.as_ref())
            .await
            .expect("initial inventory refresh");
    }
    let initial = load_system_status_uncached(state.as_ref())
        .await
        .expect("load initial raw metrics");
    assert_eq!(initial.raw_bodies.bytes, Some(5));

    fs::write(&response_path, b"expanded-response").expect("expand response raw");
    {
        let _operation = test_timing_span("production_operation");
        hydrate_raw_capture_circuit(state.as_ref())
            .await
            .expect("rehydrate raw circuit");
        refresh_system_raw_payload_metrics_inventory(state.as_ref())
            .await
            .expect("revalidate existing raw path");
    }
    let refreshed = load_system_status_uncached(state.as_ref())
        .await
        .expect("load revalidated raw metrics");
    assert_eq!(refreshed.raw_bodies.bytes, Some(17));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn system_status_surfaces_runtime_raw_metrics_deferral_without_a_db_write() {
    use std::time::Duration as StdDuration;

    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
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
    assert_eq!(deferred.raw_metrics_health.physical_coverage, "unknown");
    assert_eq!(deferred.raw_bodies.bytes, None);
    assert_eq!(deferred.request_raw_bodies.bytes, None);
    assert_eq!(deferred.response_raw_bodies.bytes, None);
    let deferred_json = serde_json::to_value(&deferred).expect("serialize deferred status");
    assert!(deferred_json["rawBodies"]["bytes"].is_null());
    assert_eq!(
        deferred_json["rawMetricsHealth"]["physicalCoverage"],
        "unknown"
    );

    set_system_raw_metrics_health_override(state.as_ref(), None).await;
    let recovered = load_system_status_uncached(state.as_ref())
        .await
        .expect("load recovered status");
    assert_eq!(recovered.raw_metrics_health.state, "ready");
    assert_eq!(recovered.raw_metrics_health.physical_coverage, "partial");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn system_status_cached_snapshot_applies_raw_metrics_override_without_io() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        "UPDATE system_raw_payload_metrics SET inventory_state = 'ready', raw_bytes = 42, request_raw_bytes = 24, response_raw_bytes = 18 WHERE singleton = 1",
    )
    .execute(&state.pool)
    .await
    .expect("seed ready raw metrics snapshot");
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
    assert_eq!(deferred.raw_metrics_health.physical_coverage, "unknown");
    assert_eq!(deferred.raw_bodies.bytes, None);
    assert_eq!(deferred.request_raw_bodies.bytes, None);
    assert_eq!(deferred.response_raw_bodies.bytes, None);

    set_system_raw_metrics_health_override(state.as_ref(), None).await;
    let restored = load_system_status_cached(state.as_ref())
        .await
        .expect("restore cached durable inventory state without SQLite");
    assert_eq!(
        restored.raw_metrics_health.state,
        baseline.raw_metrics_health.state
    );
    assert_eq!(restored.raw_bodies.bytes, Some(42));
    assert_eq!(restored.raw_metrics_health.physical_coverage, "partial");
}

#[tokio::test]
async fn runtime_pressure_health_serializes_without_sql() {
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
    assert!(payload["runtimeAttempts"]["records"].is_u64());
    assert!(payload["runtimeAttempts"]["bytes"].is_u64());
    assert!(payload["runtimeAttempts"]["rejectedRecords"].is_u64());
    assert!(payload["terminalJournal"]["pendingMaterializationRecords"].is_u64());
    assert!(payload["terminalJournal"]["pendingBytes"].is_u64());
    assert!(payload["terminalJournal"]["oldestPendingAgeMs"].is_u64());
    assert!(payload["databasePressure"]["pressureEvents"].is_u64());
    assert!(payload["databasePressure"]["sqliteBusyEvents"].is_u64());
    assert!(payload["databasePressure"]["sqliteLockedEvents"].is_u64());
    assert!(payload["databasePressure"]["poolAcquireTimeoutEvents"].is_u64());
    assert_eq!(payload["rawOrphanSweep"]["state"], "unknown");
    assert!(payload["rawOrphanSweep"]["inspectedEntries"].is_null());
    assert!(payload["rawOrphanSweep"]["failureFingerprint"].is_null());
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
    assert!(matches!(
        payload["retentionRecovery"]["state"].as_str(),
        Some("unknown" | "healthy" | "recovering" | "deferred" | "degraded")
    ));
    assert!(matches!(
        payload["rawCapture"]["state"].as_str(),
        Some("unknown" | "capturing" | "storage_suppressed")
    ));
    assert!(
        payload["rawCapture"]["rawBytes"].is_null() || payload["rawCapture"]["rawBytes"].is_u64()
    );
    assert!(payload["rawCapture"]["reservedBytes"].is_u64());
    assert!(payload["rawCapture"]["rawCloseBytes"].is_u64());
    assert!(payload["rawCapture"]["rawResumeBytes"].is_u64());
    assert!(payload["rawCapture"]["availableCloseBytes"].is_u64());
    assert!(payload["rawCapture"]["availableResumeBytes"].is_u64());
    let raw_capture_fields = payload["rawCapture"]
        .as_object()
        .expect("serialize raw capture as a bounded object");
    for field in raw_capture_fields.keys() {
        let normalized = field.to_ascii_lowercase();
        assert!(
            !["payload", "sql", "account", "path"]
                .iter()
                .any(|sensitive| normalized.contains(sensitive)),
            "raw capture status must not add sensitive field {field}"
        );
    }
    assert!(
        payload["retentionRecovery"]["preparedCount"].is_null()
            || payload["retentionRecovery"]["preparedCount"].is_u64()
    );
    assert!(
        payload["retentionRecovery"]["quarantinedCount"].is_null()
            || payload["retentionRecovery"]["quarantinedCount"].is_u64()
    );
    let recovery_fields = payload["retentionRecovery"]
        .as_object()
        .expect("serialize retention recovery as a bounded object");
    assert_eq!(recovery_fields.len(), 12);
    for field in recovery_fields.keys() {
        let normalized = field.to_ascii_lowercase();
        assert!(
            !["raw", "sql", "account", "path"]
                .iter()
                .any(|sensitive| normalized.contains(sensitive)),
            "retention recovery status must not add sensitive field {field}"
        );
    }
    if let Some(fingerprint) = recovery_fields["failureFingerprint"].as_str() {
        assert_eq!(fingerprint.len(), 16);
        assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

#[test]
fn default_retention_recovery_snapshot_serializes_unmeasured_counts_as_null() {
    let payload =
        serde_json::to_value(crate::maintenance::RetentionRecoveryHealthSnapshot::default())
            .expect("serialize default retention recovery health");

    assert_eq!(payload["state"], "unknown");
    assert!(payload["preparedCount"].is_null());
    assert!(payload["quarantinedCount"].is_null());
    assert!(payload["expiredBacklogCount"].is_null());
}

#[test]
fn default_raw_orphan_sweep_snapshot_serializes_unmeasured_counts_as_null() {
    let payload = serde_json::to_value(crate::maintenance::RawOrphanSweepHealthSnapshot::default())
        .expect("serialize default raw orphan sweep health");

    assert_eq!(payload["state"], "unknown");
    assert!(payload["inspectedEntries"].is_null());
    assert!(payload["referencedSkipped"].is_null());
    assert!(payload["quarantined"].is_null());
    assert!(payload["removed"].is_null());
}

pub(crate) fn write_backfill_response_payload_with_terminal_service_tier(
    path: &Path,
    initial_service_tier: Option<&str>,
    terminal_service_tier: Option<&str>,
) {
    let mut created_response = json!({
        "type": "response.created",
        "response": {
            "id": "resp_backfill",
            "status": "in_progress"
        }
    });
    if let Some(service_tier) = initial_service_tier {
        created_response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }

    let mut completed_response = json!({
        "type": "response.completed",
        "response": {
            "id": "resp_backfill",
            "status": "completed",
            "usage": {
                "input_tokens": 88,
                "output_tokens": 22,
                "total_tokens": 110,
                "input_tokens_details": { "cached_tokens": 9 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }
    });
    if let Some(service_tier) = terminal_service_tier {
        completed_response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }

    let raw = [
        "event: response.created".to_string(),
        format!("data: {created_response}"),
        "".to_string(),
        "event: response.completed".to_string(),
        format!("data: {completed_response}"),
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(path, compressed).expect("write response payload");
}

pub(crate) fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
}

pub(crate) fn write_backfill_request_payload_with_requested_service_tier(
    path: &Path,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(path, None, None, requested_service_tier, target);
}

pub(crate) fn write_backfill_request_payload_with_reasoning(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        reasoning_effort,
        None,
        target,
    );
}

fn write_backfill_request_payload_with_fields(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    let payload = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "metadata": {},
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"]["prompt_cache_key"] = Value::String(key.to_string());
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning"] = json!({ "effort": effort });
            }
            if let Some(service_tier) = requested_service_tier {
                payload["service_tier"] = Value::String(service_tier.to_string());
            }
            payload
        }
        ProxyCaptureTarget::ChatCompletions => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}],
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"] = json!({ "prompt_cache_key": key });
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning_effort"] = Value::String(effort.to_string());
            }
            if let Some(service_tier) = requested_service_tier {
                payload["serviceTier"] = Value::String(service_tier.to_string());
            }
            payload
        }
        ProxyCaptureTarget::StandaloneSearch => json!({
            "model": "gpt-5.3-codex",
            "query": "hello",
        }),
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
            let mut payload = json!({
                "model": "gpt-image-1",
                "prompt": "hello",
            });
            if let Some(service_tier) = requested_service_tier {
                payload["service_tier"] = Value::String(service_tier.to_string());
            }
            payload
        }
    };
    let encoded = serde_json::to_vec(&payload).expect("serialize request payload");
    fs::write(path, encoded).expect("write request payload");
}

pub(crate) async fn insert_proxy_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    response_path: &Path,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
    )
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy row");
}

pub(crate) async fn insert_proxy_cost_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    model: Option<&str>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, model, input_tokens, output_tokens, total_tokens, cost, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(model)
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => Some(input + output),
        _ => None,
    })
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert proxy cost row");
}

pub(crate) async fn insert_proxy_prompt_cache_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    request_path: &Path,
    payload: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(payload)
    .bind("{}")
    .bind(request_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy prompt cache key row");
}
