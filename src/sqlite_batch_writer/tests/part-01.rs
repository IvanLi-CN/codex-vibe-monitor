use super::*;

#[test]
fn p1_retry_backoff_is_bounded_and_new_work_does_not_reset_deadline() {
    let mut retry = P1RetryState::default();
    let expected = [250_u128, 500, 1_000, 2_000, 5_000, 5_000];
    for (generation, expected_ms) in expected.into_iter().enumerate() {
        let delay = retry.failed(0);
        assert_eq!(delay.as_millis(), expected_ms, "generation {generation}");
        let due_at = retry.due_at.expect("failed attempt sets deadline");
        assert!(!retry.ready(Instant::now()));
        assert_eq!(
            retry.due_at,
            Some(due_at),
            "enqueue does not mutate retry state"
        );
    }
    retry.succeeded();
    assert!(retry.ready(Instant::now()));
    assert_eq!(retry.generation, 0);
}

#[test]
fn p2_schedule_coalesces_and_separates_pressure_from_lock_retries() {
    let accounting = PendingQueueAccounting::default();
    let mut schedule = P2ScheduleState::default();
    let now = Instant::now();
    schedule.arm_if_idle(now);
    let initial_due = schedule.due_at;
    schedule.arm_if_idle(now + Duration::from_millis(100));
    assert_eq!(
        schedule.due_at, initial_due,
        "new work must not extend deadline"
    );

    accounting.p2_pressure_deferred();
    schedule.defer_pressure(
        Duration::from_secs(30),
        P2WakeReason::PressureCooldownElapsed,
    );
    accounting.update_p2_schedule(&schedule);
    let pressure = accounting.snapshot();
    assert_eq!(pressure.retry_count, 0);
    assert_eq!(pressure.p2_pressure_defer_count, 1);
    assert!(pressure.p2_next_attempt_in_ms >= 29_000);

    let delay = schedule.failed(0);
    accounting.p2_lock_retried();
    accounting.retry_deferred();
    accounting.update_p2_schedule(&schedule);
    let locked = accounting.snapshot();
    assert_eq!(delay, Duration::from_millis(250));
    assert_eq!(locked.retry_count, 1);
    assert_eq!(locked.p2_lock_retry_count, 1);
    assert_eq!(locked.p2_wake_reason.as_deref(), Some("lock_retry"));
}

#[tokio::test]
async fn p2_pressure_defer_waits_for_its_deadline_without_a_20ms_retry() {
    let mut schedule = P2ScheduleState::default();
    schedule.defer_pressure(
        Duration::from_millis(80),
        P2WakeReason::PressureCooldownElapsed,
    );

    assert!(
        tokio::time::timeout(
            Duration::from_millis(40),
            wait_for_p2_deadline(schedule.due_at),
        )
        .await
        .is_err(),
        "P2 pressure defer must not wake on the 20ms P1 ticker"
    );
    tokio::time::timeout(
        Duration::from_millis(100),
        wait_for_p2_deadline(schedule.due_at),
    )
    .await
    .expect("P2 pressure deadline should eventually wake");
}

#[test]
fn queued_p1_write_is_drained_before_ready_p2_deadline() {
    let accounting = PendingQueueAccounting::default();
    let mut pending = PendingBatch::default();
    pending.push(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_998,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    ));
    let mut schedule = P2ScheduleState::default();
    schedule.wake_background_eligible();
    let queued_p1_count = AtomicUsize::new(0);
    let (sender, mut receiver) = mpsc::channel(1);
    sender
        .try_send(SqliteBatchWrite::TerminalInvocation(
            terminal_write_for_coalescing("queued-p1-priority", None),
        ))
        .expect("queue P1 terminal");
    drain_queued_writes_before_dispatch(
        &mut receiver,
        &mut pending,
        &accounting,
        &mut schedule,
        SQLITE_BATCH_MAX_ROWS,
        &queued_p1_count,
    );

    assert!(
        receiver.is_empty(),
        "queued P1 must be classified before checking the P2 deadline"
    );
    assert_eq!(pending.terminal_invocations.len(), 1);
    assert!(p2_deadline_wait_armed(
        &pending,
        &schedule,
        &queued_p1_count,
        &P1RetryState::default()
    ));
}

