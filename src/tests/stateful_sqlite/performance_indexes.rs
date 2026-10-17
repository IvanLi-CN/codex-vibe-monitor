use super::*;
use crate::maintenance::{
    PerformanceIndexPreparationRequest, performance_index_definitions, prepare_performance_indexes,
};

async fn fixture() -> SqlitePool {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "sqlite:file:performance-index-fixture-{id}?mode=memory&cache=shared"
        ))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE codex_invocations(id INTEGER PRIMARY KEY, payload TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE proxy_raw_payload_blob_links(owner_kind TEXT, owner_id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

async fn prepare(
    pool: &SqlitePool,
    dry_run: bool,
) -> crate::maintenance::PerformanceIndexPreparationReport {
    prepare_performance_indexes(
        pool.connect_options().as_ref().clone(),
        PerformanceIndexPreparationRequest {
            dry_run,
            budget_ms: 2_000,
        },
        crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator(),
        CancellationToken::new(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn performance_indexes_dry_run_is_read_only_and_reentry_recognizes_partial_state() {
    let pool = fixture().await;
    let metrics = ObservabilityRuntime::new(true);
    let dry_run =
        crate::observability::contention::with_performance(metrics.clone(), prepare(&pool, true))
            .await;
    assert_eq!(dry_run.outcome, "dry_run");
    assert_eq!(dry_run.indexes.len(), 2);
    assert!(
        dry_run
            .indexes
            .iter()
            .all(|index| index.outcome == "would_create")
    );
    let before_scrape = metrics.render();
    assert!(
        before_scrape.lines().any(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("reason=\"index_missing\"")
                && line.ends_with(" 2")
        }),
        "the owned inspector inherits its explicit metric scope"
    );
    assert!(
        before_scrape.lines().any(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("reason=\"query_plan_unavailable\"")
                && line.ends_with(" 1")
        }),
        "a fixture without the conversation query schema has an unavailable plan, not a fallback"
    );
    let after_scrape = metrics.render();
    let mut before_lines: Vec<_> = before_scrape.lines().collect();
    let mut after_lines: Vec<_> = after_scrape.lines().collect();
    before_lines.sort_unstable();
    after_lines.sort_unstable();
    assert_eq!(
        after_lines, before_lines,
        "scraping must not repeat the inspection"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='index'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // A stopped earlier operation left its first independently committed index intact.
    let definitions = performance_index_definitions();
    sqlx::query(&definitions[0].1).execute(&pool).await.unwrap();
    let resumed = prepare(&pool, false).await;
    assert!(resumed.is_complete());
    assert_eq!(resumed.indexes[0].outcome, "present");
    assert_eq!(resumed.indexes[1].outcome, "created");
    let repeated = prepare(&pool, false).await;
    assert!(
        repeated
            .indexes
            .iter()
            .all(|index| index.outcome == "present")
    );
    pool.close().await;
}

#[tokio::test]
async fn performance_indexes_conflicting_definition_does_not_replace_or_hide_failure() {
    let pool = fixture().await;
    sqlx::query(&performance_index_definitions()[0].1)
        .execute(&pool)
        .await
        .unwrap();
    // JSON property case matters even though SQL keywords do not.
    sqlx::query("CREATE INDEX idx_codex_invocations_prompt_cache_key_id ON codex_invocations(CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptcachekey') AS TEXT)) END, id DESC)")
        .execute(&pool).await.unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE name='idx_codex_invocations_prompt_cache_key_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let report = prepare(&pool, false).await;
    assert_eq!(report.outcome, "definition_conflict");
    assert!(!report.is_complete());
    let after: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE name='idx_codex_invocations_prompt_cache_key_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    pool.close().await;
}

async fn last_invocation_id(pool: &SqlitePool, key: &str) -> Option<i64> {
    let rows = query_prompt_cache_conversation_statistics(pool, &[key.to_string()])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    rows[0].last_invocation_id
}

#[tokio::test]
async fn performance_indexes_preserve_live_max_normalization_and_use_target_indexes() {
    let pool = fixture().await;
    sqlx::query(
        "CREATE TABLE prompt_cache_conversations(
            prompt_cache_key TEXT, conversation_id TEXT, created_at TEXT,
            success_count INTEGER DEFAULT 0, failure_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
            cache_input_tokens INTEGER DEFAULT 0, reported_cache_write_tokens INTEGER DEFAULT 0,
            reasoning_tokens INTEGER DEFAULT 0, cost_input REAL DEFAULT 0,
            cost_cache_write REAL DEFAULT 0, cost_cache_read REAL DEFAULT 0,
            cost_output REAL DEFAULT 0, cost_reasoning REAL DEFAULT 0,
            first_invocation_at TEXT, last_invocation_at TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at) VALUES('cache','conversation-cache','2026-10-16'),('42','conversation-number','2026-10-16')")
        .execute(&pool).await.unwrap();
    for (id, payload) in [
        (1, r#"{"promptCacheKey":" cache "}"#),
        (2, r#"{"promptCacheKey":"cache"}"#),
        (3, r#"{"promptCacheKey":42}"#),
        (4, "invalid-json"),
        (5, r#"{"promptCacheKey":null}"#),
    ] {
        sqlx::query("INSERT INTO codex_invocations(id,payload) VALUES(?,?)")
            .bind(id)
            .bind(payload)
            .execute(&pool)
            .await
            .unwrap();
    }
    let before = [
        last_invocation_id(&pool, "cache").await,
        last_invocation_id(&pool, "42").await,
    ];
    assert_eq!(before, [Some(2), Some(3)]);
    assert!(prepare(&pool, false).await.is_complete());
    assert_eq!(
        before,
        [
            last_invocation_id(&pool, "cache").await,
            last_invocation_id(&pool, "42").await
        ]
    );
    sqlx::query("DELETE FROM codex_invocations WHERE id=2")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        last_invocation_id(&pool, "cache").await,
        Some(1),
        "MAX follows currently live rows, including deletion"
    );
    sqlx::query("UPDATE codex_invocations SET payload='{}' WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(last_invocation_id(&pool, "cache").await, None);
    let plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN SELECT MAX(i.id) FROM codex_invocations AS i WHERE {} = ?",
        INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL
    ))
    .bind("cache")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(plan.iter().any(|row| {
        row.get::<String, _>("detail")
            .contains("idx_codex_invocations_prompt_cache_key_id")
    }));
    let raw_plan = sqlx::query("EXPLAIN QUERY PLAN SELECT rowid FROM proxy_raw_payload_blob_links WHERE owner_kind='invocation' AND owner_id=1")
        .fetch_all(&pool).await.unwrap();
    assert!(raw_plan.iter().any(|row| {
        row.get::<String, _>("detail")
            .contains("idx_proxy_raw_payload_blob_links_owner")
    }));
    pool.close().await;
}

#[tokio::test]
async fn performance_indexes_pre_cancelled_owner_does_not_open_or_write() {
    let pool = fixture().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let report = prepare_performance_indexes(
        pool.connect_options().as_ref().clone(),
        PerformanceIndexPreparationRequest::default(),
        crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator(),
        cancel,
    )
    .await
    .unwrap();
    assert_eq!(report.outcome, "cancelled");
    assert!(report.indexes.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='index'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    pool.close().await;
}

#[tokio::test]
async fn performance_indexes_online_request_persists_budget_and_manual_only_default() {
    let store = crate::maintenance_store::ownership_test_store().await;
    store.apply_initial_task_defaults().await.unwrap();
    let controls: (bool, bool) = sqlx::query_as(
        "SELECT enabled,is_manual FROM managed_tasks WHERE task_key='prepare_performance_indexes'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(controls, (false, true));
    let request = PerformanceIndexPreparationRequest {
        dry_run: true,
        budget_ms: 17,
    };
    let id = store.request_performance_indexes(request).await.unwrap();
    let (status, details): (String, String) =
        sqlx::query_as("SELECT status,details FROM managed_task_runs WHERE id=?")
            .bind(id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(status, "requested");
    assert_eq!(
        serde_json::from_str::<Value>(&details).unwrap(),
        json!({"dryRun": true, "budgetMs":17})
    );
    assert!(
        store.request_performance_indexes(request).await.is_err(),
        "one active request per task"
    );
    assert!(
        store
            .request_performance_indexes(PerformanceIndexPreparationRequest {
                budget_ms: 0,
                ..request
            })
            .await
            .is_err()
    );
}
