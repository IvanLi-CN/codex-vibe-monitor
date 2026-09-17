async fn query_live_model_performance_duration_overrides(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
) -> Result<ModelPerformanceDurationOverrides, ApiError> {
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let model_sql = format!(
        "COALESCE(NULLIF(TRIM({}), ''), NULLIF(TRIM(model), ''), 'unknown')",
        INVOCATION_RESPONSE_MODEL_SQL
    );
    let reasoning_effort_sql = format!("NULLIF(TRIM({}), '')", INVOCATION_REASONING_EFFORT_SQL);
    let occurred_at_epoch_ms_sql = "(CAST(CASE WHEN instr(occurred_at, 'T') > 0 THEN strftime('%s', occurred_at) ELSE strftime('%s', occurred_at || '+08:00') END AS REAL) * 1000.0)";
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_billed_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class_sql}) = 'none' AND cost IS NOT NULL",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let range_end_epoch_ms = range.end.timestamp_millis() as f64;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(model_sql.as_str())
        .push(" AS model, ")
        .push(reasoning_effort_sql.as_str())
        .push(" AS reasoning_effort, ")
        .push(occurred_at_epoch_ms_sql)
        .push(" AS start_epoch_ms, MIN(")
        .push(occurred_at_epoch_ms_sql)
        .push(" + t_total_ms, ")
        .push_bind(range_end_epoch_ms)
        .push(") AS end_epoch_ms FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" AND ")
        .push(success_billed_sql.as_str())
        .push(" AND ")
        .push(sqlite_nonnegative_timing_sql("t_total_ms").as_str())
        .push(" ORDER BY occurred_at ASC, id ASC");
    let mut rows = query
        .build_query_as::<SuccessfulBilledUsageDurationIntervalRow>()
        .fetch(pool);
    let mut row_count = 0usize;
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        union_state.push_row(&row);
    }
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_model_performance_usage_duration",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count,
            elapsed_ms,
            "slow model performance usage duration query"
        );
    }
    Ok(union_state.into_overrides())
}

#[derive(Debug, Default)]
struct QueryCompletedInvocationArchiveActivityAggregateRows {
    aggregates: Vec<UpstreamAccountActivityAggregateRow>,
    skipped_materialized_ranges: Vec<ExactUtcRange>,
}

struct QueryCompletedInvocationArchiveRowResult {
    aggregates: Vec<UpstreamAccountActivityAggregateRow>,
    prompt_rows: Vec<UpstreamAccountPromptCacheCreatedAtRow>,
}

fn dashboard_activity_archive_row_overlap_range(
    archive_row: &crate::stats::ArchiveBatchPathRow,
    requested_range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let coverage_start =
        parse_to_utc_datetime(archive_row.coverage_start_at()?).unwrap_or(requested_range.start);
    let coverage_end =
        parse_to_utc_datetime(archive_row.coverage_end_at()?).unwrap_or(requested_range.end);
    let start = coverage_start.max(requested_range.start);
    let end = coverage_end.min(requested_range.end);
    (start < end).then_some(ExactUtcRange { start, end })
}

