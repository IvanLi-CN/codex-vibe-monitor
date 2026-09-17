#[tokio::test]
pub(crate) async fn ranged_summary_exposes_model_usage_and_exact_cost_breakdown() {
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
pub(crate) async fn ranged_summary_groups_model_usage_by_reasoning_effort() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_reasoning_effort_rows(&state.pool, &occurred_at).await;
    let summary = fetch_usage_summary(state, "today").await;
    assert_reasoning_effort_breakdown(summary);
}

fn assert_reasoning_effort_breakdown(summary: StatsResponse) {
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
pub(crate) async fn ranged_summary_keeps_exact_costs_when_historical_cost_is_mixed_in() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_mixed_cost_rows(&state.pool, &occurred_at).await;
    let summary = fetch_usage_summary(state.clone(), "today").await;
    assert_mixed_cost_breakdown(summary);
    leave_historical_cost_row(&state.pool).await;
    let historical_summary = fetch_usage_summary(state, "today").await;
    assert_historical_cost_only(historical_summary);
}

fn assert_mixed_cost_breakdown(summary: StatsResponse) {
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
}

fn assert_historical_cost_only(historical_summary: StatsResponse) {
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
pub(crate) async fn usage_breakdown_includes_archived_start_boundary_partial_hour_for_7d_range() {
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
pub(crate) async fn summary_projection_materialized_boundary_older_than_48h_keeps_exact_views() {
    let state = materialized_boundary_fixture().await;
    let seven_day = fetch_usage_summary(state.clone(), "7d").await;
    assert_summary_unknown_model_cost(seven_day, 1, 20, 0.20);
    state.pool.close().await;
    assert_materialized_boundary_views(state).await;
}

#[tokio::test]
pub(crate) async fn usage_breakdown_keeps_archived_boundary_partial_hour_during_partial_archive_replay()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    seed_boundary_partial_replay(&state).await;
    let summary = fetch_usage_summary(state, "7d").await;
    assert_summary_unknown_model_cost(summary, 1, 20, 0.20);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_avoids_double_counting_partially_materialized_archive_rows() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 6;
    let state = test_state_from_config(config, true).await;

    let (archive_path, first_archived_at) =
        seed_two_breakdown_rows(&state, "summary-7d-partial-usage-breakdown-backfill", 701).await;
    seed_partial_breakdown_rollup(&state, first_archived_at).await;
    insert_archive_progress(
        &state.pool,
        &archive_path,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        701,
    )
    .await;
    let summary = fetch_usage_summary(state, "7d").await;
    assert_summary_unknown_model_cost(summary, 2, 30, 0.30);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_ignores_shared_archive_progress_without_breakdown_cursor() {
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

    assert_summary_unknown_model_cost(summary, 2, 30, 0.30);
}

pub(super) async fn fetch_usage_summary(state: Arc<AppState>, window: &str) -> StatsResponse {
    fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some(window.to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("fetch {window} usage summary: {error:?}"))
    .0
}

async fn seed_reasoning_effort_rows(pool: &SqlitePool, occurred_at: &str) {
    let rows = [
        (460, "summary-effort-high", Some("high"), 100, 40, 25, 0.50),
        (
            461,
            "summary-effort-medium",
            Some("  medium  "),
            80,
            30,
            20,
            0.40,
        ),
        (462, "summary-effort-unspecified", None, 60, 20, 15, 0.30),
        (463, "summary-effort-blank", Some("   "), 50, 10, 10, 0.20),
    ];
    for (id, invoke_id, effort, input, cache_input, output, cost) in rows {
        let payload = effort.map_or_else(
            || "{}".to_string(),
            |value| json!({ "reasoningEffort": value }).to_string(),
        );
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model, status, input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, payload, raw_response) VALUES (?1, ?2, ?3, ?4, 'gpt-5.6', 'success', ?5, ?6, ?7, 0, ?8, ?9, 0.01, 0.02, 0.03, 0.04, ?10, ?11, '{}')",
        )
        .bind(id).bind(invoke_id).bind(occurred_at).bind(SOURCE_PROXY)
        .bind(input).bind(cache_input).bind(output).bind(input + output)
        .bind(cost).bind(cost - 0.10).bind(payload)
        .execute(pool).await.expect("insert reasoning effort usage row");
    }
}

