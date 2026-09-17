async fn enqueue_timeseries_projection_delta(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
) {
    let record =
        api_invocation_from_runtime_record(&test_proxy_capture_record(invoke_id, occurred_at));
    insert_timeseries_invocation(
        &state.pool,
        &record.invoke_id,
        &record.occurred_at,
        "success",
        Some(12.0),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET payload = ?1 WHERE invoke_id = ?2")
        .bind(r#"{"upstreamAccountId":17}"#)
        .bind(&record.invoke_id)
        .execute(&state.pool)
        .await
        .expect("attach the account payload used by projection selections");
    let row_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load durable row id for the terminal delta");
    let event_id = state
        .terminal_projection_hub
        .register_pending(&record, None)
        .expect("terminal event should fit in the projection journal");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &record.invoke_id,
        &record.occurred_at,
        row_id,
    );
}

async fn assert_ready_projection_selection_rows(state: &AppState, expected_count: i64) {
    for (source_scope, upstream_account_key) in [
        ("all", -1_i64),
        ("proxy_only", -1),
        ("all", 17),
        ("proxy_only", 17),
    ] {
        let aggregates = sqlx::query_scalar::<_, String>(
            "SELECT aggregate_json FROM timeseries_minute_projection_v2 \
             WHERE source_scope = ?1 AND upstream_account_key = ?2 AND coverage_state = 'ready' \
             ORDER BY minute_start_epoch",
        )
        .bind(source_scope)
        .bind(upstream_account_key)
        .fetch_all(&state.pool)
        .await
        .expect("load persisted minute aggregates for every projection selection");
        assert_eq!(aggregates.len(), 17);
        assert!(aggregates.iter().all(|json| {
            serde_json::from_str::<serde_json::Value>(json)
                .ok()
                .and_then(|aggregate| aggregate["total_count"].as_i64())
                == Some(expected_count)
        }));
    }
}

async fn warm_all_projection(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    label: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> crate::api::TimeseriesMinuteProjectionWarmOutcome {
    crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        label,
        coordinator,
    )
    .await
    .expect("warm minute projection")
}

async fn seed_non_proxy_warm_race(state: &AppState, occurred_at: &str) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, raw_response) \
         VALUES ('non-proxy-warm-race', ?1, 'cli', 'running', 10, 0.01, 12.0, '{}')",
    )
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("insert non-proxy running invocation");
}

async fn assert_direct_replacement_http_reads(state: Arc<AppState>) {
    for upstream_account_id in [None, Some(17)] {
        let query = TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id,
        };
        let Json(response) = crate::api::fetch_timeseries(
            axum::extract::State(state.clone()),
            axum::extract::Query(query),
        )
        .await
        .expect("direct replacement must fence HTTP projection reads");
        let payload = serde_json::to_value(response).expect("serialize HTTP timeseries response");
        let point = payload["points"]
            .as_array()
            .expect("HTTP timeseries points")
            .iter()
            .find(|point| point["totalCount"] == 1)
            .expect("replacement minute point");
        assert_eq!(point["successCount"], 0);
        assert_eq!(point["failureCount"], 1);
    }
    let topic_query = TimeseriesQuery {
        range: "1h".to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        time_zone: Some("UTC".to_string()),
        upstream_account_id: None,
    };
    let base = crate::api::TimeseriesTopicMaterializedBase::build(state.as_ref(), &topic_query)
        .await
        .expect("direct replacement must fence materialized topic reads");
    let payload: serde_json::Value = serde_json::from_slice(
        &base
            .serialize(&[])
            .expect("serialize exact-fallback materialized response"),
    )
    .expect("materialized timeseries JSON");
    let point = payload["points"]
        .as_array()
        .expect("materialized timeseries points")
        .iter()
        .find(|point| point["totalCount"] == 1)
        .expect("replacement minute point");
    assert_eq!(point["successCount"], 0);
    assert_eq!(point["failureCount"], 1);
}

