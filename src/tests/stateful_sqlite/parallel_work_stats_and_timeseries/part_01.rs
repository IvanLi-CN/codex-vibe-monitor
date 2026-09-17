pub(crate) static TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
pub(crate) static SUMMARY_PROJECTION_ARCHIVE_IDENTITY_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

pub(crate) async fn insert_parallel_work_prompt_cache_rollup_hourly_row(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    prompt_cache_key: &str,
    request_count: i64,
) {
    insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
        pool,
        bucket_start,
        prompt_cache_key,
        SOURCE_PROXY,
        request_count,
    )
    .await;
}

pub(crate) async fn insert_parallel_work_prompt_cache_rollup_hourly_row_with_source(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    prompt_cache_key: &str,
    source: &str,
    request_count: i64,
) {
    let first_seen_at = format_naive(bucket_start.with_timezone(&Shanghai).naive_local());
    let last_seen_at = format_naive(
        (bucket_start + ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_rollup_hourly (
            bucket_start_epoch,
            source,
            prompt_cache_key,
            request_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_seen_at,
            last_seen_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
        "#,
    )
    .bind(bucket_start.timestamp())
    .bind(source)
    .bind(prompt_cache_key)
    .bind(request_count)
    .bind(request_count)
    .bind(0_i64)
    .bind(request_count * 10)
    .bind(request_count as f64 * 0.01)
    .bind(first_seen_at)
    .bind(last_seen_at)
    .execute(pool)
    .await
    .expect("insert prompt cache hourly rollup row");
}

pub(crate) async fn insert_prompt_cache_working_set_live_row(
    pool: &SqlitePool,
    prompt_cache_key: &str,
    created_at: &str,
    last_activity_at: &str,
    proxy_created_at: Option<&str>,
    proxy_last_activity_at: Option<&str>,
    source_scope_proxy_only: bool,
) {
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_working_set_live (
            prompt_cache_key,
            source_scope_all,
            source_scope_proxy_only,
            created_at,
            last_activity_at,
            last_terminal_at,
            last_in_flight_at,
            sort_anchor_at,
            request_count,
            total_tokens,
            total_cost,
            proxy_created_at,
            proxy_last_activity_at,
            proxy_last_terminal_at,
            proxy_last_in_flight_at,
            proxy_sort_anchor_at,
            proxy_request_count,
            proxy_total_tokens,
            proxy_total_cost
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19
        )
        "#,
    )
    .bind(prompt_cache_key)
    .bind(1_i64)
    .bind(if source_scope_proxy_only {
        1_i64
    } else {
        0_i64
    })
    .bind(created_at)
    .bind(last_activity_at)
    .bind(Some(last_activity_at))
    .bind(Option::<&str>::None)
    .bind(last_activity_at)
    .bind(1_i64)
    .bind(100_i64)
    .bind(0.01_f64)
    .bind(proxy_created_at)
    .bind(proxy_last_activity_at)
    .bind(proxy_last_activity_at)
    .bind(Option::<&str>::None)
    .bind(proxy_last_activity_at)
    .bind(if proxy_created_at.is_some() {
        1_i64
    } else {
        0_i64
    })
    .bind(if proxy_created_at.is_some() {
        100_i64
    } else {
        0_i64
    })
    .bind(if proxy_created_at.is_some() {
        0.01_f64
    } else {
        0.0_f64
    })
    .execute(pool)
    .await
    .expect("insert prompt cache working-set row");
}

pub(crate) async fn insert_parallel_work_prompt_cache_upstream_account_hourly_row(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    prompt_cache_key: &str,
    upstream_account_id: i64,
    request_count: i64,
) {
    let first_seen_at = format_naive(bucket_start.with_timezone(&Shanghai).naive_local());
    let last_seen_at = format_naive(
        (bucket_start + ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_upstream_account_hourly (
            bucket_start_epoch,
            source,
            prompt_cache_key,
            upstream_account_key,
            upstream_account_id,
            upstream_account_name,
            request_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_seen_at,
            last_seen_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
        "#,
    )
    .bind(bucket_start.timestamp())
    .bind(SOURCE_PROXY)
    .bind(prompt_cache_key)
    .bind(format!("account:{upstream_account_id}"))
    .bind(upstream_account_id)
    .bind(format!("Account {upstream_account_id}"))
    .bind(request_count)
    .bind(request_count)
    .bind(0_i64)
    .bind(request_count * 10)
    .bind(request_count as f64 * 0.01)
    .bind(first_seen_at)
    .bind(last_seen_at)
    .execute(pool)
    .await
    .expect("insert prompt cache upstream-account hourly row");
}

pub(crate) async fn seed_invocation_archive_batch(
    pool: &SqlitePool,
    config: &AppConfig,
    batch_name: &str,
    rows: &[(i64, &str, &str, &str, &str, i64, f64, Option<f64>)],
) -> PathBuf {
    let rows = rows
        .iter()
        .map(
            |(id, invoke_id, occurred_at, source, status, total_tokens, cost, ttfb_ms)| {
                SeedInvocationArchiveBatchRow {
                    id: *id,
                    invoke_id,
                    occurred_at,
                    source,
                    status,
                    total_tokens: *total_tokens,
                    cost: *cost,
                    ttfb_ms: *ttfb_ms,
                    payload: Some("{}"),
                    detail_level: DETAIL_LEVEL_FULL,
                    error_message: None,
                    failure_kind: None,
                    failure_class: Some("none"),
                    is_actionable: Some(0),
                }
            },
        )
        .collect::<Vec<_>>();
    seed_invocation_archive_batch_with_details(pool, config, batch_name, &rows).await
}

pub(crate) async fn run_background_invocation_summary_rollup_repair(pool: &SqlitePool) {
    crate::stats::ensure_invocation_summary_rollups_ready(pool)
        .await
        .expect("run background invocation summary rollup repair");
}

pub(crate) async fn insert_materialized_rollup_bucket_marker(
    pool: &SqlitePool,
    target: &str,
    bucket_start_epoch: i64,
    source: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(bucket_start_epoch)
    .bind(source)
    .execute(pool)
    .await
    .expect("insert materialized rollup bucket marker");
}

pub(crate) async fn insert_hourly_rollup_archive_replay_marker(
    pool: &SqlitePool,
    target: &str,
    file_path: &Path,
) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay
            (target, dataset, file_path, archive_sha256, replayed_at)
        SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
        FROM archive_batches AS batches
        WHERE batches.dataset = ?2 AND batches.file_path = ?3
        "#,
    )
    .bind(target)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert hourly rollup archive replay marker");
}

pub(crate) async fn mark_summary_archive_replay_complete(pool: &SqlitePool, file_path: &Path) {
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        insert_hourly_rollup_archive_replay_marker(pool, target, file_path).await;
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SeedInvocationArchiveBatchRow<'a> {
    pub(crate) id: i64,
    pub(crate) invoke_id: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) source: &'a str,
    pub(crate) status: &'a str,
    pub(crate) total_tokens: i64,
    pub(crate) cost: f64,
    pub(crate) ttfb_ms: Option<f64>,
    pub(crate) payload: Option<&'a str>,
    pub(crate) detail_level: &'a str,
    pub(crate) error_message: Option<&'a str>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) failure_class: Option<&'a str>,
    pub(crate) is_actionable: Option<i64>,
}

