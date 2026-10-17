use super::*;
use crate::maintenance::{
    PerformanceIndexPreparationRequest, performance_index_definitions, prepare_performance_indexes,
};
use crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator;
use crate::sqlite_batch_writer::{
    BatchedTerminalInvocationWrite, SqliteBatchWrite, SqliteBatchWriter,
};

async fn prompt_cache_statistics_fixture() -> SqlitePool {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "sqlite:file:statistics-recovery-fixture-{id}?mode=memory&cache=shared"
        ))
        .await
        .expect("open statistics recovery fixture");
    sqlx::query(
        "CREATE TABLE codex_invocations(id INTEGER PRIMARY KEY, occurred_at TEXT, payload TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create invocation fixture");
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
    .expect("create prompt-cache fixture");
    sqlx::query("CREATE TABLE proxy_raw_payload_blob_links(owner_kind TEXT, owner_id INTEGER)")
        .execute(&pool)
        .await
        .expect("create raw payload fixture");
    sqlx::query(
        "INSERT INTO prompt_cache_conversations(prompt_cache_key, conversation_id, created_at)
         VALUES ('cache', 'conversation-cache', '2026-10-16'),
                ('42', 'conversation-number', '2026-10-16'),
                ('missing', 'conversation-missing', '2026-10-16')",
    )
    .execute(&pool)
    .await
    .expect("seed prompt-cache conversations");
    pool
}

async fn prepare_fixture_indexes(
    pool: &SqlitePool,
) -> crate::maintenance::PerformanceIndexPreparationReport {
    prepare_performance_indexes(
        pool.connect_options().as_ref().clone(),
        PerformanceIndexPreparationRequest::default(),
        test_proxy_sqlite_write_coordinator(),
        CancellationToken::new(),
    )
    .await
    .expect("prepare fixture indexes")
}

async fn prompt_cache_last_invocation_id(pool: &SqlitePool, key: &str) -> Option<i64> {
    let rows = query_prompt_cache_conversation_statistics(pool, &[key.to_string()])
        .await
        .expect("query prompt-cache statistics");
    assert_eq!(rows.len(), 1);
    rows[0].last_invocation_id
}

async fn reference_prompt_cache_last_invocation_id(pool: &SqlitePool, key: &str) -> Option<i64> {
    sqlx::query_scalar::<_, Option<i64>>(&format!(
        "SELECT MAX(id) FROM codex_invocations WHERE {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = ?1"
    ))
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("query reference prompt-cache statistics")
}

async fn prompt_cache_semantic_snapshot(
    pool: &SqlitePool,
    keys: &[&str],
) -> Vec<(Option<i64>, Option<i64>)> {
    let mut snapshot = Vec::with_capacity(keys.len());
    for key in keys {
        snapshot.push((
            prompt_cache_last_invocation_id(pool, key).await,
            reference_prompt_cache_last_invocation_id(pool, key).await,
        ));
    }
    snapshot
}

fn print_bounded_prompt_cache_plan(label: &str, details: &[String]) {
    let bounded_details = details
        .iter()
        .take(8)
        .map(String::as_str)
        .collect::<Vec<_>>();
    eprintln!("prompt-cache {label} EXPLAIN QUERY PLAN: {bounded_details:?}");
}

async fn explain_prompt_cache_statistics_query(
    pool: &SqlitePool,
    selected_keys: &[String],
) -> Vec<String> {
    build_prompt_cache_conversation_statistics_query("EXPLAIN QUERY PLAN ", selected_keys)
        .build_query_as::<(i64, i64, i64, String)>()
        .persistent(false)
        .fetch_all(pool)
        .await
        .expect("explain production prompt-cache statistics query")
        .into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect()
}

