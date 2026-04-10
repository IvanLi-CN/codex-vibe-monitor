fn split_exact_range_by_retention(
    live_ranges: &mut Vec<ExactUtcRange>,
    range: ExactUtcRange,
    raw_cutoff: DateTime<Utc>,
) -> Result<(), ApiError> {
    if range.end > raw_cutoff {
        push_exact_range(live_ranges, range.start.max(raw_cutoff), range.end)?;
    }
    Ok(())
}

fn build_hourly_rollup_exact_range_plan(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    raw_cutoff: DateTime<Utc>,
) -> Result<HourlyRollupExactRangePlan, ApiError> {
    let mut plan = HourlyRollupExactRangePlan::default();
    let start_epoch = start.timestamp();
    let end_epoch = end.timestamp();
    // Archived history is only available as hourly buckets. Keep only the full hours that are
    // completely contained in the requested range so historical queries never overstate totals.
    let full_hour_start_epoch = ceil_hour_epoch(start_epoch);
    let full_hour_end_epoch = align_bucket_epoch(end_epoch, 3_600, 0);
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour start epoch")))?;
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour end epoch")))?;
    if full_hour_start_epoch < full_hour_end_epoch {
        plan.full_hour_range = Some((full_hour_start_epoch, full_hour_end_epoch));
    }
    if let Some(range) = exact_utc_range(start, end.min(full_hour_start))? {
        split_exact_range_by_retention(&mut plan.live_exact_ranges, range, raw_cutoff)?;
    }
    if let Some(range) = exact_utc_range(start.max(full_hour_end), end)? {
        split_exact_range_by_retention(&mut plan.live_exact_ranges, range, raw_cutoff)?;
    }
    Ok(plan)
}

fn effective_range_for_hourly_rollup_plan(
    plan: &HourlyRollupExactRangePlan,
) -> Result<Option<ExactUtcRange>, ApiError> {
    let mut range: Option<ExactUtcRange> = None;
    if let Some((start_epoch, end_epoch)) = plan.full_hour_range {
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid effective range start epoch")))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid effective range end epoch")))?;
        range = Some(ExactUtcRange { start, end });
    }
    for exact_range in &plan.live_exact_ranges {
        range = Some(match range {
            Some(existing) => ExactUtcRange {
                start: existing.start.min(exact_range.start),
                end: existing.end.max(exact_range.end),
            },
            None => *exact_range,
        });
    }
    Ok(range.filter(|value| value.start < value.end))
}

async fn load_pool_attempt_account_names(
    pool: &Pool<Sqlite>,
    records: &mut [ApiPoolUpstreamRequestAttempt],
) -> Result<(), ApiError> {
    let account_ids = records
        .iter()
        .filter_map(|record| record.upstream_account_id)
        .collect::<HashSet<_>>();
    if account_ids.is_empty() {
        return Ok(());
    }

    #[derive(Debug, FromRow)]
    struct AccountNameRow {
        id: i64,
        display_name: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, display_name FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");
    let rows = query
        .build_query_as::<AccountNameRow>()
        .fetch_all(pool)
        .await?;
    let name_map = rows
        .into_iter()
        .map(|row| (row.id, row.display_name))
        .collect::<HashMap<_, _>>();
    for record in records {
        if record.upstream_account_name.is_none()
            && let Some(account_id) = record.upstream_account_id
        {
            record.upstream_account_name = name_map.get(&account_id).cloned();
        }
    }
    Ok(())
}

pub(crate) async fn query_pool_attempt_records_from_live(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
) -> Result<Vec<ApiPoolUpstreamRequestAttempt>, ApiError> {
    let mut records = sqlx::query_as::<_, ApiPoolUpstreamRequestAttempt>(
        r#"
        SELECT
            attempts.id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            attempts.started_at,
            attempts.finished_at,
            attempts.status,
            COALESCE(
                attempts.phase,
                CASE
                    WHEN attempts.status = 'pending' THEN 'sending_request'
                    WHEN attempts.status = 'success' THEN 'completed'
                    ELSE 'failed'
                END
            ) AS phase,
            attempts.http_status,
            attempts.downstream_http_status,
            attempts.failure_kind,
            attempts.error_message,
            attempts.downstream_error_message,
            attempts.connect_latency_ms,
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.created_at
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        WHERE attempts.invoke_id = ?1
        ORDER BY attempts.attempt_index ASC, attempts.id ASC
        "#,
    )
    .bind(invoke_id)
    .fetch_all(pool)
    .await?;
    load_pool_attempt_account_names(pool, &mut records).await?;
    Ok(records)
}

async fn query_invocation_aggregate_records_from_live_range(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, occurred_at, status, total_tokens, cost, error_message, failure_kind, \
            failure_class, is_actionable, t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE occurred_at >= ",
    );
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at ASC, id ASC");
    query
        .build_query_as::<InvocationAggregateRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

fn extend_unique_invocation_records(
    records: &mut Vec<InvocationAggregateRecord>,
    seen_ids: &mut HashSet<i64>,
    candidates: Vec<InvocationAggregateRecord>,
) {
    for record in candidates {
        if seen_ids.insert(record.id) {
            records.push(record);
        }
    }
}

async fn query_invocation_exact_records(
    pool: &Pool<Sqlite>,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for range in &range_plan.live_exact_ranges {
        extend_unique_invocation_records(
            &mut records,
            &mut seen_ids,
            query_invocation_aggregate_records_from_live_range(pool, *range, source_scope).await?,
        );
    }

    records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

fn add_invocation_record_to_summary_totals(
    totals: &mut StatsTotals,
    record: &InvocationAggregateRecord,
) {
    totals.total_count += 1;
    match record.status.as_deref() {
        Some("success") => totals.success_count += 1,
        Some(_) => totals.failure_count += 1,
        None => {}
    }
    totals.total_tokens += record.total_tokens.unwrap_or_default();
    totals.total_cost += record.cost.unwrap_or_default();
}

pub(crate) fn db_occurred_at_upper_bound(end_utc: DateTime<Utc>) -> String {
    if end_utc.timestamp_subsec_nanos() > 0 {
        return db_occurred_at_lower_bound(end_utc + ChronoDuration::seconds(1));
    }
    db_occurred_at_lower_bound(end_utc)
}

fn record_perf_stage_sample(
    by_stage: &mut BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
    stage: &str,
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    let entry = by_stage
        .entry(stage.to_string())
        .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
    entry.0 += 1;
    entry.1 += value;
    entry.2 = entry.2.max(value);
    add_approx_histogram_sample(&mut entry.3, value);
}

pub(crate) async fn query_hourly_backed_summary_since_with_config(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(invocation_max_days);
    if start >= retention_cutoff {
        return query_combined_totals(pool, relay, StatsFilter::Since(start), source_scope)
            .await
            .map_err(Into::into);
    }

    let mut totals = StatsTotals::default();
    let now = Utc::now();
    let range_plan = build_hourly_rollup_exact_range_plan(start, now, retention_cutoff)?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let rows = query_invocation_hourly_rollup_range(
            pool,
            range_start_epoch,
            range_end_epoch,
            source_scope,
        )
        .await?;
        for row in rows {
            totals.total_count += row.total_count;
            totals.success_count += row.success_count;
            totals.failure_count += row.failure_count;
            totals.total_tokens += row.total_tokens;
            totals.total_cost += row.total_cost;
        }
    }
    let exact_records = query_invocation_exact_records(pool, &range_plan, source_scope).await?;
    for record in &exact_records {
        add_invocation_record_to_summary_totals(&mut totals, record);
    }
    let relay_totals =
        if let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)? {
            query_crs_totals(
                pool,
                relay,
                &StatsFilter::Since(effective_range.start),
                source_scope,
            )
            .await?
        } else {
            StatsTotals::default()
        };
    Ok(totals.add(relay_totals))
}

