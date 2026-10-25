use super::*;

#[tokio::test]
async fn ownership_manual_materialization_start_preserves_paused_schedule() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    store
        .set_enabled("prompt_cache_materialization", false)
        .await
        .unwrap();
    let task_name = StartupBackfillTask::PromptCacheConversationsMaterialization.name();
    sqlx::query("INSERT INTO startup_backfill_progress(task_name,next_run_after,next_probe_at,suspension_reason,enabled) VALUES(?,'2099-01-01T00:00:00Z','2099-01-02T00:00:00Z','operator_disabled',0)")
        .bind(task_name).execute(&store.pool).await.unwrap();
    let before: (Option<String>, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT next_run_after,next_probe_at,suspension_reason,enabled FROM startup_backfill_progress WHERE task_name=?",
    ).bind(task_name).fetch_one(&store.pool).await.unwrap();
    crate::maintenance::with_maintenance_execution_options(
        crate::maintenance::MaintenanceExecutionOptions {
            manual: true,
            admitted: true,
            dry_run: false,
        },
        crate::maintenance::mark_startup_backfill_running(&store.pool, task_name, 0),
    )
    .await
    .unwrap();
    let after: (Option<String>, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT next_run_after,next_probe_at,suspension_reason,enabled FROM startup_backfill_progress WHERE task_name=?",
    ).bind(task_name).fetch_one(&store.pool).await.unwrap();
    assert_eq!(after, before);
    let status: String =
        sqlx::query_scalar("SELECT last_status FROM startup_backfill_progress WHERE task_name=?")
            .bind(task_name)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(status, "running");
}

#[tokio::test]
async fn ownership_settlement_serializes_deferred_finalization_from_timeline_writer() {
    let directory = make_temp_test_dir("maintenance-settlement-timeline-interleaving");
    let fixture_started_at = Utc::now() - chrono::Duration::minutes(1);
    let started_at = format_utc_iso_millis(fixture_started_at);
    let execution_started_at =
        format_utc_iso_millis(fixture_started_at + chrono::Duration::milliseconds(100));
    let finished_at =
        format_utc_iso_millis(fixture_started_at + chrono::Duration::milliseconds(900));
    let options = SqliteConnectOptions::new()
        .filename(directory.join("maintenance.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(2));
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await
        .unwrap();
    let store = crate::maintenance_store::MaintenanceStore::from_pool(pool);
    store.initialize_schema_for_test().await.unwrap();
    let run_id = store
        .begin_run("invocation_identity_cleanup", &started_at, "manual", None)
        .await
        .unwrap();
    store
        .start_timeline_session("settlement-interleaving", &started_at)
        .await
        .unwrap();
    store
        .write_timeline_batch(
            "settlement-interleaving",
            &[crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "settlement-execution".to_string(),
                task_key: "invocation_identity_cleanup".to_string(),
                title: "清理调用身份".to_string(),
                trigger_kind: "manual".to_string(),
                execution_class: Some("maintenance_legacy".to_string()),
                started_at: execution_started_at.clone(),
                managed_run_id: Some(run_id),
            }],
            0,
            &execution_started_at,
            false,
        )
        .await
        .unwrap();

    let hooks = store.install_settlement_interleaving_hooks();
    let finish_store = store.clone();
    let finish_task = tokio::spawn(async move {
        finish_store
            .finish_run_with_observation(
                run_id,
                "success",
                &finished_at,
                900,
                Some("已完成"),
                None,
                Some("completed"),
                Some("completed"),
                None,
            )
            .await
    });
    hooks.finish_after_select.notified().await;

    let timeline_store = store.clone();
    let timeline_task = tokio::spawn(async move {
        timeline_store
            .write_timeline_batch(
                "settlement-interleaving",
                &[crate::task_timeline::TimelineEvent::ExecutionFinished {
                    id: "settlement-execution".to_string(),
                    finished_at: finished_at.clone(),
                    duration_ms: 900,
                    status: "success".to_string(),
                }],
                0,
                &finished_at,
                false,
            )
            .await
    });
    hooks.timeline_before_admission.notified().await;

    // With the old deferred-transaction path, the timeline writer would acquire a write
    // transaction here while finalization was paused after SELECT. The bounded absence check
    // makes that interleaving observable without relying on a sleep or a retry.
    let timeline_started_while_finalization_was_paused = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        hooks.timeline_transaction_started.notified(),
    )
    .await
    .is_ok();
    if timeline_started_while_finalization_was_paused {
        hooks.continue_timeline.notify_one();
        hooks.continue_finish.notify_one();
        let finish_result = finish_task.await.expect("join finalization task");
        let timeline_result = timeline_task.await.expect("join timeline task");
        panic!(
            "timeline write entered before deferred finalization settled: finish={finish_result:?}, timeline={timeline_result:?}"
        );
    }

    hooks.continue_finish.notify_one();
    finish_task
        .await
        .expect("join finalization task")
        .expect("finalization should commit before timeline write admission");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        hooks.timeline_transaction_started.notified(),
    )
    .await
    .expect("timeline writer should enter after finalization commits");
    hooks.continue_timeline.notify_one();
    timeline_task
        .await
        .expect("join timeline task")
        .expect("timeline write should commit after finalization");

    let status: String = sqlx::query_scalar("SELECT status FROM managed_task_runs WHERE id=?")
        .bind(run_id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(status, "success");
    let timeline_status: String =
        sqlx::query_scalar("SELECT status FROM task_timeline_segments WHERE segment_id=?")
            .bind("settlement-execution")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(timeline_status, "success");
}

