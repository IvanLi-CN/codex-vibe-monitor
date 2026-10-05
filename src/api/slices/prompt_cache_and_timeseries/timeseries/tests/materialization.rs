use super::*;

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
