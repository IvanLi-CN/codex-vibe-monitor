#[tokio::test]
pub(crate) async fn enrich_window_actual_usage_for_summaries_reads_materialized_archive_usage_past_retention_cutoff()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/window-actual-usage-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = 401_i64;
    let mut summary = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );
    summary.id = account_id;
    summary.primary_window = Some(RateWindowSnapshot {
        used_percent: 12.0,
        used_text: "12% used".to_string(),
        limit_text: "3d rolling window".to_string(),
        resets_at: None,
        window_duration_mins: 60 * 24 * 3,
        actual_usage: None,
    });

    let live_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::hours(6));
    let archived_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &live_row_at,
        Some(1800),
        Some(900),
        Some(300),
        Some(3000),
        Some(0.03),
    )
    .await;
    let archived_bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_row_at).expect("archived usage bucket epoch");
    sqlx::query(
        r#"
            INSERT INTO upstream_account_usage_hourly (
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                first_seen_at,
                last_seen_at,
                updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now')
            )
            "#,
    )
    .bind(archived_bucket_start_epoch)
    .bind(account_id)
    .bind(1_i64)
    .bind(2000_i64)
    .bind(0.02_f64)
    .bind(1200_i64)
    .bind(600_i64)
    .bind(200_i64)
    .bind(&archived_row_at)
    .bind(&archived_row_at)
    .execute(&state.pool)
    .await
    .expect("insert materialized archived usage hourly row");

    let mut items = vec![summary];
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
        .await
        .expect("enrich actual usage with materialized archive usage");

    let usage = items[0]
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("primary actual usage");
    assert_eq!(usage.request_count, 2);
    assert_eq!(usage.total_tokens, 5000);
    assert_eq!(usage.input_tokens, 3000);
    assert_eq!(usage.output_tokens, 1500);
    assert_eq!(usage.cache_input_tokens, 500);
    assert_cost_close(usage.total_cost, 0.05);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_populates_upstream_account_usage_hourly_from_archive()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/upstream-account-usage-hourly-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = 587_i64;
    let archived_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    seed_window_actual_usage_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "materialize-upstream-account-usage-hourly",
        &[(
            account_id,
            archived_row_at.clone(),
            Some(1200),
            Some(600),
            Some(200),
            Some(2000),
            Some(0.02),
        )],
    )
    .await;

    let summary = materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize historical rollups");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_row_at).expect("archive bucket start");
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, f64, i64, i64, i64)>(
        r#"
            SELECT
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens
            FROM upstream_account_usage_hourly
            WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load upstream account usage hourly row");
    assert_eq!(row.2, 1);
    assert_eq!(row.3, 2000);
    assert_eq!(row.5, 1200);
    assert_eq!(row.6, 600);
    assert_eq!(row.7, 200);
    assert_cost_close(row.4, 0.02);
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_keeps_actual_usage_null_until_batch_hydrate() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Roster Usage").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 60 * 24,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: None,
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist roster usage snapshot");
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let account = response
        .items
        .into_iter()
        .find(|item| item.id == account_id)
        .expect("account in roster response");
    assert!(
        account
            .primary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .is_none(),
        "roster response should leave actual usage null until batch hydrate",
    );
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_returns_batch_actual_usage() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 60 * 24,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::days(2)),
        Some(700),
        Some(200),
        Some(100),
        Some(1000),
        Some(0.01),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");
    assert_eq!(item["accountId"], account_id);
    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 3300);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 2);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 4300);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_does_not_double_count_partial_live_rows_without_cursor()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Double Count").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::minutes(20)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage without cursor");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 2000);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_falls_back_to_live_raw_rows_for_missing_hourly_buckets()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id =
        insert_api_key_account(&state.pool, "Hydrate Usage Missing Hourly Bucket").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(2)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;
    let cursor_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("load invocation cursor");
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
    .bind(cursor_id)
    .execute(&state.pool)
    .await
    .expect("mark live rollup cursor without backfill");

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch usage with missing hourly buckets");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 2000);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_merges_hourly_rows_when_live_cursor_missing()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Missing Cursor").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 300,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");

    let archived_hourly_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    insert_upstream_account_usage_hourly_row(
        &state.pool,
        UsageHourlySample {
            account_id,
            occurred_at: &archived_hourly_at,
            request_count: 2,
            input_tokens: 2800,
            output_tokens: 1200,
            cache_input_tokens: 400,
            total_cost: 0.044,
        },
    )
    .await;

    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage without live cursor");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 3300);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 3);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 7700);
    assert_eq!(item["secondaryActualUsage"]["inputTokens"], 4900);
    assert_eq!(item["secondaryActualUsage"]["outputTokens"], 2100);
    assert_eq!(item["secondaryActualUsage"]["cacheInputTokens"], 700);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_keeps_pre_cursor_partial_minute_exact() {
    get_upstream_account_window_usage_keeps_pre_cursor_partial_minute_exact_impl().await;
}

