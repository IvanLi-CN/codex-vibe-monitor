#[derive(Clone, Copy)]
struct SummaryProjectionArchiveRowCoverage {
    bucket_is_full_rollup: bool,
    global_rollup_covered: bool,
    account_rollup_covered: bool,
    usage_global_rollup_covered: bool,
    usage_account_rollup_covered: bool,
}

struct SummaryProjectionArchiveRowCoverageInput<'a> {
    row: &'a SummaryProjectionArchiveRow,
    archive_has_materialized_rollups: bool,
    overall_rollup_archive_replayed: Option<bool>,
    account_rollup_archive_replayed: Option<bool>,
    usage_rollup_archive_replayed: Option<bool>,
    bucket: i64,
    full_rollup_start: i64,
    full_rollup_end: i64,
    protected_boundary_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    usage_rollup_cursor: Option<i64>,
}

fn summary_projection_archive_row_coverage(
    input: SummaryProjectionArchiveRowCoverageInput<'_>,
) -> SummaryProjectionArchiveRowCoverage {
    let bucket_is_full_rollup = input.bucket >= input.full_rollup_start
        && input.bucket < input.full_rollup_end
        && !input.protected_boundary_buckets.contains(&input.bucket);
    let global_rollup_covered = (input.archive_has_materialized_rollups
        || input
            .overall_rollup_archive_replayed
            .is_some_and(|replayed| replayed))
        && input
            .hourly_rollup_totals
            .contains_key(&(input.bucket, None))
        && input
            .overall_rollup_archive_replayed
            .is_none_or(|replayed| replayed);
    let account_rollup_covered = input.archive_has_materialized_rollups
        && input
            .hourly_rollup_totals
            .contains_key(&(input.bucket, input.row.upstream_account_id))
        && input
            .account_rollup_archive_replayed
            .is_none_or(|replayed| replayed);
    let usage_cursor_covers_row = input
        .usage_rollup_cursor
        .is_some_and(|cursor| input.row.id <= cursor);
    let usage_global_rollup_covered = input
        .hourly_rollup_usage
        .contains_key(&(input.bucket, None))
        && ((input.archive_has_materialized_rollups
            && input
                .usage_rollup_archive_replayed
                .is_none_or(|replayed| replayed))
            || (!input.archive_has_materialized_rollups && usage_cursor_covers_row));
    let usage_account_rollup_covered = input
        .hourly_rollup_usage
        .contains_key(&(input.bucket, input.row.upstream_account_id))
        && (input.archive_has_materialized_rollups
            && input
                .usage_rollup_archive_replayed
                .is_none_or(|replayed| replayed)
            || usage_cursor_covers_row);
    SummaryProjectionArchiveRowCoverage {
        bucket_is_full_rollup,
        global_rollup_covered,
        account_rollup_covered,
        usage_global_rollup_covered,
        usage_account_rollup_covered,
    }
}

struct SummaryProjectionArchiveRowMergeInput<'a> {
    row: SummaryProjectionArchiveRow,
    occurred_at: DateTime<Utc>,
    coverage: SummaryProjectionArchiveRowCoverage,
    archive_has_materialized_rollups: bool,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    resident_current_record_bytes: usize,
}

fn merge_summary_projection_archive_row(
    input: SummaryProjectionArchiveRowMergeInput<'_>,
) -> Result<()> {
    *input.exact_record_bytes = input
        .exact_record_bytes
        .saturating_add(summary_projection_archive_row_bytes(&input.row));
    ensure_summary_projection_resident_record_bytes(
        *input.exact_record_bytes,
        input.resident_current_record_bytes,
    )?;
    *input.exact_record_budget = input.exact_record_budget.saturating_add(1);
    if *input.exact_record_budget > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection exact-record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS}) exceeded by unmaterialized archive data"
        ));
    }
    let record = build_summary_projection_archive_record(
        input.row,
        input.occurred_at,
        input.coverage,
        input.archive_has_materialized_rollups,
    );
    let record_key = summary_projection_record_insert_key(input.records_by_invoke_id, &record);
    input.records_by_invoke_id.insert(record_key, record);
    Ok(())
}

