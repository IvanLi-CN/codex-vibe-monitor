use super::*;

#[tokio::test]
async fn parallel_work_stats_counts_distinct_prompt_cache_keys_per_bucket() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_minute_epoch =
        align_reporting_bucket_epoch(Utc::now().timestamp(), 60, Shanghai).expect("align minute");
    let minute_a = Utc
        .timestamp_opt(current_minute_epoch - 3 * 60, 0)
        .single()
        .expect("minute a");
    let minute_b = minute_a + ChronoDuration::minutes(1);

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-a-1",
        minute_a + ChronoDuration::seconds(10),
        "pck-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-a-2",
        minute_a + ChronoDuration::seconds(20),
        "pck-alpha",
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, 'success', 10, 0.01, ?4, '{}')
        "#,
    )
    .bind("parallel-minute-a-cross-source")
    .bind(format_naive(
        (minute_a + ChronoDuration::seconds(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind("direct")
    .bind(json!({ "promptCacheKey": "pck-alpha", "upstreamAccountId": 42 }).to_string())
    .execute(&state.pool)
    .await
    .expect("insert cross-source parallel-work key");
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-b-1",
        minute_b + ChronoDuration::seconds(10),
        "pck-alpha",
    )
    .await;

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-b-2",
        minute_b + ChronoDuration::seconds(20),
        "pck-beta",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-b-3",
        minute_b + ChronoDuration::seconds(30),
        "pck-gamma",
    )
    .await;

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET payload = json_set(payload, '$.upstreamAccountId', 42)
        WHERE invoke_id IN (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("parallel-minute-a-1")
    .bind("parallel-minute-a-2")
    .bind("parallel-minute-b-1")
    .bind("parallel-minute-b-2")
    .bind("parallel-minute-b-3")
    .execute(&state.pool)
    .await
    .expect("assign upstream account to parallel-work keys");

    let Json(response) = fetch_parallel_work_stats(
        State(state.clone()),
        Query(ParallelWorkStatsQuery {
            range: "7d".to_string(),
            bucket: Some("1m".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let minute_a_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_a))
        .expect("minute a point");
    let minute_b_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_b))
        .expect("minute b point");

    assert_eq!(minute_a_point.parallel_count, 1);
    assert_eq!(minute_b_point.parallel_count, 3);
    assert_eq!(response.minute7d.active_bucket_count, 2);
    assert_eq!(response.minute7d.max_count, Some(3));
    assert_eq!(response.minute7d.min_count, Some(0));
    assert_eq!(response.minute7d.active_minute_count, Some(2));
    assert_f64_close(
        response
            .minute7d
            .avg_count
            .expect("active-minute average should be present"),
        2.0,
    );

    for bucket in ["1h", "1d"] {
        let Json(window_response) = fetch_parallel_work_stats(
            State(state.clone()),
            Query(ParallelWorkStatsQuery {
                range: "7d".to_string(),
                bucket: Some(bucket.to_string()),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("fetch parallel-work stats for display bucket");
        assert_eq!(window_response.current.active_minute_count, Some(2));
        assert_f64_close(
            window_response
                .current
                .avg_count
                .expect("active-minute average should not depend on display bucket"),
            2.0,
        );
    }

    let Json(account_response) = fetch_parallel_work_stats(
        State(state.clone()),
        Query(ParallelWorkStatsQuery {
            range: "7d".to_string(),
            bucket: Some("1h".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("fetch account parallel-work stats");
    assert_eq!(account_response.current.active_minute_count, Some(2));
    assert_f64_close(
        account_response
            .current
            .avg_count
            .expect("account active-minute average should be present"),
        2.0,
    );
}

#[tokio::test]
async fn parallel_work_active_minute_stats_uses_verified_hourly_scalars_for_history() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let historic_hour = Utc
        .timestamp_opt(
            parallel_work_minute_rollup_keep_start_epoch(Utc::now())
                .expect("minute-rollup retention start")
                - 3_600,
            0,
        )
        .single()
        .expect("historic hour");
    for source_scope in ["all", "proxy_only"] {
        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            )
            VALUES (?1, ?2, 1, 1)
            "#,
        )
        .bind(historic_hour.timestamp())
        .bind(source_scope)
        .execute(&state.pool)
        .await
        .expect("insert verified parallel-work coverage");
    }
    sqlx::query(
        r#"
        INSERT INTO parallel_work_hourly_rollup (
            hour_start_epoch, source_scope, active_minute_count, parallel_count_sum
        )
        VALUES (?1, 'all', 2, 4)
        "#,
    )
    .bind(historic_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("insert global hourly scalar");
    sqlx::query(
        r#"
        INSERT INTO parallel_work_upstream_account_hourly_rollup (
            hour_start_epoch, source_scope, upstream_account_id,
            active_minute_count, parallel_count_sum
        )
        VALUES (?1, 'all', 42, 1, 3)
        "#,
    )
    .bind(historic_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("insert account hourly scalar");

    let global = query_parallel_work_active_minute_stats(
        &state.pool,
        historic_hour,
        historic_hour + ChronoDuration::hours(1),
        InvocationSourceScope::All,
        None,
        None,
    )
    .await
    .expect("query global historical active-minute scalar");
    assert_eq!(global.active_minute_count, Some(2));
    assert_f64_close(global.average().expect("global average"), 2.0);

    let account = query_parallel_work_active_minute_stats(
        &state.pool,
        historic_hour,
        historic_hour + ChronoDuration::hours(1),
        InvocationSourceScope::All,
        Some(42),
        None,
    )
    .await
    .expect("query account historical active-minute scalar");
    assert_eq!(account.active_minute_count, Some(1));
    assert_f64_close(account.average().expect("account average"), 3.0);

    let uncovered = query_parallel_work_active_minute_stats(
        &state.pool,
        historic_hour - ChronoDuration::hours(1),
        historic_hour,
        InvocationSourceScope::All,
        None,
        None,
    )
    .await
    .expect("query uncovered historical active-minute range");
    assert_eq!(uncovered.active_minute_count, None);
    assert_eq!(uncovered.average(), None);
}

#[tokio::test]
async fn parallel_work_active_minute_stats_rejects_a_raw_retention_gap_without_rollup_coverage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let minute_rollup_start = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start");
    let raw_retention_start = shanghai_retention_cutoff(7).timestamp();
    assert!(
        minute_rollup_start < raw_retention_start,
        "test requires raw detail retention to begin inside the minute-rollup window"
    );

    let result = query_parallel_work_active_minute_stats(
        &state.pool,
        Utc.timestamp_opt(minute_rollup_start, 0)
            .single()
            .expect("valid minute-rollup start"),
        Utc.timestamp_opt(minute_rollup_start + 3_600, 0)
            .single()
            .expect("valid minute-rollup end"),
        InvocationSourceScope::All,
        None,
        Some(raw_retention_start),
    )
    .await
    .expect("query active-minute stats across raw retention gap");

    assert_eq!(result.active_minute_count, None);
    assert_eq!(result.average(), None);
}

#[tokio::test]
async fn parallel_work_active_minute_stats_accepts_partial_hour_minute_rollup_ranges() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let covered_hour = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start")
        + 3_600;
    let active_minute = covered_hour + 10 * 60;
    sqlx::query(
        r#"
        INSERT INTO parallel_work_hourly_coverage (
            hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
        )
        VALUES (?1, 'all', 1, 0)
        "#,
    )
    .bind(covered_hour)
    .execute(&state.pool)
    .await
    .expect("seed complete minute-key coverage");
    sqlx::query(
        r#"
        INSERT INTO parallel_work_minute_key_rollup (
            minute_start_epoch, source, prompt_cache_key
        )
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(active_minute)
    .bind(SOURCE_PROXY)
    .bind("pck-partial-hour")
    .execute(&state.pool)
    .await
    .expect("seed active minute key");

    let range_start = Utc
        .timestamp_opt(covered_hour + 5 * 60, 0)
        .single()
        .expect("valid partial-hour start");
    let range_end = Utc
        .timestamp_opt(covered_hour + 20 * 60, 0)
        .single()
        .expect("valid partial-hour end");
    let result = query_parallel_work_active_minute_stats(
        &state.pool,
        range_start,
        range_end,
        InvocationSourceScope::All,
        None,
        Some(range_end.timestamp()),
    )
    .await
    .expect("query partial-hour minute-rollup range");

    assert_eq!(result.active_minute_count, Some(1));
    assert_f64_close(result.average().expect("partial-hour average"), 1.0);
}

#[tokio::test]
async fn parallel_work_active_minute_stats_uses_pool_attempt_account_fallback_for_raw_tail() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut now = Utc::now();
    let seconds_into_hour = now.timestamp().rem_euclid(3_600);
    if seconds_into_hour < 60 {
        tokio::time::sleep(std::time::Duration::from_secs(
            (61 - seconds_into_hour) as u64,
        ))
        .await;
        now = Utc::now();
    }
    let minute_start_epoch = align_reporting_bucket_epoch(now.timestamp(), 60, Shanghai)
        .expect("align complete minute")
        - 60;
    let minute_start = Utc
        .timestamp_opt(minute_start_epoch, 0)
        .single()
        .expect("valid complete minute");
    let occurred_at = minute_start + ChronoDuration::seconds(10);
    let invoke_id = "parallel-work-attempt-account-fallback";
    insert_parallel_work_invocation(&state.pool, invoke_id, occurred_at, "pck-account-fallback")
        .await;
    let occurred_at_text = format_naive(occurred_at.with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            upstream_account_id, upstream_route_key, attempt_index,
            distinct_account_index, same_account_retry_index, requester_ip,
            started_at, status, phase
        )
        VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route-fallback', 1, 1, 0,
                '127.0.0.1', ?2, 'success', 'completed')
        "#,
    )
    .bind(invoke_id)
    .bind(&occurred_at_text)
    .bind("pck-account-fallback")
    .bind(42_i64)
    .execute(&state.pool)
    .await
    .expect("insert pool attempt account fallback");

    let result = query_parallel_work_active_minute_stats(
        &state.pool,
        minute_start,
        minute_start + ChronoDuration::minutes(1),
        InvocationSourceScope::All,
        Some(42),
        Some(shanghai_retention_cutoff(state.config.invocation_max_days).timestamp()),
    )
    .await
    .expect("query account-scoped raw active-minute stats");

    assert_eq!(result.active_minute_count, Some(1));
    assert_f64_close(result.average().expect("account average"), 1.0);
}

#[tokio::test]
async fn parallel_work_maintenance_materializes_before_expiring_minute_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let expired_hour = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start")
        - 3_600;
    let expired_hour = Utc
        .timestamp_opt(expired_hour, 0)
        .single()
        .expect("valid expired hour");
    let expired_minute_epoch = (expired_hour + ChronoDuration::minutes(5)).timestamp();
    for source_scope in ["all", "proxy_only"] {
        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            )
            VALUES (?1, ?2, 1, 0)
            "#,
        )
        .bind(expired_hour.timestamp())
        .bind(source_scope)
        .execute(&state.pool)
        .await
        .expect("seed complete minute coverage");
    }
    sqlx::query(
        r#"
        INSERT INTO parallel_work_minute_key_rollup (
            minute_start_epoch, source, prompt_cache_key
        )
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(expired_minute_epoch)
    .bind(SOURCE_PROXY)
    .bind("pck-expired-minute-key")
    .execute(&state.pool)
    .await
    .expect("seed expired minute key");

    maintain_parallel_work_rollups(&state.pool, None)
        .await
        .expect("materialize expired parallel-work hour");

    let scalar: (i64, i64) = sqlx::query_as(
        r#"
        SELECT active_minute_count, parallel_count_sum
        FROM parallel_work_hourly_rollup
        WHERE hour_start_epoch = ?1 AND source_scope = 'all'
        "#,
    )
    .bind(expired_hour.timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("materialized global scalar");
    assert_eq!(scalar, (1, 1));
    let coverage: i64 = sqlx::query_scalar(
        r#"
        SELECT hourly_scalar_complete
        FROM parallel_work_hourly_coverage
        WHERE hour_start_epoch = ?1 AND source_scope = 'all'
        "#,
    )
    .bind(expired_hour.timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("materialized coverage marker");
    assert_eq!(coverage, 1);
    let minute_key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_minute_key_rollup \
         WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2",
    )
    .bind(expired_hour.timestamp())
    .bind((expired_hour + ChronoDuration::hours(1)).timestamp())
    .fetch_one(&state.pool)
    .await
    .expect("count expired minute keys");
    assert_eq!(minute_key_count, 0);
}

#[tokio::test]
async fn parallel_work_payload_retention_waits_for_complete_minute_coverage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let keep_start = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start");
    let payload_loss_cutoff = keep_start + 2 * 3_600;
    assert!(
        !parallel_work_minute_coverage_ready_for_payload_retention(
            &state.pool,
            payload_loss_cutoff,
        )
        .await
        .expect("check missing minute coverage")
    );

    for hour_start_epoch in [keep_start, keep_start + 3_600] {
        for source_scope in ["all", "proxy_only"] {
            sqlx::query(
                r#"
                INSERT INTO parallel_work_hourly_coverage (
                    hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
                )
                VALUES (?1, ?2, 1, 0)
                "#,
            )
            .bind(hour_start_epoch)
            .bind(source_scope)
            .execute(&state.pool)
            .await
            .expect("seed complete minute coverage");
        }
    }

    assert!(
        parallel_work_minute_coverage_ready_for_payload_retention(
            &state.pool,
            payload_loss_cutoff,
        )
        .await
        .expect("check complete minute coverage")
    );
}

#[tokio::test]
async fn parallel_work_payload_retention_backfills_recoverable_pre_cutoff_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let keep_start = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start");
    let success_detail_cutoff = keep_start + 2 * 3_600;
    let live_start = keep_start - 3_600;
    let payload_loss_cutoff = success_detail_cutoff + 3_600;

    maintain_parallel_work_rollups(&state.pool, Some(live_start))
        .await
        .expect("maintain recoverable full-detail interval");
    maintain_parallel_work_rollups(&state.pool, Some(live_start + 3_600))
        .await
        .expect("preserve earliest verified coverage start");

    assert!(
        parallel_work_minute_coverage_ready_for_payload_retention(
            &state.pool,
            payload_loss_cutoff,
        )
        .await
        .expect("check recoverable minute coverage")
    );
    let pre_cutoff_coverage_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_hourly_coverage WHERE hour_start_epoch < ?1",
    )
    .bind(success_detail_cutoff)
    .fetch_one(&state.pool)
    .await
    .expect("count recoverable pre-cutoff markers");
    assert_eq!(pre_cutoff_coverage_rows, 4);
    assert_eq!(
        load_parallel_work_full_detail_start_epoch(&state.pool)
            .await
            .expect("load verified coverage start"),
        Some(keep_start)
    );
}