#[tokio::test]
async fn statistics_recovery_matches_live_max_reference_before_and_after_index_preparation() {
    let pool = prompt_cache_statistics_fixture().await;
    for (id, occurred_at, payload) in [
        (1, "2026-10-15T23:59:00Z", r#"{"promptCacheKey":" cache "}"#),
        (2, "2026-10-16T00:01:00Z", r#"{"promptCacheKey":"cache"}"#),
        (3, "2026-10-16T00:02:00Z", r#"{"promptCacheKey":42}"#),
        (4, "2026-10-16T00:03:00Z", "invalid-json"),
        (5, "2026-10-16T00:04:00Z", r#"{"promptCacheKey":null}"#),
    ] {
        sqlx::query("INSERT INTO codex_invocations(id, occurred_at, payload) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(occurred_at)
            .bind(payload)
            .execute(&pool)
            .await
            .expect("seed invocation row");
    }

    let semantic_keys = ["cache", "42", "missing"];
    let before_index_semantics = prompt_cache_semantic_snapshot(&pool, &semantic_keys).await;
    let selected_keys = vec!["cache".to_string()];
    let missing_plan_details = explain_prompt_cache_statistics_query(&pool, &selected_keys).await;

    let prepared = prepare_fixture_indexes(&pool).await;
    let after_index_semantics = prompt_cache_semantic_snapshot(&pool, &semantic_keys).await;
    let indexed_plan_details = explain_prompt_cache_statistics_query(&pool, &selected_keys).await;
    let production_sql =
        build_prompt_cache_conversation_statistics_query("EXPLAIN QUERY PLAN ", &selected_keys)
            .sql()
            .to_string();
    let affinity_candidate_sql =
        production_sql.replace("= c.prompt_cache_key", "= +c.prompt_cache_key");
    let affinity_candidate_plan =
        sqlx::query_as::<_, (i64, i64, i64, String)>(&affinity_candidate_sql)
            .bind("cache")
            .persistent(false)
            .fetch_all(&pool)
            .await
            .expect("explain prompt-cache affinity candidate")
            .into_iter()
            .map(|(_, _, _, detail)| detail)
            .collect::<Vec<_>>();
    print_bounded_prompt_cache_plan("missing-index", &missing_plan_details);
    print_bounded_prompt_cache_plan("correlated", &indexed_plan_details);
    print_bounded_prompt_cache_plan("affinity-candidate", &affinity_candidate_plan);
    assert!(production_sql.contains("WHERE ") && production_sql.contains("= c.prompt_cache_key"));
    assert!(
        !missing_plan_details.is_empty() && missing_plan_details.len() <= 8,
        "missing-index plan evidence must stay bounded: {missing_plan_details:?}"
    );
    assert!(prepared.is_complete());
    assert_eq!(
        prepared.indexes.len(),
        performance_index_definitions().len()
    );
    assert!(
        prepared
            .indexes
            .iter()
            .all(|index| { matches!(index.outcome, "created" | "present") })
    );
    assert!(
        before_index_semantics
            .iter()
            .all(|(actual, reference)| actual == reference),
        "statistics query must match the live-row reference without optional indexes"
    );
    assert!(
        after_index_semantics
            .iter()
            .all(|(actual, reference)| actual == reference),
        "index preparation must not change live-row statistics semantics"
    );
    assert!(
        affinity_candidate_plan
            .iter()
            .any(|detail| { detail.contains("idx_codex_invocations_prompt_cache_key_id") }),
        "prompt-cache affinity candidate must retain the target index plan: {affinity_candidate_plan:?}"
    );
    assert!(
        indexed_plan_details.len() <= 8
            && indexed_plan_details
                .iter()
                .any(|detail| { detail.contains("idx_codex_invocations_prompt_cache_key_id") }),
        "production correlated statistics query must use the target expression index: {indexed_plan_details:?}"
    );

    sqlx::query(
        "UPDATE codex_invocations SET payload = '{\"promptCacheKey\":\"other\"}' WHERE id = 2",
    )
    .execute(&pool)
    .await
    .expect("change prompt-cache generation");
    assert_eq!(
        prompt_cache_last_invocation_id(&pool, "cache").await,
        reference_prompt_cache_last_invocation_id(&pool, "cache").await
    );
    sqlx::query(
        "INSERT INTO codex_invocations(id, occurred_at, payload)
         VALUES (6, '2026-10-17T00:01:00Z', '{\"promptCacheKey\":\" cache \"}')",
    )
    .execute(&pool)
    .await
    .expect("append current-generation invocation");
    assert_eq!(
        prompt_cache_last_invocation_id(&pool, "cache").await,
        Some(6),
        "live MAX(id) must follow the current generation after a cross-day append"
    );
    assert_eq!(
        prompt_cache_last_invocation_id(&pool, "cache").await,
        reference_prompt_cache_last_invocation_id(&pool, "cache").await
    );
    pool.close().await;
}

async fn commit_statistics_recovery_terminal(state: &AppState, invoke_id: &str, occurred_at: &str) {
    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::TerminalInvocation(
            statistics_recovery_terminal(invoke_id, occurred_at),
        )],
    )
    .await;
}

fn statistics_recovery_terminal(
    invoke_id: &str,
    occurred_at: &str,
) -> BatchedTerminalInvocationWrite {
    let mut record = crate::tests::test_proxy_capture_record(invoke_id, occurred_at);
    record.payload = Some("{}".to_string());
    record.usage.input_tokens = Some(16);
    record.usage.cache_input_tokens = Some(0);
    record.usage.total_tokens = Some(19);
    record.cost = Some(1.5);
    BatchedTerminalInvocationWrite {
        enqueued_at: None,
        attempts: Vec::new(),
        diagnostic: None,
        capture_started: None,
        raw_capture: false,
        dashboard_terminal_sequence: None,
        terminal_projection_event_ids: Vec::new(),
        startup_backfill_tasks: Vec::new(),
        record,
    }
}

#[tokio::test]
async fn statistics_recovery_revalidates_live_tail_before_advancing_checkpoint() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    commit_statistics_recovery_terminal(state.as_ref(), "statistics-recovery-base", &historical_at)
        .await;
    hydrate_stats_snapshot_for_test(&state).await;
    let base_cursor = crate::summary_source_change::load_summary_source_change_checkpoint(
        &state.pool,
        crate::summary_source_change::SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
    )
    .await
    .expect("read base published source cursor");

    let current_at = db_occurred_at_lower_bound(Utc::now());
    commit_statistics_recovery_terminal(state.as_ref(), "statistics-recovery-new", &current_at)
        .await;

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create live-tail interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeLiveTailPublication,
    );
    let refresh_state = state.clone();
    let mut refresh = tokio::spawn(async move {
        refresh_summary_snapshots_with_mode(
            refresh_state.as_ref(),
            SummaryProjectionBuildMode::RollingDelta,
        )
        .await
    });
    tokio::select! {
        _ = interleave.wait_for_writer() => {}
        result = &mut refresh => {
            clear_summary_projection_test_interleave();
            result.expect("join live-tail recovery").expect("live-tail recovery");
            panic!("live-tail recovery must pause before publication");
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            clear_summary_projection_test_interleave();
            refresh.abort();
            panic!("live-tail recovery did not reach publication within its budget");
        }
    }
    let concurrent_at = db_occurred_at_lower_bound(Utc::now());
    commit_statistics_recovery_terminal(
        state.as_ref(),
        "statistics-recovery-concurrent",
        &concurrent_at,
    )
    .await;
    interleave.resume_build();
    refresh
        .await
        .expect("join stale-target recovery")
        .expect("stale-target recovery remains bounded");
    clear_summary_projection_test_interleave();

    let deferred = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
        .await
        .expect("read deferred live-tail checkpoint")
        .expect("live-tail recovery must persist a checkpoint");
    assert_eq!(deferred.next_source_cursor, base_cursor);
    assert_ne!(deferred.state, "ready");
    assert_eq!(
        crate::summary_source_change::load_summary_source_change_checkpoint(
            &state.pool,
            crate::summary_source_change::SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
        )
        .await
        .expect("read published source cursor"),
        base_cursor,
        "source cursor must not advance before durable publication succeeds"
    );

    refresh_summary_snapshots_with_mode(&state, SummaryProjectionBuildMode::RollingDelta)
        .await
        .expect("retry live-tail recovery");
    let published = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
        .await
        .expect("read published live-tail checkpoint")
        .expect("published live-tail checkpoint");
    assert_eq!(published.state, "ready");
    assert_eq!(published.next_source_cursor, published.target_source_cursor);
    assert!(published.target_source_cursor > base_cursor);
    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve exact recovered recent statistics");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.total_tokens, 38);
    state.pool.close().await;
}