#[tokio::test]
async fn ownership_v2_persists_committed_as_processed_count() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let run_id = store
        .begin_run(
            "retention_archive",
            "2026-10-01T00:00:00.000Z",
            "manual",
            None,
        )
        .await
        .unwrap();
    store
        .finish_run_with_observation(
            run_id,
            "success",
            "2026-10-01T00:00:01.000Z",
            1,
            None,
            None,
            Some("complete"),
            Some("complete"),
            Some(&json!({
                "ownershipVersion": 2,
                "committed": 7,
                "processedCount": 7,
                "selectedScope": {"committed": 7}
            })),
        )
        .await
        .unwrap();
    let processed_count: Option<i64> =
        sqlx::query_scalar("SELECT processed_count FROM managed_task_runs WHERE id=?")
            .bind(run_id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(processed_count, Some(7));
}

#[tokio::test]
async fn ownership_v2_finalization_clears_catchup_with_current_run_generation() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let task_key = "pool_upstream_attempt_archive";
    sqlx::query(
        "UPDATE managed_tasks
         SET next_catchup_at='2000-01-01T00:00:00.000Z',catchup_reason='backlog_remaining'
         WHERE task_key=?",
    )
    .bind(task_key)
    .execute(&store.pool)
    .await
    .unwrap();
    let run_id = store
        .begin_run(task_key, "2026-10-01T00:00:00.000Z", "catchup", None)
        .await
        .unwrap();

    store
        .finish_run_with_observation(
            run_id,
            "success",
            "2026-10-01T00:00:01.000Z",
            1,
            None,
            None,
            Some("complete"),
            Some("complete"),
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": task_key,
                "committed": 0,
                "selectedScope": {"committed": 0, "hasMore": false},
                "waitReason": null,
                "dryRun": false
            })),
        )
        .await
        .unwrap();

    let state: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT runs.status,tasks.next_catchup_at,tasks.catchup_reason
         FROM managed_task_runs runs
         JOIN managed_tasks tasks ON tasks.task_key=runs.task_key
         WHERE runs.id=?",
    )
    .bind(run_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(state, ("success".to_string(), None, None));
}