#[tokio::test]
async fn parallel_work_maintenance_does_not_certify_hours_before_live_retention() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let keep_start = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start");
    let live_start = keep_start + 2 * 3_600;

    maintain_parallel_work_rollups(&state.pool, Some(live_start))
        .await
        .expect("maintain live retention interval");

    let pre_live_coverage_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_hourly_coverage WHERE hour_start_epoch < ?1",
    )
    .bind(live_start)
    .fetch_one(&state.pool)
    .await
    .expect("count pre-live coverage markers");
    assert_eq!(pre_live_coverage_rows, 0);
    assert_eq!(
        load_parallel_work_full_detail_start_epoch(&state.pool)
            .await
            .expect("load live coverage start"),
        Some(live_start)
    );
}

#[tokio::test]
async fn parallel_work_maintenance_never_marks_pre_retention_raw_hours_as_covered() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let full_detail_start = shanghai_retention_cutoff(state.config.invocation_success_full_days)
        .timestamp()
        .div_euclid(3_600)
        * 3_600;
    let earlier_hour = full_detail_start - 3_600;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level, raw_response
        ) VALUES (?1, ?2, ?3, 'completed', ?4, '')
        "#,
    )
    .bind("inv-pre-retention-structured")
    .bind(format_naive(
        Utc.timestamp_opt(earlier_hour, 0)
            .single()
            .expect("valid earlier hour")
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_STRUCTURED_ONLY)
    .execute(&state.pool)
    .await
    .expect("seed unrecoverable pre-retention invocation");

    maintain_parallel_work_rollups(&state.pool, Some(full_detail_start))
        .await
        .expect("maintain only known full-detail coverage");

    let coverage_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM parallel_work_hourly_coverage
        WHERE hour_start_epoch = ?1
        "#,
    )
    .bind(earlier_hour)
    .fetch_one(&state.pool)
    .await
    .expect("count pre-retention coverage");
    assert_eq!(coverage_count, 0);
}

