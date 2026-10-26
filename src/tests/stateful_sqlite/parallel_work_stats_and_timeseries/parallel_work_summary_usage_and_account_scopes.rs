use super::*;

#[tokio::test]
async fn account_scoped_summary_and_timeseries_filter_by_payload_upstream_account_id() {
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

    for (id, invoke_id, row_occurred_at, status, payload, total_tokens, cost) in [
        (
            301_i64,
            "account-stats-target",
            occurred_at.as_str(),
            "success",
            Some(json!({ "upstreamAccountId": 42 }).to_string()),
            120_i64,
            0.42_f64,
        ),
        (
            304_i64,
            "account-stats-target-recent-complete-hour",
            recent_complete_hour.as_str(),
            "success",
            Some(json!({ "upstreamAccountId": 42 }).to_string()),
            60_i64,
            0.24_f64,
        ),
        (
            302_i64,
            "account-stats-other",
            occurred_at.as_str(),
            "failed",
            Some(json!({ "upstreamAccountId": 17 }).to_string()),
            900_i64,
            9.0_f64,
        ),
        (
            303_i64,
            "account-stats-legacy-missing-payload-id",
            occurred_at.as_str(),
            "success",
            Some(json!({ "upstreamAccountName": "legacy" }).to_string()),
            700_i64,
            7.0_f64,
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
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(row_occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert account-scoped stats invocation row");
    }

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
        Query(TimeseriesQuery {
            range: "today".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
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
        Query(TimeseriesQuery {
            range: "7d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
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
async fn summary_reports_invocation_based_in_progress_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(t_req_read_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(payload)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert in-progress summary row");
    }

    hydrate_stats_snapshot_for_test(&state).await;
    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch stats with in-progress invocations");
    assert_eq!(stats.in_progress_conversation_count, Some(4));
    let stats_phase_counts = stats
        .in_progress_phase_counts
        .expect("stats should include live phase counts");
    assert_eq!(stats_phase_counts.queued, 3);
    assert_eq!(stats_phase_counts.requesting, 1);
    assert_eq!(stats_phase_counts.responding, 0);

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
    assert_eq!(summary.in_progress_conversation_count, Some(4));
    let summary_phase_counts = summary
        .in_progress_phase_counts
        .expect("summary should include live phase counts");
    assert_eq!(summary_phase_counts.queued, 3);
    assert_eq!(summary_phase_counts.requesting, 1);
    assert_eq!(summary_phase_counts.responding, 0);
}

#[tokio::test]
async fn ranged_summary_exposes_model_usage_and_exact_cost_breakdown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, model, status,
            input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens,
            cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(449_i64)
    .bind("summary-usage-breakdown")
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind("gpt-5.6")
    .bind("success")
    .bind(100_i64)
    .bind(40_i64)
    .bind(25_i64)
    .bind(5_i64)
    .bind(125_i64)
    .bind(0.50_f64)
    .bind(0.06_f64)
    .bind(0.14_f64)
    .bind(0.04_f64)
    .bind(0.21_f64)
    .bind(0.05_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert ranged summary usage row");

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
    .expect("fetch today usage breakdown");

    let breakdown = summary
        .usage_breakdown
        .expect("today summary should include usage breakdown");
    assert_eq!(breakdown.cache_write_tokens, 60);
    assert_eq!(breakdown.cache_read_tokens, 40);
    assert_eq!(breakdown.output_tokens, 25);
    let costs = breakdown.costs.expect("exact costs should be available");
    assert_f64_close(costs.input, 0.06);
    assert_f64_close(costs.cache_write, 0.14);
    assert_f64_close(costs.cache_read, 0.04);
    assert_f64_close(costs.output, 0.21);
    assert_f64_close(costs.reasoning, 0.05);
    assert_f64_close(costs.unknown, 0.0);
    assert_f64_close(
        costs.input
            + costs.cache_write
            + costs.cache_read
            + costs.output
            + costs.reasoning
            + costs.unknown,
        summary.total_cost,
    );
    assert_eq!(breakdown.models.len(), 1);
    assert_eq!(breakdown.models[0].model, "gpt-5.6");
}

#[tokio::test]
async fn ranged_summary_groups_model_usage_by_reasoning_effort() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (id, invoke_id, reasoning_effort, input_tokens, cache_input_tokens, output_tokens, cost) in [
        (
            460_i64,
            "summary-effort-high",
            Some("high"),
            100_i64,
            40_i64,
            25_i64,
            0.50_f64,
        ),
        (
            461_i64,
            "summary-effort-medium",
            Some("  medium  "),
            80_i64,
            30_i64,
            20_i64,
            0.40_f64,
        ),
        (
            462_i64,
            "summary-effort-unspecified",
            None,
            60_i64,
            20_i64,
            15_i64,
            0.30_f64,
        ),
        (
            463_i64,
            "summary-effort-blank",
            Some("   "),
            50_i64,
            10_i64,
            10_i64,
            0.20_f64,
        ),
    ] {
        let payload = reasoning_effort
            .map(|value| json!({ "reasoningEffort": value }).to_string())
            .unwrap_or_else(|| "{}".to_string());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, status,
                input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, 0.01, 0.02, 0.03, 0.04, ?12, ?13, '{}')
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("gpt-5.6")
        .bind("success")
        .bind(input_tokens)
        .bind(cache_input_tokens)
        .bind(output_tokens)
        .bind(input_tokens + output_tokens)
        .bind(cost)
        .bind(cost - 0.10)
        .bind(payload)
        .execute(&state.pool)
        .await
        .expect("insert reasoning effort usage row");
    }

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
    .expect("fetch reasoning effort usage breakdown");

    let breakdown = summary.usage_breakdown.expect("usage breakdown");
    assert_eq!(breakdown.cache_write_tokens, 190);
    assert_eq!(breakdown.cache_read_tokens, 100);
    assert_eq!(breakdown.output_tokens, 70);
    assert_eq!(breakdown.models.len(), 3);
    let costs = breakdown.costs.as_ref().expect("exact cost breakdown");
    assert_f64_close(costs.input, 0.04);
    assert_f64_close(costs.cache_write, 0.08);
    assert_f64_close(costs.cache_read, 0.12);
    assert_f64_close(costs.output, 0.16);
    assert_f64_close(costs.reasoning, 1.00);
    assert_f64_close(costs.unknown, 0.00);
    assert_f64_close(
        costs.input
            + costs.cache_write
            + costs.cache_read
            + costs.output
            + costs.reasoning
            + costs.unknown,
        summary.total_cost,
    );

    let find_effort = |effort: Option<&str>| {
        breakdown
            .models
            .iter()
            .find(|item| item.model == "gpt-5.6" && item.reasoning_effort.as_deref() == effort)
            .expect("model and reasoning effort group")
    };
    assert_eq!(find_effort(Some("high")).cache_write_tokens, 60);
    assert_eq!(find_effort(Some("high")).cache_read_tokens, 40);
    assert_f64_close(
        find_effort(Some("high"))
            .costs
            .as_ref()
            .expect("high effort costs")
            .reasoning,
        0.40,
    );
    assert_eq!(find_effort(Some("medium")).cache_write_tokens, 50);
    assert_eq!(find_effort(Some("medium")).cache_read_tokens, 30);
    assert_f64_close(
        find_effort(Some("medium"))
            .costs
            .as_ref()
            .expect("medium effort costs")
            .reasoning,
        0.30,
    );
    assert_eq!(find_effort(None).cache_write_tokens, 80);
    assert_eq!(find_effort(None).cache_read_tokens, 30);
    let unspecified_costs = find_effort(None)
        .costs
        .as_ref()
        .expect("unspecified effort costs");
    assert_f64_close(unspecified_costs.input, 0.02);
    assert_f64_close(unspecified_costs.cache_write, 0.04);
    assert_f64_close(unspecified_costs.cache_read, 0.06);
    assert_f64_close(unspecified_costs.output, 0.08);
    assert_f64_close(unspecified_costs.reasoning, 0.30);
}

#[tokio::test]
async fn ranged_summary_keeps_exact_costs_when_historical_cost_is_mixed_in() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (
        id,
        invoke_id,
        model,
        cost,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
    ) in [
        (
            450_i64,
            "summary-exact-cost",
            "gpt-5.6",
            Some(0.50_f64),
            Some(0.06_f64),
            Some(0.14_f64),
            Some(0.04_f64),
            Some(0.21_f64),
            Some(0.05_f64),
        ),
        (
            451_i64,
            "summary-historical-cost",
            "gpt-5.4",
            Some(0.30_f64),
            None,
            None,
            None,
            None,
            None,
        ),
        (
            452_i64,
            "summary-missing-total-cost",
            "gpt-no-cost",
            None,
            None,
            None,
            None,
            None,
            None,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, status,
                input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind("success")
        .bind(100_i64)
        .bind(40_i64)
        .bind(25_i64)
        .bind(5_i64)
        .bind(125_i64)
        .bind(cost)
        .bind(cost_input)
        .bind(cost_cache_write)
        .bind(cost_cache_read)
        .bind(cost_output)
        .bind(cost_reasoning)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert mixed ranged summary usage row");
    }

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch mixed today usage breakdown");

    let breakdown = summary
        .usage_breakdown
        .expect("today summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("historical costs must not hide exact realtime cost buckets");
    assert_f64_close(costs.input, 0.06);
    assert_f64_close(costs.cache_write, 0.14);
    assert_f64_close(costs.cache_read, 0.04);
    assert_f64_close(costs.output, 0.21);
    assert_f64_close(costs.reasoning, 0.05);
    assert_f64_close(costs.unknown, 0.30);
    assert_f64_close(
        costs.input
            + costs.cache_write
            + costs.cache_read
            + costs.output
            + costs.reasoning
            + costs.unknown,
        summary.total_cost,
    );

    let exact_model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5.6")
        .expect("exact model usage");
    let exact_model_costs = exact_model.costs.as_ref().expect("exact model costs");
    assert_f64_close(exact_model_costs.unknown, 0.0);
    assert_f64_close(
        exact_model_costs.input
            + exact_model_costs.cache_write
            + exact_model_costs.cache_read
            + exact_model_costs.output
            + exact_model_costs.reasoning
            + exact_model_costs.unknown,
        0.50,
    );

    let historical_model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5.4")
        .expect("historical model usage");
    let historical_model_costs = historical_model
        .costs
        .as_ref()
        .expect("historical total cost should be represented as unknown");
    assert_f64_close(historical_model_costs.input, 0.0);
    assert_f64_close(historical_model_costs.unknown, 0.30);

    let missing_cost_model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-no-cost")
        .expect("missing cost model usage");
    assert!(
        missing_cost_model.costs.is_none(),
        "a missing total cost must not fabricate unknown cost"
    );

    sqlx::query(
        "DELETE FROM codex_invocations WHERE invoke_id IN ('summary-exact-cost', 'summary-missing-total-cost')",
    )
    .execute(&state.pool)
    .await
    .expect("leave only the historical cost row");

    let Json(historical_summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch historical-only today usage breakdown");
    let historical_costs = historical_summary
        .usage_breakdown
        .expect("historical summary usage breakdown")
        .costs
        .expect("known historical total cost should remain available");
    assert_f64_close(historical_costs.input, 0.0);
    assert_f64_close(historical_costs.cache_write, 0.0);
    assert_f64_close(historical_costs.cache_read, 0.0);
    assert_f64_close(historical_costs.output, 0.0);
    assert_f64_close(historical_costs.reasoning, 0.0);
    assert_f64_close(historical_costs.unknown, 0.30);
    assert_f64_close(historical_summary.total_cost, 0.30);
}

#[tokio::test]
async fn usage_breakdown_includes_archived_start_boundary_partial_hour_for_7d_range() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let archived_boundary_at = archived_start_partial_hour_at();
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-archived-boundary-breakdown",
        &[SeedInvocationArchiveBatchRow {
            id: 601_i64,
            invoke_id: "summary-7d-archived-boundary-breakdown",
            occurred_at: archived_boundary_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20_i64,
            cost: 0.20_f64,
            ttfb_ms: Some(150.0_f64),
            payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0_i64),
        }],
    )
    .await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d summary with archived boundary usage breakdown");

    let breakdown = summary
        .usage_breakdown
        .expect("7d summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("archived boundary cost should still surface in usage breakdown");
    assert_f64_close(costs.unknown, 0.20);

    let boundary_model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("archived boundary model group should remain visible");
    let boundary_model_costs = boundary_model
        .costs
        .as_ref()
        .expect("archived boundary model costs should remain visible");
    assert_f64_close(boundary_model_costs.unknown, 0.20);
}

#[tokio::test]
async fn summary_projection_materialized_boundary_older_than_48h_keeps_exact_views() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let archived_at = archived_start_partial_hour_at();

    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-materialized-boundary-over-48h",
        &[SeedInvocationArchiveBatchRow {
            id: 901,
            invoke_id: "summary-materialized-boundary-over-48h",
            occurred_at: archived_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20,
            cost: 0.20,
            ttfb_ms: Some(150.0),
            payload: Some(
                r#"{"responseModel":"gpt-5","reasoningEffort":"high","upstreamAccountId":42}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path LIKE '%summary-materialized-boundary-over-48h%'",
    )
    .execute(&state.pool)
    .await
    .expect("mark boundary archive materialized");
    let bucket = invocation_bucket_start_epoch(&archived_at).expect("boundary bucket");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, \
          terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) \
         VALUES (?1, ?2, 1, 1, 0, 1, 20, 0.20, 1, 20, 0.20)",
    )
    .bind(bucket)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("seed materialized boundary rollup");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket,
        SOURCE_PROXY,
    )
    .await;
    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &archive_path,
    )
    .await;

    let Json(seven_day) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("hydrate materialized boundary projection");
    assert_eq!(seven_day.total_count, 1);
    assert_eq!(seven_day.total_tokens, 20);
    assert_f64_close(seven_day.total_cost, 0.20);
    let seven_day_model = seven_day
        .usage_breakdown
        .as_ref()
        .expect("7d usage breakdown")
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("7d boundary model");
    assert_f64_close(
        seven_day_model
            .costs
            .as_ref()
            .expect("7d boundary model costs")
            .unknown,
        0.20,
    );
    state.pool.close().await;
    for window in ["7d", "previous7d"] {
        for upstream_account_id in [None, Some(42_i64)] {
            let Json(response) = fetch_summary(
                State(state.clone()),
                Query(SummaryQuery {
                    window: Some(window.to_string()),
                    limit: None,
                    time_zone: Some("Asia/Shanghai".to_string()),
                    upstream_account_id,
                }),
            )
            .await
            .expect("serve boundary summary without sqlite");
            assert_eq!(response.total_count, 1, "{window} boundary count");
            assert_eq!(response.total_tokens, 20, "{window} boundary tokens");
            assert_f64_close(response.total_cost, 0.20);
            assert_f64_close(
                response
                    .usage_breakdown
                    .expect("boundary usage breakdown")
                    .costs
                    .expect("boundary usage costs")
                    .unknown,
                0.20,
            );
        }
    }
}

#[tokio::test]
async fn usage_breakdown_keeps_archived_boundary_partial_hour_during_partial_archive_replay() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let archived_boundary_at = archived_start_partial_hour_at();
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-archived-boundary-partial-progress",
        &[SeedInvocationArchiveBatchRow {
            id: 801_i64,
            invoke_id: "summary-7d-archived-boundary-partial-progress",
            occurred_at: archived_boundary_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20_i64,
            cost: 0.20_f64,
            ttfb_ms: Some(150.0_f64),
            payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0_i64),
        }],
    )
    .await;

    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 801_i64,
            occurred_at: archived_boundary_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: None,
            output_tokens: Some(20_i64),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(20_i64),
            cost: Some(0.20_f64),
            upstream_account_id: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none".to_string()),
            is_actionable: Some(0_i64),
            payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#.to_string()),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(150.0_f64),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed boundary partial-hour usage breakdown rollup row");
    tx.commit().await.expect("commit breakdown rollup tx");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(801_i64)
    .execute(&state.pool)
    .await
    .expect("insert boundary partial archive replay progress");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d summary with partially replayed archived boundary usage breakdown");

    let breakdown = summary
        .usage_breakdown
        .expect("7d summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("boundary partial-hour cost should still surface in usage breakdown");
    assert_f64_close(costs.unknown, 0.20);

    let boundary_model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("boundary partial-hour model group should remain visible");
    let boundary_model_costs = boundary_model
        .costs
        .as_ref()
        .expect("boundary partial-hour model costs should remain visible");
    assert_f64_close(boundary_model_costs.unknown, 0.20);
}