async fn query_completed_invocation_archive_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<QueryCompletedInvocationArchiveActivityAggregateRows, ApiError> {
    let archive_rows = crate::stats::load_completed_invocation_archive_paths_in_range(
        pool,
        Some((range.start, range.end)),
    )
    .await?;
    let mut aggregates = Vec::new();
    let mut skipped_materialized_ranges = Vec::new();
    let mut earliest_created_at_by_prompt_cache_key = HashMap::<String, String>::new();
    let mut prompt_cache_keys_by_account = HashMap::<Option<i64>, HashSet<String>>::new();
    for archive_row in archive_rows {
        let Some(result) = query_completed_invocation_archive_activity_row(
            pool,
            &archive_row,
            source_scope,
            range,
        )
        .await?
        else {
            if let Some(skipped_range) = result_skipped_materialized_range(&archive_row, range) {
                skipped_materialized_ranges.push(skipped_range);
            }
            continue;
        };
        aggregates.extend(result.aggregates);
        for row in result.prompt_rows {
            let UpstreamAccountPromptCacheCreatedAtRow {
                upstream_account_id,
                prompt_cache_key,
                first_occurred_at,
            } = row;
            earliest_created_at_by_prompt_cache_key
                .entry(prompt_cache_key.clone())
                .and_modify(|current| {
                    if first_occurred_at < *current {
                        *current = first_occurred_at.clone();
                    }
                })
                .or_insert(first_occurred_at);
            prompt_cache_keys_by_account
                .entry(upstream_account_id)
                .or_default()
                .insert(prompt_cache_key);
        }
    }
    let mut latest_created_at_by_account = HashMap::<Option<i64>, String>::new();
    for (upstream_account_id, prompt_cache_keys) in prompt_cache_keys_by_account {
        let mut latest_created_at = None;
        for prompt_cache_key in prompt_cache_keys {
            merge_latest_optional_timestamp(
                &mut latest_created_at,
                earliest_created_at_by_prompt_cache_key
                    .get(&prompt_cache_key)
                    .cloned(),
            );
        }
        if let Some(latest_created_at) = latest_created_at {
            latest_created_at_by_account.insert(upstream_account_id, latest_created_at);
        }
    }
    for aggregate in &mut aggregates {
        if let Some(latest_created_at) =
            latest_created_at_by_account.get(&aggregate.upstream_account_id)
        {
            aggregate.latest_conversation_created_at = Some(latest_created_at.clone());
        }
    }
    Ok(QueryCompletedInvocationArchiveActivityAggregateRows {
        aggregates,
        skipped_materialized_ranges,
    })
}

fn result_skipped_materialized_range(
    archive_row: &crate::stats::ArchiveBatchPathRow,
    range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    archive_row
        .has_materialized_historical_rollups()
        .then(|| dashboard_activity_archive_row_overlap_range(archive_row, range))
        .flatten()
}

async fn query_completed_invocation_archive_activity_row(
    pool: &Pool<Sqlite>,
    archive_row: &crate::stats::ArchiveBatchPathRow,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Option<QueryCompletedInvocationArchiveRowResult>, ApiError> {
    let Some((archive_pool, temp_cleanup)) =
        crate::stats::open_invocation_archive_batch_pool(archive_row, "dashboard-activity-summary")
            .await?
    else {
        return Ok(None);
    };
    let overlapping_live_ids = query_archive_upstream_account_activity_overlapping_live_ids(
        pool,
        &archive_pool,
        source_scope,
        range,
    )
    .await?;
    let exclude_invocation_ids_filter = prepare_dashboard_activity_excluded_invocation_ids_filter(
        &archive_pool,
        Some(&overlapping_live_ids),
    )
    .await?;
    let aggregates = query_live_upstream_account_activity_aggregate_rows(
        &archive_pool,
        source_scope,
        range,
        false,
        exclude_invocation_ids_filter,
    )
    .await?;
    let prompt_rows = query_live_upstream_account_prompt_cache_created_at_rows(
        &archive_pool,
        source_scope,
        range,
        false,
        exclude_invocation_ids_filter,
    )
    .await?;
    archive_pool.close().await;
    drop(temp_cleanup);
    Ok(Some(QueryCompletedInvocationArchiveRowResult {
        aggregates,
        prompt_rows,
    }))
}

#[derive(Debug, FromRow)]
struct UpstreamAccountActivityRateRow {
    upstream_account_id: Option<i64>,
    occurred_at: String,
    total_tokens: i64,
    total_cost: f64,
}

async fn query_live_upstream_account_activity_rate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountActivityRateRow>, ApiError> {
    let rate_start = range.start.max(range.end - chrono::Duration::minutes(5));
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, occurred_at, COALESCE(total_tokens, 0) AS total_tokens, COALESCE(cost, 0.0) AS total_cost FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(rate_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query
        .build_query_as::<UpstreamAccountActivityRateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

fn merge_upstream_account_activity_recent_row_metadata(
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    row: &UpstreamAccountInvocationPreviewRow,
) {
    let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
        return;
    };
    let entry = account_activity.entry(row.upstream_account_id).or_default();
    entry.last_occurred_at_epoch_ms = entry
        .last_occurred_at_epoch_ms
        .max(occurred_at.timestamp_millis());
    merge_latest_optional_timestamp(&mut entry.last_invocation_at, Some(row.occurred_at.clone()));
    merge_latest_optional_timestamp(
        &mut entry.latest_conversation_created_at,
        row.conversation_created_at.clone(),
    );
    if entry.display_name_hint.is_none() {
        entry.display_name_hint =
            normalize_trimmed_optional_string_local(row.upstream_account_name.clone());
    }
    if entry.plan_type_hint.is_none() {
        entry.plan_type_hint =
            normalize_trimmed_optional_string_local(row.upstream_account_plan_type.clone());
    }
    if matches!(
        normalized_runtime_text(Some(row.status.as_str())).as_str(),
        "running" | "pending"
    ) && let Some(ttfb_ms) = row
        .t_upstream_ttfb_ms
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        entry.in_progress_wait_sum_ms += ttfb_ms;
        entry.in_progress_wait_sample_count += 1;
    }
}

#[derive(Debug, Clone, Copy)]
struct UpstreamAccountActivityPreviewReadTelemetry {
    route: &'static str,
    builder: &'static str,
    purpose: &'static str,
}

#[derive(Clone, Copy)]
struct DashboardActivityPreviewRowsQuery {
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    limit: Option<usize>,
    max_id: Option<i64>,
    in_progress_only: bool,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    query_live_upstream_account_activity_preview_rows_with_limit(
        pool,
        DashboardActivityPreviewRowsQuery {
            source_scope,
            range,
            upstream_account_id: None,
            limit: None,
            max_id: None,
            in_progress_only: false,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "shared",
                builder: "usage_breakdown",
                purpose: "full_range_preview_rows",
            },
        },
    )
    .await
}

