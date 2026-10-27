use super::*;
#[tokio::test]
async fn prompt_cache_conversation_materialization_checkpoints_and_resumes_with_new_keys() {
    let temp_dir = make_temp_test_dir("prompt-cache-conversation-materialization-restart");
    let db_path = temp_dir.join("state.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(5))
                .expect("build sqlite options"),
        )
        .await
        .expect("file-backed sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache conversation structure");

    async fn insert_invocation(pool: &Pool<Sqlite>, invoke_id: &str, prompt_cache_key: &str) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, '2026-09-01 00:00:00', ?2, 'success', 7, 0.07, ?3, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": prompt_cache_key}).to_string())
        .execute(pool)
        .await
        .expect("insert checkpoint invocation");
    }

    insert_invocation(&pool, "checkpoint-a", "checkpoint-a").await;
    insert_invocation(&pool, "checkpoint-b", "checkpoint-b").await;

    let first_page = run_prompt_cache_conversations_materialization(&pool, 1, None)
        .await
        .expect("run first identity page");
    assert_eq!(first_page.phase, "identity_backfill");
    assert_eq!(first_page.scanned, 1);
    assert!(!first_page.complete);
    let checkpoint = sqlx::query_as::<_, (String, i64, Option<String>)>(
        "SELECT phase, source_max_invocation_id, cursor_key \
         FROM prompt_cache_conversation_migration_progress \
         WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load materialization checkpoint");
    assert_eq!(checkpoint.0, "identity_backfill");
    assert!(checkpoint.1 > 0);
    assert_eq!(checkpoint.2.as_deref(), Some("checkpoint-a"));

    pool.close().await;
    let resumed_pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(
            build_sqlite_connect_options(&db_url, Duration::from_secs(5))
                .expect("build resumed sqlite options"),
        )
        .await
        .expect("reopen file-backed sqlite after checkpoint");
    ensure_schema(&resumed_pool)
        .await
        .expect("restart should preserve prompt-cache structure");
    insert_invocation(&resumed_pool, "checkpoint-c", "checkpoint-c").await;
    for _ in 0..24 {
        let outcome = run_prompt_cache_conversations_materialization(&resumed_pool, 1, None)
            .await
            .expect("resume prompt-cache materialization");
        if outcome.complete {
            break;
        }
    }

    assert!(
        prompt_cache_conversation_materialization_is_complete(&resumed_pool)
            .await
            .expect("check completed materialization")
    );
    let identity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
        .fetch_one(&resumed_pool)
        .await
        .expect("count materialized identities");
    assert_eq!(identity_count, 3);
    let total_requests: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_conversations",
    )
    .fetch_one(&resumed_pool)
    .await
    .expect("sum materialized statistics");
    assert_eq!(total_requests, 3);
    let queue_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue")
            .fetch_one(&resumed_pool)
            .await
            .expect("count drained refresh queue");
    assert_eq!(queue_count, 0);
    let resumed = run_prompt_cache_conversations_materialization(&resumed_pool, 1, None)
        .await
        .expect("rerun completed materialization");
    assert!(resumed.complete);
    resumed_pool.close().await;
    let _ = fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn prompt_cache_stats_generation_clock_survives_queue_drain() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache conversation structure");

    let payload = json!({"promptCacheKey": "generation-key"}).to_string();
    for invoke_id in ["generation-a", "generation-b"] {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response) VALUES (?1,'2026-09-01 00:00:00',?2,'success',?3,'{}')",
        )
        .bind(invoke_id)
        .bind(SOURCE_PROXY)
        .bind(&payload)
        .execute(&pool)
        .await
        .expect("insert generation fixture");
    }
    let first_generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key='generation-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load first queue generation");
    assert_eq!(first_generation, 2);

    for _ in 0..16 {
        if run_prompt_cache_conversations_materialization(&pool, 16, None)
            .await
            .expect("drain generation fixture")
            .complete
        {
            break;
        }
    }
    let queue_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key='generation-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("count drained generation queue");
    assert_eq!(queue_count, 0);

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response) VALUES ('generation-c','2026-09-01 00:00:00',?1,'success',?2,'{}')",
    )
    .bind(SOURCE_PROXY)
    .bind(&payload)
    .execute(&pool)
    .await
    .expect("insert post-drain generation fixture");
    let next_generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key='generation-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load post-drain queue generation");
    assert!(next_generation > first_generation);
}

