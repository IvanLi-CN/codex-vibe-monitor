use super::*;
use crate::maintenance_store::MaintenanceStore;
use serde_json::json;

async fn prompt_cache_materialization_maintenance_store(enabled: bool) -> MaintenanceStore {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect independent maintenance pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance schema and task registry");
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    sqlx::query("UPDATE managed_tasks SET enabled=? WHERE task_key=?")
        .bind(enabled)
        .bind(task_key)
        .execute(&store.pool)
        .await
        .expect("set initial maintenance control");
    store
        .initialize_prompt_cache_materialization_control(task_key, task.name())
        .await
        .expect("initialize in-memory maintenance control");
    store
}

#[tokio::test]
async fn prompt_cache_materialization_status_reports_progress_history_and_control() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    for index in 0..4 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, '2026-09-01 00:00:00', ?2, 'success', 7, 0.07, ?3, '{}')
            "#,
        )
        .bind(format!("status-invocation-{index}"))
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": format!("status-key-{index}")}).to_string())
        .execute(&pool)
        .await
        .expect("insert status fixture invocation");
    }
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;

    let outcome = run_prompt_cache_conversations_materialization(&pool, 400, None)
        .await
        .expect("materialize status fixture");
    record_prompt_cache_conversation_materialization_run(
        &pool,
        "2026-09-29T04:16:45.000Z",
        120,
        &outcome,
        "success",
        None,
    )
    .await
    .expect("record materialization history");

    let status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("load materialization status");
    assert!(status.enabled);
    assert_eq!(status.total_keys, Some(4));
    assert_eq!(status.completed_keys, 4);
    assert!(outcome.complete);
    assert_eq!(status.progress_percent, Some(100.0));
    assert_eq!(status.estimated_remaining_ms, Some(0));
    assert_eq!(status.queue_pending, 0);
    assert_eq!(status.recent_runs.len(), 1);

    let disabled = set_prompt_cache_materialization_enabled_with_store(
        &pool,
        &maintenance,
        StartupBackfillTask::PromptCacheConversationsMaterialization,
        false,
    )
    .await
    .expect("disable prompt-cache materialization");
    assert!(!disabled.enabled);
    let disabled_status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("load disabled materialization status");
    assert!(!disabled_status.enabled);
    assert_eq!(disabled_status.last_status, "disabled");

    let enabled = set_prompt_cache_materialization_enabled_with_store(
        &pool,
        &maintenance,
        StartupBackfillTask::PromptCacheConversationsMaterialization,
        true,
    )
    .await
    .expect("enable prompt-cache materialization");
    assert!(enabled.enabled);
}

#[tokio::test]
async fn prompt_cache_materialization_uses_maintenance_control_when_legacy_business_flag_disagrees()
{
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory business sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('dual-control-invocation', '2026-09-01 00:00:00', ?1, 'success', 7, 0.07, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "dual-control-key"}).to_string())
    .execute(&pool)
    .await
    .expect("insert dual-control fixture invocation");
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    sqlx::query(
        "INSERT INTO startup_backfill_progress (task_name,cursor_id,zero_update_streak,last_scanned,last_updated,last_status,wake_generation,enabled) \
         VALUES (?,0,0,0,0,'idle',0,0) \
         ON CONFLICT(task_name) DO UPDATE SET enabled=0",
    )
    .bind(task.name())
    .execute(&pool)
    .await
    .expect("make legacy business flag disagree with enabled maintenance task");

    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let should_yield = || false;
    let enabled_snapshot = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("enabled maintenance control");
    let enabled_run = {
        let mut completed = None;
        for _ in 0..8 {
            let run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
                &pool,
                400,
                None,
                &should_yield,
                &maintenance.prompt_cache_materialization_control,
                enabled_snapshot.generation,
            )
            .await
            .expect("run materialization using enabled maintenance control");
            if run.complete {
                completed = Some(run);
                break;
            }
            assert_ne!(run.defer_reason, Some("operator_disabled"));
        }
        completed.expect("enabled materialization should finish its durable phases")
    };
    assert!(enabled_run.complete);
    let legacy_enabled: bool =
        sqlx::query_scalar("SELECT enabled FROM startup_backfill_progress WHERE task_name=?")
            .bind(task.name())
            .fetch_one(&pool)
            .await
            .expect("read unchanged legacy business flag");
    assert!(!legacy_enabled);
    let enabled_status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("read status from maintenance control");
    assert!(enabled_status.enabled);

    sqlx::query("UPDATE startup_backfill_progress SET enabled=1 WHERE task_name=?")
        .bind(task.name())
        .execute(&pool)
        .await
        .expect("make legacy business flag disagree with disabled maintenance task");
    set_prompt_cache_materialization_enabled_with_store(&pool, &maintenance, task, false)
        .await
        .expect("disable through the maintenance control");
    let disabled_snapshot = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("disabled maintenance control");
    let disabled_run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &should_yield,
        &maintenance.prompt_cache_materialization_control,
        disabled_snapshot.generation,
    )
    .await
    .expect("run materialization using disabled maintenance control");
    assert_eq!(disabled_run.defer_reason, Some("operator_disabled"));
    let legacy_enabled: bool =
        sqlx::query_scalar("SELECT enabled FROM startup_backfill_progress WHERE task_name=?")
            .bind(task.name())
            .fetch_one(&pool)
            .await
            .expect("read unchanged legacy business flag after disable");
    assert!(legacy_enabled);
    let disabled_status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("read disabled status from maintenance control");
    assert!(!disabled_status.enabled);
}