#[test]
fn queued_p2_does_not_block_ready_p2_deadline() {
    let accounting = PendingQueueAccounting::default();
    let mut pending = PendingBatch::default();
    pending.push(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_997,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    ));
    let mut schedule = P2ScheduleState::default();
    schedule.wake_background_eligible();
    let queued_p1_count = AtomicUsize::new(0);
    let (sender, mut receiver) = mpsc::channel(1);
    sender
        .try_send(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: 999_996,
                selected_at: "2026-08-10T12:00:01Z".to_string(),
            },
        ))
        .expect("queue P2 write");

    drain_queued_writes_before_dispatch(
        &mut receiver,
        &mut pending,
        &accounting,
        &mut schedule,
        SQLITE_BATCH_MAX_ROWS,
        &queued_p1_count,
    );

    assert!(receiver.is_empty());
    assert!(p2_deadline_wait_armed(
        &pending,
        &schedule,
        &queued_p1_count,
        &P1RetryState::default()
    ));
}

#[test]
fn p1_retry_backoff_blocks_p2_deadline() {
    let mut pending = PendingBatch::default();
    pending.push(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_995,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    ));
    let mut schedule = P2ScheduleState::default();
    schedule.wake_background_eligible();
    let queued_p1_count = AtomicUsize::new(0);
    let mut p1_retry = P1RetryState::default();
    p1_retry.failed(0);

    assert!(!p2_deadline_wait_armed(
        &pending,
        &schedule,
        &queued_p1_count,
        &p1_retry
    ));
}

#[test]
fn p2_deadline_wait_registers_before_its_coalescing_deadline() {
    let mut pending = PendingBatch::default();
    pending.push(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: 999_994,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    ));
    let mut schedule = P2ScheduleState::default();
    schedule.arm_if_idle(Instant::now());
    let queued_p1_count = AtomicUsize::new(0);

    assert!(!schedule.ready(Instant::now()));
    assert!(p2_deadline_wait_armed(
        &pending,
        &schedule,
        &queued_p1_count,
        &P1RetryState::default()
    ));
}

#[tokio::test]
async fn p2_eligibility_wake_resumes_a_deferred_flush() {
    let mut schedule = P2ScheduleState::default();
    schedule.arm_if_idle(Instant::now());
    schedule.defer_until_background_eligible();

    assert!(!schedule.ready(Instant::now() + Duration::from_secs(30)));
    assert_eq!(schedule.next_attempt_in_ms(), 0);
    assert_eq!(schedule.wake_reason, Some(P2WakeReason::BackgroundEligible));

    schedule.arm_if_idle(Instant::now());
    assert!(schedule.due_at.is_none(), "new P2 work must only coalesce");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let permit = gate
        .try_begin_background("test_p2_eligibility_wake")
        .expect("acquire background permit");
    let observed_generation = gate.eligibility_generation();
    let wait_for_eligibility = gate.wait_for_eligibility_change(observed_generation);
    drop(permit);
    tokio::time::timeout(Duration::from_millis(100), wait_for_eligibility)
        .await
        .expect("releasing background capacity should notify P2 eligibility waiters");

    schedule.wake_background_eligible();
    assert!(schedule.ready(Instant::now()));
    tokio::time::timeout(
        Duration::from_millis(20),
        wait_for_p2_deadline(schedule.due_at),
    )
    .await
    .expect("eligibility wake should make P2 dispatch immediately");
}

#[test]
fn pending_queue_accounting_clamps_underflow_and_degrades_health() {
    let accounting = PendingQueueAccounting::default();

    accounting.complete(0, 0, 64, 0);

    let snapshot = accounting.snapshot();
    assert_eq!(snapshot.pending_depth, 0);
    assert_eq!(snapshot.pending_bytes, 0);
    assert_eq!(snapshot.state, "degraded");
    assert_eq!(snapshot.invariant_violation_count, 1);
    let violation = snapshot
        .last_invariant_violation
        .expect("underflow should retain actionable telemetry");
    assert_eq!(violation.operation, "completion");
    assert_eq!(violation.expected_bytes, 64);
    assert_eq!(violation.actual_bytes, 0);
}