pub(crate) async fn seed_invocation_archive_batch_with_details(
    pool: &SqlitePool,
    config: &AppConfig,
    batch_name: &str,
    rows: &[SeedInvocationArchiveBatchRow<'_>],
) -> PathBuf {
    let month_key = rows
        .first()
        .map(|row| row.occurred_at[..7].to_string())
        .expect("archive batch rows should not be empty");
    let archive_path = config.archive_dir.join(format!(
        "codex_invocations-{batch_name}-{month_key}.sqlite.gz"
    ));
    fs::create_dir_all(
        archive_path
            .parent()
            .expect("invocation archive batch should have parent"),
    )
    .expect("create invocation archive dir");

    let archive_db_path = config.archive_dir.join(format!("{batch_name}.sqlite"));
    let _ = fs::remove_file(&archive_db_path);
    fs::File::create(&archive_db_path).expect("create invocation archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open invocation archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create invocation archive schema");
    for row in rows {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, payload, detail_level, error_message, failure_kind, failure_class, is_actionable, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(row.id)
        .bind(row.invoke_id)
        .bind(row.occurred_at)
        .bind(row.source)
        .bind(row.status)
        .bind(row.total_tokens)
        .bind(row.cost)
        .bind(row.ttfb_ms)
        .bind(row.payload)
        .bind(row.detail_level)
        .bind(row.error_message)
        .bind(row.failure_kind)
        .bind(row.failure_class)
        .bind(row.is_actionable)
        .bind("{}")
        .bind(row.occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert invocation archive row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress invocation archive batch");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(pool)
    .await
    .expect("insert invocation archive batch manifest");
    archive_path
}

pub(crate) fn bucket_date_in_tz(bucket_start: &str, tz: Tz) -> NaiveDate {
    DateTime::parse_from_rfc3339(bucket_start)
        .expect("valid bucket start")
        .with_timezone(&tz)
        .date_naive()
}

pub(crate) fn shanghai_bucket_date(bucket_start: &str) -> NaiveDate {
    bucket_date_in_tz(bucket_start, Shanghai)
}

pub(crate) fn archived_start_partial_hour_at() -> String {
    let range_start = Utc::now() - ChronoDuration::days(7);
    let range_start_epoch = range_start.timestamp();
    let floor_hour_epoch = range_start_epoch - range_start_epoch.rem_euclid(3_600);
    let first_full_hour_epoch = if floor_hour_epoch < range_start_epoch {
        floor_hour_epoch + 3_600
    } else {
        floor_hour_epoch
    };
    let first_full_hour = Utc
        .timestamp_opt(first_full_hour_epoch, 0)
        .single()
        .expect("valid first full hour");
    let latest_partial_second = first_full_hour - ChronoDuration::seconds(1);
    let preferred = range_start + ChronoDuration::minutes(5);
    let occurred_at = if latest_partial_second < range_start {
        preferred
    } else {
        preferred.min(latest_partial_second)
    };
    format_naive(occurred_at.with_timezone(&Shanghai).naive_local())
}

pub(crate) fn assert_f64_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff < 1e-6,
        "expected {expected}, got {actual}, diff={diff}"
    );
}

pub(crate) async fn archive_retention_test_state() -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    test_state_from_config(config, true).await
}

