#[tokio::test]
pub(crate) async fn timeseries_hourly_backed_omits_pre_cutoff_partial_archived_hours() {
    let (state, bucket_start, window) = archived_partial_hour_fixture().await;
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state.clone(),
        account_timeseries_query("ignored", "1h", None),
        Shanghai,
        InvocationSourceScope::ProxyOnly,
        window,
        TimeseriesBucketSelection {
            bucket_seconds: 3_600,
            effective_bucket: "1h".to_string(),
            available_buckets: vec!["1h".to_string()],
            bucket_limited_to_daily: false,
        },
    )
    .await
    .expect("fetch exact archived timeseries");

    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(bucket_start))
        .expect("archived hour bucket should remain visible");
    assert_eq!(point.total_count, 0);
    assert_eq!(point.success_count, 0);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 0);
    assert_f64_close(point.total_cost, 0.0);
}

#[tokio::test]
pub(crate) async fn hourly_backed_summary_omits_pre_cutoff_partial_hour_rollups() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let pre_cutoff_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local()
        - ChronoDuration::minutes(15);
    let occurred_at = format_naive(pre_cutoff_local);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pre-cutoff-live-exact")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert pre-cutoff live exact row");

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&occurred_at).expect("derive pre-cutoff bucket epoch");
    let bucket_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid pre-cutoff bucket start");
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        HourlyRollupFixture {
            bucket_start,
            source: SOURCE_PROXY,
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 12,
            total_cost: 0.12,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;

    let start = local_naive_to_utc(pre_cutoff_local - ChronoDuration::minutes(15), Shanghai);
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::ProxyOnly)
            .await
            .expect("load summary totals across retention cutoff");

    assert_eq!(totals.total_count, 0);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 0);
    assert_f64_close(totals.total_cost, 0.0);
}

#[tokio::test]
pub(crate) async fn hourly_backed_summary_replays_pre_cutoff_full_hour_live_rows_after_rollup_cursor()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let full_hour_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local()
        - ChronoDuration::hours(2);
    let occurred_at = format_naive(
        full_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("valid occurred_at"),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pre-cutoff-full-hour-live-tail")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert pre-cutoff full-hour live row");

    let start = local_naive_to_utc(full_hour_local - ChronoDuration::hours(1), Shanghai);
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::ProxyOnly)
            .await
            .expect("load summary totals across full archived hour");

    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 12);
    assert_f64_close(totals.total_cost, 0.12);
}

#[tokio::test]
pub(crate) async fn invocation_hourly_rollup_ignores_null_status_for_success_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 1,
            occurred_at,
            source: SOURCE_PROXY.to_string(),
            status: None,
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: Some(4),
            output_tokens: Some(3),
            cache_input_tokens: Some(2),
            reasoning_tokens: Some(1),
            total_tokens: Some(7),
            cost: Some(0.07),
            upstream_account_id: Some(42),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await
    .expect("upsert hourly rollup source row");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 1);
    assert_eq!(row.success_count, 0);
    assert_eq!(row.failure_count, 0);
    assert_eq!(row.total_tokens, 7);
    assert_eq!(row.input_tokens, 4);
    assert_eq!(row.output_tokens, 3);
    assert_eq!(row.cache_input_tokens, 2);
    assert_eq!(row.reasoning_tokens, 1);
    assert_f64_close(row.total_cost, 0.07);

    for table in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        let account_tokens = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(&format!(
            "SELECT total_tokens, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens FROM {table} WHERE upstream_account_id = 42",
        ))
        .fetch_one(&state.pool)
        .await
        .unwrap_or_else(|error| panic!("load {table} token components: {error}"));
        assert_eq!(account_tokens, (7, 4, 3, 2, 1));
    }
}

#[tokio::test]
pub(crate) async fn invocation_hourly_rollup_ignores_running_and_pending_for_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    let records = active_and_terminal_rollup_records(occurred_at);
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &records, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await
        .expect("upsert hourly rollup source rows");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 4);
    assert_eq!(row.success_count, 1);
    assert_eq!(row.failure_count, 1);
    assert_eq!(row.total_tokens, 40);
    assert_f64_close(row.total_cost, 0.4);
}

#[tokio::test]
pub(crate) async fn invocation_hourly_rollup_excludes_structured_legacy_http_200_failures_from_ttfb_samples()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    let records = legacy_http_200_rollup_records(occurred_at);
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &records, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await
        .expect("upsert hourly rollup source rows");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 2);
    assert_eq!(row.success_count, 1);
    assert_eq!(row.failure_count, 1);
    assert_eq!(row.first_byte_sample_count, 1);
    assert_f64_close(row.first_byte_sum_ms, 120.0);
}

#[tokio::test]
pub(crate) async fn combined_totals_ignore_null_status_for_success_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
    .bind(201_i64)
    .bind("summary-null-status")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind(7_i64)
    .bind(0.07_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null-status invocation row");

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
    .bind(202_i64)
    .bind("summary-success")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(11_i64)
    .bind(0.11_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert success invocation row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 2);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 18);
    assert_f64_close(totals.total_cost, 0.18);
}

