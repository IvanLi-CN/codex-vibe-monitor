use super::*;

pub(crate) async fn run_compacted_source_recovery_one_page_for_test(
    state: &AppState,
) -> Result<()> {
    ensure_compacted_source_recovery(state).await?;
    update_compacted_source_recovery_state(
        state,
        SummaryCompactedSourceRecoveryState::Running,
        None,
        None,
    )
    .await?;
    let checkpoint = load_compacted_source_recovery_checkpoint(&state.pool)
        .await?
        .ok_or_else(|| anyhow!("test recovery checkpoint was not initialized"))?;
    let windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?;
    let candidates = load_compacted_source_recovery_candidates(
        state,
        &checkpoint,
        &windows,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET,
    )
    .await?
    .ok_or_else(|| anyhow!("test recovery candidate read exceeded its budget"))?;
    let (selected_candidates, cursor, completed) = select_compacted_source_recovery_candidate_page(
        candidates,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    if selected_candidates.is_empty() && cursor.is_none() && !completed {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(());
    }
    let selected_rows = selected_candidates
        .into_iter()
        .map(CompactedSourceRecoveryCandidate::into_source_row)
        .collect::<Vec<_>>();
    match stage_compacted_source_recovery_page(
        state,
        &checkpoint,
        &selected_rows,
        cursor.as_ref(),
        &windows,
        completed,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET,
    )
    .await?
    {
        CompactedSourceRecoveryPageOutcome::Progress { .. } => Ok(()),
        CompactedSourceRecoveryPageOutcome::Reset => {
            bail!("test recovery page observed a generation fence reset")
        }
        CompactedSourceRecoveryPageOutcome::Stale => {
            bail!("test recovery page observed a stale checkpoint")
        }
    }
}

#[cfg(test)]
pub(crate) async fn explain_compacted_source_recovery_key_page_for_test(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<String>> {
    let parse_time = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("parse test recovery plan time {value}"))
            .map(|parsed| parsed.with_timezone(&Utc))
    };
    let checkpoint = CompactedSourceRecoveryCheckpoint {
        scope: SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE.to_string(),
        format_version: 1,
        time_zone: SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE.to_string(),
        range_start: "2026-10-01T00:00:00Z".to_string(),
        range_end: "2026-10-08T00:00:00Z".to_string(),
        window_ranges_json: "[]".to_string(),
        cursor_occurred_at: Some("2026-10-02T00:00:00Z".to_string()),
        cursor_id: 100,
        target_live_high_watermark_id: 100_001,
        target_rollup_live_cursor: 0,
        target_account_rollup_live_cursor: None,
        target_manifest_high_watermark_id: None,
        target_coverage_revision: 0,
        target_account_coverage_revision: 0,
        target_terminal_sequence: 0,
        list_limit_max: 512,
        processed_rows: 0,
        state: SummaryCompactedSourceRecoveryState::Running
            .as_str()
            .to_string(),
        last_error_code: None,
        last_error_message: None,
    };
    let windows = vec![
        (
            "today".to_string(),
            parse_time("2026-10-01T00:00:00Z")?,
            parse_time("2026-10-05T00:00:00Z")?,
        ),
        (
            "yesterday".to_string(),
            parse_time("2026-10-02T00:00:00Z")?,
            parse_time("2026-10-06T00:00:00Z")?,
        ),
        (
            "previous7d".to_string(),
            parse_time("2026-10-01T00:00:00Z")?,
            parse_time("2026-10-08T00:00:00Z")?,
        ),
    ];
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("EXPLAIN QUERY PLAN ");
    push_compacted_source_recovery_key_page_query(
        &mut query,
        &checkpoint,
        &windows,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    let plan: Vec<(i64, i64, i64, String)> = query.build_query_as().fetch_all(pool).await?;
    Ok(plan.into_iter().map(|(_, _, _, detail)| detail).collect())
}

#[cfg(test)]
pub(crate) async fn run_compacted_source_recovery_until_complete_for_test(
    state: &AppState,
) -> Result<()> {
    ensure_compacted_source_recovery(state).await?;
    let mut runtime = CompactedSourceRecoveryRuntime::default();
    for _ in 0..128 {
        let Some(turn) =
            run_compacted_source_recovery_turn_with_runtime(state, &mut runtime).await?
        else {
            let checkpoint = load_compacted_source_recovery_checkpoint(&state.pool).await?;
            if checkpoint.as_ref().is_some_and(|row| {
                row.state == SummaryCompactedSourceRecoveryState::Failed.as_str()
            }) {
                bail!("test compacted source recovery entered failed state")
            }
            return Ok(());
        };
        if turn.next_turn == SummaryCoverageRecoveryNextTurn::Idle {
            break;
        }
    }
    bail!("test compacted source recovery did not complete within 128 turns")
}