#[tokio::test]
async fn usage_breakdown_avoids_double_counting_partially_materialized_archive_rows() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let first_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7) + ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let second_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7)
            + ChronoDuration::hours(2)
            + ChronoDuration::minutes(10))
        .with_timezone(&Shanghai)
        .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-partial-usage-breakdown-backfill",
        &[
            SeedInvocationArchiveBatchRow {
                id: 701_i64,
                invoke_id: "summary-7d-partial-usage-breakdown-a",
                occurred_at: first_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 702_i64,
                invoke_id: "summary-7d-partial-usage-breakdown-b",
                occurred_at: second_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
        ],
    )
    .await;

    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 701_i64,
            occurred_at: first_archived_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: None,
            output_tokens: Some(10_i64),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(10_i64),
            cost: Some(0.10_f64),
            upstream_account_id: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none".to_string()),
            is_actionable: Some(0_i64),
            payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#.to_string()),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(100.0_f64),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed partially materialized usage breakdown rollup row");
    tx.commit().await.expect("commit breakdown rollup tx");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(701_i64)
    .execute(&state.pool)
    .await
    .expect("insert partial usage breakdown archive replay progress");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d summary with partially materialized archive usage breakdown");

    let breakdown = summary
        .usage_breakdown
        .expect("7d summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("partially materialized archive costs should remain visible");
    assert_f64_close(costs.unknown, 0.30);

    let model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("partially materialized model group should remain visible");
    let model_costs = model
        .costs
        .as_ref()
        .expect("partially materialized model costs should remain visible");
    assert_f64_close(model_costs.unknown, 0.30);
}

