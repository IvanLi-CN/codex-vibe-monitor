#[tokio::test]
pub(crate) async fn timeseries_includes_first_byte_avg_and_p95_for_success_samples() {
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

    let response = fetch_test_timeseries(state, "1h", "15m").await;
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
pub(crate) async fn timeseries_includes_legacy_http_200_success_like_ttfb_samples() {
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
pub(crate) async fn timeseries_and_summary_count_completed_rows_as_success() {
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
pub(crate) async fn failure_summary_excludes_completed_success_like_rows_from_recent_totals() {
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
pub(crate) async fn timeseries_reports_snapshot_id_for_live_exact_queries() {
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
pub(crate) async fn timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures()
 {
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

    let summary = fetch_test_summary(state.clone(), "1d").await;
    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);

    let response = fetch_test_timeseries(state, "1h", "15m").await;
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
pub(crate) async fn timeseries_and_summary_count_http_200_rows_with_downstream_only_failure_metadata()
 {
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
pub(crate) async fn timeseries_and_summary_treat_warning_success_as_success_like() {
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

    let summary = fetch_test_summary(state.clone(), "1d").await;
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

    let response = fetch_test_timeseries(state, "1h", "15m").await;
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
pub(crate) async fn all_time_summary_ignores_stale_rollup_failure_counts_for_running_rows() {
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

    let summary = fetch_test_summary(state, "all").await;

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
}

async fn seed_archived_stale_rollup_rows(pool: &SqlitePool, config: &AppConfig) -> String {
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_at = |minutes| {
        format_naive(
            archived_hour_local
                .checked_add_signed(ChronoDuration::minutes(minutes))
                .expect("valid archived invocation time"),
        )
    };
    let archived_success_at = archived_at(5);
    let archived_pending_at = archived_at(15);
    let archived_failed_at = archived_at(25);
    seed_invocation_archive_batch(
        pool,
        config,
        "summary-all-archived-stale-rollup",
        &[
            (
                1,
                "summary-all-archived-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10,
                0.10,
                Some(100.0),
            ),
            (
                2,
                "summary-all-archived-pending",
                archived_pending_at.as_str(),
                SOURCE_PROXY,
                "pending",
                10,
                0.10,
                Some(110.0),
            ),
            (
                3,
                "summary-all-archived-failed",
                archived_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                10,
                0.10,
                Some(120.0),
            ),
        ],
    )
    .await;
    archived_success_at
}

async fn materialize_stale_archived_rollup(pool: &SqlitePool, archived_success_at: &str) {
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations'",
    )
    .execute(pool)
    .await
    .expect("mark archived invocation batch as materialized");
    let bucket_start_epoch = invocation_bucket_start_epoch(archived_success_at)
        .expect("bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(3_i64)
    .bind(1_i64)
    .bind(2_i64)
    .bind(30_i64)
    .bind(0.30_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(pool)
    .await
    .expect("seed stale archived invocation rollup counts");
}

struct SummaryLiveRowFixture<'a> {
    id: i64,
    invoke_id: &'a str,
    minutes_ago: i64,
    status: &'a str,
    total_tokens: i64,
    cost: f64,
    ttfb_ms: Option<f64>,
}

async fn insert_summary_live_row(pool: &SqlitePool, fixture: SummaryLiveRowFixture<'_>) {
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(fixture.minutes_ago))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost,
            t_upstream_ttfb_ms, raw_response
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')
        "#,
    )
    .bind(fixture.id)
    .bind(fixture.invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(fixture.status)
    .bind(fixture.total_tokens)
    .bind(fixture.cost)
    .bind(fixture.ttfb_ms)
    .execute(pool)
    .await
    .expect("insert summary live row");
}

async fn assert_summary_repair_cursors_absent(pool: &SqlitePool) {
    for (dataset, message) in [
        (
            "codex_invocations_summary_rollup_v2",
            "read-only summary should not materialize repair markers inline",
        ),
        (
            "codex_invocations_summary_rollup_v2_live_cursor",
            "read-only summary should not materialize repair live cursors inline",
        ),
        (
            "codex_invocations",
            "read-only summary should not advance the shared hourly cursor inline",
        ),
    ] {
        let cursor = sqlx::query_scalar::<_, i64>(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(dataset)
        .fetch_optional(pool)
        .await
        .expect("load summary repair cursor presence");
        assert_eq!(cursor, None, "{message}");
    }
}

async fn publish_post_repair_live_tail(state: &Arc<AppState>) {
    insert_summary_live_row(
        &state.pool,
        SummaryLiveRowFixture {
            id: 11,
            invoke_id: "summary-all-live-tail-failed",
            minutes_ago: 3,
            status: "failed",
            total_tokens: 5,
            cost: 0.05,
            ttfb_ms: None,
        },
    )
    .await;
    let live_tail =
        sqlx::query_as::<_, ApiInvocation>("SELECT * FROM codex_invocations WHERE id = ?1")
            .bind(11_i64)
            .fetch_one(&state.pool)
            .await
            .expect("load committed post-repair live tail invocation");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &live_tail)
        .await
        .terminal_delta
        .expect("register committed post-repair live tail delta");
    state
        .subscription_hub
        .acknowledge_summary_delta(delta)
        .await;
}

#[tokio::test]
pub(crate) async fn all_time_summary_preserves_archived_history_when_rollup_failures_are_stale() {
    let state = archive_retention_test_state().await;
    let archived_success_at = seed_archived_stale_rollup_rows(&state.pool, &state.config).await;
    materialize_stale_archived_rollup(&state.pool, &archived_success_at).await;
    insert_summary_live_row(
        &state.pool,
        SummaryLiveRowFixture {
            id: 10,
            invoke_id: "summary-all-live-success",
            minutes_ago: 10,
            status: "success",
            total_tokens: 10,
            cost: 0.01,
            ttfb_ms: Some(130.0),
        },
    )
    .await;

    let response = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("missing live-rollup cursor must fail closed instead of omitting live history");
    assert!(matches!(response, ApiError::Unavailable(_)));

    assert_summary_repair_cursors_absent(&state.pool).await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary_repeat = fetch_test_summary(state.clone(), "all").await;

    assert_eq!(summary_repeat.total_count, 4);
    assert_eq!(summary_repeat.success_count, 2);
    assert_eq!(summary_repeat.failure_count, 1);
    assert_eq!(summary_repeat.total_tokens, 40);
    assert!((summary_repeat.total_cost - 0.31).abs() < 1e-9);

    publish_post_repair_live_tail(&state).await;
    let summary_with_live_tail = fetch_test_summary(state, "all").await;

    assert_eq!(summary_with_live_tail.total_count, 5);
    assert_eq!(summary_with_live_tail.success_count, 2);
    assert_eq!(summary_with_live_tail.failure_count, 2);
    assert_eq!(summary_with_live_tail.total_tokens, 45);
    assert!((summary_with_live_tail.total_cost - 0.36).abs() < 1e-9);
}

use super::*;