pub(crate) async fn fetch_test_summary(state: Arc<AppState>, window: &str) -> StatsResponse {
    fetch_test_summary_in_zone(state, window, "Asia/Shanghai").await
}

pub(crate) async fn fetch_test_summary_in_zone(
    state: Arc<AppState>,
    window: &str,
    time_zone: &str,
) -> StatsResponse {
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some(window.to_string()),
            limit: None,
            time_zone: Some(time_zone.to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch test summary");
    summary
}

pub(crate) async fn fetch_test_timeseries(
    state: Arc<AppState>,
    range: &str,
    bucket: &str,
) -> TimeseriesResponse {
    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: range.to_string(),
            bucket: Some(bucket.to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch test timeseries");
    timeseries
}

pub(crate) async fn fetch_test_failure_summary(
    state: Arc<AppState>,
    range: &str,
) -> FailureSummaryResponse {
    let Json(summary) = fetch_failure_summary(
        State(state),
        Query(FailureSummaryQuery {
            range: range.to_string(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch test failure summary");
    summary
}

async fn seed_distinct_prompt_cache_keys_per_bucket(
    pool: &SqlitePool,
    minute_a: DateTime<Utc>,
    minute_b: DateTime<Utc>,
) {
    insert_parallel_work_invocation(
        pool,
        "parallel-minute-a-1",
        minute_a + ChronoDuration::seconds(10),
        "pck-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        pool,
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
    .execute(pool)
    .await
    .expect("insert cross-source parallel-work key");
    for (invoke_id, occurred_at, prompt_cache_key) in [
        (
            "parallel-minute-b-1",
            minute_b + ChronoDuration::seconds(10),
            "pck-alpha",
        ),
        (
            "parallel-minute-b-2",
            minute_b + ChronoDuration::seconds(20),
            "pck-beta",
        ),
        (
            "parallel-minute-b-3",
            minute_b + ChronoDuration::seconds(30),
            "pck-gamma",
        ),
    ] {
        insert_parallel_work_invocation(pool, invoke_id, occurred_at, prompt_cache_key).await;
    }
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
    .execute(pool)
    .await
    .expect("assign upstream account to parallel-work keys");
}

#[tokio::test]
pub(crate) async fn parallel_work_stats_counts_distinct_prompt_cache_keys_per_bucket() {
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
    seed_distinct_prompt_cache_keys_per_bucket(&state.pool, minute_a, minute_b).await;

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
pub(crate) async fn parallel_work_active_minute_stats_uses_verified_hourly_scalars_for_history() {
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
pub(crate) async fn parallel_work_active_minute_stats_rejects_a_raw_retention_gap_without_rollup_coverage()
 {
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
pub(crate) async fn parallel_work_active_minute_stats_accepts_partial_hour_minute_rollup_ranges() {
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
pub(crate) async fn parallel_work_active_minute_stats_uses_pool_attempt_account_fallback_for_raw_tail()
 {
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
pub(crate) async fn parallel_work_maintenance_materializes_before_expiring_minute_keys() {
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

use super::*;
