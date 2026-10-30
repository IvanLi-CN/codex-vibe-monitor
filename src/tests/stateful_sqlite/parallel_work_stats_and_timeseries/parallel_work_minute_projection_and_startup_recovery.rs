use super::*;

#[tokio::test]
async fn minute_projection_flush_defers_while_p1_terminal_write_is_active() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let record = api_invocation_from_runtime_record(&test_proxy_capture_record(
        "timeseries-projection-p1-priority",
        "2026-08-21 12:00:12",
    ));
    insert_timeseries_invocation(
        &state.pool,
        &record.invoke_id,
        &record.occurred_at,
        "success",
        Some(12.0),
    )
    .await;
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);
    let event_id = state
        .terminal_projection_hub
        .register_pending(&record, None)
        .expect("terminal event should fit in the projection journal");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &record.invoke_id,
        &record.occurred_at,
        91_001,
    );

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let p1_write = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_p1_priority",
        &coordinator,
    )
    .await
    .expect("flush should yield without a database error");
    assert!(matches!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Deferred(_)
    ));
    assert_eq!(
        state
            .terminal_projection_hub
            .pending_timeseries_deltas(10)
            .len(),
        1,
        "the P1 preemption path must leave the delta available for retry"
    );
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count minute projections before retry");
    assert_eq!(
        projection_count, 0,
        "deferred work must not open a write transaction"
    );

    drop(p1_write);
    let outcome = crate::api::flush_timeseries_minute_projection_with_coordinator(
        state.as_ref(),
        "stateful_p1_priority_retry",
        &coordinator,
    )
    .await
    .expect("flush should succeed after P1 completes");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    assert!(
        state
            .terminal_projection_hub
            .pending_timeseries_deltas(10)
            .is_empty(),
        "the retried flush should acknowledge the terminal delta"
    );
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count minute projections after retry");
    assert!(
        projection_count > 0,
        "retry should persist the minute projections"
    );
    let aggregate_json = sqlx::query_scalar::<_, String>(
        "SELECT aggregate_json FROM timeseries_minute_projection_v2 WHERE source_scope = 'all' AND upstream_account_key = -1 AND coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load the all-scope minute aggregate after retry");
    let aggregate: serde_json::Value =
        serde_json::from_str(&aggregate_json).expect("valid persisted minute aggregate");
    assert_eq!(
        aggregate["total_count"].as_i64(),
        Some(1),
        "retry must persist the terminal aggregate, not only a ready projection row"
    );
}