pub(crate) async fn query_hourly_backed_summary_since(
    state: &AppState,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_since_with_config(
        &state.pool,
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        start,
        source_scope,
    )
    .await
    .map_err(Into::into)
}

pub(crate) async fn fetch_forward_proxy_live_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ForwardProxyLiveStatsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let response = build_forward_proxy_live_stats_response(state.as_ref()).await?;
    Ok(Json(response))
}

pub(crate) async fn fetch_forward_proxy_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<ForwardProxyTimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    ensure_forward_proxy_hourly_tz_supported(reporting_tz, &range_window)?;
    let bucket_spec = params.bucket.as_deref().unwrap_or("1h");
    if bucket_seconds_from_spec(bucket_spec) != Some(3_600) {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported forward proxy bucket specification: {bucket_spec}; only 1h is supported"
        )));
    }
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let response = build_forward_proxy_timeseries_response(state.as_ref(), range_window).await?;
    Ok(Json(response))
}

fn ensure_forward_proxy_hourly_tz_supported(
    reporting_tz: Tz,
    range_window: &RangeWindow,
) -> Result<(), ApiError> {
    if reporting_tz_has_whole_hour_offsets(reporting_tz, range_window) {
        return Ok(());
    }
    Err(ApiError::bad_request(anyhow!(
        "unsupported timeZone for forward proxy hourly timeseries: {reporting_tz}; hourly buckets require whole-hour UTC offsets"
    )))
}

pub(crate) fn reporting_tz_has_whole_hour_offsets(
    reporting_tz: Tz,
    range_window: &RangeWindow,
) -> bool {
    const SAMPLE_STEP_DAYS: i64 = 1;

    fn offset_is_hour_aligned(reporting_tz: Tz, instant: DateTime<Utc>) -> bool {
        instant
            .with_timezone(&reporting_tz)
            .offset()
            .fix()
            .local_minus_utc()
            .rem_euclid(3_600)
            == 0
    }

    let mut cursor = range_window.start;
    while cursor < range_window.end {
        if !offset_is_hour_aligned(reporting_tz, cursor) {
            return false;
        }
        let Some(next) = cursor.checked_add_signed(ChronoDuration::days(SAMPLE_STEP_DAYS)) else {
            break;
        };
        if next >= range_window.end {
            break;
        }
        cursor = next;
    }
    if let Some(last_instant) = range_window
        .end
        .checked_sub_signed(ChronoDuration::nanoseconds(1))
        .filter(|instant| *instant >= range_window.start)
    {
        return offset_is_hour_aligned(reporting_tz, last_instant);
    }
    true
}

pub(crate) async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let request = resolve_prompt_cache_conversations_request(params)?;
    let response = if request.page_size.is_none() && request.cursor.is_none() && request.snapshot_at.is_none() {
        let response =
            fetch_prompt_cache_conversations_cached(state.as_ref(), request.selection).await?;
        match request.detail_level {
            PromptCacheConversationDetailLevel::Full => response,
            PromptCacheConversationDetailLevel::Compact => {
                compact_prompt_cache_conversations_response(response)
            }
        }
    } else {
        build_prompt_cache_conversations_response_for_request(state.as_ref(), request).await?
    };
    Ok(Json(response))
}

pub(crate) fn normalize_prompt_cache_conversation_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(value @ (20 | 50 | 100)) => value,
        _ => PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_hours(raw: Option<i64>) -> Option<i64> {
    match raw {
        Some(value @ (1 | 3 | 6 | 12 | 24)) => Some(value),
        _ => None,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_minutes(
    raw: Option<i64>,
) -> Option<i64> {
    match raw {
        Some(5) => Some(5),
        _ => None,
    }
}

pub(crate) fn resolve_prompt_cache_conversation_selection(
    params: PromptCacheConversationsQuery,
) -> Result<PromptCacheConversationSelection, ApiError> {
    let activity_param_count =
        i64::from(params.activity_hours.is_some()) + i64::from(params.activity_minutes.is_some());
    if params.limit.is_some() && activity_param_count > 0 {
        return Err(ApiError::bad_request(anyhow!(
            "limit, activityHours, and activityMinutes are mutually exclusive"
        )));
    }
    if params.activity_hours.is_some() && params.activity_minutes.is_some() {
        return Err(ApiError::bad_request(anyhow!(
            "activityHours and activityMinutes are mutually exclusive"
        )));
    }

    if let Some(hours) = normalize_prompt_cache_conversation_activity_hours(params.activity_hours) {
        return Ok(PromptCacheConversationSelection::ActivityWindowHours(hours));
    }

    if let Some(minutes) =
        normalize_prompt_cache_conversation_activity_minutes(params.activity_minutes)
    {
        return Ok(PromptCacheConversationSelection::ActivityWindowMinutes(
            minutes,
        ));
    }

    Ok(PromptCacheConversationSelection::Count(
        normalize_prompt_cache_conversation_limit(params.limit),
    ))
}

fn normalize_prompt_cache_conversation_page_size(raw: Option<i64>) -> Result<Option<i64>, ApiError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    if !(1..=100).contains(&value) {
        return Err(ApiError::bad_request(anyhow!(
            "pageSize must be between 1 and 100"
        )));
    }
    Ok(Some(value))
}

fn resolve_prompt_cache_conversation_detail_level(
    raw: Option<&str>,
) -> Result<PromptCacheConversationDetailLevel, ApiError> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(PromptCacheConversationDetailLevel::Full);
    };
    match value {
        "full" => Ok(PromptCacheConversationDetailLevel::Full),
        "compact" => Ok(PromptCacheConversationDetailLevel::Compact),
        _ => Err(ApiError::bad_request(anyhow!(
            "detail must be one of: full, compact"
        ))),
    }
}

fn resolve_prompt_cache_conversations_request(
    params: PromptCacheConversationsQuery,
) -> Result<PromptCacheConversationsRequest, ApiError> {
    let selection = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: params.limit,
        activity_hours: params.activity_hours,
        activity_minutes: params.activity_minutes,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
    })?;
    let detail_level = resolve_prompt_cache_conversation_detail_level(params.detail.as_deref())?;
    let normalized_page_size = normalize_prompt_cache_conversation_page_size(params.page_size)?;
    let uses_pagination =
        normalized_page_size.is_some() || params.cursor.is_some() || params.snapshot_at.is_some();

    if params.cursor.is_some() && params.snapshot_at.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "cursor requires snapshotAt"
        )));
    }

    if uses_pagination
        && !matches!(
            selection,
            PromptCacheConversationSelection::ActivityWindowMinutes(_)
        )
    {
        return Err(ApiError::bad_request(anyhow!(
            "pageSize, cursor, and snapshotAt are only supported for activityMinutes working conversations"
        )));
    }

    Ok(PromptCacheConversationsRequest {
        selection,
        detail_level,
        page_size: if uses_pagination {
            Some(normalized_page_size.unwrap_or(20))
        } else {
            normalized_page_size
        },
        cursor: params.cursor,
        snapshot_at: params.snapshot_at,
    })
}

pub(crate) fn resolve_prompt_cache_conversation_snapshot_at_with_default(
    raw: Option<&str>,
    default_now: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default_now);
    };
    parse_to_utc_datetime(value).ok_or_else(|| anyhow!("invalid snapshotAt: {value}"))
}

pub(crate) fn resolve_prompt_cache_conversation_snapshot_at(
    raw: Option<&str>,
) -> Result<DateTime<Utc>> {
    resolve_prompt_cache_conversation_snapshot_at_with_default(raw, Utc::now())
}

