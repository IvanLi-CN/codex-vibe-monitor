use super::*;

#[tokio::test]
async fn runtime_terminal_append_does_not_confirm_materializer_admission() {
    let fixture = std::env::temp_dir().join(format!("runtime-confirmation-{}", nanoid::nanoid!()));
    fs::create_dir_all(&fixture).expect("create owned confirmation fixture");
    let mut config = test_config();
    config.database_path = fixture.join("business.sqlite");
    let state = test_state_from_config(config, true).await;
    let writer = SqliteBatchWriter::spawn(
        state.pool.clone(),
        state.shutdown.clone(),
        state.prompt_cache_conversation_cache.clone(),
        state.pricing_catalog.clone(),
        &state.config.database_path,
    )
    .expect("start file journal and materializer");
    writer.set_auto_flush_terminal_for_test(false);
    let pause = writer.pause_runtime_terminal_confirmation_for_test().await;
    let producer = tokio::spawn({
        let writer = writer.clone();
        async move {
            writer
                .enqueue_runtime_terminal(
                    crate::sqlite_batch_writer::BatchedTerminalInvocationWrite {
                        enqueued_at: None,
                        record: test_proxy_capture_record(
                            "confirmation-boundary",
                            "2026-10-13 00:00:00",
                        ),
                        attempts: Vec::new(),
                        diagnostic: None,
                        capture_started: None,
                        raw_capture: false,
                        dashboard_terminal_sequence: None,
                        terminal_projection_event_ids: Vec::new(),
                        startup_backfill_tasks: Vec::new(),
                    },
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while writer
            .terminal_journal_stats()
            .pending_materialization_records
            != 1
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("producer must append while confirmation is paused");
    assert_eq!(writer.runtime_terminal_confirmation_count_for_test(), 0);
    writer
        .flush_now(&state.pool)
        .await
        .expect("flush admitted work");
    let before_confirmation: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&state.pool)
        .await
        .expect("query materialized facts before producer confirmation");
    assert_eq!(before_confirmation, 0);
    assert!(!producer.is_finished());
    drop(pause);
    let outcome = tokio::time::timeout(Duration::from_secs(2), producer)
        .await
        .expect("producer confirmation deadline")
        .expect("producer task completes");
    assert!(outcome.enqueued);
    assert_eq!(writer.runtime_terminal_confirmation_count_for_test(), 1);
    writer
        .flush_now(&state.pool)
        .await
        .expect("flush confirmed terminal");
    let committed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&state.pool)
        .await
        .expect("query materialized confirmed fact");
    assert_eq!(committed, 1);
    assert_eq!(
        writer
            .terminal_journal_stats()
            .pending_materialization_records,
        0
    );
    state.shutdown.cancel();
    writer.shutdown_and_drain().await;
    fs::remove_dir_all(fixture).expect("remove owned confirmation fixture");
}

#[tokio::test]
async fn proxy_runtime_attempt_lifecycle_avoids_sqlite_admission_under_write_pressure() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_secs(1)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_secs(5);
    let fixture = std::env::temp_dir().join(format!("runtime-pressure-{}", nanoid::nanoid!()));
    fs::create_dir_all(&fixture).expect("create owned pressure fixture");
    config.database_path = fixture.join("business.sqlite");
    let mut state = test_state_from_config(config, true).await;
    let mutable = Arc::get_mut(&mut state).expect("fixture has one state owner");
    mutable.sqlite_batch_writer = SqliteBatchWriter::spawn(
        mutable.pool.clone(),
        mutable.shutdown.clone(),
        mutable.prompt_cache_conversation_cache.clone(),
        mutable.pricing_catalog.clone(),
        &mutable.config.database_path,
    )
    .expect("use the production file journal and background materializer");
    mutable
        .sqlite_batch_writer
        .set_terminal_runtime_store(mutable.proxy_runtime_invocations.clone());
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    seed_pool_routing_api_key(&state, "pool-pressure-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Pressure Route",
        "route-pressure",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let request_count = 32;
    for index in 0..request_count {
        let prompt_cache_key = format!("pressure-key-{index}");
        allocate_proxy_invoke_id_with_active_lease(&state, Some(&prompt_cache_key))
            .await
            .expect("prewarm invocation range for pressure request");
    }
    let progress_before = state
        .sqlite_batch_writer
        .accounting_snapshot()
        .attempt_progress_enqueued;
    let coordinator_before =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    let mut locked_connection = state
        .pool
        .acquire()
        .await
        .expect("acquire pressure connection");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locked_connection)
        .await
        .expect("hold SQLite write pressure lock");
    let scenario_started = Instant::now();
    let mut requests = Vec::with_capacity(request_count);
    for index in 0..request_count {
        let state = state.clone();
        requests.push(tokio::spawn(async move {
            let started = Instant::now();
            let response = proxy_openai_v1_inner(
                state.clone(),
                20_000 + index as u64,
                "/v1/responses".parse().expect("valid uri"),
                Method::POST,
                HeaderMap::from_iter([
                    (
                        http_header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer pool-pressure-key"),
                    ),
                    (
                        http_header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    ),
                    (
                        http_header::HeaderName::from_static("x-prompt-cache-key"),
                        HeaderValue::from_str(&format!("pressure-key-{index}"))
                            .expect("valid prompt-cache key"),
                    ),
                ]),
                Body::from(Bytes::from_static(
                    br#"{"model":"gpt-5","input":"pressure"}"#,
                )),
                state
                    .config
                    .openai_upstream_base_url
                    .join("v1/responses")
                    .expect("target URL"),
                None,
                true,
                resolve_proxy_request_timeouts(state.as_ref(), true)
                    .await
                    .expect("resolve proxy request timeouts"),
                None,
                None,
                started,
            )
            .await
            .expect("proxy request should complete");
            let status = response.status();
            let _ = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read proxy response body");
            (status, started.elapsed())
        }));
    }

    let attempts_deadline = Instant::now() + Duration::from_secs(2);
    while state.proxy_runtime_attempts.health_snapshot().records < request_count
        && Instant::now() < attempts_deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.proxy_runtime_attempts.health_snapshot().records,
        request_count,
        "all active attempts should be visible in bounded memory before SQLite pressure"
    );
    let coordinator_after_admission =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    assert_eq!(
        coordinator_after_admission
            .write_admission
            .interactive_proxy
            .admission_count
            .saturating_sub(
                coordinator_before
                    .write_admission
                    .interactive_proxy
                    .admission_count,
            ),
        0,
        "prewarmed request admission should add no SQLite writes for attempt lifecycle"
    );

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        state
            .sqlite_batch_writer
            .accounting_snapshot()
            .attempt_progress_enqueued,
        progress_before,
        "runtime attempt progress must not enter the SQLite progress queue"
    );
    let coordinator_during_pressure =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    assert_eq!(
        coordinator_during_pressure
            .write_admission
            .interactive_proxy
            .admission_count
            .saturating_sub(
                coordinator_after_admission
                    .write_admission
                    .interactive_proxy
                    .admission_count,
            ),
        0,
        "active attempt progress must not acquire InteractiveProxy write admission"
    );
    assert_eq!(
        state.proxy_runtime_attempts.health_snapshot().records,
        request_count,
        "SQLite pressure must not remove active runtime attempt state"
    );
    let results = futures_util::future::join_all(requests)
        .await
        .into_iter()
        .map(|result| result.expect("join proxy pressure request"))
        .collect::<Vec<_>>();
    let statuses = results
        .iter()
        .map(|(status, _)| *status)
        .collect::<Vec<_>>();
    assert!(
        statuses.iter().all(|status| *status == StatusCode::OK),
        "representative proxy pressure requests should complete successfully: {statuses:?}"
    );
    let mut latencies = results
        .iter()
        .map(|(_, elapsed)| *elapsed)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95 = latencies[(latencies.len() * 95).div_ceil(100).saturating_sub(1)];
    assert!(
        p95 < Duration::from_secs(2),
        "proxy request p95 must stay within the pressure budget: {p95:?}"
    );
    assert_eq!(
        state
            .sqlite_batch_writer
            .accounting_snapshot()
            .attempt_progress_enqueued,
        progress_before,
        "terminal materialization must not reintroduce AttemptProgress SQL writes"
    );
    // EOF closes the response channel before the capture task finishes journal admission.
    // Append statistics include unconfirmed producers. Wait for sync and queue admission
    // before taking the flush barrier, while keeping the SQLite lock held.
    let journal_deadline = Instant::now() + Duration::from_secs(2);
    while state
        .sqlite_batch_writer
        .runtime_terminal_confirmation_count_for_test()
        < request_count
        && Instant::now() < journal_deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state
            .sqlite_batch_writer
            .runtime_terminal_confirmation_count_for_test(),
        request_count,
        "every producer must confirm durability and materializer admission within two seconds"
    );
    let held_lock_journal = state.sqlite_batch_writer.terminal_journal_stats();
    assert!(
        !held_lock_journal.unavailable,
        "file journal must be active"
    );
    assert_eq!(
        held_lock_journal.pending_materialization_records, request_count,
        "all confirmed terminal events must remain recoverable while the writer is locked: {held_lock_journal:?}"
    );
    assert!(held_lock_journal.pending_bytes > 0);
    assert!(
        held_lock_journal.checkpoint_lag_ms < 5_000,
        "journal age budget"
    );
    let before_commit: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations WHERE source='proxy'")
            .fetch_one(&mut *locked_connection)
            .await
            .expect("read held-lock snapshot");
    assert_eq!(
        before_commit, 0,
        "no terminal fact commits through the held write lock"
    );
    let coordinator_after_terminal =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    assert_eq!(
        coordinator_after_terminal
            .write_admission
            .interactive_proxy
            .admission_count,
        coordinator_before
            .write_admission
            .interactive_proxy
            .admission_count,
        "complete runtime requests must never enter InteractiveProxy write admission"
    );
    println!(
        "runtime_pressure requests={request_count} confirmed_terminals={request_count} sqlite_lock_held_through_terminal=true request_p95_ms={} interactive_admissions=0 attempt_progress_sql=0 terminal_pending_records={} terminal_pending_materialization={} terminal_pending_bytes={}",
        p95.as_millis(),
        held_lock_journal.pending_records,
        held_lock_journal.pending_materialization_records,
        held_lock_journal.pending_bytes
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *locked_connection)
        .await
        .expect("release SQLite write pressure lock");
    drop(locked_connection);
    let drain_started = Instant::now();
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("materialize confirmed terminal batch");
    assert!(
        drain_started.elapsed() < Duration::from_secs(5),
        "terminal materialization drain budget"
    );
    let committed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations WHERE source='proxy'")
            .fetch_one(&state.pool)
            .await
            .expect("read materialized business facts");
    assert_eq!(committed, request_count as i64);
    assert_eq!(
        state
            .sqlite_batch_writer
            .terminal_journal_stats()
            .pending_materialization_records,
        0
    );
    assert!(
        scenario_started.elapsed() < Duration::from_secs(5),
        "request-start to durable terminal commit upper bound includes journal and materialization delay"
    );
    println!(
        "runtime_pressure materialization_drain_ms={} terminal_commit_latency_upper_bound_ms={} committed={committed}",
        drain_started.elapsed().as_millis(),
        scenario_started.elapsed().as_millis()
    );
    if let Some(runtime) = state.upstream_accounts.route_facts.lock().await.clone() {
        runtime.shutdown_and_sync().await;
    }
    state.shutdown.cancel();
    state.sqlite_batch_writer.shutdown_and_drain().await;
    upstream_handle.abort();
    fs::remove_dir_all(fixture).expect("remove owned pressure fixture");
}