#[tokio::test]
async fn ownership_v2_archive_finalization_updates_catchup_before_commit() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    sqlx::query(
        "UPDATE managed_tasks
         SET next_catchup_at='2000-01-01T00:00:00.000Z',catchup_reason='backlog_remaining'
         WHERE task_key='retention_archive'",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let run_id = store
        .begin_run(
            "retention_archive",
            "2026-10-01T00:00:00.000Z",
            "catchup",
            None,
        )
        .await
        .unwrap();

    store
        .finish_run_with_observation(
            run_id,
            "success",
            "2026-10-01T00:00:01.000Z",
            1,
            None,
            None,
            Some("complete"),
            Some("complete"),
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": "retention_archive",
                "globalBacklogTotal": 0,
                "selectedScope": {"committed": 0},
                "waitReason": null,
                "dryRun": false
            })),
        )
        .await
        .unwrap();

    let state: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT runs.status,tasks.next_catchup_at,tasks.catchup_reason
         FROM managed_task_runs runs
         JOIN managed_tasks tasks ON tasks.task_key=runs.task_key
         WHERE runs.id=?",
    )
    .bind(run_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(state, ("success".to_string(), None, None));
}

#[tokio::test]
async fn failed_retention_archive_run_schedules_bounded_retry() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let run_id = store
        .begin_run(
            "retention_archive",
            "2026-10-01T00:00:00.000Z",
            "schedule",
            None,
        )
        .await
        .unwrap();
    store
        .finish_run_with_observation(
            run_id,
            "failed",
            "2026-10-01T00:00:01.000Z",
            1,
            Some("retention_archive 执行失败"),
            Some("owner failed after the invocation snapshot"),
            Some("failed"),
            None,
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": "retention_archive",
                "result": "failed",
                "globalBacklogTotal": 0,
                "committed": 0,
                "selectedScope": {"committed": 0},
                "waitReason": null
            })),
        )
        .await
        .unwrap();
    let scheduled: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT next_catchup_at,catchup_reason FROM managed_tasks WHERE task_key='retention_archive'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert!(scheduled.0.is_some());
    assert_eq!(scheduled.1.as_deref(), Some("owner_failed"));
}

#[tokio::test]
async fn failed_retention_archive_finalization_commits_retry_with_terminal_run() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let run_id = store
        .begin_run(
            "retention_archive",
            "2026-10-01T00:00:00.000Z",
            "schedule",
            None,
        )
        .await
        .unwrap();
    store
        .finish_run_with_observation_and_retry(
            run_id,
            "failed",
            "2026-10-01T00:00:01.000Z",
            1,
            Some("retention_archive 执行失败"),
            Some("owner failed after the invocation snapshot"),
            Some("failed"),
            None,
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": "retention_archive",
                "result": "failed",
                "globalBacklogTotal": 0,
                "committed": 0,
                "selectedScope": {"committed": 0},
                "waitReason": "owner_failed"
            })),
            300,
            "owner_failed",
        )
        .await
        .unwrap();

    let state: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT runs.status,tasks.next_catchup_at,tasks.catchup_reason,retry.reason
         FROM managed_task_runs runs
         JOIN managed_tasks tasks ON tasks.task_key=runs.task_key
         LEFT JOIN maintenance_owner_retry_state retry ON retry.task_key=runs.task_key
         WHERE runs.id=?",
    )
    .bind(run_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(state.0, "failed");
    assert!(state.1.is_some());
    assert_eq!(state.2.as_deref(), Some("owner_failed"));
    assert_eq!(state.3.as_deref(), Some("owner_failed"));
}