fn resolve_working_conversation_sort_anchor<'a>(
    last_terminal_at: Option<&'a str>,
    last_in_flight_at: Option<&'a str>,
    created_at: &'a str,
) -> &'a str {
    match (last_terminal_at, last_in_flight_at) {
        (Some(last_terminal_at), Some(last_in_flight_at)) => {
            if last_terminal_at >= last_in_flight_at {
                last_terminal_at
            } else {
                last_in_flight_at
            }
        }
        (Some(last_terminal_at), None) => last_terminal_at,
        (None, Some(last_in_flight_at)) => last_in_flight_at,
        (None, None) => created_at,
    }
}

fn encode_prompt_cache_conversation_cursor(
    sort_anchor_at: &str,
    created_at: &str,
    prompt_cache_key: &str,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> String {
    let payload = serde_json::to_vec(&(
        sort_anchor_at,
        created_at,
        prompt_cache_key,
        snapshot_boundary_row_id_ceiling,
    ))
    .expect("prompt cache cursor payload should serialize");
    base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload,
    )
}

fn decode_prompt_cache_conversation_cursor(raw: &str) -> Result<(String, String, String, Option<i64>)> {
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        raw.trim(),
    )
    .map_err(|err| anyhow!("invalid cursor encoding: {err}"))?;
    if let Ok((sort_anchor_at, created_at, prompt_cache_key, snapshot_boundary_row_id_ceiling)) =
        serde_json::from_slice::<(String, String, String, Option<i64>)>(&decoded)
    {
        let sort_anchor_at = sort_anchor_at.trim();
        let created_at = created_at.trim();
        let prompt_cache_key = prompt_cache_key.trim();
        if sort_anchor_at.is_empty() || created_at.is_empty() || prompt_cache_key.is_empty() {
            return Err(anyhow!("invalid cursor payload"));
        }
        return Ok((
            sort_anchor_at.to_string(),
            created_at.to_string(),
            prompt_cache_key.to_string(),
            snapshot_boundary_row_id_ceiling,
        ));
    }

    let decoded = String::from_utf8(decoded).map_err(|err| anyhow!("invalid cursor bytes: {err}"))?;
    let mut parts = decoded.splitn(4, '|');
    let Some(sort_anchor_at) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let Some(created_at) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let Some(prompt_cache_key) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let sort_anchor_at = sort_anchor_at.trim();
    let created_at = created_at.trim();
    let prompt_cache_key = prompt_cache_key.trim();
    if sort_anchor_at.is_empty() || created_at.is_empty() || prompt_cache_key.is_empty() {
        return Err(anyhow!("invalid cursor payload"));
    }
    let snapshot_boundary_row_id_ceiling = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|err| anyhow!("invalid cursor snapshot row id ceiling: {err}"))
        })
        .transpose()?;
    Ok((
        sort_anchor_at.to_string(),
        created_at.to_string(),
        prompt_cache_key.to_string(),
        snapshot_boundary_row_id_ceiling,
    ))
}

fn build_prompt_cache_conversation_cursor(
    row: &PromptCacheConversationAggregateRow,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> String {
    let sort_anchor_at = row.sort_anchor_at.as_deref().unwrap_or_else(|| {
        resolve_working_conversation_sort_anchor(
            row.last_terminal_at.as_deref(),
            row.last_in_flight_at.as_deref(),
            row.created_at.as_str(),
        )
    });
    encode_prompt_cache_conversation_cursor(
        sort_anchor_at,
        &row.created_at,
        &row.prompt_cache_key,
        snapshot_boundary_row_id_ceiling,
    )
}

async fn query_prompt_cache_conversation_snapshot_boundary_row_id_ceiling(
    pool: &Pool<Sqlite>,
    snapshot_boundary_second_start: &str,
    snapshot_upper_bound: &str,
    source_scope: InvocationSourceScope,
) -> Result<Option<i64>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT MAX(id) AS max_id \
         FROM codex_invocations \
         WHERE occurred_at >= ",
    );
    query
        .push_bind(snapshot_boundary_second_start)
        .push(" AND occurred_at < ")
        .push_bind(snapshot_upper_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let (max_id,) = query.build_query_as::<(Option<i64>,)>().fetch_one(pool).await?;
    Ok(max_id)
}

async fn resolve_prompt_cache_conversation_snapshot_filter(
    pool: &Pool<Sqlite>,
    snapshot_at: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    cursor_snapshot_boundary_row_id_ceiling: Option<i64>,
) -> Result<PromptCacheConversationSnapshotFilter> {
    let snapshot_upper_bound = db_occurred_at_upper_bound(snapshot_at);
    if snapshot_at.timestamp_subsec_nanos() == 0 {
        return Ok(PromptCacheConversationSnapshotFilter {
            snapshot_upper_bound,
            snapshot_boundary_second_start: None,
            snapshot_boundary_row_id_ceiling: None,
        });
    }

    let snapshot_boundary_second_start = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_at.timestamp(), 0)
            .single()
            .ok_or_else(|| anyhow!("invalid snapshotAt boundary second"))?,
    );
    let snapshot_boundary_row_id_ceiling = Some(match cursor_snapshot_boundary_row_id_ceiling {
        Some(value) => value,
        None => query_prompt_cache_conversation_snapshot_boundary_row_id_ceiling(
            pool,
            &snapshot_boundary_second_start,
            &snapshot_upper_bound,
            source_scope,
        )
        .await?
        .unwrap_or(0),
    });
    Ok(PromptCacheConversationSnapshotFilter {
        snapshot_upper_bound,
        snapshot_boundary_second_start: Some(snapshot_boundary_second_start),
        snapshot_boundary_row_id_ceiling,
    })
}

pub(crate) async fn fetch_prompt_cache_conversations_cached(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    loop {
        let mut wait_on: Option<watch::Receiver<bool>> = None;
        let mut flight_guard: Option<PromptCacheConversationFlightGuard> = None;
        let build_generation: u64;
        {
            let mut cache = state.prompt_cache_conversation_cache.lock().await;
            let generation = cache.generation;
            if let Some(entry) = cache.entries.get(&selection)
                && entry.generation == generation
                && entry.cached_at.elapsed()
                    <= Duration::from_secs(PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS)
            {
                return Ok(entry.response.clone());
            }

            let in_flight_generation = cache
                .in_flight
                .get(&selection)
                .map(|flight| flight.generation);
            match in_flight_generation {
                Some(current_generation) if current_generation == generation => {
                    if let Some(in_flight) = cache.in_flight.get(&selection) {
                        wait_on = Some(in_flight.signal.subscribe());
                    }
                }
                Some(_) => {
                    cache.in_flight.remove(&selection);
                }
                None => {}
            }

            if wait_on.is_none() {
                let (signal, _receiver) = watch::channel(false);
                cache.in_flight.insert(
                    selection,
                    PromptCacheConversationInFlight { signal, generation },
                );
                build_generation = generation;
                flight_guard = Some(PromptCacheConversationFlightGuard::new(
                    state.prompt_cache_conversation_cache.clone(),
                    selection,
                    generation,
                ));
            } else {
                build_generation = generation;
            }
        }

        if let Some(mut receiver) = wait_on {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let result = build_prompt_cache_conversations_response(state, selection).await;

        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }

        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        let stale_result = result.is_ok() && cache.generation != build_generation;
        let in_flight = match cache.in_flight.remove(&selection) {
            Some(in_flight) if in_flight.generation == build_generation => Some(in_flight),
            Some(in_flight) => {
                cache.in_flight.insert(selection, in_flight);
                None
            }
            None => None,
        };
        if let Some(in_flight) = in_flight {
            if let Ok(response) = &result {
                if !stale_result && cache.generation == build_generation {
                    cache.entries.insert(
                        selection,
                        PromptCacheConversationsCacheEntry {
                            cached_at: Instant::now(),
                            generation: build_generation,
                            response: response.clone(),
                        },
                    );
                }
            }
            let _ = in_flight.signal.send(true);
        }

        return result;
    }
}