#[tokio::test]
async fn minute_projection_flush_commits_all_key_slices_before_acknowledging_deltas() {
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
        let record = api_invocation_from_runtime_record(&test_proxy_capture_record(
            &format!("timeseries-projection-slice-{minute}"),
            &occurred_at,
        ));
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
            .expect("attach the account payload used by all projection selections");
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
    for (source_scope, upstream_account_key) in [
        ("all", -1_i64),
        ("proxy_only", -1),
        ("all", 17),
        ("proxy_only", 17),
    ] {
        let aggregates = sqlx::query_scalar::<_, String>(
            "SELECT aggregate_json FROM timeseries_minute_projection_v2 WHERE source_scope = ?1 AND upstream_account_key = ?2 AND coverage_state = 'ready' ORDER BY minute_start_epoch",
        )
        .bind(source_scope)
        .bind(upstream_account_key)
        .fetch_all(&state.pool)
        .await
        .expect("load persisted minute aggregates for every projection selection");
        assert_eq!(
            aggregates.len(),
            17,
            "every selection must persist all bounded transaction slices"
        );
        assert!(
            aggregates.iter().all(|aggregate_json| {
                serde_json::from_str::<serde_json::Value>(aggregate_json)
                    .ok()
                    .and_then(|aggregate| aggregate["total_count"].as_i64())
                    == Some(1)
            }),
            "every committed key slice must persist its terminal aggregate contents"
        );
    }

    for minute in 0..17_i64 {
        let occurred_at = format!("2026-08-21 12:{minute:02}:42");
        let record = api_invocation_from_runtime_record(&test_proxy_capture_record(
            &format!("timeseries-projection-incremental-{minute}"),
            &occurred_at,
        ));
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
            .expect("attach the account payload used by the incremental projection selection");
        let row_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .fetch_one(&state.pool)
        .await
        .expect("load durable incremental row id");
        let event_id = state
            .terminal_projection_hub
            .register_pending(&record, None)
            .expect("incremental terminal event should fit in the projection journal");
        state.terminal_projection_hub.acknowledge_persisted(
            Some(event_id),
            &record.invoke_id,
            &record.occurred_at,
            row_id,
        );
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
async fn exact_fallback_warm_write_defers_behind_p1_terminal_work() {
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
async fn exact_fallback_warm_rebuilds_after_non_proxy_terminal_invalidation() {
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
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, raw_response) VALUES (?1, ?2, 'cli', 'running', 10, 0.01, 12.0, '{}')",
    )
    .bind("non-proxy-warm-race")
    .bind(&occurred_at)
    .execute(&state.pool)
    .await
    .expect("insert non-proxy running invocation");

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_non_proxy_warm_seed",
        &coordinator,
    )
    .await
    .expect("seed warm projection");
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

    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_non_proxy_warm_rebuild",
        &coordinator,
    )
    .await
    .expect("defer rewarm until durable recovery invalidates existing coverage");
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
    let outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_non_proxy_warm_rebuild_after_recovery",
        &coordinator,
    )
    .await
    .expect("rebuild invalidated projection from transaction-time source rows");
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
async fn exact_fallback_warm_invalidates_direct_non_proxy_terminal_replacement() {
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

    let recovery = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
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
        &coordinator,
    )
    .await
    .expect("re-warm durable replacement coverage after P2 recovery");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
}

#[tokio::test]
async fn exact_fallback_warm_invalidates_non_proxy_stream_duration_replacement() {
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
async fn timeseries_startup_recovery_baseline_runs_once_and_clean_restart_skips_invalidation() {
    let pool = test_current_schema_pool().await;
    let pending: i64 = sqlx::query_scalar(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&pool)
    .await
    .expect("load baseline recovery marker");
    assert_eq!(
        pending, 1,
        "first schema upgrade must publish one baseline marker"
    );

    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&pool)
    .await
    .expect("complete baseline warming");
    crate::ensure_schema(&pool)
        .await
        .expect("clean restart schema refresh");
    let pending: i64 = sqlx::query_scalar(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&pool)
    .await
    .expect("load clean restart recovery marker");
    assert_eq!(
        pending, 0,
        "clean restart must not reopen a completed baseline"
    );
    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name = 'timeseries_minute_projection_startup_recovery_baseline_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load startup baseline migration marker");
    assert_eq!(marker_count, 1);
}

#[tokio::test]
async fn timeseries_startup_recovery_baseline_reenters_after_stopped_release() {
    let pool = test_current_schema_pool().await;
    sqlx::query(
        "DELETE FROM schema_refresh_migrations WHERE migration_name = 'timeseries_minute_projection_startup_recovery_baseline_v1'",
    )
    .execute(&pool)
    .await
    .expect("remove incomplete baseline marker");
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&pool)
    .await
    .expect("clear stopped baseline marker");
    crate::ensure_schema(&pool)
        .await
        .expect("forward repair baseline migration");
    let pending: i64 = sqlx::query_scalar(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&pool)
    .await
    .expect("load repaired recovery marker");
    assert_eq!(
        pending, 1,
        "stopped baseline must re-enter warming on next startup"
    );
}

