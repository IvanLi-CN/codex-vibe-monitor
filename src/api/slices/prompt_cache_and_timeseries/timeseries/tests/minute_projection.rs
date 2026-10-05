use super::*;

#[test]
fn minute_projection_rebuilds_when_a_flush_batch_repeats_a_row_id() {
    let delta = TimeseriesTerminalDelta {
        occurred_at: "2026-08-01T00:00:12Z".to_string(),
        source: SOURCE_PROXY.to_string(),
        upstream_account_id: None,
        status: Some("success".to_string()),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        total_tokens: Some(3),
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reasoning_tokens: None,
        cost: Some(0.25),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
    };

    assert!(timeseries_projection_requires_exact_rebuild(
        &[(42, delta.clone()), (42, delta)],
        41,
    ));
}

#[test]
fn minute_projection_incrementally_merges_distinct_new_row_ids() {
    let delta = TimeseriesTerminalDelta {
        occurred_at: "2026-08-01T00:00:12Z".to_string(),
        source: SOURCE_PROXY.to_string(),
        upstream_account_id: None,
        status: Some("success".to_string()),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        total_tokens: Some(3),
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reasoning_tokens: None,
        cost: Some(0.25),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
    };

    assert!(!timeseries_projection_requires_exact_rebuild(
        &[(42, delta.clone()), (43, delta.clone())],
        41,
    ));
    assert!(timeseries_projection_requires_exact_rebuild(
        &[(41, delta)],
        41,
    ));
}

#[tokio::test]
async fn minute_projection_returns_exact_records_and_cursor_for_covered_minutes() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_records (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, records_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create projection table");
    let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
    let end = Utc.with_ymd_and_hms(2026, 8, 1, 0, 3, 0).single().unwrap();
    store_timeseries_minute_projection_records(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[record(17, "2026-08-01 08:01:12")],
    )
    .await
    .expect("store projection");

    let (records, cursor) = load_timeseries_minute_projection_records(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
    )
    .await
    .expect("load projection")
    .expect("complete minute coverage");
    assert_eq!(cursor, 17);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].invoke_id, "invoke-17");
}

#[tokio::test]
async fn minute_projection_v2_preserves_exact_latency_samples_without_raw_records() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_recovery (consumer TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0, invalidation_pending INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection recovery table");
    sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
    let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
    let end = Utc.with_ymd_and_hms(2026, 8, 1, 0, 2, 0).single().unwrap();
    let mut first = record(17, "2026-08-01 08:00:12");
    first.t_total_ms = Some(20.0);
    first.t_upstream_ttfb_ms = Some(10.0);
    first.first_token_ms = Some(11.0);
    let mut second = record(18, "2026-08-01 08:00:30");
    second.t_total_ms = Some(200.0);
    second.t_upstream_ttfb_ms = Some(100.0);
    second.first_token_ms = Some(101.0);
    let mut in_flight = record(19, "2026-08-01 08:00:40");
    in_flight.status = Some("running".to_string());
    store_timeseries_minute_projection_v2_for_test(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[first, second, in_flight],
    )
    .await
    .expect("store v2 projection");

    let projection =
        load_timeseries_minute_projection_v2(&pool, start, end, InvocationSourceScope::All, None)
            .await
            .expect("load v2 projection")
            .expect("complete v2 minute coverage");
    let aggregates = projection.aggregates;
    let cursor = projection.cursor;
    let minute = start.timestamp();
    let aggregate = aggregates.get(&minute).expect("first minute aggregate");
    assert_eq!(cursor, 18);
    assert_eq!(aggregate.total_count, 2);
    let stored_total_latency_samples = sqlx::query_scalar::<_, String>(
            "SELECT total_latency_samples_json FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
        )
        .bind(minute)
        .fetch_one(&pool)
        .await
        .expect("stored total-latency samples");
    assert_eq!(
        serde_json::from_str::<Vec<f64>>(&stored_total_latency_samples)
            .expect("total-latency sample JSON"),
        vec![20.0, 200.0]
    );
    assert_eq!(aggregate.total_latency_values, vec![20.0, 200.0]);
    assert_eq!(aggregate.first_byte_ttfb_values, vec![10.0, 100.0]);
    assert_eq!(aggregate.first_token_values, vec![11.0, 101.0]);
    assert_eq!(aggregate.first_byte_p95_ms(), Some(95.5));
}
#[tokio::test]
async fn minute_projection_v2_warms_account_scope_with_empty_minutes() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_recovery (consumer TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0, invalidation_pending INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection recovery table");
    sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
    let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
    let end = start + ChronoDuration::minutes(2);

    store_timeseries_minute_projection_v2_for_test(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        Some(42),
        &[record(17, "2026-08-01 08:00:12")],
    )
    .await
    .expect("warm account projection");

    let projection = load_timeseries_minute_projection_v2(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        Some(42),
    )
    .await
    .expect("load account projection")
    .expect("complete account minute coverage");
    let aggregates = projection.aggregates;
    let cursor = projection.cursor;
    assert_eq!(cursor, 17);
    assert_eq!(aggregates.len(), 2);
    assert_eq!(
        aggregates[&(start + ChronoDuration::minutes(1)).timestamp()].total_count,
        0
    );
}

