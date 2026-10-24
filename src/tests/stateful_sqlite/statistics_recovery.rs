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

#[tokio::test]
async fn explicit_index_inspection_observes_actual_statistics_plan_without_scrape_queries() {
    let pool = prompt_cache_statistics_fixture().await;
    let metrics = ObservabilityRuntime::new(true);
    for reason in ["query_plan_fallback", "query_plan_indexed"] {
        let report = crate::observability::contention::with_performance(
            metrics.clone(),
            prepare_performance_indexes(
                pool.connect_options().as_ref().clone(),
                PerformanceIndexPreparationRequest {
                    dry_run: true,
                    budget_ms: 2_000,
                },
                test_proxy_sqlite_write_coordinator(),
                CancellationToken::new(),
            ),
        )
        .await
        .unwrap();
        assert!(report.is_complete());
        let snapshot = metrics.render();
        assert!(
            snapshot.lines().any(|line| {
                line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                    && line.contains(&format!("reason=\"{reason}\""))
                    && line.ends_with(" 1")
            }),
            "the explicit inspector must observe the actual {reason} plan"
        );
        let after_scrape = metrics.render();
        let mut before_lines: Vec<_> = snapshot.lines().collect();
        let mut after_lines: Vec<_> = after_scrape.lines().collect();
        before_lines.sort_unstable();
        after_lines.sort_unstable();
        assert_eq!(after_lines, before_lines, "scrape does not inspect plans");
        if reason == "query_plan_fallback" {
            assert!(prepare_fixture_indexes(&pool).await.is_complete());
        }
    }
    pool.close().await;
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
    print_bounded_prompt_cache_plan("missing-index", &missing_plan_details);
    print_bounded_prompt_cache_plan("correlated", &indexed_plan_details);
    assert!(production_sql.contains("WHERE ") && production_sql.contains("= +c.prompt_cache_key"));
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
        Some(1),
        "live MAX(id) must fall back after the newer generation stops matching"
    );
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
    sqlx::query("DELETE FROM codex_invocations WHERE id = 6")
        .execute(&pool)
        .await
        .expect("delete current-generation invocation");
    assert_eq!(
        prompt_cache_last_invocation_id(&pool, "cache").await,
        Some(1),
        "live MAX(id) must fall back after deleting the current generation"
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

async fn insert_compacted_source_recovery_rows(state: &Arc<AppState>, count: usize) -> String {
    let occurred_at = db_occurred_at_lower_bound(Utc::now() - chrono::Duration::days(2));
    for start in (0..count).step_by(256) {
        let end = (start + 256).min(count);
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO codex_invocations (\
                invoke_id, occurred_at, source, status, model, payload, raw_response, input_tokens, output_tokens,\
                cache_input_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read,\
                cost_output, cost_reasoning, failure_kind\
            ) ",
        );
        query.push_values(start..end, |mut values, index| {
            let failed = index % 10 == 0;
            let account_id = if index % 2 == 0 { 7 } else { 9 };
            let payload = format!(
                r#"{{"upstreamAccountId":{account_id},"responseModel":"gpt-test","reasoningEffort":"high"}}"#
            );
            values
                .push_bind(format!("compacted-source-{index}"))
                .push_bind(&occurred_at)
                .push_bind("proxy")
                .push_bind(if failed { "http_500" } else { "success" })
                .push_bind("gpt-test")
                .push_bind(payload)
                .push_bind("{}")
                .push_bind(16_i64)
                .push_bind(3_i64)
                .push_bind(4_i64)
                .push_bind(19_i64)
                .push_bind(1.5_f64)
                .push_bind(0.6_f64)
                .push_bind(0.1_f64)
                .push_bind(0.2_f64)
                .push_bind(0.5_f64)
                .push_bind(0.1_f64)
                .push_bind(if failed {
                    Some("upstream_http_5xx")
                } else {
                    None
                });
        });
        query
            .build()
            .execute(&state.pool)
            .await
            .expect("insert compacted source recovery rows");
    }
    occurred_at
}