#[tokio::test]
async fn parallel_work_maintenance_preserves_minute_keys_outside_full_detail_coverage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let full_detail_start = (Utc::now() - ChronoDuration::days(7))
        .timestamp()
        .div_euclid(3_600)
        * 3_600;
    let retained_hour = full_detail_start - 3_600;
    sqlx::query(
        r#"
        INSERT INTO parallel_work_minute_key_rollup (
            minute_start_epoch, source, prompt_cache_key
        )
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(retained_hour + 5 * 60)
    .bind(SOURCE_PROXY)
    .bind("pck-preserve-without-payload")
    .execute(&state.pool)
    .await
    .expect("seed retained minute key");

    maintain_parallel_work_rollups(&state.pool, Some(full_detail_start))
        .await
        .expect("maintain parallel-work keys after payload pruning");

    let key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_minute_key_rollup \
         WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2",
    )
    .bind(retained_hour)
    .bind(retained_hour + 3_600)
    .fetch_one(&state.pool)
    .await
    .expect("count preserved minute keys");
    assert_eq!(key_count, 1);
    let coverage_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_hourly_coverage WHERE hour_start_epoch = ?1",
    )
    .bind(retained_hour)
    .fetch_one(&state.pool)
    .await
    .expect("count unverified coverage rows");
    assert_eq!(coverage_count, 0);
}

