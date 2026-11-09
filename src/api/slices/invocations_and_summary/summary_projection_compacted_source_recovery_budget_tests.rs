use super::*;
use serde_json::to_string;
use sqlx::Execute;

fn candidate(id: i64, materialized_text_bytes: usize) -> CompactedSourceRecoveryCandidate {
    CompactedSourceRecoveryCandidate {
        id,
        invoke_id: format!("test-{id}"),
        occurred_at: "2026-10-22 12:00:00".to_string(),
        upstream_account_id: None,
        model: None,
        response_model: None,
        reasoning_effort: None,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        total_tokens: None,
        cost: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        status: Some("success".to_string()),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: Some(0),
        estimated_bytes: i64::try_from(192 + materialized_text_bytes).expect("test byte estimate"),
        error_message_bytes: 0,
    }
}

#[test]
fn tuple_cursor_filter_separates_occurred_at_and_id_bindings() {
    let checkpoint = CompactedSourceRecoveryCheckpoint {
        scope: SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE.to_string(),
        format_version: 1,
        time_zone: SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE.to_string(),
        range_start: "2026-10-22 00:00:00".to_string(),
        range_end: "2026-10-29 00:00:00".to_string(),
        window_ranges_json: "[]".to_string(),
        cursor_occurred_at: Some("2026-10-22 12:00:00".to_string()),
        cursor_id: 100,
        target_live_high_watermark_id: 1_000,
        target_rollup_live_cursor: 0,
        target_account_rollup_live_cursor: None,
        target_manifest_high_watermark_id: None,
        target_coverage_revision: 0,
        target_account_coverage_revision: 0,
        target_terminal_sequence: 0,
        list_limit_max: 50,
        processed_rows: 100,
        state: "running".to_string(),
        last_error_code: None,
        last_error_message: None,
    };
    let start = Utc::now();
    let end = start + chrono::Duration::hours(1);
    let mut query =
        sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT id FROM codex_invocations WHERE ");
    push_compacted_source_recovery_filter(
        &mut query,
        &checkpoint,
        &[("previous7d".to_string(), start, end)],
    )
    .expect("build tuple cursor recovery filter");
    let query = query.build();
    assert!(
        query.sql().contains("AND (occurred_at, id) > (?, ?) "),
        "tuple cursor must bind occurred_at and id as separate SQL values: {}",
        query.sql()
    );
}

#[test]
fn candidate_budget_query_does_not_materialize_full_fields() {
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_budget_columns(&mut query);
    let sql = query.build().sql().to_string();
    assert!(sql.starts_with("SELECT id, occurred_at"));
    assert!(sql.contains("AS estimated_bytes"));
    assert!(!sql.contains("AS response_model"));
    assert!(!sql.contains("AS reasoning_effort"));
}

#[test]
fn candidate_materialization_query_reuses_snapshot_budget_metadata() {
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_columns(&mut query);
    let sql = query.build().sql().to_string();
    assert!(sql.contains("NULL AS failure_kind"));
    assert!(sql.contains("0 AS estimated_bytes"));
    assert!(!sql.contains("= 'service_failure'"));
    assert!(!sql.contains("(192 + length"));
}

#[test]
fn recovery_runtime_keeps_adaptive_page_size_across_scheduler_turns() {
    let mut runtime = CompactedSourceRecoveryRuntime::default();
    runtime.halve_page_size();
    runtime.mark_progress();

    assert_eq!(
        runtime.page_size,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE / 2
    );
    assert!(runtime.take_retry_reset());

    runtime.mark_progress();
    assert!(runtime.take_retry_reset());
    assert_eq!(
        runtime.page_size,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE / 2
    );
}

#[test]
fn page_selection_is_capped_at_512_rows() {
    let page = (1..=513).map(|id| candidate(id, 0)).collect();
    let (selected, completed) = select_compacted_source_recovery_candidates(page)
        .expect("select bounded recovery candidates");
    assert_eq!(selected.len(), SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE);
    assert!(!completed);
}

#[test]
fn page_selection_honors_adaptive_row_limit() {
    let page = CompactedSourceRecoveryCandidatePage {
        keys: vec![
            CompactedSourceRecoveryKey {
                id: 1,
                occurred_at: "2026-10-22 12:00:00".to_string(),
            },
            CompactedSourceRecoveryKey {
                id: 2,
                occurred_at: "2026-10-22 12:01:00".to_string(),
            },
        ],
        candidates: vec![candidate(1, 0), candidate(2, 0)],
    };
    let (selected, cursor, completed) = select_compacted_source_recovery_candidate_page(page, 1)
        .expect("select adaptive recovery candidates");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, 1);
    assert_eq!(cursor, Some(("2026-10-22 12:00:00".to_string(), 1)));
    assert!(!completed);
}

#[test]
fn page_selection_keeps_live_boundary_at_cursor() {
    let page = CompactedSourceRecoveryCandidatePage {
        keys: vec![
            CompactedSourceRecoveryKey {
                id: 1,
                occurred_at: "2026-10-22 12:00:00".to_string(),
            },
            CompactedSourceRecoveryKey {
                id: 2,
                occurred_at: "2026-10-22 12:01:00".to_string(),
            },
        ],
        candidates: vec![candidate(1, 0)],
    };
    let (selected, cursor, completed) = select_compacted_source_recovery_candidate_page(page, 512)
        .expect("select recovery page around live boundary");
    assert_eq!(selected.len(), 1);
    assert_eq!(cursor, Some(("2026-10-22 12:00:00".to_string(), 1)));
    assert!(!completed);
}

