use super::*;

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

#[test]
fn timeseries_point_clears_latency_when_bucket_has_no_calls() {
    let mut aggregate = BucketAggregate {
        first_byte_sample_count: 1,
        first_byte_ttfb_sum_ms: 750.0,
        first_response_byte_total_sample_count: 1,
        first_response_byte_total_sum_ms: 18_225.02,
        total_latency_sample_count: 1,
        total_latency_sum_ms: 24_000.0,
        ..Default::default()
    };
    aggregate.first_byte_ttfb_values.push(750.0);
    aggregate.first_response_byte_total_values.push(18_225.02);

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.total_count, 0);
    assert_eq!(point.first_byte_sample_count, 0);
    assert!(point.first_byte_avg_ms.is_none());
    assert!(point.first_byte_p95_ms.is_none());
    assert_eq!(point.first_response_byte_total_sample_count, 0);
    assert!(point.first_response_byte_total_avg_ms.is_none());
    assert!(point.first_response_byte_total_p95_ms.is_none());
    assert!(point.avg_total_ms.is_none());
}

#[test]
fn timeseries_point_keeps_rollup_backed_total_latency_average() {
    let aggregate = BucketAggregate {
        total_count: 4,
        success_count: 3,
        failure_count: 1,
        total_latency_sample_count: 2,
        total_latency_sum_ms: 1_800.0,
        ..Default::default()
    };

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.total_latency_sample_count, 2);
    assert_eq!(point.avg_total_ms, Some(900.0));
}

#[test]
fn timeseries_point_exports_in_flight_phase_counts_and_compat_total() {
    let mut aggregate = BucketAggregate {
        total_count: 5,
        success_count: 1,
        failure_count: 1,
        in_flight_count: 3,
        ..Default::default()
    };
    aggregate.in_flight_phase_counts.queued = 1;
    aggregate.in_flight_phase_counts.requesting = 1;
    aggregate.in_flight_phase_counts.responding = 1;

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.in_flight_count, 3);
    assert_eq!(point.in_flight_phase_counts.queued, 1);
    assert_eq!(point.in_flight_phase_counts.requesting, 1);
    assert_eq!(point.in_flight_phase_counts.responding, 1);
}

#[test]
fn timeseries_point_hides_incomplete_token_components() {
    let aggregate = BucketAggregate {
        total_tokens: 30,
        input_tokens: 20,
        output_tokens: 10,
        cache_input_tokens: 5,
        token_components_observed: true,
        token_component_incomplete_count: 1,
        ..Default::default()
    };

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.total_tokens, 30);
    assert_eq!(point.input_tokens, None);
    assert_eq!(point.output_tokens, None);
    assert_eq!(point.cache_input_tokens, None);
    assert_eq!(point.reasoning_tokens, None);
}

#[test]
fn timeseries_point_hides_zero_filled_legacy_token_components() {
    let aggregate = BucketAggregate {
        total_tokens: 30,
        token_components_observed: true,
        ..Default::default()
    };

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.total_tokens, 30);
    assert_eq!(point.input_tokens, None);
    assert_eq!(point.output_tokens, None);
    assert_eq!(point.cache_input_tokens, None);
    assert_eq!(point.reasoning_tokens, None);
}

#[test]
fn timeseries_point_hides_cache_input_when_it_exceeds_input_tokens() {
    let aggregate = BucketAggregate {
        total_tokens: 10,
        input_tokens: 0,
        output_tokens: 10,
        cache_input_tokens: 2,
        token_components_observed: true,
        ..Default::default()
    };

    let point = timeseries_point_from_aggregate(
        Utc.timestamp_opt(1_775_608_200, 0)
            .single()
            .expect("valid start timestamp"),
        Utc.timestamp_opt(1_775_608_260, 0)
            .single()
            .expect("valid end timestamp"),
        &aggregate,
    );

    assert_eq!(point.total_tokens, 10);
    assert_eq!(point.input_tokens, None);
    assert_eq!(point.cache_input_tokens, None);
}

