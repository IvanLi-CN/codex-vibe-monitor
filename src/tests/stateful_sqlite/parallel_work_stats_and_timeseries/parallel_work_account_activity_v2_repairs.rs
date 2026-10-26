use super::*;

#[tokio::test]
async fn account_activity_v2_sub_hour_range_uses_one_exact_boundary() {
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
async fn account_activity_v2_upgrade_repair_does_not_double_count_new_live_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 2 * 3_600;
    let occurred_at = format_naive(
        Utc.timestamp_opt(bucket_epoch + 10 * 60, 0)
            .single()
            .expect("completed repair bucket")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    for (id, invoke_id) in [(1_i64, "v2-upgrade-existing"), (2_i64, "v2-upgrade-new")] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, detail_level,
                total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, 'success', ?5, 10, 0.01, ?6, '{}')
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(DETAIL_LEVEL_FULL)
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert v2 upgrade source row");
    }
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, 1, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .execute(&state.pool)
    .await
    .expect("seed pre-v2 shared live cursor");

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
async fn account_activity_v2_upgrade_repair_rebuilds_false_covered_hours() {
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
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, detail_level,
                total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, 'success', ?5, 10, 0.01, ?6, '{}')
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(DETAIL_LEVEL_FULL)
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert false-covered source row");
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
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target, bucket_start_epoch, source, materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(bucket_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&state.pool)
    .await
    .expect("mark sparse v2 hour covered");
    for dataset in [
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
            VALUES (?1, 2, datetime('now'))
            "#,
        )
        .bind(dataset)
        .execute(&state.pool)
        .await
        .expect("seed caught-up v2 cursor");
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
async fn account_activity_v2_covered_hour_falls_back_for_late_live_rows() {
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
async fn account_activity_v2_partial_merge_reads_only_covered_hours_from_rollup() {
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
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost,
                cache_input_tokens, t_total_ms, t_upstream_ttfb_ms, payload, raw_response
            )
            VALUES (?1, ?2, ?3, 'success', ?4, ?5, 10, 100, 20, ?6, '{}')
            "#,
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
        .expect("insert account activity invocation");
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
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target, bucket_start_epoch, source, materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(covered_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&state.pool)
    .await
    .expect("mark covered v2 hour");
    let repair_cursor =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("load partial-merge repair cursor");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        "#,
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET)
    .bind(repair_cursor)
    .execute(&state.pool)
    .await
    .expect("seed partial-merge repair cursor");

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
async fn account_activity_v2_priority_repair_materializes_active_closed_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let repair_hour_epoch = current_hour_epoch - 2 * 3_600;
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, 1, datetime('now'))
        "#,
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    let occurred_at = Utc
        .timestamp_opt(repair_hour_epoch + 10 * 60, 0)
        .single()
        .expect("priority repair invocation time");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-priority-repair', ?1, ?2, 'failed', ?3, 45, 0.45, ?4, '{}')
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
    .expect("insert priority repair source row");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-priority-running', ?1, ?2, 'running', ?3, 99, 0.99, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        (occurred_at + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert in-flight row in priority repair bucket");
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
    let covered = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(repair_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load priority repair marker");
    assert_eq!(covered, 1);
    let current_hour_covered = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(current_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load current-hour marker");
    assert_eq!(current_hour_covered, 0);
    let rollup = sqlx::query_as::<_, (i64, i64, i64, f64)>(
        r#"
        SELECT activity_v2_request_count,
               activity_v2_failure_count,
               activity_v2_non_success_tokens,
               activity_v2_total_cost
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load priority repaired rollup");
    assert_eq!(rollup.0, 1);
    assert_eq!(rollup.1, 1);
    assert_eq!(rollup.2, 45);
    assert_f64_close(rollup.3, 0.45);
    let repair_watermark = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM account_activity_v2_bucket_repair_watermarks WHERE bucket_start_epoch = ?1",
    )
    .bind(repair_hour_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load terminal-only priority repair watermark");
    assert_eq!(repair_watermark, 1);
    assert_eq!(
        replay_live_invocation_hourly_rollups(&state.pool)
            .await
            .expect("replay rows after priority repair"),
        2
    );
    let replayed_request_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT activity_v2_request_count
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load v2 request count after shared replay");
    assert_eq!(replayed_request_count, 1);
    let archived_rollup = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT activity_v2_request_count, activity_v2_total_tokens
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(archived_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved archive-derived v2 rollup");
    assert_eq!(archived_rollup, (9, 900));
}

#[tokio::test]
async fn account_activity_v2_priority_repair_skips_archive_overlapping_live_hours() {
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
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, 1, datetime('now'))
        "#,
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-overlap-live', ?1, ?2, 'success', ?3, 10, 0.10, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        (overlap_start + ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert live row overlapping archive coverage");
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
    let overlap_covered = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(overlap_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load overlap coverage marker");
    assert_eq!(overlap_covered, 0);
}

#[tokio::test]
async fn account_activity_v2_priority_repair_preserves_unknown_archive_coverage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let repair_hour_epoch = current_hour_epoch - 2 * 3_600;
    let repair_time = Utc
        .timestamp_opt(repair_hour_epoch + 10 * 60, 0)
        .single()
        .expect("repair invocation time");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-unknown-archive-live', ?1, ?2, 'success', ?3, 10, 0.10, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        repair_time.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert live row in unknown archive month");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_total_tokens
        )
        VALUES (?1, ?2, 42, 9, 900)
        "#,
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert archive-derived values with unknown coverage");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES (?1, ?2, ?3, 'unknown-coverage-sha', 1, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(
        repair_time
            .with_timezone(&Shanghai)
            .format("%Y-%m")
            .to_string(),
    )
    .bind("/tmp/account-activity-v2-unknown-coverage.sqlite.gz")
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert archive manifest without coverage bounds");

    repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("run active coverage repair");

    let rollup = sqlx::query_as::<_, (i64, i64)>(
        "SELECT activity_v2_request_count, activity_v2_total_tokens \
         FROM upstream_account_stats_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved unknown-coverage rollup");
    assert_eq!(rollup, (9, 900));
    let covered = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets \
         WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(repair_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load unknown-coverage marker");
    assert_eq!(covered, 0);
}

#[tokio::test]
async fn account_activity_v2_priority_repair_uses_indexed_archive_epoch_coverage_within_budget() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let oldest_hour_epoch = current_hour_epoch - 7 * 24 * 3_600;
    let oldest_occurred_at = Utc
        .timestamp_opt(oldest_hour_epoch + 60, 0)
        .single()
        .expect("oldest active window timestamp");

    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-priority-window-floor', ?1, ?2, 'success', ?3, 1, 0.01, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        oldest_occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert active-window floor invocation");
    sqlx::query(
        r#"
        WITH RECURSIVE
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 199
            ),
            units(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM units WHERE value < 99
            )
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at
        )
        SELECT
            'codex_invocations',
            ?1,
            '/tmp/v2-priority-coverage-' || (hundreds.value * 100 + units.value) || '.sqlite.gz',
            'fixture-sha',
            1,
            'completed',
            ?2,
            ?3
        FROM hundreds CROSS JOIN units
        "#,
    )
    .bind(
        oldest_occurred_at
            .with_timezone(&Shanghai)
            .format("%Y-%m")
            .to_string(),
    )
    .bind(format_naive(
        oldest_occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        (oldest_occurred_at + ChronoDuration::seconds(3_599))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("seed archive coverage fixture");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at
        )
        VALUES
            ('forward_proxy_attempts', ?1, '/tmp/v2-priority-other-dataset.sqlite.gz', 'other-sha', 1, 'completed', ?2, ?3),
            ('codex_invocations', ?1, '/tmp/v2-priority-pending.sqlite.gz', 'pending-sha', 1, 'pending', ?2, ?3)
        "#,
    )
    .bind(
        oldest_occurred_at
            .with_timezone(&Shanghai)
            .format("%Y-%m")
            .to_string(),
    )
    .bind(format_naive(
        oldest_occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        (oldest_occurred_at + ChronoDuration::seconds(3_599))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("seed archive coverage filter controls");
    let matching_archive_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_invocations' AND status = 'completed' AND coverage_start_epoch IS NOT NULL AND coverage_end_epoch IS NOT NULL",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count matching archive coverage fixture rows");
    assert_eq!(matching_archive_rows, 20_000);

    let mut explain_query = build_active_account_activity_v2_archive_epoch_coverage_query(
        "EXPLAIN QUERY PLAN ",
        oldest_hour_epoch,
        current_hour_epoch,
    );
    assert!(explain_query.sql().contains("coverage_start_epoch <"));
    let explain = explain_query
        .build_query_as::<(i64, i64, i64, String)>()
        .fetch_all(&state.pool)
        .await
        .expect("explain indexed archive epoch coverage query");
    let explain_details = explain
        .iter()
        .map(|(_, _, _, detail)| detail.as_str())
        .collect::<Vec<_>>();
    assert!(
        explain_details.iter().any(|detail| {
            detail.contains(
                "SEARCH archive_batches USING INDEX idx_archive_batches_invocation_coverage_epoch",
            ) && detail.contains("coverage_end_epoch")
        }),
        "unexpected archive epoch coverage plan: {explain_details:?}"
    );

    let selection_started_at = std::time::Instant::now();
    let selected = crate::select_active_account_activity_v2_priority_buckets_with_deadline(
        &state.pool,
        current_hour_epoch,
        selection_started_at,
        selection_started_at + Duration::from_secs(2),
        crate::ActiveAccountActivityV2ProgressHandlerOptions::new(None, 1_000, false),
    )
    .await
    .expect("select priority coverage against archive fixture")
    .expect("priority selection should finish within budget");
    assert_eq!(selected.len(), 2);
    assert!(
        selection_started_at.elapsed() < Duration::from_secs(2),
        "priority coverage selection exceeded its two-second budget: {:?}",
        selection_started_at.elapsed()
    );

    let outcome = repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("repair active coverage against archive fixture");
    assert_eq!(outcome.priority_bucket_count, 2);
    assert_eq!(outcome.repaired_bucket_count, 2);
}

#[tokio::test]
async fn summary_non_success_tokens_includes_unreplayed_live_tail_in_covered_hour() {
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
            invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
        )
        VALUES ('unreplayed-non-success', ?1, ?2, 'failed', 7, ?3, '{}')
        "#,
    )
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert unreplayed invocation");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_non_success_tokens
        )
        VALUES (?1, ?2, 42, 3)
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert covered v2 non-success rollup");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target, bucket_start_epoch, source, materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(bucket_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&state.pool)
    .await
    .expect("mark covered v2 hour");

    let value = load_non_success_tokens_snapshot(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        None,
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
        SummaryRangeBuildTelemetry::new(
            SummaryBuildRoute::Http,
            &SummaryWindow::Duration(ChronoDuration::days(1)),
            Some("1d"),
        ),
    )
    .await
    .expect("load non-success tokens")
    .expect("non-success tokens should be available");

    assert_eq!(value, 10);

    sqlx::query(
        r#"
        UPDATE upstream_account_stats_hourly
        SET activity_v2_non_success_tokens = 7
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("update authoritative v2 non-success rollup");
    sqlx::query(
        r#"
        INSERT INTO account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch, cursor_id, updated_at
        )
        VALUES (?1, 1, datetime('now'))
        ON CONFLICT(bucket_start_epoch) DO UPDATE SET cursor_id = excluded.cursor_id
        "#,
    )
    .bind(bucket_epoch)
    .execute(&state.pool)
    .await
    .expect("mark non-success row included by bucket recompute");

    let recomputed_value = load_non_success_tokens_snapshot(
        state.as_ref(),
        InvocationSourceScope::ProxyOnly,
        None,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(bucket_epoch, 0)
                .single()
                .expect("recomputed range start"),
            end: Utc
                .timestamp_opt(bucket_epoch + 3_600, 0)
                .single()
                .expect("recomputed range end"),
        },
        SummaryRangeBuildTelemetry::new(
            SummaryBuildRoute::Http,
            &SummaryWindow::Duration(ChronoDuration::days(1)),
            Some("1d"),
        ),
    )
    .await
    .expect("load recomputed non-success tokens")
    .expect("recomputed non-success tokens should be available");
    assert_eq!(recomputed_value, 7);
}