#[tokio::test]
async fn prompt_cache_materialization_large_key_is_paged_once_across_phases() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory business sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool).await.expect("install schema");
    let mut transaction = pool.begin().await.expect("begin fixture");
    for index in 0..1024 {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,payload,raw_response) \
             VALUES (?1,'2026-09-01T00:00:00Z',?2,'success',1,?3,'{}')",
        )
        .bind(format!("single-scan-{index:04}"))
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": "single-scan-key"}).to_string())
        .execute(&mut *transaction)
        .await
        .expect("insert invocation");
    }
    transaction.commit().await.expect("commit fixture");
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let control = &maintenance.prompt_cache_materialization_control;
    let generation = control.snapshot().expect("initialized control").generation;
    let mut pending_pages = 0;
    let mut complete = false;
    for _ in 0..20 {
        let run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
            &pool,
            2000,
            None,
            &|| false,
            control,
            generation,
        )
        .await
        .expect("run bounded materialization");
        if run.defer_reason == Some("stats_page_pending") {
            pending_pages += 1;
        }
        if run.complete {
            complete = true;
            break;
        }
    }
    assert!(complete, "materialization must converge");
    assert_eq!(pending_pages, 4, "each 256-row page must be scanned once");
    let count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations WHERE prompt_cache_key='single-scan-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("read complete aggregate");
    assert_eq!(count, 1024);
    assert!(
        prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check complete markers and queue")
    );
}