#[test]
fn add_rollup_rows_preserves_total_latency_metrics() {
    let bucket_epoch =
        align_reporting_bucket_epoch(1_775_608_200, 3_600, chrono_tz::Asia::Shanghai)
            .expect("aligned bucket");
    let mut aggregates = BTreeMap::from([(bucket_epoch, BucketAggregate::default())]);
    add_rollup_rows_to_timeseries_aggregates(
        &mut aggregates,
        vec![UpstreamAccountStatsRollupRecord {
            bucket_start_epoch: 1_775_608_200,
            total_count: 3,
            success_count: 2,
            failure_count: 1,
            in_flight_count: 0,
            total_tokens: 99,
            input_tokens: 44,
            output_tokens: 55,
            cache_input_tokens: 7,
            reasoning_tokens: 11,
            total_cost: 1.25,
            non_success_cost: 0.3,
            total_latency_sample_count: 2,
            total_latency_sum_ms: 1_100.0,
            first_byte_sample_count: 1,
            first_byte_sum_ms: 450.0,
            first_byte_max_ms: 450.0,
            first_byte_histogram: encode_approx_histogram(&[0, 1]).expect("histogram"),
            first_response_byte_total_sample_count: 1,
            first_response_byte_total_sum_ms: 900.0,
            first_response_byte_total_max_ms: 900.0,
            first_response_byte_total_histogram: encode_approx_histogram(&[0, 1])
                .expect("histogram"),
            first_token_sample_count: 0,
            first_token_sum_ms: 0.0,
            first_token_max_ms: 0.0,
            first_token_histogram: encode_approx_histogram(&[]).expect("histogram"),
        }],
        3_600,
        chrono_tz::Asia::Shanghai,
    )
    .expect("rollup rows should aggregate");

    let aggregate = aggregates.get(&bucket_epoch).expect("bucket");
    assert_eq!(aggregate.total_count, 3);
    assert_eq!(aggregate.total_tokens, 99);
    assert_eq!(aggregate.input_tokens, 44);
    assert_eq!(aggregate.output_tokens, 55);
    assert_eq!(aggregate.cache_input_tokens, 7);
    assert_eq!(aggregate.reasoning_tokens, 11);
    assert_eq!(aggregate.total_latency_sample_count, 2);
    assert_eq!(aggregate.total_latency_sum_ms, 1_100.0);
}