#[tokio::test]
async fn parallel_work_recompute_preserves_covered_keys_after_payload_pruning() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let hour_start = (Utc::now() - ChronoDuration::days(2))
        .timestamp()
        .div_euclid(3_600)
        * 3_600;
    sqlx::query(
        r#"
        INSERT INTO parallel_work_minute_key_rollup (
            minute_start_epoch, source, prompt_cache_key
        ) VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(hour_start + 5 * 60)
    .bind(SOURCE_PROXY)
    .bind("pck-preserved-after-prune")
    .execute(&state.pool)
    .await
    .expect("seed covered minute key");
    for source_scope in ["all", "proxy_only"] {
        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            ) VALUES (?1, ?2, 1, 0)
            "#,
        )
        .bind(hour_start)
        .bind(source_scope)
        .execute(&state.pool)
        .await
        .expect("seed minute coverage");
    }
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level, raw_response
        ) VALUES (?1, ?2, ?3, 'completed', ?4, '')
        "#,
    )
    .bind("inv-pruned-covered-hour")
    .bind(format_naive(
        Utc.timestamp_opt(hour_start + 10 * 60, 0)
            .single()
            .expect("valid pruned invocation time")
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_STRUCTURED_ONLY)
    .execute(&state.pool)
    .await
    .expect("seed pruned invocation");
    sqlx::query(
        "INSERT INTO parallel_work_rollup_coverage_state (id, full_detail_start_epoch) VALUES (1, ?1)",
    )
    .bind(hour_start - 3_600)
    .execute(&state.pool)
    .await
    .expect("seed historical coverage start");

    let mut tx = state.pool.begin().await.expect("begin recompute");
    rebuild_parallel_work_rollups_for_hours_tx(tx.as_mut(), &[hour_start])
        .await
        .expect("skip destructive rebuild");
    tx.commit().await.expect("commit recompute");

    let key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_minute_key_rollup WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2",
    )
    .bind(hour_start)
    .bind(hour_start + 3_600)
    .fetch_one(&state.pool)
    .await
    .expect("count preserved covered keys");
    assert_eq!(key_count, 1);
}

