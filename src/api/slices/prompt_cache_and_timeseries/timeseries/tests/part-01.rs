use super::*;

fn record(id: i64, occurred_at: &str) -> InvocationAggregateRecord {
    InvocationAggregateRecord {
        id,
        invoke_id: format!("invoke-{id}"),
        occurred_at: occurred_at.to_string(),
        status: Some("success".to_string()),
        total_tokens: Some(3),
        input_tokens: Some(2),
        output_tokens: Some(1),
        cache_input_tokens: Some(1),
        reasoning_tokens: Some(0),
        cost: Some(0.25),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: Some(0),
        live_phase: None,
        t_total_ms: Some(10.0),
        t_req_read_ms: Some(1.0),
        t_req_parse_ms: Some(1.0),
        t_upstream_connect_ms: Some(2.0),
        t_upstream_ttfb_ms: Some(3.0),
        first_token_ms: Some(4.0),
        t_upstream_stream_ms: Some(5.0),
        t_resp_parse_ms: Some(1.0),
        t_persist_ms: Some(1.0),
    }
}

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
    let pool = create_snapshot_fence_pool().await;
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

#[test]
fn contract_test_timeseries_topic_materializer_preserves_exact_p95() {
    let now = Utc::now();
    let start = now - ChronoDuration::seconds(10);
    let end = now + ChronoDuration::seconds(60);
    let occurred_at = format_naive(now.with_timezone(&Shanghai).naive_local());
    let mut base = TimeseriesTopicMaterializedBase {
        range_start: start,
        range_end: end,
        range_spec: "1m".to_string(),
        bucket_selection: TimeseriesBucketSelection {
            bucket_seconds: 60,
            effective_bucket: "1m".to_string(),
            available_buckets: vec!["1m".to_string()],
            bucket_limited_to_daily: false,
        },
        reporting_tz: Shanghai,
        source_scope: InvocationSourceScope::All,
        upstream_account_id: None,
        snapshot_id: 0,
        terminal_sequence: 0,
        aggregates: BTreeMap::new(),
        db_runtime_records: HashMap::new(),
    };
    let delta = |ttfb_ms, first_token_ms| TimeseriesTerminalDelta {
        occurred_at: occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        upstream_account_id: None,
        status: Some("success".to_string()),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        total_tokens: Some(3),
        input_tokens: Some(2),
        output_tokens: Some(1),
        cache_input_tokens: Some(1),
        reasoning_tokens: Some(0),
        cost: Some(0.25),
        t_total_ms: Some(ttfb_ms * 2.0),
        t_req_read_ms: Some(1.0),
        t_req_parse_ms: Some(2.0),
        t_upstream_connect_ms: Some(3.0),
        t_upstream_ttfb_ms: Some(ttfb_ms),
        first_token_ms: Some(first_token_ms),
    };
    base.apply_terminal_delta(1, Some(1), &delta(10.0, 10.0));
    base.apply_terminal_delta(2, Some(2), &delta(100.0, 100.0));

    let payload: serde_json::Value =
        serde_json::from_slice(&base.serialize(&[]).expect("serialize materialized topic"))
            .expect("materialized topic JSON");
    let point = payload["points"]
        .as_array()
        .expect("timeseries points")
        .iter()
        .find(|point| point["firstByteSampleCount"] == 2)
        .expect("materialized bucket");
    assert_eq!(point["firstByteP95Ms"], 95.5);
    assert_eq!(point["firstResponseByteTotalP95Ms"], 101.5);
    assert_eq!(point["firstTokenP95Ms"], 95.5);
    assert_eq!(payload["snapshotId"], 2);
}

