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
