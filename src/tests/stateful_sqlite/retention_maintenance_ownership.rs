use super::*;

#[tokio::test]
async fn ownership_initialization_preserves_controls_and_recovers_interrupted_schedule_write() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    for key in [
        "retention_archive",
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
                Some(&json!({"total":100,"completed":1,"ownershipVersion":1})),
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