async fn seed_mixed_cost_rows(pool: &SqlitePool, occurred_at: &str) {
    let rows = [
        (
            450,
            "summary-exact-cost",
            "gpt-5.6",
            Some(0.50),
            Some([0.06, 0.14, 0.04, 0.21, 0.05]),
        ),
        (451, "summary-historical-cost", "gpt-5.4", Some(0.30), None),
        (452, "summary-missing-total-cost", "gpt-no-cost", None, None),
    ];
    for (id, invoke_id, model, cost, parts) in rows {
        let [input, cache_write, cache_read, output, reasoning] = parts.unwrap_or([0.0; 5]);
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model, status, input_tokens, cache_input_tokens, output_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, 'success', 100, 40, 25, 5, 125, ?6, ?7, ?8, ?9, ?10, ?11, '{}')",
        )
        .bind(id).bind(invoke_id).bind(occurred_at).bind(SOURCE_PROXY).bind(model).bind(cost)
        .bind(parts.map(|_| input)).bind(parts.map(|_| cache_write)).bind(parts.map(|_| cache_read))
        .bind(parts.map(|_| output)).bind(parts.map(|_| reasoning))
        .execute(pool).await.expect("insert mixed ranged summary usage row");
    }
}

async fn leave_historical_cost_row(pool: &SqlitePool) {
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id IN ('summary-exact-cost', 'summary-missing-total-cost')")
        .execute(pool).await.expect("leave only the historical cost row");
}

pub(super) fn assert_summary_unknown_model_cost(
    summary: StatsResponse,
    total_count: i64,
    total_tokens: i64,
    expected_cost: f64,
) {
    assert_eq!(summary.total_count, total_count);
    assert_eq!(summary.total_tokens, total_tokens);
    assert_f64_close(summary.total_cost, expected_cost);
    let breakdown = summary.usage_breakdown.expect("usage breakdown");
    assert_f64_close(breakdown.costs.expect("usage costs").unknown, expected_cost);
    let model = breakdown
        .models
        .iter()
        .find(|model| model.model == "gpt-5" && model.reasoning_effort.as_deref() == Some("high"))
        .expect("gpt-5 high model group");
    assert_f64_close(
        model.costs.as_ref().expect("model costs").unknown,
        expected_cost,
    );
}

async fn materialized_boundary_fixture() -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse("https://api.openai.com/").unwrap();
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let archived_at = archived_start_partial_hour_at();
    let mut row = archived_breakdown_row(
        901,
        "summary-materialized-boundary-over-48h",
        &archived_at,
        20,
        0.20,
        150.0,
    );
    row.payload =
        Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high","upstreamAccountId":42}"#);
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-materialized-boundary-over-48h",
        &[row],
    )
    .await;
    sqlx::query("UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path LIKE '%summary-materialized-boundary-over-48h%'")
        .execute(&state.pool).await.expect("mark boundary archive materialized");
    let bucket = invocation_bucket_start_epoch(&archived_at).expect("boundary bucket");
    sqlx::query("INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 0, 1, 20, 0.20, 1, 20, 0.20)")
        .bind(bucket).bind(SOURCE_PROXY).execute(&state.pool).await
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
    state
}

async fn assert_materialized_boundary_views(state: Arc<AppState>) {
    for window in ["7d", "previous7d"] {
        for upstream_account_id in [None, Some(42)] {
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
                    .expect("breakdown")
                    .costs
                    .expect("costs")
                    .unknown,
                0.20,
            );
        }
    }
}

