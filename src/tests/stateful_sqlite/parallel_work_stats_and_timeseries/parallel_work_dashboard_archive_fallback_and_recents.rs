use super::*;

#[tokio::test]
async fn dashboard_activity_summary_only_uses_rollups_when_materialized_archive_file_is_missing() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Recovered Rollup")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert dashboard activity recovered rollup account");

    let archived_hour_local = Utc::now()
        .with_timezone(&Shanghai)
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .expect("valid local noon")
        .checked_sub_signed(ChronoDuration::days(1))
        .expect("valid archived dashboard summary hour");
    let archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("valid archived dashboard summary invocation time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "dashboard-summary-only-missing-materialized-archive",
        &[(
            95_001_i64,
            "dashboard-summary-only-missing-materialized",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            250_i64,
            0.25_f64,
            Some(100.0),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
            SET historical_rollups_materialized_at = datetime('now'), \
                coverage_start_at = ?2, \
                coverage_end_at = ?3 \
          WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(format_naive(archived_hour_local))
    .bind(format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("valid dashboard summary archive coverage end"),
    ))
    .execute(&state.pool)
    .await
    .expect("mark dashboard summary archive as materialized");
    fs::remove_file(&archive_path).expect("remove materialized archive raw file");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_at)
        .expect("dashboard summary rollup bucket start should be derivable");
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(250_i64)
    .bind(0.25_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed dashboard summary materialized rollup row");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch,
            source,
            upstream_account_id,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_cost,
            non_success_cost,
            total_latency_sample_count,
            total_latency_sum_ms,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(42_i64)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(250_i64)
    .bind(200_i64)
    .bind(50_i64)
    .bind(25_i64)
    .bind(0.25_f64)
    .bind(0.0_f64)
    .bind(1_i64)
    .bind(500.0_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .execute(&state.pool)
    .await
    .expect("seed dashboard account materialized rollup row");

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch summary-only dashboard activity from materialized rollup");

    assert_eq!(response.summary.stats.total_count, 1);
    assert_eq!(response.summary.stats.success_count, 1);
    assert_eq!(response.summary.stats.failure_count, 0);
    assert_eq!(response.summary.stats.total_tokens, 250);
    assert_f64_close(response.summary.stats.total_cost, 0.25);
    assert!(response.summary.stats.usage_breakdown.is_none());
    assert!(response.summary.stats.non_success_tokens.is_none());

    let Json(full_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch full dashboard activity from materialized rollup");

    assert_eq!(full_response.summary.stats.total_count, 1);
    assert_eq!(full_response.summary.stats.success_count, 1);
    assert_eq!(full_response.summary.stats.failure_count, 0);
    assert_eq!(full_response.summary.stats.total_tokens, 250);
    assert_f64_close(full_response.summary.stats.total_cost, 0.25);
    assert!(full_response.summary.stats.usage_breakdown.is_none());
    assert!(full_response.summary.stats.non_success_tokens.is_none());
    let full_accounts = full_response
        .accounts
        .as_ref()
        .expect("full dashboard accounts should be included");
    let recovered_account = full_accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("full dashboard should include recovered rollup account");
    assert_eq!(recovered_account.display_name, "Recovered Rollup");
    assert_eq!(recovered_account.request_count, 1);
    assert_eq!(recovered_account.success_count, 1);
    assert_eq!(recovered_account.total_tokens, 250);
    assert_f64_close(recovered_account.total_cost, 0.25);
    assert_eq!(
        full_accounts
            .iter()
            .map(|account| account.request_count)
            .sum::<i64>(),
        full_response.summary.stats.total_count,
    );

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity from materialized rollup");
    let upstream_account = activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 42)
        .expect("upstream activity should include recovered rollup account");
    assert_eq!(upstream_account.request_count, 1);
    assert_eq!(upstream_account.success_count, 1);
    assert_eq!(upstream_account.total_tokens, 250);
    assert_f64_close(upstream_account.total_cost, 0.25);
}

#[tokio::test]
async fn dashboard_activity_rollup_fallback_preserves_account_attribution_for_partial_hour_ranges()
{
    let mut config = test_config();
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Partial Hour Recovery")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert partial-hour recovery account");

    let range_window =
        resolve_range_window("7d", Shanghai).expect("7d dashboard range should resolve");
    let full_hour_end_epoch = align_bucket_epoch(range_window.end.timestamp(), 3_600, 0);
    let full_hour_start_epoch = full_hour_end_epoch - 3_600;
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .expect("valid full-hour start");
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .expect("valid full-hour end");
    let skipped_start = full_hour_start - ChronoDuration::minutes(10);
    let after_full_hour_secs = (range_window.end.timestamp() - full_hour_end_epoch).max(2);
    let skipped_end = full_hour_end + ChronoDuration::seconds(after_full_hour_secs.min(600));
    assert!(skipped_start >= range_window.start);
    assert!(skipped_end <= range_window.end);

    let before_edge_at = full_hour_start - ChronoDuration::minutes(5);
    let after_edge_at = full_hour_end + ChronoDuration::seconds((after_full_hour_secs / 2).max(1));
    for (id, invoke_id, occurred_at) in [
        (
            96_001_i64,
            "dashboard-partial-hour-before-edge",
            before_edge_at,
        ),
        (
            96_002_i64,
            "dashboard-partial-hour-after-edge",
            after_edge_at,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(20_i64)
        .bind(0.02_f64)
        .bind(
            json!({
                "promptCacheKey": format!("pck-{invoke_id}"),
                "upstreamAccountId": 42_i64,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert partial-hour live edge invocation");
    }

    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "dashboard-partial-hour-materialized-gap",
        &[(
            96_100_i64,
            "dashboard-partial-hour-materialized-placeholder",
            format_naive(
                (full_hour_start + ChronoDuration::minutes(5))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            )
            .as_str(),
            SOURCE_PROXY,
            "success",
            200_i64,
            0.20_f64,
            Some(100.0_f64),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
            SET historical_rollups_materialized_at = datetime('now'), \
                coverage_start_at = ?2, \
                coverage_end_at = ?3 \
          WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(format_naive(
        skipped_start.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        skipped_end.with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("mark partial-hour archive as materialized");
    fs::remove_file(&archive_path).expect("remove partial-hour materialized archive raw file");

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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(full_hour_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(400_i64)
    .bind(0.40_f64)
    .bind(2_i64)
    .bind(200.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,2,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed partial-hour dashboard summary rollup row");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch,
            source,
            upstream_account_id,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_cost,
            non_success_cost,
            total_latency_sample_count,
            total_latency_sum_ms,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
    )
    .bind(full_hour_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(42_i64)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(400_i64)
    .bind(320_i64)
    .bind(80_i64)
    .bind(40_i64)
    .bind(0.40_f64)
    .bind(0.0_f64)
    .bind(2_i64)
    .bind(1_000.0_f64)
    .bind(2_i64)
    .bind(200.0_f64)
    .bind(100.0_f64)
    .execute(&state.pool)
    .await
    .expect("seed partial-hour dashboard account rollup row");

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch dashboard activity with partial-hour rollup fallback");

    assert_eq!(response.summary.stats.total_count, 4);
    assert_eq!(response.summary.stats.success_count, 4);
    assert_eq!(response.summary.stats.total_tokens, 440);
    assert_f64_close(response.summary.stats.total_cost, 0.44);

    let accounts = response
        .accounts
        .expect("dashboard activity should include accounts");
    let recovered_account = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("partial-hour fallback should remain on the source account");
    assert_eq!(recovered_account.request_count, 4);
    assert_eq!(recovered_account.success_count, 4);
    assert_eq!(recovered_account.total_tokens, 440);
    assert_f64_close(recovered_account.total_cost, 0.44);
    assert!(
        accounts
            .iter()
            .filter(|account| account.is_unassigned)
            .all(|account| account.request_count == 0 && account.total_tokens == 0)
    );

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity with partial-hour rollup fallback");
    let upstream_account = activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 42)
        .expect("upstream activity should keep the recovered account attribution");
    assert_eq!(upstream_account.request_count, 4);
    assert_eq!(upstream_account.success_count, 4);
    assert_eq!(upstream_account.total_tokens, 440);
    assert_f64_close(upstream_account.total_cost, 0.44);
}

#[tokio::test]
async fn dashboard_activity_recent_excludes_archived_rows_for_all_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Archive Overlap")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert recent archive overlap account");

    let range_start = Utc::now() - ChronoDuration::days(1);
    let range_end = range_start + ChronoDuration::minutes(10);
    let live_overlap_at = format_naive(
        (range_start + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let live_selected_at = format_naive(
        (range_start + ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_newer_at = format_naive(
        (range_start + ChronoDuration::minutes(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (id, invoke_id, occurred_at, total_tokens) in [
        (
            91_001_i64,
            "dashboard-recent-live-overlap",
            live_overlap_at.as_str(),
            10_i64,
        ),
        (
            91_002_i64,
            "dashboard-recent-live-selected",
            live_selected_at.as_str(),
            20_i64,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
        .bind(0.01_f64)
        .bind(
            json!({
                "promptCacheKey": format!("pck-{invoke_id}"),
                "upstreamAccountId": 42_i64,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert live recent archive overlap invocation");
    }

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-recent-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 91_001_i64,
            invoke_id: "dashboard-recent-live-overlap",
            occurred_at: archive_newer_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.01_f64,
            ttfb_ms: None,
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-recent-live-overlap","upstreamAccountId":42}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: range_start.to_rfc3339(),
            range_end: range_end.to_rfc3339(),
            snapshot_id: range_end.timestamp_millis(),
            recent_limit: Some(1),
        }),
    )
    .await
    .expect("fetch dashboard activity recent with overlapping archive row");
    let account = response
        .accounts
        .iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("recent response should include account");
    assert_eq!(account.recent_invocations.len(), 1);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "dashboard-recent-live-selected"
    );
}

#[tokio::test]
async fn dashboard_activity_recent_chunks_large_preview_hydration_id_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let range_start = Utc::now() - ChronoDuration::hours(1);
    let range_end = range_start + ChronoDuration::hours(2);
    for account_id in 1_i64..=64_i64 {
        for invocation_index in 0_i64..16_i64 {
            let occurred_at = format_naive(
                (range_start + ChronoDuration::seconds(account_id * 20_i64 + invocation_index))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            );
            let invoke_id = format!("dashboard-large-preview-{account_id}-{invocation_index}");
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
            )
            .bind(93_000_i64 + account_id * 100_i64 + invocation_index)
            .bind(invoke_id.as_str())
            .bind(occurred_at.as_str())
            .bind(SOURCE_PROXY)
            .bind("success")
            .bind(1_i64)
            .bind(0.01_f64)
            .bind(
                json!({
                    "promptCacheKey": format!("pck-{invoke_id}"),
                    "upstreamAccountId": account_id,
                })
                .to_string(),
            )
            .bind("{}")
            .execute(&state.pool)
            .await
            .expect("insert large preview hydration invocation");
        }
    }

    let Json(response) = fetch_dashboard_activity_recent(
        State(state.clone()),
        Query(DashboardActivityRecentQuery {
            range_start: range_start.to_rfc3339(),
            range_end: range_end.to_rfc3339(),
            snapshot_id: range_end.timestamp_millis(),
            recent_limit: Some(16),
        }),
    )
    .await
    .expect("fetch dashboard activity recent with chunked preview hydration");
    assert_eq!(response.accounts.len(), 64);
    assert!(
        response
            .accounts
            .iter()
            .all(|account| account.recent_invocations.len() == 16)
    );
}

#[tokio::test]
async fn dashboard_activity_account_recent_excludes_archived_rows_for_all_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Archive Overlap")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert dashboard account recent archive overlap account");

    let base_utc = Utc::now() - ChronoDuration::minutes(30);
    let live_overlap_at = format_naive(base_utc.with_timezone(&Shanghai).naive_local());
    let live_selected_at = format_naive(
        (base_utc + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_newer_at = format_naive(
        (base_utc + ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (id, invoke_id, occurred_at, total_tokens) in [
        (
            94_001_i64,
            "dashboard-account-recent-live-overlap",
            live_overlap_at.as_str(),
            10_i64,
        ),
        (
            94_002_i64,
            "dashboard-account-recent-live-selected",
            live_selected_at.as_str(),
            20_i64,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
        .bind(0.01_f64)
        .bind(
            json!({
                "promptCacheKey": format!("pck-{invoke_id}"),
                "upstreamAccountId": 42_i64,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert dashboard account recent archive overlap live invocation");
    }

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-account-recent-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 94_001_i64,
            invoke_id: "dashboard-account-recent-live-overlap",
            occurred_at: archive_newer_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.01_f64,
            ttfb_ms: None,
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-account-recent-live-overlap","upstreamAccountId":42}"#,
            ),
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
            recent_limit: Some(1),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(true),
        }),
    )
    .await
    .expect("fetch dashboard activity with account recent archive/live overlap");
    let account = response
        .accounts
        .expect("dashboard activity should include accounts")
        .into_iter()
        .find(|account| account.account_key == "upstream:42")
        .expect("dashboard activity should include overlap account");
    assert_eq!(account.recent_invocations.len(), 1);
    assert_eq!(
        account.recent_invocations[0].invoke_id,
        "dashboard-account-recent-live-selected"
    );
}

#[tokio::test]
async fn dashboard_activity_progressive_summary_keeps_archived_account_aggregates() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    let archived_hour = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(3))
    .with_minute(0)
    .and_then(|value| value.with_second(0))
    .expect("valid archived hour");
    let success_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let failure_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("archived failure time"),
    );
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-aggregate",
        &[
            SeedInvocationArchiveBatchRow {
                id: 91_001,
                invoke_id: "dashboard-progressive-archive-success",
                occurred_at: success_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10,
                cost: 0.10,
                ttfb_ms: Some(100.0),
                payload: Some(r#"{"upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 91_002,
                invoke_id: "dashboard-progressive-archive-failure",
                occurred_at: failure_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20,
                cost: 0.20,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1),
            },
        ],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch progressive dashboard activity with archived aggregates");
    let account = response
        .accounts
        .expect("accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("archived account summary");
    assert_eq!(account.request_count, 2);
    assert_eq!(account.success_count, 1);
    assert_eq!(account.failure_count, 1);
    assert_eq!(account.total_tokens, 30);
    assert_f64_close(account.total_cost, 0.30);
    assert_eq!(
        account.latest_conversation_created_at.as_deref(),
        Some(failure_at.as_str())
    );
    assert_eq!(
        account.last_invocation_at.as_deref(),
        Some(failure_at.as_str())
    );
    assert!(account.recent_invocations.is_empty());

    let Json(compatible_response) = fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: None,
        }),
    )
    .await
    .expect("fetch compatible dashboard activity with archived recent rows");
    let compatible_account = compatible_response
        .accounts
        .expect("compatible accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("compatible archived account summary");
    assert_eq!(compatible_account.request_count, 2);
    assert_eq!(compatible_account.total_tokens, 30);
    assert_eq!(compatible_account.recent_invocations.len(), 2);
}

#[tokio::test]
async fn upstream_account_activity_does_not_populate_dashboard_snapshot_cache() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Cache Isolation")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert cache isolation account");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(97_001_i64)
    .bind("upstream-cache-isolation")
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(123_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-upstream-cache-isolation",
            "upstreamAccountId": 42_i64,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert cache isolation invocation");

    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        assert!(cache.entries.is_empty());
        assert!(cache.in_flight.is_empty());
    }

    let Json(response) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity without dashboard cache coupling");
    assert_eq!(response.accounts.len(), 1);
    assert_eq!(response.accounts[0].upstream_account_id, 42_i64);

    {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        assert!(cache.entries.is_empty());
        assert!(cache.in_flight.is_empty());
    }
}

#[tokio::test]
async fn dashboard_activity_progressive_summary_keeps_archive_created_at_across_batches() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    let archived_hour = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(3))
    .with_minute(0)
    .and_then(|value| value.with_second(0))
    .expect("valid archived hour");
    let first_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived time"),
    );
    let second_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::hours(5))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("second archived time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-created-at-early",
        &[SeedInvocationArchiveBatchRow {
            id: 92_001,
            invoke_id: "dashboard-progressive-archive-created-at-early",
            occurred_at: first_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10,
            cost: 0.10,
            ttfb_ms: Some(100.0),
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-progressive-archive-created-at","upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-progressive-archive-created-at-late",
        &[SeedInvocationArchiveBatchRow {
            id: 92_002,
            invoke_id: "dashboard-progressive-archive-created-at-late",
            occurred_at: second_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 20,
            cost: 0.20,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"promptCacheKey":"pck-dashboard-progressive-archive-created-at","upstreamAccountId":42,"upstreamAccountName":"Archive Alpha"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch progressive dashboard activity with cross-batch created_at");
    let account = response
        .accounts
        .expect("accounts included")
        .into_iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("archived account summary");
    assert_eq!(
        account.latest_conversation_created_at.as_deref(),
        Some(first_at.as_str())
    );
    assert_eq!(
        account.last_invocation_at.as_deref(),
        Some(second_at.as_str())
    );
}

#[tokio::test]
async fn upstream_account_activity_uses_rollup_created_at_when_working_set_row_is_missing() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pool Alpha")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    let now = Utc::now();
    let rollup_bucket_start = now - ChronoDuration::seconds(3);
    let expected_created_at =
        format_naive(rollup_bucket_start.with_timezone(&Shanghai).naive_local());
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        rollup_bucket_start,
        "pck-upstream-history-only",
        1,
    )
    .await;

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
    .bind(9_100_i64)
    .bind("upstream-history-only")
    .bind(format_naive(now.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind("completed")
    .bind(120_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-upstream-history-only",
            "upstreamAccountId": 42,
            "upstreamAccountName": "Pool Alpha"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert upstream account invocation");

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity");

    assert_eq!(activity.accounts.len(), 1);
    assert_eq!(
        activity.accounts[0]
            .latest_conversation_created_at
            .as_deref(),
        Some(expected_created_at.as_str())
    );
}

#[tokio::test]
async fn upstream_account_activity_all_scope_uses_actual_created_at_for_mixed_source_conversations()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pool Alpha")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    let now = Utc::now();
    let all_created_bucket_start = now - ChronoDuration::seconds(4);
    let proxy_created_bucket_start = now - ChronoDuration::seconds(3);
    let all_created_at = format_naive(
        all_created_bucket_start
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let proxy_created_at = format_naive(
        proxy_created_bucket_start
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let last_activity_at = format_naive(now.with_timezone(&Shanghai).naive_local());

    insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
        &state.pool,
        all_created_bucket_start,
        "pck-upstream-mixed",
        SOURCE_XY,
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
        &state.pool,
        proxy_created_bucket_start,
        "pck-upstream-mixed",
        SOURCE_PROXY,
        1,
    )
    .await;
    insert_prompt_cache_working_set_live_row(
        &state.pool,
        "pck-upstream-mixed",
        &all_created_at,
        &last_activity_at,
        Some(&proxy_created_at),
        Some(&last_activity_at),
        true,
    )
    .await;

    for (id, invoke_id, source, occurred_at) in [
        (
            9_200_i64,
            "upstream-mixed-all",
            SOURCE_XY,
            format_naive(
                (now - ChronoDuration::seconds(1))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
        ),
        (
            9_201_i64,
            "upstream-mixed-proxy",
            SOURCE_PROXY,
            format_naive(now.with_timezone(&Shanghai).naive_local()),
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
        .bind(occurred_at)
        .bind(source)
        .bind("completed")
        .bind(120_i64)
        .bind(0.12_f64)
        .bind(
            json!({
                "promptCacheKey": "pck-upstream-mixed",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Pool Alpha"
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert mixed-source upstream account invocation");
    }

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch mixed-source upstream account activity");

    assert_eq!(activity.accounts.len(), 1);
    assert_eq!(
        activity.accounts[0]
            .latest_conversation_created_at
            .as_deref(),
        Some(all_created_at.as_str())
    );
}

#[tokio::test]
async fn upstream_account_activity_uses_pool_attempt_account_for_running_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(77_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pool Fallback")
    .bind("Primary")
    .bind("pro")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert fallback upstream account");

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
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(7_700_i64)
    .bind("pool-running-selected-before-payload-update-previous-failed")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-pool-fallback",
            "routeMode": "pool",
            "upstreamAccountId": 77
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert previous failed fallback account invocation");

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
    .bind(7_701_i64)
    .bind("pool-running-selected-before-payload-update")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(0_i64)
    .bind(0.0_f64)
    .bind(json!({ "promptCacheKey": "pck-pool-fallback", "routeMode": "pool" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert running invocation without payload upstream account");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            status,
            phase,
            created_at
        )
        VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, ?5, 1, 1, 0, '127.0.0.1', ?2, 'running', 'streaming_response', datetime('now'))
        "#,
    )
    .bind("pool-running-selected-before-payload-update")
    .bind(&occurred_at)
    .bind("pck-pool-fallback")
    .bind(77_i64)
    .bind("route-pool-fallback")
    .execute(&state.pool)
    .await
    .expect("insert selected pool attempt for running invocation");
    state
        .proxy_runtime_invocations
        .upsert(crate::api::ApiInvocation {
            id: 7_701_i64,
            invoke_id: "pool-running-selected-before-payload-update".to_string(),
            occurred_at: occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: Some("gpt-5".to_string()),
            request_model: Some("gpt-5".to_string()),
            response_model: Some("gpt-5".to_string()),
            input_tokens: Some(0_i64),
            output_tokens: Some(0_i64),
            cache_input_tokens: Some(0_i64),
            reported_cache_write_tokens: None,
            reasoning_tokens: Some(0_i64),
            reasoning_effort: None,
            total_tokens: Some(0_i64),
            cost: Some(0.0_f64),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: Some(0_i64),
            status: Some("running".to_string()),
            live_phase: None,
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: Some("127.0.0.1".to_string()),
            prompt_cache_key: Some("pck-pool-fallback".to_string()),
            sticky_key: Some("pck-pool-fallback".to_string()),
            route_mode: Some("pool".to_string()),
            upstream_account_id: None,
            upstream_account_name: None,
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: None,
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            price_version: None,
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(120.0_f64),
            first_token_ms: Some(120.0_f64),
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: occurred_at.clone(),
        });

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream activity with running fallback");

    assert_eq!(activity.accounts.len(), 1);
    let account = activity
        .accounts
        .first()
        .expect("fallback activity account");
    assert_eq!(account.upstream_account_id, 77);
    assert_eq!(account.display_name, "Pool Fallback");
    assert_eq!(account.request_count, 1);
    assert_eq!(account.failure_count, 1);
    assert_eq!(account.total_tokens, 0);
    assert_f64_close(account.total_cost, 0.0);
    assert_eq!(account.tokens_per_minute, Some(0.0));
    assert_eq!(account.spend_rate, Some(0.0));
    assert_eq!(account.in_progress_invocation_count, Some(1));
    let account_phase_counts = account
        .in_progress_phase_counts
        .expect("account should include live phase counts");
    assert_eq!(account_phase_counts.queued, 0);
    assert_eq!(account_phase_counts.requesting, 0);
    assert_eq!(account_phase_counts.responding, 1);
    assert_eq!(account.retry_invocation_count, Some(1));
    let recent_invocation = account
        .recent_invocations
        .first()
        .expect("fallback recent invocation");
    assert_eq!(
        recent_invocation.invoke_id,
        "pool-running-selected-before-payload-update"
    );
    assert_eq!(recent_invocation.live_phase.as_deref(), Some("responding"));

    let mut runtime_terminal = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == "pool-running-selected-before-payload-update")
        .expect("running fallback runtime record");
    runtime_terminal.status = Some("success".to_string());
    runtime_terminal.live_phase = None;
    state
        .proxy_runtime_invocations
        .upsert_terminal(runtime_terminal);
    let Json(terminal_activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream activity with terminal fallback");
    let terminal_account = terminal_activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 77)
        .expect("terminal fallback activity account");
    let terminal_recent = terminal_account
        .recent_invocations
        .iter()
        .filter(|row| row.invoke_id == "pool-running-selected-before-payload-update")
        .collect::<Vec<_>>();
    assert_eq!(terminal_recent.len(), 1);
    assert_eq!(terminal_recent[0].status, "success");

    let Json(account_summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(77),
        }),
    )
    .await
    .expect("fetch fallback account summary");
    assert_eq!(account_summary.in_progress_conversation_count, Some(0));
    assert_eq!(
        account_summary.in_progress_retry_conversation_count,
        Some(0)
    );
    let account_summary_phase_counts = account_summary
        .in_progress_phase_counts
        .expect("account summary should include live phase counts");
    assert_eq!(account_summary_phase_counts.queued, 0);
    assert_eq!(account_summary_phase_counts.requesting, 0);
    assert_eq!(account_summary_phase_counts.responding, 0);
}

#[tokio::test]
async fn upstream_account_activity_keeps_memory_runtime_rows_range_bounded() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(88_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Runtime Pool")
    .bind("Primary")
    .bind("team")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert runtime upstream account");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(89_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Runtime Pool Retry")
    .bind("Primary")
    .bind("team")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert retry runtime upstream account");

    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        prompt_cache_key: Some("pck-runtime-activity".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let record = build_running_proxy_capture_record(
        "runtime-account-activity-running",
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.88"),
        None,
        Some("pck-runtime-activity"),
        true,
        Some(88),
        Some("Runtime Pool"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("runtime-proxy"),
        Some(2),
        Some(1),
        None,
        None,
        11.0,
        2.0,
        33.0,
        44.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("store runtime account activity snapshot in memory");

    let persisted_running_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'runtime-account-activity-running'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count persisted memory-only running row");
    assert_eq!(persisted_running_count, 0);
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let retry_record = build_running_proxy_capture_record(
        "runtime-account-activity-running",
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("203.0.113.88"),
        None,
        Some("pck-runtime-activity"),
        true,
        Some(89),
        Some("Runtime Pool Retry"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("runtime-proxy"),
        Some(3),
        Some(2),
        None,
        None,
        12.0,
        3.0,
        34.0,
        45.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, retry_record)
        .await
        .expect("retry runtime account activity snapshot should update memory only");

    let Json(activity) = fetch_upstream_account_activity(
        State(state),
        Query(UpstreamAccountActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(4),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity with memory overlay");

    assert_eq!(activity.accounts.len(), 1);
    let account = activity.accounts.first().expect("runtime activity account");
    assert_eq!(account.upstream_account_id, 89);
    assert_eq!(account.display_name, "Runtime Pool Retry");
    assert_eq!(account.request_count, 0);
    assert_eq!(account.success_count, 0);
    assert_eq!(account.failure_count, 0);
    assert_eq!(account.total_tokens, 0);
    assert_eq!(account.total_cost, 0.0);
    assert_eq!(account.recent_invocations.len(), 0);
    assert_eq!(account.in_progress_invocation_count, Some(1));
    assert_eq!(account.retry_invocation_count, Some(1));
}