#[tokio::test]
pub(crate) async fn account_scoped_summary_and_timeseries_filter_by_payload_upstream_account_id() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let recent_complete_hour = format_naive(
        (Utc::now() - ChronoDuration::days(2))
            .with_timezone(&Shanghai)
            .date_naive()
            .and_hms_opt(1, 0, 0)
            .expect("valid recent complete hour"),
    );

    seed_account_scoped_stats_rows(&state.pool, &occurred_at, &recent_complete_hour).await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("fetch account-scoped summary");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 120);
    assert_f64_close(summary.total_cost, 0.42);
    assert_eq!(summary.in_progress_conversation_count, Some(0));

    let Json(timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(account_timeseries_query("today", "1m", Some(42))),
    )
    .await
    .expect("fetch account-scoped timeseries");

    let populated_points: Vec<_> = timeseries
        .points
        .iter()
        .filter(|point| point.total_count > 0)
        .collect();
    assert_eq!(populated_points.len(), 1);
    let point = populated_points[0];
    assert_eq!(point.total_count, 1);
    assert_eq!(point.success_count, 1);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 120);
    assert_f64_close(point.total_cost, 0.42);
    assert_f64_close(point.non_success_cost, 0.0);

    let Json(hourly_timeseries) = fetch_timeseries(
        State(state),
        Query(account_timeseries_query("7d", "1h", Some(42))),
    )
    .await
    .expect("fetch recent account-scoped hourly timeseries");

    let total_hourly_count: i64 = hourly_timeseries
        .points
        .iter()
        .map(|point| point.total_count)
        .sum();
    let total_hourly_tokens: i64 = hourly_timeseries
        .points
        .iter()
        .map(|point| point.total_tokens)
        .sum();
    assert_eq!(total_hourly_count, 2);
    assert_eq!(total_hourly_tokens, 180);
}

#[tokio::test]
pub(crate) async fn summary_reports_invocation_based_in_progress_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_in_progress_summary_rows(&state.pool, &occurred_at).await;

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch stats with in-progress invocations");
    assert_live_phase_counts(
        stats.in_progress_conversation_count,
        stats.in_progress_phase_counts,
        "stats",
    );

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch today summary with in-progress invocations");
    assert_live_phase_counts(
        summary.in_progress_conversation_count,
        summary.in_progress_phase_counts,
        "summary",
    );
}