#[tokio::test]
async fn prompt_cache_materialization_yields_only_after_a_committed_micro_batch() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache conversation structure");

    for index in 0..65 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, '2026-09-01 00:00:00', ?2, 'success', 7, 0.07, ?3, '{}')
            "#,
        )
        .bind(format!("pressure-invocation-{index:04}"))
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": format!("pressure-key-{index:04}")}).to_string())
        .execute(&pool)
        .await
        .expect("insert pressure fixture invocation");
    }

    let checks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let should_yield = {
        let checks = std::sync::Arc::clone(&checks);
        move || checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2
    };
    let deferred =
        run_prompt_cache_conversations_materialization_with_test_batch_size_and_pressure(
            &pool,
            65,
            None,
            32,
            &should_yield,
        )
        .await
        .expect("run pressure-aware materialization");
    assert!(deferred.deferred);
    assert_eq!(deferred.batch_count, 1);
    assert_eq!(deferred.scanned, 32);
    assert!(!deferred.page_complete);

    let materialized_before_yield: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
            .fetch_one(&pool)
            .await
            .expect("count committed micro-batch identities");
    assert_eq!(materialized_before_yield, 32);
    let checkpoint: Option<String> = sqlx::query_scalar(
        "SELECT cursor_key FROM prompt_cache_conversation_migration_progress \
         WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load committed micro-batch cursor");
    assert_eq!(checkpoint.as_deref(), Some("pressure-key-0031"));

    for _ in 0..16 {
        let outcome = run_prompt_cache_conversations_materialization_with_test_batch_size(
            &pool, 65, None, 32,
        )
        .await
        .expect("resume after pressure boundary");
        if outcome.complete {
            break;
        }
    }
    assert!(
        prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check pressure fixture completion")
    );
    let identity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
        .fetch_one(&pool)
        .await
        .expect("count completed identities");
    assert_eq!(identity_count, 65);
    let total_requests: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_conversations",
    )
    .fetch_one(&pool)
    .await
    .expect("sum completed statistics");
    assert_eq!(total_requests, 65);
}