#[test]
fn pending_queue_accounting_preserves_retry_transfer_and_retained_bytes() {
    let accounting = PendingQueueAccounting::default();

    accounting.enqueue(120);
    accounting.retry_deferred();
    accounting.replace_batch(1, 1, 120, 96);
    accounting.transfer_p1_to_p2(40);
    accounting.complete(1, 1, 96, 40);

    let snapshot = accounting.snapshot();
    assert_eq!(snapshot.pending_depth, 1);
    assert_eq!(snapshot.pending_bytes, 40);
    assert_eq!(snapshot.transfer_bytes, 40);
    assert_eq!(snapshot.retry_count, 1);
    assert_eq!(snapshot.state, "healthy");
    assert!(snapshot.last_invariant_violation.is_none());

    accounting.complete(1, 0, 40, 0);
    let completed = accounting.snapshot();
    assert_eq!(completed.pending_depth, 0);
    assert_eq!(completed.pending_bytes, 0);
    assert_eq!(completed.state, "healthy");
}

#[test]
fn pending_queue_accounting_rolls_back_failed_sender_admission() {
    let accounting = PendingQueueAccounting::default();

    accounting.enqueue(72);
    accounting.rollback_enqueue(72);

    let snapshot = accounting.snapshot();
    assert_eq!(snapshot.pending_depth, 0);
    assert_eq!(snapshot.pending_bytes, 0);
    assert_eq!(snapshot.state, "healthy");
    assert_eq!(snapshot.invariant_violation_count, 0);
}
use axum::http::StatusCode;
use sqlx::SqlitePool;

fn attempt_trace(invoke_id: &str) -> PoolUpstreamAttemptTraceContext {
    PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: "2026-07-01 10:00:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some(format!("{invoke_id}-sticky")),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    }
}

async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect sqlite memory pool");
    ensure_schema(&pool).await.expect("ensure schema");
    pool
}

const PRIORITY_ACCOUNT_ID: i64 = 999_998;
const PRIORITY_P1_INVOKE_ID: &str = "deep-deferred-p2-p1-priority";

async fn seed_priority_dispatch_schema(pool: &SqlitePool) {
    sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
            )
            VALUES (999998, 'api_key', 'codex', 'Priority Test', 'active', 1, NULL, '2026-08-10T12:00:00Z', '2026-08-10T12:00:00Z')
            "#,
        )
        .execute(pool)
        .await
        .expect("seed P2 account");
    for offset in 1..=SQLITE_BATCH_MAX_ROWS {
        sqlx::query(
                r#"
                INSERT INTO pool_upstream_accounts (
                    id, kind, provider, display_name, status, enabled, last_selected_at, created_at, updated_at
                )
                VALUES (?1, 'api_key', 'codex', 'Priority Test', 'active', 1, NULL, '2026-08-10T12:00:00Z', '2026-08-10T12:00:00Z')
                "#,
            )
            .bind(PRIORITY_ACCOUNT_ID + offset as i64)
            .execute(pool)
            .await
            .expect("seed deep P2 account");
    }
    sqlx::query("CREATE TABLE batch_writer_dispatch_order (write_class TEXT NOT NULL)")
        .execute(pool)
        .await
        .expect("create dispatch order table");
    sqlx::query(
        r#"
            CREATE TRIGGER batch_writer_p1_dispatch_order
            AFTER INSERT ON codex_invocations
            WHEN NEW.invoke_id = 'deep-deferred-p2-p1-priority'
            BEGIN
                INSERT INTO batch_writer_dispatch_order (write_class) VALUES ('p1');
            END
            "#,
    )
    .execute(pool)
    .await
    .expect("create P1 dispatch order trigger");
    sqlx::query(
        r#"
            CREATE TRIGGER batch_writer_p2_dispatch_order
            AFTER UPDATE OF last_selected_at ON pool_upstream_accounts
            WHEN NEW.id BETWEEN 999998 AND 1000030
            BEGIN
                INSERT INTO batch_writer_dispatch_order (write_class) VALUES ('p2');
            END
            "#,
    )
    .execute(pool)
    .await
    .expect("create P2 dispatch order trigger");
}

