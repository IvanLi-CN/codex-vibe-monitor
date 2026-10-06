use super::*;
use crate::maintenance::{RETENTION_TEST_DB_PRESSURE_GATE, RETENTION_TEST_WRITE_COORDINATOR};
use crate::proxy_sqlite_write_coordinator::{
    ProxySqliteWriteClass, test_proxy_sqlite_write_coordinator,
};
use std::{
    fs::File,
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

fn setting(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn payload(index: usize) -> String {
    let mut state = index as u64 + 1;
    let mut entropy = String::with_capacity(128);
    for _ in 0..16 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        entropy.push_str(&format!("{state:08x}"));
    }
    serde_json::json!({
        "promptCacheKey": if index < 500_000 { "service-rate-hot-key".to_string() } else { format!("key-{index}") },
        "padding": "model-output-token ".repeat(140),
        "identity": entropy,
    }).to_string()
}

async fn suspend_triggers(pool: &SqlitePool) -> Vec<String> {
    let definitions = sqlx::query_as::<_, (String, String)>(
        "SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name IN ('codex_invocations','pool_upstream_request_attempts') AND sql IS NOT NULL",
    ).fetch_all(pool).await.expect("fixture trigger definitions");
    for (name, _) in &definitions {
        sqlx::query(&format!("DROP TRIGGER {name}"))
            .execute(pool)
            .await
            .expect("suspend fixture triggers");
    }
    definitions.into_iter().map(|(_, sql)| sql).collect()
}

async fn seed(pool: &SqlitePool, config: &AppConfig, rows: usize) -> (i64, i64) {
    let triggers = suspend_triggers(pool).await;
    let dates = (0..10)
        .map(|month| {
            shanghai_local_days_ago(
                (config
                    .invocation_max_days
                    .max(config.pool_upstream_request_attempts_retention_days)
                    + 2
                    + month * 31) as i64,
                12,
                0,
                0,
            )
        })
        .collect::<Vec<_>>();
    for start in (0..rows).step_by(1_000) {
        let end = (start + 1_000).min(rows);
        let mut tx = pool.begin().await.expect("fixture transaction");
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) ",
        );
        query.push_values(start..end, |mut row, index| {
            row.push_bind(format!("service-cohort-{index}"))
                .push_bind(&dates[index * 10 / rows])
                .push_bind(SOURCE_PROXY)
                .push_bind("success")
                .push_bind(7_i64)
                .push_bind(0.07_f64)
                .push_bind(payload(index))
                .push_bind("{}");
        });
        query
            .build()
            .execute(&mut *tx)
            .await
            .expect("seed invocation cohort");
        let attempt_start = start * 6 / 5;
        let attempt_end = end * 6 / 5;
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO pool_upstream_request_attempts(attempt_public_id,invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status) ",
        );
        query.push_values(attempt_start..attempt_end, |mut row, index| {
            row.push_bind(format!("service-attempt-{index}"))
                .push_bind(format!("service-cohort-{}", index * 5 / 6))
                .push_bind(&dates[(index * 5 / 6 * 10 / rows).min(9)])
                .push_bind("/v1/responses")
                .push_bind("pool")
                .push_bind((index % 2) as i64)
                .push_bind(0_i64)
                .push_bind(0_i64)
                .push_bind("success");
        });
        query
            .build()
            .execute(&mut *tx)
            .await
            .expect("seed attempt cohort");
        tx.commit().await.expect("fixture commit");
        if start % 100_000 == 0 {
            eprintln!("service-rate-fixture seeded={end}/{rows}");
        }
    }
    let invocation_max: i64 = sqlx::query_scalar("SELECT MAX(id) FROM codex_invocations")
        .fetch_one(pool)
        .await
        .expect("invocation cohort boundary");
    let attempt_max: i64 = sqlx::query_scalar("SELECT MAX(id) FROM pool_upstream_request_attempts")
        .fetch_one(pool)
        .await
        .expect("attempt cohort boundary");
    // Seed a full day of ordinary arrivals independently of the expired cohort. Stable
    // timestamps keep the compared inputs equal and allow the production rate observer.
    let recent = shanghai_local_days_ago(0, 0, 0, 0);
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<30000) INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) SELECT 'service-recent-'||i,?1,'proxy','success',7,0.07,'{}','{}' FROM n")
        .bind(&recent).execute(pool).await.expect("ordinary arrival fixture");
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<36000) INSERT INTO pool_upstream_request_attempts(attempt_public_id,invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status) SELECT 'service-recent-attempt-'||i,'service-recent-'||(i*5/6),?1,'/v1/responses','pool',0,0,0,'success' FROM n")
        .bind(recent).execute(pool).await.expect("ordinary attempt fixture");
    for sql in triggers {
        sqlx::query(&sql)
            .execute(pool)
            .await
            .expect("restore production triggers");
    }
    for index in 0..64 {
        let path = config
            .proxy_raw_dir
            .join(format!("service-orphan-{index}.bin"));
        fs::write(&path, b"orphan capacity fixture").expect("sparse raw orphan");
        File::open(&path)
            .expect("orphan file")
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_secs(7 * 86_400)),
            )
            .expect("orphan age");
        let linked = config
            .proxy_raw_dir
            .join(format!("service-linked-{index}.bin"));
        fs::write(&linked, b"shared invocation and attempt raw fixture")
            .expect("linked raw fixture");
        let path = linked.to_string_lossy();
        sqlx::query("UPDATE codex_invocations SET response_raw_path=?1,response_raw_codec='identity',response_raw_size=40 WHERE id=?2")
            .bind(path.as_ref()).bind(index+1_i64).execute(pool).await.expect("invocation raw link");
        sqlx::query("UPDATE pool_upstream_request_attempts SET response_raw_path=?1,response_raw_codec='identity',response_raw_size=40 WHERE id=?2")
            .bind(path.as_ref()).bind(index+1_i64).execute(pool).await.expect("attempt raw link");
    }
    (invocation_max, attempt_max)
}