#[tokio::test]
async fn prompt_cache_materialization_pages_resume_across_generation_change_and_control_restart() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory business sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache materialization schema");
    let mut transaction = pool.begin().await.expect("begin invocation fixture");
    for index in 0..1024 {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) \
             VALUES (?1,?2,?3,'success',1,0.01,?4,'{}')",
        )
        .bind(format!("paged-invocation-{index:04}"))
        .bind(format!("2026-09-01T00:{:02}:{:02}.000Z", index / 60, index % 60))
        .bind(SOURCE_PROXY)
        .bind(json!({"promptCacheKey": "paged-materialization-key"}).to_string())
        .execute(&mut *transaction)
        .await
        .expect("insert paged materialization invocation");
    }
    transaction
        .commit()
        .await
        .expect("commit invocation fixture");

    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let snapshot = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized materialization control");
    let should_yield = || false;
    let first_page = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &should_yield,
        &maintenance.prompt_cache_materialization_control,
        snapshot.generation,
    )
    .await
    .expect("run first statistics page");
    assert_eq!(first_page.defer_reason, Some("stats_page_pending"));
    let (outer_cursor, completed_keys): (Option<String>, i64) = sqlx::query_as(
        "SELECT cursor_key,completed_keys FROM prompt_cache_conversation_migration_progress \
         WHERE migration_name='prompt_cache_conversations_materialization_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load outer materialization cursor");
    assert_eq!(outer_cursor, None);
    // Identity coverage is committed separately; incomplete statistics still leave
    // the statistics cursor and published aggregate untouched.
    assert_eq!(completed_keys, 1);
    let staged_cursor: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM prompt_cache_conversation_stats_refresh_staging \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load committed statistics staging cursor");
    assert_eq!(staged_cursor, 256);
    let partial_request_count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load pre-completion aggregate");
    assert_eq!(partial_request_count, 0);
    assert!(
        !prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check incomplete materialization")
    );

    sqlx::query(
        "UPDATE prompt_cache_conversation_stats_refresh_queue SET generation=generation+1 \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .execute(&pool)
    .await
    .expect("advance source generation during paged refresh");
    let generation_change =
        run_prompt_cache_conversations_materialization_with_pressure_and_control(
            &pool,
            400,
            None,
            &should_yield,
            &maintenance.prompt_cache_materialization_control,
            snapshot.generation,
        )
        .await
        .expect("observe changed statistics source generation");
    assert_eq!(
        generation_change.defer_reason,
        Some("stats_generation_changed")
    );
    let reset_cursor: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM prompt_cache_conversation_stats_refresh_staging \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load reset statistics staging cursor");
    assert_eq!(reset_cursor, 0);

    let restarted = MaintenanceStore::from_pool(maintenance.pool.clone());
    restarted
        .initialize_prompt_cache_materialization_control(
            "startup_backfill.prompt_cache_conversations_materialization",
            task.name(),
        )
        .await
        .expect("restore control from persisted maintenance state");
    let restarted_snapshot = restarted
        .prompt_cache_materialization_control
        .snapshot()
        .expect("restored materialization control");
    let mut complete = false;
    for _ in 0..16 {
        let run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
            &pool,
            400,
            None,
            &should_yield,
            &restarted.prompt_cache_materialization_control,
            restarted_snapshot.generation,
        )
        .await
        .expect("continue statistics materialization after restart");
        if run.complete {
            complete = true;
            break;
        }
        assert_ne!(run.defer_reason, Some("operator_disabled"));
    }
    assert!(complete, "all paged statistics should eventually complete");
    let final_request_count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load final aggregate");
    assert_eq!(final_request_count, 1024);
    let queue_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue \
         WHERE prompt_cache_key='paged-materialization-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("check drained statistics queue");
    assert_eq!(queue_count, 0);
    assert!(
        prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check complete materialization")
    );
}

#[tokio::test]
async fn prompt_cache_materialization_status_does_not_report_complete_with_pending_refresh() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;

    sqlx::query(
        "INSERT INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key) \
         VALUES ('pending-after-complete')",
    )
    .execute(&pool)
    .await
    .expect("enqueue pending refresh");

    let status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("load incomplete materialization status");
    assert_eq!(status.queue_pending, 1);
    assert_ne!(status.progress_percent, Some(100.0));
    assert_ne!(status.estimated_remaining_ms, Some(0));
}

#[tokio::test]
async fn prompt_cache_materialization_repairs_complete_progress_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache materialization schema");
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    sqlx::query(
        "INSERT INTO prompt_cache_conversations (conversation_id, prompt_cache_key) \
         VALUES ('ABCDEF', 'legacy-complete-key')",
    )
    .execute(&pool)
    .await
    .expect("insert legacy complete conversation");
    sqlx::query(
        "UPDATE prompt_cache_conversation_migration_progress \
         SET phase = 'complete', total_keys = NULL, completed_keys = 0 \
         WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .execute(&pool)
    .await
    .expect("reset legacy complete progress counters");

    ensure_schema(&pool)
        .await
        .expect("repair legacy complete progress counters");
    let status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &maintenance.prompt_cache_materialization_control,
    )
    .await
    .expect("load repaired materialization status");
    assert_eq!(status.total_keys, Some(1));
    assert_eq!(status.completed_keys, 1);
    assert_eq!(status.progress_percent, Some(99.0));
    assert_eq!(status.estimated_remaining_ms, None);
}

#[tokio::test]
async fn prompt_cache_materialization_honors_operator_disable_before_a_batch() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    let maintenance = prompt_cache_materialization_maintenance_store(false).await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('operator-disable-invocation', '2026-09-01 00:00:00', ?1, 'success', 7, 0.07, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "operator-disable-key"}).to_string())
    .execute(&pool)
    .await
    .expect("insert operator-disable fixture invocation");
    let should_yield = || false;
    let control = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized maintenance control");
    let outcome = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &should_yield,
        &maintenance.prompt_cache_materialization_control,
        control.generation,
    )
    .await
    .expect("run disabled prompt-cache materialization");
    assert!(outcome.deferred);
    assert_eq!(outcome.defer_reason, Some("operator_disabled"));
    assert_eq!(outcome.batch_count, 0);

    let identity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM prompt_cache_conversations")
        .fetch_one(&pool)
        .await
        .expect("count identities after operator disable");
    assert_eq!(identity_count, 0);
}