async fn assert_direct_replacement_recovery(
    state: &AppState,
    minute_start: i64,
    warm_start: DateTime<Utc>,
    warm_end: DateTime<Utc>,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) {
    let recovery = crate::api::prepare_timeseries_minute_projection_after_restart(
        state,
        &CancellationToken::new(),
    )
    .await
    .expect("recover durable replacement coverage in bounded P2 transactions");
    assert_eq!(
        recovery,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    let coverage_state = sqlx::query_scalar::<_, String>(
        "SELECT coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(minute_start)
    .fetch_one(&state.pool)
    .await
    .expect("load recovered coverage state");
    assert_eq!(coverage_state, "warming");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load cleared durable replacement recovery marker");
    assert_eq!(recovery_pending, 0);
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        warm_start,
        warm_end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_non_proxy_recovery_rewarm",
        coordinator,
    )
    .await
    .expect("re-warm durable replacement coverage after P2 recovery");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
}

#[tokio::test]
pub(crate) async fn minute_projection_flush_commits_all_key_slices_before_acknowledging_deltas() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);

    // Each proxy terminal event with an upstream account contributes four selection keys.
    // Seventeen events therefore cross the 64-key transaction boundary.
    for minute in 0..17_i64 {
        let occurred_at = format!("2026-08-21 12:{minute:02}:12");
        enqueue_timeseries_projection_delta(
            &state,
            &format!("timeseries-projection-slice-{minute}"),
            &occurred_at,
        )
        .await;
    }

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_key_slices",
        &coordinator,
    )
    .await
    .expect("flush should commit every bounded slice");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    assert!(
        state
            .terminal_projection_hub
            .pending_timeseries_deltas(100)
            .is_empty(),
        "deltas are acknowledged only after all committed key slices succeed"
    );
    let projection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count all committed minute projection keys");
    assert_eq!(projection_count, 68);
    assert_ready_projection_selection_rows(&state, 1).await;

    for minute in 0..17_i64 {
        let occurred_at = format!("2026-08-21 12:{minute:02}:42");
        enqueue_timeseries_projection_delta(
            &state,
            &format!("timeseries-projection-incremental-{minute}"),
            &occurred_at,
        )
        .await;
    }
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_key_slices_incremental",
        &coordinator,
    )
    .await
    .expect("incremental key slices should flush after the initial rebuild");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    let aggregate_counts = sqlx::query_scalar::<_, String>(
        "SELECT aggregate_json FROM timeseries_minute_projection_v2 WHERE coverage_state = 'ready'",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load aggregate rows after incremental update");
    assert!(
        aggregate_counts.iter().all(|aggregate_json| {
            serde_json::from_str::<serde_json::Value>(aggregate_json)
                .ok()
                .and_then(|aggregate| aggregate["total_count"].as_i64())
                == Some(2)
        }),
        "existing ready rows must incorporate newer distinct durable row ids incrementally"
    );
}

#[tokio::test]
pub(crate) async fn exact_fallback_warm_write_defers_behind_p1_terminal_work() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let start = Utc
        .with_ymd_and_hms(2026, 8, 21, 12, 0, 0)
        .single()
        .expect("valid warm range start");
    let end = start + ChronoDuration::minutes(1);
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let p1_write = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_exact_warm_p1_priority",
        &coordinator,
    )
    .await
    .expect("warm write should defer without a database error");
    assert!(matches!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Deferred(_)
    ));
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count warm projection rows before retry");
    assert_eq!(
        projection_count, 0,
        "P1 preemption must occur before a warm transaction opens"
    );

    drop(p1_write);
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_exact_warm_p1_retry",
        &coordinator,
    )
    .await
    .expect("warm write should retry after P1 completes");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
    let projection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count warm projection rows after retry");
    assert_eq!(projection_count, 1);
}