fn build_upstream_account_activity_preview_select(
    query: &mut QueryBuilder<Sqlite>,
    source_scope: InvocationSourceScope,
) {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let conversation_created_at_sql = format!(
        "COALESCE({}, occurred_at)",
        prompt_cache_conversation_created_at_sql(INVOCATION_PROMPT_CACHE_KEY_SQL, source_scope)
    );
    let final_first_token_timing_sql =
        final_pool_invocation_timing_sql("codex_invocations", "first_token_ms");
    let final_live_phase_sql = invocation_live_phase_sql_with_timing_sql(
        "codex_invocations",
        final_first_token_timing_sql.as_str(),
    );
    query.push("SELECT id, invoke_id, ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key, occurred_at, ")
        .push(conversation_created_at_sql.as_str())
        .push(" AS conversation_created_at, ")
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(final_live_phase_sql.as_str())
        .push(" AS live_phase, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL)
        .push(" AS upstream_account_plan_type, ")
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(" AS response_content_encoding, ")
        .push(invocation_request_compression_algorithm_with_attempt_fallback_sql(
            "codex_invocations",
        ))
        .push(" AS request_compression_algorithm, ")
        .push(INVOCATION_TRANSPORT_SQL)
        .push(" AS transport, ")
        .push(INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(" AS compaction_request_kind, ")
        .push(INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(" AS compaction_response_kind, ")
        .push(INVOCATION_IMAGE_INTENT_SQL)
        .push(
            " AS image_intent, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
                 THEN json_extract(payload, '$.requestedServiceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
                 THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
                 THEN json_extract(payload, '$.serviceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
                 THEN json_extract(payload, '$.service_tier') END AS service_tier, \
             ",
        )
        .push(INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(" AS billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, ")
        .push(final_first_token_timing_sql.as_str())
        .push(" AS first_token_ms, ")
        .push(final_pool_invocation_timing_sql("codex_invocations", "t_upstream_stream_ms").as_str())
        .push(" AS t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ")
        .push(INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint FROM codex_invocations");
}

// Summary hydration only needs the fields consumed by its totals, failure, and usage reducers.
// Keeping this separate from the dashboard activity projection avoids evaluating every display
// JSON expression for each bounded source-admission record during an off-request rebuild.
fn build_summary_projection_preview_select(query: &mut QueryBuilder<Sqlite>) {
    query
        .push(
            "SELECT NULL AS upstream_account_id, id, invoke_id, \
               NULL AS prompt_cache_key, occurred_at, NULL AS conversation_created_at, ",
        )
        .push(invocation_display_status_sql().as_str())
        .push(" AS status, NULL AS live_phase, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, NULL AS route_mode, model, NULL AS request_model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(
            " AS response_model, COALESCE(total_tokens, 0) AS total_tokens, \
               cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, \
               NULL AS source, input_tokens, output_tokens, cache_input_tokens, \
               NULL AS reasoning_tokens, ",
        )
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(
            " AS reasoning_effort, error_message, NULL AS downstream_status_code, \
               NULL AS downstream_error_message, ",
        )
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(
            " = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, \
               NULL AS proxy_display_name, NULL AS upstream_account_name, \
               NULL AS upstream_account_plan_type, NULL AS response_content_encoding, \
               NULL AS request_compression_algorithm, NULL AS transport, \
               NULL AS requested_service_tier, NULL AS service_tier, \
               NULL AS billing_service_tier, NULL AS t_req_read_ms, \
               NULL AS t_req_parse_ms, NULL AS t_upstream_connect_ms, \
               NULL AS t_upstream_ttfb_ms, NULL AS first_token_ms, \
               NULL AS t_upstream_stream_ms, NULL AS t_resp_parse_ms, \
               NULL AS t_persist_ms, NULL AS t_total_ms, NULL AS endpoint, \
               NULL AS compaction_request_kind, NULL AS compaction_response_kind, \
               NULL AS image_intent FROM codex_invocations",
        );
}

async fn query_live_upstream_account_activity_preview_rows_with_limit(
    pool: &Pool<Sqlite>,
    query: DashboardActivityPreviewRowsQuery,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    query_live_upstream_account_activity_preview_rows_with_limit_executor(pool, query).await
}

async fn query_live_upstream_account_activity_preview_rows_with_limit_executor<'e, E>(
    executor: E,
    query_input: DashboardActivityPreviewRowsQuery,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let DashboardActivityPreviewRowsQuery {
        source_scope,
        range,
        upstream_account_id,
        limit,
        max_id,
        in_progress_only,
        telemetry,
    } = query_input;
    let started_at = Instant::now();
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("");
    build_upstream_account_activity_preview_select(&mut query, source_scope);
    query
        .push(" WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if in_progress_only {
        query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str());
        match upstream_account_id {
            Some(upstream_account_id) => {
                query.push(" = ").push_bind(upstream_account_id);
            }
            None => {
                query.push(" IS NULL");
            }
        }
    }
    if let Some(max_id) = max_id {
        query.push(" AND id <= ").push_bind(max_id);
    }
    query.push(" ORDER BY occurred_at DESC, id DESC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit as i64);
    }

    let rows = query
        .build_query_as::<UpstreamAccountInvocationPreviewRow>()
        .fetch_all(executor)
        .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            ?source_scope,
            start = %range.start,
            end = %range.end,
            range_seconds = (range.end - range.start).num_seconds(),
            upstream_account_id = ?upstream_account_id.flatten(),
            limit = ?limit,
            in_progress_only,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity read"
        );
    }
    Ok(rows)
}

async fn load_summary_projection_live_text_columns(
    pool: &Pool<Sqlite>,
) -> Result<Vec<&'static str>, anyhow::Error> {
    let mut text_columns = vec!["invoke_id", "occurred_at", "status", "source", "payload"];
    for column in [
        "model",
        "request_model",
        "upstream_request_model",
        "error_message",
        "failure_kind",
        "failure_class",
        "downstream_error_message",
    ] {
        if crate::stats::sqlite_table_has_column(pool, "codex_invocations", column).await? {
            text_columns.push(column);
        }
    }
    text_columns.push("__upstream_account_plan_type__");
    text_columns.sort_unstable();
    text_columns.dedup();
    Ok(text_columns)
}

#[derive(Debug, Clone, FromRow)]
struct SummaryProjectionLiveCandidateRow {
    id: i64,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    source_bytes: i64,
}

#[derive(Debug, Clone)]
struct SummaryProjectionLiveCandidate {
    id: i64,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    source_bytes: usize,
    global_rank: usize,
    account_rank: usize,
}

#[derive(Debug, Default)]
struct SummaryProjectionLiveAdmission {
    rows: Vec<UpstreamAccountInvocationPreviewRow>,
    gaps: Vec<SummaryProjectionLiveCandidate>,
    overflow: Option<SummaryProjectionLiveCandidate>,
}

fn summary_projection_live_candidate_bucket(
    candidate: &SummaryProjectionLiveCandidate,
) -> Option<i64> {
    parse_to_utc_datetime(&candidate.occurred_at)
        .map(|occurred_at| align_bucket_epoch(occurred_at.timestamp(), 3_600, 0))
}

fn summary_projection_live_source_row_bytes_sql(text_columns: &[&str]) -> String {
    let row_bytes = text_columns
        .iter()
        .map(|column| {
            if *column == "__upstream_account_plan_type__" {
                format!(
                    "length(CAST(COALESCE(({INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL}), '') AS BLOB))"
                )
            } else {
                format!("length(CAST(COALESCE({column}, '') AS BLOB))")
            }
        })
        .collect::<Vec<_>>()
        .join(" + ");
    format!("512 + {row_bytes}")
}

async fn load_summary_projection_live_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<i64, anyhow::Error> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
        .fetch_one(pool)
        .await
        .map(|high_watermark| high_watermark.unwrap_or_default())
        .context("summary projection live high-watermark hydration failed")
}

fn pack_summary_projection_live_candidates(
    mut candidates: Vec<SummaryProjectionLiveCandidate>,
    limit: usize,
) -> (
    Vec<Vec<SummaryProjectionLiveCandidate>>,
    Vec<SummaryProjectionLiveCandidate>,
    Option<SummaryProjectionLiveCandidate>,
) {
    let admitted_limit = limit.saturating_sub(1);
    let overflow = candidates.get(admitted_limit).cloned();
    candidates.truncate(admitted_limit);
    let mut pages = Vec::<Vec<SummaryProjectionLiveCandidate>>::new();
    let mut page = Vec::new();
    let mut page_bytes = 0_usize;
    let mut gaps = Vec::new();
    for candidate in candidates {
        if candidate.source_bytes > SUMMARY_PROJECTION_MAX_SOURCE_RECORD_BYTES {
            gaps.push(candidate);
            continue;
        }
        let exceeds_page = !page.is_empty()
            && page_bytes.saturating_add(candidate.source_bytes)
                > SUMMARY_PROJECTION_MAX_SOURCE_PAGE_BYTES;
        let exceeds_chunk = page.len() >= DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE;
        if exceeds_page || exceeds_chunk {
            pages.push(std::mem::take(&mut page));
            page_bytes = 0;
        }
        page_bytes = page_bytes.saturating_add(candidate.source_bytes);
        page.push(candidate);
    }
    if !page.is_empty() {
        pages.push(page);
    }
    (pages, gaps, overflow)
}

struct SummaryProjectionLiveCandidatesQuery<'a> {
    pool: &'a Pool<Sqlite>,
    text_columns: &'a [&'a str],
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    high_watermark_id: i64,
    min_id_exclusive: Option<i64>,
    upstream_account_id: Option<Option<i64>>,
    in_progress_only: bool,
    limit: usize,
}

