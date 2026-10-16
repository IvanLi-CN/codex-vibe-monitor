use super::*;

#[tokio::test]
async fn runtime_terminal_append_and_confirmation_failures_preserve_recovery_fact() {
    for fail_after_append in [false, true] {
        let fixture = std::env::temp_dir().join(format!("runtime-failure-{}", nanoid::nanoid!()));
        fs::create_dir_all(&fixture).expect("create owned recovery fixture");
        let mut config = test_config();
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
        .expect("production file journal");
        let writer = state.sqlite_batch_writer.clone();
        writer.set_auto_flush_terminal_for_test(false);
        let pause = writer.pause_runtime_terminal_confirmation_for_test().await;
        if !fail_after_append {
            writer.fail_runtime_journal_for_test();
        }
        let id = if fail_after_append {
            "unconfirmed-terminal"
        } else {
            "rejected-terminal"
        };
        let producer = tokio::spawn({
            let writer = writer.clone();
            async move {
                writer
                    .enqueue_runtime_terminal(BatchedTerminalInvocationWrite {
                        enqueued_at: None,
                        diagnostic: None,
                        record: test_proxy_capture_record(id, "2026-10-13 12:00:00"),
                        attempts: Vec::new(),
                        capture_started: None,
                        raw_capture: false,
                        dashboard_terminal_sequence: None,
                        terminal_projection_event_ids: Vec::new(),
                        startup_backfill_tasks: Vec::new(),
                    })
                    .await
            }
        });
        if fail_after_append {
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
            .expect("append precedes the confirmation failure");
            writer.fail_runtime_journal_for_test();
        }
        drop(pause);
        let outcome = tokio::time::timeout(Duration::from_secs(2), producer)
            .await
            .expect("failure is bounded")
            .expect("producer completed");
        assert!(!outcome.enqueued, "unconfirmed event cannot report success");
        assert_eq!(writer.runtime_terminal_confirmation_count_for_test(), 0);
        let recovery =
            crate::terminal_journal::terminal_journal_directory(&state.config.database_path)
                .join("shutdown-terminal-quarantine.jsonl");
        let contents = fs::read_to_string(recovery).expect("durable supplemental recovery fact");
        let line: Value = serde_json::from_str(contents.lines().next().expect("recovery frame"))
            .expect("complete recovery frame");
        assert_eq!(line["invoke_id"], id);
        assert!(
            line["record"].is_object(),
            "full safe terminal fact is recoverable"
        );
        state.shutdown.cancel();
        writer.shutdown_and_drain().await;
        drop(writer);
        drop(state);
        fs::remove_dir_all(fixture).expect("remove owned recovery fixture");
    }
}

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

#[tokio::test(flavor = "multi_thread")]
async fn proxy_runtime_attempt_lifecycle_avoids_sqlite_admission_under_write_pressure() {
    proxy_runtime_write_pressure_scenario(true).await;
}