#[tokio::test]
pub(crate) async fn exact_fallback_warm_rebuilds_after_non_proxy_terminal_invalidation() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let start = Utc
        .with_ymd_and_hms(2026, 8, 21, 12, 0, 0)
        .single()
        .expect("valid warm range start");
    let end = start + ChronoDuration::minutes(1);
    let occurred_at = format_naive(start.with_timezone(&Shanghai).naive_local());
    seed_non_proxy_warm_race(&state, &occurred_at).await;

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = warm_all_projection(
        &state,
        start,
        end,
        "stateful_non_proxy_warm_seed",
        &coordinator,
    )
    .await;
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );

    sqlx::query("UPDATE codex_invocations SET status = 'success' WHERE invoke_id = ?1")
        .bind("non-proxy-warm-race")
        .execute(&state.pool)
        .await
        .expect("terminalize non-proxy invocation");
    let coverage_state = sqlx::query_scalar::<_, String>(
        "SELECT coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(start.timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("load coverage while terminal recovery is pending");
    assert_eq!(coverage_state, "ready");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load terminal recovery marker");
    assert_eq!(recovery_pending, 1);

    let outcome = warm_all_projection(
        &state,
        start,
        end,
        "stateful_non_proxy_warm_rebuild",
        &coordinator,
    )
    .await;
    assert!(matches!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Deferred(_)
    ));

    let recovery = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &CancellationToken::new(),
    )
    .await
    .expect("invalidate terminal coverage through bounded P2 recovery");
    assert_eq!(
        recovery,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    let outcome = warm_all_projection(
        &state,
        start,
        end,
        "stateful_non_proxy_warm_rebuild_after_recovery",
        &coordinator,
    )
    .await;
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
    let (coverage_state, aggregate_json): (String, String) = sqlx::query_as(
        "SELECT coverage_state, aggregate_json FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(start.timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("load rebuilt non-proxy projection");
    let aggregate: serde_json::Value =
        serde_json::from_str(&aggregate_json).expect("valid rebuilt aggregate payload");
    assert_eq!(coverage_state, "ready");
    assert_eq!(aggregate["total_count"].as_i64(), Some(1));
}

#[tokio::test]
pub(crate) async fn exact_fallback_warm_invalidates_direct_non_proxy_terminal_replacement() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, 'cli', 'success', 10, 0.01, ?3, '{}')",
    )
    .bind("non-proxy-terminal-replacement")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": 17_i64 }).to_string())
    .execute(&state.pool)
    .await
    .expect("insert terminal non-proxy invocation");

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let warm_start = now - ChronoDuration::minutes(62);
    let warm_end = now + ChronoDuration::minutes(2);
    for upstream_account_id in [None, Some(17)] {
        let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
            &state.pool,
            warm_start,
            warm_end,
            InvocationSourceScope::All,
            upstream_account_id,
            state.terminal_projection_hub.as_ref(),
            "stateful_non_proxy_terminal_seed",
            &coordinator,
        )
        .await
        .expect("seed ready coverage before direct terminal replacement");
        assert_eq!(
            outcome,
            crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
        );
    }
    let minute_start = parse_to_utc_datetime(&occurred_at)
        .expect("parse persisted occurrence")
        .timestamp()
        .div_euclid(60)
        * 60;

    sqlx::query(
        "UPDATE codex_invocations SET status = 'failed', failure_kind = 'upstream_response_failed', failure_class = 'service_failure', is_actionable = 1 WHERE invoke_id = ?1",
    )
    .bind("non-proxy-terminal-replacement")
    .execute(&state.pool)
    .await
    .expect("replace terminal non-proxy invocation");
    let coverage_state = sqlx::query_scalar::<_, String>(
        "SELECT coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(minute_start)
    .fetch_one(&state.pool)
    .await
    .expect("load coverage while durable replacement recovery is pending");
    assert_eq!(
        coverage_state, "ready",
        "the terminal transaction must only publish the constant-size durable recovery marker"
    );
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load durable replacement recovery marker");
    assert_eq!(
        recovery_pending, 1,
        "direct terminal replacements must fence all minute coverage before it can be reused"
    );

    assert_direct_replacement_http_reads(state.clone()).await;

    assert_direct_replacement_recovery(&state, minute_start, warm_start, warm_end, &coordinator)
        .await;
}