fn build_summary_projection_archive_record(
    row: SummaryProjectionArchiveRow,
    occurred_at: DateTime<Utc>,
    coverage: SummaryProjectionArchiveRowCoverage,
    archive_has_materialized_rollups: bool,
) -> SummaryProjectionRecord {
    SummaryProjectionRecord {
        occurred_at,
        global_rollup_covered: coverage.global_rollup_covered,
        account_rollup_covered: coverage.account_rollup_covered,
        usage_global_rollup_covered: coverage.usage_global_rollup_covered,
        usage_account_rollup_covered: coverage.usage_account_rollup_covered,
        is_persisted_live_record: false,
        is_archive_record: true,
        archive_has_materialized_rollups,
        account_archive_totals_fallback_included: false,
        row: UpstreamAccountInvocationPreviewRow {
            upstream_account_id: row.upstream_account_id,
            id: row.id,
            invoke_id: row.invoke_id,
            prompt_cache_key: None,
            occurred_at: row.occurred_at,
            conversation_created_at: None,
            status: row.status,
            live_phase: None,
            failure_class: row.failure_class,
            route_mode: None,
            model: row.model,
            request_model: None,
            response_model: row.response_model,
            total_tokens: row.total_tokens,
            cost: row.cost,
            cost_input: row.cost_input,
            cost_cache_write: row.cost_cache_write,
            cost_cache_read: row.cost_cache_read,
            cost_output: row.cost_output,
            cost_reasoning: row.cost_reasoning,
            source: Some(row.source),
            input_tokens: Some(row.input_tokens),
            output_tokens: Some(row.output_tokens),
            cache_input_tokens: Some(row.cache_input_tokens),
            reasoning_tokens: Some(row.reasoning_tokens),
            reasoning_effort: row.reasoning_effort,
            error_message: row.error_message,
            downstream_status_code: None,
            downstream_error_message: None,
            failure_kind: row.failure_kind,
            is_actionable: None,
            proxy_display_name: None,
            upstream_account_name: None,
            upstream_account_plan_type: None,
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            t_total_ms: None,
            endpoint: None,
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
        },
    }
}

struct SummaryProjectionArchiveRowsInput<'a> {
    archive_pool: &'a Pool<Sqlite>,
    columns: &'a HashMap<&'static str, bool>,
    exact_ranges: &'a [ExactUtcRange],
    archive_has_materialized_rollups: bool,
    resident_current_record_bytes: usize,
    snapshot_rows: Option<Vec<SummaryProjectionArchiveRow>>,
}

async fn load_summary_projection_archive_columns(
    archive_pool: &Pool<Sqlite>,
) -> Result<HashMap<&'static str, bool>> {
    let mut columns = HashMap::new();
    for column in [
        "source",
        "model",
        "payload",
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
        "total_tokens",
        "cost",
        "cost_input",
        "cost_cache_write",
        "cost_cache_read",
        "cost_output",
        "cost_reasoning",
        "status",
        "error_message",
        "failure_kind",
        "failure_class",
    ] {
        columns.insert(
            column,
            crate::stats::sqlite_table_has_column(archive_pool, "codex_invocations", column)
                .await?,
        );
    }
    Ok(columns)
}

