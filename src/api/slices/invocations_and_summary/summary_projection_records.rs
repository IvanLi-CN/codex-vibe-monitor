fn summary_projection_all_time_sequence_watermark(
    all_time_was_fully_rebuilt: bool,
    all_time_terminal_coverage_complete: bool,
    durable_terminal_sequence_watermark: u64,
    previous_all_time_terminal_sequence_watermark: u64,
) -> u64 {
    if all_time_was_fully_rebuilt && all_time_terminal_coverage_complete {
        durable_terminal_sequence_watermark
    } else {
        previous_all_time_terminal_sequence_watermark
    }
}

fn summary_projection_all_time_scope_covered(
    global_coverage_complete: bool,
    terminal_sequence_watermark: u64,
    terminal_sequence: u64,
    identity_covered: bool,
) -> bool {
    global_coverage_complete
        && (terminal_sequence_watermark >= terminal_sequence || identity_covered)
}

fn runtime_record_is_success_for_summary_row(row: &UpstreamAccountInvocationPreviewRow) -> bool {
    let status = normalized_runtime_text(Some(row.status.as_str()));
    matches!(
        status.as_str(),
        "success" | "completed" | INVOCATION_STATUS_WARNING_SUCCESS
    ) || (status == "http_200" && normalized_runtime_text(row.error_message.as_deref()).is_empty())
}

fn summary_projection_record_totals(record: &SummaryProjectionRecord) -> StatsTotals {
    summary_projection_record_totals_with_archived_pending_terminal(record, false)
}

fn summary_projection_all_time_record_totals(record: &SummaryProjectionRecord) -> StatsTotals {
    // Completed archive batches can contain legacy rows whose status stayed `pending` after the
    // terminal writer failed. The all-time rollup contract treats those retained historical rows
    // as terminal; their normal failure classification still determines non-success totals.
    summary_projection_record_totals_with_archived_pending_terminal(record, true)
}

fn summary_projection_all_time_uses_global_exact_record(
    record: &SummaryProjectionRecord,
    rollup_live_cursor: i64,
) -> bool {
    (record.is_persisted_live_record || record.archive_has_materialized_rollups)
        && !record.global_rollup_covered
        // The bounded global live-tail aggregate already owns rows after its cursor.
        && !(record.is_persisted_live_record && record.row.id > rollup_live_cursor)
}

fn summary_projection_all_time_uses_account_exact_record(
    record: &SummaryProjectionRecord,
    account_rollup_live_cursor: Option<i64>,
) -> bool {
    (record.is_persisted_live_record || record.archive_has_materialized_rollups)
        && !record.account_rollup_covered
        // The bounded account live-tail aggregate already owns rows after its cursor. An absent
        // cursor deliberately leaves the exact retained record visible instead of undercounting.
        && !(record.is_persisted_live_record
            && account_rollup_live_cursor.is_some_and(|cursor| record.row.id > cursor))
}

fn summary_projection_record_totals_with_archived_pending_terminal(
    record: &SummaryProjectionRecord,
    archived_pending_is_terminal: bool,
) -> StatsTotals {
    let classification = resolve_failure_classification(
        Some(record.row.status.as_str()),
        record.row.error_message.as_deref(),
        record.row.failure_kind.as_deref(),
        record.row.failure_class.as_deref(),
        record.row.is_actionable,
    );
    let status = normalized_runtime_text(Some(record.row.status.as_str()));
    let terminal = !matches!(status.as_str(), "running" | "pending")
        || (archived_pending_is_terminal && record.is_archive_record && status == "pending");
    let mut totals = StatsTotals {
        total_count: 1,
        total_tokens: record.row.total_tokens,
        total_cost: record.row.cost.unwrap_or_default(),
        ..StatsTotals::default()
    };
    if runtime_record_is_success_for_summary_row(&record.row)
        && classification.failure_class == FailureClass::None
    {
        totals.success_count = 1;
    }
    if terminal && classification.failure_class != FailureClass::None {
        totals.failure_count = 1;
        totals.non_success_cost = record.row.cost.unwrap_or_default();
    }
    totals
}

fn summary_projection_account_record_totals(
    record: &SummaryProjectionRecord,
) -> Option<(i64, StatsTotals)> {
    let account_id = record.row.upstream_account_id.filter(|id| *id > 0)?;
    if record.account_rollup_covered
        || record.account_archive_totals_fallback_included
        // Unmaterialized archive totals are loaded once by the bounded account aggregate below;
        // retaining the exact row for rolling/usage views must not add it to all-time a second time.
        || (record.is_archive_record && !record.archive_has_materialized_rollups)
    {
        return None;
    }
    Some((account_id, summary_projection_record_totals(record)))
}

fn summary_projection_optional_string_bytes(value: Option<&String>) -> usize {
    value.map_or(0, String::len)
}

