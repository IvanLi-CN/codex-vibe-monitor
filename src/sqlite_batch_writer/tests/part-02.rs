#[tokio::test]
async fn failed_p1_flush_retains_the_full_accounted_batch() {
    let pool = test_pool().await;
    pool.close().await;

    let accounting = PendingQueueAccounting::default();
    let write = SqliteBatchWrite::TerminalInvocation(terminal_write_for_coalescing(
        "failed-p1-accounting",
        Some(9),
    ));
    accounting.enqueue(write.estimated_memory_bytes());
    let mut batch = PendingBatch::default();
    batch.push_accounted(write, &accounting);
    let submitted_bytes = batch.estimated_memory_bytes();

    let retained = flush_pending_batch_accounted!(
        &accounting,
        &pool,
        None,
        batch,
        FlushReason::Barrier,
        None,
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(Mutex::new(())),
        &Arc::new(std::sync::Mutex::new(None)),
    )
    .await
    .expect("failed P1 flush should retain its batch");

    assert!(retained.failed);
    assert_eq!(retained.batch.estimated_memory_bytes(), submitted_bytes);
    let snapshot = accounting.snapshot();
    assert_eq!(snapshot.pending_depth, retained.batch.logical_rows());
    assert_eq!(snapshot.pending_bytes, submitted_bytes);
    assert_eq!(snapshot.retry_count, 0);
    assert_eq!(snapshot.state, "healthy");

    let retried = flush_pending_batch_accounted!(
        &accounting,
        &pool,
        None,
        retained.batch,
        FlushReason::Interval,
        None,
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(Mutex::new(())),
        &Arc::new(std::sync::Mutex::new(None)),
    )
    .await
    .expect("failed retained batch should remain available after retry");
    assert!(retried.failed);
    assert_eq!(accounting.snapshot().retry_count, 1);
}

#[tokio::test]
async fn attempt_progress_batch_coalesces_by_attempt_id() {
    let pool = test_pool().await;
    let pending = pending_attempt(&pool, "batch-progress-coalesce").await;
    let attempt_id = pending.attempt_id.expect("attempt id");

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![
            SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST.to_string(),
                connect_latency_ms: Some(12.0),
                first_byte_latency_ms: None,
                compact_support_status: None,
                compact_support_reason: None,
            }),
            SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
                connect_latency_ms: Some(18.0),
                first_byte_latency_ms: Some(33.0),
                compact_support_status: Some("supported".to_string()),
                compact_support_reason: Some("cached_probe".to_string()),
            }),
        ],
    )
    .await;

    let row = sqlx::query_as::<
            _,
            (
                String,
                Option<f64>,
                Option<f64>,
                Option<String>,
                Option<String>,
            ),
        >(
            r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms, compact_support_status, compact_support_reason
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced attempt");

    assert_eq!(
        row.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE
    );
    assert_eq!(row.1, Some(18.0));
    assert_eq!(row.2, Some(33.0));
    assert_eq!(row.3.as_deref(), Some("supported"));
    assert_eq!(row.4.as_deref(), Some("cached_probe"));
}

#[tokio::test]
async fn attempt_progress_batch_does_not_overwrite_terminal_finalize() {
    let pool = test_pool().await;
    let pending = pending_attempt(&pool, "batch-progress-terminal-cover").await;
    let attempt_id = pending.attempt_id.expect("attempt id");

    finalize_pool_upstream_request_attempt(
        &pool,
        &pending,
        "2026-07-01 10:00:05",
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(StatusCode::OK),
        None,
        None,
        None,
        None,
        Some(42.0),
        Some(16.0),
        Some(188.0),
        Some("req_terminal"),
        None,
        None,
    )
    .await
    .expect("finalize attempt synchronously");

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string(),
            connect_latency_ms: Some(99.0),
            first_byte_latency_ms: Some(99.0),
            compact_support_status: Some("stale".to_string()),
            compact_support_reason: Some("should_not_apply".to_string()),
        })],
    )
    .await;

    let row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<i64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
            SELECT
                status,
                phase,
                http_status,
                connect_latency_ms,
                first_byte_latency_ms,
                stream_latency_ms,
                upstream_request_id,
                compact_support_status
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
    )
    .bind(attempt_id)
    .fetch_one(&pool)
    .await
    .expect("load finalized attempt");

    assert_eq!(row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        row.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(row.2, Some(200));
    assert_eq!(row.3, Some(42.0));
    assert_eq!(row.4, Some(16.0));
    assert_eq!(row.5, Some(188.0));
    assert_eq!(row.6.as_deref(), Some("req_terminal"));
    assert_eq!(row.7, None);
}