#[tokio::test]
pub(crate) async fn exact_fallback_warm_invalidates_non_proxy_stream_duration_replacement() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, raw_response) VALUES (?1, ?2, 'cli', 'success', 10, 0.01, '{}')",
    )
    .bind("non-proxy-stream-duration-replacement")
    .bind(&occurred_at)
    .execute(&state.pool)
    .await
    .expect("insert terminal non-proxy invocation");

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        now - ChronoDuration::minutes(62),
        now + ChronoDuration::minutes(2),
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_non_proxy_stream_duration_seed",
        &coordinator,
    )
    .await
    .expect("seed ready coverage before stream-duration replacement");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );

    sqlx::query("UPDATE codex_invocations SET t_upstream_stream_ms = 975.0 WHERE invoke_id = ?1")
        .bind("non-proxy-stream-duration-replacement")
        .execute(&state.pool)
        .await
        .expect("replace only the terminal stream duration");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load durable replacement recovery marker");
    assert_eq!(
        recovery_pending, 1,
        "stream-duration-only terminal replacements must fence ready minute coverage"
    );
}

#[tokio::test]
pub(crate) async fn restart_schema_refreshes_existing_objects_across_primed_pool_connections() {
    let temp_dir = make_temp_test_dir("existing-schema-objects-across-pool");
    let database_path = temp_dir.join("projection.db");
    fs::File::create(&database_path).expect("create projection database");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&test_sqlite_url_for_path(&database_path))
        .await
        .expect("open projection database");
    crate::ensure_schema(&pool)
        .await
        .expect("initialize the replacement fence");
    let schema_before = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'table', 'trigger', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(&pool)
    .await
    .expect("load schema before restart");

    let mut connections = Vec::new();
    for _ in 0..5 {
        let mut connection = pool.acquire().await.expect("acquire schema connection");
        let trigger_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_timeseries_minute_projection_non_proxy_terminal_replacement'",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("prime the replacement fence schema cache");
        assert_eq!(trigger_count, 1);
        connections.push(connection);
    }
    drop(connections);

    crate::ensure_schema(&pool)
        .await
        .expect("restart schema initialization must refresh existing schema objects");

    let trigger_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_timeseries_minute_projection_non_proxy_terminal_replacement'",
    )
    .fetch_one(&pool)
    .await
    .expect("count refreshed replacement fence");
    assert_eq!(trigger_count, 1);
    let schema_after = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'table', 'trigger', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(&pool)
    .await
    .expect("load schema after restart");
    assert_eq!(
        schema_after, schema_before,
        "schema refresh must preserve every installed object"
    );
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn restart_schema_installs_durable_non_proxy_replacement_fence_before_projection_supervisor()
 {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let (state, minute_start, _warm_start, _warm_end) = seed_restart_schema_fence_fixture().await;
    sqlx::query("DROP TRIGGER trg_timeseries_minute_projection_non_proxy_terminal_replacement")
        .execute(&state.pool)
        .await
        .expect("model an older database before restart schema initialization");
    crate::ensure_schema(&state.pool)
        .await
        .expect("restart schema initialization installs the durable replacement fence");
    crate::api::mark_timeseries_minute_projection_startup_recovery(&state.pool)
        .await
        .expect("runtime startup marks minute projection recovery before HTTP reads");
    sqlx::query(
        "UPDATE codex_invocations SET status = 'failed', failure_kind = 'upstream_response_failed', failure_class = 'service_failure', is_actionable = 1 WHERE invoke_id = ?1",
    )
    .bind("timeseries-missing-durable-replacement-fence")
    .execute(&state.pool)
    .await
    .expect("replace the terminal row before the projection supervisor starts");
    let (status, failure_class): (String, String) =
        sqlx::query_as("SELECT status, failure_class FROM codex_invocations WHERE invoke_id = ?1")
            .bind("timeseries-missing-durable-replacement-fence")
            .fetch_one(&state.pool)
            .await
            .expect("load direct terminal replacement");
    assert_eq!(status, "failed");
    assert_eq!(failure_class, "service_failure");

    for upstream_account_key in [-1_i64, 17_i64] {
        let coverage_state = sqlx::query_scalar::<_, String>(
            "SELECT coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = ?2",
        )
        .bind(minute_start)
        .bind(upstream_account_key)
        .fetch_one(&state.pool)
        .await
        .expect("load coverage fenced by durable startup recovery");
        assert_eq!(coverage_state, "ready");
    }
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load startup durable recovery marker");
    assert_eq!(recovery_pending, 1);

    assert_direct_replacement_http_reads(state.clone()).await;

    let trigger_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_timeseries_minute_projection_non_proxy_terminal_replacement'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check durable replacement fence is installed");
    assert_eq!(
        trigger_count, 1,
        "schema initialization must install the durable replacement fence before HTTP reads"
    );
}