#[tokio::test]
#[ignore = "controlled representative SQLite acceptance; run explicitly"]
async fn prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale() {
    async fn setup_scale_database(path: &std::path::Path) -> (Pool<Sqlite>, Pool<Sqlite>) {
        let db_url = test_sqlite_url_for_path(path);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(
                build_sqlite_connect_options(&db_url, Duration::from_secs(5))
                    .expect("build scale sqlite options"),
            )
            .await
            .expect("open scale sqlite database");
        sqlx::query(&codex_invocations_create_sql("codex_invocations"))
            .execute(&pool)
            .await
            .expect("create scale invocation schema");
        ensure_schema(&pool)
            .await
            .expect("install scale prompt-cache structure");

        let mut transaction = pool.begin().await.expect("begin scale fixture insert");
        for key_index in 0..4_000 {
            let prompt_cache_key = format!("scale-key-{key_index:04}");
            for invocation_index in 0..10 {
                sqlx::query(
                    r#"
                    INSERT INTO codex_invocations (
                        invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                    ) VALUES (?1, '2026-09-01 00:00:00', ?2, 'success', 7, 0.07, ?3, '{}')
                    "#,
                )
                .bind(format!("scale-{key_index:04}-{invocation_index:02}"))
                .bind(SOURCE_PROXY)
                .bind(json!({"promptCacheKey": prompt_cache_key}).to_string())
                .execute(&mut *transaction)
                .await
                .expect("insert scale fixture invocation");
            }
        }
        transaction
            .commit()
            .await
            .expect("commit scale fixture insert");

        let read_pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                build_sqlite_connect_options(&db_url, Duration::from_secs(5))
                    .expect("build scale reader sqlite options"),
            )
            .await
            .expect("open scale reader database");
        (pool, read_pool)
    }

    async fn run_scale_case(
        path: &std::path::Path,
        adaptive: bool,
    ) -> (Duration, Vec<u128>, i64, i64, i64) {
        let (pool, read_pool) = setup_scale_database(path).await;
        if adaptive {
            reset_prompt_cache_conversation_batch_controller_for_test();
        }
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let materialization_finished = std::sync::Arc::clone(&finished);
        let materialization = async {
            let started_at = std::time::Instant::now();
            for _ in 0..128 {
                let outcome = if adaptive {
                    run_prompt_cache_conversations_materialization(&pool, 400, None)
                        .await
                        .expect("run adaptive scale materialization")
                } else {
                    run_prompt_cache_conversations_materialization_with_test_batch_size(
                        &pool, 400, None, 400,
                    )
                    .await
                    .expect("run fixed scale materialization")
                };
                if outcome.complete {
                    materialization_finished.store(true, std::sync::atomic::Ordering::SeqCst);
                    return started_at.elapsed();
                }
            }
            panic!("scale materialization did not complete within the test budget");
        };
        let foreground_reads = async {
            const FOREGROUND_SAMPLE_COUNT: usize = 500;
            let mut latencies = Vec::with_capacity(FOREGROUND_SAMPLE_COUNT);
            while !finished.load(std::sync::atomic::Ordering::SeqCst)
                || latencies.len() < FOREGROUND_SAMPLE_COUNT
            {
                let started_at = std::time::Instant::now();
                sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_conversations",
                )
                .fetch_one(&read_pool)
                .await
                .expect("run foreground aggregate read");
                latencies.push(started_at.elapsed().as_micros());
                tokio::task::yield_now().await;
            }
            latencies
        };
        let (elapsed, latencies) = tokio::join!(materialization, foreground_reads);
        let identity_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
                .fetch_one(&pool)
                .await
                .expect("count scale identities");
        let request_count: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_conversations",
        )
        .fetch_one(&pool)
        .await
        .expect("sum scale requests");
        let queue_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue",
        )
        .fetch_one(&pool)
        .await
        .expect("count scale queue");
        pool.close().await;
        read_pool.close().await;
        (
            elapsed,
            latencies,
            identity_count,
            request_count,
            queue_count,
        )
    }

    fn percentile_micros(latencies: &[u128], percentile: usize) -> u128 {
        let mut sorted = latencies.to_vec();
        sorted.sort_unstable();
        let rank = ((sorted.len() * percentile).saturating_add(99) / 100).max(1) - 1;
        sorted[rank.min(sorted.len() - 1)]
    }

    fn median_duration(durations: &mut [Duration]) -> Duration {
        durations.sort_unstable();
        durations[durations.len() / 2]
    }

    let temp_dir = make_temp_test_dir("prompt-cache-materialization-scale");
    const REPRESENTATIVE_TRIALS: usize = 3;
    let mut fixed_durations = Vec::with_capacity(REPRESENTATIVE_TRIALS);
    let mut adaptive_durations = Vec::with_capacity(REPRESENTATIVE_TRIALS);
    let mut fixed_latencies = Vec::with_capacity(REPRESENTATIVE_TRIALS * 500);
    let mut adaptive_latencies = Vec::with_capacity(REPRESENTATIVE_TRIALS * 500);
    for trial in 0..REPRESENTATIVE_TRIALS {
        let fixed = run_scale_case(&temp_dir.join(format!("fixed-{trial}.db")), false).await;
        assert_eq!((fixed.2, fixed.3, fixed.4), (4_000, 40_000, 0));
        fixed_durations.push(fixed.0);
        fixed_latencies.extend(fixed.1);

        let adaptive = run_scale_case(&temp_dir.join(format!("adaptive-{trial}.db")), true).await;
        assert_eq!((adaptive.2, adaptive.3, adaptive.4), (4_000, 40_000, 0));
        adaptive_durations.push(adaptive.0);
        adaptive_latencies.extend(adaptive.1);
    }
    let fixed_ms = median_duration(&mut fixed_durations).as_millis();
    let adaptive_ms = median_duration(&mut adaptive_durations).as_millis();
    let fixed_p95 = percentile_micros(&fixed_latencies, 95);
    let fixed_p99 = percentile_micros(&fixed_latencies, 99);
    let adaptive_p95 = percentile_micros(&adaptive_latencies, 95);
    let adaptive_p99 = percentile_micros(&adaptive_latencies, 99);
    eprintln!(
        "prompt-cache scale trials={} fixed_median_ms={} adaptive_median_ms={} fixed_p95_us={} adaptive_p95_us={} fixed_p99_us={} adaptive_p99_us={}",
        REPRESENTATIVE_TRIALS,
        fixed_ms,
        adaptive_ms,
        fixed_p95,
        adaptive_p95,
        fixed_p99,
        adaptive_p99,
    );
    assert!(
        adaptive_ms <= fixed_ms * 10 / 7,
        "adaptive throughput fell below 70%: fixed_median_ms={fixed_ms} adaptive_median_ms={adaptive_ms}"
    );
    assert!(
        adaptive_p95 <= fixed_p95.saturating_mul(110) / 100 + 1,
        "adaptive foreground p95 regressed: fixed={fixed_p95}us adaptive={adaptive_p95}us"
    );
    assert!(
        adaptive_p99 <= fixed_p99.saturating_mul(110) / 100 + 1,
        "adaptive foreground p99 regressed: fixed={fixed_p99}us adaptive={adaptive_p99}us"
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn prompt_cache_conversation_read_snapshot_keeps_completeness_with_aggregate_reads() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "prompt-cache-conversation-read-snapshot",
        Duration::from_secs(5),
    )
    .await;
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let mut transaction = state
        .pool
        .begin()
        .await
        .expect("begin prompt-cache aggregate read snapshot");
    assert!(
        prompt_cache_conversation_materialization_is_complete_on_connection(transaction.as_mut())
            .await
            .expect("check prompt-cache completeness in read transaction")
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('snapshot-new-invocation', '2026-09-01 00:00:00', ?1, 'success', 12, 0.25, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "snapshot-new-key"}).to_string())
    .execute(&state.pool)
    .await
    .expect("commit concurrent prompt-cache invocation");

    assert!(
        prompt_cache_conversation_materialization_is_complete_on_connection(transaction.as_mut())
            .await
            .expect("recheck completeness on the aggregate read snapshot")
    );
    let snapshot_queue_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue")
            .fetch_one(transaction.as_mut())
            .await
            .expect("read prompt-cache queue on the aggregate snapshot");
    assert_eq!(snapshot_queue_count, 0);
    transaction
        .rollback()
        .await
        .expect("rollback prompt-cache read snapshot");

    assert!(
        !prompt_cache_conversation_materialization_is_complete(&state.pool)
            .await
            .expect("check post-write prompt-cache completeness")
    );
    state.pool.close().await;
    let _ = fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn ensure_schema_recovers_prompt_cache_stats_when_refresh_marker_is_missing() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('LEGACYRECOVER1', '2026-09-01 00:00:00', ?1, 'success', 12, 0.25, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "conversation-stats-recovery-key"}).to_string())
    .execute(&pool)
    .await
    .expect("insert invocation for statistics recovery");

    ensure_schema(&pool)
        .await
        .expect("install initial prompt-cache conversation structure");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;
    sqlx::query(
        r#"
        UPDATE prompt_cache_conversations
        SET request_count = 99,
            success_count = 0,
            total_tokens = 999,
            cost = 99.0
        WHERE prompt_cache_key = 'conversation-stats-recovery-key'
        "#,
    )
    .execute(&pool)
    .await
    .expect("make prompt-cache statistics stale");
    sqlx::query(
        "UPDATE prompt_cache_conversation_stats_freshness SET state = 'stale' WHERE marker_name = 'prompt_cache_conversations_stats_v2'",
    )
    .execute(&pool)
    .await
    .expect("remove statistics freshness marker");

    ensure_schema(&pool)
        .await
        .expect("rerun prompt-cache conversation structure migration");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;

    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, f64)>(
        r#"
        SELECT request_count, success_count, failure_count, total_tokens, cost
        FROM prompt_cache_conversations
        WHERE prompt_cache_key = 'conversation-stats-recovery-key'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load recovered prompt-cache statistics");
    assert_eq!(stats.0, 1);
    assert_eq!(stats.1, 1);
    assert_eq!(stats.2, 0);
    assert_eq!(stats.3, 12);
    assert!((stats.4 - 0.25).abs() < 1e-9);

    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_freshness WHERE marker_name = 'prompt_cache_conversations_stats_v2' AND state = 'fresh'",
    )
    .fetch_one(&pool)
    .await
    .expect("count recovered statistics marker");
    assert_eq!(marker_count, 1);
}