#[tokio::test]
async fn parallel_work_maintenance_rolls_back_minute_deletion_when_scalar_write_fails() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let expired_hour = parallel_work_minute_rollup_keep_start_epoch(Utc::now())
        .expect("minute-rollup retention start")
        - 3_600;
    for source_scope in ["all", "proxy_only"] {
        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            )
            VALUES (?1, ?2, 1, 0)
            "#,
        )
        .bind(expired_hour)
        .bind(source_scope)
        .execute(&state.pool)
        .await
        .expect("seed complete minute coverage");
    }
    sqlx::query(
        r#"
        INSERT INTO parallel_work_minute_key_rollup (
            minute_start_epoch, source, prompt_cache_key
        )
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(expired_hour + 5 * 60)
    .bind(SOURCE_PROXY)
    .bind("pck-transaction-rollback")
    .execute(&state.pool)
    .await
    .expect("seed expired minute key");
    sqlx::query(
        r#"
        CREATE TRIGGER fail_parallel_work_hourly_scalar
        BEFORE INSERT ON parallel_work_hourly_rollup
        BEGIN
            SELECT RAISE(ABORT, 'injected hourly scalar failure');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install scalar failure trigger");

    let error = maintain_parallel_work_rollups(
        &state.pool,
        Some(shanghai_retention_cutoff(state.config.invocation_max_days).timestamp()),
    )
    .await
    .expect_err("scalar failure should abort materialization");
    assert!(error.to_string().contains("injected hourly scalar failure"));

    let minute_key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM parallel_work_minute_key_rollup \
         WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2",
    )
    .bind(expired_hour)
    .bind(expired_hour + 3_600)
    .fetch_one(&state.pool)
    .await
    .expect("count rolled-back minute keys");
    assert_eq!(minute_key_count, 1);
}