fn summary_projection_archive_query(columns: &HashMap<&str, bool>) -> String {
    let has = |column| columns.get(column).copied().unwrap_or(false);
    let payload_expression = |path: &str, cast: &str| {
        if has("payload") {
            format!(
                "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '{path}') AS {cast}) END"
            )
        } else {
            "NULL".to_string()
        }
    };
    let response_model = payload_expression("$.responseModel", "TEXT");
    let reasoning_effort = payload_expression("$.reasoningEffort", "TEXT");
    let upstream_account_id = payload_expression("$.upstreamAccountId", "INTEGER");
    format!(
        "SELECT id, invoke_id, occurred_at, \
         COALESCE({}, '') AS source, {}, {} AS response_model, \
         COALESCE({}, 0) AS input_tokens, COALESCE({}, 0) AS output_tokens, \
         COALESCE({}, 0) AS cache_input_tokens, COALESCE({}, 0) AS reasoning_tokens, \
         {} AS reasoning_effort, COALESCE({}, 0) AS total_tokens, {}, {}, {}, {}, {}, {}, \
         COALESCE({}, '') AS status, {}, {}, {}, {} AS upstream_account_id \
         FROM codex_invocations",
        if has("source") { "source" } else { "NULL" },
        summary_projection_archive_column("model", has("model"), "NULL"),
        response_model,
        if has("input_tokens") {
            "input_tokens"
        } else {
            "NULL"
        },
        if has("output_tokens") {
            "output_tokens"
        } else {
            "NULL"
        },
        if has("cache_input_tokens") {
            "cache_input_tokens"
        } else {
            "NULL"
        },
        if has("reasoning_tokens") {
            "reasoning_tokens"
        } else {
            "NULL"
        },
        reasoning_effort,
        if has("total_tokens") {
            "total_tokens"
        } else {
            "NULL"
        },
        summary_projection_archive_column("cost", has("cost"), "NULL"),
        summary_projection_archive_column("cost_input", has("cost_input"), "NULL"),
        summary_projection_archive_column("cost_cache_write", has("cost_cache_write"), "NULL"),
        summary_projection_archive_column("cost_cache_read", has("cost_cache_read"), "NULL"),
        summary_projection_archive_column("cost_output", has("cost_output"), "NULL"),
        summary_projection_archive_column("cost_reasoning", has("cost_reasoning"), "NULL"),
        if has("status") { "status" } else { "NULL" },
        summary_projection_archive_column("error_message", has("error_message"), "NULL"),
        summary_projection_archive_column("failure_kind", has("failure_kind"), "NULL"),
        summary_projection_archive_column("failure_class", has("failure_class"), "NULL"),
        upstream_account_id,
    )
}

async fn load_summary_projection_archive_rows(
    input: SummaryProjectionArchiveRowsInput<'_>,
) -> Result<Vec<SummaryProjectionArchiveRow>> {
    if input.snapshot_rows.is_none() {
        ensure_summary_projection_archive_text_budget(
            input.archive_pool,
            input.exact_ranges,
            input.columns,
            SUMMARY_PROJECTION_MAX_PREVIEW_BYTES
                .saturating_sub(input.resident_current_record_bytes),
        )
        .await?;
    }
    let mut bind_index = 1usize;
    let mut bounded_query = summary_projection_archive_query(input.columns);
    bounded_query.push_str(" WHERE ");
    if input.exact_ranges.is_empty() {
        bounded_query.push_str("0 = 1");
    } else {
        for (index, _) in input.exact_ranges.iter().enumerate() {
            if index > 0 {
                bounded_query.push_str(" OR ");
            }
            bounded_query.push_str(&format!(
                "(occurred_at >= ?{} AND occurred_at < ?{})",
                bind_index,
                bind_index + 1
            ));
            bind_index += 2;
        }
    }
    bounded_query.push_str(&format!(" LIMIT ?{bind_index}"));
    let archive_read_limit = (summary_projection_exact_record_limit() + 1) as i64;
    let mut archive_query = sqlx::query_as::<_, SummaryProjectionArchiveRow>(&bounded_query);
    for range in input.exact_ranges {
        archive_query = archive_query
            .bind(db_occurred_at_lower_bound(range.start))
            .bind(db_occurred_at_upper_bound(range.end));
    }
    let rows = match input.snapshot_rows {
        Some(snapshot_rows) => snapshot_rows,
        None => {
            archive_query
                .bind(archive_read_limit)
                .fetch_all(input.archive_pool)
                .await?
        }
    };
    if !input.archive_has_materialized_rollups
        && rows.len() > summary_projection_exact_record_limit()
    {
        return Err(anyhow!(
            "summary projection unmaterialized archive exact-record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS}) exceeded"
        ));
    }
    Ok(rows)
}