#[tokio::test]
async fn stale_terminal_replay_cannot_overwrite_recovered_retry_state() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let run_id = store
        .begin_run(
            "retention_archive",
            "2026-10-01T00:00:00.000Z",
            "schedule",
            None,
        )
        .await
        .unwrap();
    let recovered_details = json!({
        "ownershipVersion": 2,
        "ownerScope": "retention_archive",
        "result": "failed",
        "selectedScope": {"committed": 0, "hasMore": true},
        "waitReason": "owner_failed"
    });
    store
        .finish_run_with_observation_and_retry(
            run_id,
            "failed",
            "2026-10-01T00:00:01.000Z",
            1,
            Some("recovered failure"),
            Some("startup recovery"),
            Some("failed"),
            None,
            Some(&recovered_details),
            300,
            "owner_failed",
        )
        .await
        .unwrap();

    store
        .finish_run_with_observation(
            run_id,
            "success",
            "2026-10-01T00:00:02.000Z",
            2,
            Some("stale replay success"),
            None,
            Some("complete"),
            Some("complete"),
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": "retention_archive",
                "selectedScope": {"committed": 9, "hasMore": false},
                "dryRun": false
            })),
        )
        .await
        .unwrap();

    let state: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT runs.status,runs.summary,tasks.catchup_reason,retry.reason
         FROM managed_task_runs runs
         JOIN managed_tasks tasks ON tasks.task_key=runs.task_key
         LEFT JOIN maintenance_owner_retry_state retry ON retry.task_key=runs.task_key
         WHERE runs.id=?",
    )
    .bind(run_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(state.0, "failed");
    assert_eq!(state.1.as_deref(), Some("recovered failure"));
    assert_eq!(state.2.as_deref(), Some("owner_failed"));
    assert_eq!(state.3.as_deref(), Some("owner_failed"));
}

#[tokio::test]
async fn owner_progress_schedule_sync_preserves_stale_workload_freshness() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let task_key = "pool_upstream_attempt_archive";
    sqlx::query(
        "INSERT INTO managed_task_progress(task_key,total,completed,updated_at,freshness)
         VALUES(?,9,3,'2020-01-01T00:00:00.000Z','stale')",
    )
    .bind(task_key)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE managed_tasks
         SET next_catchup_at='2026-10-01T00:00:00.000Z',catchup_reason='owner_failed'
         WHERE task_key=?",
    )
    .bind(task_key)
    .execute(&store.pool)
    .await
    .unwrap();

    store
        .sync_retention_owner_progress_schedule(task_key)
        .await
        .unwrap();

    let progress: (Option<i64>, Option<i64>, String, String, Option<String>) = sqlx::query_as(
        "SELECT total,completed,freshness,updated_at,catchup_state
         FROM managed_task_progress WHERE task_key=?",
    )
    .bind(task_key)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(progress.0, Some(9));
    assert_eq!(progress.1, Some(3));
    assert_eq!(progress.2, "stale");
    assert_eq!(progress.3, "2020-01-01T00:00:00.000Z");
    assert_eq!(progress.4.as_deref(), Some("scheduled"));
}

#[tokio::test]
async fn successful_owner_run_clears_prior_retry_state() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let task_key = "retention_archive";
    let failed_run = store
        .begin_run(task_key, "2026-10-01T00:00:00.000Z", "schedule", None)
        .await
        .unwrap();
    store
        .finish_run_with_observation_and_retry(
            failed_run,
            "failed",
            "2026-10-01T00:00:01.000Z",
            1,
            Some("owner failed"),
            None,
            Some("failed"),
            None,
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": task_key,
                "result": "failed",
                "selectedScope": {"committed": 0, "hasMore": true},
                "waitReason": "owner_failed"
            })),
            300,
            "owner_failed",
        )
        .await
        .unwrap();

    let successful_run = store
        .begin_run(task_key, "2026-10-01T00:01:00.000Z", "manual", None)
        .await
        .unwrap();
    store
        .finish_run_with_observation(
            successful_run,
            "success",
            "2026-10-01T00:01:01.000Z",
            1,
            None,
            None,
            Some("complete"),
            Some("complete"),
            Some(&json!({
                "ownershipVersion": 2,
                "ownerScope": task_key,
                "selectedScope": {"committed": 0, "hasMore": false},
                "waitReason": null,
                "dryRun": false
            })),
        )
        .await
        .unwrap();

    let retry_reason: Option<String> =
        sqlx::query_scalar("SELECT reason FROM maintenance_owner_retry_state WHERE task_key=?")
            .bind(task_key)
            .fetch_optional(&store.pool)
            .await
            .unwrap();
    assert_eq!(retry_reason, None);
}