fn summary_projection_preview_row_bytes(row: &UpstreamAccountInvocationPreviewRow) -> usize {
    row.invoke_id.len()
        + row.occurred_at.len()
        + row.status.len()
        + summary_projection_optional_string_bytes(row.prompt_cache_key.as_ref())
        + summary_projection_optional_string_bytes(row.conversation_created_at.as_ref())
        + summary_projection_optional_string_bytes(row.live_phase.as_ref())
        + summary_projection_optional_string_bytes(row.failure_class.as_ref())
        + summary_projection_optional_string_bytes(row.route_mode.as_ref())
        + summary_projection_optional_string_bytes(row.model.as_ref())
        + summary_projection_optional_string_bytes(row.request_model.as_ref())
        + summary_projection_optional_string_bytes(row.response_model.as_ref())
        + summary_projection_optional_string_bytes(row.reasoning_effort.as_ref())
        + summary_projection_optional_string_bytes(row.source.as_ref())
        + summary_projection_optional_string_bytes(row.error_message.as_ref())
        + summary_projection_optional_string_bytes(row.downstream_error_message.as_ref())
        + summary_projection_optional_string_bytes(row.failure_kind.as_ref())
        + summary_projection_optional_string_bytes(row.proxy_display_name.as_ref())
        + summary_projection_optional_string_bytes(row.upstream_account_name.as_ref())
        + summary_projection_optional_string_bytes(row.upstream_account_plan_type.as_ref())
        + summary_projection_optional_string_bytes(row.response_content_encoding.as_ref())
        + summary_projection_optional_string_bytes(row.request_compression_algorithm.as_ref())
        + summary_projection_optional_string_bytes(row.transport.as_ref())
        + summary_projection_optional_string_bytes(row.requested_service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.billing_service_tier.as_ref())
        + summary_projection_optional_string_bytes(row.endpoint.as_ref())
        + summary_projection_optional_string_bytes(row.compaction_request_kind.as_ref())
        + summary_projection_optional_string_bytes(row.compaction_response_kind.as_ref())
        + summary_projection_optional_string_bytes(row.image_intent.as_ref())
}

fn ensure_summary_projection_resident_record_bytes(
    rolling_record_bytes: usize,
    current_record_bytes: usize,
) -> Result<()> {
    let resident_record_bytes = rolling_record_bytes
        .checked_add(current_record_bytes)
        .ok_or_else(|| anyhow!("summary projection resident record byte accounting overflowed"))?;
    if resident_record_bytes > SUMMARY_PROJECTION_MAX_PREVIEW_BYTES {
        return Err(anyhow!(
            "summary projection resident record bytes exceeded bounded budget ({SUMMARY_PROJECTION_MAX_PREVIEW_BYTES})"
        ));
    }
    Ok(())
}

fn summary_projection_resident_record_budget_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("resident record bytes exceeded bounded budget")
}

fn summary_projection_archive_row_bytes(row: &SummaryProjectionArchiveRow) -> usize {
    row.invoke_id.len()
        + row.occurred_at.len()
        + row.source.len()
        + summary_projection_optional_string_bytes(row.model.as_ref())
        + summary_projection_optional_string_bytes(row.response_model.as_ref())
        + summary_projection_optional_string_bytes(row.reasoning_effort.as_ref())
        + row.status.len()
        + summary_projection_optional_string_bytes(row.error_message.as_ref())
        + summary_projection_optional_string_bytes(row.failure_kind.as_ref())
        + summary_projection_optional_string_bytes(row.failure_class.as_ref())
}

async fn ensure_summary_projection_archive_text_budget(
    archive_pool: &Pool<Sqlite>,
    exact_ranges: &[ExactUtcRange],
    columns: &HashMap<&str, bool>,
    max_resident_bytes: usize,
) -> Result<(), anyhow::Error> {
    if exact_ranges.is_empty() {
        return Ok(());
    }
    let mut text_columns = vec!["invoke_id", "occurred_at"];
    for column in [
        "source",
        "model",
        "payload",
        "status",
        "error_message",
        "failure_kind",
        "failure_class",
    ] {
        if columns.get(column).copied().unwrap_or(false) {
            text_columns.push(column);
        }
    }
    text_columns.sort_unstable();
    text_columns.dedup();
    let row_bytes = text_columns
        .iter()
        .map(|column| format!("length(CAST(COALESCE({column}, '') AS BLOB))"))
        .collect::<Vec<_>>()
        .join(" + ");
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COALESCE(SUM(row_bytes), 0) FROM (SELECT 512 + ");
    query
        .push(row_bytes.as_str())
        .push(" AS row_bytes FROM codex_invocations WHERE ");
    for (index, _) in exact_ranges.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(exact_ranges[index].start));
        query
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(exact_ranges[index].end))
            .push(")");
    }
    query.push(")");
    let estimated_bytes = query
        .build_query_scalar::<i64>()
        .fetch_one(archive_pool)
        .await?;
    if estimated_bytes > max_resident_bytes as i64 {
        return Err(anyhow!(
            "summary projection archive byte budget exceeded before preview fetch ({estimated_bytes} > {max_resident_bytes})"
        ));
    }
    Ok(())
}