#[tokio::test]
async fn shutdown_drains_pending_batch_writes() {
    let pool = test_pool().await;
    let pending = pending_attempt(&pool, "batch-progress-shutdown-drain").await;
    let attempt_id = pending.attempt_id.expect("attempt id");
    let shutdown = CancellationToken::new();
    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        shutdown.clone(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );

    assert!(
        writer.enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
            connect_latency_ms: Some(21.0),
            first_byte_latency_ms: Some(34.0),
            compact_support_status: None,
            compact_support_reason: None,
        }))
    );

    shutdown.cancel();
    writer.shutdown_and_drain().await;

    let row = sqlx::query_as::<_, (Option<String>, Option<f64>, Option<f64>)>(
        r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
    )
    .bind(attempt_id)
    .fetch_one(&pool)
    .await
    .expect("load drained attempt progress");

    assert_eq!(
        row.0.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    assert_eq!(row.1, Some(21.0));
    assert_eq!(row.2, Some(34.0));
    assert_eq!(writer.stats_snapshot(), (0, 0));
    let accounting = writer.accounting_snapshot();
    assert_eq!(accounting.pending_depth, 0);
    assert_eq!(accounting.pending_bytes, 0);
    assert_eq!(accounting.state, "healthy");
}

#[tokio::test]
async fn flush_now_applies_pending_writes_through_control_path() {
    let pool = test_pool().await;
    let pending = pending_attempt(&pool, "batch-progress-flush-now").await;
    let attempt_id = pending.attempt_id.expect("attempt id");
    let shutdown = CancellationToken::new();
    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        shutdown.clone(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );

    assert!(
        writer.enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string(),
            connect_latency_ms: Some(23.0),
            first_byte_latency_ms: Some(37.0),
            compact_support_status: None,
            compact_support_reason: None,
        }))
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), writer.flush_now(&pool))
        .await
        .expect("flush_now should not be starved by normal write traffic")
        .expect("flush pending write");

    let row = sqlx::query_as::<_, (Option<String>, Option<f64>, Option<f64>)>(
        r#"
            SELECT phase, connect_latency_ms, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            "#,
    )
    .bind(attempt_id)
    .fetch_one(&pool)
    .await
    .expect("load flushed attempt progress");

    assert_eq!(
        row.0.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    assert_eq!(row.1, Some(23.0));
    assert_eq!(row.2, Some(37.0));
    writer.shutdown_and_drain().await;
}

#[tokio::test]
async fn flush_now_resets_completed_p2_schedule() {
    let pool = test_pool().await;
    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        CancellationToken::new(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );
    assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_995,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    )));

    tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            if writer.accounting_snapshot().p2_wake_reason.as_deref() == Some("coalesced_deadline")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("writer should arm the P2 coalescing deadline before FlushNow");

    writer
        .flush_now(&pool)
        .await
        .expect("FlushNow should complete the pending P2 batch");
    let reset = writer.accounting_snapshot();
    assert_eq!(reset.pending_depth, 0);
    assert_eq!(reset.p2_next_attempt_in_ms, 0);
    assert_eq!(reset.p2_wake_reason, None);

    assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_994,
            selected_at: "2026-08-10T12:00:01Z".to_string(),
        },
    )));
    tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            let snapshot = writer.accounting_snapshot();
            if snapshot.p2_wake_reason.as_deref() == Some("coalesced_deadline") {
                assert!(snapshot.p2_next_attempt_in_ms >= 200);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("new P2 work should receive a full coalescing interval");
    writer.shutdown_and_drain().await;
}