fn compact_prompt_cache_conversations_response(
    mut response: PromptCacheConversationsResponse,
) -> PromptCacheConversationsResponse {
    for conversation in &mut response.conversations {
        conversation.upstream_accounts.clear();
        conversation.last24h_requests.clear();
        conversation
            .recent_invocations
            .truncate(2);
    }
    response
}

pub(crate) struct PromptCacheConversationHydrationSnapshot<'a> {
    snapshot_upper_bound: &'a str,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &'a str,
    snapshot_boundary_second_start: Option<&'a str>,
    snapshot_boundary_row_id_ceiling: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationSnapshotFilter {
    snapshot_upper_bound: String,
    snapshot_boundary_second_start: Option<String>,
    snapshot_boundary_row_id_ceiling: Option<i64>,
}

impl PromptCacheConversationSnapshotFilter {
    fn snapshot_upper_bound(&self) -> &str {
        self.snapshot_upper_bound.as_str()
    }

    fn boundary_second_start(&self) -> Option<&str> {
        self.snapshot_boundary_second_start.as_deref()
    }
}

fn push_snapshot_invocation_visibility_clause(
    query: &mut QueryBuilder<Sqlite>,
    occurred_at_expr: &str,
    id_expr: &str,
    snapshot: Option<&PromptCacheConversationSnapshotFilter>,
) {
    if let Some(snapshot) = snapshot {
        let snapshot_upper_bound = snapshot.snapshot_upper_bound().to_string();
        if let (Some(boundary_second_start), Some(row_id_ceiling)) = (
            snapshot.boundary_second_start().map(str::to_string),
            snapshot.snapshot_boundary_row_id_ceiling,
        ) {
            query
                .push("(")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(boundary_second_start.clone())
                .push(" OR (")
                .push(occurred_at_expr)
                .push(" >= ")
                .push_bind(boundary_second_start)
                .push(" AND ")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(snapshot_upper_bound)
                .push(" AND ")
                .push(id_expr)
                .push(" <= ")
                .push_bind(row_id_ceiling)
                .push("))");
            return;
        }

        query
            .push(occurred_at_expr)
            .push(" < ")
            .push_bind(snapshot_upper_bound);
    }
}

