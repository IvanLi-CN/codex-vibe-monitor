use super::*;

async fn flush_obligation_batch(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
) -> PendingBatch {
    flush_pending_batch_inner(
        pool,
        batch,
        pricing_catalog,
        None,
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(std::sync::Mutex::new(None)),
        &Arc::new(Mutex::new(())),
        None,
    )
    .await
    .expect("flush business fact independently of its wake obligation")
}

#[tokio::test]
async fn rejected_backfill_notification_preserves_committed_terminal_and_retry_obligation() {
    let pool = Pool::<Sqlite>::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect isolated obligation fixture");
    ensure_schema(&pool).await.expect("prepare current schema");
    let task = StartupBackfillTask::ProxyCost;
    let record = crate::tests::test_proxy_capture_record(
        "terminal-retained-backfill-obligation",
        "2026-08-09 12:00:00",
    );
    let mut terminal = PendingBatch::default();
    terminal.push(SqliteBatchWrite::TerminalInvocation(
        BatchedTerminalInvocationWrite {
            enqueued_at: None,
            attempts: Vec::new(),
            diagnostic: None,
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: vec![task],
            record,
        },
    ));
    let committed = flush_obligation_batch(&pool, &terminal, None).await;
    assert_eq!(committed.startup_backfill_wake_tasks, vec![task]);
    let rejected = flush_obligation_batch(&pool, &committed, None).await;
    assert!(rejected.terminal_invocations.is_empty());
    assert_eq!(rejected.startup_backfill_wake_tasks, vec![task]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id='terminal-retained-backfill-obligation' AND status='success'")
            .fetch_one(&pool).await.expect("business terminal is already durable"),
        1,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
            .fetch_one(&pool)
            .await
            .expect("rejected notification made no confirmation write"),
        0,
    );

    let catalog = crate::pricing::load_pricing_catalog(&pool)
        .await
        .expect("restore the required runtime catalog");
    let progress_key = startup_backfill_task_progress_key_for_catalog(task, &catalog);
    let catalog = Arc::new(RwLock::new(catalog));
    let accepted = flush_obligation_batch(&pool, &rejected, Some(&catalog)).await;
    assert!(
        accepted.is_empty(),
        "accepted wake belongs to the background owner"
    );
    crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&pool).await;
    let progress = load_startup_backfill_progress(&pool, &progress_key)
        .await
        .expect("background owner confirms the retained versioned obligation");
    assert_eq!(progress.wake_generation, 1);
    assert!(progress.is_due(Utc::now()));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id='terminal-retained-backfill-obligation'")
            .fetch_one(&pool).await.expect("retry did not duplicate the terminal"),
        1,
    );
    pool.close().await;
}
