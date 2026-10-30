use super::*;

#[tokio::test]
async fn get_upstream_account_window_usage_falls_back_to_live_raw_rows_for_missing_hourly_buckets_per_account()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id =
        insert_api_key_account(&state.pool, "Hydrate Usage Missing Hourly Bucket").await;
    let covered_account_id =
        insert_api_key_account(&state.pool, "Hydrate Usage Covered Hourly Bucket").await;
    for account_id in [account_id, covered_account_id] {
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            &format_utc_iso(Utc::now()),
            Some(18.0),
            Some(9.0),
        )
        .await;
    }

    let occurred_at = shanghai_local_iso(Utc::now() - ChronoDuration::hours(2));
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &occurred_at,
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;
    insert_window_actual_usage_invocation(
        &state.pool,
        covered_account_id,
        &occurred_at,
        Some(1800),
        Some(900),
        Some(300),
        Some(3000),
        Some(0.03),
    )
    .await;
    insert_account_window_usage_hourly_row(
        &state.pool,
        covered_account_id,
        &occurred_at,
        1200,
        600,
        200,
        0.02,
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
            account_ids: vec![account_id, covered_account_id],
        }),
    )
    .await
    .expect("load batch usage with missing hourly buckets");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let missing_item = payload["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["accountId"] == account_id))
        .cloned()
        .expect("missing-hour account usage item");
    let covered_item = payload["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["accountId"] == covered_account_id)
        })
        .cloned()
        .expect("covered-hour account usage item");

    assert_eq!(missing_item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(missing_item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(missing_item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(missing_item["secondaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(covered_item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(covered_item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(covered_item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(covered_item["secondaryActualUsage"]["totalTokens"], 2000);
}

async fn insert_account_window_usage_hourly_row(
    pool: &SqlitePool,
    account_id: i64,
    occurred_at: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    total_cost: f64,
) {
    let bucket_start_epoch =
        invocation_bucket_start_epoch(occurred_at).expect("derive usage hourly bucket");
    let total_tokens = input_tokens + output_tokens + cache_input_tokens;
    sqlx::query(
        r#"
            INSERT INTO upstream_account_usage_hourly (
                bucket_start_epoch, upstream_account_id, request_count, total_tokens,
                total_cost, input_tokens, output_tokens, cache_input_tokens,
                first_seen_at, last_seen_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9)
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(account_id)
    .bind(1_i64)
    .bind(total_tokens)
    .bind(total_cost)
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(cache_input_tokens)
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("insert upstream account usage hourly row");
}

#[tokio::test]
async fn account_window_bucket_query_deduplicates_overlapping_ranges_and_excludes_end_boundary() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Bucket Bounds").await;
    let bucket_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600 - 7_200;
    let bucket_start = Utc
        .timestamp_opt(bucket_epoch, 0)
        .single()
        .expect("bucket start");
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(bucket_start + ChronoDuration::seconds(30)),
        Some(100),
        Some(50),
        Some(10),
        Some(160),
        Some(0.016),
    )
    .await;
    insert_window_actual_usage_invocation(
        &state.pool,
        account_id,
        &shanghai_local_iso(bucket_start + ChronoDuration::hours(1)),
        Some(200),
        Some(100),
        Some(20),
        Some(320),
        Some(0.032),
    )
    .await;

    let mut bucket_ranges = (1_i64..=63)
        .map(|offset| (bucket_epoch - offset * 7_200, 60))
        .collect::<Vec<_>>();
    bucket_ranges.push((bucket_epoch, 60));
    bucket_ranges.push((bucket_epoch, 3_600));
    let rows = load_window_actual_usage_rows_for_bucket_ranges_from_pool(
        &state.pool,
        &[account_id],
        &bucket_ranges,
        None,
        None,
    )
    .await
    .expect("load overlapping account window buckets");
    assert_eq!(rows.len(), 1, "overlapping ranges must not duplicate rows");
    assert_eq!(rows[0].total_tokens, Some(160));
}

#[test]
fn account_window_missing_hour_fallback_keeps_account_scope() {
    let bucket_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600 - 7_200;
    let occurred_at = shanghai_local_iso(
        Utc.timestamp_opt(bucket_epoch + 120, 0)
            .single()
            .expect("missing hour row time"),
    );
    let rows = vec![
        AccountWindowUsageRow {
            id: 1,
            occurred_at: occurred_at.clone(),
            upstream_account_id: 41,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.03),
        },
        AccountWindowUsageRow {
            id: 2,
            occurred_at,
            upstream_account_id: 42,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.03),
        },
    ];
    let filtered = filter_account_window_usage_rows_for_exact_fallback(
        rows,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([(42, bucket_epoch)]),
        &HashSet::new(),
    )
    .expect("filter account-scoped missing hour rows");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].upstream_account_id, 42);
}

#[test]
fn account_window_partial_minute_fallback_keeps_account_scope() {
    let bucket_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600 - 7_200;
    let occurred_at = shanghai_local_iso(
        Utc.timestamp_opt(bucket_epoch + 120, 0)
            .single()
            .expect("partial minute row time"),
    );
    let rows = vec![
        AccountWindowUsageRow {
            id: 3,
            occurred_at: occurred_at.clone(),
            upstream_account_id: 41,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.03),
        },
        AccountWindowUsageRow {
            id: 4,
            occurred_at,
            upstream_account_id: 42,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.03),
        },
    ];
    let filtered = filter_account_window_usage_rows_for_exact_fallback(
        rows,
        &HashSet::from([(42, bucket_epoch + 120)]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([(41, bucket_epoch)]),
    )
    .expect("filter account-scoped partial minute rows");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].upstream_account_id, 42);
}