// The producer intentionally saturates the queue while the writer waits for its deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_p2_work_does_not_starve_its_scheduled_flush() {
    let pool = test_pool().await;
    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        CancellationToken::new(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );
    assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_993,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    )));
    tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            if writer.accounting_snapshot().p2_wake_reason.as_deref() == Some("coalesced_deadline")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("writer should arm the P2 coalescing deadline");

    let producer_active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let producer = {
        let producer_active = producer_active.clone();
        let producer_writer = writer.clone();
        tokio::spawn(async move {
            while producer_active.load(std::sync::atomic::Ordering::Acquire) {
                for index in 0..1024 {
                    let _ = producer_writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
                        BatchedAccountSelectedTouch {
                            account_id: 999_992,
                            selected_at: "2026-08-10T12:00:01Z".to_string(),
                        },
                    ));
                    if index % 32 == 31 {
                        tokio::task::yield_now().await;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
    };

    let resumed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if writer.accounting_snapshot().p2_flush_attempt_count > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok();
    producer_active.store(false, std::sync::atomic::Ordering::Release);
    producer.abort();
    let _ = producer.await;
    writer.shutdown_and_drain().await;
    assert!(
        resumed,
        "queued P2 writes must not prevent the scheduled P2 flush"
    );
}

#[tokio::test]
async fn system_task_finish_batch_marks_running_task_terminal() {
    let pool = test_pool().await;
    let handle = begin_system_task_run(
        &pool,
        SystemTaskKind::StartupBackfill,
        "test",
        Some("started".to_string()),
    )
    .await
    .expect("begin system task");

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![SqliteBatchWrite::SystemTaskFinish(
            BatchedSystemTaskFinish {
                run_id: handle.id,
                task_kind: handle.task_kind,
                trigger_kind: handle.trigger_kind.clone(),
                status: SystemTaskStatus::Success,
                summary: Some("completed".to_string()),
                detail: None,
                finished_at: "2026-07-01T10:00:05Z".to_string(),
                duration_ms: 125,
            },
        )],
    )
    .await;

    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<i64>)>(
        r#"
            SELECT status, summary, finished_at, duration_ms
            FROM system_task_runs
            WHERE id = ?1
            "#,
    )
    .bind(handle.id)
    .fetch_one(&pool)
    .await
    .expect("load finished system task run");

    assert_eq!(row.0, SystemTaskStatus::Success.as_str());
    assert_eq!(row.1.as_deref(), Some("completed"));
    assert_eq!(row.2.as_deref(), Some("2026-07-01T10:00:05Z"));
    assert_eq!(row.3, Some(125));
}

#[test]
fn quarantined_system_task_finish_is_not_acknowledged_as_committed() {
    let run_id = 991_001;
    let retained = RetainedBatch::new(PendingBatch::default(), false)
        .with_quarantined_system_task_ids(vec![run_id]);
    assert!(successfully_flushed_system_task_ids(&[run_id], Some(&retained)).is_empty());
    assert_eq!(
        successfully_flushed_system_task_ids(&[run_id], None),
        vec![run_id]
    );
}