#[tokio::test]
async fn timeseries_startup_recovery_empty_projection_invalidation_completes_without_rows() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query("DELETE FROM timeseries_minute_projection_v2")
        .execute(&state.pool)
        .await
        .expect("clear minute projection candidates");
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET generation = generation + 1, invalidation_pending = 1 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&state.pool)
    .await
    .expect("publish empty invalidation marker");

    let outcome = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &CancellationToken::new(),
    )
    .await
    .expect("empty coverage invalidation should complete");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );

    let pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load empty invalidation marker");
    assert_eq!(pending, 0);
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count empty minute projections");
    assert_eq!(projection_count, 0);
}

#[tokio::test]
async fn timeseries_startup_recovery_invalidates_multiple_projection_batches() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for minute_start_epoch in 0_i64..257 {
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state) VALUES (?1, 'all', -1, '{}', '[]', '[]', '[]', '[]', 0, 'ready')",
        )
        .bind(minute_start_epoch)
        .execute(&state.pool)
        .await
        .expect("seed multiple invalidation batches");
    }
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET generation = generation + 1, invalidation_pending = 1 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&state.pool)
    .await
    .expect("publish multi-batch invalidation marker");

    let outcome = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &CancellationToken::new(),
    )
    .await
    .expect("multi-batch coverage invalidation should complete");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    let ready_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining ready projection rows");
    assert_eq!(ready_count, 0);
    let warming_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'warming'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count invalidated projection rows");
    assert_eq!(warming_count, 257);
}

#[tokio::test]
async fn restart_schema_refreshes_existing_objects_across_primed_pool_connections() {
    let _target_timing = test_timing_target(
        "tests::stateful_sqlite::parallel_work_stats_and_timeseries::restart_schema_refreshes_existing_objects_across_primed_pool_connections",
    );
    let _whole_test = test_timing_span("whole_test");
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

    crate::test_timed!(
        "production_operation",
        crate::ensure_schema(&pool)
            .await
            .expect("restart schema initialization must refresh existing schema objects")
    );

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
async fn restart_schema_installs_durable_non_proxy_replacement_fence_before_projection_supervisor()
{
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
    let invoke_id = "timeseries-missing-durable-replacement-fence";
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, 'cli', 'success', 10, 0.01, ?3, '{}')",
    )
    .bind(invoke_id)
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
    .bind(invoke_id)
    .execute(&state.pool)
    .await
    .expect("replace the terminal row before the projection supervisor starts");
    let (status, failure_class): (String, String) =
        sqlx::query_as("SELECT status, failure_class FROM codex_invocations WHERE invoke_id = ?1")
            .bind(invoke_id)
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
        .expect("build HTTP timeseries before the projection supervisor starts");
        let payload = serde_json::to_value(response).expect("serialize HTTP timeseries response");
        let point = payload["points"]
            .as_array()
            .expect("HTTP timeseries points")
            .iter()
            .find(|point| point["totalCount"] == 1)
            .expect("replacement minute point");
        assert_eq!(
            point["successCount"], 0,
            "HTTP selection {upstream_account_id:?} must not reuse stale ready coverage"
        );
        assert_eq!(
            point["failureCount"], 1,
            "HTTP selection {upstream_account_id:?} must use the direct terminal replacement"
        );
    }

    let query = TimeseriesQuery {
        range: "1h".to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        time_zone: Some("UTC".to_string()),
        upstream_account_id: None,
    };
    let base = crate::api::TimeseriesTopicMaterializedBase::build(state.as_ref(), &query)
        .await
        .expect("build materialized timeseries base before the projection supervisor starts");
    let payload: serde_json::Value = serde_json::from_slice(
        &base
            .serialize(&[])
            .expect("serialize exact-fallback materialized response"),
    )
    .expect("materialized timeseries JSON");
    let point = payload["points"]
        .as_array()
        .expect("timeseries points")
        .iter()
        .find(|point| point["totalCount"] == 1)
        .expect("replacement minute point");
    assert_eq!(point["successCount"], 0);
    assert_eq!(point["failureCount"], 1);

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