#[tokio::test]
async fn normal_p2_schedule_becomes_ready_after_its_deadline() {
    let mut schedule = P2ScheduleState::default();
    schedule.arm_if_idle(Instant::now());
    tokio::time::timeout(
        SQLITE_P2_COALESCE_INTERVAL + Duration::from_millis(100),
        wait_for_p2_deadline(schedule.due_at),
    )
    .await
    .expect("normal P2 coalescing deadline should elapse");

    assert!(
        schedule.ready(Instant::now()),
        "normal P2 schedule should be eligible after its fixed deadline"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn p1_queued_after_a_deep_p2_backlog_runs_first() {
    let pool = test_pool().await;
    seed_priority_dispatch_schema(&pool).await;

    let writer = SqliteBatchWriter::spawn(
        pool.clone(),
        CancellationToken::new(),
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default())),
        Arc::new(RwLock::new(PricingCatalog::default())),
        &std::env::temp_dir().join(format!("sqlite-batch-writer-{}.db", nanoid::nanoid!())),
    );

    assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
        BatchedAccountSelectedTouch {
            account_id: PRIORITY_ACCOUNT_ID,
            selected_at: "2026-08-10T12:00:00Z".to_string(),
        },
    )));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if writer.accounting_snapshot().p2_wake_reason.as_deref() == Some("coalesced_deadline")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("initial P2 batch should arm its fixed coalescing deadline");

    for offset in 0..=SQLITE_BATCH_MAX_ROWS {
        assert!(writer.enqueue(SqliteBatchWrite::AccountSelectedTouch(
            BatchedAccountSelectedTouch {
                account_id: PRIORITY_ACCOUNT_ID + offset as i64,
                selected_at: format!("2026-08-10T12:01:{offset:02}Z"),
            },
        )));
    }
    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(
        terminal_write_for_coalescing(PRIORITY_P1_INVOKE_ID, None),
    )));

    // Keep the single-threaded writer asleep until its fixed P2 deadline is overdue.
    std::thread::sleep(SQLITE_P2_COALESCE_INTERVAL + Duration::from_millis(10));
    let order = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let order = sqlx::query_scalar::<_, String>(
                "SELECT write_class FROM batch_writer_dispatch_order ORDER BY rowid",
            )
            .fetch_all(&pool)
            .await
            .expect("load writer dispatch order");
            if order.len() >= 2 {
                return order;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await;
    writer.shutdown_and_drain().await;

    let order = order.expect("writer should dispatch both P1 and P2 work");
    assert_eq!(
        order.first().map(String::as_str),
        Some("p1"),
        "the writer must classify the full queued snapshot before dispatching overdue P2 work"
    );
    assert!(
        order.iter().skip(1).all(|class| class == "p2"),
        "all remaining writes should be bounded P2 chunks"
    );
}

async fn pending_attempt(pool: &SqlitePool, invoke_id: &str) -> PendingPoolAttemptRecord {
    let trace = attempt_trace(invoke_id);
    let pending = begin_pool_upstream_request_attempt(
        pool,
        &trace,
        PoolAttemptStartScope {
            upstream_account_id: 101,
            upstream_route_key: "route-primary",
            ..PoolAttemptStartScope::default()
        },
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
        },
        "2026-07-01 10:00:00",
    )
    .await;
    assert!(
        pending.attempt_id.is_some(),
        "pending attempt should be inserted synchronously"
    );
    pending
}

fn terminal_write_for_coalescing(
    invoke_id: &str,
    dashboard_terminal_sequence: Option<u64>,
) -> BatchedTerminalInvocationWrite {
    let request_info = RequestCaptureInfo::default();
    BatchedTerminalInvocationWrite {
        record: build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
            invoke_id,
            "2026-07-01 10:00:00",
            ProxyCaptureTarget::Responses,
            &request_info,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
        )),
        capture_started: None,
        raw_capture: false,
        dashboard_terminal_sequence,
        terminal_projection_event_ids: Vec::new(),
        startup_backfill_tasks: Vec::new(),
    }
}