#[tokio::test]
async fn ownership_task_history_cleanup_is_bounded_and_dry_run_is_read_only() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    for _ in 0..70 {
        sqlx::query(
            "INSERT INTO managed_task_runs(task_key,trigger_kind,started_at,status,summary) VALUES('retention_archive','test',?,'success','old run')",
        )
        .bind("2020-01-01T00:00:00.000Z")
        .execute(&store.pool)
        .await
        .unwrap();
    }
    let preview = store.cleanup_expired_history_bounded(true).await.unwrap();
    assert_eq!(preview.selected, 64);
    assert_eq!(preview.committed, 0);
    assert!(preview.has_more);
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM managed_task_runs")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(before, 70);

    let result = store.cleanup_expired_history_bounded(false).await.unwrap();
    assert_eq!(result.selected, 64);
    assert_eq!(result.committed, 64);
    assert!(result.has_more);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM managed_task_runs")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(after, 6);
}

#[tokio::test]
async fn retention_owner_preview_and_admission_defer_use_real_owner_classification() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("owner-preview-classification").await;
    let account_id = 8_901_i64;
    let timestamp = format_utc_iso_millis(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, status, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
    )
    .bind(account_id)
    .bind("api_key")
    .bind("test")
    .bind("Owner classification test")
    .bind("active")
    .bind(true)
    .bind(&timestamp)
    .execute(&pool)
    .await
    .expect("seed model route account");
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, last_seen_at) VALUES (?1, ?2, ?3)",
    )
    .bind(account_id)
    .bind("gpt-owner-classification")
    .bind("2000-01-01T00:00:00.000Z")
    .execute(&pool)
    .await
    .expect("seed expired model route");

    let preview = crate::maintenance::run_retention_owner_task(
        &pool,
        &config,
        "model_route_expiry",
        true,
        &CancellationToken::new(),
    )
    .await
    .expect("run model route dry-run owner");
    assert_eq!(preview.discovered, 1);
    assert_eq!(preview.selected, 1);
    assert_eq!(preview.committed, 0);
    assert_eq!(preview.completion, "complete");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE account_id=?1",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("count preview routes");
    assert_eq!(remaining, 1);

    let deferred = crate::maintenance::retention_test_with_work_budget(
        Duration::ZERO,
        crate::maintenance::run_retention_owner_task(
            &pool,
            &config,
            "model_route_expiry",
            false,
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("run model route deferred owner");
    assert_eq!(deferred.discovered, 1);
    assert_eq!(deferred.selected, 0);
    assert_eq!(deferred.committed, 0);
    assert_eq!(deferred.completion, "deferred");
    assert_eq!(
        deferred.wait_reason.as_deref(),
        Some("retention_work_budget")
    );

    let remaining_after_defer: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE account_id=?1",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("count deferred routes");
    assert_eq!(remaining_after_defer, 1);
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ownership_initialization_preserves_controls_and_recovers_interrupted_schedule_write() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    for key in [
        "retention_archive",
        "pool_upstream_attempt_archive",
        "forward_proxy_attempt_archive",
        "invocation_detail_prune",
        "quota_snapshot_compaction",
        "model_route_expiry",
        "task_run_history_cleanup",
        "prompt_cache_materialization",
        "invocation_identity_cleanup",
        "raw_orphan_sweep",
    ] {
        assert!(store.detail(key).await.unwrap().unwrap().task.enabled);
    }
    store
        .update_control("retention_archive", Some(false), Some(Some(600)), None)
        .await
        .unwrap();
    store
        .update_control(
            "raw_orphan_sweep",
            Some(false),
            Some(None),
            Some(Some("*/7 * * * *")),
        )
        .await
        .unwrap();
    sqlx::query("DELETE FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'")
        .execute(&store.pool)
        .await
        .unwrap();
    // An actual interrupted transaction must not publish either partial scheduling or its marker.
    let mut tx = store.pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE managed_tasks SET interval_secs=42 WHERE task_key='invocation_identity_cleanup'",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert!(store.apply_initial_task_defaults().await.unwrap());
    assert!(!store.apply_initial_task_defaults().await.unwrap());
    let archive = store
        .detail("retention_archive")
        .await
        .unwrap()
        .unwrap()
        .task;
    assert!(!archive.enabled);
    assert_eq!(archive.interval_secs, Some(600));
    let raw = store
        .detail("raw_orphan_sweep")
        .await
        .unwrap()
        .unwrap()
        .task;
    assert!(!raw.enabled);
    assert_eq!(raw.cron_expr.as_deref(), Some("*/7 * * * *"));
    assert_eq!(
        store
            .detail("invocation_identity_cleanup")
            .await
            .unwrap()
            .unwrap()
            .task
            .interval_secs,
        Some(300)
    );
}

#[tokio::test]
async fn ownership_pause_preserves_accepted_manual_requests_and_blocks_new_automatic_requests() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    for key in [
        "retention_archive",
        "pool_upstream_attempt_archive",
        "forward_proxy_attempt_archive",
        "invocation_detail_prune",
        "quota_snapshot_compaction",
        "model_route_expiry",
        "task_run_history_cleanup",
        "prompt_cache_materialization",
        "invocation_identity_cleanup",
        "raw_orphan_sweep",
    ] {
        let id = store.request_run_with_mode(key, true).await.unwrap();
        store.set_enabled(key, false).await.unwrap();
        let claim = store.claim_requested_run().await.unwrap().unwrap();
        assert_eq!(claim.0, id);
        assert_eq!(claim.1, key);
        assert_eq!(claim.3, "manual");
        let ownership_version = if matches!(
            key,
            "retention_archive"
                | "pool_upstream_attempt_archive"
                | "forward_proxy_attempt_archive"
                | "invocation_detail_prune"
                | "quota_snapshot_compaction"
                | "model_route_expiry"
                | "task_run_history_cleanup"
        ) {
            2
        } else {
            1
        };
        store
            .finish_run_with_observation(
                id,
                "success",
                &format_utc_iso_millis(Utc::now()),
                1,
                None,
                None,
                Some("partial"),
                None,
                Some(&json!({"total":100,"completed":1,"ownershipVersion":ownership_version})),
            )
            .await
            .unwrap();
        let task = store.detail(key).await.unwrap().unwrap().task;
        assert!(!task.enabled);
        assert!(task.next_catchup_at.is_none());
        let next = store.request_run(key).await.unwrap();
        store
            .finish_run(
                next,
                "success",
                &format_utc_iso_millis(Utc::now()),
                1,
                None,
                None,
            )
            .await
            .unwrap();
    }
    assert_eq!(store.enqueue_due_runs().await.unwrap(), 0);
}

