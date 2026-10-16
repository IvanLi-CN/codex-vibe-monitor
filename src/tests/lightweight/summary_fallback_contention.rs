use super::*;
use crate::sqlite_batch_writer::{
    BatchedTerminalInvocationWrite, SqliteBatchWrite, SqliteBatchWriter,
};
use std::cell::Cell;

#[tokio::test]
async fn summary_projection_bootstrap_fallback_reopens_only_for_coverage_changes() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(false).await;
}

#[tokio::test]
async fn summary_projection_bootstrap_fallback_recovers_compacted_live_tail() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(true).await;
}

async fn assert_bootstrap_fallback_reopens_only_for_coverage_changes(compact: bool) {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-bootstrap-fence', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .bind(historical_at)
    .execute(&state.pool)
    .await
    .expect("seed historical source");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create Bootstrap interleave gate");

    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeHistoricalLiveCoverage,
    );
    let initial_state = state.clone();
    let initial = tokio::spawn(async move {
        hydrate_summary_snapshots_with_deadline(initial_state.as_ref(), Duration::from_secs(1))
            .await
    });
    tokio::pin!(initial);
    tokio::select! {
        _ = interleave.wait_for_writer() => {}
        result = &mut initial => {
            clear_summary_projection_test_interleave();
            result.expect("join Bootstrap fallback").expect("Bootstrap fallback");
            panic!("Bootstrap must reach the deterministic timeout stage");
        }
    }
    initial
        .await
        .expect("join Bootstrap fallback")
        .expect("Bootstrap fallback");
    clear_summary_projection_test_interleave();

    let (first_result, first_hydrations) = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
        .scope(Cell::new(0), async {
            let result = refresh_summary_snapshots(state.as_ref()).await;
            let calls = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
                .try_with(Cell::get)
                .expect("boundary hydration counter scope");
            (result, calls)
        })
        .await;
    first_result.expect("unchanged fallback fence refresh");
    assert_eq!(
        first_hydrations, 0,
        "unchanged fallback fence must skip hydration"
    );

    // A normal terminal commits its source descriptor in the same transaction. A raw SQL
    // INSERT deliberately lacks that proof and must remain unavailable during tail recovery.
    let mut record = crate::tests::test_proxy_capture_record(
        "summary-bootstrap-fence-new",
        &db_occurred_at_lower_bound(Utc::now()),
    );
    record.payload = Some("{}".to_string());
    record.usage.input_tokens = Some(16);
    record.usage.cache_input_tokens = Some(0);
    record.usage.total_tokens = Some(19);
    record.cost = Some(1.5);
    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::TerminalInvocation(
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
            },
        )],
    )
    .await;
    if compact {
        let cursor = crate::summary_source_change::summary_source_change_cursor(&state.pool)
            .await
            .expect("load committed source descriptor cursor");
        assert!(cursor > 0, "terminal writer must persist its descriptor");
        assert!(
            crate::summary_source_change::compact_summary_source_change_journal(
                &state.pool,
                cursor,
                cursor + 1,
            )
            .await
            .expect("compact committed descriptors with a durable proof")
        );
    }
    let (changed_result, changed_hydrations) =
        SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
            .scope(Cell::new(0), async {
                let result = refresh_summary_snapshots(state.as_ref()).await;
                let calls = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
                    .try_with(Cell::get)
                    .expect("boundary hydration counter scope");
                (result, calls)
            })
            .await;
    changed_result.expect("changed source fence refresh");
    assert_eq!(
        changed_hydrations, 0,
        "a live terminal must use bounded reconciliation while historical proof is unchanged"
    );
    let Json(recent) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("new live terminal remains exact beside a historical fallback gap");
    assert_eq!(recent.total_count, 1);
    assert_eq!(recent.total_tokens, 19);
    let historical = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(historical, Err(ApiError::Unavailable(_))));

    sqlx::query("UPDATE summary_coverage_revision SET revision = revision + 1 WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("advance historical authority independently of live terminals");
    let (coverage_result, coverage_hydrations) =
        SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
            .scope(Cell::new(0), async {
                let result = refresh_summary_snapshots(state.as_ref()).await;
                let calls = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
                    .try_with(Cell::get)
                    .expect("boundary hydration counter scope");
                (result, calls)
            })
            .await;
    coverage_result.expect("changed coverage fence refresh");
    assert_eq!(coverage_hydrations, 1);
    state.pool.close().await;
}
