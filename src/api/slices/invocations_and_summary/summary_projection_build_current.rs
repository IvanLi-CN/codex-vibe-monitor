struct SummaryProjectionCurrentAdmissionInput<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    end: DateTime<Utc>,
    live_high_watermark_id: i64,
    live_preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
}

async fn admit_summary_projection_current_rows(
    input: SummaryProjectionCurrentAdmissionInput<'_>,
) -> Result<SummaryProjectionLiveAdmission> {
    if input.state.config.list_limit_max > SUMMARY_PROJECTION_MAX_EXACT_RECORDS {
        return Err(anyhow!(
            "summary projection current limit exceeds bounded record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    let recent_candidate_limit = input.state.config.list_limit_max.saturating_add(1);
    info!(
        ?input.mode,
        stage = "current_index_admission",
        "summary projection build stage started"
    );
    let admission =
        query_summary_projection_live_rows_with_budget(SummaryProjectionLiveRowsQuery {
            pool: input.pool,
            source_scope: InvocationSourceScope::All,
            range: ExactUtcRange {
                start: Utc
                    .timestamp_opt(0, 0)
                    .single()
                    .ok_or_else(|| anyhow!("invalid Unix epoch"))?,
                end: input.end,
            },
            high_watermark_id: input.live_high_watermark_id,
            min_id_exclusive: None,
            upstream_account_id: None,
            limit: recent_candidate_limit,
            in_progress_only: false,
            preview_cache: input.live_preview_cache,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "summary_projection",
                builder: "bounded_recent_index",
                purpose: "summary_projection_recent_index",
            },
        })
        .await?;
    info!(
        ?input.mode,
        stage = "current_index_admission",
        elapsed_ms = input.build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(admission)
}

struct SummaryProjectionCurrentIndex {
    current_records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_record_bytes: usize,
    current_source_unavailable_from_rank: Option<usize>,
    current_account_source_unavailable_from_rank: HashMap<i64, usize>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
}

struct SummaryProjectionCurrentIndexInput<'a> {
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    records_by_invoke_id: &'a HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: usize,
    admission: SummaryProjectionLiveAdmission,
}

struct SummaryProjectionCurrentRowsInput<'a> {
    rows: Vec<UpstreamAccountInvocationPreviewRow>,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    records_by_invoke_id: &'a HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: usize,
    current_records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    current_record_bytes: &'a mut usize,
    current_source_unavailable_from_rank: &'a mut Option<usize>,
    current_account_source_unavailable_from_rank: &'a mut HashMap<i64, usize>,
}

fn materialize_summary_projection_current_rows(
    input: SummaryProjectionCurrentRowsInput<'_>,
) -> Result<()> {
    let mut current_account_ranks = HashMap::<Option<i64>, usize>::new();
    for (global_index, row) in input.rows.into_iter().enumerate() {
        let account_rank = current_account_ranks
            .entry(row.upstream_account_id)
            .or_default();
        *account_rank += 1;
        if let Some((_invoke_id, record)) = summary_projection_live_record_from_preview(
            row,
            input.rollup_live_cursor,
            input.account_rollup_live_cursor,
            input.hourly_rollup_totals,
            input.hourly_rollup_usage,
        ) {
            let replacement_bytes = summary_projection_preview_row_bytes(&record.row);
            let replaced_bytes = summary_projection_record_bytes_for_identity(
                input.records_by_invoke_id,
                record.row.id,
                &record.row.invoke_id,
                &record.row.occurred_at,
            );
            let next_current_record_bytes = input
                .current_record_bytes
                .saturating_sub(replaced_bytes)
                .saturating_add(replacement_bytes);
            if let Err(error) = ensure_summary_projection_resident_record_bytes(
                input.exact_record_bytes,
                next_current_record_bytes,
            ) {
                if !summary_projection_resident_record_budget_exceeded(&error) {
                    return Err(error);
                }
                let global_rank = global_index + 1;
                *input.current_source_unavailable_from_rank = Some(
                    input
                        .current_source_unavailable_from_rank
                        .map(|rank| rank.min(global_rank))
                        .unwrap_or(global_rank),
                );
                if let Some(account_id) = record.row.upstream_account_id.filter(|id| *id > 0) {
                    input
                        .current_account_source_unavailable_from_rank
                        .entry(account_id)
                        .and_modify(|rank| *rank = (*rank).min(*account_rank))
                        .or_insert(*account_rank);
                }
                continue;
            }
            *input.current_record_bytes = next_current_record_bytes;
            let record_key =
                summary_projection_record_insert_key(input.current_records_by_invoke_id, &record);
            input
                .current_records_by_invoke_id
                .insert(record_key, record);
        }
    }
    Ok(())
}

fn materialize_summary_projection_current_index(
    input: SummaryProjectionCurrentIndexInput<'_>,
) -> Result<SummaryProjectionCurrentIndex> {
    let SummaryProjectionCurrentIndexInput {
        mode,
        build_started_at,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        records_by_invoke_id,
        exact_record_bytes,
        admission,
    } = input;
    let recent_live_admission_gaps = admission.gaps;
    let recent_live_admission_overflow = admission.overflow;
    let mut current_source_unavailable_from_rank = None::<usize>;
    let mut current_account_source_unavailable_from_rank = HashMap::<i64, usize>::new();
    for candidate in recent_live_admission_gaps
        .iter()
        .chain(recent_live_admission_overflow.iter())
    {
        current_source_unavailable_from_rank = Some(
            current_source_unavailable_from_rank
                .map(|rank| rank.min(candidate.global_rank))
                .unwrap_or(candidate.global_rank),
        );
        if let Some(account_id) = candidate
            .upstream_account_id
            .filter(|account_id| *account_id > 0)
        {
            current_account_source_unavailable_from_rank
                .entry(account_id)
                .and_modify(|rank| *rank = (*rank).min(candidate.account_rank))
                .or_insert(candidate.account_rank);
        }
    }
    let recent_index_complete = recent_live_admission_overflow.is_none();
    let recent_index_overflow_at = recent_live_admission_overflow
        .as_ref()
        .map(|candidate| {
            parse_to_utc_datetime(&candidate.occurred_at).ok_or_else(|| {
                anyhow!("summary projection recent index overflow had an invalid occurred_at")
            })
        })
        .transpose()?;
    let mut recent_rows = admission.rows;
    recent_rows.truncate(summary_projection_exact_record_limit());
    let mut current_records_by_invoke_id = HashMap::<String, SummaryProjectionRecord>::new();
    let mut current_record_bytes = 0_usize;
    materialize_summary_projection_current_rows(SummaryProjectionCurrentRowsInput {
        rows: recent_rows,
        rollup_live_cursor,
        account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        records_by_invoke_id,
        exact_record_bytes,
        current_records_by_invoke_id: &mut current_records_by_invoke_id,
        current_record_bytes: &mut current_record_bytes,
        current_source_unavailable_from_rank: &mut current_source_unavailable_from_rank,
        current_account_source_unavailable_from_rank:
            &mut current_account_source_unavailable_from_rank,
    })?;
    if current_records_by_invoke_id.len() > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection current index exceeded bounded record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    info!(
        ?mode,
        stage = "current_index_materialization",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection current index materialized"
    );
    Ok(SummaryProjectionCurrentIndex {
        current_records_by_invoke_id,
        current_record_bytes,
        current_source_unavailable_from_rank,
        current_account_source_unavailable_from_rank,
        recent_index_complete,
        recent_index_overflow_at,
    })
}