async fn hydrate_prompt_cache_conversations(
    state: &AppState,
    source_scope: InvocationSourceScope,
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    range_end: DateTime<Utc>,
    detail_level: PromptCacheConversationDetailLevel,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
) -> Result<Vec<PromptCacheConversationResponse>> {
    if aggregates.is_empty() {
        return Ok(Vec::new());
    }

    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let recent_invocation_limit = match detail_level {
        PromptCacheConversationDetailLevel::Full => {
            PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT as i64
        }
        PromptCacheConversationDetailLevel::Compact => 2,
    };

    let events = if detail_level == PromptCacheConversationDetailLevel::Full {
        let chart_range_start_bound = resolve_prompt_cache_conversation_chart_range_start(
            range_end,
            aggregates.iter().map(|row| row.created_at.as_str()).min(),
        );
        query_prompt_cache_conversation_events(
            &state.pool,
            &chart_range_start_bound,
            snapshot,
            source_scope,
            &selected_keys,
        )
        .await?
    } else {
        Vec::new()
    };

    let upstream_account_rows = if detail_level == PromptCacheConversationDetailLevel::Full {
        if let Some(snapshot) = snapshot {
            query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
                &state.pool,
                source_scope,
                &selected_keys,
                snapshot.snapshot_hour_start_epoch,
                snapshot.snapshot_hour_start_bound,
                snapshot,
            )
            .await?
        } else {
            query_prompt_cache_conversation_upstream_account_summaries(
                &state.pool,
                source_scope,
                &selected_keys,
            )
            .await?
        }
    } else {
        Vec::new()
    };

    let recent_invocation_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        source_scope,
        &selected_keys,
        recent_invocation_limit,
        snapshot,
    )
    .await?;

    let mut grouped_events: HashMap<String, Vec<PromptCacheConversationRequestPointResponse>> =
        HashMap::new();
    for row in events {
        let status = row.status.trim().to_string();
        let status = if status.is_empty() {
            "unknown".to_string()
        } else {
            status
        };
        let is_success = status.eq_ignore_ascii_case("success");
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.prompt_cache_key).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(PromptCacheConversationRequestPointResponse {
            occurred_at: row.occurred_at,
            status,
            is_success,
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut upstream_account_rows_by_key: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
    > = HashMap::new();
    for row in upstream_account_rows {
        upstream_account_rows_by_key
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(row);
    }
    let mut grouped_recent_invocations: HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    > = HashMap::new();
    for row in recent_invocation_rows {
        grouped_recent_invocations
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(PromptCacheConversationInvocationPreviewResponse {
                id: row.id,
                invoke_id: row.invoke_id,
                occurred_at: row.occurred_at,
                status: row.status,
                failure_class: normalize_trimmed_optional_string(row.failure_class),
                route_mode: normalize_trimmed_optional_string(row.route_mode),
                model: normalize_trimmed_optional_string(row.model),
                total_tokens: row.total_tokens.max(0),
                cost: row.cost,
                proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
                upstream_account_id: row.upstream_account_id,
                upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
                endpoint: normalize_trimmed_optional_string(row.endpoint),
                source: normalize_trimmed_optional_string(row.source),
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_input_tokens: row.cache_input_tokens,
                reasoning_tokens: row.reasoning_tokens,
                reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
                error_message: normalize_trimmed_optional_string(row.error_message),
                downstream_status_code: row.downstream_status_code,
                downstream_error_message: normalize_trimmed_optional_string(
                    row.downstream_error_message,
                ),
                failure_kind: normalize_trimmed_optional_string(row.failure_kind),
                is_actionable: row.is_actionable.map(|value| value != 0),
                response_content_encoding: normalize_trimmed_optional_string(
                    row.response_content_encoding,
                ),
                requested_service_tier: normalize_trimmed_optional_string(
                    row.requested_service_tier,
                ),
                service_tier: normalize_trimmed_optional_string(row.service_tier),
                billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
                t_req_read_ms: row.t_req_read_ms,
                t_req_parse_ms: row.t_req_parse_ms,
                t_upstream_connect_ms: row.t_upstream_connect_ms,
                t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
                t_upstream_stream_ms: row.t_upstream_stream_ms,
                t_resp_parse_ms: row.t_resp_parse_ms,
                t_persist_ms: row.t_persist_ms,
                t_total_ms: row.t_total_ms,
            });
    }

    let mut grouped_upstream_accounts: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountResponse>,
    > = HashMap::new();
    for (prompt_cache_key, rows) in upstream_account_rows_by_key {
        let mut unique_ids_by_name: HashMap<String, Option<i64>> = HashMap::new();
        for row in &rows {
            let Some(normalized_name) =
                normalize_trimmed_optional_string(row.upstream_account_name.clone())
            else {
                continue;
            };
            let Some(upstream_account_id) = row.upstream_account_id else {
                continue;
            };
            match unique_ids_by_name.entry(normalized_name) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(upstream_account_id));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry
                        .get()
                        .is_some_and(|existing_id| existing_id != upstream_account_id)
                    {
                        entry.insert(None);
                    }
                }
            }
        }

        let mut account_entries: HashMap<String, PromptCacheConversationUpstreamAccountResponse> =
            HashMap::new();
        for row in rows {
            let normalized_name =
                normalize_trimmed_optional_string(row.upstream_account_name.clone());
            let resolved_upstream_account_id = row.upstream_account_id.or_else(|| {
                normalized_name
                    .as_ref()
                    .and_then(|name| unique_ids_by_name.get(name).copied().flatten())
            });
            let account_group_key = resolve_prompt_cache_upstream_account_group_key(
                resolved_upstream_account_id,
                normalized_name.as_deref(),
            );
            let entry = account_entries.entry(account_group_key).or_insert_with(|| {
                PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: resolved_upstream_account_id,
                    upstream_account_name: normalized_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: row.last_activity_at.clone(),
                }
            });

            if entry.upstream_account_id.is_none() && resolved_upstream_account_id.is_some() {
                entry.upstream_account_id = resolved_upstream_account_id;
            }
            if entry.upstream_account_name.is_none() && normalized_name.is_some() {
                entry.upstream_account_name = normalized_name;
            }
            entry.request_count += row.request_count;
            entry.total_tokens += row.total_tokens.max(0);
            entry.total_cost += row.total_cost;
            if row.last_activity_at > entry.last_activity_at {
                entry.last_activity_at = row.last_activity_at;
            }
        }
        grouped_upstream_accounts.insert(
            prompt_cache_key,
            account_entries.into_values().collect::<Vec<_>>(),
        );
    }

    for accounts in grouped_upstream_accounts.values_mut() {
        accounts.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| {
                    resolve_prompt_cache_upstream_account_label(
                        right.upstream_account_name.as_deref(),
                        right.upstream_account_id,
                    )
                    .cmp(&resolve_prompt_cache_upstream_account_label(
                        left.upstream_account_name.as_deref(),
                        left.upstream_account_id,
                    ))
                })
                .then_with(|| {
                    right
                        .upstream_account_id
                        .unwrap_or(i64::MIN)
                        .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
                })
                .then_with(|| right.total_tokens.cmp(&left.total_tokens))
                .then_with(|| right.request_count.cmp(&left.request_count))
        });
        accounts.truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);
    }

    Ok(aggregates
        .into_iter()
        .map(|row| PromptCacheConversationResponse {
            prompt_cache_key: row.prompt_cache_key.clone(),
            request_count: row.request_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
            last_terminal_at: row.last_terminal_at,
            last_in_flight_at: row.last_in_flight_at,
            cursor: None,
            upstream_accounts: grouped_upstream_accounts
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
            recent_invocations: grouped_recent_invocations
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
            last24h_requests: grouped_events
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>())
}

pub(crate) async fn build_prompt_cache_conversations_response_for_request(
    state: &AppState,
    request: PromptCacheConversationsRequest,
) -> Result<PromptCacheConversationsResponse, ApiError> {
    if request.page_size.is_none() && request.cursor.is_none() && request.snapshot_at.is_none() {
        let response = build_prompt_cache_conversations_response(state, request.selection)
            .await
            .map_err(ApiError::from)?;
        return Ok(match request.detail_level {
            PromptCacheConversationDetailLevel::Full => response,
            PromptCacheConversationDetailLevel::Compact => {
                compact_prompt_cache_conversations_response(response)
            }
        });
    }

    let selection = request.selection;
    let PromptCacheConversationSelection::ActivityWindowMinutes(_) = selection else {
        return Err(ApiError::bad_request(anyhow!(
            "paginated prompt cache conversations only support activityMinutes working conversations"
        )));
    };
    let page_size = request.page_size.unwrap_or(20);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_at = resolve_prompt_cache_conversation_snapshot_at(request.snapshot_at.as_deref())
        .map_err(ApiError::bad_request)?;
    let range_end = snapshot_at;
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let snapshot_hour_start_epoch = align_bucket_epoch(range_end.timestamp(), 3_600, 0);
    let snapshot_hour_start_bound = db_occurred_at_lower_bound(
        Utc.timestamp_opt(snapshot_hour_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid snapshot hour start epoch"))?,
    );
    let cursor = request
        .cursor
        .as_deref()
        .map(decode_prompt_cache_conversation_cursor)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let snapshot_filter = resolve_prompt_cache_conversation_snapshot_filter(
        &state.pool,
        snapshot_at,
        source_scope,
        cursor
            .as_ref()
            .and_then(|(_, _, _, snapshot_boundary_row_id_ceiling)| {
                *snapshot_boundary_row_id_ceiling
            }),
    )
    .await?;
    let mut aggregates = query_prompt_cache_working_conversation_aggregates_page(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        snapshot_hour_start_epoch,
        &snapshot_hour_start_bound,
        source_scope,
        cursor.as_ref(),
        page_size + 1,
    )
    .await?;
    let has_more = aggregates.len() as i64 > page_size;
    if has_more {
        aggregates.truncate(page_size as usize);
    }
    let total_matched = query_working_prompt_cache_conversation_count_at_snapshot(
        &state.pool,
        &range_start_bound,
        &snapshot_filter,
        source_scope,
    )
    .await?;
    let next_cursor = if has_more {
        aggregates.last().map(|row| {
            build_prompt_cache_conversation_cursor(
                row,
                snapshot_filter.snapshot_boundary_row_id_ceiling,
            )
        })
    } else {
        None
    };
    let row_cursors_by_key = aggregates
        .iter()
        .map(|row| {
            (
                row.prompt_cache_key.clone(),
                build_prompt_cache_conversation_cursor(
                    row,
                    snapshot_filter.snapshot_boundary_row_id_ceiling,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let hydration_snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: snapshot_filter.snapshot_upper_bound(),
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound: &snapshot_hour_start_bound,
        snapshot_boundary_second_start: snapshot_filter.boundary_second_start(),
        snapshot_boundary_row_id_ceiling: snapshot_filter.snapshot_boundary_row_id_ceiling,
    };
    let mut conversations = hydrate_prompt_cache_conversations(
        state,
        source_scope,
        aggregates,
        range_end,
        request.detail_level,
        Some(&hydration_snapshot),
    )
    .await?;
    for conversation in &mut conversations {
        conversation.cursor = row_cursors_by_key
            .get(&conversation.prompt_cache_key)
            .cloned();
    }

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso_precise(range_end),
        snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        selected_activity_minutes: selection.selected_activity_minutes(),
        implicit_filter: PromptCacheConversationImplicitFilter {
            kind: None,
            filtered_count: 0,
        },
        total_matched: Some(total_matched),
        has_more,
        next_cursor,
        conversations,
    })
}

pub(crate) async fn build_prompt_cache_conversations_response(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let display_limit = selection.display_limit();

    let (aggregates, active_filtered_count) = match selection {
        PromptCacheConversationSelection::Count(limit) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let filtered_count = query_prompt_cache_conversation_hidden_count(
                &state.pool,
                &range_start_bound,
                source_scope,
                limit,
                aggregates.len() as i64,
            )
            .await?;
            (aggregates, filtered_count)
        }
        PromptCacheConversationSelection::ActivityWindowHours(_) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_active_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(_) => {
            let aggregates = query_prompt_cache_working_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_working_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
    };
    let implicit_filter = selection.implicit_filter(active_filtered_count);

    if aggregates.is_empty() {
        return Ok(PromptCacheConversationsResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            snapshot_at: None,
            selection_mode: selection.selection_mode(),
            selected_limit: selection.selected_limit(),
            selected_activity_hours: selection.selected_activity_hours(),
            selected_activity_minutes: selection.selected_activity_minutes(),
            implicit_filter,
            total_matched: None,
            has_more: false,
            next_cursor: None,
            conversations: Vec::new(),
        });
    }
    let conversations = hydrate_prompt_cache_conversations(
        state,
        source_scope,
        aggregates,
        range_end,
        PromptCacheConversationDetailLevel::Full,
        None,
    )
    .await?;

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        snapshot_at: None,
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        selected_activity_minutes: selection.selected_activity_minutes(),
        implicit_filter,
        total_matched: None,
        has_more: false,
        next_cursor: None,
        conversations,
    })
}