#[tokio::test]
async fn proxy_runtime_attempt_lifecycle_preserves_facts_under_current_thread_write_pressure() {
    proxy_runtime_write_pressure_scenario(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_runtime_terminal_commits_survive_retention_and_backfill_contention() {
    proxy_runtime_write_pressure_scenario_with_maintenance(true, true).await;
}

async fn proxy_runtime_write_pressure_scenario(performance_gate: bool) {
    proxy_runtime_write_pressure_scenario_with_maintenance(performance_gate, false).await;
}

async fn proxy_runtime_write_pressure_scenario_with_maintenance(
    performance_gate: bool,
    maintenance_contention: bool,
) {
    if performance_gate {
        assert!(
            matches!(
                tokio::runtime::Handle::current().runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ),
            "representative pressure acceptance must use the service's multi-thread executor"
        );
    }
    let mode = if maintenance_contention {
        "maintenance_contention_performance"
    } else if performance_gate {
        "service_performance"
    } else {
        "current_thread_functional"
    };
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
    let maintenance_pool = if maintenance_contention {
        let (business_pool, archive_config, archive_fixture) =
            retention_test_pool_and_config("runtime-maintenance-contention").await;
        // Initialize runtime caches normally, then install the owned file-backed business
        // pool before seeding routes or starting the real terminal writer.
        mutable.pool.close().await;
        mutable.pool = business_pool;
        mutable.config.database_path = archive_config.database_path;
        mutable.config.archive_dir = archive_config.archive_dir;
        mutable.config.proxy_raw_dir = archive_config.proxy_raw_dir;
        mutable.config.retention_batch_rows = 2;
        let old =
            shanghai_local_days_ago((mutable.config.invocation_max_days + 2) as i64, 12, 0, 0);
        sqlx::query(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<64)
             INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response)
             SELECT 'maintenance-source-'||i,?,'xy','success',7,0.07,'{}','{}' FROM n",
        ).bind(&old).execute(&mutable.pool).await.expect("seed real archive sources");
        let path = archive_fixture.join("maintenance.sqlite");
        fs::File::create(&path).expect("create independent maintenance file");
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                build_sqlite_connect_options(
                    &test_sqlite_url_for_path(&path),
                    Duration::from_millis(20),
                )
                .expect("maintenance connect options"),
            )
            .await
            .expect("connect independent maintenance pool");
        let store = Arc::new(crate::maintenance_store::MaintenanceStore::from_pool(
            pool.clone(),
        ));
        store
            .initialize_schema_for_test()
            .await
            .expect("maintenance schema");
        assert_eq!(
            sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key='startup_backfill.reasoning_effort'")
                .execute(&pool)
                .await
                .expect("enable the task exercised by the independent maintenance lock")
                .rows_affected(),
            1,
        );
        assert!(
            crate::maintenance_store::global().is_none(),
            "joint fixture owns this process"
        );
        crate::maintenance_store::set_global(store);
        Some((pool, archive_fixture))
    } else {
        None
    };
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
    if !performance_gate {
        // Empty fixture preparation is outside the outstanding-event age budget.
        tokio::time::sleep(Duration::from_secs(6)).await;
        let empty_journal = state.sqlite_batch_writer.terminal_journal_stats();
        assert_eq!(empty_journal.pending_records, 0);
        assert_eq!(empty_journal.oldest_pending_age_ms, 0);
        assert!(empty_journal.checkpoint_lag_ms >= 6_000);
    }
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
    let mut prewarm_timeout_retries = 0;
    for index in 0..request_count {
        let prompt_cache_key = format!("pressure-key-{index}");
        // Prepare committed ranges before the measured request/write-lock scenario.
        // Retain the production 100 ms deadline and the existing fixture policy of
        // at most five attempts for its known scheduling timeout.
        for attempt in 0..5 {
            match allocate_proxy_invoke_id_with_active_lease(&state, Some(&prompt_cache_key)).await
            {
                Ok(_) => break,
                Err(error)
                    if attempt < 4
                        && error.to_string()
                            == "invocation range allocation timed out after 100ms" =>
                {
                    prewarm_timeout_retries += 1;
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("prewarm invocation range for pressure request: {error}"),
            }
        }
    }
    println!("runtime_pressure_fixture prewarm_timeout_retries={prewarm_timeout_retries}");
    let progress_before = state
        .sqlite_batch_writer
        .accounting_snapshot()
        .attempt_progress_enqueued;
    let coordinator_before =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    // The joint scenario models a project foreground writer, which takes write
    // admission before a connection. Retention must wait without taking the
    // remaining read connection. The original scenario still covers an external
    // SQLite writer that bypasses the project's coordinator.
    let pressure_admission = if maintenance_contention {
        Some(
            crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
                .await,
        )
    } else {
        None
    };
    let mut locked_connection = state
        .pool
        .acquire()
        .await
        .expect("acquire pressure connection");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locked_connection)
        .await
        .expect("hold SQLite write pressure lock");
    let mut maintenance_lock = if let Some((pool, _)) = &maintenance_pool {
        let mut connection = pool
            .acquire()
            .await
            .expect("own maintenance contention lock");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await
            .expect("hold maintenance lock");
        Some(connection)
    } else {
        None
    };
    let maintenance_cancel = CancellationToken::new();
    let maintenance_confirmation = maintenance_pool.as_ref().map(|(pool, _)| {
        let pool = pool.clone();
        let cancel = maintenance_cancel.clone();
        tokio::spawn(async move {
            while !cancel.is_cancelled() {
                crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&pool).await;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {}
                }
            }
        })
    });
    let archive_metrics = crate::ObservabilityRuntime::new(maintenance_contention);
    if maintenance_contention {
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .bind_observability(archive_metrics.clone())
            .expect("observe maintenance holds including file publication");
    }
    let archive = maintenance_contention.then(|| {
        let state = state.clone();
        let metrics = archive_metrics.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            // The production run budget reserves five seconds before selecting
            // a file batch. A five-second synthetic run budget would therefore
            // select nothing. Bound the real elapsed work instead, retaining
            // the production selection and native-drain ownership semantics.
            let archived = tokio::time::timeout(
                Duration::from_secs(5),
                crate::observability::contention::with_performance(
                    metrics,
                    archive_old_invocations(
                        &state.pool,
                        &state.config,
                        state.config.database_path.parent(),
                        false,
                    ),
                ),
            )
            .await
            .expect("all real archive work must finish within five elapsed seconds")
            .expect("archive owner must complete actual work");
            (archived, started.elapsed())
        })
    });
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
            if maintenance_contention {
                crate::maintenance::notify_startup_backfill_tasks(
                    &state.pool,
                    &[StartupBackfillTask::ReasoningEffort],
                    None,
                    "terminal_capture",
                )
                .expect("bounded notification while the maintenance database is locked");
            }
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
    if performance_gate {
        assert!(
            p95 < Duration::from_secs(2),
            "proxy request p95 must stay within the pressure budget: {p95:?}"
        );
    }
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
        held_lock_journal.oldest_pending_age_ms < 5_000,
        "oldest pending journal event must stay within the age budget: {held_lock_journal:?}"
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
        "runtime_pressure mode={mode} requests={request_count} confirmed_terminals={request_count} sqlite_lock_held_through_terminal=true coordinated_write_pressure={maintenance_contention} request_p95_ms={} interactive_admissions=0 attempt_progress_sql=0 terminal_pending_records={} terminal_pending_materialization={} terminal_pending_bytes={}",
        p95.as_millis(),
        held_lock_journal.pending_records,
        held_lock_journal.pending_materialization_records,
        held_lock_journal.pending_bytes
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *locked_connection)
        .await
        .expect("release SQLite write pressure lock");
    locked_connection.return_to_pool().await;
    drop(pressure_admission);
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
    if performance_gate {
        assert!(
            scenario_started.elapsed() < Duration::from_secs(5),
            "request-start to durable terminal commit upper bound includes journal and materialization delay"
        );
    }
    println!(
        "runtime_pressure mode={mode} materialization_drain_ms={} terminal_commit_latency_upper_bound_ms={} committed={committed}",
        drain_started.elapsed().as_millis(),
        scenario_started.elapsed().as_millis()
    );
    if let Some(archive) = archive {
        let (archived, archive_elapsed) = tokio::time::timeout(Duration::from_secs(5), archive)
            .await
            .expect("archive makes progress after foreground commits")
            .expect("join real archive owner");
        assert_eq!(
            archived.0, 64,
            "successful requests must not hide stopped maintenance"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id LIKE 'maintenance-source-%'"
            )
            .fetch_one(&state.pool)
            .await
            .expect("read remaining sources"),
            0,
        );
        let committed_while_maintenance_locked = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE source='proxy'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("business facts remain independently durable");
        assert_eq!(committed_while_maintenance_locked, 32);
        let text = archive_metrics.render();
        let metric = |name: &str, required: &[&str]| {
            text.lines()
                .find(|line| {
                    line.starts_with(name)
                        && line.contains("operation=\"invocation_archive\"")
                        && required.iter().all(|label| line.contains(label))
                })
                .unwrap_or_else(|| panic!("missing actual archive metric {name} {required:?}"))
                .split_whitespace()
                .last()
                .expect("metric value")
                .parse::<f64>()
                .expect("numeric archive metric")
        };
        let holds = metric(
            "cvm_retention_phase_duration_seconds_count{",
            &["phase=\"write_hold\""],
        );
        let within_budget = metric(
            "cvm_retention_phase_duration_seconds_bucket{",
            &["phase=\"write_hold\"", "le=\"0.25\""],
        );
        let committed_rows = metric("cvm_maintenance_archived_rows_total{", &[]);
        let coordinator_metric = |name: &str, required: &[&str]| {
            text.lines()
                .find(|line| {
                    line.starts_with(name)
                        && line.contains("class=\"maintenance_retention\"")
                        && required.iter().all(|label| line.contains(label))
                })
                .unwrap_or_else(|| panic!("missing full maintenance hold metric {name}"))
                .split_whitespace()
                .last()
                .expect("coordinator metric value")
                .parse::<f64>()
                .expect("numeric coordinator metric")
        };
        let all_holds =
            coordinator_metric("cvm_sqlite_coordinator_hold_duration_seconds_count{", &[]);
        let all_within_budget = coordinator_metric(
            "cvm_sqlite_coordinator_hold_duration_seconds_bucket{",
            &["le=\"0.25\""],
        );
        assert_eq!(committed_rows, 64.0);
        assert!(
            holds >= 20.0,
            "enough actual chunks to evaluate this controlled p95"
        );
        assert!(
            within_budget >= (holds * 0.95).ceil(),
            "controlled full write-hold p95 exceeds 250ms: samples={holds}, within_budget={within_budget}"
        );
        assert!(all_holds >= holds);
        assert!(
            all_within_budget >= (all_holds * 0.95).ceil(),
            "full maintenance holds including publication exceed controlled p95<=250ms: samples={all_holds}, within_budget={all_within_budget}"
        );
        println!(
            "runtime_maintenance_contention archived_rows={} terminal_committed={} maintenance_lock_held_through_terminal=true archive_elapsed_ms={} archive_rows_per_second={} source_backlog=0 write_hold_samples={} write_hold_le_250ms={} all_maintenance_hold_samples={} all_maintenance_hold_le_250ms={} low_sample={}",
            archived.0,
            committed_while_maintenance_locked,
            archive_elapsed.as_millis(),
            archived.0 as f64 / archive_elapsed.as_secs_f64(),
            holds,
            within_budget,
            all_holds,
            all_within_budget,
            all_holds < 100.0,
        );
    }
    if let Some(mut locked) = maintenance_lock.take() {
        sqlx::query("ROLLBACK")
            .execute(&mut *locked)
            .await
            .expect("unlock confirmations");
        drop(locked);
    }
    if let Some((pool, _)) = &maintenance_pool {
        // Confirmation can remain in capped backoff after unlock; business durability
        // above was checked while the independent maintenance lock was still held.
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let confirmed = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM startup_backfill_progress WHERE wake_generation > 0",
                )
                .fetch_one(pool)
                .await
                .expect("read recovered maintenance obligations");
                if confirmed > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("valid obligations recover after maintenance unlock");
    }
    maintenance_cancel.cancel();
    if let Some(confirmation) = maintenance_confirmation {
        confirmation.await.expect("drain confirmation owner");
    }
    if let Some(runtime) = state.upstream_accounts.route_facts.lock().await.clone() {
        runtime.shutdown_and_sync().await;
    }
    state.shutdown.cancel();
    state.sqlite_batch_writer.shutdown_and_drain().await;
    upstream_handle.abort();
    state.pool.close().await;
    if let Some((pool, archive_fixture)) = maintenance_pool {
        pool.close().await;
        cleanup_temp_test_dir(&archive_fixture);
    }
    fs::remove_dir_all(fixture).expect("remove owned pressure fixture");
}
