use super::*;
#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountActivityRateRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) occurred_at: String,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

pub(crate) async fn query_live_upstream_account_activity_rate_rows(
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

pub(crate) fn merge_upstream_account_activity_recent_row_metadata(
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
pub(crate) struct UpstreamAccountActivityPreviewReadTelemetry {
    pub(crate) route: &'static str,
    pub(crate) builder: &'static str,
    pub(crate) purpose: &'static str,
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    query_live_upstream_account_activity_preview_rows_with_limit(
        pool,
        source_scope,
        range,
        None,
        None,
        None,
        false,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "shared",
            builder: "usage_breakdown",
            purpose: "full_range_preview_rows",
        },
    )
    .await
}

pub(crate) fn build_upstream_account_activity_preview_select(
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
pub(crate) fn build_summary_projection_preview_select(query: &mut QueryBuilder<Sqlite>) {
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

pub(crate) async fn query_live_upstream_account_activity_preview_rows_with_limit(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    limit: Option<usize>,
    max_id: Option<i64>,
    in_progress_only: bool,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    query_live_upstream_account_activity_preview_rows_with_limit_executor(
        pool,
        source_scope,
        range,
        upstream_account_id,
        limit,
        max_id,
        in_progress_only,
        telemetry,
    )
    .await
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows_with_limit_executor<'e, E>(
    executor: E,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    limit: Option<usize>,
    max_id: Option<i64>,
    in_progress_only: bool,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
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

pub(crate) async fn load_summary_projection_live_text_columns(
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
pub(crate) struct SummaryProjectionLiveCandidateRow {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) source_bytes: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryProjectionLiveCandidate {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) source_bytes: usize,
    pub(crate) global_rank: usize,
    pub(crate) account_rank: usize,
}

#[derive(Debug, Default)]
pub(crate) struct SummaryProjectionLiveAdmission {
    pub(crate) rows: Vec<UpstreamAccountInvocationPreviewRow>,
    pub(crate) gaps: Vec<SummaryProjectionLiveCandidate>,
    pub(crate) overflow: Option<SummaryProjectionLiveCandidate>,
}

pub(crate) fn summary_projection_live_candidate_bucket(
    candidate: &SummaryProjectionLiveCandidate,
) -> Option<i64> {
    parse_to_utc_datetime(&candidate.occurred_at)
        .map(|occurred_at| align_bucket_epoch(occurred_at.timestamp(), 3_600, 0))
}

pub(crate) fn summary_projection_live_source_row_bytes_sql(text_columns: &[&str]) -> String {
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

pub(crate) async fn load_summary_projection_live_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<i64, anyhow::Error> {
    observe_query(
        StatsQuery::Summary,
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(pool),
    )
    .await
    .map(|high_watermark| high_watermark.unwrap_or_default())
    .context("summary projection live high-watermark hydration failed")
}

pub(crate) fn pack_summary_projection_live_candidates(
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

pub(crate) async fn query_summary_projection_live_candidates(
    pool: &Pool<Sqlite>,
    text_columns: &[&str],
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    high_watermark_id: i64,
    min_id_exclusive: Option<i64>,
    upstream_account_id: Option<Option<i64>>,
    in_progress_only: bool,
    limit: usize,
) -> Result<Vec<SummaryProjectionLiveCandidate>, anyhow::Error> {
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

pub(crate) async fn query_summary_projection_live_rows_with_budget(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    high_watermark_id: i64,
    min_id_exclusive: Option<i64>,
    upstream_account_id: Option<Option<i64>>,
    limit: usize,
    in_progress_only: bool,
    _max_resident_bytes: usize,
    preview_cache: &mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<SummaryProjectionLiveAdmission, anyhow::Error> {
    let text_columns = load_summary_projection_live_text_columns(pool).await?;
    let candidates = query_summary_projection_live_candidates(
        pool,
        &text_columns,
        source_scope,
        range,
        high_watermark_id,
        min_id_exclusive,
        upstream_account_id,
        in_progress_only,
        limit,
    )
    .await?;
    let (pages, mut gaps, overflow) = pack_summary_projection_live_candidates(candidates, limit);
    let pages_to_fetch = pages
        .into_iter()
        .map(|page| {
            let uncached_ids = page
                .iter()
                .filter(|candidate| !preview_cache.contains_key(&candidate.id))
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            (page, uncached_ids)
        })
        .collect::<Vec<_>>();
    let page_results = futures_util::stream::iter(pages_to_fetch.into_iter().map(
        |(page, uncached_ids)| async move {
            let mut fetched_rows = if uncached_ids.is_empty() {
                Vec::new()
            } else {
                query_summary_projection_preview_rows_by_ids(
                    pool,
                    source_scope,
                    &uncached_ids,
                    telemetry,
                )
                .await
                .map_err(|error| {
                    anyhow!("summary projection live preview fetch failed: {error:?}")
                })?
            };
            let upstream_account_ids = page
                .iter()
                .map(|candidate| (candidate.id, candidate.upstream_account_id))
                .collect::<HashMap<_, _>>();
            for row in &mut fetched_rows {
                row.upstream_account_id = upstream_account_ids.get(&row.id).copied().flatten();
            }
            Ok::<_, anyhow::Error>((page, fetched_rows))
        },
    ))
    .buffer_unordered(8)
    .try_collect::<Vec<_>>()
    .await?;
    let mut rows = Vec::new();
    for (page, fetched_rows) in page_results {
        for row in fetched_rows {
            preview_cache.insert(row.id, row);
        }
        let mut page_rows = Vec::with_capacity(page.len());
        for candidate in page {
            if let Some(row) = preview_cache.get(&candidate.id) {
                page_rows.push(row.clone());
            } else {
                gaps.push(candidate);
            }
        }
        rows.extend(page_rows);
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut connection = pool.acquire().await?;
    restore_summary_projection_persisted_statuses(&mut connection, &mut rows)
        .await
        .context("summary projection live status hydration failed")?;
    Ok(SummaryProjectionLiveAdmission {
        rows,
        gaps,
        overflow,
    })
}

pub(crate) async fn query_summary_projection_preview_rows_by_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let mut connection = pool.acquire().await?;
    let mut rows = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        build_summary_projection_preview_select(&mut query);
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query.push(" ORDER BY occurred_at DESC, id DESC");
        rows.extend(
            query
                .build_query_as::<UpstreamAccountInvocationPreviewRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/summary",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            candidate_preview_id_count = ids.len(),
            hydrated_preview_row_count = rows.len(),
            ?source_scope,
            selected_preview_row_count = ids.len(),
            row_count = rows.len(),
            elapsed_ms,
            "slow summary projection preview hydration"
        );
    }
    Ok(rows)
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionHistoricalLiveBucketRow {
    pub(crate) occurred_at: String,
    pub(crate) max_id: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionHistoricalLiveAccountBucketRow {
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) max_id: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionHistoricalLiveTerminalRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionHistoricalLiveChangeRow {
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
}

pub(crate) fn summary_projection_intersect_exact_ranges(
    left: ExactUtcRange,
    right: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let range = ExactUtcRange {
        start: left.start.max(right.start),
        end: left.end.min(right.end),
    };
    (range.start < range.end).then_some(range)
}

pub(crate) fn summary_projection_bucket_intersects_range(
    bucket_start_epoch: i64,
    range: ExactUtcRange,
) -> bool {
    let Some(bucket_start) = Utc.timestamp_opt(bucket_start_epoch, 0).single() else {
        return false;
    };
    bucket_start < range.end && range.start < bucket_start + ChronoDuration::hours(1)
}

pub(crate) async fn reuse_summary_projection_historical_live_coverage(
    pool: &Pool<Sqlite>,
    previous_coverage: Option<&SummaryProjectionHistoricalLiveCoverage>,
    range: ExactUtcRange,
    high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    fully_admitted_live_buckets: &BTreeSet<i64>,
    previous_unavailable_global_ranges: &[ExactUtcRange],
    previous_unavailable_account_ranges: &HashMap<i64, Vec<ExactUtcRange>>,
    previous_terminal_identities: &HashSet<String>,
    pending_terminal_identities: &HashSet<String>,
    unavailable_global_buckets: &mut BTreeSet<i64>,
    unavailable_account_buckets: &mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &mut HashSet<String>,
) -> Result<SummaryProjectionHistoricalLiveCoverage> {
    let Some(previous_coverage) = previous_coverage else {
        // A Rolling revision can be reached only after an older process failed before it
        // published Bootstrap proof. Preserve its independent current/recent selections, but
        // never infer historical exactness without the bounded coverage snapshot.
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [range])?;
        return Ok(SummaryProjectionHistoricalLiveCoverage {
            range,
            high_watermark_id,
            reconciliation_required: true,
        });
    };
    let Some(overlap) = summary_projection_intersect_exact_ranges(previous_coverage.range, range)
    else {
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [range])?;
        return Ok(SummaryProjectionHistoricalLiveCoverage {
            range,
            high_watermark_id,
            reconciliation_required: true,
        });
    };

    let mut reconciliation_required = previous_coverage.reconciliation_required;
    for previous_range in previous_unavailable_global_ranges {
        if let Some(intersection) =
            summary_projection_intersect_exact_ranges(*previous_range, overlap)
        {
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_global_buckets,
                [intersection],
            )?;
        }
    }
    for (account_id, ranges) in previous_unavailable_account_ranges {
        let account_buckets = unavailable_account_buckets.entry(*account_id).or_default();
        for previous_range in ranges {
            if let Some(intersection) =
                summary_projection_intersect_exact_ranges(*previous_range, overlap)
            {
                summary_projection_mark_unavailable_archive_ranges(
                    account_buckets,
                    [intersection],
                )?;
            }
        }
    }
    for identity in previous_terminal_identities {
        let Some((_, occurred_at)) = identity.split_once('\0') else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(occurred_at) else {
            continue;
        };
        if overlap.start <= occurred_at && occurred_at < overlap.end {
            global_covered_terminal_invoke_ids.insert(identity.clone());
        }
    }

    // The moving historical boundary normally advances by seconds. Validate only that new tail
    // synchronously; a large clock/cadence jump is conservatively localized and recovered by
    // the background pass rather than putting the whole Rolling projection back on the slow
    // historical group-by path.
    if previous_coverage.range.end < range.end {
        let tail = ExactUtcRange {
            start: previous_coverage.range.end.max(range.start),
            end: range.end,
        };
        if tail.start < tail.end {
            if tail.end - tail.start <= ChronoDuration::hours(1) {
                mark_summary_projection_uncovered_historical_live_ranges(
                    pool,
                    tail,
                    high_watermark_id,
                    rollup_live_cursor,
                    account_rollup_live_cursor,
                    hourly_rollup_totals,
                    hourly_rollup_usage,
                    fully_admitted_live_buckets,
                    pending_terminal_identities,
                    unavailable_global_buckets,
                    unavailable_account_buckets,
                    global_covered_terminal_invoke_ids,
                )
                .await?;
            } else {
                summary_projection_mark_unavailable_archive_ranges(
                    unavailable_global_buckets,
                    [tail],
                )?;
                reconciliation_required = true;
            }
        }
    }

    // New IDs can be inserted with an old timestamp. Reading this bounded metadata delta is
    // enough to invalidate only its affected hour without re-grouping every historical live
    // record. The later recovery pass re-establishes full compact proof before the gap clears.
    const TERMINAL_WHERE: &str =
        " AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')";
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut changes_query = QueryBuilder::<Sqlite>::new("SELECT occurred_at, ");
    changes_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id FROM codex_invocations WHERE id > ")
        .push_bind(previous_coverage.high_watermark_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(overlap.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(overlap.end))
        .push(TERMINAL_WHERE)
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind(summary_projection_exact_record_limit().saturating_add(1) as i64);
    let changes = changes_query
        .build_query_as::<SummaryProjectionHistoricalLiveChangeRow>()
        .fetch_all(pool)
        .await
        .context("summary projection historical live delta hydration failed")?;
    if changes.len() > summary_projection_exact_record_limit() {
        summary_projection_mark_unavailable_archive_ranges(unavailable_global_buckets, [overlap])?;
        reconciliation_required = true;
    } else {
        for change in changes {
            let Some(occurred_at) = parse_to_utc_datetime(&change.occurred_at) else {
                continue;
            };
            let changed_range = ExactUtcRange {
                start: occurred_at,
                end: occurred_at + ChronoDuration::seconds(1),
            };
            summary_projection_mark_unavailable_archive_ranges(
                unavailable_global_buckets,
                [changed_range],
            )?;
            if let Some(account_id) = change
                .upstream_account_id
                .filter(|account_id| *account_id > 0)
            {
                summary_projection_mark_unavailable_archive_ranges(
                    unavailable_account_buckets.entry(account_id).or_default(),
                    [changed_range],
                )?;
            }
            reconciliation_required = true;
        }
    }

    Ok(SummaryProjectionHistoricalLiveCoverage {
        range,
        high_watermark_id,
        reconciliation_required,
    })
}

pub(crate) async fn mark_summary_projection_uncovered_historical_live_ranges(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    high_watermark_id: i64,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    fully_admitted_live_buckets: &BTreeSet<i64>,
    pending_terminal_identities: &HashSet<String>,
    unavailable_global_buckets: &mut BTreeSet<i64>,
    unavailable_account_buckets: &mut HashMap<i64, BTreeSet<i64>>,
    global_covered_terminal_invoke_ids: &mut HashSet<String>,
) -> Result<()> {
    // Historical coverage is a bucket property: a compact rollup either covers every terminal
    // row in an hour or it does not. Reading one metadata row per source record made a retained
    // 34-day horizon fail closed once its *aggregate* record count crossed the resident record
    // limit, even when a requested 7-day selection had complete compact proof.
    const TERMINAL_WHERE: &str =
        " AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')";
    // `occurred_at` is stored as a naive local timestamp in legacy databases. Group by its raw
    // hour text and use the project parser below; SQLite's `strftime` would reinterpret that
    // text as UTC and shift every historical proof by the local offset.
    const BUCKET_SQL: &str = "substr(occurred_at, 1, 13)";
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut globally_covered_buckets = HashSet::new();
    // The durable rollup cursor already proves every source row at or below its fence. Clear
    // stale resident-budget gaps from those buckets before checking only the unrolled tail.
    for bucket in unavailable_global_buckets
        .iter()
        .copied()
        .collect::<Vec<_>>()
    {
        if summary_projection_bucket_intersects_range(bucket, range)
            && hourly_rollup_totals.contains_key(&(bucket, None))
            && hourly_rollup_usage.contains_key(&(bucket, None))
        {
            globally_covered_buckets.insert(bucket);
            unavailable_global_buckets.remove(&bucket);
        }
    }
    let mut global_query = QueryBuilder::<Sqlite>::new("SELECT ");
    global_query
        .push("MIN(occurred_at) AS occurred_at, MAX(id) AS max_id FROM codex_invocations WHERE id <= ")
        .push_bind(high_watermark_id)
        .push(" AND id > ")
        .push_bind(rollup_live_cursor)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(TERMINAL_WHERE)
        .push(" GROUP BY ")
        .push(BUCKET_SQL);
    let global_rows = global_query
        .build_query_as::<SummaryProjectionHistoricalLiveBucketRow>()
        .fetch_all(pool)
        .await
        .context("summary projection historical live coverage hydration failed")?;

    for row in global_rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket_start_epoch = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        let globally_covered = fully_admitted_live_buckets.contains(&bucket_start_epoch)
            || (row.max_id <= rollup_live_cursor
                && hourly_rollup_totals.contains_key(&(bucket_start_epoch, None))
                && hourly_rollup_usage.contains_key(&(bucket_start_epoch, None)));
        if globally_covered {
            globally_covered_buckets.insert(bucket_start_epoch);
        } else {
            unavailable_global_buckets.insert(bucket_start_epoch);
        }
    }
    // The initial bounded live admission may have marked a bucket unavailable before its
    // durable rollup was committed.  A complete global rollup is stronger proof than that
    // stale resident-budget gap, so remove only the buckets this pass proves covered.
    for bucket in &globally_covered_buckets {
        unavailable_global_buckets.remove(bucket);
    }

    let mut covered_account_buckets = HashMap::<i64, BTreeSet<i64>>::new();
    for (account_id, unavailable_buckets) in unavailable_account_buckets.iter_mut() {
        for bucket in unavailable_buckets.clone() {
            if summary_projection_bucket_intersects_range(bucket, range)
                && account_rollup_live_cursor.is_some_and(|cursor| {
                    hourly_rollup_totals.contains_key(&(bucket, Some(*account_id)))
                        && hourly_rollup_usage.contains_key(&(bucket, Some(*account_id)))
                        && cursor >= 0
                })
            {
                covered_account_buckets
                    .entry(*account_id)
                    .or_default()
                    .insert(bucket);
                unavailable_buckets.remove(&bucket);
            }
        }
    }
    unavailable_account_buckets.retain(|_, buckets| !buckets.is_empty());
    let mut account_query = QueryBuilder::<Sqlite>::new("SELECT ");
    account_query
        .push("MIN(occurred_at) AS occurred_at, ")
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, MAX(id) AS max_id FROM codex_invocations WHERE id <= ")
        .push_bind(high_watermark_id)
        .push(" AND id > ");
    if let Some(cursor) = account_rollup_live_cursor {
        account_query.push_bind(cursor);
    } else {
        account_query.push_bind(0_i64);
    }
    account_query
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(TERMINAL_WHERE)
        .push(" GROUP BY ")
        .push(BUCKET_SQL)
        .push(", ")
        .push(resolved_upstream_account_id_sql.as_str());
    let account_rows = account_query
        .build_query_as::<SummaryProjectionHistoricalLiveAccountBucketRow>()
        .fetch_all(pool)
        .await
        .context("summary projection historical live account coverage hydration failed")?;
    for row in account_rows {
        let Some(account_id) = row.upstream_account_id.filter(|account_id| *account_id > 0) else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket_start_epoch = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        let account_covered = fully_admitted_live_buckets.contains(&bucket_start_epoch)
            || (account_rollup_live_cursor.is_some_and(|cursor| row.max_id <= cursor)
                && hourly_rollup_totals.contains_key(&(bucket_start_epoch, Some(account_id)))
                && hourly_rollup_usage.contains_key(&(bucket_start_epoch, Some(account_id))));
        if account_covered {
            covered_account_buckets
                .entry(account_id)
                .or_default()
                .insert(bucket_start_epoch);
        } else {
            unavailable_account_buckets
                .entry(account_id)
                .or_default()
                .insert(bucket_start_epoch);
        }
    }
    for (account_id, buckets) in covered_account_buckets {
        if let Some(unavailable) = unavailable_account_buckets.get_mut(&account_id) {
            for bucket in buckets {
                unavailable.remove(&bucket);
            }
            if unavailable.is_empty() {
                unavailable_account_buckets.remove(&account_id);
            }
        }
    }

    // The complete identity path below is only safe when the entire historical population fits
    // in the resident budget. Probe the same population with a hard limit rather than counting
    // only the unrolled tail; otherwise a zero tail admits an unbounded historical fetch.
    let mut terminal_identity_probe = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM (SELECT 1 FROM codex_invocations WHERE id <= ",
    );
    terminal_identity_probe
        .push_bind(high_watermark_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(TERMINAL_WHERE)
        .push(" LIMIT ")
        .push_bind((summary_projection_exact_record_limit() + 1) as i64)
        .push(")");
    let terminal_identity_count = terminal_identity_probe
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .context("summary projection historical terminal identity admission failed")?;
    if terminal_identity_count <= summary_projection_exact_record_limit() as i64 {
        // Preserve the complete identity proof for the common bounded case. Existing overlays
        // then disappear as soon as their durable compact bucket is published, exactly as they
        // did before the production-scale path was introduced.
        let mut identity_query = QueryBuilder::<Sqlite>::new(
            "SELECT id, invoke_id, occurred_at, status FROM codex_invocations WHERE id <= ",
        );
        identity_query
            .push_bind(high_watermark_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(range.end))
            .push(TERMINAL_WHERE);
        let rows = identity_query
            .build_query_as::<SummaryProjectionHistoricalLiveTerminalRow>()
            .fetch_all(pool)
            .await
            .context("summary projection historical terminal identity hydration failed")?;
        #[cfg(test)]
        note_summary_projection_historical_identity_hydration(rows.len());
        for row in rows {
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
            if row.id <= rollup_live_cursor
                && (globally_covered_buckets.contains(&bucket)
                    || (hourly_rollup_totals.contains_key(&(bucket, None))
                        && hourly_rollup_usage.contains_key(&(bucket, None))))
            {
                global_covered_terminal_invoke_ids.insert(summary_projection_source_identity_key(
                    row.id,
                    &row.invoke_id,
                    &row.occurred_at,
                ));
            }
        }
    }

    // SSE retains identities, not buckets. Query only the bounded terminal overlay currently in
    // memory so a freshly published compact bucket can consume an already-persisted delta without
    // retaining every historical identity.
    let pending_terminal_identity_refs =
        if terminal_identity_count > summary_projection_exact_record_limit() as i64 {
            pending_terminal_identities
                .iter()
                .filter_map(|identity| {
                    decode_summary_projection_pending_terminal_identity(identity).map(
                        |(row_id, occurred_at, invoke_id)| {
                            (row_id, occurred_at.to_string(), invoke_id.to_string())
                        },
                    )
                })
                .take(250)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
    for identities in pending_terminal_identity_refs.chunks(250) {
        if identities.is_empty() {
            continue;
        }
        let mut identity_query = QueryBuilder::<Sqlite>::new(
            "SELECT id, invoke_id, occurred_at, status FROM codex_invocations WHERE id <= ",
        );
        identity_query
            .push_bind(high_watermark_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(range.end))
            .push(" AND id IN (");
        for (index, (row_id, _, _)) in identities.iter().enumerate() {
            if index > 0 {
                identity_query.push(", ");
            }
            identity_query.push_bind(*row_id);
        }
        identity_query.push(") ").push(TERMINAL_WHERE);
        let rows = identity_query
            .build_query_as::<SummaryProjectionHistoricalLiveTerminalRow>()
            .fetch_all(pool)
            .await
            .context("summary projection historical terminal identity hydration failed")?;
        for row in rows {
            let matches_requested_identity =
                identities.iter().any(|(row_id, occurred_at, invoke_id)| {
                    row.id == *row_id
                        && row.occurred_at == *occurred_at
                        && row.invoke_id == *invoke_id
                });
            if !matches_requested_identity {
                continue;
            }
            if matches!(
                normalized_runtime_text(row.status.as_deref()).as_str(),
                "pending" | "running"
            ) {
                continue;
            }
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
            if row.id <= rollup_live_cursor && globally_covered_buckets.contains(&bucket) {
                global_covered_terminal_invoke_ids.insert(summary_projection_source_identity_key(
                    row.id,
                    &row.invoke_id,
                    &row.occurred_at,
                ));
            }
        }
    }
    Ok(())
}

pub(crate) async fn query_live_upstream_account_activity_preview_candidate_ids_per_account(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    max_candidate_ids: Option<usize>,
) -> Result<Vec<i64>, ApiError> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH ranked AS (\
           SELECT id \
             FROM (\
               SELECT id, ROW_NUMBER() OVER (PARTITION BY ",
    );
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(
            " ORDER BY occurred_at DESC, id DESC) AS account_rank \
               FROM codex_invocations \
               WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(") WHERE account_rank <= ")
        .push_bind(limit_per_account as i64)
        .push(") SELECT id FROM ranked");
    if let Some(max_candidate_ids) = max_candidate_ids {
        query
            .push(" LIMIT ")
            .push_bind(max_candidate_ids.saturating_add(1) as i64);
    }
    Ok(query.build_query_scalar::<i64>().fetch_all(pool).await?)
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows_by_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_live_upstream_account_activity_preview_rows_by_ids_connection(
        &mut connection,
        source_scope,
        ids,
        telemetry,
    )
    .await
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows_by_ids_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let mut rows = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        build_upstream_account_activity_preview_select(&mut query, source_scope);
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        query.push(" ORDER BY occurred_at DESC, id DESC");

        rows.extend(
            query
                .build_query_as::<UpstreamAccountInvocationPreviewRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            candidate_preview_id_count = ids.len(),
            hydrated_preview_row_count = rows.len(),
            ?source_scope,
            selected_preview_row_count = ids.len(),
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity preview hydration"
        );
    }
    Ok(rows)
}
