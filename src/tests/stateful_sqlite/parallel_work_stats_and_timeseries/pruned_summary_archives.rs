use super::*;

#[tokio::test]
async fn all_time_summary_repair_replays_existing_materialized_archives_when_others_are_pruned() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let pruned_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        // Legacy manifests protect the full month. Keep the pruned and readable
        // sources at least 60 days apart, including a 31-day boundary month.
        - ChronoDuration::days(500))
    .and_hms_opt(8, 0, 0)
    .expect("valid pruned local hour");
    let pruned_success_at = format_naive(
        pruned_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("pruned success time"),
    );
    let pruned_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-pruned-mixed",
        &[(
            1_i64,
            "summary-all-materialized-pruned-mixed-success",
            pruned_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(pruned_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark pruned archive as materialized");

    let pruned_bucket_start_epoch =
        invocation_bucket_start_epoch(&pruned_success_at).expect("derive pruned bucket epoch");
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
    .bind(pruned_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed preserved pruned archive rollup");
    mark_summary_archive_replay_complete(&state.pool, &pruned_archive_path).await;

    fs::remove_file(&pruned_archive_path).expect("prune older materialized archive");

    let existing_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(440))
    .and_hms_opt(8, 0, 0)
    .expect("valid existing local hour");
    let existing_success_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("existing success time"),
    );
    let existing_failed_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("existing failed time"),
    );
    let existing_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-existing-mixed",
        &[
            (
                1_i64,
                "summary-all-materialized-existing-success",
                existing_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
            (
                2_i64,
                "summary-all-materialized-existing-failed",
                existing_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30_i64,
                0.30_f64,
                Some(300.0),
            ),
        ],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(existing_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark existing archive as materialized");

    let existing_bucket_start_epoch =
        invocation_bucket_start_epoch(&existing_success_at).expect("derive existing bucket epoch");
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
    .bind(existing_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(50_i64)
    .bind(0.50_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale existing materialized rollup");

    assert_ne!(
        &pruned_success_at[..7],
        &existing_success_at[..7],
        "fixture archives must not share a protected legacy month"
    );

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with mixed materialized archive states");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 60);
    assert!((summary.total_cost - 0.60).abs() < 1e-9);

    let existing_replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(existing_archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load replay markers for existing materialized archive");
    assert_eq!(existing_replayed_targets, 2);

    let rollup_failure_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(failure_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired failure count for existing materialized archive");
    assert_eq!(rollup_failure_count, 1);
}

#[tokio::test]
async fn all_time_summary_repair_rebuilds_non_materialized_archives_when_others_are_pruned() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let pruned_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        // A missing legacy archive protects its complete month, not only its rows.
        - ChronoDuration::days(500))
    .and_hms_opt(8, 0, 0)
    .expect("valid pruned local hour");
    let pruned_success_at = format_naive(
        pruned_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("pruned success time"),
    );
    let pruned_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-nonmaterialized-pruned-mixed",
        &[(
            1_i64,
            "summary-all-nonmaterialized-pruned-success",
            pruned_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(pruned_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark pruned archive as materialized");

    let pruned_bucket_start_epoch =
        invocation_bucket_start_epoch(&pruned_success_at).expect("derive pruned bucket epoch");
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
    .bind(pruned_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed preserved pruned archive rollup");
    mark_summary_archive_replay_complete(&state.pool, &pruned_archive_path).await;

    fs::remove_file(&pruned_archive_path).expect("prune older materialized archive");

    let existing_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(440))
    .and_hms_opt(8, 0, 0)
    .expect("valid existing local hour");
    let existing_success_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("existing success time"),
    );
    let existing_failed_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("existing failed time"),
    );
    let existing_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-nonmaterialized-existing-mixed",
        &[
            (
                1_i64,
                "summary-all-nonmaterialized-existing-success",
                existing_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
            (
                2_i64,
                "summary-all-nonmaterialized-existing-failed",
                existing_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30_i64,
                0.30_f64,
                Some(300.0),
            ),
        ],
    )
    .await;

    let existing_bucket_start_epoch =
        invocation_bucket_start_epoch(&existing_success_at).expect("derive existing bucket epoch");
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
    .bind(existing_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(50_i64)
    .bind(0.50_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale non-materialized rollup");

    assert_ne!(
        &pruned_success_at[..7],
        &existing_success_at[..7],
        "fixture archives must not share a protected legacy month"
    );

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with mixed non-materialized archive states");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 60);
    assert!((summary.total_cost - 0.60).abs() < 1e-9);

    let existing_replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(existing_archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load replay markers for existing non-materialized archive");
    assert_eq!(existing_replayed_targets, 2);

    let rollup_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired total count for existing non-materialized archive");
    assert_eq!(rollup_total_count, 2);

    let rollup_failure_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(failure_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired failure count for existing non-materialized archive");
    assert_eq!(rollup_failure_count, 1);
}