#[tokio::test]
async fn usage_breakdown_ignores_shared_archive_progress_without_breakdown_cursor() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let first_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7) + ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let second_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7)
            + ChronoDuration::hours(2)
            + ChronoDuration::minutes(10))
        .with_timezone(&Shanghai)
        .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-shared-progress-without-breakdown-cursor",
        &[
            SeedInvocationArchiveBatchRow {
                id: 901_i64,
                invoke_id: "summary-7d-shared-progress-without-breakdown-cursor-a",
                occurred_at: first_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 902_i64,
                invoke_id: "summary-7d-shared-progress-without-breakdown-cursor-b",
                occurred_at: second_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
        ],
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(902_i64)
    .execute(&state.pool)
    .await
    .expect("insert legacy shared archive replay progress");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d summary with shared-only archive replay progress");

    let breakdown = summary
        .usage_breakdown
        .expect("7d summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("shared-only progress should not skip archived breakdown costs");
    assert_f64_close(costs.unknown, 0.30);

    let model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("shared-only progress should not skip archived breakdown model group");
    let model_costs = model
        .costs
        .as_ref()
        .expect("shared-only progress model costs should remain visible");
    assert_f64_close(model_costs.unknown, 0.30);
}

#[tokio::test]
async fn usage_breakdown_prefers_breakdown_specific_archive_progress_over_shared_cursor() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let first_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7) + ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let second_archived_at = format_naive(
        (Utc::now() - ChronoDuration::days(7)
            + ChronoDuration::hours(2)
            + ChronoDuration::minutes(10))
        .with_timezone(&Shanghai)
        .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-breakdown-progress-override",
        &[
            SeedInvocationArchiveBatchRow {
                id: 801_i64,
                invoke_id: "summary-7d-breakdown-progress-override-a",
                occurred_at: first_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 802_i64,
                invoke_id: "summary-7d-breakdown-progress-override-b",
                occurred_at: second_archived_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0_f64),
                payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
        ],
    )
    .await;

    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 801_i64,
            occurred_at: first_archived_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: None,
            output_tokens: Some(10_i64),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(10_i64),
            cost: Some(0.10_f64),
            upstream_account_id: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none".to_string()),
            is_actionable: Some(0_i64),
            payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#.to_string()),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(100.0_f64),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed partially materialized usage breakdown rollup row");
    tx.commit().await.expect("commit breakdown rollup tx");

    for (dataset, cursor_id) in [
        (HOURLY_ROLLUP_DATASET_INVOCATIONS, 802_i64),
        (INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET, 801_i64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(dataset)
        .bind(archive_path.to_string_lossy().to_string())
        .bind(cursor_id)
        .execute(&state.pool)
        .await
        .expect("insert archive replay progress");
    }

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d summary with breakdown-specific progress override");

    let breakdown = summary
        .usage_breakdown
        .expect("7d summary should include usage breakdown");
    let costs = breakdown
        .costs
        .expect("usage breakdown costs should remain visible with progress override");
    assert_f64_close(costs.unknown, 0.30);

    let model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("partially materialized model group should remain visible");
    let model_costs = model
        .costs
        .as_ref()
        .expect("partially materialized model costs should remain visible");
    assert_f64_close(model_costs.unknown, 0.30);
}