async fn seed_restart_schema_fence_fixture() -> (Arc<AppState>, i64, DateTime<Utc>, DateTime<Utc>) {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, 'cli', 'success', 10, 0.01, ?3, '{}')",
    )
    .bind("timeseries-missing-durable-replacement-fence")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": 17_i64 }).to_string())
    .execute(&state.pool)
    .await
    .expect("insert terminal non-proxy invocation with an account");
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let warm_start = now - ChronoDuration::minutes(62);
    let warm_end = now + ChronoDuration::minutes(2);
    for upstream_account_id in [None, Some(17)] {
        let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
            &state.pool,
            warm_start,
            warm_end,
            InvocationSourceScope::All,
            upstream_account_id,
            state.terminal_projection_hub.as_ref(),
            "stateful_missing_durable_replacement_fence_seed",
            &coordinator,
        )
        .await
        .expect("seed ready minute projection coverage");
        assert_eq!(
            outcome,
            crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
        );
    }
    let minute_start = parse_to_utc_datetime(&occurred_at)
        .expect("parse persisted occurrence")
        .timestamp()
        .div_euclid(60)
        * 60;
    (state, minute_start, warm_start, warm_end)
}

#[tokio::test]
pub(crate) async fn exact_fallback_warm_acknowledges_the_transaction_snapshot_not_the_response_snapshot()
 {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let start = Utc
        .with_ymd_and_hms(2026, 8, 21, 12, 0, 0)
        .single()
        .expect("valid warm range start");
    let end = start + ChronoDuration::minutes(1);
    let occurred_at = format_naive(start.with_timezone(&Shanghai).naive_local());
    let capture = test_proxy_capture_record("warm-transaction-snapshot", &occurred_at);
    let terminal = api_invocation_from_runtime_record(&capture);
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);
    let event_id = state
        .terminal_projection_hub
        .register_pending(&terminal, None)
        .expect("terminal event should fit in the projection journal");
    let persisted = persist_proxy_capture_record(&state.pool, Instant::now(), capture)
        .await
        .expect("persist terminal row")
        .expect("terminal row should be persisted");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &terminal.invoke_id,
        &terminal.occurred_at,
        persisted.id,
    );

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_warm_transaction_snapshot",
        &coordinator,
    )
    .await
    .expect("warm write should succeed");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
    let pending = state
        .terminal_projection_hub
        .pending_timeseries_deltas_for_selection(
            crate::TimeseriesProjectionSelection {
                source_scope: "all",
                upstream_account_id: None,
            },
            10,
        );
    assert!(
        pending.is_empty(),
        "the committed transaction source row must be acknowledged so a later projection read cannot add it twice; pending={pending:#?}"
    );
    let cursor = sqlx::query_scalar::<_, i64>(
        "SELECT max_row_id FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(start.timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("load committed projection cursor");
    assert_eq!(cursor, persisted.id);
}

#[tokio::test]
pub(crate) async fn startup_minute_projection_recovery_stops_before_writes_when_cancelled() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let outcome = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &cancellation,
    )
    .await
    .expect("cancelled startup recovery should not report a database error");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Cancelled
    );
    let state_row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup state writes");
    assert_eq!(state_row_count, 0);
}

use super::*;
