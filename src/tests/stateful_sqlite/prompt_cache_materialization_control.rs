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

async fn prompt_cache_statistics_checkpoint_fixture(counts: &[usize]) -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("independent business sqlite");
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(&pool)
        .await
        .expect("create invocation schema");
    ensure_schema(&pool).await.expect("install schema");
    let mut tx = pool.begin().await.expect("begin checkpoint fixture");
    for (key, count) in counts.iter().enumerate() {
        for index in 0..*count {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,payload,raw_response) \
                 VALUES (?1,'2026-09-01T00:00:00Z',?2,'success',1,?3,'{}')",
            )
            .bind(format!("checkpoint-{key}-{index}"))
            .bind(SOURCE_PROXY)
            .bind(json!({"promptCacheKey": format!("checkpoint-key-{key:03}")}).to_string())
            .execute(&mut *tx)
            .await
            .expect("insert checkpoint invocation");
        }
    }
    tx.commit().await.expect("commit checkpoint fixture");
    run_prompt_cache_conversations_materialization(&pool, counts.len() as u64, None)
        .await
        .expect("commit fixture identity discovery");
    sqlx::query(
        "UPDATE prompt_cache_conversation_migration_progress SET phase='stats_rebuild',cursor_key=NULL \
         WHERE migration_name='prompt_cache_conversations_materialization_v1'",
    )
    .execute(&pool)
    .await
    .expect("start statistics rebuild from its first key");
    sqlx::query(
        "INSERT OR IGNORE INTO schema_refresh_migrations (migration_name) VALUES ('prompt_cache_conversations_v1')",
    )
    .execute(&pool)
    .await
    .expect("record complete identity coverage");
    sqlx::query("CREATE TABLE checkpoint_publications (prompt_cache_key TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create publication observation table");
    sqlx::query(
        "CREATE TRIGGER checkpoint_publication AFTER UPDATE OF request_count ON prompt_cache_conversations \
         BEGIN INSERT INTO checkpoint_publications VALUES (NEW.prompt_cache_key); END",
    )
    .execute(&pool)
    .await
    .expect("observe real statistics publications");
    pool
}

#[tokio::test]
async fn prompt_cache_statistics_checkpoint_continues_multiple_large_keys_in_one_run() {
    let counts = [1, 512, 37, 768, 257];
    let pool = prompt_cache_statistics_checkpoint_fixture(&counts).await;
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let control = &maintenance.prompt_cache_materialization_control;
    let generation = control.snapshot().expect("trusted control").generation;
    let outcome = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        Some(Duration::from_secs(3)),
        &|| false,
        control,
        generation,
    )
    .await
    .expect("materialize unchanged multi-key statistics");
    assert!(
        outcome.complete,
        "partial source pages must use the remaining run budget"
    );
    let actual: Vec<i64> = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations ORDER BY prompt_cache_key",
    )
    .fetch_all(&pool)
    .await
    .expect("read exact aggregates");
    assert_eq!(actual, counts.map(|count| count as i64));
    let publications: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM checkpoint_publications")
        .fetch_one(&pool)
        .await
        .expect("count complete publications");
    assert_eq!(publications, counts.len() as i64);
}

#[tokio::test]
async fn prompt_cache_statistics_checkpoint_rolls_back_publication_when_cursor_commit_fails() {
    let pool = prompt_cache_statistics_checkpoint_fixture(&[37]).await;
    sqlx::query(
        "CREATE TRIGGER fail_checkpoint BEFORE UPDATE OF cursor_key ON prompt_cache_conversation_migration_progress \
         WHEN NEW.phase='stats_rebuild' AND NEW.cursor_key IS NOT NULL \
         BEGIN SELECT RAISE(ABORT,'checkpoint commit failure'); END",
    )
    .execute(&pool)
    .await
    .expect("inject a final-page checkpoint failure");
    let result = run_prompt_cache_conversations_materialization(&pool, 400, None).await;
    assert!(result.is_err(), "the final checkpoint must fail");
    let (request_count, queue_count, publications): (i64, i64, i64) = sqlx::query_as(
        "SELECT request_count, \
         (SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue), \
         (SELECT COUNT(*) FROM checkpoint_publications) FROM prompt_cache_conversations",
    )
    .fetch_one(&pool)
    .await
    .expect("read atomic publication state");
    assert_eq!((request_count, queue_count, publications), (0, 1, 0));
    sqlx::query("DROP TRIGGER fail_checkpoint")
        .execute(&pool)
        .await
        .expect("remove injected failure");
    let repaired = run_prompt_cache_conversations_materialization(&pool, 400, None)
        .await
        .expect("resume after transaction failure");
    assert!(repaired.complete);
}

