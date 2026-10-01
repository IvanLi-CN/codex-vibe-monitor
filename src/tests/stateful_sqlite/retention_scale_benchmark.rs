use super::*;
use std::time::{Duration, Instant};

const HOT_KEY: &str = "retention-benchmark-hot-key";
const TOTAL_ROWS: usize = 1_300_000;
const HOT_ROWS: usize = 500_000;
const INSERT_BATCH: usize = 500;
const READ_SAMPLES: usize = 512;

fn percentile_micros(samples: &[u128], percentile: usize) -> u128 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() * percentile).saturating_add(99) / 100).max(1) - 1;
    sorted[rank.min(sorted.len().saturating_sub(1))]
}

#[tokio::test]
#[ignore = "manual million-row retention and prompt-cache benchmark"]
async fn retention_prompt_cache_million_row_candidate_benchmark() {
    let temp_dir = make_temp_test_dir("retention-prompt-cache-million-row");
    let db_path = temp_dir.join("benchmark.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(30))
                .expect("build benchmark sqlite options"),
        )
        .await
        .expect("open benchmark sqlite database");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create benchmark invocation schema");
    sqlx::query("CREATE TABLE prompt_cache_benchmark_writes (id INTEGER PRIMARY KEY AUTOINCREMENT, written_at TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create benchmark write probe");

    let mut transaction = pool.begin().await.expect("begin benchmark fixture load");
    for start in (0..TOTAL_ROWS).step_by(INSERT_BATCH) {
        let end = (start + INSERT_BATCH).min(TOTAL_ROWS);
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) ",
        );
        query.push_values(start..end, |mut row, index| {
            let payload = if index < HOT_ROWS {
                format!(r#"{{"promptCacheKey":"{HOT_KEY}"}}"#)
            } else {
                "{}".to_string()
            };
            row.push_bind(format!("retention-benchmark-{index:07}"))
                .push_bind("2024-01-01 00:00:00")
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
            .expect("insert benchmark invocation batch");
    }
    transaction
        .commit()
        .await
        .expect("commit benchmark fixture load");

    ensure_schema(&pool)
        .await
        .expect("install prompt-cache schema after fixture load");
    ensure_prompt_cache_conversation_row(&pool, HOT_KEY)
        .await
        .expect("create benchmark prompt-cache identity");
    sqlx::query(
        "INSERT INTO prompt_cache_conversation_stats_generation_clock(prompt_cache_key,generation) VALUES (?1,1) ON CONFLICT(prompt_cache_key) DO UPDATE SET generation=1",
    )
    .bind(HOT_KEY)
    .execute(&pool)
    .await
    .expect("seed benchmark generation clock");
    sqlx::query(
        "INSERT INTO prompt_cache_conversation_stats_refresh_queue(prompt_cache_key,generation) VALUES (?1,1) ON CONFLICT(prompt_cache_key) DO UPDATE SET generation=1",
    )
    .bind(HOT_KEY)
    .execute(&pool)
    .await
    .expect("seed benchmark refresh queue");

    let read_pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(30))
                .expect("build benchmark reader options"),
        )
        .await
        .expect("open benchmark reader database");
    let reader_pool = read_pool.clone();
    let reader = tokio::spawn(async move {
        let mut samples = Vec::with_capacity(READ_SAMPLES);
        while samples.len() < READ_SAMPLES {
            let started_at = Instant::now();
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations WHERE id > ?1")
                .bind(0_i64)
                .fetch_one(&reader_pool)
                .await
                .expect("run benchmark online read");
            samples.push(started_at.elapsed().as_micros());
            tokio::task::yield_now().await;
        }
        samples
    });
    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..READ_SAMPLES {
            sqlx::query("INSERT INTO prompt_cache_benchmark_writes(written_at) VALUES (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now'))")
                .execute(&writer_pool)
                .await
                .expect("run benchmark online write");
            tokio::task::yield_now().await;
        }
    });

    let started_at = Instant::now();
    let mut pages = 0_usize;
    let key_set = std::collections::HashSet::from([HOT_KEY.to_string()]);
    loop {
        let mut lock_retries = 0_u16;
        loop {
            match refresh_prompt_cache_conversation_stats(&pool, &key_set).await {
                Ok(_) => break,
                Err(error) if is_sqlite_lock_error(&error) && lock_retries < 200 => {
                    lock_retries = lock_retries.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => panic!("run bounded million-row prompt-cache page: {error:#}"),
            }
        }
        pages = pages.saturating_add(1);
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key=?1",
        )
        .bind(HOT_KEY)
        .fetch_one(&pool)
        .await
        .expect("read benchmark queue");
        if pending == 0 {
            break;
        }
        assert!(
            pages < 4_096,
            "million-row bounded refresh did not converge"
        );
    }
    let read_samples = reader.await.expect("join benchmark online reader");
    writer.await.expect("join benchmark online writer");
    let elapsed_ms = started_at.elapsed().as_millis();
    let (request_count, queue_count): (i64, i64) = sqlx::query_as(
        "SELECT request_count, (SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key=?1) FROM prompt_cache_conversations WHERE prompt_cache_key=?1",
    )
    .bind(HOT_KEY)
    .fetch_one(&pool)
    .await
    .expect("read benchmark exact aggregate");
    assert_eq!(request_count, HOT_ROWS as i64);
    assert_eq!(queue_count, 0);
    eprintln!(
        "retention-million-row candidate total_rows={} hot_rows={} pages={} elapsed_ms={} read_samples={} read_p95_us={} read_p99_us={} exact_request_count={} queue_count={}",
        TOTAL_ROWS,
        HOT_ROWS,
        pages,
        elapsed_ms,
        read_samples.len(),
        percentile_micros(&read_samples, 95),
        percentile_micros(&read_samples, 99),
        request_count,
        queue_count,
    );

    pool.close().await;
    read_pool.close().await;
    let _ = fs::remove_dir_all(temp_dir);
}
