use super::*;

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
