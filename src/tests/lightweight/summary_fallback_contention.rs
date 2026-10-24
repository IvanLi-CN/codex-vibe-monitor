use super::*;
use crate::sqlite_batch_writer::{
    BatchedTerminalInvocationWrite, SqliteBatchWrite, SqliteBatchWriter,
};
use crate::summary_source_change::{
    SummarySourceChangeDescriptor, SummarySourceChangeEntry,
    append_summary_source_change_descriptor_tx,
};
use std::cell::Cell;

#[derive(Clone, Copy)]
enum TailRecoveryFixture {
    Descriptors,
    Compacted,
    ConcurrentTerminal,
    AdmissionBusy,
}

#[tokio::test]
async fn summary_projection_bootstrap_fallback_reopens_only_for_coverage_changes() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(TailRecoveryFixture::Descriptors)
        .await;
}

#[tokio::test]
async fn summary_projection_compaction_stays_pending_until_staged_recovery_publishes() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(TailRecoveryFixture::Compacted)
        .await;
}

#[tokio::test]
async fn summary_projection_live_tail_does_not_checkpoint_beyond_reconstructed_source() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(
        TailRecoveryFixture::ConcurrentTerminal,
    )
    .await;
}

#[tokio::test]
async fn summary_projection_live_tail_yields_publication_to_terminal_admission() {
    assert_bootstrap_fallback_reopens_only_for_coverage_changes(TailRecoveryFixture::AdmissionBusy)
        .await;
}

async fn assert_bootstrap_fallback_reopens_only_for_coverage_changes(fixture: TailRecoveryFixture) {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let historical_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::days(3));
    // Keep the source row and durable descriptor atomic, but leave the derived live cursor behind
    // the source row so Bootstrap still has a historical live-coverage gap to reconcile.
    commit_fixture_historical_source_at(&state, "summary-bootstrap-fence", &historical_at).await;
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

    // A normal terminal changes the source fence by committing its descriptor in the same
    // transaction as the live source row.
    commit_fixture_terminal(state.as_ref(), "summary-bootstrap-fence-new").await;
    if matches!(fixture, TailRecoveryFixture::Compacted) {
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
    let mut concurrent_hydrations = 0;
    if matches!(fixture, TailRecoveryFixture::AdmissionBusy) {
        let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
        let terminal = coordinator
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
            .await;
        let (result, hydrations) = tokio::time::timeout(
            Duration::from_secs(2),
            refresh_with_hydration_count(state.as_ref()),
        )
        .await
        .expect("publication must defer rather than queue behind P1");
        result.expect("busy publication retains its reconciliation obligation");
        assert_eq!(hydrations, 0);
        let checkpoint = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
            .await
            .expect("deferred reconciliation checkpoint")
            .expect("durable reconstruction remains pending");
        assert_eq!(checkpoint.next_source_cursor, 0);
        assert_ne!(checkpoint.state, "ready");
        drop(terminal);
    }
    if matches!(fixture, TailRecoveryFixture::ConcurrentTerminal) {
        let interleave = install_summary_projection_test_interleave_at(
            SummaryProjectionTestInterleaveStage::BeforeLiveTailPublication,
        );
        let refresh_state = state.clone();
        let mut refresh =
            tokio::spawn(async move { refresh_with_hydration_count(refresh_state.as_ref()).await });
        tokio::select! {
            _ = interleave.wait_for_writer() => {}
            result = &mut refresh => {
                clear_summary_projection_test_interleave();
                result.expect("join tail reconstruction").0.expect("tail reconstruction");
                panic!("tail reconstruction must pause before publication");
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                clear_summary_projection_test_interleave();
                refresh.abort();
                panic!("tail reconstruction did not reach publication within its budget");
            }
        }
        // Leave derived work pending: this race must recover from the P1 source fence
        // even when the hourly-rollup cursor has not moved to force another replay.
        let rollup_cursor = crate::load_hourly_rollup_live_progress(
            &state.pool,
            crate::HOURLY_ROLLUP_DATASET_INVOCATIONS,
        )
        .await
        .expect("read derived cursor before the concurrent terminal");
        let writer = SqliteBatchWriter::spawn_for_test();
        assert!(
            writer
                .enqueue_terminal(fixture_terminal("summary-bootstrap-fence-concurrent"))
                .enqueued
        );
        writer.flush_buffered_for_test(&state.pool).await;
        assert_eq!(
            crate::load_hourly_rollup_live_progress(
                &state.pool,
                crate::HOURLY_ROLLUP_DATASET_INVOCATIONS,
            )
            .await
            .expect("read derived cursor after the concurrent terminal"),
            rollup_cursor,
            "P1 must commit while P2 remains pending"
        );
        interleave.resume_build();
        let (result, hydrations) = refresh.await.expect("join stale target refresh");
        result.expect("defer stale target without losing its obligation");
        concurrent_hydrations = hydrations;
        clear_summary_projection_test_interleave();
        let checkpoint = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
            .await
            .expect("read deferred reconciliation checkpoint")
            .expect("reconstruction created a checkpoint");
        assert_eq!(checkpoint.next_source_cursor, 0);
        assert_ne!(checkpoint.state, "ready");
    }
    let (changed_result, changed_hydrations) = refresh_with_hydration_count(state.as_ref()).await;
    changed_result.expect("changed source fence refresh");
    assert_eq!(
        changed_hydrations + concurrent_hydrations,
        0,
        "a live terminal must use bounded reconciliation while historical proof is unchanged"
    );
    if matches!(fixture, TailRecoveryFixture::Compacted) {
        let pending = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("today".to_string()),
                limit: None,
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await;
        assert!(
            matches!(pending, Err(ApiError::Unavailable(_))),
            "compacted dashboard source must remain unavailable until staged recovery publishes"
        );
        run_compacted_source_recovery_until_complete_for_test(state.as_ref())
            .await
            .expect("complete staged compacted source recovery");
        let recovery_checkpoint =
            sqlx::query_as::<_, (String, Option<String>, i64, i64, i64, i64, i64)>(
                "SELECT state, cursor_occurred_at, cursor_id, processed_rows,\
                    target_live_high_watermark_id, target_coverage_revision,\
                    target_account_coverage_revision\
               FROM summary_compacted_source_recovery_checkpoint\
              WHERE scope = 'dashboard'",
            )
            .fetch_optional(&state.pool)
            .await
            .expect("read completed compacted source recovery checkpoint");
        let recovery_snapshot = state
            .subscription_hub
            .summary_dashboard_recovery()
            .await
            .map(|snapshot| {
                (
                    snapshot.time_zone.clone(),
                    snapshot.window_ranges_json.clone(),
                    snapshot.generation_fence,
                    snapshot.entries.len(),
                    snapshot.bounded_record_count,
                )
            });
        let SummaryRollingDeltaSnapshot {
            projection: rolling_projection,
            entries: rolling_deltas,
            gaps: rolling_gaps,
        } = state
            .subscription_hub
            .summary_projection_with_rolling_delta()
            .await
            .expect("read rolling summary state after recovery");
        let recovery_status = summary_source_gap_status(
            rolling_projection.as_ref(),
            &rolling_gaps,
            &rolling_deltas,
            &SummaryWindow::Calendar("today".to_string()),
            "Asia/Shanghai"
                .parse()
                .expect("parse Shanghai test timezone"),
            None,
        );
        println!(
            "compacted recovery diagnostics: checkpoint={recovery_checkpoint:?} snapshot={recovery_snapshot:?} status={recovery_status:?}"
        );
        let Json(recent) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("today".to_string()),
                limit: None,
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("published staged recovery remains exact");
        assert_eq!(recent.total_count, 1);
        assert_eq!(recent.total_tokens, 19);
    } else {
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
        let expected_count = if matches!(fixture, TailRecoveryFixture::ConcurrentTerminal) {
            2
        } else {
            1
        };
        assert_eq!(recent.total_count, expected_count);
        assert_eq!(recent.total_tokens, expected_count * 19);
    }
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
    let (coverage_result, coverage_hydrations) = refresh_with_hydration_count(state.as_ref()).await;
    coverage_result.expect("changed coverage fence refresh");
    assert_eq!(coverage_hydrations, 1);
    state.pool.close().await;
}