#[tokio::test]
async fn ownership_resume_starts_from_now_and_safety_retry_survives_inspection_override() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    sqlx::query("UPDATE managed_tasks SET enabled=0,next_trigger_at='2000-01-01T00:00:00Z' WHERE task_key='invocation_identity_cleanup'").execute(&store.pool).await.unwrap();
    let now = format_utc_iso_millis(Utc::now());
    store
        .set_enabled("invocation_identity_cleanup", true)
        .await
        .unwrap();
    assert!(
        store
            .detail("invocation_identity_cleanup")
            .await
            .unwrap()
            .unwrap()
            .task
            .next_trigger_at
            .unwrap()
            > now
    );
    for seconds in [300, 600, 1200, 2400, 3600, 3600] {
        assert_eq!(
            store
                .record_owned_retry("invocation_identity_cleanup", false)
                .await
                .unwrap(),
            seconds
        );
    }
    store
        .set_schedule("invocation_identity_cleanup", Some(60), None)
        .await
        .unwrap();
    sqlx::query("UPDATE managed_tasks SET next_trigger_at='2000-01-01T00:00:00Z' WHERE task_key='invocation_identity_cleanup'").execute(&store.pool).await.unwrap();
    // Archive's startup occurrence is independent; isolate the owner's admission assertion.
    sqlx::query("UPDATE managed_tasks SET enabled=0 WHERE task_key!='invocation_identity_cleanup'")
        .execute(&store.pool)
        .await
        .unwrap();
    assert_eq!(store.enqueue_due_runs().await.unwrap(), 0);
}