#[test]
fn terminal_batch_coalescing_preserves_the_persistence_ack_sequence() {
    let mut batch = PendingBatch::default();
    let accounting = PendingQueueAccounting::default();
    let mut first = terminal_write_for_coalescing("coalesced-terminal", Some(7));
    first.terminal_projection_event_ids.extend(0..64);
    first
        .startup_backfill_tasks
        .push(StartupBackfillTask::ProxyUsage);
    let first = SqliteBatchWrite::TerminalInvocation(first);
    accounting.enqueue(first.estimated_memory_bytes());
    batch.push_accounted(first, &accounting);
    let mut second = terminal_write_for_coalescing("coalesced-terminal", None);
    second.terminal_projection_event_ids.extend(64..128);
    second
        .startup_backfill_tasks
        .push(StartupBackfillTask::ReasoningEffort);
    let second = SqliteBatchWrite::TerminalInvocation(second);
    accounting.enqueue(second.estimated_memory_bytes());
    batch.push_accounted(second, &accounting);

    let terminal = batch
        .terminal_invocations
        .values()
        .next()
        .expect("coalesced terminal");
    assert_eq!(terminal.dashboard_terminal_sequence, Some(7));
    assert_eq!(
        terminal.terminal_projection_event_ids,
        (0..128).collect::<Vec<_>>()
    );
    assert_eq!(
        terminal.startup_backfill_tasks,
        vec![
            StartupBackfillTask::ReasoningEffort,
            StartupBackfillTask::ProxyUsage,
        ]
    );
    assert_eq!(batch.coalesced_rows, 1);
    assert_eq!(
        batch.estimated_memory_bytes(),
        terminal.estimated_memory_bytes()
    );
    assert_eq!(
        accounting.snapshot().pending_bytes,
        batch.estimated_memory_bytes()
    );
    assert_eq!(accounting.snapshot().pending_depth, batch.logical_rows());
}

#[tokio::test]
async fn p1_terminal_defers_backfill_wake_to_the_coordinated_p2_batch() {
    let pool = test_pool().await;
    let task = StartupBackfillTask::ReasoningEffort;
    let record = crate::tests::test_proxy_capture_record(
        "batch-terminal-backfill-wake",
        "2026-08-09 12:00:00",
    );
    let mut batch = PendingBatch::default();
    batch.push(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: vec![task],
            record,
        },
    ));
    let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
    let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
    let summary_delta_hub = Arc::new(std::sync::Mutex::new(None));
    let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
    let dashboard_reconcile_gate = Arc::new(Mutex::new(()));

    let deferred = flush_pending_batch_inner!(
        &pool,
        &batch,
        None,
        None,
        &terminal_runtime_store,
        &dashboard_activity_snapshot_cache,
        &summary_delta_hub,
        &terminal_projection_hub,
        &dashboard_reconcile_gate,
    )
    .await
    .expect("flush terminal P1 batch");

    let direct_wake_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
    )
    .bind(task.name())
    .fetch_one(&pool)
    .await
    .expect("count direct P1 backfill wakes");
    assert_eq!(
        direct_wake_count, 0,
        "P1 persistence must not directly write startup backfill progress"
    );
    assert_eq!(deferred.startup_backfill_wake_tasks, vec![task]);

    flush_pending_batch_inner!(
        &pool,
        &deferred,
        None,
        None,
        &terminal_runtime_store,
        &dashboard_activity_snapshot_cache,
        &summary_delta_hub,
        &terminal_projection_hub,
        &dashboard_reconcile_gate,
    )
    .await
    .expect("flush coordinated P2 backfill wake");

    let progress = load_startup_backfill_progress(&pool, task.name())
        .await
        .expect("load terminal-woken backfill progress");
    assert_eq!(progress.wake_generation, 1);
    assert!(progress.is_due(Utc::now()));
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
}

#[tokio::test]
async fn summary_delta_journal_acknowledges_only_committed_terminal() {
    let pool = test_pool().await;
    let mut batch = PendingBatch::default();
    let record = crate::tests::test_proxy_capture_record(
        "summary-delta-committed-ack",
        "2026-08-09 12:00:00",
    );
    batch.push(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: Some(1),
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
            record,
        },
    ));
    let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
    let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
    let summary_hub = Arc::new(SubscriptionHub::new());
    let summary_delta_hub = Arc::new(std::sync::Mutex::new(Some(summary_hub.clone())));
    let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
    let dashboard_reconcile_gate = Arc::new(Mutex::new(()));

    let pending_record = crate::proxy::api_invocation_from_runtime_record(
        &batch
            .terminal_invocations
            .values()
            .next()
            .expect("one queued terminal")
            .record,
    );
    let mut pending_delta = crate::persisted_dashboard_activity_terminal_delta(&pending_record);
    pending_delta.terminal_sequence = 1;
    pending_delta.persisted_row_id = None;
    summary_hub
        .register_summary_delta_pending(pending_delta)
        .await;
    assert!(
        summary_hub
            .summary_terminal_overlay_identities()
            .await
            .is_empty(),
        "a queued terminal must not be visible to Summary before SQLite commit"
    );
    flush_pending_batch_inner!(
        &pool,
        &batch,
        None,
        None,
        &terminal_runtime_store,
        &dashboard_activity_snapshot_cache,
        &summary_delta_hub,
        &terminal_projection_hub,
        &dashboard_reconcile_gate,
    )
    .await
    .expect("commit terminal and acknowledge its Summary delta");
    assert_eq!(
        summary_hub
            .summary_terminal_overlay_identities()
            .await
            .len(),
        1,
        "only the committed terminal belongs to the Summary Delta Journal"
    );
    let descriptor_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM summary_source_change_journal WHERE source_kind = 'terminal_batch'",
    )
    .fetch_one(&pool)
    .await
    .expect("count committed Summary source descriptors");
    assert_eq!(descriptor_count, 1);
}