#[tokio::test]
async fn prompt_cache_materialization_honors_operator_disable_in_queue_drain() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    let maintenance = prompt_cache_materialization_maintenance_store(false).await;
    sqlx::query(
        "UPDATE prompt_cache_conversation_migration_progress \
         SET phase = 'queue_drain' \
         WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .execute(&pool)
    .await
    .expect("set queue-drain phase");
    let should_yield = || false;
    let control = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized maintenance control");
    let outcome = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &should_yield,
        &maintenance.prompt_cache_materialization_control,
        control.generation,
    )
    .await
    .expect("run disabled queue-drain materialization");
    assert!(outcome.deferred);
    assert_eq!(outcome.defer_reason, Some("operator_disabled"));
    assert!(!outcome.complete);

    let phase: String = sqlx::query_scalar(
        "SELECT phase FROM prompt_cache_conversation_migration_progress \
         WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load queue-drain phase after disable");
    assert_eq!(phase, "queue_drain");
}

#[tokio::test]
async fn prompt_cache_materialization_honors_operator_disable_before_empty_phase_transition() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool)
        .await
        .expect("install prompt-cache status schema");
    let maintenance = prompt_cache_materialization_maintenance_store(false).await;
    let control = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized maintenance control");

    for phase in ["identity_reconciliation", "stats_rebuild"] {
        sqlx::query(
            "UPDATE prompt_cache_conversation_migration_progress SET phase = ?1 \
             WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
        )
        .bind(phase)
        .execute(&pool)
        .await
        .expect("set empty materialization phase");

        let should_yield = || false;
        let outcome = run_prompt_cache_conversations_materialization_with_pressure_and_control(
            &pool,
            400,
            None,
            &should_yield,
            &maintenance.prompt_cache_materialization_control,
            control.generation,
        )
        .await
        .expect("run disabled empty materialization phase");
        assert!(outcome.deferred);
        assert_eq!(outcome.defer_reason, Some("operator_disabled"));

        let persisted_phase: String = sqlx::query_scalar(
            "SELECT phase FROM prompt_cache_conversation_migration_progress \
             WHERE migration_name = 'prompt_cache_conversations_materialization_v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load empty materialization phase after disable");
        assert_eq!(persisted_phase, phase);
    }
}

#[tokio::test]
async fn prompt_cache_materialization_wake_preserves_operator_disable() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let maintenance_pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect independent maintenance pool");
    let maintenance = crate::maintenance_store::MaintenanceStore::from_pool(maintenance_pool);
    maintenance
        .initialize_schema_for_test()
        .await
        .expect("initialize independent maintenance schema");
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key=?")
        .bind(task_key)
        .execute(&maintenance.pool)
        .await
        .expect("enable maintenance task fixture");
    maintenance
        .initialize_prompt_cache_materialization_control(task_key, task.name())
        .await
        .expect("initialize injected prompt-cache control");

    let disabled =
        set_prompt_cache_materialization_enabled_with_store(&state.pool, &maintenance, task, false)
            .await
            .expect("disable prompt-cache materialization");
    let woken = wake_prompt_cache_materialization_with_store(&maintenance, "test_terminal_write")
        .await
        .expect("wake prompt-cache materialization");

    let progress = load_startup_backfill_progress_from_pool(&maintenance.pool, task.name())
        .await
        .expect("load disabled prompt-cache progress");
    assert_eq!(woken, 0);
    assert!(!progress.enabled);
    assert_eq!(
        progress.suspension_reason,
        Some("operator_disabled".to_string())
    );
    assert_eq!(progress.wake_generation, disabled.wake_generation);
    assert!(!progress.is_due(Utc::now()));
}