#[tokio::test]
async fn invocation_derived_batch_advances_live_progress_cursor_with_replay() {
    let pool = test_pool().await;
    save_hourly_rollup_live_progress_tx(
        pool.acquire().await.expect("acquire").as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        0,
    )
    .await
    .expect("seed live progress");

    let row_count = BACKFILL_BATCH_SIZE + 5;
    for index in 0..row_count {
        sqlx::query(
            r#"
                INSERT INTO codex_invocations (
                    invoke_id,
                    occurred_at,
                    source,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_tokens,
                    cost,
                    status,
                    raw_response,
                    detail_level
                )
                VALUES (?1, ?2, 'proxy', 1, 2, 0, 3, 0.01, 'success', '', 'full')
                "#,
        )
        .bind(format!("batch-derived-{index}"))
        .bind(format!("2026-07-01 10:{:02}:00", index % 60))
        .execute(&pool)
        .await
        .expect("seed invocation");
    }

    let max_id = sqlx::query_scalar::<_, i64>("SELECT MAX(id) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("load max invocation id");

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![SqliteBatchWrite::InvocationDerived(
            BatchedInvocationDerivedWrites {
                invocation_id: max_id,
                occurred_at: "2026-07-01 10:00:00".to_string(),
                upstream_account_id: None,
                terminal_overlay_key: None,
            },
        )],
    )
    .await;

    let cursor = load_hourly_rollup_live_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .await
        .expect("load live progress");
    assert_eq!(cursor, max_id);
}

#[tokio::test]
async fn terminal_invocation_batch_persists_and_updates_rollups() {
    let pool = test_pool().await;
    save_hourly_rollup_live_progress_tx(
        pool.acquire().await.expect("acquire").as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        0,
    )
    .await
    .expect("seed live progress");

    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let mut record = build_running_proxy_capture_record(
        "batch-terminal-invocation",
        "2026-07-01 10:00:00",
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("192.0.2.44"),
        Some("sticky-a"),
        Some("pck-a"),
        true,
        Some(99),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        3.0,
        4.0,
        5.0,
        6.0,
    );
    record.status = "success".to_string();
    record.usage.input_tokens = Some(2);
    record.usage.output_tokens = Some(3);
    record.usage.total_tokens = Some(5);
    record.cost = Some(0.02);

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: Vec::new(),
                record,
            },
        )],
    )
    .await;

    let row = sqlx::query_as::<_, (String, i64, i64, Option<i64>)>(
        r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END
            FROM codex_invocations
            WHERE invoke_id = 'batch-terminal-invocation'
            LIMIT 1
            "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load terminal invocation");

    assert_eq!(row.0, "success");
    assert_eq!(row.1, 2);
    assert_eq!(row.2, 3);
    assert_eq!(row.3, Some(99));

    let rollup = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
            SELECT
                COALESCE(SUM(total_count), 0),
                COALESCE(SUM(success_count), 0),
                COALESCE(SUM(total_tokens), 0)
            FROM invocation_rollup_hourly
            WHERE source = 'proxy'
            "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation rollup");

    assert_eq!(rollup.0, 1);
    assert_eq!(rollup.1, 1);
    assert_eq!(rollup.2, 5);
}

async fn assert_deferred_terminal_flush_before_p2(
    pool: &SqlitePool,
    writer: &SqliteBatchWriter,
    runtime_store: &Arc<ProxyRuntimeInvocationStore>,
) {
    let retained = writer.accounting_snapshot();
    assert_eq!(retained.state, "healthy");
    assert!(retained.pending_bytes > 0);
    assert!(retained.pending_depth > 0);
    assert!(retained.transfer_bytes > 0);
    let persisted_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'batch-terminal-flush-now-deferred' AND status = 'success'",
        )
        .fetch_one(pool)
        .await
        .expect("count terminal invocation after forced flush");
    assert_eq!(persisted_count, 1);
    let rollup_count = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = 'proxy'",
    )
    .fetch_one(pool)
    .await
    .expect("load rollup before deferred flush");
    assert_eq!(rollup_count, 0);
    assert_eq!(runtime_store.snapshot().len(), 1);
}

async fn assert_deferred_terminal_flush_completed(
    pool: &SqlitePool,
    writer: &SqliteBatchWriter,
    runtime_store: &Arc<ProxyRuntimeInvocationStore>,
) {
    let rollup_count = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = 'proxy'",
    )
    .fetch_one(pool)
    .await
    .expect("load rollup after deferred flush");
    assert_eq!(rollup_count, 1);
    assert!(runtime_store.snapshot().is_empty());
    let completed = writer.accounting_snapshot();
    assert_eq!(completed.pending_depth, 0);
    assert_eq!(completed.pending_bytes, 0);
    assert_eq!(completed.state, "healthy");
}