pub(super) fn archived_breakdown_row<'a>(
    id: i64,
    invoke_id: &'a str,
    occurred_at: &'a str,
    total_tokens: i64,
    cost: f64,
    ttfb_ms: f64,
) -> SeedInvocationArchiveBatchRow<'a> {
    SeedInvocationArchiveBatchRow {
        id,
        invoke_id,
        occurred_at,
        source: SOURCE_PROXY,
        status: "success",
        total_tokens,
        cost,
        ttfb_ms: Some(ttfb_ms),
        payload: Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#),
        detail_level: DETAIL_LEVEL_FULL,
        error_message: None,
        failure_kind: None,
        failure_class: Some("none"),
        is_actionable: Some(0),
    }
}

async fn seed_boundary_partial_replay(state: &AppState) {
    let archived_at = archived_start_partial_hour_at();
    let row = archived_breakdown_row(
        801,
        "summary-7d-archived-boundary-partial-progress",
        &archived_at,
        20,
        0.20,
        150.0,
    );
    let path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-7d-archived-boundary-partial-progress",
        &[row],
    )
    .await;
    let record = breakdown_rollup_record(801, archived_at, 20, 0.20, 150.0);
    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[record],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed boundary breakdown rollup");
    tx.commit().await.expect("commit breakdown rollup tx");
    insert_archive_progress(&state.pool, &path, HOURLY_ROLLUP_DATASET_INVOCATIONS, 801).await;
}

pub(super) fn breakdown_rollup_record(
    id: i64,
    occurred_at: String,
    total_tokens: i64,
    cost: f64,
    ttfb_ms: f64,
) -> InvocationHourlySourceRecord {
    let mut record =
        super::part_13::hourly_source_record(id, occurred_at, Some("success"), total_tokens, cost);
    record.output_tokens = Some(total_tokens);
    record.failure_class = Some("none".to_string());
    record.is_actionable = Some(0);
    record.payload = Some(r#"{"responseModel":"gpt-5","reasoningEffort":"high"}"#.to_string());
    record.t_upstream_ttfb_ms = Some(ttfb_ms);
    record
}

pub(super) async fn insert_archive_progress(
    pool: &SqlitePool,
    path: &std::path::Path,
    dataset: &str,
    cursor: i64,
) {
    sqlx::query("INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at) VALUES (?1, ?2, ?3, datetime('now'))")
        .bind(dataset).bind(path.to_string_lossy().to_string()).bind(cursor)
        .execute(pool).await.expect("insert archive replay progress");
}

pub(super) async fn seed_two_breakdown_rows(
    state: &AppState,
    scenario: &str,
    first_id: i64,
) -> (std::path::PathBuf, String) {
    let first_at = format_naive(
        (Utc::now() - ChronoDuration::days(7) + ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let second_at = format_naive(
        (Utc::now() - ChronoDuration::days(7)
            + ChronoDuration::hours(2)
            + ChronoDuration::minutes(10))
        .with_timezone(&Shanghai)
        .naive_local(),
    );
    let first_invoke_id = format!("{scenario}-a");
    let second_invoke_id = format!("{scenario}-b");
    let rows = [
        archived_breakdown_row(first_id, &first_invoke_id, &first_at, 10, 0.10, 100.0),
        archived_breakdown_row(first_id + 1, &second_invoke_id, &second_at, 20, 0.20, 120.0),
    ];
    let path =
        seed_invocation_archive_batch_with_details(&state.pool, &state.config, scenario, &rows)
            .await;
    (path, first_at)
}

async fn seed_partial_breakdown_rollup(state: &AppState, occurred_at: String) {
    let record = breakdown_rollup_record(701, occurred_at, 10, 0.10, 100.0);
    let mut tx = state.pool.begin().await.expect("begin breakdown rollup tx");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[record],
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await
    .expect("seed partially materialized breakdown rollup");
    tx.commit().await.expect("commit breakdown rollup tx");
}

use super::*;