#[test]
fn timeseries_topic_applies_terminal_replacement_at_snapshot_once() {
    let now = Utc::now();
    let occurred_at = format_naive(now.with_timezone(&Shanghai).naive_local());
    let mut base = TimeseriesTopicMaterializedBase {
        range_start: now - ChronoDuration::seconds(10),
        range_end: now + ChronoDuration::seconds(60),
        range_spec: "1m".to_string(),
        bucket_selection: TimeseriesBucketSelection {
            bucket_seconds: 60,
            effective_bucket: "1m".to_string(),
            available_buckets: vec!["1m".to_string()],
            bucket_limited_to_daily: false,
        },
        reporting_tz: Shanghai,
        source_scope: InvocationSourceScope::All,
        upstream_account_id: None,
        snapshot_id: 17,
        terminal_sequence: 0,
        aggregates: BTreeMap::new(),
        db_runtime_records: HashMap::new(),
    };
    let delta = TimeseriesTerminalDelta {
        occurred_at,
        source: SOURCE_PROXY.to_string(),
        upstream_account_id: None,
        status: Some("success".to_string()),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        total_tokens: Some(3),
        input_tokens: Some(2),
        output_tokens: Some(1),
        cache_input_tokens: Some(1),
        reasoning_tokens: Some(0),
        cost: Some(0.25),
        t_total_ms: Some(20.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: Some(10.0),
        first_token_ms: Some(11.0),
    };

    // A terminal write can replace a running row without changing its SQLite ID.
    base.apply_terminal_delta(1, Some(17), &delta);
    base.apply_terminal_delta(1, Some(17), &delta);

    assert_eq!(
        base.aggregates
            .values()
            .map(|aggregate| aggregate.total_count)
            .sum::<i64>(),
        1,
        "the sequence watermark must admit one terminal replacement and reject its replay",
    );
    assert_eq!(base.snapshot_id, 17);
}

#[test]
fn timeseries_topic_routes_hour_aligned_history_through_rollup_baseline() {
    let end = Utc::now();
    let range_window = RangeWindow {
        start: end - ChronoDuration::days(60),
        end,
        display_end: end,
        duration: ChronoDuration::days(60),
    };
    let params = TimeseriesQuery {
        range: "60d".to_string(),
        bucket: Some("1h".to_string()),
        settlement_hour: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };

    assert!(
        timeseries_topic_uses_hourly_rollup_baseline(&params, Shanghai, &range_window, 3_600, 7,)
            .expect("hour-aligned history should use rollups")
    );
    let account_params = TimeseriesQuery {
        range: "60d".to_string(),
        bucket: Some("1h".to_string()),
        settlement_hour: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: Some(42),
    };
    assert!(
        timeseries_topic_uses_hourly_rollup_baseline(
            &account_params,
            Shanghai,
            &range_window,
            3_600,
            7,
        )
        .expect("account-scoped hour-aligned history should use rollups")
    );
    let half_hour_tz = "Asia/Kathmandu".parse::<Tz>().expect("valid timezone");
    assert!(
        timeseries_topic_uses_hourly_rollup_baseline(
            &params,
            half_hour_tz,
            &range_window,
            3_600,
            7,
        )
        .is_err()
    );
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
    let pool = create_snapshot_fence_pool().await;
    let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
    let end = start + ChronoDuration::minutes(1);
    let initial_projection = seed_snapshot_fence_projection(&pool, start, end).await;
    let terminal_projection_hub = TerminalProjectionHub::default();
    invalidate_and_rewarm_snapshot_fence(
        &pool,
        start,
        end,
        &terminal_projection_hub,
        &initial_projection,
    )
    .await;
    let re_warmed_projection = load_snapshot_fence_projection(&pool, start, end).await;
    assert_recovery_marker_fence(
        &pool,
        start,
        end,
        &terminal_projection_hub,
        &re_warmed_projection,
    )
    .await;

    assert_terminal_replacement_fence(
        &pool,
        start,
        end,
        &terminal_projection_hub,
        &re_warmed_projection,
    )
    .await;
}

async fn create_snapshot_fence_pool() -> Pool<Sqlite> {
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
    pool
}

async fn seed_snapshot_fence_projection(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> TimeseriesMinuteProjectionV2Load {
    let old = record(17, "2026-08-01 08:00:12");
    let newer = record(18, "2026-08-01 08:00:30");
    store_timeseries_minute_projection_v2_for_test(
        pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[old.clone(), newer],
    )
    .await
    .expect("store newer snapshot");
    store_timeseries_minute_projection_v2_for_test(
        pool,
        start,
        end,
        InvocationSourceScope::All,
        None,
        &[old],
    )
    .await
    .expect("attempt stale warm snapshot");
    let projection =
        load_timeseries_minute_projection_v2(pool, start, end, InvocationSourceScope::All, None)
            .await
            .expect("load v2 projection")
            .expect("complete minute coverage");
    assert_eq!(projection.cursor, 18);
    assert_eq!(projection.aggregates[&start.timestamp()].total_count, 2);
    projection
}

async fn load_snapshot_fence_projection(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> TimeseriesMinuteProjectionV2Load {
    load_timeseries_minute_projection_v2(pool, start, end, InvocationSourceScope::All, None)
        .await
        .expect("load re-warmed projection")
        .expect("re-warmed minute coverage")
}

async fn invalidate_and_rewarm_snapshot_fence(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    hub: &TerminalProjectionHub,
    initial: &TimeseriesMinuteProjectionV2Load,
) {
    sqlx::query("UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming'")
        .execute(pool)
        .await
        .expect("invalidate stale projection coverage");
    assert!(
        load_timeseries_minute_projection_v2(pool, start, end, InvocationSourceScope::All, None)
            .await
            .expect("load invalidated projection")
            .is_none()
    );
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &initial.coverage_rows,
        )
        .await
        .expect("check invalidated coverage fence")
    );
    store_timeseries_minute_projection_v2_for_test(
        pool,
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
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &initial.coverage_rows,
        )
        .await
        .expect("check changed re-warmed coverage fence")
    );
}