fn resolve_prompt_cache_conversation_chart_range_start(
    range_end: DateTime<Utc>,
    earliest_created_at: Option<&str>,
) -> String {
    let floor = range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS);
    let created_at = earliest_created_at
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc));
    let chart_start = match created_at {
        Some(created_at) if created_at > floor => created_at,
        _ => floor,
    };
    format_utc_iso(chart_start)
}

fn normalize_trimmed_optional_string(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_prompt_cache_upstream_account_label(
    upstream_account_name: Option<&str>,
    upstream_account_id: Option<i64>,
) -> String {
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if let Some(account_id) = upstream_account_id {
        return format!("账号 #{account_id}");
    }
    "—".to_string()
}

fn resolve_prompt_cache_upstream_account_group_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    if let Some(account_id) = upstream_account_id {
        return format!("id:{account_id}");
    }
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("name:{name}");
    }
    "unknown".to_string()
}

pub(crate) async fn query_prompt_cache_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT prompt_cache_key, MIN(first_seen_at) AS first_seen_24h \
             FROM prompt_cache_rollup_hourly \
             WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT prompt_cache_key, \
                 SUM(request_count) AS request_count, \
                 SUM(total_tokens) AS total_tokens, \
                 SUM(total_cost) AS total_cost, \
                 MIN(first_seen_at) AS created_at, \
                 MAX(last_seen_at) AS last_activity_at \
             FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM active)",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
         FROM aggregates \
         ORDER BY created_at DESC, prompt_cache_key DESC \
         LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_active_prompt_cache_conversation_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(DISTINCT prompt_cache_key) AS count \
         FROM prompt_cache_rollup_hourly \
         WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_working_prompt_cache_conversation_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key FROM recent_terminal \
            UNION \
            SELECT prompt_cache_key FROM in_flight\
         ) \
         SELECT COUNT(*) AS count FROM working",
    );

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_working_prompt_cache_conversation_count_at_snapshot(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: &PromptCacheConversationSnapshotFilter,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE ",
        );
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key FROM recent_terminal \
            UNION \
            SELECT prompt_cache_key FROM in_flight\
         ) \
         SELECT COUNT(*) AS count FROM working",
    );

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_prompt_cache_conversation_hidden_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    requested_limit: i64,
    selected_active_count: i64,
) -> Result<i64> {
    if requested_limit <= 0 {
        return Ok(0);
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT DISTINCT prompt_cache_key \
         FROM prompt_cache_rollup_hourly \
         WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " ), history AS (\
            SELECT prompt_cache_key, MIN(first_seen_at) AS created_at \
             FROM prompt_cache_rollup_hourly",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" WHERE source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), ranked AS (\
            SELECT history.prompt_cache_key, \
                   CASE WHEN active.prompt_cache_key IS NULL THEN 0 ELSE 1 END AS is_active, \
                   ROW_NUMBER() OVER (\
                       ORDER BY history.created_at DESC, history.prompt_cache_key DESC\
                   ) AS history_rank, \
                   SUM(CASE WHEN active.prompt_cache_key IS NULL THEN 0 ELSE 1 END) OVER (\
                       ORDER BY history.created_at DESC, history.prompt_cache_key DESC \
                       ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW\
                   ) AS active_rank \
            FROM history \
            LEFT JOIN active ON active.prompt_cache_key = history.prompt_cache_key\
         ) \
         SELECT COUNT(*) AS count \
         FROM ranked \
         WHERE is_active = 0 AND ((",
    );
    query
        .push_bind(selected_active_count)
        .push(" < ")
        .push_bind(requested_limit)
        .push(" AND history_rank <= ")
        .push_bind(requested_limit)
        .push(") OR (")
        .push_bind(selected_active_count)
        .push(" >= ")
        .push_bind(requested_limit)
        .push(" AND active_rank < ")
        .push_bind(requested_limit)
        .push("))");

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_prompt_cache_working_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_terminal_at \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_in_flight_at \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key, last_terminal_at, NULL AS last_in_flight_at \
              FROM recent_terminal \
            UNION ALL \
            SELECT prompt_cache_key, NULL AS last_terminal_at, last_in_flight_at \
              FROM in_flight \
         ), collapsed_working AS (\
            SELECT prompt_cache_key, \
                   MAX(last_terminal_at) AS last_terminal_at, \
                   MAX(last_in_flight_at) AS last_in_flight_at, \
                   CASE \
                       WHEN MAX(last_terminal_at) IS NULL THEN MAX(last_in_flight_at) \
                       WHEN MAX(last_in_flight_at) IS NULL THEN MAX(last_terminal_at) \
                       WHEN MAX(last_terminal_at) >= MAX(last_in_flight_at) THEN MAX(last_terminal_at) \
                       ELSE MAX(last_in_flight_at) \
                   END AS sort_anchor_at \
              FROM working \
              GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT prompt_cache_key, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MIN(first_seen_at) AS created_at, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM collapsed_working)",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT aggregates.prompt_cache_key, aggregates.request_count, aggregates.total_tokens, \
                aggregates.total_cost, aggregates.created_at, aggregates.last_activity_at \
           FROM aggregates \
           INNER JOIN collapsed_working ON collapsed_working.prompt_cache_key = aggregates.prompt_cache_key \
          ORDER BY collapsed_working.sort_anchor_at DESC, aggregates.created_at DESC, aggregates.prompt_cache_key DESC \
          LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_working_conversation_aggregates_page(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: &PromptCacheConversationSnapshotFilter,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
    source_scope: InvocationSourceScope,
    cursor: Option<&(String, String, String, Option<i64>)>,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_terminal_at \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_in_flight_at \
             FROM codex_invocations \
             WHERE ",
        );
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key, last_terminal_at, NULL AS last_in_flight_at \
              FROM recent_terminal \
            UNION ALL \
            SELECT prompt_cache_key, NULL AS last_terminal_at, last_in_flight_at \
              FROM in_flight \
         ), collapsed_working AS (\
            SELECT prompt_cache_key, \
                   MAX(last_terminal_at) AS last_terminal_at, \
                   MAX(last_in_flight_at) AS last_in_flight_at, \
                   CASE \
                       WHEN MAX(last_terminal_at) IS NULL THEN MAX(last_in_flight_at) \
                       WHEN MAX(last_in_flight_at) IS NULL THEN MAX(last_terminal_at) \
                       WHEN MAX(last_terminal_at) >= MAX(last_in_flight_at) THEN MAX(last_terminal_at) \
                       ELSE MAX(last_in_flight_at) \
                   END AS sort_anchor_at \
              FROM working \
              GROUP BY prompt_cache_key\
         ), history_rollup AS (\
            SELECT prompt_cache_key, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MIN(first_seen_at) AS created_at, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM collapsed_working) \
               AND bucket_start_epoch < ",
    );
    query.push_bind(snapshot_hour_start_epoch);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), history_live AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens, \
                   COALESCE(SUM(COALESCE(cost, 0)), 0) AS total_cost, \
                   MIN(occurred_at) AS created_at, \
                   MAX(occurred_at) AS last_activity_at \
              FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(snapshot_hour_start_bound)
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (SELECT prompt_cache_key FROM collapsed_working)");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), history_inputs AS (\
            SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
              FROM history_rollup \
            UNION ALL \
            SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
              FROM history_live\
         ), history_aggregates AS (\
            SELECT prompt_cache_key, \
                   COALESCE(SUM(request_count), 0) AS request_count, \
                   COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                   COALESCE(SUM(total_cost), 0.0) AS total_cost, \
                   MIN(created_at) AS created_at, \
                   MAX(last_activity_at) AS last_activity_at \
              FROM history_inputs \
             GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT collapsed_working.prompt_cache_key AS prompt_cache_key, \
                   COALESCE(history_aggregates.request_count, 0) AS request_count, \
                   COALESCE(history_aggregates.total_tokens, 0) AS total_tokens, \
                   COALESCE(history_aggregates.total_cost, 0.0) AS total_cost, \
                   COALESCE(history_aggregates.created_at, collapsed_working.sort_anchor_at) AS created_at, \
                   COALESCE(history_aggregates.last_activity_at, collapsed_working.sort_anchor_at) AS last_activity_at, \
                   collapsed_working.sort_anchor_at AS sort_anchor_at, \
                   collapsed_working.last_terminal_at AS last_terminal_at, \
                   collapsed_working.last_in_flight_at AS last_in_flight_at \
              FROM collapsed_working \
              LEFT JOIN history_aggregates ON history_aggregates.prompt_cache_key = collapsed_working.prompt_cache_key",
    );

    query.push(
        " ) \
         SELECT aggregates.prompt_cache_key, aggregates.request_count, aggregates.total_tokens, \
                aggregates.total_cost, aggregates.created_at, aggregates.last_activity_at, \
                aggregates.sort_anchor_at, aggregates.last_terminal_at, \
                aggregates.last_in_flight_at \
           FROM aggregates",
    );

    if let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor {
        query
            .push(" WHERE (aggregates.sort_anchor_at < ")
            .push_bind(cursor_sort_anchor_at)
            .push(" OR (aggregates.sort_anchor_at = ")
            .push_bind(cursor_sort_anchor_at)
            .push(" AND (aggregates.created_at < ")
            .push_bind(cursor_created_at)
            .push(" OR (aggregates.created_at = ")
            .push_bind(cursor_created_at)
            .push(" AND aggregates.prompt_cache_key < ")
            .push_bind(cursor_prompt_cache_key)
            .push("))))");
    }

    query
        .push(
            " ORDER BY aggregates.sort_anchor_at DESC, aggregates.created_at DESC, aggregates.prompt_cache_key DESC \
              LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_events(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, \
         COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ");
    if let Some(snapshot) = snapshot {
        let snapshot_filter = PromptCacheConversationSnapshotFilter {
            snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
            snapshot_boundary_second_start: snapshot
                .snapshot_boundary_second_start
                .map(ToOwned::to_owned),
            snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
        };
        push_snapshot_invocation_visibility_clause(
            &mut query,
            "occurred_at",
            "id",
            Some(&snapshot_filter),
        );
        query.push(" AND ");
    }
    query
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(" ORDER BY prompt_cache_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<PromptCacheConversationEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_recent_invocations(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    limit_per_key: i64,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
) -> Result<Vec<PromptCacheConversationInvocationPreviewRow>> {
    if selected_keys.is_empty() || limit_per_key <= 0 {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    let mut query =
        QueryBuilder::<Sqlite>::new("WITH ranked AS (SELECT id, invoke_id, occurred_at, ");
    query
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
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
        .push(
            " AS billing_service_tier, \
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ROW_NUMBER() OVER (PARTITION BY ")
        .push(KEY_EXPR)
        .push(" ORDER BY occurred_at DESC, id DESC) AS row_number FROM codex_invocations WHERE ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if let Some(snapshot) = snapshot {
        let snapshot_filter = PromptCacheConversationSnapshotFilter {
            snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
            snapshot_boundary_second_start: snapshot
                .snapshot_boundary_second_start
                .map(ToOwned::to_owned),
            snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
        };
        query.push(" AND ");
        push_snapshot_invocation_visibility_clause(
            &mut query,
            "occurred_at",
            "id",
            Some(&snapshot_filter),
        );
    }

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(") SELECT prompt_cache_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, downstream_status_code, downstream_error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, requested_service_tier, service_tier, billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint FROM ranked WHERE row_number <= ")
        .push_bind(limit_per_key)
        .push(" ORDER BY prompt_cache_key ASC, occurred_at DESC, id DESC");

    query
        .build_query_as::<PromptCacheConversationInvocationPreviewRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
    snapshot: &PromptCacheConversationHydrationSnapshot<'_>,
) -> Result<Vec<PromptCacheConversationUpstreamAccountSummaryRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    const UPSTREAM_ACCOUNT_ID_EXPR: &str =
        "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    const UPSTREAM_ACCOUNT_NAME_EXPR: &str =
        "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END";
    const UPSTREAM_ACCOUNT_KEY_EXPR: &str =
        "CASE \
            WHEN CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END <> '' \
              THEN 'id:' || CAST(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS TEXT) || '|name:' || CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END \
            WHEN CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END IS NOT NULL \
              THEN 'id:' || CAST(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS TEXT) \
            WHEN CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END <> '' \
              THEN 'name:' || CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END \
            ELSE 'unknown' \
         END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH historical AS (\
            SELECT prompt_cache_key, \
                   upstream_account_key, \
                   upstream_account_id, \
                   upstream_account_name, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_upstream_account_hourly \
             WHERE prompt_cache_key IN (",
    );

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }

    query
        .push(") AND bucket_start_epoch < ")
        .push_bind(snapshot_hour_start_epoch);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name\
         ), current_hour_live AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ")
        .push(UPSTREAM_ACCOUNT_KEY_EXPR)
        .push(
            " AS upstream_account_key, ",
        )
        .push(UPSTREAM_ACCOUNT_ID_EXPR)
        .push(" AS upstream_account_id, ")
        .push(UPSTREAM_ACCOUNT_NAME_EXPR)
        .push(
            " AS upstream_account_name, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens, \
                   COALESCE(SUM(COALESCE(cost, 0)), 0) AS total_cost, \
                   MAX(occurred_at) AS last_activity_at \
              FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(snapshot_hour_start_bound)
        .push(" AND ");
    let snapshot_filter = PromptCacheConversationSnapshotFilter {
        snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
        snapshot_boundary_second_start: snapshot
            .snapshot_boundary_second_start
            .map(ToOwned::to_owned),
        snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
    };
    push_snapshot_invocation_visibility_clause(
        &mut query,
        "occurred_at",
        "id",
        Some(&snapshot_filter),
    );
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(" GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name\
         ), combined AS (\
            SELECT prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name, request_count, total_tokens, total_cost, last_activity_at \
              FROM historical \
            UNION ALL \
            SELECT prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name, request_count, total_tokens, total_cost, last_activity_at \
              FROM current_hour_live\
         ) \
         SELECT prompt_cache_key, \
                upstream_account_id, \
                upstream_account_name, \
                SUM(request_count) AS request_count, \
                SUM(total_tokens) AS total_tokens, \
                SUM(total_cost) AS total_cost, \
                MAX(last_activity_at) AS last_activity_at \
           FROM combined \
          GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name \
          ORDER BY prompt_cache_key ASC, last_activity_at DESC, upstream_account_name DESC, upstream_account_id DESC");

    query
        .build_query_as::<PromptCacheConversationUpstreamAccountSummaryRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_upstream_account_summaries(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationUpstreamAccountSummaryRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT prompt_cache_key, \
             upstream_account_id, \
             upstream_account_name, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MAX(last_seen_at) AS last_activity_at \
         FROM prompt_cache_upstream_account_hourly \
         WHERE prompt_cache_key IN (",
    );

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name \
              ORDER BY prompt_cache_key ASC, last_activity_at DESC, upstream_account_name DESC, upstream_account_id DESC",
        )
        .build_query_as::<PromptCacheConversationUpstreamAccountSummaryRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let bucket_selection = resolve_timeseries_bucket_selection(
        &params,
        &range_window,
        state.config.invocation_max_days,
    )?;
    let bucket_seconds = bucket_selection.bucket_seconds;

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned {
            if needs_historical_rollups {
                return Err(ApiError::bad_request(anyhow!(
                    "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
                )));
            }
        } else {
            return fetch_timeseries_from_hourly_rollups(
                state,
                params,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
            )
            .await;
        }
    }

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
    );
    records_query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        records_query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    records_query.push(" ORDER BY occurred_at ASC");
    let records = records_query
        .build_query_as::<TimeseriesRecord>()
        .fetch_all(&state.pool)
        .await?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    let start_epoch = start_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        let bucket_epoch = align_reporting_bucket_epoch(epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            Some(_) => entry.failure_count += 1,
            None => {}
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        query_crs_deltas(&state.pool, relay, start_epoch, end_dt.timestamp()).await?
    } else {
        Vec::new()
    };

    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += delta.total_count;
        entry.success_count += delta.success_count;
        entry.failure_count += delta.failure_count;
        entry.total_tokens += delta.total_tokens;
        entry.total_cost += delta.total_cost;
    }

    // Fill every bucket that intersects the requested range using reporting-timezone
    // boundaries rather than fixed UTC-duration strides. This keeps DST transition
    // days aligned to local clock buckets.
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = next_reporting_bucket_epoch(
        align_reporting_bucket_epoch(end_dt.timestamp(), bucket_seconds, reporting_tz)?,
        bucket_seconds,
        reporting_tz,
    )?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
        let first_response_byte_total_avg_ms = agg.first_response_byte_total_avg_ms();
        let first_response_byte_total_p95_ms = agg.first_response_byte_total_p95_ms();
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms,
            first_byte_p95_ms,
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms,
            first_response_byte_total_p95_ms,
        });
    }

    let response = TimeseriesResponse {
        range_start: start_str_iso,
        range_end: {
            let end = Utc
                .timestamp_opt(fill_end_epoch, 0)
                .single()
                .unwrap_or_else(Utc::now);
            format_utc_iso(end)
        },
        bucket_seconds,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    };

    Ok(Json(response))
}