async fn refresh_with_hydration_count(state: &AppState) -> (Result<()>, usize) {
    SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
        .scope(Cell::new(0), async {
            let result = refresh_summary_snapshots(state).await;
            let calls = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS
                .try_with(Cell::get)
                .expect("boundary hydration counter scope");
            (result, calls)
        })
        .await
}

async fn commit_fixture_terminal(state: &AppState, invoke_id: &str) {
    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::TerminalInvocation(fixture_terminal(
            invoke_id,
        ))],
    )
    .await;
}

async fn commit_fixture_historical_source_at(state: &AppState, invoke_id: &str, occurred_at: &str) {
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin historical source fixture transaction");
    let record = fixture_terminal_at(invoke_id, occurred_at).record;
    let persisted = crate::persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, false)
        .await
        .expect("persist historical source fixture without derived rollups")
        .expect("historical source fixture must persist an invocation");
    let descriptor = SummarySourceChangeDescriptor::terminal_batch(
        0,
        vec![SummarySourceChangeEntry {
            row_id: persisted.id,
            invoke_id: persisted.invoke_id,
            occurred_at: persisted.occurred_at,
            upstream_account_id: None,
            current_rank: None,
        }],
    )
    .expect("build historical source fixture descriptor");
    append_summary_source_change_descriptor_tx(tx.as_mut(), &descriptor)
        .await
        .expect("persist historical source fixture descriptor");
    tx.commit()
        .await
        .expect("commit historical source fixture transaction");
    assert_eq!(
        crate::load_hourly_rollup_live_progress(
            &state.pool,
            crate::HOURLY_ROLLUP_DATASET_INVOCATIONS,
        )
        .await
        .expect("load historical source fixture live cursor"),
        0,
        "historical source fixture must not advance the derived live cursor"
    );
}

fn fixture_terminal(invoke_id: &str) -> BatchedTerminalInvocationWrite {
    let occurred_at = db_occurred_at_lower_bound(Utc::now());
    fixture_terminal_at(invoke_id, &occurred_at)
}

fn fixture_terminal_at(invoke_id: &str, occurred_at: &str) -> BatchedTerminalInvocationWrite {
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