#[tokio::test]
async fn ensure_schema_recovers_prompt_cache_identity_after_invocation_payload_update() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('LEGACYKEY0001', '2026-09-01 00:00:00', ?1, 'success', 8, 0.5, '{}', '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("insert invocation without prompt-cache key");

    ensure_schema(&pool)
        .await
        .expect("install initial prompt-cache conversation structure");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET payload = '{"promptCacheKey":"backfilled-prompt-cache-key"}'
        WHERE invoke_id = 'LEGACYKEY0001'
        "#,
    )
    .execute(&pool)
    .await
    .expect("update invocation prompt-cache metadata");

    let queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key = 'backfilled-prompt-cache-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("count queued prompt-cache refresh");
    assert_eq!(queued, 1);

    ensure_schema(&pool)
        .await
        .expect("rerun prompt-cache conversation structure migration");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;

    let stats = sqlx::query_as::<_, (i64, i64, f64)>(
        r#"
        SELECT request_count, total_tokens, cost
        FROM prompt_cache_conversations
        WHERE prompt_cache_key = 'backfilled-prompt-cache-key'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load recovered prompt-cache identity");
    assert_eq!(stats.0, 1);
    assert_eq!(stats.1, 8);
    assert!((stats.2 - 0.5).abs() < 1e-9);

    let remaining_queue: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue")
            .fetch_one(&pool)
            .await
            .expect("count remaining prompt-cache refresh queue");
    assert_eq!(remaining_queue, 0);
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_keeps_sequences_in_memory() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let first = allocate_proxy_invoke_id(&state, Some("allocator-key"))
        .await
        .expect("allocate first conversation invoke id");
    let second = allocate_proxy_invoke_id(&state, Some("allocator-key"))
        .await
        .expect("allocate second conversation invoke id");
    assert_eq!(first.len(), PROXY_INVOKE_ID_LENGTH);
    assert_eq!(&first[..6], &second[..6]);
    assert_eq!(&first[6..], "AAAA");
    assert_eq!(&second[6..], "AAAB");

    for invoke_id in [&first, &second] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response
            ) VALUES (?1, ?2, ?3, 'success', ?4, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind("2026-09-01 00:00:00")
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": "allocator-key"}).to_string())
        .execute(&state.pool)
        .await
        .expect("persist allocated conversation invocation");
    }
    clear_prompt_cache_conversation_identity_cache(&state.prompt_cache_conversation_cache).await;
    let recovered = allocate_proxy_invoke_id(&state, Some("allocator-key"))
        .await
        .expect("recover conversation invoke sequence from database");
    assert_eq!(&recovered[..6], &first[..6]);
    assert_eq!(
        &recovered[6..],
        encode_prompt_cache_conversation_sequence(64).unwrap()
    );

    let unbound_first = allocate_proxy_invoke_id(&state, None)
        .await
        .expect("allocate first unbound invoke id");
    let unbound_second = allocate_proxy_invoke_id(&state, None)
        .await
        .expect("allocate second unbound invoke id");
    assert_eq!(&unbound_first[6..], "AAAA");
    if unbound_first[..6] == unbound_second[..6] {
        assert_eq!(&unbound_second[6..], "AAAB");
    } else {
        assert_eq!(&unbound_second[6..], "AAAA");
    }

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key = 'allocator-key'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count allocated conversation identity");
    assert_eq!(row_count, 1);

    sqlx::query(
        "UPDATE prompt_cache_conversations SET last_invocation_at = '2026-09-01 00:00:00', updated_at = '2026-09-01 00:00:00' WHERE prompt_cache_key = 'allocator-key'",
    )
    .execute(&state.pool)
    .await
    .expect("mark conversation as having historical activity");
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id IN (?1, ?2, ?3)")
        .bind(&first)
        .bind(&second)
        .bind(&recovered)
        .execute(&state.pool)
        .await
        .expect("remove live allocator invocations");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversations (
            conversation_id, prompt_cache_key, created_at, updated_at
        ) VALUES ('ABCD23', 'orphan-without-refreshed-stats', '2020-01-01T00:00:00.000Z', '2020-01-01T00:00:00.000Z')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("insert orphan prompt-cache conversation without refreshed stats");
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            true,
            &state.prompt_cache_conversation_cache
        )
        .await
        .expect("count releasable prompt-cache conversation"),
        2
    );
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache
        )
        .await
        .expect("release orphan prompt-cache conversation"),
        2
    );
    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key = 'allocator-key'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count released conversation identity");
    assert_eq!(row_count, 0);
    let orphan_queue_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key IN ('allocator-key', 'orphan-without-refreshed-stats')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count released prompt-cache refresh obligations");
    assert_eq!(orphan_queue_count, 0);
}