#[tokio::test]
async fn prompt_cache_materialization_failure_history_reports_per_run_work() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        ) VALUES ('prompt-cache-failure-history', '2026-09-01 00:00:00', ?1, 'success', ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "prompt-cache-failure-history-key"}).to_string())
    .execute(&state.pool)
    .await
    .expect("insert prompt-cache failure fixture invocation");
    sqlx::query(
        r#"
        CREATE TRIGGER fail_prompt_cache_materialization
        BEFORE INSERT ON prompt_cache_conversations
        BEGIN
            SELECT RAISE(ABORT, 'forced prompt-cache materialization failure');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install prompt-cache failure trigger");

    let maintenance_pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect independent maintenance pool");
    let maintenance = crate::maintenance_store::MaintenanceStore::from_pool(maintenance_pool);
    maintenance
        .initialize_schema_for_test()
        .await
        .expect("initialize independent maintenance schema");
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key=?")
        .bind(task_key)
        .execute(&maintenance.pool)
        .await
        .expect("enable maintenance task fixture");
    let snapshot = maintenance
        .initialize_prompt_cache_materialization_control(task_key, task.name())
        .await
        .expect("initialize injected prompt-cache control");

    run_prompt_cache_materialization_with_control_for_test(
        &state,
        &maintenance.prompt_cache_materialization_control,
        snapshot.generation,
    )
    .await
    .expect_err("database trigger should fail materialization and record the failure");

    let (status, scanned, updated, error): (String, i64, i64, Option<String>) = sqlx::query_as(
        "SELECT status, scanned, updated, error \
         FROM prompt_cache_conversation_materialization_runs \
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load failed prompt-cache materialization history");
    assert_eq!(status, "failed");
    assert_eq!(scanned, 0);
    assert_eq!(updated, 0);
    assert!(error.is_some());
}

#[tokio::test]
async fn prompt_cache_stale_checkpoint_cannot_overwrite_pause_or_resume() {
    use crate::maintenance::{
        StartupBackfillProgressUpdate, save_startup_backfill_progress_to_pool,
    };
    let store = prompt_cache_materialization_maintenance_store(true).await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    let old_generation = store
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized control")
        .generation;
    for enabled in [false, true] {
        store
            .update_prompt_cache_materialization_control(task_key, task.name(), enabled)
            .await
            .expect("commit pause or resume");
        for status in ["ok", "idle", "failed", "running"] {
            if let Some(_guard) = store
                .prompt_cache_materialization_control
                .lock_current_generation(old_generation)
                .await
            {
                save_startup_backfill_progress_to_pool(
                    &store.pool,
                    task.name(),
                    StartupBackfillProgressUpdate {
                        cursor_id: 99,
                        scanned: 1,
                        updated: 1,
                        zero_update_streak: 0,
                        next_run_after: "2099-01-01T00:00:00Z",
                        status,
                        suspension_reason: Some("stale-result"),
                    },
                )
                .await
                .expect("persist admitted checkpoint");
                panic!("an obsolete result must not be admitted");
            }
        }
        let progress = load_startup_backfill_progress_from_pool(&store.pool, task.name())
            .await
            .expect("read committed control checkpoint");
        assert_eq!(progress.enabled, enabled);
        assert_eq!(progress.cursor_id, 0);
        assert_eq!(
            progress.suspension_reason.as_deref(),
            (!enabled).then_some("operator_disabled")
        );
        assert_eq!(progress.next_run_after.is_none(), enabled);
    }
}

#[tokio::test]
async fn prompt_cache_control_commit_waits_for_admitted_checkpoint_finalization() {
    use crate::maintenance::{
        StartupBackfillProgressUpdate, save_startup_backfill_progress_to_pool,
    };
    let store = prompt_cache_materialization_maintenance_store(true).await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let generation = store
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized control")
        .generation;
    let guard = store
        .prompt_cache_materialization_control
        .lock_current_generation(generation)
        .await
        .expect("register current checkpoint finalization");
    let mut pause = Box::pin(store.update_prompt_cache_materialization_control(
        "startup_backfill.prompt_cache_conversations_materialization",
        task.name(),
        false,
    ));
    std::future::poll_fn(|context| {
        assert!(std::future::Future::poll(pause.as_mut(), context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    save_startup_backfill_progress_to_pool(
        &store.pool,
        task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 17,
            scanned: 3,
            updated: 2,
            zero_update_streak: 0,
            next_run_after: "2099-01-01T00:00:00Z",
            status: "idle",
            suspension_reason: Some("stats_page_pending"),
        },
    )
    .await
    .expect("commit the already admitted result before pause");
    drop(guard);
    pause.await.expect("commit pause after finalization");
    let progress = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("read final paused checkpoint");
    assert_eq!(progress.cursor_id, 17, "committed work is preserved");
    assert!(!progress.enabled);
    assert_eq!(
        progress.suspension_reason.as_deref(),
        Some("operator_disabled")
    );
    assert_ne!(
        progress.next_run_after.as_deref(),
        Some("2099-01-01T00:00:00Z")
    );
}