#[tokio::test]
async fn compacted_source_recovery_accepts_more_than_10000_rows_and_resumes_after_restart() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = insert_compacted_source_recovery_rows(&state, 10_001).await;

    sqlx::query(
        "INSERT INTO summary_source_change_compaction_proof (
            first_cursor, last_cursor, retained_after_cursor, proof_kind, proof_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(1_i64)
    .bind(10_001_i64)
    .bind(10_002_i64)
    .bind("source_union")
    .bind(r#"{"retainedAfterCursor":10002}"#)
    .execute(&state.pool)
    .await
    .expect("insert compacted source proof");

    run_compacted_source_recovery_one_page_for_test(state.as_ref())
        .await
        .expect("commit the first bounded source recovery page");
    let first_checkpoint = sqlx::query_as::<_, (String, i64, i64, Option<String>)>(
        "SELECT state, processed_rows, cursor_id, cursor_occurred_at
           FROM summary_compacted_source_recovery_checkpoint
          WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read first compacted source recovery checkpoint");
    assert_eq!(first_checkpoint.0, "running");
    assert_eq!(first_checkpoint.1, 512);
    assert!(first_checkpoint.2 > 0);
    assert_eq!(first_checkpoint.3.as_deref(), Some(occurred_at.as_str()));
    let first_total = sqlx::query_scalar::<_, i64>(
        "SELECT total_count
           FROM summary_compacted_source_recovery_aggregate
          WHERE scope = ?1 AND window_name = 'previous7d' AND upstream_account_id = 0",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read staged first page total");
    assert_eq!(first_total, 512);
    let target_live_high_watermark = sqlx::query_scalar::<_, i64>(
        "SELECT target_live_high_watermark_id
           FROM summary_compacted_source_recovery_checkpoint
          WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read recovery target live watermark");
    commit_statistics_recovery_terminal(
        state.as_ref(),
        "compacted-source-live-tail-after-target",
        &db_occurred_at_lower_bound(Utc::now()),
    )
    .await;
    run_compacted_source_recovery_one_page_for_test(state.as_ref())
        .await
        .expect("live-tail changes must not reset committed recovery progress");
    let after_live_tail = sqlx::query_as::<_, (i64, i64)>(
        "SELECT processed_rows, target_live_high_watermark_id
           FROM summary_compacted_source_recovery_checkpoint
          WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read recovery progress after live-tail append");
    assert_eq!(after_live_tail.0, 1_024);
    assert_eq!(after_live_tail.1, target_live_high_watermark);
    let restarted_state =
        crate::tests::test_state_from_existing_pool(state.pool.clone(), state.config.clone(), true)
            .await;
    run_compacted_source_recovery_until_complete_for_test(restarted_state.as_ref())
        .await
        .expect("resume compacted source recovery after the committed page");
    let completed = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT state, processed_rows, cursor_id
           FROM summary_compacted_source_recovery_checkpoint
          WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read completed compacted source recovery checkpoint");
    assert_eq!(completed.0, "completed");
    assert_eq!(completed.1, 10_001);
    assert!(completed.2 > first_checkpoint.2);

    let totals = sqlx::query_as::<_, (i64, i64, i64, i64, f64)>(
        "SELECT total_count, success_count, failure_count, total_tokens, total_cost
           FROM summary_compacted_source_recovery_aggregate
          WHERE scope = ?1 AND window_name = 'previous7d' AND upstream_account_id = 0",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read completed compacted source recovery totals");
    assert_eq!(totals.0, 10_001);
    assert_eq!(totals.1, 9_000);
    assert_eq!(totals.2, 1_001);
    assert_eq!(totals.3, 190_019);
    assert!((totals.4 - 15_001.5).abs() < 1e-6);

    let usage = sqlx::query_as::<_, (i64, i64, i64, f64, f64, f64, f64, f64)>(
        "SELECT cache_write_tokens, cache_read_tokens, output_tokens, cost_input,
                cost_cache_write, cost_cache_read, cost_output, cost_reasoning
           FROM summary_compacted_source_recovery_usage
          WHERE scope = ?1 AND window_name = 'previous7d' AND upstream_account_id = 0
                AND normalized_model = 'gpt-test' AND normalized_reasoning_effort = 'high'",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read completed compacted source recovery usage");
    assert_eq!(usage.0, 120_012);
    assert_eq!(usage.1, 40_004);
    assert_eq!(usage.2, 30_003);
    assert!((usage.3 - 6_000.6).abs() < 1e-6);
    assert!((usage.4 - 1_000.1).abs() < 1e-6);
    assert!((usage.5 - 2_000.2).abs() < 1e-6);
    assert!((usage.6 - 5_000.5).abs() < 1e-6);
    assert!((usage.7 - 1_000.1).abs() < 1e-6);

    let record_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM summary_compacted_source_recovery_record WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("count bounded recovery record identities");
    assert!(
        record_count
            <= i64::try_from(state.config.list_limit_max.max(1) * 6)
                .expect("record budget fits in sqlite integer")
    );

    run_compacted_source_recovery_until_complete_for_test(restarted_state.as_ref())
        .await
        .expect("re-running completed recovery is idempotent");
    let repeated_total = sqlx::query_scalar::<_, i64>(
        "SELECT total_count
           FROM summary_compacted_source_recovery_aggregate
          WHERE scope = ?1 AND window_name = 'previous7d' AND upstream_account_id = 0",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&state.pool)
    .await
    .expect("read repeated compacted source recovery total");
    assert_eq!(repeated_total, totals.0);
    state.pool.close().await;
}