#[tokio::test]
async fn prompt_cache_conversation_retention_preserves_active_and_live_identities() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let active_key = "active-retention-key";
    let live_key = "live-retention-key";
    super::invocation_ranges::allocate_fixture_with_bounded_timeout_retry(|| {
        allocate_proxy_invoke_id(&state, Some(active_key))
    })
    .await
    .expect("allocate active prompt-cache identity");
    sqlx::query(
        "UPDATE prompt_cache_conversations SET created_at = '2020-01-01T00:00:00.000Z', updated_at = '2020-01-01T00:00:00.000Z' WHERE prompt_cache_key = ?1",
    )
    .bind(active_key)
    .execute(&state.pool)
    .await
    .expect("age active prompt-cache identity");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversations (
            conversation_id, prompt_cache_key, created_at, updated_at
        ) VALUES ('ABCD23', ?1, '2020-01-01T00:00:00.000Z', '2020-01-01T00:00:00.000Z')
        "#,
    )
    .bind(live_key)
    .execute(&state.pool)
    .await
    .expect("insert live prompt-cache identity");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        ) VALUES ('ABCD23AAAA', '2026-09-01 00:00:00', ?1, 'running', ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": live_key}).to_string())
    .execute(&state.pool)
    .await
    .expect("insert live prompt-cache invocation");

    retain_active_prompt_cache_conversation(&state.prompt_cache_conversation_cache, active_key)
        .await;
    retain_active_prompt_cache_conversation(&state.prompt_cache_conversation_cache, active_key)
        .await;
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache,
        )
        .await
        .expect("retain active and live prompt-cache identities"),
        0
    );
    let preserved_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key IN (?1, ?2)",
    )
    .bind(active_key)
    .bind(live_key)
    .fetch_one(&state.pool)
    .await
    .expect("count preserved prompt-cache identities");
    assert_eq!(preserved_count, 2);

    release_active_prompt_cache_conversation(&state.prompt_cache_conversation_cache, active_key)
        .await;
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache,
        )
        .await
        .expect("release inactive prompt-cache identity"),
        0
    );
    release_active_prompt_cache_conversation(&state.prompt_cache_conversation_cache, active_key)
        .await;
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache,
        )
        .await
        .expect("release fully inactive prompt-cache identity"),
        1
    );
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(active_key)
    .fetch_one(&state.pool)
    .await
    .expect("count released active prompt-cache identity");
    assert_eq!(active_count, 0);

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = 'ABCD23AAAA'")
        .execute(&state.pool)
        .await
        .expect("remove live invocation");
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache,
        )
        .await
        .expect("release orphaned live identity"),
        1
    );
}