#[tokio::test]
async fn exact_fallback_warm_acknowledges_the_transaction_snapshot_not_the_response_snapshot() {
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
async fn startup_minute_projection_recovery_stops_before_writes_when_cancelled() {
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

#[tokio::test]
async fn startup_minute_projection_recovery_keeps_the_durable_fence_when_cancelled_after_warming() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for minute_start_epoch in 0_i64..=1_024 {
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state) VALUES (?1, 'all', -1, '{}', '[]', '[]', '[]', '[]', 0, 'ready')",
        )
        .bind(minute_start_epoch)
        .execute(&state.pool)
        .await
        .expect("seed coverage invalidation batches");
    }
    crate::api::mark_timeseries_minute_projection_startup_recovery(&state.pool)
        .await
        .expect("publish durable recovery marker");

    let cancellation = CancellationToken::new();
    let recovery_state = state.clone();
    let recovery_cancellation = cancellation.clone();
    let recovery = tokio::spawn(async move {
        crate::api::prepare_timeseries_minute_projection_after_restart(
            recovery_state.as_ref(),
            &recovery_cancellation,
        )
        .await
    });
    let mut saw_warming = false;
    for _ in 0..1_000 {
        let recovery_state = sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_error FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
        )
        .fetch_optional(&state.pool)
        .await
        .expect("load startup recovery state")
        .flatten();
        if recovery_state.as_deref() == Some("warming") {
            cancellation.cancel();
            saw_warming = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        saw_warming,
        "cancellation must occur after recovery publishes warming and before it can report ready"
    );
    let outcome = recovery
        .await
        .expect("recovery task should not panic")
        .expect("cancelled recovery should not report a database error");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Cancelled
    );
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load retained recovery marker");
    assert_eq!(recovery_pending, 1);
    let recovery_state = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_error FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_optional(&state.pool)
    .await
    .expect("load cancelled recovery state")
    .flatten();
    assert_eq!(recovery_state.as_deref(), Some("warming"));
}

#[tokio::test]
async fn live_update_trigger_rebuild_fences_source_changing_terminal_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query("DROP TABLE timeseries_minute_projection_v2_recovery")
        .execute(&state.pool)
        .await
        .expect("model an interrupted startup with a missing recovery table");
    crate::schema::rebuild_invocation_in_progress_live_triggers(&state.pool)
        .await
        .expect("rebuild must restore the recovery table with its live update trigger");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) VALUES ('rebuild-recovery-table', datetime('now'), 'proxy', 'running', '{}')",
    )
    .execute(&state.pool)
    .await
    .expect("insert an in-flight proxy invocation");
    sqlx::query("UPDATE codex_invocations SET source = 'cli', status = 'success' WHERE invoke_id = 'rebuild-recovery-table'")
        .execute(&state.pool)
        .await
    .expect("source-changing terminal write must not target a missing recovery table");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("live update trigger publishes the restored recovery marker");
    assert_eq!(recovery_pending, 1);
}