#[tokio::test]
async fn prompt_cache_statistics_budget_before_connection_does_not_count_a_scanned_key() {
    let business = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("independent single-connection business pool");
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let control = &maintenance.prompt_cache_materialization_control;
    let generation = control.snapshot().expect("trusted control").generation;
    let held_connection = business.acquire().await.expect("hold business connection");
    let (scanned, updated, reason) = run_prompt_cache_statistics_key_with_budget_for_test(
        &business,
        "unvisited-key".to_owned(),
        Duration::from_millis(50),
        control,
        generation,
    )
    .await
    .expect("connection wait consumes only the statistics budget");
    assert_eq!(reason, Some("stats_budget_exhausted"));
    assert_eq!((scanned, updated), (0, 0));
    drop(held_connection);
    assert!(business.acquire().await.is_ok(), "no connection is leaked");
}

#[tokio::test]
async fn prompt_cache_statistics_checkpoint_budget_preserves_prefix_and_real_work_counts() {
    let pool = prompt_cache_statistics_checkpoint_fixture(&[1, 10_000, 1]).await;
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let control = &maintenance.prompt_cache_materialization_control;
    let generation = control.snapshot().expect("trusted control").generation;
    let run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        Some(Duration::from_millis(150)),
        &|| false,
        control,
        generation,
    )
    .await
    .expect("bounded statistics run");
    assert_eq!(run.defer_reason, Some("stats_budget_exhausted"));
    assert_eq!(
        run.scanned, 2,
        "a partial key consumes one allowance; the later key is untouched"
    );
    assert_eq!(run.updated, 1, "only the completed key was published");
    let cursor: Option<String> =
        sqlx::query_scalar("SELECT cursor_key FROM prompt_cache_conversation_migration_progress")
            .fetch_one(&pool)
            .await
            .expect("continuous checkpoint");
    assert_eq!(cursor.as_deref(), Some("checkpoint-key-000"));
    let status =
        load_prompt_cache_conversation_materialization_status(&pool, &maintenance.pool, control)
            .await
            .expect("incomplete statistics status");
    assert_eq!(status.estimated_remaining_ms, None);
    // Restoring the control object models process restart, leaving business checkpoints intact.
    let restarted = MaintenanceStore::from_pool(maintenance.pool.clone());
    restarted
        .initialize_prompt_cache_materialization_control(
            "startup_backfill.prompt_cache_conversations_materialization",
            StartupBackfillTask::PromptCacheConversationsMaterialization.name(),
        )
        .await
        .expect("restore maintenance control");
    let generation = restarted
        .prompt_cache_materialization_control
        .snapshot()
        .unwrap()
        .generation;
    let done = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        Some(Duration::from_secs(3)),
        &|| false,
        &restarted.prompt_cache_materialization_control,
        generation,
    )
    .await
    .expect("resume partial key after restart");
    assert!(done.complete);
    assert_eq!(done.scanned, 2);
    assert_eq!(done.updated, 2);
    let publications: Vec<String> =
        sqlx::query_scalar("SELECT prompt_cache_key FROM checkpoint_publications ORDER BY rowid")
            .fetch_all(&pool)
            .await
            .expect("publication sequence");
    assert_eq!(
        publications,
        [
            "checkpoint-key-000",
            "checkpoint-key-001",
            "checkpoint-key-002"
        ]
    );
    let status = load_prompt_cache_conversation_materialization_status(
        &pool,
        &maintenance.pool,
        &restarted.prompt_cache_materialization_control,
    )
    .await
    .expect("complete statistics status");
    assert_eq!(status.estimated_remaining_ms, Some(0));
}