async fn get_upstream_account_window_usage_keeps_pre_cursor_partial_minute_exact_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_oauth_account(&state.pool, "Hydrate Usage Partial Minute").await;
    let now = (Utc::now() - ChronoDuration::minutes(1))
        .with_second(30)
        .and_then(|value| value.with_nanosecond(0))
        .expect("align fixed now");
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 300,
            resets_at: Some(now.to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some(now.to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");

    let boundary_row_at =
        shanghai_local_iso(now - ChronoDuration::hours(5) + ChronoDuration::seconds(15));
    let full_minute_row_at = shanghai_local_iso(now - ChronoDuration::hours(4));
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &boundary_row_at,
        Some(1000),
        Some(500),
        Some(100),
        Some(1600),
        Some(0.016),
    )
    .await;
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &full_minute_row_at,
        Some(1500),
        Some(700),
        Some(200),
        Some(2400),
        Some(0.024),
    )
    .await;

    seed_partial_minute_rollup(&state.pool, account_id, &full_minute_row_at).await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage with partial minute boundary");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 2);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 4000);
    assert_eq!(item["primaryActualUsage"]["inputTokens"], 2500);
    assert_eq!(item["primaryActualUsage"]["outputTokens"], 1200);
    assert_eq!(item["primaryActualUsage"]["cacheInputTokens"], 300);
}

async fn seed_partial_minute_rollup(pool: &SqlitePool, account_id: i64, occurred_at: &str) {
    sqlx::query(
        r#"
            INSERT INTO upstream_account_stats_minute (
                bucket_start_epoch, source, upstream_account_id, total_count,
                success_count, failure_count, in_flight_count, total_tokens,
                input_tokens, output_tokens, cache_input_tokens, total_cost, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6, ?7, ?8, ?9, ?10, datetime('now'))
            "#,
    )
    .bind(invocation_bucket_start_epoch_for_seconds(occurred_at, 60).expect("minute bucket"))
    .bind(SOURCE_PROXY)
    .bind(account_id)
    .bind(1_i64)
    .bind(1_i64)
    .bind(2400_i64)
    .bind(1500_i64)
    .bind(700_i64)
    .bind(200_i64)
    .bind(0.024_f64)
    .execute(pool)
    .await
    .expect("insert minute rollup row");
    let cursor_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(pool)
            .await
            .expect("load invocation cursor");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = datetime('now')",
    )
    .bind("codex_invocations")
    .bind(cursor_id)
    .execute(pool)
    .await
    .expect("mark live rollup cursor");
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_includes_archived_partial_bucket_before_retention_cutoff()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/upstream-account-usage-boundary-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Archived Boundary").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    let now = Utc::now();
    let archived_boundary_at =
        shanghai_local_iso(now - ChronoDuration::days(7) + ChronoDuration::minutes(5));
    let archived_full_hour_at = shanghai_local_iso(now - ChronoDuration::days(2));
    seed_window_actual_usage_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "window-usage-archived-boundary",
        &[
            (
                account_id,
                archived_boundary_at.clone(),
                Some(900),
                Some(500),
                Some(100),
                Some(1500),
                Some(0.015),
            ),
            (
                account_id,
                archived_full_hour_at,
                Some(2800),
                Some(1200),
                Some(400),
                Some(4400),
                Some(0.044),
            ),
        ],
    )
    .await;
    materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize historical rollups");

    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(now - ChronoDuration::minutes(20)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage across retention cutoff");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 3);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 7900);
    assert_eq!(item["secondaryActualUsage"]["inputTokens"], 4900);
    assert_eq!(item["secondaryActualUsage"]["outputTokens"], 2300);
    assert_eq!(item["secondaryActualUsage"]["cacheInputTokens"], 700);
}

#[tokio::test]
pub(crate) async fn load_upstream_account_detail_with_actual_usage_serializes_actual_usage_camel_case()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_oauth_account(&state.pool, "Detail Usage OAuth").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(33.0),
        Some(55.0),
    )
    .await;

    let primary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(25));
    let secondary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(1));
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &primary_row_at,
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &secondary_row_at,
        Some(700),
        Some(200),
        Some(100),
        Some(1000),
        Some(0.01),
    )
    .await;

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load detail with actual usage")
        .expect("detail exists");
    let primary_usage = detail
        .summary
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("primary actual usage");
    let secondary_usage = detail
        .summary
        .secondary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("secondary actual usage");

    assert_eq!(primary_usage.request_count, 1);
    assert_eq!(primary_usage.total_tokens, 3300);
    assert_cost_close(primary_usage.total_cost, 0.033);

    assert_eq!(secondary_usage.request_count, 2);
    assert_eq!(secondary_usage.total_tokens, 4300);
    assert_cost_close(secondary_usage.total_cost, 0.043);

    let payload = serde_json::to_value(&detail).expect("serialize detail payload");
    assert_eq!(payload["primaryWindow"]["actualUsage"]["requestCount"], 1);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["totalTokens"], 3300);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["inputTokens"], 2100);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["outputTokens"], 900);
    assert_eq!(
        payload["primaryWindow"]["actualUsage"]["cacheInputTokens"],
        300
    );
    assert_eq!(payload["secondaryWindow"]["actualUsage"]["requestCount"], 2);
    assert_eq!(
        payload["secondaryWindow"]["actualUsage"]["totalTokens"],
        4300
    );
}

