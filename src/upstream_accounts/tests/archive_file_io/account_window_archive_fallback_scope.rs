use super::*;

#[tokio::test]
async fn enrich_window_actual_usage_archive_fallback_keeps_partial_minute_account_scope() {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/upstream-account-partial-minute-scope-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let covered_account_id = 601_i64;
    let partial_account_id = 602_i64;
    let bucket_start_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600 - 2 * 86_400;
    let bucket_start_at = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("archive bucket start");
    let occurred_at = shanghai_local_iso(bucket_start_at + ChronoDuration::minutes(5));
    let covered_reset_at = bucket_start_at + ChronoDuration::hours(1);
    let partial_reset_at =
        bucket_start_at + ChronoDuration::minutes(5) + ChronoDuration::seconds(30);

    seed_window_actual_usage_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "partial-minute-account-scope",
        &[
            (
                covered_account_id,
                occurred_at.clone(),
                Some(1200),
                Some(600),
                Some(200),
                Some(2000),
                Some(0.02),
            ),
            (
                partial_account_id,
                occurred_at.clone(),
                Some(1800),
                Some(900),
                Some(300),
                Some(3000),
                Some(0.03),
            ),
        ],
    )
    .await;
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
            ) VALUES (?1, ?2, 1, 2000, 0.02, 1200, 600, 200, ?3, ?3, ?3)
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(covered_account_id)
    .bind(&occurred_at)
    .execute(&state.pool)
    .await
    .expect("insert covered account hourly usage");

    let mut covered_summary = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );
    covered_summary.id = covered_account_id;
    covered_summary.primary_window = Some(RateWindowSnapshot {
        used_percent: 1.0,
        used_text: "1% used".to_string(),
        limit_text: "1h rolling window".to_string(),
        resets_at: Some(covered_reset_at.to_rfc3339()),
        window_duration_mins: 60,
        actual_usage: None,
    });
    let mut partial_summary = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );
    partial_summary.id = partial_account_id;
    partial_summary.primary_window = Some(RateWindowSnapshot {
        used_percent: 1.0,
        used_text: "1% used".to_string(),
        limit_text: "1m rolling window".to_string(),
        resets_at: Some(partial_reset_at.to_rfc3339()),
        window_duration_mins: 1,
        actual_usage: None,
    });

    let mut summaries = vec![covered_summary, partial_summary];
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut summaries)
        .await
        .expect("enrich archived account-scoped partial minute usage");

    let covered_usage = summaries[0]
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("covered account hourly usage");
    assert_eq!(covered_usage.request_count, 1);
    assert_eq!(covered_usage.total_tokens, 2000);
    let partial_usage = summaries[1]
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("partial account archived usage");
    assert_eq!(partial_usage.request_count, 1);
    assert_eq!(partial_usage.total_tokens, 3000);
}