#[tokio::test]
async fn prompt_cache_statistics_checkpoint_pause_cancels_only_future_pages_and_requeues_changed_prefix()
 {
    let pool = prompt_cache_statistics_checkpoint_fixture(&[37, 4096, 58]).await;
    let maintenance =
        std::sync::Arc::new(prompt_cache_materialization_maintenance_store(true).await);
    let generation = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .unwrap()
        .generation;
    let observer_pool = pool.clone();
    let observer_store = maintenance.clone();
    let observer = tokio::spawn(async move {
        loop {
            let staged: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_staging WHERE prompt_cache_key='checkpoint-key-001' AND cursor_id > 0",
            ).fetch_one(&observer_pool).await.expect("observe committed partial page");
            if staged > 0 {
                observer_store
                    .update_prompt_cache_materialization_control(
                        "startup_backfill.prompt_cache_conversations_materialization",
                        StartupBackfillTask::PromptCacheConversationsMaterialization.name(),
                        false,
                    )
                    .await
                    .expect("pause between committed pages");
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    let run = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &|| false,
        &maintenance.prompt_cache_materialization_control,
        generation,
    )
    .await
    .expect("stop after registered page commits");
    observer.await.expect("pause observer");
    assert!(
        run.defer_reason == Some("operator_disabled") || run.control_generation_changed,
        "pause at the page boundary must prevent the next page: {run:?}"
    );
    assert_eq!((run.scanned, run.updated), (2, 1));
    let staged_cursor: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM prompt_cache_conversation_stats_refresh_staging WHERE prompt_cache_key='checkpoint-key-001'",
    ).fetch_one(&pool).await.expect("durable partial cursor");
    assert!(staged_cursor > 37 && staged_cursor < 37 + 4096);
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,payload,raw_response) VALUES ('prefix-new','2026-09-02T00:00:00Z','proxy','success',1,?,'{}')",
    ).bind(json!({"promptCacheKey":"checkpoint-key-000"}).to_string())
    .execute(&pool).await.expect("enqueue new source behind completed prefix");
    maintenance
        .update_prompt_cache_materialization_control(
            "startup_backfill.prompt_cache_conversations_materialization",
            StartupBackfillTask::PromptCacheConversationsMaterialization.name(),
            true,
        )
        .await
        .expect("resume with a new control generation");
    let generation = maintenance
        .prompt_cache_materialization_control
        .snapshot()
        .unwrap()
        .generation;
    let done = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        None,
        &|| false,
        &maintenance.prompt_cache_materialization_control,
        generation,
    )
    .await
    .expect("finish rebuild and drain changed prefix");
    assert!(done.complete);
    let publications: Vec<String> =
        sqlx::query_scalar("SELECT prompt_cache_key FROM checkpoint_publications ORDER BY rowid")
            .fetch_all(&pool)
            .await
            .expect("only changed prefix gets a second publication");
    assert_eq!(
        publications,
        [
            "checkpoint-key-000",
            "checkpoint-key-001",
            "checkpoint-key-002",
            "checkpoint-key-000"
        ]
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations WHERE prompt_cache_key='checkpoint-key-000'",
    ).fetch_one(&pool).await.expect("exact changed aggregate");
    assert_eq!(count, 38);
}

#[tokio::test]
async fn prompt_cache_statistics_checkpoint_queue_drain_does_not_use_rebuild_cursor() {
    let pool = prompt_cache_statistics_checkpoint_fixture(&[512, 768, 58]).await;
    sqlx::query("UPDATE prompt_cache_conversation_migration_progress SET phase='queue_drain',cursor_key='preserved-until-complete'")
        .execute(&pool).await.expect("old queue-drain checkpoint");
    let run = run_prompt_cache_conversations_materialization(&pool, 1, None)
        .await
        .expect("drain only first multi-page conversation");
    assert_eq!((run.scanned, run.updated), (1, 1));
    let cursor: String =
        sqlx::query_scalar("SELECT cursor_key FROM prompt_cache_conversation_migration_progress")
            .fetch_one(&pool)
            .await
            .expect("queue drain leaves scan cursor alone");
    assert_eq!(cursor, "preserved-until-complete");
    let done = run_prompt_cache_conversations_materialization(&pool, 400, None)
        .await
        .expect("remaining queued conversations advance");
    assert!(done.complete);
    assert_eq!((done.scanned, done.updated), (2, 2));
    let publications: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM checkpoint_publications")
        .fetch_one(&pool)
        .await
        .expect("each queued conversation published once");
    assert_eq!(publications, 3);
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
    assert_eq!(
        pending_pages, 0,
        "unchanged pages continue within the same run"
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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
    let observer_pool = pool.clone();
    // Observe the committed page at the priority check itself. A separately
    // scheduled observer can run after several pages under suite contention.
    let should_yield = || {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let staged: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_staging WHERE cursor_id > 0",
                )
                .fetch_one(&observer_pool)
                .await
                .expect("observe a committed page before yielding");
                staged > 0
            })
        })
    };
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
    assert_eq!(first_page.defer_reason, Some("coordinator_priority"));
    let should_yield = || false;
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
    assert_eq!(status.estimated_remaining_ms, None);
}

