use super::*;
use std::time::{Duration, Instant};

const DEFAULT_TOTAL_ROWS: usize = 1_300_000;
const DEFAULT_HOT_ROWS: usize = 500_000;
const INSERT_BATCH: usize = 1_000;
const ONLINE_SAMPLES: usize = 512;
const ORPHAN_FILES: usize = 64;
const HOT_KEY: &str = "retention-capacity-hot-key";

fn benchmark_total_rows() -> usize {
    std::env::var("CVM_RETENTION_BENCH_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TOTAL_ROWS)
}

fn benchmark_hot_rows(total_rows: usize) -> usize {
    std::env::var("CVM_RETENTION_BENCH_HOT_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_HOT_ROWS)
        .min(total_rows)
}

async fn suspend_invocation_triggers(pool: &SqlitePool) -> Vec<String> {
    let trigger_sql = sqlx::query_as::<_, (String, String)>(
        "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'codex_invocations' AND sql IS NOT NULL ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("list invocation triggers for fixture load");
    for (name, _) in &trigger_sql {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {name}"))
            .execute(pool)
            .await
            .expect("suspend invocation trigger for fixture load");
    }
    trigger_sql.into_iter().map(|(_, sql)| sql).collect()
}

async fn restore_invocation_triggers(pool: &SqlitePool, trigger_sql: &[String]) {
    for sql in trigger_sql {
        sqlx::query(sql)
            .execute(pool)
            .await
            .expect("restore invocation trigger after fixture load");
    }
}

fn percentile_micros(samples: &[u128], percentile: usize) -> u128 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() * percentile).saturating_add(99) / 100).max(1) - 1;
    sorted[rank.min(sorted.len().saturating_sub(1))]
}

async fn seed_retention_capacity_fixture(pool: &SqlitePool, config: &AppConfig) -> usize {
    let total_rows = benchmark_total_rows();
    let hot_rows = benchmark_hot_rows(total_rows);
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    let mut transaction = pool
        .begin()
        .await
        .expect("begin retention capacity fixture load");
    for start in (0..total_rows).step_by(INSERT_BATCH) {
        let end = (start + INSERT_BATCH).min(total_rows);
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) ",
        );
        query.push_values(start..end, |mut row, index| {
            let payload = if index < hot_rows {
                format!(r#"{{"promptCacheKey":"{HOT_KEY}"}}"#)
            } else {
                "{}".to_string()
            };
            row.push_bind(format!("retention-capacity-{index:07}"))
                .push_bind(&occurred_at)
                .push_bind(SOURCE_PROXY)
                .push_bind("success")
                .push_bind(7_i64)
                .push_bind(0.07_f64)
                .push_bind(payload)
                .push_bind("{}");
        });
        query
            .build()
            .execute(&mut *transaction)
            .await
            .expect("insert retention capacity fixture batch");
    }
    transaction
        .commit()
        .await
        .expect("commit retention capacity fixture");

    let raw_dir = &config.proxy_raw_dir;
    for index in 0..ORPHAN_FILES {
        fs::write(
            raw_dir.join(format!("retention-capacity-orphan-{index:03}.bin")),
            b"sparse-orphan-fixture",
        )
        .expect("write sparse orphan fixture");
    }
    total_rows
}