async fn wait_for_interactive_waiter(
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if coordinator.snapshot().await.interactive_waiter_count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "interactive coordinator waiter did not register"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn wait_for_p1_waiter(
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if coordinator.snapshot().await.p1_waiter_count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "P1 coordinator waiter did not register"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn flush_now_treats_deferred_terminal_derived_writes_as_success() {
    let pool = test_pool().await;
    save_hourly_rollup_live_progress_tx(
        pool.acquire().await.expect("acquire").as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        0,
    )
    .await
    .expect("seed live progress");

    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let mut record = build_running_proxy_capture_record(
        "batch-terminal-flush-now-deferred-derived",
        "2026-07-01 10:00:00",
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("192.0.2.44"),
        Some("sticky-a"),
        Some("pck-a"),
        true,
        Some(99),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        3.0,
        4.0,
        5.0,
        6.0,
    );
    record.status = "success".to_string();
    record.usage.input_tokens = Some(2);
    record.usage.output_tokens = Some(3);
    record.usage.total_tokens = Some(5);

    let runtime_store = Arc::new(ProxyRuntimeInvocationStore::default());
    let runtime_record = api_invocation_from_runtime_record(&record);
    runtime_store.upsert_terminal(runtime_record);
    assert_eq!(runtime_store.snapshot().len(), 1);

    let writer = SqliteBatchWriter::spawn_for_test();
    writer.set_terminal_runtime_store(runtime_store.clone());
    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
            record,
        },
    )));

    writer
        .flush_now(&pool)
        .await
        .expect("P1 terminal flush should not fail because P2 derived work was deferred");
    assert_deferred_terminal_flush_before_p2(&pool, &writer, &runtime_store).await;

    writer
        .flush_now(&pool)
        .await
        .expect("deferred P2 derived write should flush later");
    assert_deferred_terminal_flush_completed(&pool, &writer, &runtime_store).await;

    writer.shutdown_and_drain().await;
}

#[tokio::test]
async fn flush_now_schedules_retained_p2_for_coordinator_eligibility() {
    let pool = test_pool().await;
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let initial = coordinator.snapshot().await;
    assert!(initial.active_write_class.is_none());
    assert_eq!(initial.p1_waiter_count, 0);
    assert_eq!(initial.interactive_waiter_count, 0);
    assert_eq!(initial.p2_waiter_count, 0);

    let active_p2 = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .await;
    let interactive_coordinator = coordinator.clone();
    let interactive_waiter = tokio::spawn(async move {
        interactive_coordinator
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
            .await
    });
    wait_for_interactive_waiter(&coordinator).await;

    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        CancellationToken::new(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );
    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            record: crate::tests::test_proxy_capture_record(
                "flush-now-coordinator-p2",
                "2026-08-10 12:00:00",
            ),
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        },
    )));

    let flush_writer = writer.clone();
    let flush_pool = pool.clone();
    let flush_task = tokio::spawn(async move { flush_writer.flush_now(&flush_pool).await });
    wait_for_p1_waiter(&coordinator).await;
    drop(active_p2);

    tokio::time::timeout(Duration::from_secs(2), flush_task)
        .await
        .expect("flush_now should finish after the P1 coordinator permit is released")
        .expect("flush_now task should complete")
        .expect("deferred P2 should not fail the flush_now barrier");
    let scheduled = writer.accounting_snapshot();
    assert_eq!(
        scheduled.p2_wake_reason.as_deref(),
        Some("background_eligible"),
        "retained P2 must wait on coordinator eligibility"
    );
    assert!(scheduled.pending_depth > 0);

    let interactive_permit = tokio::time::timeout(Duration::from_secs(2), interactive_waiter)
        .await
        .expect("interactive waiter should be admitted after P1 commits")
        .expect("interactive waiter task should complete");
    drop(interactive_permit);

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = writer.accounting_snapshot();
            if snapshot.pending_depth == 0 && snapshot.p2_flush_attempt_count > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("eligibility notification should resume the retained P2 flush");
    writer.shutdown_and_drain().await;
}

