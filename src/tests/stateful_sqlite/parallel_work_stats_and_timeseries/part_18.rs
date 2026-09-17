async fn seed_v2_invocation(
    state: &AppState,
    id: Option<i64>,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    status: &str,
    tokens: i64,
    cost: f64,
) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, detail_level, total_tokens, cost, payload, raw_response) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}')",
    )
    .bind(id)
    .bind(invoke_id)
    .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind(status)
    .bind(DETAIL_LEVEL_FULL)
    .bind(tokens)
    .bind(cost)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert account activity v2 invocation");
}

async fn seed_rollup_progress(state: &AppState, dataset: &str, cursor_id: i64) {
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, ?2, datetime('now'))",
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&state.pool)
    .await
    .expect("seed hourly rollup progress");
}

async fn v2_coverage_count(state: &AppState, bucket_epoch: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets \
         WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(bucket_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load account activity v2 coverage marker")
}

async fn mark_v2_bucket_covered(state: &AppState, bucket_epoch: i64) {
    sqlx::query(
        "INSERT INTO hourly_rollup_materialized_buckets \
         (target, bucket_start_epoch, source, materialized_at) \
         VALUES (?1, ?2, ?3, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(bucket_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&state.pool)
    .await
    .expect("mark account activity v2 bucket covered");
}

async fn seed_v2_timed_invocation(
    state: &AppState,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    tokens: i64,
    cost: f64,
) {
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, cache_input_tokens, \
          t_total_ms, t_upstream_ttfb_ms, payload, raw_response) \
         VALUES (?1, ?2, ?3, 'success', ?4, ?5, 10, 100, 20, ?6, '{}')",
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(tokens)
    .bind(cost)
    .bind(r#"{"upstreamAccountId":42,"promptCacheKey":"account-v2"}"#)
    .execute(&state.pool)
    .await
    .expect("insert timed account activity v2 invocation");
}

async fn assert_priority_repair_storage(
    state: &AppState,
    repair_hour_epoch: i64,
    current_hour_epoch: i64,
    archived_hour_epoch: i64,
) {
    assert_eq!(v2_coverage_count(state, repair_hour_epoch).await, 1);
    assert_eq!(v2_coverage_count(state, current_hour_epoch).await, 0);
    let rollup = sqlx::query_as::<_, (i64, i64, i64, f64)>(
        "SELECT activity_v2_request_count, activity_v2_failure_count, \
         activity_v2_non_success_tokens, activity_v2_total_cost \
         FROM upstream_account_stats_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load priority repaired rollup");
    assert_eq!((rollup.0, rollup.1, rollup.2), (1, 1, 45));
    assert_f64_close(rollup.3, 0.45);
    let watermark = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM account_activity_v2_bucket_repair_watermarks \
         WHERE bucket_start_epoch = ?1",
    )
    .bind(repair_hour_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load terminal-only priority repair watermark");
    assert_eq!(watermark, 1);
    assert_eq!(
        replay_live_invocation_hourly_rollups(&state.pool)
            .await
            .expect("replay rows after priority repair"),
        2
    );
    let replayed = sqlx::query_scalar::<_, i64>(
        "SELECT activity_v2_request_count FROM upstream_account_stats_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load v2 request count after shared replay");
    assert_eq!(replayed, 1);
    let archived = sqlx::query_as::<_, (i64, i64)>(
        "SELECT activity_v2_request_count, activity_v2_total_tokens \
         FROM upstream_account_stats_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(archived_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved archive-derived v2 rollup");
    assert_eq!(archived, (9, 900));
}

#[tokio::test]
pub(crate) async fn dashboard_activity_summary_only_excludes_archived_rows_for_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let range_start = Utc::now() - ChronoDuration::days(1);
    let overlap_at = format_naive(
        (range_start + ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(92_001_i64)
    .bind("dashboard-summary-only-live-overlap")
    .bind(overlap_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.10_f64)
    .bind(json!({ "promptCacheKey": "pck-dashboard-summary-only-live-overlap" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live summary-only overlap invocation");

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-summary-only-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 92_001_i64,
            invoke_id: "dashboard-summary-only-live-overlap",
            occurred_at: overlap_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 100_i64,
            cost: 0.10_f64,
            ttfb_ms: None,
            payload: Some(r#"{"promptCacheKey":"pck-dashboard-summary-only-live-overlap"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "1d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch summary-only dashboard activity with archive/live overlap");
    assert_eq!(response.summary.stats.total_count, 1);
    assert_eq!(response.summary.stats.total_tokens, 100);
    assert_f64_close(response.summary.stats.total_cost, 0.10);
}

#[tokio::test]
pub(crate) async fn dashboard_activity_cache_selection_reuses_today_snapshot_within_reconcile_interval()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(first_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch first rolling dashboard activity snapshot");

    let first_range_end =
        parse_to_utc_datetime(&first_response.range_end).expect("first range end should parse");
    tokio::time::sleep(std::time::Duration::from_millis(2_100)).await;

    let Json(second_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch second rolling dashboard activity snapshot");

    let second_range_end =
        parse_to_utc_datetime(&second_response.range_end).expect("second range end should parse");
    assert!(second_range_end > first_range_end);
    assert!(second_response.snapshot_id > first_response.snapshot_id);
    let cache = state.dashboard_activity_snapshot_cache.lock().await;
    assert_eq!(cache.entries.len(), 1);
    let selection = cache
        .entries
        .keys()
        .next()
        .expect("cached dashboard selection");
    assert_eq!(selection.range, "today");
    assert!(!selection.range_anchor.is_empty());
    assert!(cache.in_flight.is_empty());
}

#[tokio::test]
pub(crate) async fn account_activity_v2_sub_hour_range_uses_one_exact_boundary() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let occurred_at = Utc
        .timestamp_opt(current_hour_epoch + 20 * 60, 0)
        .single()
        .expect("sub-hour invocation time");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-sub-hour', ?1, ?2, 'success', ?3, 25, 0.25, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert sub-hour account activity row");

    let build = load_upstream_account_activity_range_rows(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(current_hour_epoch + 10 * 60, 0)
                .single()
                .expect("sub-hour range start"),
            end: Utc
                .timestamp_opt(current_hour_epoch + 30 * 60, 0)
                .single()
                .expect("sub-hour range end"),
        },
        "dashboard",
    )
    .await
    .expect("build sub-hour account activity");
    assert_eq!(build.telemetry.boundary_tail_count, 1);
    assert_eq!(build.telemetry.raw_fallback_range_count, 1);
    let account = build
        .rows
        .iter()
        .find(|row| row.upstream_account_id == Some(42))
        .expect("sub-hour account aggregate");
    assert_eq!(account.request_count, 1);
    assert_eq!(account.total_tokens, 25);
    assert_f64_close(account.total_cost, 0.25);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_upgrade_repair_does_not_double_count_new_live_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 2 * 3_600;
    let occurred_at = Utc
        .timestamp_opt(bucket_epoch + 10 * 60, 0)
        .single()
        .expect("completed repair bucket");
    for (id, invoke_id) in [(1_i64, "v2-upgrade-existing"), (2_i64, "v2-upgrade-new")] {
        seed_v2_invocation(
            &state,
            Some(id),
            invoke_id,
            occurred_at,
            "success",
            10,
            0.01,
        )
        .await;
    }
    seed_rollup_progress(&state, HOURLY_ROLLUP_DATASET_INVOCATIONS, 1).await;

    assert_eq!(
        repair_live_invocation_account_activity_v2_once(&state.pool)
            .await
            .expect("repair first row while second row is ahead of the shared cursor"),
        1
    );
    let mut tx = state.pool.begin().await.expect("begin bucket recompute");
    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[2])
        .await
        .expect("recompute completed bucket while repair lags");
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, 2)
        .await
        .expect("advance shared cursor after bucket recompute");
    mark_hourly_rollup_bucket_materialized_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
        bucket_epoch,
        HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
    )
    .await
    .expect("preserve covered bucket while recomputed late row awaits repair");
    tx.commit().await.expect("commit bucket recompute");

    let pre_repair = load_upstream_account_activity_range_rows(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(bucket_epoch, 0)
                .single()
                .expect("pre-repair range start"),
            end: Utc
                .timestamp_opt(bucket_epoch + 3_600, 0)
                .single()
                .expect("pre-repair range end"),
        },
        "dashboard",
    )
    .await
    .expect("read recomputed covered bucket before repair catches up");
    assert_eq!(pre_repair.telemetry.covered_hour_count, 1);
    assert_eq!(
        pre_repair
            .rows
            .iter()
            .find(|row| row.upstream_account_id == Some(42))
            .expect("pre-repair account aggregate")
            .request_count,
        2
    );
    assert_eq!(
        repair_live_invocation_account_activity_v2_once(&state.pool)
            .await
            .expect("repair v2 rows through shared cursor"),
        1
    );

    let request_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COALESCE(SUM(activity_v2_request_count), 0)
        FROM upstream_account_stats_hourly
        WHERE source = ?1 AND upstream_account_id = 42
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired v2 request count");
    assert_eq!(request_count, 2);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_upgrade_repair_rebuilds_false_covered_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 2 * 3_600;
    let occurred_at = Utc
        .timestamp_opt(bucket_epoch + 10 * 60, 0)
        .single()
        .expect("false-covered invocation time");
    for (id, invoke_id) in [(1_i64, "v2-false-covered-a"), (2_i64, "v2-false-covered-b")] {
        seed_v2_invocation(
            &state,
            Some(id),
            invoke_id,
            occurred_at,
            "success",
            10,
            0.01,
        )
        .await;
    }
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_success_count,
            activity_v2_total_tokens, activity_v2_success_tokens,
            activity_v2_total_cost
        )
        VALUES (?1, ?2, 42, 1, 1, 10, 10, 0.01)
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert sparse v2 rollup row");
    mark_v2_bucket_covered(&state, bucket_epoch).await;
    for dataset in [
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    ] {
        seed_rollup_progress(&state, dataset, 2).await;
    }

    assert_eq!(
        repair_live_invocation_account_activity_v2_once(&state.pool)
            .await
            .expect("repair false-covered v2 hour"),
        2
    );
    let build = load_upstream_account_activity_range_rows(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(bucket_epoch, 0)
                .single()
                .expect("range start"),
            end: Utc
                .timestamp_opt(bucket_epoch + 3_600, 0)
                .single()
                .expect("range end"),
        },
        "dashboard",
    )
    .await
    .expect("build repaired account activity");
    assert_eq!(build.telemetry.covered_hour_count, 1);
    assert_eq!(build.telemetry.fallback_hour_count, 0);
    let account = build
        .rows
        .iter()
        .find(|row| row.upstream_account_id == Some(42))
        .expect("repaired account aggregate");
    assert_eq!(account.request_count, 2);
    assert_eq!(account.total_tokens, 20);
    assert_f64_close(account.total_cost, 0.02);

    assert_eq!(
        repair_live_invocation_account_activity_v2_once(&state.pool)
            .await
            .expect("repeat repaired v2 generation"),
        0
    );
    let request_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COALESCE(SUM(activity_v2_request_count), 0)
        FROM upstream_account_stats_hourly
        WHERE source = ?1 AND upstream_account_id = 42
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load repeated repair request count");
    assert_eq!(request_count, 2);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_covered_hour_falls_back_for_late_live_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 2 * 3_600;
    let occurred_at = Utc
        .timestamp_opt(bucket_epoch + 10 * 60, 0)
        .single()
        .expect("covered invocation time");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES (1, 'v2-covered-before-late', ?1, ?2, 'success', ?3, 10, 0.01, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert initial covered source row");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, 1, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .execute(&state.pool)
    .await
    .expect("seed initial shared cursor");
    assert_eq!(
        repair_live_invocation_account_activity_v2_once(&state.pool)
            .await
            .expect("repair initial covered row"),
        1
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES (2, 'v2-covered-late', ?1, ?2, 'success', ?3, 20, 0.02, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        (occurred_at + ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert late source row");

    let build = load_upstream_account_activity_range_rows(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(bucket_epoch, 0)
                .single()
                .expect("range start"),
            end: Utc
                .timestamp_opt(bucket_epoch + 3_600, 0)
                .single()
                .expect("range end"),
        },
        "dashboard",
    )
    .await
    .expect("build activity with late row");
    assert_eq!(build.telemetry.covered_hour_count, 1);
    assert_eq!(build.telemetry.fallback_hour_count, 0);
    let account = build
        .rows
        .iter()
        .find(|row| row.upstream_account_id == Some(42))
        .expect("late-row account aggregate");
    assert_eq!(account.request_count, 2);
    assert_eq!(account.total_tokens, 30);
    assert_f64_close(account.total_cost, 0.03);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_partial_merge_reads_only_covered_hours_from_rollup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let covered_hour_epoch = current_hour_epoch - 2 * 3_600;
    let fallback_hour_epoch = current_hour_epoch - 3_600;
    let covered_at = Utc
        .timestamp_opt(covered_hour_epoch + 10 * 60, 0)
        .single()
        .expect("covered invocation time");
    let fallback_at = Utc
        .timestamp_opt(fallback_hour_epoch + 10 * 60, 0)
        .single()
        .expect("fallback invocation time");
    for (invoke_id, occurred_at, tokens, cost) in [
        ("account-v2-covered", covered_at, 100_i64, 0.10_f64),
        ("account-v2-fallback", fallback_at, 200_i64, 0.20_f64),
    ] {
        seed_v2_timed_invocation(&state, invoke_id, occurred_at, tokens, cost).await;
    }

    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_success_count,
            activity_v2_total_tokens, activity_v2_success_tokens,
            activity_v2_cache_input_tokens, activity_v2_total_cost,
            activity_v2_first_response_sample_count, activity_v2_first_response_sum_ms,
            activity_v2_total_latency_sample_count, activity_v2_total_latency_sum_ms,
            activity_v2_last_invocation_at, activity_v2_latest_first_response_at,
            activity_v2_latest_first_response_ms, activity_v2_latest_total_latency_at,
            activity_v2_latest_total_latency_ms
        )
        VALUES (?1, ?2, 42, 1, 1, 100, 100, 10, 0.10, 1, 20, 1, 100, ?3, ?3, 20, ?3, 100)
        "#,
    )
    .bind(covered_hour_epoch)
    .bind(SOURCE_PROXY)
    .bind(format_naive(
        covered_at.with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert covered v2 rollup row");
    mark_v2_bucket_covered(&state, covered_hour_epoch).await;
    let repair_cursor =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("load partial-merge repair cursor");
    seed_rollup_progress(
        &state,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
        repair_cursor,
    )
    .await;

    let range = ExactUtcRange {
        start: Utc
            .timestamp_opt(covered_hour_epoch - 30 * 60, 0)
            .single()
            .expect("range start"),
        end: Utc
            .timestamp_opt(current_hour_epoch + 30 * 60, 0)
            .single()
            .expect("range end"),
    };
    let build = load_upstream_account_activity_range_rows(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        range,
        "dashboard",
    )
    .await
    .expect("build mixed-coverage account activity");
    assert_eq!(build.telemetry.covered_hour_count, 1);
    assert_eq!(build.telemetry.fallback_hour_count, 1);
    assert_eq!(build.telemetry.boundary_tail_count, 2);
    assert_eq!(build.telemetry.raw_fallback_range_count, 3);
    let account = build
        .rows
        .iter()
        .find(|row| row.upstream_account_id == Some(42))
        .expect("account aggregate");
    assert_eq!(account.request_count, 2);
    assert_eq!(account.success_count, 2);
    assert_eq!(account.total_tokens, 300);
    assert_f64_close(account.total_cost, 0.30);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_priority_repair_materializes_active_closed_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let repair_hour_epoch = current_hour_epoch - 2 * 3_600;
    seed_rollup_progress(
        &state,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
        1,
    )
    .await;
    let occurred_at = Utc
        .timestamp_opt(repair_hour_epoch + 10 * 60, 0)
        .single()
        .expect("priority repair invocation time");
    seed_v2_invocation(
        &state,
        None,
        "v2-priority-repair",
        occurred_at,
        "failed",
        45,
        0.45,
    )
    .await;
    seed_v2_invocation(
        &state,
        None,
        "v2-priority-running",
        occurred_at + ChronoDuration::minutes(1),
        "running",
        99,
        0.99,
    )
    .await;
    let archived_hour_epoch = current_hour_epoch - 6 * 24 * 3_600;
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_total_tokens
        )
        VALUES (?1, ?2, 42, 9, 900)
        "#,
    )
    .bind(archived_hour_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert archive-derived v2 rollup outside live coverage");

    let outcome = repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("repair active account activity coverage");

    assert_eq!(outcome.priority_bucket_count, 2);
    assert_eq!(outcome.repaired_bucket_count, 2);
    assert_priority_repair_storage(
        &state,
        repair_hour_epoch,
        current_hour_epoch,
        archived_hour_epoch,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn account_activity_v2_priority_repair_skips_archive_overlapping_live_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let overlap_hour_epoch = current_hour_epoch - 2 * 3_600;
    let overlap_start = Utc
        .timestamp_opt(overlap_hour_epoch, 0)
        .single()
        .expect("archive overlap start");
    let overlap_end = overlap_start + ChronoDuration::hours(1);
    seed_rollup_progress(
        &state,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
        1,
    )
    .await;
    seed_v2_invocation(
        &state,
        None,
        "v2-overlap-live",
        overlap_start + ChronoDuration::minutes(10),
        "success",
        10,
        0.10,
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_total_tokens
        )
        VALUES (?1, ?2, 42, 9, 900)
        "#,
    )
    .bind(overlap_hour_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert archive-derived values in overlapping hour");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at, created_at
        )
        VALUES (?1, ?2, ?3, 'overlap-sha', 1, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(
        overlap_start
            .with_timezone(&Shanghai)
            .format("%Y-%m")
            .to_string(),
    )
    .bind("/tmp/account-activity-v2-overlap.sqlite.gz")
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(format_naive(
        overlap_start.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        (overlap_end - ChronoDuration::seconds(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert overlapping archive manifest");

    repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("run active coverage repair");

    let overlap_rollup = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT activity_v2_request_count, activity_v2_total_tokens
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(overlap_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved overlap rollup");
    assert_eq!(overlap_rollup, (9, 900));
    assert_eq!(v2_coverage_count(&state, overlap_hour_epoch).await, 0);
}

use super::*;