pub(crate) struct UsageHourlySample<'a> {
    pub(crate) account_id: i64,
    pub(crate) occurred_at: &'a str,
    pub(crate) request_count: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
}

pub(crate) async fn insert_upstream_account_usage_hourly_row(
    pool: &SqlitePool,
    sample: UsageHourlySample<'_>,
) {
    let bucket_start_epoch =
        invocation_bucket_start_epoch(sample.occurred_at).expect("derive usage hourly bucket");
    let total_tokens = sample.input_tokens + sample.output_tokens + sample.cache_input_tokens;
    sqlx::query(
        r#"
            INSERT INTO upstream_account_usage_hourly (
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                first_seen_at,
                last_seen_at,
                updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9
            )
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(sample.account_id)
    .bind(sample.request_count)
    .bind(total_tokens)
    .bind(sample.total_cost)
    .bind(sample.input_tokens)
    .bind(sample.output_tokens)
    .bind(sample.cache_input_tokens)
    .bind(sample.occurred_at)
    .execute(pool)
    .await
    .expect("insert upstream account usage hourly row");
}

pub(crate) fn benchmark_percentile(samples_ms: &[f64], percentile: f64) -> f64 {
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("finite samples"));
    let rank = ((sorted.len().saturating_sub(1) as f64) * percentile).ceil() as usize;
    sorted[rank]
}

pub(crate) fn benchmark_average(samples_ms: &[f64]) -> f64 {
    samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
}

pub(crate) fn round_millis(value_ms: f64) -> f64 {
    (value_ms * 100.0).round() / 100.0
}

#[tokio::test]
#[ignore = "manual benchmark: seeds a prod-sized fixture and prints latency samples"]
pub(crate) async fn benchmark_upstream_account_roster_prod_sized() {
    benchmark_upstream_account_roster_prod_sized_impl().await;
}

async fn benchmark_upstream_account_roster_prod_sized_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_ids = seed_benchmark_accounts(&state).await;

    let batch_account_ids = load_benchmark_batch_account_ids(&state, &account_ids).await;
    let (flat_samples_ms, include_all_samples_ms, window_usage_samples_ms) =
        measure_benchmark_queries(&state, &batch_account_ids).await;

    let flat_p95 = benchmark_percentile(&flat_samples_ms, 0.95);
    let include_all_p95 = benchmark_percentile(&include_all_samples_ms, 0.95);
    let window_usage_p95 = benchmark_percentile(&window_usage_samples_ms, 0.95);

    println!(
        "benchmark_upstream_account_roster_prod_sized flat_page20 total_accounts={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        account_ids.len(),
        flat_samples_ms.len(),
        round_millis(benchmark_average(&flat_samples_ms)),
        round_millis(benchmark_percentile(&flat_samples_ms, 0.50)),
        round_millis(flat_p95),
    );
    println!(
        "benchmark_upstream_account_roster_prod_sized include_all total_accounts={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        account_ids.len(),
        include_all_samples_ms.len(),
        round_millis(benchmark_average(&include_all_samples_ms)),
        round_millis(benchmark_percentile(&include_all_samples_ms, 0.50)),
        round_millis(include_all_p95),
    );
    println!(
        "benchmark_upstream_account_roster_prod_sized window_usage_batch batch_size={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        batch_account_ids.len(),
        window_usage_samples_ms.len(),
        round_millis(benchmark_average(&window_usage_samples_ms)),
        round_millis(benchmark_percentile(&window_usage_samples_ms, 0.50)),
        round_millis(window_usage_p95),
    );

    assert!(
        flat_p95 <= 100.0,
        "flat roster p95 exceeded budget: {flat_p95:.2}ms"
    );
    assert!(
        include_all_p95 <= 100.0,
        "includeAll roster p95 exceeded budget: {include_all_p95:.2}ms"
    );
    assert!(
        window_usage_p95 <= 100.0,
        "window usage batch p95 exceeded budget: {window_usage_p95:.2}ms"
    );
}