fn percentile(samples: &[u64], percent: usize) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "GitHub Actions only: optional single-task synthetic capacity diagnosis"]
async fn retention_task_local_capacity_fixture_diagnostic() {
    let directory = PathBuf::from(
        std::env::var("CVM_RETENTION_DIAGNOSTIC_FIXTURE").expect("owned fixture directory"),
    );
    assert!(directory.starts_with("/work") || directory.starts_with("/tmp"));
    let mut config = test_config();
    config.database_path = directory.join("codex-vibe-monitor.db");
    config.proxy_raw_dir = directory.join("proxy_raw_payloads");
    config.archive_dir = directory.join("archives");
    config.invocation_archive_ttl_days = 365;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            build_sqlite_connect_options(
                &test_sqlite_url_for_path(&config.database_path),
                Duration::from_secs(5),
            )
            .expect("WAL options"),
        )
        .await
        .expect("fixture pool");
    ensure_schema(&pool)
        .await
        .expect("current fixture definitions");
    let started = Instant::now();
    let summary = RETENTION_TEST_WRITE_COORDINATOR
        .scope(
            test_proxy_sqlite_write_coordinator(),
            RETENTION_TEST_DB_PRESSURE_GATE.scope(
                Arc::new(crate::db_pressure::DbPressureGate::new(
                    1,
                    Duration::from_secs(30),
                )),
                run_data_retention_maintenance(&pool, &config, Some(false), None),
            ),
        )
        .await
        .expect("diagnostic task");
    eprintln!(
        "capacity-diagnostic elapsed={:?} summary={summary:?}",
        started.elapsed()
    );
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "GitHub Actions only: optional release-build service-rate/online-latency acceptance"]
async fn retention_task_local_service_rate_release_benchmark() {
    let rows = setting("CVM_RETENTION_SERVICE_ROWS", 1_270_000);
    let load_multiplier = setting("CVM_RETENTION_SERVICE_LOAD_MULTIPLIER", 1).min(20);
    let window = std::env::var("CVM_RETENTION_SERVICE_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let (pool, mut config, temp_dir) = retention_test_pool_and_config("service-rate-release").await;
    config.retention_batch_rows = 1_000;
    config.invocation_success_full_days = config.invocation_max_days;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let (invocation_max, attempt_max) = seed(&pool, &config, rows).await;
    let database_url = test_sqlite_url_for_path(&config.database_path);
    pool.close().await;
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            build_sqlite_connect_options(&database_url, Duration::from_secs(5))
                .expect("production WAL options"),
        )
        .await
        .expect("online/recovery pool");
    let coordinator = test_proxy_sqlite_write_coordinator();
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let stopped = Arc::new(AtomicBool::new(false));
    let load_started = Instant::now();
    let load_finished = move |stopped: &AtomicBool| {
        stopped.load(Ordering::Acquire)
            || window.is_some_and(|seconds| load_started.elapsed().as_secs() >= seconds)
    };
    let reader_stopped = stopped.clone();
    let reader_pool = pool.clone();
    let reader = tokio::spawn(async move {
        let mut samples = Vec::new();
        let mut requests = tokio::task::JoinSet::new();
        let mut sequence = 0usize;
        let mut ticker = tokio::time::interval(Duration::from_millis(960 / load_multiplier as u64));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        while !load_finished(&reader_stopped) {
            let scheduled = ticker.tick().await;
            if load_finished(&reader_stopped) {
                break;
            }
            while let Some(result) = requests.try_join_next() {
                samples.push(result.expect("online read sample"));
            }
            let request_pool = reader_pool.clone();
            let request_sequence = sequence;
            sequence += 1;
            requests.spawn(async move {
            // Rotate dashboard aggregate, live list and record lookup. The sequence is
            // deterministic and runs for the whole archive trial, not only its beginning.
            match request_sequence % 3 {
                0 => {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at >= ?1",
                    )
                    .bind(shanghai_local_days_ago(0, 0, 0, 0))
                    .fetch_one(&request_pool)
                    .await
                    .expect("online dashboard");
                }
                1 => {
                    sqlx::query("SELECT id,status,total_tokens,cost FROM codex_invocations ORDER BY occurred_at DESC,id DESC LIMIT 80")
                    .fetch_all(&request_pool).await.expect("online live list");
                }
                _ => {
                    sqlx::query("SELECT id,status,total_tokens,cost,payload FROM codex_invocations WHERE invoke_id=?1")
                    .bind(format!("service-recent-{}", request_sequence%30_000+1)).fetch_optional(&request_pool).await.expect("online detail");
                }
            }
            scheduled.elapsed().as_micros() as u64
            });
        }
        while let Some(result) = requests.join_next().await {
            samples.push(result.expect("settled online read sample"));
        }
        samples
    });
    let writer_stopped = stopped.clone();
    let writer_pool = pool.clone();
    let writer_coordinator = coordinator.clone();
    let writer = tokio::spawn(async move {
        let mut samples = Vec::new();
        let mut requests = tokio::task::JoinSet::new();
        let mut sequence = 0usize;
        let mut ticker =
            tokio::time::interval(Duration::from_millis(2_880 / load_multiplier as u64));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        while !load_finished(&writer_stopped) {
            let scheduled = ticker.tick().await;
            if load_finished(&writer_stopped) {
                break;
            }
            while let Some(result) = requests.try_join_next() {
                samples.push(result.expect("online write sample"));
            }
            let request_pool = writer_pool.clone();
            let request_coordinator = writer_coordinator.clone();
            let request_sequence = sequence;
            sequence += 1;
            requests.spawn(async move {
            let timestamp = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let permit = request_coordinator
                .acquire(ProxySqliteWriteClass::P1Terminal)
                .await;
            let mut tx = request_pool
                .begin()
                .await
                .expect("online terminal transaction");
            let invocation = format!("service-online-{request_sequence}");
            sqlx::query("INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) VALUES(?1,?2,'proxy','success',7,0.07,?3,'{}')")
                .bind(&invocation).bind(&timestamp).bind(payload(request_sequence)).execute(&mut *tx).await.expect("online invocation write");
            for attempt in 0..if request_sequence.is_multiple_of(5) { 2 } else { 1 } {
                sqlx::query("INSERT INTO pool_upstream_request_attempts(attempt_public_id,invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status) VALUES(?1,?2,?3,'/v1/responses','pool',?4,0,0,'success')")
                    .bind(format!("{invocation}-attempt-{attempt}")).bind(&invocation).bind(&timestamp).bind(attempt as i64)
                    .execute(&mut *tx).await.expect("online attempt write");
            }
            tx.commit().await.expect("online terminal commit");
            drop(permit);
            scheduled.elapsed().as_micros() as u64
            });
        }
        while let Some(result) = requests.join_next().await {
            samples.push(result.expect("settled online write sample"));
        }
        samples
    });
    // Exercise a real competing maintenance owner on the same pool, pressure gate,
    // and write coordinator. Synthetic online traffic alone does not prove the
    // accepted capacity contract under other background work.
    let background_stopped = stopped.clone();
    let background_pool = pool.clone();
    let background_gate = gate.clone();
    let background_coordinator = coordinator.clone();
    let background_shutdown = Arc::new(tokio::sync::Notify::new());
    let background_shutdown_wait = background_shutdown.clone();
    let background = tokio::spawn(async move {
        let control =
            Arc::new(crate::maintenance_store::PromptCacheMaterializationControl::default());
        let generation = control.initialize(true).generation;
        let mut runs = 0_u64;
        let mut scanned = 0_u64;
        let mut updated = 0_u64;
        let mut deferred = 0_u64;
        let mut admission_defers = 0_u64;
        let mut pressure_failures = 0_u64;
        let retry_interval = Duration::from_secs(crate::STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS);
        let mut ticker = tokio::time::interval(retry_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !load_finished(&background_stopped) {
            tokio::select! {
                _ = ticker.tick() => {},
                _ = background_shutdown_wait.notified() => break,
            }
            if load_finished(&background_stopped) {
                break;
            }
            let Ok(_background_permit) = background_gate.try_begin_background("startup_backfill")
            else {
                admission_defers += 1;
                continue;
            };
            let Some(_write_permit) =
                background_coordinator.try_acquire(ProxySqliteWriteClass::P2Derived)
            else {
                admission_defers += 1;
                continue;
            };
            let should_yield =
                || background_coordinator.p2_should_yield() || load_finished(&background_stopped);
            runs += 1;
            let result = crate::prompt_cache_conversations::run_prompt_cache_conversations_materialization_with_pressure_and_control(
                &background_pool,
                crate::STARTUP_BACKFILL_SCAN_LIMIT,
                Some(Duration::from_secs(crate::STARTUP_BACKFILL_RUN_BUDGET_SECS)),
                &should_yield,
                &control,
                generation,
            )
            .await;
            match result {
                Ok(run) => {
                    scanned += run.scanned;
                    updated += run.updated;
                    deferred += u64::from(run.deferred);
                }
                Err(error) => {
                    // The production owner records pressure before releasing admission,
                    // then retries after its existing active interval. A lock conflict
                    // must not silently terminate the competing owner in this replay.
                    assert!(
                        background_gate.record_error("startup_backfill", &error),
                        "unexpected competing Prompt materialization failure: {error:#}"
                    );
                    pressure_failures += 1;
                    ticker.reset_after(retry_interval);
                }
            }
        }
        (
            runs,
            scanned,
            updated,
            deferred,
            admission_defers,
            pressure_failures,
        )
    });
    let started = Instant::now();
    let mut runs = 0;
    let mut timeouts = 0;
    let mut invoked = 0usize;
    let mut attempted = 0usize;
    let mut remaining = (rows as i64, (rows * 6 / 5) as i64);
    while remaining != (0, 0) && window.is_none_or(|seconds| started.elapsed().as_secs() < seconds)
    {
        assert!(
            started.elapsed() < Duration::from_secs(86_400),
            "fixed cohort missed 24-hour capacity target"
        );
        let run_start = Instant::now();
        let summary = RETENTION_TEST_WRITE_COORDINATOR
            .scope(
                coordinator.clone(),
                RETENTION_TEST_DB_PRESSURE_GATE.scope(
                    gate.clone(),
                    run_data_retention_maintenance(&pool, &config, Some(false), None),
                ),
            )
            .await
            .expect("retention trial");
        eprintln!("retention-service-rate-stages {summary:?}");
        invoked += summary.invocation_rows_archived;
        attempted += summary.pool_upstream_request_attempt_rows_archived;
        timeouts += usize::from(summary.budget_exhausted);
        runs += 1;
        assert!(
            summary.fatal_error.is_none(),
            "archive failure: {:?}",
            summary.fatal_error
        );
        remaining = (
            sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations INDEXED BY idx_codex_invocations_occurred_at WHERE id<=?1")
                .bind(invocation_max)
                .fetch_one(&pool)
                .await
                .expect("fixed invocation remainder"),
            sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts INDEXED BY idx_pool_upstream_request_attempts_occurred_at WHERE id<=?1")
                .bind(attempt_max)
                .fetch_one(&pool)
                .await
                .expect("fixed attempt remainder"),
        );
        eprintln!(
            "retention-service-rate-run {}",
            serde_json::json!({"run":runs,"elapsedMs":run_start.elapsed().as_millis(),"invocations":summary.invocation_rows_archived,"attempts":summary.pool_upstream_request_attempt_rows_archived,"remainingInvocations":remaining.0,"remainingAttempts":remaining.1,"timeout":summary.budget_exhausted,"waitReason":summary.wait_reason})
        );
        // Production catchup lets other maintenance owners run between qualifications.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let manifests = sqlx::query_as::<_, (i64, String, String, String, i64, String)>(
        "SELECT id,dataset,file_path,sha256,row_count,status FROM archive_batches
         WHERE dataset IN ('codex_invocations','pool_upstream_request_attempts')",
    )
    .fetch_all(&pool)
    .await
    .expect("published manifest evidence");
    let mut archive_counts = (0_i64, 0_i64);
    for (id, dataset, path, digest, count, status) in &manifests {
        assert_eq!(status, ARCHIVE_STATUS_COMPLETED, "unfinished manifest {id}");
        assert_eq!(
            sha256_hex_file(Path::new(path)).expect("actual published digest"),
            *digest
        );
        if dataset == "codex_invocations" {
            assert!(
                summary_archive_snapshot_has_proof(&pool, *id, digest)
                    .await
                    .expect("semantic Summary proof")
            );
            archive_counts.0 += count;
        } else {
            archive_counts.1 += count;
        }
    }
    assert_eq!(
        archive_counts,
        (
            rows as i64 - remaining.0,
            (rows * 6 / 5) as i64 - remaining.1
        ),
        "source removal must equal proved archive rows"
    );
    if remaining == (0, 0) {
        let shared_links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path LIKE '%service-linked-%'")
            .fetch_one(&pool).await.expect("settled raw ownership");
        assert_eq!(shared_links, 0);
        for index in 0..64 {
            assert!(
                !config
                    .proxy_raw_dir
                    .join(format!("service-linked-{index}.bin"))
                    .exists()
            );
        }
    }
    stopped.store(true, Ordering::Release);
    background_shutdown.notify_waiters();
    let reads = reader.await.expect("online reader join");
    let writes = writer.await.expect("online writer join");
    let background = background.await.expect("background owner join");
    let seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "retention-service-rate-result {}",
        serde_json::json!({
            "rows":rows,"attemptRows":rows*6/5,"months":10,"hotRows":500_000.min(rows),
            "runs":runs,"timeouts":timeouts,"verifiedArchiveRows":archive_counts.0,"verifiedAttemptRows":archive_counts.1,"verifiedManifestCount":manifests.len(),"elapsedSeconds":seconds,"invocationServiceRate":invoked as f64/seconds,"attemptServiceRate":attempted as f64/seconds,
            "remainingInvocations":remaining.0,"remainingAttempts":remaining.1,"complete":remaining==(0,0),
            "readSamples":reads.len(),"writeSamples":writes.len(),"readP95Us":percentile(&reads,95),"readP99Us":percentile(&reads,99),"writeP95Us":percentile(&writes,95),"writeP99Us":percentile(&writes,99),
            "invocationArrivalRate":30_000.0/86_400.0,"attemptArrivalRate":36_000.0/86_400.0,"readWriteRatio":3,
            "readWriteRatioSource":"explicit conservative replay assumption; production HTTP method ratio unavailable",
            "loadMultiplier":load_multiplier,
            "backgroundTask":"prompt_cache_conversations_materialization",
            "backgroundRuns":background.0,"backgroundScanned":background.1,"backgroundUpdated":background.2,"backgroundDeferred":background.3,"backgroundAdmissionDefers":background.4,
            "backgroundPressureFailures":background.5,
            "backgroundIntervalSeconds":crate::STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS,"backgroundScanLimit":crate::STARTUP_BACKFILL_SCAN_LIMIT,"backgroundRunBudgetSeconds":crate::STARTUP_BACKFILL_RUN_BUDGET_SECS,
        })
    );
    assert!(
        background.0 > 0 && background.1 + background.2 > 0,
        "capacity trial did not exercise real competing background work"
    );
    if window.is_none() {
        assert_eq!(remaining, (0, 0));
        assert!(
            invoked as f64 / seconds >= 17.4,
            "invocation service rate below 50x target"
        );
        assert!(
            attempted as f64 / seconds >= 20.8,
            "attempt service rate below 50x target"
        );
        assert_eq!(
            timeouts, 0,
            "ordinary load repeatedly reached timeout fallback"
        );
    }
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