#[test]
fn page_selection_yields_before_the_8_mib_byte_budget() {
    let row_bytes = SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES / 2;
    let page = vec![candidate(1, row_bytes), candidate(2, row_bytes)];
    let (selected, completed) = select_compacted_source_recovery_candidates(page)
        .expect("select byte-bounded recovery candidates");
    assert_eq!(selected.len(), 1);
    assert!(!completed);
}

#[test]
fn oversized_error_message_is_admitted_as_a_singleton() {
    let page = vec![
        CompactedSourceRecoveryCandidate {
            error_message_bytes: i64::try_from(SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1)
                .expect("test error message byte estimate"),
            ..candidate(1, SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1)
        },
        candidate(2, 1),
    ];
    let (selected, completed) = select_compacted_source_recovery_candidates(page)
        .expect("select oversized source singleton");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, 1);
    assert!(!completed);
}

#[test]
fn oversized_materialized_source_row_fails_closed() {
    let page = vec![candidate(
        1,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1,
    )];
    let error = select_compacted_source_recovery_candidates(page)
        .expect_err("oversized materialized source row should fail closed");
    assert!(
        error
            .to_string()
            .contains("bounded recovery materialization budget")
    );
}

#[test]
fn deferred_recovery_uses_bounded_exponential_backoff() {
    assert_eq!(
        summary_coverage_recovery_retry_delay(Duration::from_secs(10)),
        Duration::from_secs(20)
    );
    assert_eq!(
        summary_coverage_recovery_retry_delay(Duration::from_secs(20)),
        Duration::from_secs(40)
    );
    assert_eq!(
        summary_coverage_recovery_retry_delay(Duration::from_secs(40)),
        Duration::from_secs(60)
    );
    assert_eq!(
        summary_coverage_recovery_retry_delay(Duration::from_secs(60)),
        Duration::from_secs(60)
    );
    assert_eq!(
        summary_coverage_recovery_scheduler_delay(
            SummaryCoverageRecoveryNextTurn::Deferred,
            Duration::from_secs(20),
        ),
        Duration::from_secs(20)
    );
    assert_eq!(
        summary_coverage_recovery_scheduler_delay(
            SummaryCoverageRecoveryNextTurn::Immediate,
            Duration::from_secs(20),
        ),
        Duration::from_secs(1)
    );
}

#[test]
fn live_tail_advancement_does_not_invalidate_recovery_coverage() {
    let coverage = SummaryProjectionGenerationFence {
        live_high_watermark_id: 10,
        rollup_live_cursor: 20,
        account_rollup_live_cursor: Some(30),
        completed_manifest_high_watermark_id: Some(40),
        coverage_revision: 50,
        account_coverage_revision: 60,
        durable_terminal_sequence_watermark: 70,
    };
    let live_tail_advanced = SummaryProjectionGenerationFence {
        live_high_watermark_id: 11,
        rollup_live_cursor: 21,
        account_rollup_live_cursor: Some(31),
        completed_manifest_high_watermark_id: Some(40),
        coverage_revision: 50,
        account_coverage_revision: 60,
        durable_terminal_sequence_watermark: 71,
    };
    assert!(coverage.coverage_sources_match(live_tail_advanced));
    assert_ne!(coverage, live_tail_advanced);
}

#[test]
fn recovery_snapshot_window_ranges_expire_at_the_next_reporting_day() {
    let before_midnight = DateTime::parse_from_rfc3339("2026-10-24T15:59:30Z")
        .expect("parse pre-midnight test time")
        .with_timezone(&Utc);
    let stored = to_string(&compacted_source_recovery_windows_at(before_midnight).unwrap())
        .expect("serialize recovery window ranges");

    assert!(
        compacted_source_recovery_window_ranges_match_at(&stored, before_midnight)
            .expect("match same-day recovery window ranges")
    );
    assert!(
        compacted_source_recovery_window_ranges_match_at(
            &stored,
            before_midnight + ChronoDuration::seconds(20),
        )
        .expect("match same-day recovery window ranges after time advances")
    );
    assert!(
        !compacted_source_recovery_window_ranges_match_at(
            &stored,
            before_midnight + ChronoDuration::minutes(2),
        )
        .expect("match next-day recovery window ranges")
    );
}

#[test]
fn recovery_states_have_stable_unavailable_codes() {
    assert_eq!(
        SummaryCompactedSourceRecoveryState::Pending.unavailable_code(),
        super::super::summary_recovery::SUMMARY_RECOVERY_PENDING_CODE
    );
    assert_eq!(
        SummaryCompactedSourceRecoveryState::Deferred.unavailable_code(),
        super::super::summary_recovery::SUMMARY_RECOVERY_PENDING_CODE
    );
    assert_eq!(
        SummaryCompactedSourceRecoveryState::Failed.unavailable_code(),
        SUMMARY_RECOVERY_FAILED_CODE
    );
}