async fn seed_in_progress_summary_rows(pool: &SqlitePool, occurred_at: &str) {
    for (id, invoke_id, status, payload, t_req_read_ms, t_upstream_ttfb_ms) in [
        (
            401_i64,
            "in-progress-running-a",
            "running",
            Some(json!({ "promptCacheKey": "pck-live-a" }).to_string()),
            Some(8.0_f64),
            None,
        ),
        (
            402_i64,
            "in-progress-pending-a",
            "pending",
            Some(json!({ "promptCacheKey": "pck-live-a" }).to_string()),
            None,
            None,
        ),
        (
            403_i64,
            "in-progress-running-b",
            "running",
            Some(json!({ "promptCacheKey": "pck-live-b" }).to_string()),
            None,
            Some(120.0_f64),
        ),
        (
            406_i64,
            "in-progress-running-zero-placeholder",
            "running",
            Some(json!({ "promptCacheKey": "pck-live-zero" }).to_string()),
            None,
            Some(0.0_f64),
        ),
        (
            404_i64,
            "in-progress-success-a",
            "success",
            Some(json!({ "promptCacheKey": "pck-live-a" }).to_string()),
            None,
            None,
        ),
        (
            405_i64,
            "in-progress-success-c",
            "success",
            Some(json!({ "promptCacheKey": "pck-finished-c" }).to_string()),
            None,
            None,
        ),
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
                t_req_read_ms,
                t_upstream_ttfb_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(t_req_read_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(payload)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert in-progress summary row");
    }
}

fn assert_live_phase_counts(
    conversation_count: Option<i64>,
    phase_counts: Option<InvocationPhaseCountsResponse>,
    label: &str,
) {
    assert_eq!(conversation_count, Some(4));
    let phase_counts =
        phase_counts.unwrap_or_else(|| panic!("{label} should include live phase counts"));
    assert_eq!(phase_counts.queued, 3);
    assert_eq!(phase_counts.requesting, 1);
    assert_eq!(phase_counts.responding, 0);
}

async fn archived_partial_hour_fixture() -> (Arc<AppState>, DateTime<Utc>, RangeWindow) {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let local_hour = (Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(10))
        .and_hms_opt(8, 0, 0)
        .expect("valid archived local hour");
    let bucket_start = local_naive_to_utc(local_hour, Shanghai);
    let before_start = format_naive(local_hour + ChronoDuration::minutes(10));
    let after_start = format_naive(local_hour + ChronoDuration::minutes(50));
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "timeseries-exact-archived-start",
        &[
            (
                1,
                "timeseries-before-start",
                &before_start,
                SOURCE_PROXY,
                "success",
                10,
                0.1,
                Some(100.0),
            ),
            (
                2,
                "timeseries-after-start",
                &after_start,
                SOURCE_PROXY,
                "success",
                10,
                0.1,
                Some(200.0),
            ),
        ],
    )
    .await;
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        HourlyRollupFixture {
            bucket_start,
            source: SOURCE_PROXY,
            total_count: 2,
            success_count: 2,
            failure_count: 0,
            total_tokens: 20,
            total_cost: 0.2,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;
    let start = local_naive_to_utc(local_hour + ChronoDuration::minutes(30), Shanghai);
    let end = local_naive_to_utc(local_hour + ChronoDuration::minutes(55), Shanghai);
    let window = RangeWindow {
        start,
        end,
        display_end: end,
        duration: end - start,
    };
    (state, bucket_start, window)
}

fn account_timeseries_query(
    range: &str,
    bucket: &str,
    upstream_account_id: Option<i64>,
) -> TimeseriesQuery {
    TimeseriesQuery {
        range: range.to_string(),
        bucket: Some(bucket.to_string()),
        settlement_hour: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id,
    }
}

pub(super) fn hourly_source_record(
    id: i64,
    occurred_at: String,
    status: Option<&str>,
    total_tokens: i64,
    cost: f64,
) -> InvocationHourlySourceRecord {
    InvocationHourlySourceRecord {
        id,
        occurred_at,
        source: SOURCE_PROXY.to_string(),
        status: status.map(str::to_string),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(total_tokens),
        cost: Some(cost),
        upstream_account_id: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    }
}

fn active_and_terminal_rollup_records(occurred_at: String) -> Vec<InvocationHourlySourceRecord> {
    let success = hourly_source_record(1, occurred_at.clone(), Some("success"), 7, 0.07);
    let mut running = hourly_source_record(2, occurred_at.clone(), Some("running"), 9, 0.09);
    running.error_message =
        Some("[upstream_response_failed] upstream response stream reported failure".to_string());
    running.failure_kind = Some("upstream_response_failed".to_string());
    running.failure_class = Some("service_failure".to_string());
    running.is_actionable = Some(1);
    let mut pending = hourly_source_record(3, occurred_at.clone(), Some("pending"), 11, 0.11);
    pending.error_message =
        Some("[downstream_closed] downstream closed while streaming upstream response".to_string());
    pending.failure_kind = Some("downstream_closed".to_string());
    pending.failure_class = Some("client_abort".to_string());
    pending.is_actionable = Some(0);
    let mut failed = hourly_source_record(4, occurred_at, Some("failed"), 13, 0.13);
    failed.error_message = Some("upstream stream error".to_string());
    failed.failure_kind = Some("upstream_stream_error".to_string());
    failed.is_actionable = Some(1);
    vec![success, running, pending, failed]
}

fn legacy_http_200_rollup_records(occurred_at: String) -> Vec<InvocationHourlySourceRecord> {
    let mut success = hourly_source_record(11, occurred_at.clone(), Some("http_200"), 10, 0.10);
    success.error_message = Some(String::new());
    success.t_upstream_ttfb_ms = Some(120.0);
    let mut failure = hourly_source_record(12, occurred_at, Some("http_200"), 20, 0.20);
    failure.error_message = Some(String::new());
    failure.failure_kind = Some("upstream_response_failed".to_string());
    failure.failure_class = Some("service_failure".to_string());
    failure.is_actionable = Some(1);
    failure.t_upstream_ttfb_ms = Some(840.0);
    vec![success, failure]
}

async fn seed_account_scoped_stats_rows(
    pool: &SqlitePool,
    occurred_at: &str,
    recent_complete_hour: &str,
) {
    let rows = [
        (
            301,
            "account-stats-target",
            occurred_at,
            "success",
            json!({ "upstreamAccountId": 42 }),
            120,
            0.42,
        ),
        (
            304,
            "account-stats-target-recent-complete-hour",
            recent_complete_hour,
            "success",
            json!({ "upstreamAccountId": 42 }),
            60,
            0.24,
        ),
        (
            302,
            "account-stats-other",
            occurred_at,
            "failed",
            json!({ "upstreamAccountId": 17 }),
            900,
            9.0,
        ),
        (
            303,
            "account-stats-legacy-missing-payload-id",
            occurred_at,
            "success",
            json!({ "upstreamAccountName": "legacy" }),
            700,
            7.0,
        ),
    ];
    for (id, invoke_id, row_occurred_at, status, payload, total_tokens, cost) in rows {
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')",
        )
        .bind(id)
        .bind(invoke_id)
        .bind(row_occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(payload.to_string())
        .execute(pool)
        .await
        .expect("insert account-scoped stats invocation row");
    }
}

use super::*;