#[tokio::test]
async fn summary_source_change_descriptor_failure_rolls_back_source() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.expect("begin source transaction");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) \
             VALUES ('descriptor-rollback', '2026-08-09 12:02:00', 'proxy', 'success', '')",
    )
    .execute(tx.as_mut())
    .await
    .expect("insert source row");
    let mut descriptor = SummarySourceChangeDescriptor::terminal_batch(
        1,
        vec![SummarySourceChangeEntry {
            row_id: 1,
            invoke_id: "descriptor-rollback".to_string(),
            occurred_at: "2026-08-09 12:02:00".to_string(),
            upstream_account_id: None,
            current_rank: None,
        }],
    )
    .expect("build descriptor");
    descriptor.version = SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION + 1;
    assert!(
        append_summary_source_change_descriptor_tx(tx.as_mut(), &descriptor)
            .await
            .is_err()
    );
    tx.rollback().await.expect("rollback source transaction");
    let source_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'descriptor-rollback'",
    )
    .fetch_one(&pool)
    .await
    .expect("count rolled back source row");
    assert_eq!(source_count, 0);
    let descriptor_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_source_change_journal")
            .fetch_one(&pool)
            .await
            .expect("count rolled back descriptors");
    assert_eq!(descriptor_count, 0);
}

#[tokio::test]
async fn terminal_journal_replay_acknowledges_committed_summary_recovery_delta() {
    let pool = test_pool().await;
    let mut batch = PendingBatch::default();
    let record = crate::tests::test_proxy_capture_record(
        "summary-delta-replayed-ack",
        "2026-08-09 12:01:00",
    );
    batch.push(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            capture_started: None,
            raw_capture: false,
            // Terminal journal replay uses the reserved post-restart marker rather than an
            // obsolete dashboard sequence from the prior process.
            dashboard_terminal_sequence: Some(0),
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
            record,
        },
    ));
    let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
    let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
    let summary_hub = Arc::new(SubscriptionHub::new());
    let summary_delta_hub = Arc::new(std::sync::Mutex::new(Some(summary_hub.clone())));
    let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
    let dashboard_reconcile_gate = Arc::new(Mutex::new(()));

    flush_pending_batch_inner!(
        &pool,
        &batch,
        None,
        None,
        &terminal_runtime_store,
        &dashboard_activity_snapshot_cache,
        &summary_delta_hub,
        &terminal_projection_hub,
        &dashboard_reconcile_gate,
    )
    .await
    .expect("commit terminal journal replay and acknowledge Summary recovery delta");

    assert_eq!(summary_hub.summary_delta_journal_counts().await, (1, 0));
    assert_eq!(
        summary_hub
            .summary_terminal_overlay_identities()
            .await
            .len(),
        1,
        "a committed replay must become an exact Summary recovery overlay"
    );
}