async fn measure_benchmark_queries(
    state: &Arc<AppState>,
    batch_account_ids: &[i64],
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let flat_query = || ListUpstreamAccountsQuery {
        page: Some(1),
        page_size: Some(20),
        ..Default::default()
    };
    let include_all_query = || ListUpstreamAccountsQuery {
        page: Some(1),
        page_size: Some(20),
        include_all: Some(true),
        ..Default::default()
    };
    for _ in 0..5 {
        let _ = list_upstream_accounts_from_params(state.clone(), flat_query())
            .await
            .expect("warm flat roster");
        let _ = list_upstream_accounts_from_params(state.clone(), include_all_query())
            .await
            .expect("warm includeAll roster");
        let _ = get_upstream_account_window_usage(
            State(state.clone()),
            Json(UpstreamAccountWindowUsageRequest {
                account_ids: batch_account_ids.to_vec(),
            }),
        )
        .await
        .expect("warm usage batch");
    }
    let mut flat_samples_ms = Vec::with_capacity(30);
    let mut include_all_samples_ms = Vec::with_capacity(30);
    let mut window_usage_samples_ms = Vec::with_capacity(30);
    for _ in 0..30 {
        let started_at = std::time::Instant::now();
        let _ = list_upstream_accounts_from_params(state.clone(), flat_query())
            .await
            .expect("benchmark flat roster");
        flat_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
        let started_at = std::time::Instant::now();
        let _ = list_upstream_accounts_from_params(state.clone(), include_all_query())
            .await
            .expect("benchmark includeAll roster");
        include_all_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
        let started_at = std::time::Instant::now();
        let _ = get_upstream_account_window_usage(
            State(state.clone()),
            Json(UpstreamAccountWindowUsageRequest {
                account_ids: batch_account_ids.to_vec(),
            }),
        )
        .await
        .expect("benchmark window usage batch");
        window_usage_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
    }
    (
        flat_samples_ms,
        include_all_samples_ms,
        window_usage_samples_ms,
    )
}

async fn load_benchmark_batch_account_ids(state: &Arc<AppState>, account_ids: &[i64]) -> Vec<i64> {
    let query = ListUpstreamAccountsQuery {
        page: Some(1),
        page_size: Some(20),
        ..Default::default()
    };
    let Json(response) = list_upstream_accounts_from_params(state.clone(), query)
        .await
        .expect("load flat roster for benchmark batch ids");
    assert_eq!(account_ids.len(), 159);
    let batch_account_ids = response
        .items
        .iter()
        .map(|item| item.id)
        .take(20)
        .collect::<Vec<_>>();
    assert_eq!(batch_account_ids.len(), 20);
    batch_account_ids
}

async fn seed_benchmark_accounts(state: &Arc<AppState>) -> Vec<i64> {
    ensure_window_actual_usage_test_tables(&state.pool).await;
    let group_names = (0..12)
        .map(|index| format!("benchmark-group-{index:02}"))
        .collect::<Vec<_>>();
    for group_name in &group_names {
        ensure_test_group_binding(&state.pool, group_name).await;
    }
    let captured_at = format_utc_iso(Utc::now());
    let now = Utc::now();
    let mut account_ids = Vec::with_capacity(159);
    for index in 0..159_usize {
        let display_name = format!("Benchmark Account {index:03}");
        let account_id = insert_api_key_account(&state.pool, &display_name).await;
        if index % 11 != 0 {
            let group_name = &group_names[index % group_names.len()];
            set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
        }
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            &captured_at,
            Some((10 + (index % 65)) as f64),
            Some((5 + (index % 35)) as f64),
        )
        .await;
        for hour_offset in 0..(24 * 7) {
            let occurred_at =
                shanghai_local_iso(now - ChronoDuration::hours(hour_offset as i64 + 1));
            let request_count = 1 + ((index + hour_offset) % 3) as i64;
            let input_tokens = 900 + ((index + hour_offset) % 300) as i64;
            let output_tokens = 400 + ((index * 3 + hour_offset) % 180) as i64;
            let cache_input_tokens = 90 + ((index * 5 + hour_offset) % 70) as i64;
            let total_cost =
                ((input_tokens + output_tokens + cache_input_tokens) as f64) / 100_000.0;
            insert_upstream_account_usage_hourly_row(
                &state.pool,
                UsageHourlySample {
                    account_id,
                    occurred_at: &occurred_at,
                    request_count,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost,
                },
            )
            .await;
        }
        let live_tail_at = shanghai_local_iso(now - ChronoDuration::minutes((index % 45) as i64));
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &live_tail_at,
            Some(700 + (index % 120) as i64),
            Some(320 + (index % 90) as i64),
            Some(80 + (index % 30) as i64),
            Some(1100 + (index % 210) as i64),
            Some(0.011 + (index as f64 * 0.00001)),
        )
        .await;
        account_ids.push(account_id);
    }
    account_ids
}

use super::*;