#[tokio::test]
#[ignore = "shared-testbox retention capacity and online-latency acceptance"]
async fn retention_capacity_fixed_cohort_candidate_benchmark() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-capacity-fixed-cohort").await;
    let total_rows = benchmark_total_rows();
    let hot_rows = benchmark_hot_rows(total_rows);
    let db_url = test_sqlite_url_for_path(&config.database_path);
    pool.close().await;
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(30))
                .expect("build retention capacity sqlite options"),
        )
        .await
        .expect("reopen retention capacity sqlite pool");
    config.retention_batch_rows = 64;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;

    let invocation_triggers = suspend_invocation_triggers(&pool).await;
    let cohort = seed_retention_capacity_fixture(&pool, &config).await;
    restore_invocation_triggers(&pool, &invocation_triggers).await;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at_id ON codex_invocations (occurred_at, id)",
    )
    .execute(&pool)
    .await
    .expect("ensure fixed-cohort retention index");
    // Keep the hot prompt-cache key in the invocation payload to preserve skew, but do not
    // create a candidate conversation identity here. Prompt identity orphan probes are covered
    // by the safety regressions; A7 measures the retention archive path and raw-file boundary.
    let cohort_source_max: i64 = sqlx::query_scalar("SELECT MAX(id) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("capture fixed retention cohort source max");
    let online_pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(30))
                .expect("build online retention capacity sqlite options"),
        )
        .await
        .expect("open online retention capacity pool");
    sqlx::query(
        "CREATE TABLE retention_capacity_online_writes (id INTEGER PRIMARY KEY AUTOINCREMENT, written_at TEXT NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("create online write probe");

    let reader_pool = online_pool.clone();
    let reader = tokio::spawn(async move {
        let mut samples = Vec::with_capacity(ONLINE_SAMPLES);
        while samples.len() < ONLINE_SAMPLES {
            let started_at = Instant::now();
            sqlx::query_scalar::<_, i64>(
                "SELECT id FROM codex_invocations WHERE id > ?1 ORDER BY id LIMIT 1",
            )
            .bind(0_i64)
            .fetch_one(&reader_pool)
            .await
            .expect("sample online retention read");
            samples.push(started_at.elapsed().as_micros());
            tokio::task::yield_now().await;
        }
        samples
    });
    let writer_pool = online_pool.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..ONLINE_SAMPLES {
            let mut lock_retries = 0_u16;
            loop {
                let result = sqlx::query(
                    "INSERT INTO retention_capacity_online_writes(written_at) VALUES (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now'))",
                )
                .execute(&writer_pool)
                .await
                .map_err(anyhow::Error::from);
                match result {
                    Ok(_) => break,
                    Err(error) if is_sqlite_lock_error(&error) && lock_retries < 400 => {
                        lock_retries = lock_retries.saturating_add(1);
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => panic!("sample online retention write: {error}"),
                }
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let started_at = Instant::now();
    let adaptive = std::env::var_os("CVM_RETENTION_TEST_ADAPTIVE").is_some();
    let run_label = if adaptive { "candidate" } else { "baseline" };
    let mut run_count = 0_usize;
    let mut submitted = 0_usize;
    let mut observed_archived = 0_usize;
    let mut budget_exhausted_runs = 0_usize;
    let mut lock_retry_attempts = 0_usize;
    let mut recoverable_retry_attempts = 0_usize;
    let deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
    loop {
        assert!(
            Instant::now() < deadline,
            "fixed retention cohort did not clear within the 24-hour acceptance budget"
        );
        let run_started_at = Instant::now();
        let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
            .await
            .expect("run candidate retention capacity pass");
        run_count = run_count.saturating_add(1);
        submitted = submitted.saturating_add(summary.invocation_rows_archived);
        budget_exhausted_runs += usize::from(summary.budget_exhausted);
        eprintln!(
            "retention-capacity-{run_label} run={} adaptive={} elapsed_ms={} archived={} backlog_remaining={:?} deferred={} recoverable_failure={} fatal_error={:?} budget_exhausted={} wait_reason={:?}",
            run_count,
            adaptive,
            run_started_at.elapsed().as_millis(),
            summary.invocation_rows_archived,
            summary
                .backlog_total
                .map(|value| value.saturating_sub(summary.invocation_rows_archived as i64)),
            summary.deferred,
            summary.recoverable_failure,
            summary.fatal_error,
            summary.budget_exhausted,
            summary.wait_reason,
        );
        eprintln!("retention-capacity-{run_label} stages={:?}", summary);
        let lock_pressure = summary
            .fatal_error
            .as_deref()
            .is_some_and(|error| error.contains("database is locked"))
            || summary.wait_reason.as_deref().is_some_and(|reason| {
                matches!(reason, "sqlite_pressure" | "retention_write_admission")
            });
        if summary.invocation_rows_archived == 0 && lock_pressure {
            lock_retry_attempts = lock_retry_attempts.saturating_add(1);
            assert!(
                lock_retry_attempts < 1_000,
                "retention capacity benchmark stayed locked for too many retries"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if summary.invocation_rows_archived == 0 && summary.recoverable_failure {
            recoverable_retry_attempts = recoverable_retry_attempts.saturating_add(1);
            assert!(
                recoverable_retry_attempts < 100,
                "retention capacity benchmark stayed in recoverable failure for too many retries"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1 AND id <= ?2",
        )
        .bind(shanghai_local_cutoff_string(config.invocation_max_days))
        .bind(cohort_source_max)
        .fetch_one(&pool)
        .await
        .expect("count fixed retention cohort");
        let previous_archived = observed_archived;
        let previous_remaining = cohort.saturating_sub(previous_archived);
        observed_archived = cohort.saturating_sub(remaining.max(0) as usize);
        assert!(
            observed_archived >= previous_archived,
            "fixed cohort count moved backwards while measuring retention progress"
        );
        if remaining == 0 {
            break;
        }
        assert!(
            summary.invocation_rows_archived > 0 || (remaining as usize) < previous_remaining,
            "retention pass made no archive progress while the fixed cohort remained"
        );
    }
    let read_samples = reader.await.expect("join online retention reader");
    writer.await.expect("join online retention writer");
    let elapsed_ms = started_at.elapsed().as_millis();
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1 AND id <= ?2",
    )
    .bind(shanghai_local_cutoff_string(config.invocation_max_days))
    .bind(cohort_source_max)
    .fetch_one(&pool)
    .await
    .expect("verify fixed cohort completion");
    assert_eq!(remaining, 0);
    assert_eq!(observed_archived, cohort);
    eprintln!(
        "retention-capacity-{run_label} total_rows={} hot_rows={} orphan_files={} cohort_source_max={} runs={} adaptive={} elapsed_ms={} summary_archived={} observed_archived={} budget_exhausted_runs={} lock_retries={} recoverable_retries={} online_samples={} read_p95_us={} read_p99_us={} remaining={}",
        total_rows,
        hot_rows,
        ORPHAN_FILES,
        cohort_source_max,
        run_count,
        adaptive,
        elapsed_ms,
        submitted,
        observed_archived,
        budget_exhausted_runs,
        lock_retry_attempts,
        recoverable_retry_attempts,
        read_samples.len(),
        percentile_micros(&read_samples, 95),
        percentile_micros(&read_samples, 99),
        remaining,
    );

    online_pool.close().await;
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