#[test]
fn exact_records_survive_bucket_fill_before_aggregation() {
    let start = Utc
        .with_ymd_and_hms(2026, 10, 30, 0, 0, 0)
        .single()
        .expect("valid timeseries start");
    let end = start + ChronoDuration::minutes(1);
    let mut aggregates = BTreeMap::new();
    super::aggregation::fill_timeseries_buckets(
        &mut aggregates,
        start,
        end,
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect("fill timeseries bucket");
    super::aggregation::add_exact_records_to_timeseries_aggregates(
        &mut aggregates,
        vec![record(101, "2026-10-30T00:00:12Z")],
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect("add exact record");

    let aggregate = aggregates.values().next().expect("filled bucket");
    assert_eq!(aggregate.total_count, 1);
    assert_eq!(aggregate.success_count, 1);
}

#[test]
fn exact_records_fail_closed_on_numeric_overflow() {
    let mut cost_overflow = BucketAggregate {
        total_cost: f64::MAX,
        ..Default::default()
    };
    let mut cost_record = record(102, "2026-10-30T00:00:12Z");
    cost_record.cost = Some(f64::MAX);
    let cost_error = super::aggregation::add_exact_record_to_timeseries_aggregate(
        &mut cost_overflow,
        &cost_record,
    )
    .expect_err("cost overflow must make the projection unavailable");
    assert!(matches!(cost_error, crate::stats::ApiError::Unavailable(_)));

    let mut token_overflow = BucketAggregate {
        total_tokens: i64::MAX,
        ..Default::default()
    };
    let mut token_record = record(103, "2026-10-30T00:00:12Z");
    token_record.total_tokens = Some(1);
    let token_error = super::aggregation::add_exact_record_to_timeseries_aggregate(
        &mut token_overflow,
        &token_record,
    )
    .expect_err("token overflow must make the projection unavailable");
    assert!(matches!(
        token_error,
        crate::stats::ApiError::Unavailable(_)
    ));
}

#[test]
fn exact_records_reject_invalid_numeric_facts_before_mutating_aggregates() {
    type InvalidFactCase = (&'static str, fn(&mut InvocationAggregateRecord));
    let cases: &[InvalidFactCase] = &[
        (
            "negative tokens",
            |record: &mut InvocationAggregateRecord| {
                record.total_tokens = Some(-1);
            },
        ),
        (
            "cache exceeds input",
            |record: &mut InvocationAggregateRecord| {
                record.cache_input_tokens = Some(3);
            },
        ),
        (
            "total tokens disagree with components",
            |record: &mut InvocationAggregateRecord| {
                record.total_tokens = Some(3);
                record.input_tokens = Some(1);
                record.output_tokens = Some(1);
            },
        ),
        ("negative cost", |record: &mut InvocationAggregateRecord| {
            record.cost = Some(-0.1);
        }),
        (
            "non-finite cost",
            |record: &mut InvocationAggregateRecord| {
                record.cost = Some(f64::NAN);
            },
        ),
    ];
    for (label, mutate) in cases {
        let mut aggregate = BucketAggregate::default();
        let mut record = record(104, "2026-10-30T00:00:12Z");
        mutate(&mut record);
        let error =
            super::aggregation::add_exact_record_to_timeseries_aggregate(&mut aggregate, &record)
                .expect_err(label);
        assert!(matches!(error, crate::stats::ApiError::Unavailable(_)));
        assert_eq!(
            aggregate.total_count, 0,
            "{label} must not partially mutate"
        );
    }
}

#[test]
fn terminal_deltas_and_rollups_fail_closed_without_partial_mutation() {
    let mut delta =
        TimeseriesTerminalDelta::from(&crate::api::summary_projection_test_invocation());
    delta.total_tokens = Some(-1);
    let mut delta_aggregate = BucketAggregate {
        total_count: 4,
        ..Default::default()
    };
    let delta_before = delta_aggregate.clone();
    let delta_error = super::aggregation::add_timeseries_terminal_delta_to_aggregate(
        &mut delta_aggregate,
        &delta,
    )
    .expect_err("invalid terminal delta must fail closed");
    assert!(matches!(
        delta_error,
        crate::stats::ApiError::Unavailable(_)
    ));
    assert_eq!(format!("{delta_aggregate:?}"), format!("{delta_before:?}"));

    let mut rollup_target = BucketAggregate {
        total_count: 3,
        ..Default::default()
    };
    let rollup_before = rollup_target.clone();
    let rollup_error = super::aggregation::merge_timeseries_bucket_aggregate(
        &mut rollup_target,
        BucketAggregate {
            total_count: 1,
            input_tokens: 1,
            cache_input_tokens: 2,
            total_cost: f64::NAN,
            ..Default::default()
        },
    )
    .expect_err("invalid rollup must fail closed");
    assert!(matches!(
        rollup_error,
        crate::stats::ApiError::Unavailable(_)
    ));
    assert_eq!(format!("{rollup_target:?}"), format!("{rollup_before:?}"));
}

#[test]
fn exact_record_batches_commit_only_after_every_record_is_valid() {
    let valid = record(105, "2026-10-30T00:00:12Z");
    let mut invalid = record(106, "2026-10-30T00:00:12Z");
    invalid.total_tokens = Some(-1);
    let bucket_epoch = align_reporting_bucket_epoch(
        parse_to_utc_datetime(&valid.occurred_at)
            .expect("valid record timestamp")
            .timestamp(),
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect("aligned record bucket");
    let original = BucketAggregate {
        total_count: 7,
        ..Default::default()
    };
    let mut aggregates = BTreeMap::from([(bucket_epoch, original.clone())]);

    let error = super::aggregation::add_exact_records_to_timeseries_aggregates(
        &mut aggregates,
        vec![valid, invalid],
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect_err("an invalid record must reject the whole batch");
    assert!(matches!(error, crate::stats::ApiError::Unavailable(_)));
    assert_eq!(
        format!("{:?}", aggregates.get(&bucket_epoch)),
        format!("{:?}", Some(&original))
    );
}

#[test]
fn runtime_overlay_rejects_inconsistent_tokens_and_invalid_timing() {
    let start = Utc
        .with_ymd_and_hms(2026, 10, 30, 0, 0, 0)
        .single()
        .expect("valid timeseries start");
    let end = start + ChronoDuration::minutes(1);

    let mut inconsistent = crate::api::summary_projection_test_invocation();
    inconsistent.status = Some("pending".to_string());
    inconsistent.occurred_at = "2026-10-30T00:00:12Z".to_string();
    inconsistent.input_tokens = Some(1);
    inconsistent.output_tokens = Some(0);
    inconsistent.total_tokens = Some(1);
    inconsistent.cache_input_tokens = Some(2);
    let bucket_epoch =
        align_reporting_bucket_epoch(start.timestamp(), 60, chrono_tz::Asia::Shanghai)
            .expect("aligned runtime bucket");
    let original_aggregate = BucketAggregate {
        total_count: 7,
        ..Default::default()
    };
    let mut aggregates = BTreeMap::from([(bucket_epoch, original_aggregate.clone())]);
    let token_error = super::aggregation::overlay_runtime_timeseries_snapshot(
        &mut aggregates,
        &[inconsistent],
        InvocationSourceScope::All,
        None,
        start,
        end,
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect_err("inconsistent runtime token components must fail closed");
    assert!(matches!(
        token_error,
        crate::stats::ApiError::Unavailable(_)
    ));
    assert_eq!(
        format!("{:?}", aggregates.get(&bucket_epoch)),
        format!("{:?}", Some(&original_aggregate))
    );

    let mut invalid_timing = crate::api::summary_projection_test_invocation();
    invalid_timing.status = Some("pending".to_string());
    invalid_timing.occurred_at = "2026-10-30T00:00:12Z".to_string();
    invalid_timing.t_upstream_ttfb_ms = Some(f64::NAN);
    let timing_error = super::aggregation::overlay_runtime_timeseries_snapshot(
        &mut BTreeMap::new(),
        &[invalid_timing],
        InvocationSourceScope::All,
        None,
        start,
        end,
        60,
        chrono_tz::Asia::Shanghai,
    )
    .expect_err("non-finite runtime timing must fail closed");
    assert!(matches!(
        timing_error,
        crate::stats::ApiError::Unavailable(_)
    ));
}