async fn assert_recovery_marker_fence(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    hub: &TerminalProjectionHub,
    projection: &TimeseriesMinuteProjectionV2Load,
) {
    sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending) VALUES ('timeseries_minute_v2', 1, 1)",
        )
        .execute(pool)
        .await
        .expect("publish a direct terminal replacement recovery marker");
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &projection.coverage_rows,
        )
        .await
        .expect("check durable recovery snapshot fence")
    );
    sqlx::query(
            "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0 WHERE consumer = 'timeseries_minute_v2'",
        )
        .execute(pool)
        .await
        .expect("clear recovered terminal replacement marker");
    assert!(
        timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &projection.coverage_rows,
        )
        .await
        .expect("check re-warmed coverage fence")
    );
}

async fn assert_terminal_replacement_fence(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    hub: &TerminalProjectionHub,
    projection: &TimeseriesMinuteProjectionV2Load,
) {
    let replacement_terminal =
        crate::proxy::api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
            "snapshot-fence-terminal-replacement",
            "2026-08-01 08:00:12",
        ));
    let replacement_event_id = hub
        .register_pending(&replacement_terminal, None)
        .expect("terminal replacement should fit in the projection journal");
    hub.acknowledge_persisted(
        Some(replacement_event_id),
        &replacement_terminal.invoke_id,
        &replacement_terminal.occurred_at,
        18,
    );
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &projection.coverage_rows,
        )
        .await
        .expect("check successful replacement snapshot fence")
    );
    hub.mark_timeseries_deltas_flushed(&[replacement_event_id]);
    assert!(
        timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &projection.coverage_rows,
        )
        .await
        .expect("check flushed replacement snapshot fence")
    );
    assert_rejected_terminal_fence(pool, start, end, hub, projection).await;
}

async fn assert_rejected_terminal_fence(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    hub: &TerminalProjectionHub,
    projection: &TimeseriesMinuteProjectionV2Load,
) {
    let rejected_terminal =
        crate::proxy::api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
            "snapshot-fence-rejected-terminal",
            "2026-08-01 08:00:12",
        ));
    for _ in 0..=crate::terminal_projection::TERMINAL_PROJECTION_MAX_PENDING_EVENTS {
        if hub.register_pending(&rejected_terminal, None).is_none() {
            break;
        }
    }
    assert!(hub.timeseries_coverage_invalidation_pending().is_some());
    assert!(
        !timeseries_minute_projection_v2_snapshot_is_current(
            pool,
            hub,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &projection.coverage_rows,
        )
        .await
        .expect("check in-memory invalidation snapshot fence")
    );
}

#[test]
fn minute_projection_folds_exact_samples_into_the_requested_reporting_bucket() {
    let mut minute_aggregates = BTreeMap::new();
    for minute in [0_i64, 60, 120, 180, 240] {
        let mut aggregate = BucketAggregate {
            total_count: 1,
            first_byte_sample_count: 1,
            ..Default::default()
        };
        aggregate.first_byte_ttfb_values.push(minute as f64);
        minute_aggregates.insert(minute, aggregate);
    }

    let folded =
        fold_minute_projection_aggregates(minute_aggregates, 300, chrono_tz::Asia::Shanghai)
            .expect("fold minute aggregates");
    assert_eq!(folded.len(), 1);
    let aggregate = folded.get(&0).expect("five-minute bucket");
    assert_eq!(aggregate.total_count, 5);
    assert_eq!(
        aggregate.first_byte_ttfb_values,
        vec![0.0, 60.0, 120.0, 180.0, 240.0]
    );
}