#[tokio::test]
async fn account_activity_v2_rollup_keeps_latest_timestamp_and_value_paired() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 3_600;
    for (offset_minutes, invoke_id, status, tokens, cost, ttfb, total_ms, failure_class) in [
        (
            5_i64,
            "v2-success-old",
            "success",
            30_i64,
            0.30_f64,
            10.0_f64,
            100.0_f64,
            "none",
        ),
        (
            25,
            "v2-success-latest",
            "success",
            40,
            0.40,
            25.0,
            250.0,
            "none",
        ),
        (
            35,
            "v2-failure",
            "failed",
            50,
            0.50,
            90.0,
            900.0,
            "service_failure",
        ),
        (45, "v2-running", "running", 60, 0.60, 95.0, 950.0, "none"),
    ] {
        let occurred_at = Utc
            .timestamp_opt(bucket_epoch + offset_minutes * 60, 0)
            .single()
            .expect("v2 invocation timestamp");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, detail_level, total_tokens, cost,
                cache_input_tokens, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
                t_upstream_ttfb_ms, t_total_ms, failure_class, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 5, 0, 0, 0, ?8, ?9, ?10, ?11, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(DETAIL_LEVEL_FULL)
        .bind(tokens)
        .bind(cost)
        .bind(ttfb)
        .bind(total_ms)
        .bind(failure_class)
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert v2 rollup source row");
    }

    let mut tx = state.pool.begin().await.expect("begin v2 rollup tx");
    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx.as_mut(), &[bucket_epoch])
        .await
        .expect("load v2 source rows");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
    )
    .await
    .expect("upsert v2 account activity");
    tx.commit().await.expect("commit v2 account activity");

    let stored = sqlx::query_as::<
        _,
        (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            f64,
            String,
            String,
            f64,
            String,
            f64,
        ),
    >(
        r#"
        SELECT
            activity_v2_request_count,
            activity_v2_success_count,
            activity_v2_failure_count,
            activity_v2_non_success_count,
            activity_v2_total_tokens,
            activity_v2_failure_tokens,
            activity_v2_failure_cost,
            activity_v2_latest_unkeyed_conversation_at,
            activity_v2_latest_first_response_at,
            activity_v2_latest_first_response_ms,
            activity_v2_latest_total_latency_at,
            activity_v2_latest_total_latency_ms
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load stored v2 account activity");
    assert_eq!(stored.0, 3);
    assert_eq!(stored.1, 2);
    assert_eq!(stored.2, 1);
    assert_eq!(stored.3, 1);
    assert_eq!(stored.4, 120);
    assert_eq!(stored.5, 50);
    assert_f64_close(stored.6, 0.50);
    let latest_success_at = format_naive(
        Utc.timestamp_opt(bucket_epoch + 25 * 60, 0)
            .single()
            .expect("latest success timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let latest_unkeyed_at = format_naive(
        Utc.timestamp_opt(bucket_epoch + 35 * 60, 0)
            .single()
            .expect("latest unkeyed terminal timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    assert_eq!(stored.7, latest_unkeyed_at);
    assert_eq!(stored.8, latest_success_at);
    assert_eq!(stored.9, 25.0);
    assert_eq!(stored.10, latest_success_at);
    assert_eq!(stored.11, 250.0);
}