pub(crate) async fn fetch_parallel_work_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let now = Utc::now();

    let minute7d_window = resolve_complete_parallel_work_window(
        now,
        ChronoDuration::days(7),
        60,
        requested_reporting_tz,
    )?;
    let requested_hour30d_window = resolve_complete_parallel_work_window(
        now,
        ChronoDuration::days(30),
        3_600,
        requested_reporting_tz,
    )?;
    let (hour30d_reporting_tz, hour30d_time_zone_fallback) =
        resolve_parallel_work_rollup_reporting_tz(
            requested_reporting_tz,
            &requested_hour30d_window,
        );
    let hour30d_window = if hour30d_time_zone_fallback {
        resolve_complete_parallel_work_window(
            now,
            ChronoDuration::days(30),
            3_600,
            hour30d_reporting_tz,
        )?
    } else {
        requested_hour30d_window
    };

    let minute7d_counts = query_parallel_work_minute_counts(
        &state.pool,
        minute7d_window.start,
        minute7d_window.end,
        requested_reporting_tz,
        source_scope,
    )
    .await?;
    let hour30d_counts = query_parallel_work_hourly_counts(
        &state.pool,
        hour30d_window.start,
        hour30d_window.end,
        source_scope,
    )
    .await?;

    let (day_all_window, day_all_reporting_tz, day_all_time_zone_fallback) =
        resolve_parallel_work_day_all_window_with_fallback(
            &state.pool,
            requested_reporting_tz,
            source_scope,
        )
        .await?;
    let day_all_counts = if let Some(window) = day_all_window.as_ref() {
        query_parallel_work_day_counts_from_hourly_rollups(
            &state.pool,
            window.start,
            window.end,
            day_all_reporting_tz,
            source_scope,
        )
        .await?
    } else {
        BTreeMap::new()
    };

    let latest_complete_day_end = local_midnight_utc(
        now.with_timezone(&day_all_reporting_tz).date_naive(),
        day_all_reporting_tz,
    );

    Ok(Json(ParallelWorkStatsResponse {
        minute7d: build_parallel_work_window_response(
            minute7d_window.start,
            minute7d_window.end,
            60,
            requested_reporting_tz,
            &minute7d_counts,
            requested_reporting_tz,
            false,
        )?,
        hour30d: build_parallel_work_window_response(
            hour30d_window.start,
            hour30d_window.end,
            3_600,
            hour30d_reporting_tz,
            &hour30d_counts,
            hour30d_reporting_tz,
            hour30d_time_zone_fallback,
        )?,
        day_all: if let Some(window) = day_all_window {
            build_parallel_work_window_response(
                window.start,
                window.end,
                86_400,
                day_all_reporting_tz,
                &day_all_counts,
                day_all_reporting_tz,
                day_all_time_zone_fallback,
            )?
        } else {
            empty_parallel_work_window_response(
                latest_complete_day_end,
                86_400,
                day_all_reporting_tz,
                day_all_time_zone_fallback,
            )
        },
    }))
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_epoch = range_window.start.timestamp();
    let end_epoch = range_window.end.timestamp();
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = next_reporting_bucket_epoch(
        align_reporting_bucket_epoch(end_epoch, bucket_seconds, reporting_tz)?,
        bucket_seconds,
        reporting_tz,
    )?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    if let Some((hourly_cursor, hourly_end_epoch)) = range_plan.full_hour_range {
        let rows = query_invocation_hourly_rollup_range(
            &state.pool,
            hourly_cursor,
            hourly_end_epoch,
            source_scope,
        )
        .await?;
        for row in rows {
            let bucket_epoch =
                align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
            let entry = aggregates.entry(bucket_epoch).or_default();
            entry.total_count += row.total_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.total_tokens += row.total_tokens;
            entry.total_cost += row.total_cost;
            entry.first_byte_sample_count += row.first_byte_sample_count;
            entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
            entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
                decode_approx_histogram(&row.first_byte_histogram)
            } else {
                let mut merged = entry.first_byte_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_byte_histogram),
                )?;
                merged
            };
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_response_byte_total_histogram =
                if entry.first_response_byte_total_histogram.is_empty() {
                    decode_approx_histogram(&row.first_response_byte_total_histogram)
                } else {
                    let mut merged = entry.first_response_byte_total_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_response_byte_total_histogram),
                    )?;
                    merged
                };
        }
    }

    let exact_records =
        query_invocation_exact_records(&state.pool, &range_plan, source_scope).await?;
    for record in exact_records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += 1;
            match record.status.as_deref() {
                Some("success") => entry.success_count += 1,
                Some(_) => entry.failure_count += 1,
                None => {}
            }
            entry.record_exact_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
            entry.record_exact_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
            entry.total_tokens += record.total_tokens.unwrap_or_default();
            entry.total_cost += record.cost.unwrap_or_default();
        }
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
        && let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)?
    {
        query_crs_deltas(
            &state.pool,
            relay,
            effective_range.start.timestamp(),
            effective_range.end.timestamp(),
        )
        .await?
    } else {
        Vec::new()
    };
    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += delta.total_count;
            entry.success_count += delta.success_count;
            entry.failure_count += delta.failure_count;
            entry.total_tokens += delta.total_tokens;
            entry.total_cost += delta.total_cost;
        }
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms: agg.first_byte_avg_ms(),
            first_byte_p95_ms: agg.first_byte_p95_ms(),
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms: agg.first_response_byte_total_avg_ms(),
            first_response_byte_total_p95_ms: agg.first_response_byte_total_p95_ms(),
        });
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: {
            let end = Utc
                .timestamp_opt(fill_end_epoch, 0)
                .single()
                .unwrap_or_else(Utc::now);
            format_utc_iso(end)
        },
        bucket_seconds,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}