#[tokio::test]
async fn prompt_cache_conversation_stats_refresh_is_deferred_after_terminal_batch() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "delayed-stats-refresh-key";
    let first_invoke_id = allocate_proxy_invoke_id(&state, Some(prompt_cache_key))
        .await
        .expect("allocate first stats invoke id");
    let second_invoke_id = allocate_proxy_invoke_id(&state, Some(prompt_cache_key))
        .await
        .expect("allocate second stats invoke id");
    let first_occurred_at = "2026-09-01 00:00:01";
    let second_occurred_at = "2026-09-01 00:00:02";

    let mut success = test_proxy_capture_record(&first_invoke_id, first_occurred_at);
    success.usage.input_tokens = Some(10);
    success.usage.output_tokens = Some(4);
    success.usage.total_tokens = Some(14);
    success.cost = Some(0.25);
    let mut success_payload: Value =
        serde_json::from_str(success.payload.as_deref().expect("test payload"))
            .expect("decode success payload");
    success_payload["promptCacheKey"] = json!(prompt_cache_key);
    success.payload = Some(success_payload.to_string());

    let mut failure = test_proxy_capture_record(&second_invoke_id, second_occurred_at);
    failure.status = "failed".to_string();
    failure.error_message = Some("[test_failure] terminal failure".to_string());
    failure.failure_kind = Some("test_failure".to_string());
    failure.usage.input_tokens = Some(20);
    failure.usage.output_tokens = Some(6);
    failure.usage.total_tokens = Some(26);
    failure.cost = Some(0.75);
    let mut failure_payload: Value =
        serde_json::from_str(failure.payload.as_deref().expect("test payload"))
            .expect("decode failure payload");
    failure_payload["promptCacheKey"] = json!(prompt_cache_key);
    failure_payload["statusCode"] = json!(502);
    failure.payload = Some(failure_payload.to_string());

    retain_active_prompt_cache_conversation(
        &state.prompt_cache_conversation_cache,
        prompt_cache_key,
    )
    .await;
    retain_active_prompt_cache_conversation(
        &state.prompt_cache_conversation_cache,
        prompt_cache_key,
    )
    .await;
    persist_and_broadcast_proxy_capture_terminal_record(&state, success)
        .await
        .expect("queue successful terminal record");
    persist_and_broadcast_proxy_capture_terminal_record(&state, failure)
        .await
        .expect("queue failed terminal record");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let queued_refreshes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("count deferred prompt-cache aggregate refresh");
    assert_eq!(
        queued_refreshes, 1,
        "terminal persistence must defer the historical aggregate refresh"
    );
    let fresh_marker: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM prompt_cache_conversation_stats_freshness WHERE marker_name = 'prompt_cache_conversations_stats_v2' AND state = 'fresh')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check deferred prompt-cache aggregate marker");
    assert_eq!(
        fresh_marker, 0,
        "deferred aggregates must remain unavailable"
    );

    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, f64, String, String)>(
        r#"
        SELECT request_count, success_count, failure_count, input_tokens, output_tokens,
               cost, first_invocation_at, last_invocation_at
        FROM prompt_cache_conversations
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load delayed prompt-cache aggregate");
    assert_eq!(stats.0, 2);
    assert_eq!(stats.1, 1);
    assert_eq!(stats.2, 1);
    assert_eq!(stats.3, 30);
    assert_eq!(stats.4, 10);
    assert!((stats.5 - 1.0).abs() < 1e-9);
    assert_eq!(stats.6, first_occurred_at);
    assert_eq!(stats.7, second_occurred_at);
    assert!(
        !state
            .prompt_cache_conversation_cache
            .lock()
            .await
            .identity_cache
            .active_prompt_cache_keys
            .contains_key(prompt_cache_key)
    );

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind(&second_invoke_id)
        .execute(&state.pool)
        .await
        .expect("delete archived prompt-cache invocation");
    refresh_all_prompt_cache_conversation_stats(&state.pool)
        .await
        .expect("refresh prompt-cache aggregate after archive deletion");
    let refreshed_stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, f64, String, String)>(
        r#"
        SELECT request_count, success_count, failure_count, input_tokens, output_tokens,
               cost, first_invocation_at, last_invocation_at
        FROM prompt_cache_conversations
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load refreshed prompt-cache aggregate");
    assert_eq!(refreshed_stats.0, 1);
    assert_eq!(refreshed_stats.1, 1);
    assert_eq!(refreshed_stats.2, 0);
    assert_eq!(refreshed_stats.3, 10);
    assert_eq!(refreshed_stats.4, 4);
    assert!((refreshed_stats.5 - 0.25).abs() < 1e-9);
    assert_eq!(refreshed_stats.6, first_occurred_at);
    assert_eq!(refreshed_stats.7, first_occurred_at);

    let last_invoke_sequence: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load prompt-cache conversation sequence high-water mark");
    assert_eq!(last_invoke_sequence, 63);

    clear_prompt_cache_conversation_identity_cache(&state.prompt_cache_conversation_cache).await;
    let next_invoke_id = allocate_proxy_invoke_id(&state, Some(prompt_cache_key))
        .await
        .expect("allocate after prompt-cache archive deletion");
    assert_ne!(next_invoke_id, second_invoke_id);
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_recovers_same_key_database_race() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let second_state = test_state_from_existing_pool(state.pool.clone(), test_config(), true).await;

    let (first, second) = tokio::join!(
        allocate_proxy_invoke_id(&state, Some("concurrent-allocator-key")),
        allocate_proxy_invoke_id(&second_state, Some("concurrent-allocator-key")),
    );
    let first = first.expect("first concurrent allocation");
    let second = second.expect("second concurrent allocation");
    assert_eq!(
        &first[..PROMPT_CACHE_CONVERSATION_ID_LENGTH],
        &second[..PROMPT_CACHE_CONVERSATION_ID_LENGTH]
    );
    let mut suffixes = [
        &first[PROMPT_CACHE_CONVERSATION_ID_LENGTH..],
        &second[PROMPT_CACHE_CONVERSATION_ID_LENGTH..],
    ];
    suffixes.sort_unstable();
    assert_eq!(
        suffixes,
        [
            "AAAA",
            encode_prompt_cache_conversation_sequence(64)
                .unwrap()
                .as_str()
        ]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key = 'concurrent-allocator-key'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count raced prompt-cache conversation rows"),
        1
    );
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_reinitializes_after_instance_cleanup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "cross-state-cleanup-key";
    let first = allocate_proxy_invoke_id(&state, Some(prompt_cache_key))
        .await
        .expect("allocate initial prompt-cache invoke id");

    sqlx::query(
        "UPDATE prompt_cache_conversations SET last_invocation_at = '2026-09-01 00:00:00', updated_at = '2026-09-01 00:00:00' WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .execute(&state.pool)
    .await
    .expect("mark prompt-cache identity as historically active");
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(
            &state.pool,
            false,
            &state.prompt_cache_conversation_cache,
        )
        .await
        .expect("release identity through its instance allocation manager"),
        1
    );

    let recovered = allocate_proxy_invoke_id(&state, Some(prompt_cache_key))
        .await
        .expect("recover stale prompt-cache identity cache");
    assert_eq!(recovered.len(), PROXY_INVOKE_ID_LENGTH);
    assert_eq!(&recovered[PROMPT_CACHE_CONVERSATION_ID_LENGTH..], "AAAA");
    assert_ne!(
        &recovered[..PROMPT_CACHE_CONVERSATION_ID_LENGTH],
        &first[..PROMPT_CACHE_CONVERSATION_ID_LENGTH]
    );
    let persisted_conversation_id: String = sqlx::query_scalar(
        "SELECT conversation_id FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load recreated prompt-cache identity");
    assert_eq!(
        persisted_conversation_id,
        recovered[..PROMPT_CACHE_CONVERSATION_ID_LENGTH].to_string()
    );
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_rejects_sequence_exhaustion() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF','exhausted-allocator-key',?1)")
        .bind(i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY) - 1)
        .execute(&state.pool).await.unwrap();

    let error = allocate_proxy_invoke_id(&state, Some("exhausted-allocator-key"))
        .await
        .expect_err("exhausted prompt-cache sequence should fail explicitly");
    assert!(error.to_string().contains("sequence overflow"));
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_retries_colliding_candidates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversations (conversation_id, prompt_cache_key)
        VALUES ('ABCDEF', 'existing-collision-key')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("insert colliding prompt-cache conversation");

    let identity = create_prompt_cache_conversation_row_with_test_candidates(
        &state.pool,
        "collision-retry-key",
        &["ABCDEF", "ABCDEF", "ZZZZZZ"],
    )
    .await
    .expect("allocator should retry a colliding candidate");
    assert_eq!(identity.conversation_id, "ZZZZZZ");
    assert_eq!(identity.next_sequence, 0);
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_rejects_invocation_only_prefix_collision() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        ) VALUES ('ABCDEFZAAA', '2026-09-01 00:00:00', ?1, 'running', '{}', '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert invocation-only prefix collision");

    let identity = create_prompt_cache_conversation_row_with_test_candidates(
        &state.pool,
        "invocation-only-collision-key",
        &["ABCDEF", "ZZZZZZ"],
    )
    .await
    .expect("allocator should reject invocation-only prefix collision");
    assert_eq!(identity.conversation_id, "ZZZZZZ");
}

