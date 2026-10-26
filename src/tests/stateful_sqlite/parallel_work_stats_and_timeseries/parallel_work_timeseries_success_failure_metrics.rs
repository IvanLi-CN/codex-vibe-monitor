use super::*;

#[tokio::test]
async fn timeseries_includes_first_byte_avg_and_p95_for_success_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-1",
        &occurred_at,
        "success",
        Some(100.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-2",
        &occurred_at,
        "success",
        Some(200.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-3",
        &occurred_at,
        "success",
        Some(400.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-failure",
        &occurred_at,
        "failed",
        Some(800.0),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET input_tokens = 5,
            output_tokens = 5,
            cache_input_tokens = 0,
            reasoning_tokens = 0
        WHERE invoke_id LIKE 'ttfb-sample-%'
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed reconciled token components");
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET input_tokens = 30,
            output_tokens = 20,
            cache_input_tokens = 10,
            reasoning_tokens = 5,
            total_tokens = 50
        WHERE invoke_id = 'ttfb-sample-1'
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed token components");

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 3);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        (100.0 + 200.0 + 400.0) / 3.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        380.0,
    );
    assert_eq!(bucket.input_tokens, Some(45));
    assert_eq!(bucket.output_tokens, Some(35));
    assert_eq!(bucket.cache_input_tokens, Some(10));
    assert_eq!(bucket.reasoning_tokens, Some(5));
    assert_eq!(
        bucket.input_tokens.unwrap_or_default() + bucket.output_tokens.unwrap_or_default(),
        bucket.total_tokens
    );
}

#[tokio::test]
async fn timeseries_includes_legacy_http_200_success_like_ttfb_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-legacy-http-200",
        &occurred_at,
        "http_200",
        Some(250.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries for legacy http_200");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 1)
        .expect("should include populated bucket");

    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.first_byte_sample_count, 1);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        250.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        250.0,
    );
}

#[tokio::test]
async fn timeseries_and_summary_count_completed_rows_as_success() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "timeseries-success-completed-control",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-completed-success-like",
        &occurred_at,
        "completed",
        Some(120.0),
    )
    .await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch summary for completed success-like row");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries for completed success-like row");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 2);
    assert_eq!(bucket.success_count, 2);
    assert_eq!(bucket.failure_count, 0);
    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        100.0,
    );
}

#[tokio::test]
async fn failure_summary_excludes_completed_success_like_rows_from_recent_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "failure-summary-completed-success-like",
        &occurred_at,
        "completed",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "failure-summary-failed-control",
        &occurred_at,
        "failed",
        Some(120.0),
    )
    .await;

    let Json(summary) = fetch_failure_summary(
        State(state),
        Query(FailureSummaryQuery {
            range: "1h".to_string(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary for completed success-like row");

    assert_eq!(summary.total_failures, 1);
    assert_eq!(summary.service_failure_count, 1);
    assert_eq!(summary.client_failure_count, 0);
    assert_eq!(summary.client_abort_count, 0);
    assert_eq!(summary.actionable_failure_count, 1);
    assert_f64_close(summary.actionable_failure_rate, 1.0);
}

#[tokio::test]
async fn timeseries_reports_snapshot_id_for_live_exact_queries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    for (id, invoke_id, status) in [
        (101_i64, "snapshot-row-1", "success"),
        (105_i64, "snapshot-row-2", "failed"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert snapshot timeseries invocation");
    }

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");

    assert_eq!(response.snapshot_id, 105);
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");
    assert_eq!(bucket.total_count, 2);
}

#[tokio::test]
async fn timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "timeseries-success",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-running",
        &occurred_at,
        "running",
        Some(120.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-pending",
        &occurred_at,
        "pending",
        Some(160.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-failed",
        &occurred_at,
        "failed",
        Some(240.0),
    )
    .await;
    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("upstream_response_failed")
    .bind("service_failure")
    .bind("[upstream_response_failed] upstream response stream reported failure")
    .bind("timeseries-running")
    .execute(&state.pool)
    .await
    .expect("annotate running row with failure metadata");
    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("downstream_closed")
    .bind("client_abort")
    .bind("[downstream_closed] downstream closed while streaming upstream response")
    .bind("timeseries-pending")
    .execute(&state.pool)
    .await
    .expect("annotate pending row with failure metadata");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch summary for mixed running statuses");
    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries for mixed running statuses");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 4);
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
    assert_eq!(bucket.in_flight_count, 2);
}

#[tokio::test]
async fn timeseries_and_summary_count_http_200_rows_with_downstream_only_failure_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "timeseries-success-downstream-control",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-http-200-downstream-only",
        &occurred_at,
        "http_200",
        Some(120.0),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET payload = ?1 WHERE invoke_id = ?2")
        .bind(
            json!({
                "downstreamErrorMessage": "socket closed after response"
            })
            .to_string(),
        )
        .bind("timeseries-http-200-downstream-only")
        .execute(&state.pool)
        .await
        .expect("annotate http_200 row with downstream-only failure metadata");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch summary for downstream-only http_200 row");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries for downstream-only http_200 row");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 2);
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
async fn timeseries_and_summary_treat_warning_success_as_success_like() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "timeseries-success-control",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-warning-success",
        &occurred_at,
        INVOCATION_STATUS_WARNING_SUCCESS,
        Some(120.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-failure-control",
        &occurred_at,
        "failed",
        Some(240.0),
    )
    .await;

    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, payload = ?3 WHERE invoke_id = ?4",
    )
    .bind("downstream_closed")
    .bind("none")
    .bind(
        json!({
            "downstreamErrorMessage":
                "[downstream_closed] downstream closed while streaming upstream response"
        })
        .to_string(),
    )
    .bind("timeseries-warning-success")
    .execute(&state.pool)
    .await
    .expect("annotate warning success row");

    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("upstream_response_failed")
    .bind("service_failure")
    .bind("[upstream_response_failed] upstream response stream reported failure")
    .bind("timeseries-failure-control")
    .execute(&state.pool)
    .await
    .expect("annotate failure control row");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch summary for warning success rows");
    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_f64_close(summary.total_cost, 0.03);
    assert_f64_close(
        summary
            .non_success_cost
            .expect("non-success cost should exclude warning success"),
        0.01,
    );
    assert_eq!(summary.non_success_tokens, Some(10));

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries for warning success rows");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 3)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 3);
    assert_eq!(bucket.success_count, 2);
    assert_eq!(bucket.failure_count, 1);
    assert_f64_close(bucket.non_success_cost, 0.01);
    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket
            .first_byte_avg_ms
            .expect("warning success should contribute latency"),
        (80.0 + 120.0) / 2.0,
    );
}

#[tokio::test]
async fn all_time_summary_ignores_stale_rollup_failure_counts_for_running_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "summary-all-success",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-running",
        &occurred_at,
        "running",
        Some(120.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-pending",
        &occurred_at,
        "pending",
        Some(160.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-failed",
        &occurred_at,
        "failed",
        Some(240.0),
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(4_i64)
    .bind(1_i64)
    .bind(3_i64)
    .bind(40_i64)
    .bind(0.4_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale invocation rollup counts");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind("codex_invocations")
    .bind(4_i64)
    .execute(&state.pool)
    .await
    .expect("mark invocation rollup progress as caught up");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with stale rollup counts");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
}