#[test]
fn barrier_drain_includes_deferred_journal_terminals() {
    let root = std::env::temp_dir().join(format!("batch-writer-deferred-{}", nanoid::nanoid!()));
    std::fs::create_dir_all(&root).expect("create journal test directory");
    let database_path = root.join("monitor.db");
    let mut journal = TerminalJournal::open(&database_path).expect("open terminal journal");
    let record = crate::tests::test_proxy_capture_record(
        "batch-writer-deferred-terminal",
        "2026-07-29T00:00:00Z",
    );
    assert!(journal.defer_write(BatchedTerminalInvocationWrite {
        record,
        capture_started: None,
        raw_capture: true,
        dashboard_terminal_sequence: None,
        terminal_projection_event_ids: Vec::new(),
        startup_backfill_tasks: Vec::new(),
    }));
    let journal = Arc::new(std::sync::Mutex::new(Some(journal)));
    let mut pending = PendingBatch::default();

    let accounting = PendingQueueAccounting::default();
    let queued_p1_count = AtomicUsize::new(0);
    drain_terminal_journal_deferred_writes(
        &journal,
        &mut pending,
        &accounting,
        usize::MAX,
        &queued_p1_count,
    );

    assert_eq!(pending.terminal_invocations.len(), 1);
    let snapshot = accounting.snapshot();
    assert_eq!(snapshot.pending_depth, 1);
    assert_eq!(snapshot.pending_bytes, pending.estimated_memory_bytes());
    assert_eq!(snapshot.retry_count, 1);
    assert_eq!(snapshot.state, "healthy");
    assert!(
        journal
            .lock()
            .expect("lock terminal journal")
            .as_mut()
            .expect("terminal journal available")
            .take_deferred_writes(usize::MAX)
            .is_empty()
    );
    std::fs::remove_dir_all(root).expect("remove journal test directory");
}

#[tokio::test]
async fn shutdown_drain_flushes_terminal_invocations() {
    let pool = test_pool().await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.5".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let mut record = build_running_proxy_capture_record(
        "batch-terminal-shutdown-drain",
        "2026-07-01 10:00:00",
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("192.0.2.44"),
        Some("sticky-a"),
        Some("pck-a"),
        true,
        Some(42),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        3.0,
        4.0,
        5.0,
        6.0,
    );
    record.status = "success".to_string();

    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        CancellationToken::new(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );
    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
            record,
        },
    )));

    writer.shutdown_and_drain().await;

    let persisted_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'batch-terminal-shutdown-drain' AND status = 'success'",
        )
        .fetch_one(&pool)
        .await
        .expect("count drained terminal invocation");
    assert_eq!(persisted_count, 1);
}

#[tokio::test]
async fn account_selected_touch_batch_coalesces_by_account_id() {
    let pool = test_pool().await;
    sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
            )
            VALUES (77, 'api_key', 'codex', 'Primary', 'active', 1, NULL, '2026-07-01T09:59:00Z', '2026-07-01T09:59:00Z')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed account");

    SqliteBatchWriter::flush_for_test(
        &pool,
        vec![
            SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                account_id: 77,
                selected_at: "2026-07-01T10:00:00Z".to_string(),
            }),
            SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                account_id: 77,
                selected_at: "2026-07-01T10:00:05Z".to_string(),
            }),
            SqliteBatchWrite::AccountSelectedTouch(BatchedAccountSelectedTouch {
                account_id: 77,
                selected_at: "2026-07-01T10:00:02Z".to_string(),
            }),
        ],
    )
    .await;

    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_selected_at, updated_at FROM pool_upstream_accounts WHERE id = 77",
    )
    .fetch_one(&pool)
    .await
    .expect("load selected account");

    assert_eq!(row.0.as_deref(), Some("2026-07-01T10:00:05Z"));
    assert_eq!(row.1.as_deref(), Some("2026-07-01T10:00:05Z"));
}