#[tokio::test]
async fn prompt_cache_queue_event_wake_converges_past_idle_deadline() {
    let pool = prompt_cache_statistics_checkpoint_fixture(&[1]).await;
    let maintenance = prompt_cache_materialization_maintenance_store(true).await;
    let control = &maintenance.prompt_cache_materialization_control;
    let generation = control.snapshot().expect("trusted control").generation;
    let initial = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        Some(Duration::from_secs(3)),
        &|| false,
        control,
        generation,
    )
    .await
    .expect("complete the baseline materialization");
    assert!(initial.complete);
    assert!(
        prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check baseline freshness")
    );

    let prompt_cache_key = "checkpoint-key-000";
    let source_generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM prompt_cache_conversation_stats_generation_clock \
         WHERE prompt_cache_key=?",
    )
    .bind(prompt_cache_key)
    .fetch_one(&pool)
    .await
    .expect("load queue generation");
    sqlx::query(
        "INSERT INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key,generation) \
         VALUES (?,?) ON CONFLICT(prompt_cache_key) DO UPDATE SET generation=excluded.generation",
    )
    .bind(prompt_cache_key)
    .bind(source_generation)
    .execute(&pool)
    .await
    .expect("enqueue a post-completion statistics refresh");
    mark_prompt_cache_conversation_stats_stale(&pool)
        .await
        .expect("mark queued statistics stale");

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(Utc::now() + ChronoDuration::hours(6)))
        .bind(StartupBackfillTask::PromptCacheConversationsMaterialization.name())
        .execute(&maintenance.pool)
        .await
        .expect("seed future idle deadline");
    let woken =
        wake_prompt_cache_materialization_for_test(&maintenance, "test_terminal_queue_event")
            .await
            .expect("queue event should preempt idle deadline");
    assert_eq!(woken, 1);
    let progress = load_startup_backfill_progress_from_pool(
        &maintenance.pool,
        StartupBackfillTask::PromptCacheConversationsMaterialization.name(),
    )
    .await
    .expect("load event-woken checkpoint");
    assert!(progress.is_due(Utc::now()));
    assert!(progress.next_run_after.is_none());

    let completed = run_prompt_cache_conversations_materialization_with_pressure_and_control(
        &pool,
        400,
        Some(Duration::from_secs(3)),
        &|| false,
        control,
        generation,
    )
    .await
    .expect("drain the event-triggered refresh");
    assert!(completed.complete);
    let status =
        load_prompt_cache_conversation_materialization_status(&pool, &maintenance.pool, control)
            .await
            .expect("load complete materialization status");
    assert_eq!(status.queue_pending, 0);
    assert_eq!(status.progress_percent, Some(100.0));
    assert!(
        prompt_cache_conversation_materialization_is_complete(&pool)
            .await
            .expect("check fresh statistics marker")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM schema_refresh_migrations \
             WHERE migration_name='prompt_cache_conversations_stats_v2')",
        )
        .fetch_one(&pool)
        .await
        .expect("read statistics freshness marker"),
        1
    );
}

#[tokio::test]
async fn prompt_cache_coordinator_retry_routes_as_non_pressure_and_wakes_on_event() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let maintenance = MaintenanceStore::from_pool(state.pool.clone());
    maintenance
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance schema");
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key=?")
        .bind(task_key)
        .execute(&maintenance.pool)
        .await
        .expect("enable prompt-cache coordinator fixture");
    maintenance
        .initialize_prompt_cache_materialization_control(task_key, task.name())
        .await
        .expect("initialize prompt-cache coordinator control");
    sqlx::query(
        "UPDATE startup_backfill_progress
         SET next_run_after=NULL, suspension_reason=NULL, last_status='idle', enabled=1
         WHERE task_name=?",
    )
    .bind(task.name())
    .execute(&state.pool)
    .await
    .expect("make prompt-cache task due");

    clear_startup_backfill_task_scheduler_for_test(task);
    let pressure_defer_count = startup_backfill_health_snapshot().pressure_defer_count;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let foreground_permit = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let outcome =
        run_startup_backfill_task_if_due_with_store_for_test(&state, task, &gate, &maintenance)
            .await
            .expect("coordinator contention should return a deferred outcome");
    assert!(outcome.is_pressure_deferred());
    record_startup_backfill_task_outcome_for_test(task, outcome);
    assert_eq!(
        startup_backfill_health_snapshot().pressure_defer_count,
        pressure_defer_count,
        "coordinator priority must not increment database-pressure telemetry"
    );
    drop(foreground_permit);

    let future_deadline = Utc::now() + ChronoDuration::hours(6);
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(future_deadline))
        .bind(task.name())
        .execute(&maintenance.pool)
        .await
        .expect("seed coordinator retry checkpoint");
    let woken = wake_prompt_cache_materialization_with_store(
        &maintenance,
        "test_queue_event_after_coordinator_retry",
    )
    .await
    .expect("queue event should preempt coordinator retry");
    assert_eq!(woken, 1);
    assert!(
        load_startup_backfill_progress_from_pool(&maintenance.pool, task.name())
            .await
            .expect("load event-woken coordinator checkpoint")
            .is_due(Utc::now())
    );
    clear_startup_backfill_task_scheduler_for_test(task);
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
