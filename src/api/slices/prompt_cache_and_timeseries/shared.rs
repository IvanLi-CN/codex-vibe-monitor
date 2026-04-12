use super::*;

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

pub(super) fn build_hourly_rollup_exact_range_plan(
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

pub(super) fn effective_range_for_hourly_rollup_plan(
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

pub(super) fn resolve_timeseries_fill_end_epoch(
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<i64, ApiError> {
    let aligned_end_epoch =
        align_reporting_bucket_epoch(end_dt.timestamp(), bucket_seconds, reporting_tz)?;
    if end_dt.timestamp_subsec_nanos() == 0 && aligned_end_epoch == end_dt.timestamp() {
        return Ok(aligned_end_epoch);
    }
    Ok(next_reporting_bucket_epoch(
        aligned_end_epoch,
        bucket_seconds,
        reporting_tz,
    )?)
}

pub(super) fn invocation_status_is_success_like(status: Option<&str>, error_message: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    let error_message_empty = error_message.map(str::trim).is_none_or(str::is_empty);

    normalized_status.eq_ignore_ascii_case("success")
        || normalized_status.eq_ignore_ascii_case("completed")
        || (normalized_status.eq_ignore_ascii_case("http_200") && error_message_empty)
}

pub(super) fn invocation_status_counts_toward_terminal_totals(status: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    !normalized_status.eq_ignore_ascii_case("running")
        && !normalized_status.eq_ignore_ascii_case("pending")
}

pub(super) fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    normalized_status.eq_ignore_ascii_case("running")
        || normalized_status.eq_ignore_ascii_case("pending")
}

fn invocation_metadata_has_failure_text(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|text| !text.is_empty())
}

fn invocation_point_has_explicit_failure_metadata(
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
) -> bool {
    if failure_class
        .map(str::trim)
        .unwrap_or_default()
        .eq_ignore_ascii_case("none")
    {
        return invocation_metadata_has_failure_text(error_message)
            || invocation_metadata_has_failure_text(downstream_error_message)
            || failure_kind
                .map(str::trim)
                .is_some_and(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"));
    }

    true
}

fn invocation_point_is_success(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_class: Option<&str>,
) -> bool {
    invocation_status_is_success_like(status, error_message)
        && failure_class
            .map(str::trim)
            .unwrap_or_default()
            .eq_ignore_ascii_case("none")
}

pub(super) fn invocation_point_outcome(
    status: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
) -> &'static str {
    if invocation_point_is_success(status, error_message, failure_class) {
        return "success";
    }
    if invocation_point_has_explicit_failure_metadata(
        error_message,
        downstream_error_message,
        failure_kind,
        failure_class,
    ) {
        return "failure";
    }
    if invocation_status_is_in_flight(status) {
        return "in_flight";
    }
    if status
        .map(str::trim)
        .is_some_and(|value| value.starts_with("http_4") || value.starts_with("http_5"))
    {
        return "failure";
    }
    "neutral"
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

async fn query_invocation_aggregate_records_from_live_range_executor<'e, E>(
    executor: E,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
) -> Result<Vec<InvocationAggregateRecord>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, occurred_at, status, total_tokens, cost, error_message, ",
    );
    query
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(
            " AS failure_kind, \
            ",
        )
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(
            " = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, \
            t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE occurred_at >= ",
        );
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if let Some(start_after_id) = start_after_id {
        query.push(" AND id > ").push_bind(start_after_id);
    }
    if let Some(snapshot_id) = snapshot_id {
        query.push(" AND id <= ").push_bind(snapshot_id);
    }
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at ASC, id ASC");
    query
        .build_query_as::<InvocationAggregateRecord>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(super) async fn query_invocation_aggregate_records_from_live_range(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    query_invocation_aggregate_records_from_live_range_executor(
        pool,
        range,
        source_scope,
        start_after_id,
        snapshot_id,
    )
    .await
}

async fn query_invocation_aggregate_records_from_live_range_tx(
    tx: &mut SqliteConnection,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    query_invocation_aggregate_records_from_live_range_executor(
        &mut *tx,
        range,
        source_scope,
        start_after_id,
        snapshot_id,
    )
    .await
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

pub(super) async fn query_invocation_exact_records(
    pool: &Pool<Sqlite>,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for range in &range_plan.live_exact_ranges {
        extend_unique_invocation_records(
            &mut records,
            &mut seen_ids,
            query_invocation_aggregate_records_from_live_range(
                pool,
                *range,
                source_scope,
                None,
                Some(snapshot_id),
            )
            .await?,
        );
    }

    records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

pub(crate) async fn query_invocation_exact_records_tx(
    tx: &mut SqliteConnection,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for range in &range_plan.live_exact_ranges {
        extend_unique_invocation_records(
            &mut records,
            &mut seen_ids,
            query_invocation_aggregate_records_from_live_range_tx(
                &mut *tx,
                *range,
                source_scope,
                None,
                Some(snapshot_id),
            )
            .await?,
        );
    }

    records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

pub(crate) async fn query_invocation_full_hour_tail_records_tx(
    tx: &mut SqliteConnection,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
    rollup_live_cursor: i64,
    snapshot_id: i64,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    if snapshot_id <= rollup_live_cursor {
        return Ok(Vec::new());
    }
    let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range else {
        return Ok(Vec::new());
    };
    let range = ExactUtcRange {
        start: Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour tail start epoch")))?,
        end: Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour tail end epoch")))?,
    };
    query_invocation_aggregate_records_from_live_range_tx(
        &mut *tx,
        range,
        source_scope,
        Some(rollup_live_cursor),
        Some(snapshot_id),
    )
    .await
}

pub(crate) async fn load_invocation_summary_rollup_live_cursor_tx(
    tx: &mut SqliteConnection,
) -> Result<i64> {
    Ok(
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await?
            .max(
                load_hourly_rollup_live_progress_tx(
                    tx,
                    INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
                )
                .await?,
            ),
    )
}

pub(crate) async fn resolve_invocation_snapshot_id_tx(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
) -> Result<i64, ApiError> {
    #[derive(Debug, FromRow)]
    struct SnapshotRow {
        snapshot_id: Option<i64>,
    }

    let mut query =
        QueryBuilder::new("SELECT MAX(id) AS snapshot_id FROM codex_invocations WHERE 1 = 1");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let row = query
        .build_query_as::<SnapshotRow>()
        .fetch_one(&mut *tx)
        .await?;
    Ok(row.snapshot_id.unwrap_or(0))
}

pub(super) async fn query_invocation_hourly_rollup_range_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationHourlyRollupRecord>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<InvocationHourlyRollupRecord>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(super) fn add_invocation_record_to_summary_totals(
    totals: &mut StatsTotals,
    record: &InvocationAggregateRecord,
) {
    totals.total_count += 1;
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    );
    if invocation_status_is_success_like(record.status.as_deref(), record.error_message.as_deref())
        && classification.failure_class == FailureClass::None
    {
        totals.success_count += 1;
    } else if invocation_status_counts_toward_terminal_totals(record.status.as_deref())
        && classification.failure_class != FailureClass::None
    {
        totals.failure_count += 1;
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

pub(super) fn record_perf_stage_sample(
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
