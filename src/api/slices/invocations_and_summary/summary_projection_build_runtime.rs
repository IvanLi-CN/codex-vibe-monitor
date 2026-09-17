struct SummaryProjectionRuntimeOverlayInput<'a> {
    state: &'a AppState,
    mode: SummaryProjectionBuildMode,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    current_records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: &'a mut usize,
    current_record_bytes: &'a mut usize,
    unavailable_exact_live_buckets: &'a mut BTreeSet<i64>,
    current_source_unavailable: &'a mut bool,
}

fn apply_summary_projection_runtime_overlay(
    mut input: SummaryProjectionRuntimeOverlayInput<'_>,
) -> Result<()> {
    info!(
        ?input.mode,
        stage = "runtime_overlay",
        "summary projection build stage started"
    );
    for runtime_record in input.state.proxy_runtime_invocations.snapshot() {
        apply_summary_projection_runtime_record(runtime_record, &mut input)?;
    }
    info!(
        ?input.mode,
        stage = "runtime_overlay",
        "summary projection build stage completed"
    );
    Ok(())
}

fn apply_summary_projection_runtime_record(
    runtime_record: ApiInvocation,
    input: &mut SummaryProjectionRuntimeOverlayInput<'_>,
) -> Result<()> {
    let Some(row) = runtime_upstream_account_activity_preview_row_with_terminal(
        runtime_record,
        InvocationSourceScope::All,
        true,
    ) else {
        return Ok(());
    };
    let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
        return Ok(());
    };
    let is_new_rolling_record = !summary_projection_records_contains_identity(
        input.records_by_invoke_id,
        row.id,
        &row.invoke_id,
        &row.occurred_at,
    );
    if is_new_rolling_record
        && input.records_by_invoke_id.len() >= summary_projection_exact_record_limit()
    {
        let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        input.unavailable_exact_live_buckets.insert(bucket);
        return Ok(());
    }
    let record = SummaryProjectionRecord {
        row,
        occurred_at,
        global_rollup_covered: false,
        account_rollup_covered: false,
        usage_global_rollup_covered: false,
        usage_account_rollup_covered: false,
        is_persisted_live_record: false,
        is_archive_record: false,
        archive_has_materialized_rollups: false,
        account_archive_totals_fallback_included: false,
    };
    let replacement_bytes = summary_projection_preview_row_bytes(&record.row);
    let replaced_rolling_bytes = summary_projection_record_bytes_for_identity(
        input.records_by_invoke_id,
        record.row.id,
        &record.row.invoke_id,
        &record.row.occurred_at,
    );
    let next_rolling_record_bytes = input
        .exact_record_bytes
        .saturating_sub(replaced_rolling_bytes)
        .saturating_add(replacement_bytes);
    let replaced_bytes = summary_projection_record_bytes_for_identity(
        input.current_records_by_invoke_id,
        record.row.id,
        &record.row.invoke_id,
        &record.row.occurred_at,
    );
    let next_current_record_bytes = input
        .current_record_bytes
        .saturating_sub(replaced_bytes)
        .saturating_add(replacement_bytes);
    if let Err(error) = ensure_summary_projection_resident_record_bytes(
        next_rolling_record_bytes,
        next_current_record_bytes,
    ) {
        if !summary_projection_resident_record_budget_exceeded(&error) {
            return Err(error);
        }
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        summary_projection_mark_unavailable_archive_ranges(
            input.unavailable_exact_live_buckets,
            [ExactUtcRange {
                start: Utc
                    .timestamp_opt(bucket, 0)
                    .single()
                    .ok_or_else(|| anyhow!("invalid runtime overflow bucket start"))?,
                end: Utc
                    .timestamp_opt(bucket.saturating_add(3_600), 0)
                    .single()
                    .ok_or_else(|| anyhow!("invalid runtime overflow bucket end"))?,
            }],
        )?;
        *input.current_source_unavailable = true;
        return Ok(());
    }
    *input.exact_record_bytes = next_rolling_record_bytes;
    *input.current_record_bytes = next_current_record_bytes;
    let record_key = summary_projection_record_insert_key(input.records_by_invoke_id, &record);
    input
        .records_by_invoke_id
        .insert(record_key, record.clone());
    let current_record_key =
        summary_projection_record_insert_key(input.current_records_by_invoke_id, &record);
    input
        .current_records_by_invoke_id
        .insert(current_record_key, record);
    Ok(())
}