#[tokio::test]
async fn persisted_terminal_wakes_versioned_proxy_cost_backfill_after_p1_commit() {
    let pool = test_pool().await;
    let task = StartupBackfillTask::ProxyCost;
    let persisted_catalog = crate::pricing::load_pricing_catalog(&pool)
        .await
        .expect("load persisted pricing catalog for terminal wake");
    let persisted_task_name =
        startup_backfill_task_progress_key_for_catalog(task, &persisted_catalog);
    let mut runtime_catalog = persisted_catalog;
    runtime_catalog.version = "runtime-terminal-wake".to_string();
    let task_name = startup_backfill_task_progress_key_for_catalog(task, &runtime_catalog);
    assert_ne!(task_name, persisted_task_name);
    let pricing_catalog = Arc::new(RwLock::new(runtime_catalog));
    let record = crate::tests::test_proxy_capture_record(
        "batch-terminal-proxy-cost-backfill-wake",
        "2026-08-09 12:00:00",
    );

    SqliteBatchWriter::flush_for_test_with_pricing_catalog(
        &pool,
        Some(pricing_catalog),
        vec![SqliteBatchWrite::TerminalInvocation(
            BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: None,
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: vec![task],
                record,
            },
        )],
    )
    .await;

    let wake_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let progress = load_startup_backfill_progress(&pool, &task_name)
            .await
            .expect("load terminal-woken ProxyCost backfill progress");
        if progress.wake_generation > 0 {
            assert!(progress.is_due(Utc::now()));
            assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
            break;
        }
        assert!(
            Instant::now() < wake_deadline,
            "terminal P1 commit did not wake the versioned ProxyCost startup backfill task"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let bare_task_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
    )
    .bind(task.name())
    .fetch_one(&pool)
    .await
    .expect("count bare ProxyCost startup backfill progress records");
    assert_eq!(bare_task_count, 0);

    let persisted_task_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1",
    )
    .bind(&persisted_task_name)
    .fetch_one(&pool)
    .await
    .expect("count persisted-catalog ProxyCost progress records");
    assert_eq!(persisted_task_count, 0);
}

#[tokio::test]
async fn missing_proxy_cost_catalog_does_not_block_other_backfill_wakes() {
    let pool = test_pool().await;
    let unaffected_task = StartupBackfillTask::ReasoningEffort;

    let error = wake_startup_backfill_tasks(
        &pool,
        &[StartupBackfillTask::ProxyCost, unaffected_task],
        "test_missing_proxy_cost_catalog",
    )
    .await
    .expect_err("ProxyCost wake without a catalog should be reported");
    assert!(
        error
            .to_string()
            .contains("requires the runtime pricing catalog"),
        "unexpected missing catalog error: {error:#}"
    );

    let progress = load_startup_backfill_progress(&pool, unaffected_task.name())
        .await
        .expect("load unaffected task progress after missing ProxyCost catalog");
    assert_eq!(progress.wake_generation, 1);
    assert!(progress.is_due(Utc::now()));
}

#[tokio::test]
async fn p1_terminal_batch_keeps_summary_journal_unacknowledged_on_poison_record() {
    let pool = test_pool().await;
    sqlx::query(
        r#"
            CREATE TRIGGER reject_poison_terminal
            BEFORE INSERT ON codex_invocations
            WHEN NEW.invoke_id = 'poison-terminal'
            BEGIN
                SELECT RAISE(ABORT, 'poison terminal');
            END
            "#,
    )
    .execute(&pool)
    .await
    .expect("install poison trigger");

    let mut batch = PendingBatch::default();
    batch.push(SqliteBatchWrite::TerminalInvocation(
        terminal_write_for_coalescing("valid-terminal", Some(1)),
    ));
    batch.push(SqliteBatchWrite::TerminalInvocation(
        terminal_write_for_coalescing("poison-terminal", Some(2)),
    ));
    let runtime_store = Arc::new(std::sync::Mutex::new(None));
    let dashboard_cache = Arc::new(std::sync::Mutex::new(None));
    let summary_hub = Arc::new(SubscriptionHub::new());
    let summary_delta_hub = Arc::new(std::sync::Mutex::new(Some(summary_hub.clone())));
    let projection_hub = Arc::new(std::sync::Mutex::new(None));
    let reconcile_gate = Arc::new(Mutex::new(()));

    let error = flush_pending_batch_inner!(
        &pool,
        &batch,
        None,
        None,
        &runtime_store,
        &dashboard_cache,
        &summary_delta_hub,
        &projection_hub,
        &reconcile_gate,
    )
    .await
    .expect_err("poison record aborts the complete P1 transaction");
    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains("poison terminal")),
        "unexpected error chain: {error:#}"
    );

    let persisted = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id IN ('valid-terminal', 'poison-terminal')",
        )
        .fetch_one(&pool)
        .await
        .expect("count terminal rows");
    assert_eq!(persisted, 0, "P1 transaction must not commit a prefix");
    assert!(
        summary_hub
            .summary_terminal_overlay_identities()
            .await
            .is_empty(),
        "a failed transaction must not acknowledge a terminal into the Summary Delta Journal"
    );
}