#[tokio::test]
async fn ownership_identity_preview_is_bounded_and_does_not_commit_either_cursor() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    ensure_schema(&pool).await.unwrap();
    for n in 0..40 {
        let encoded = encode_prompt_cache_conversation_sequence(n).unwrap();
        let prefix = format!("QZ{encoded}");
        sqlx::query("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES(?,?,'2020-01-01','2020-01-01')")
            .bind(format!("ownership-preview-{n:04}")).bind(prefix).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO hourly_invoke_prefixes(utc_hour,prefix) VALUES(?,?)")
            .bind(i64::from(n))
            .bind(format!("HX{encoded}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let before: Vec<(String, Option<String>, i64)> = sqlx::query_as("SELECT scope,cursor_key,epoch FROM prompt_cache_conversation_orphan_cleanup_state ORDER BY scope").fetch_all(&pool).await.unwrap();
    let preview = cleanup_invocation_identities(&pool, true, &cache)
        .await
        .unwrap();
    assert_eq!(preview.conversations_checked, 32);
    assert_eq!(preview.hours_checked, Some(32));
    assert_eq!(preview.hours_released, Some(32));
    assert!(preview.has_more);
    let after: Vec<(String, Option<String>, i64)> = sqlx::query_as("SELECT scope,cursor_key,epoch FROM prompt_cache_conversation_orphan_cleanup_state ORDER BY scope").fetch_all(&pool).await.unwrap();
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        40
    );
    let actual = cleanup_invocation_identities(&pool, false, &cache)
        .await
        .unwrap();
    assert_eq!(actual.hours_released, Some(32));
    let rest = cleanup_invocation_identities(&pool, false, &cache)
        .await
        .unwrap();
    assert_eq!(rest.hours_released, Some(8));
    assert!(!rest.has_more);
}

#[tokio::test]
async fn ownership_marker_failure_rolls_back_schedules_and_allows_forward_reentry() {
    let store = crate::maintenance_store::ownership_test_store().await;
    sqlx::query("CREATE TRIGGER interrupt_ownership_marker BEFORE INSERT ON maintenance_metadata WHEN NEW.key='retention_maintenance_ownership_v1' BEGIN SELECT RAISE(ABORT,'injected initialization interruption'); END")
        .execute(&store.pool).await.unwrap();
    assert!(store.apply_initial_task_defaults().await.is_err());
    let marker: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(marker, 0);
    let interval: Option<i64> = sqlx::query_scalar(
        "SELECT interval_secs FROM managed_tasks WHERE task_key='invocation_identity_cleanup'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(interval, None);
    sqlx::query("DROP TRIGGER interrupt_ownership_marker")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.apply_initial_task_defaults().await.unwrap());
    assert!(!store.apply_initial_task_defaults().await.unwrap());
    assert_eq!(
        store
            .detail("invocation_identity_cleanup")
            .await
            .unwrap()
            .unwrap()
            .task
            .interval_secs,
        Some(300)
    );
}