#[tokio::test]
async fn materialized_timeseries_uses_exact_fallback_for_same_cursor_terminal_replacement() {
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
    let invoke_id = "timeseries-materializer-same-cursor-replacement";
    let persisted = persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("persist initial terminal row")
    .expect("initial terminal row should be stored");
    assert_eq!(persisted.status.as_deref(), Some("success"));

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let warm_start = now - ChronoDuration::minutes(62);
    let warm_end = now + ChronoDuration::minutes(2);
    let warm_outcome = crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
        &state.pool,
        warm_start,
        warm_end,
        InvocationSourceScope::All,
        None,
        state.terminal_projection_hub.as_ref(),
        "stateful_same_cursor_materializer_seed",
        &coordinator,
    )
    .await
    .expect("warm the complete materializer selection");
    assert_eq!(
        warm_outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );
    let account_warm_outcome =
        crate::api::store_timeseries_minute_projection_v2_warm_with_coordinator(
            &state.pool,
            warm_start,
            warm_end,
            InvocationSourceScope::All,
            Some(17),
            state.terminal_projection_hub.as_ref(),
            "stateful_same_cursor_account_materializer_seed",
            &coordinator,
        )
        .await
        .expect("warm the account materializer selection");
    assert_eq!(
        account_warm_outcome,
        crate::api::TimeseriesMinuteProjectionWarmOutcome::Stored
    );

    let minute_start = parse_to_utc_datetime(&occurred_at)
        .expect("parse persisted occurrence")
        .timestamp()
        .div_euclid(60)
        * 60;
    let (coverage_state, projection_cursor): (String, i64) = sqlx::query_as(
        "SELECT coverage_state, max_row_id FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(minute_start)
    .fetch_one(&state.pool)
    .await
    .expect("load warm minute projection");
    assert_eq!(coverage_state, "ready");
    assert_eq!(projection_cursor, persisted.id);

    sqlx::query(
        "UPDATE codex_invocations SET status = 'failed', failure_kind = 'upstream_response_failed', failure_class = 'service_failure', is_actionable = 1 WHERE id = ?1",
    )
    .bind(persisted.id)
    .execute(&state.pool)
    .await
    .expect("replace the terminal row without advancing its cursor");

    let mut replacement =
        api_invocation_from_runtime_record(&test_proxy_capture_record(invoke_id, &occurred_at));
    replacement.status = Some("failed".to_string());
    replacement.failure_kind = Some("upstream_response_failed".to_string());
    replacement.failure_class = Some("service_failure".to_string());
    replacement.is_actionable = Some(true);
    state
        .terminal_projection_hub
        .activate_timeseries_consumer(0);
    let event_id = state
        .terminal_projection_hub
        .register_pending(&replacement, None)
        .expect("replacement must fit in the terminal projection journal");
    state.terminal_projection_hub.acknowledge_persisted(
        Some(event_id),
        &replacement.invoke_id,
        &replacement.occurred_at,
        persisted.id,
    );

    for upstream_account_id in [None, Some(17)] {
        assert!(
            crate::api::timeseries_minute_projection_has_uncovered_terminal_delta(
                state.terminal_projection_hub.as_ref(),
                InvocationSourceScope::All,
                upstream_account_id,
                now - ChronoDuration::hours(1),
                now,
            ),
            "same-cursor terminal replacement must invalidate every matching HTTP selection"
        );
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
        .expect("build HTTP exact-fallback timeseries response");
        let payload = serde_json::to_value(response).expect("serialize HTTP timeseries response");
        let point = payload["points"]
            .as_array()
            .expect("HTTP timeseries points")
            .iter()
            .find(|point| point["totalCount"] == 1)
            .expect("replacement minute point");
        assert_eq!(
            point["successCount"], 0,
            "HTTP selection {upstream_account_id:?} must not retain the stale success"
        );
        assert_eq!(
            point["failureCount"], 1,
            "HTTP selection {upstream_account_id:?} must include the terminal replacement"
        );
    }

    let query = TimeseriesQuery {
        range: "1h".to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        time_zone: Some("UTC".to_string()),
        upstream_account_id: None,
    };
    let base = crate::api::TimeseriesTopicMaterializedBase::build(state.as_ref(), &query)
        .await
        .expect("build materialized timeseries base");
    let payload: serde_json::Value = serde_json::from_slice(
        &base
            .serialize(&[])
            .expect("serialize exact-fallback materialized response"),
    )
    .expect("materialized timeseries JSON");
    let point = payload["points"]
        .as_array()
        .expect("timeseries points")
        .iter()
        .find(|point| point["totalCount"] == 1)
        .expect("replacement minute point");
    assert_eq!(point["successCount"], 0);
    assert_eq!(point["failureCount"], 1);
}