async fn query_summary_projection_live_candidates(
    query_input: SummaryProjectionLiveCandidatesQuery<'_>,
) -> Result<Vec<SummaryProjectionLiveCandidate>, anyhow::Error> {
    let SummaryProjectionLiveCandidatesQuery {
        pool,
        text_columns,
        source_scope,
        range,
        high_watermark_id,
        min_id_exclusive,
        upstream_account_id,
        in_progress_only,
        limit,
    } = query_input;
    let row_bytes = summary_projection_live_source_row_bytes_sql(text_columns);
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, occurred_at, ");
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(row_bytes.as_str())
        .push(" AS source_bytes FROM codex_invocations WHERE id <= ")
        .push_bind(high_watermark_id)
        .push(" AND id > ");
    if let Some(min_id_exclusive) = min_id_exclusive {
        query.push_bind(min_id_exclusive);
    } else {
        query.push_bind(0_i64);
    }
    query.push(" AND occurred_at >= ");
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if in_progress_only {
        query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str());
        match upstream_account_id {
            Some(upstream_account_id) => query.push(" = ").push_bind(upstream_account_id),
            None => query.push(" IS NULL"),
        };
    }
    query
        .push(" ORDER BY occurred_at DESC, id DESC LIMIT ")
        .push_bind(limit as i64);
    let rows = query
        .build_query_as::<SummaryProjectionLiveCandidateRow>()
        .fetch_all(pool)
        .await?;
    let mut account_ranks = HashMap::<Option<i64>, usize>::new();
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let account_rank = account_ranks.entry(row.upstream_account_id).or_default();
            *account_rank += 1;
            SummaryProjectionLiveCandidate {
                id: row.id,
                occurred_at: row.occurred_at,
                upstream_account_id: row.upstream_account_id,
                source_bytes: usize::try_from(row.source_bytes).unwrap_or(usize::MAX),
                global_rank: index + 1,
                account_rank: *account_rank,
            }
        })
        .collect())
}