#[tokio::test]
async fn prompt_cache_conversation_allocator_bounds_colliding_candidate_retries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversations (conversation_id, prompt_cache_key)
        VALUES ('ABCDEF', 'existing-exhaustion-key')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("insert exhausted prompt-cache conversation");

    let error = create_prompt_cache_conversation_row_with_test_candidates(
        &state.pool,
        "collision-exhaustion-key",
        &["ABCDEF", "ABCDEF", "ABCDEF", "ABCDEF", "ABCDEF"],
    )
    .await
    .expect_err("allocator should stop after bounded collision retries");
    assert!(error.to_string().contains("after 5 attempts"));
}

#[tokio::test]
async fn dropped_pool_invocation_recovery_releases_prompt_cache_lease() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "dropped-recovery-lease-key";
    let invoke_id = allocate_proxy_invoke_id_with_active_lease(&state, Some(prompt_cache_key))
        .await
        .expect("allocate active prompt-cache invoke id");

    recover_guard_dropped_pool_invocation_orphan_with_prompt_cache_key(
        &state,
        InvocationRecoverySelector::new(invoke_id, "2026-09-28 00:00:00".to_string()),
        "test_recovery",
        Some(prompt_cache_key.to_string()),
    )
    .await
    .expect("recover dropped invocation without a persisted row");

    let cache = state.prompt_cache_conversation_cache.lock().await;
    assert_eq!(
        cache
            .identity_cache
            .active_prompt_cache_keys
            .get(prompt_cache_key),
        None
    );
}