#[tokio::test]
async fn minute_projection_snapshot_fence_rejects_changed_rewarm_with_same_cursor() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
    sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_recovery (consumer TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0, invalidation_pending INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection recovery table");
    sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
    let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
    let end = start + ChronoDuration::minutes(1);
    let old = record(17, "2026-08-01 08:00:12");
    let newer = record(18, "2026-08-01 08:00:30");

    store_timeseries_minute_projection_v2_for_test(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[old.clone(), newer],
    )
    .await
    .expect("store newer snapshot");
    store_timeseries_minute_projection_v2_for_test(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[old],
    )
    .await
    .expect("attempt stale warm snapshot");

    let initial_projection =
        load_timeseries_minute_projection_v2(&pool, start, end, InvocationSourceScope::All, None)
            .await
            .expect("load v2 projection")
            .expect("complete minute coverage");
    assert_eq!(initial_projection.cursor, 18);
    assert_eq!(
        initial_projection.aggregates[&start.timestamp()].total_count,
        2
    );

    sqlx::query("UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming'")
        .execute(&pool)
        .await
        .expect("invalidate stale projection coverage");
    assert!(
        load_timeseries_minute_projection_v2(&pool, start, end, InvocationSourceScope::All, None,)
            .await
            .expect("load invalidated projection")
            .is_none()
    );
    let terminal_projection_hub = TerminalProjectionHub::default();
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &initial_projection.snapshot_fence,
        )
        .await
        .expect("check invalidated coverage fence"),
        "a coverage change after the projection read must force an exact fallback"
    );
    store_timeseries_minute_projection_v2_for_test(
        &pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[
            record(17, "2026-08-01 08:00:12"),
            InvocationAggregateRecord {
                status: Some("failed".to_string()),
                ..record(18, "2026-08-01 08:00:30")
            },
        ],
    )
    .await
    .expect("re-warm invalidated projection");
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &initial_projection.snapshot_fence,
        )
        .await
        .expect("check changed re-warmed coverage fence"),
        "re-warming the same primary keys must not accept stale aggregate payloads"
    );
    let re_warmed_projection =
        load_timeseries_minute_projection_v2(&pool, start, end, InvocationSourceScope::All, None)
            .await
            .expect("load re-warmed projection")
            .expect("re-warmed minute coverage");
    assert_eq!(re_warmed_projection.cursor, 18);
    assert_eq!(
        re_warmed_projection.aggregates[&start.timestamp()].total_count,
        2
    );
    sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending) VALUES ('timeseries_minute_v2', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("publish a direct terminal replacement recovery marker");
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &re_warmed_projection.snapshot_fence,
        )
        .await
        .expect("check durable recovery snapshot fence"),
        "a direct replacement marker must force an exact fallback even when projection rows are unchanged"
    );
    sqlx::query(
            "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0 WHERE consumer = 'timeseries_minute_v2'",
        )
        .execute(&pool)
        .await
        .expect("clear recovered terminal replacement marker");
    assert!(
        timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &re_warmed_projection.snapshot_fence,
        )
        .await
        .expect("check re-warmed coverage fence")
    );

    let replacement_terminal =
        crate::proxy::api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
            "snapshot-fence-terminal-replacement",
            "2026-08-01 08:00:12",
        ));
    let replacement_event_id = terminal_projection_hub
        .register_pending(&replacement_terminal, None)
        .expect("terminal replacement should fit in the projection journal");
    terminal_projection_hub.acknowledge_persisted(
        Some(replacement_event_id),
        &replacement_terminal.invoke_id,
        &replacement_terminal.occurred_at,
        18,
    );
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &re_warmed_projection.snapshot_fence,
        )
        .await
        .expect("check successful replacement snapshot fence"),
        "a same-cursor terminal replacement registered after projection loading must force an exact fallback"
    );
    terminal_projection_hub.mark_timeseries_deltas_flushed(&[replacement_event_id]);
    assert!(
        timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &re_warmed_projection.snapshot_fence,
        )
        .await
        .expect("check flushed replacement snapshot fence")
    );

    let rejected_terminal =
        crate::proxy::api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
            "snapshot-fence-rejected-terminal",
            "2026-08-01 08:00:12",
        ));
    for _ in 0..=crate::terminal_projection::TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
        if terminal_projection_hub
            .register_pending(&rejected_terminal, None)
            .is_none()
        {
            break;
        }
    }
    assert!(
        terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_some(),
        "a rejected terminal delta must keep the loaded projection snapshot fenced"
    );
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            &pool,
            &terminal_projection_hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &re_warmed_projection.snapshot_fence,
        )
        .await
        .expect("check in-memory invalidation snapshot fence"),
        "a hub invalidation after projection loading must force an exact fallback"
    );
}
#[tokio::test]
async fn pressure_cooldown_deferral_does_not_wake_its_own_retry_waiter() {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
    pressure_gate.record_pressure("timeseries_minute_projection_test", "sqlite_lock");
    let observed_eligibility_generation = pressure_gate.eligibility_generation();

    let outcome = try_acquire_timeseries_minute_projection_write(
        &coordinator,
        &pressure_gate,
        "test",
        "pressure_cooldown",
        1,
    )
    .await;

    let TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) = outcome else {
        panic!("pressure cooldown must defer minute projection work");
    };
    assert!(
        deferred
            .retry_after
            .is_some_and(|retry_after| retry_after >= Duration::from_secs(29)),
        "pressure cooldown must provide the retry deadline"
    );
    assert_eq!(
        pressure_gate.eligibility_generation(),
        observed_eligibility_generation,
        "releasing a denied P2 permit must not wake its own retry waiter"
    );
}

#[tokio::test]
async fn projection_sqlite_pressure_error_enters_cooldown_with_a_retry_deadline() {
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
    let error = ApiError::Internal(anyhow!("database is locked"));

    let deferred = timeseries_minute_projection_pressure_deferred(
        &pressure_gate,
        "timeseries_minute_projection_test",
        &error,
    )
    .expect("SQLite lock errors must defer projection writes through the pressure gate");

    assert!(
        deferred
            .retry_after
            .is_some_and(|retry_after| retry_after >= Duration::from_secs(29)),
        "pressure deferral must retain the gate cooldown deadline"
    );
    assert_eq!(pressure_gate.snapshot().pressure_events, 1);
    let observed_eligibility_generation = pressure_gate.eligibility_generation();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(5),
            pressure_gate.wait_for_eligibility_change(observed_eligibility_generation),
        )
        .await
        .is_err(),
        "retry wait must observe the post-error generation instead of self-waking"
    );
}
