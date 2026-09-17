pub(crate) async fn fetch_failure_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FailureSummaryQuery>,
) -> Result<Json<FailureSummaryResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if start_dt < shanghai_retention_cutoff(state.config.invocation_max_days) {
        return Ok(Json(
            build_historical_failure_summary(&state, start_dt, display_end, source_scope).await?,
        ));
    }

    #[derive(sqlx::FromRow)]
    struct Row {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let rows: Vec<Row> = query.build_query_as().fetch_all(&state.pool).await?;
    let mut total_failures = 0_i64;
    let mut service_failure_count = 0_i64;
    let mut client_failure_count = 0_i64;
    let mut client_abort_count = 0_i64;
    let mut actionable_failure_count = 0_i64;

    for row in rows {
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        if classification.failure_class == FailureClass::None {
            continue;
        }
        total_failures += 1;
        match classification.failure_class {
            FailureClass::ServiceFailure => service_failure_count += 1,
            FailureClass::ClientFailure => client_failure_count += 1,
            FailureClass::ClientAbort => client_abort_count += 1,
            FailureClass::None => {}
        }
        if classification.is_actionable {
            actionable_failure_count += 1;
        }
    }

    let actionable_failure_rate = if total_failures > 0 {
        actionable_failure_count as f64 / total_failures as f64
    } else {
        0.0
    };

    Ok(Json(FailureSummaryResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        total_failures,
        service_failure_count,
        client_failure_count,
        client_abort_count,
        actionable_failure_count,
        actionable_failure_rate,
    }))
}

#[derive(Default)]
struct FailureSummaryCounts {
    total_failures: i64,
    service_failure_count: i64,
    client_failure_count: i64,
    client_abort_count: i64,
    actionable_failure_count: i64,
}

impl FailureSummaryCounts {
    fn add_classification(&mut self, classification: FailureClassification) {
        if classification.failure_class == FailureClass::None {
            return;
        }
        self.total_failures += 1;
        match classification.failure_class {
            FailureClass::ServiceFailure => self.service_failure_count += 1,
            FailureClass::ClientFailure => self.client_failure_count += 1,
            FailureClass::ClientAbort => self.client_abort_count += 1,
            FailureClass::None => {}
        }
        if classification.is_actionable {
            self.actionable_failure_count += 1;
        }
    }

    fn add_rollup(&mut self, row: &InvocationFailureHourlyRollupRecord) {
        let Some(classification) = FailureClass::from_db_str(&row.failure_class) else {
            return;
        };
        if classification == FailureClass::None {
            return;
        }
        self.total_failures += row.failure_count;
        match classification {
            FailureClass::ServiceFailure => self.service_failure_count += row.failure_count,
            FailureClass::ClientFailure => self.client_failure_count += row.failure_count,
            FailureClass::ClientAbort => self.client_abort_count += row.failure_count,
            FailureClass::None => {}
        }
        if row.is_actionable != 0 {
            self.actionable_failure_count += row.failure_count;
        }
    }

    fn response(
        self,
        start_dt: DateTime<Utc>,
        display_end: DateTime<Utc>,
    ) -> FailureSummaryResponse {
        let actionable_failure_rate = if self.total_failures > 0 {
            self.actionable_failure_count as f64 / self.total_failures as f64
        } else {
            0.0
        };
        FailureSummaryResponse {
            range_start: format_utc_iso(start_dt),
            range_end: format_utc_iso(display_end),
            total_failures: self.total_failures,
            service_failure_count: self.service_failure_count,
            client_failure_count: self.client_failure_count,
            client_abort_count: self.client_abort_count,
            actionable_failure_count: self.actionable_failure_count,
            actionable_failure_rate,
        }
    }
}

async fn build_historical_failure_summary(
    state: &AppState,
    start_dt: DateTime<Utc>,
    display_end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<FailureSummaryResponse, ApiError> {
    let range_plan = build_hourly_rollup_exact_range_plan(
        start_dt,
        display_end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;
    let (hourly_rows, exact_records, archive_overlap_ids) =
        load_historical_error_records(state, &range_plan, source_scope).await?;
    let mut counts = FailureSummaryCounts::default();
    for row in &hourly_rows {
        counts.add_rollup(row);
    }
    for record in &exact_records {
        counts.add_classification(resolve_failure_classification(
            record.status.as_deref(),
            record.error_message.as_deref(),
            record.failure_kind.as_deref(),
            record.failure_class.as_deref(),
            record.is_actionable,
        ));
    }
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid failure summary archive start epoch"))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid failure summary archive end epoch")))?;
        for row in crate::stats::load_unmaterialized_invocation_archive_failure_rows(
            &state.pool,
            archived_start,
            archived_end,
            source_scope,
            Some(&archive_overlap_ids),
        )
        .await?
        {
            counts.add_classification(resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            ));
        }
    }
    Ok(counts.response(start_dt, display_end))
}

#[derive(sqlx::FromRow)]
struct PerfTimingRow {
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
}

async fn fetch_historical_perf_stats(
    state: &AppState,
    range_window: RangeWindow,
) -> Result<PerfStatsResponse, ApiError> {
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.display_end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;
    let mut by_stage: BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)> = BTreeMap::new();
    let (exact_records, archive_overlap_ids) =
        load_historical_perf_records(state, &range_plan).await?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        for row in query_proxy_perf_stage_hourly_rollup_range(
            &state.pool,
            range_start_epoch,
            range_end_epoch,
        )
        .await?
        {
            merge_perf_stage_rollup(
                &mut by_stage,
                &row.stage,
                row.sample_count,
                row.sum_ms,
                row.max_ms,
                decode_approx_histogram(&row.histogram),
            )?;
        }
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid perf archive start epoch")))?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid perf archive end epoch")))?;
        for (stage, delta) in
            crate::stats::query_unmaterialized_proxy_perf_stage_rollups_from_archives(
                &state.pool,
                archived_start,
                archived_end,
                Some(&archive_overlap_ids),
            )
            .await?
        {
            merge_perf_stage_rollup(
                &mut by_stage,
                &stage,
                delta.sample_count,
                delta.sum_ms,
                delta.max_ms,
                delta.histogram,
            )?;
        }
    }
    for record in exact_records {
        record_perf_stages(&mut by_stage, &record);
    }
    Ok(PerfStatsResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        source: SOURCE_PROXY.to_string(),
        stages: build_perf_stages(by_stage),
    })
}

async fn load_historical_perf_records(
    state: &AppState,
    range_plan: &HourlyRollupExactRangePlan,
) -> Result<(Vec<InvocationAggregateRecord>, HashSet<i64>), ApiError> {
    if range_plan.full_hour_range.is_some() {
        let mut tx = state.pool.begin().await?;
        let snapshot_id =
            resolve_invocation_snapshot_id_tx(tx.as_mut(), InvocationSourceScope::ProxyOnly)
                .await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
        let mut exact_records = query_invocation_exact_records_tx(
            tx.as_mut(),
            range_plan,
            InvocationSourceScope::ProxyOnly,
            snapshot_id,
        )
        .await?;
        let tail_records = query_invocation_full_hour_tail_records_tx(
            tx.as_mut(),
            range_plan,
            InvocationSourceScope::ProxyOnly,
            rollup_live_cursor,
            snapshot_id,
        )
        .await?;
        let archive_overlap_ids = tail_records.iter().map(|record| record.id).collect();
        exact_records.extend(tail_records);
        Ok((exact_records, archive_overlap_ids))
    } else {
        let snapshot_id =
            resolve_invocation_snapshot_id(&state.pool, InvocationSourceScope::ProxyOnly).await?;
        Ok((
            query_invocation_exact_records(
                &state.pool,
                range_plan,
                InvocationSourceScope::ProxyOnly,
                snapshot_id,
            )
            .await?,
            HashSet::new(),
        ))
    }
}

fn merge_perf_stage_rollup(
    by_stage: &mut BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
    stage: &str,
    sample_count: i64,
    sum_ms: f64,
    max_ms: f64,
    histogram: ApproxHistogramCounts,
) -> Result<(), ApiError> {
    let Some((sample_count, sum_ms, max_ms, histogram)) =
        validated_proxy_perf_stage_rollup(stage, sample_count, sum_ms, max_ms, histogram)
    else {
        return Ok(());
    };
    let entry = by_stage
        .entry(stage.to_string())
        .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
    entry.0 += sample_count;
    entry.1 += sum_ms;
    entry.2 = entry.2.max(max_ms);
    merge_approx_histogram_into(&mut entry.3, &histogram)?;
    Ok(())
}

fn record_perf_stages(
    by_stage: &mut BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
    record: &InvocationAggregateRecord,
) {
    for (stage, value) in [
        ("total", record.t_total_ms),
        ("requestRead", record.t_req_read_ms),
        ("requestParse", record.t_req_parse_ms),
        ("upstreamConnect", record.t_upstream_connect_ms),
        ("upstreamFirstByte", record.t_upstream_ttfb_ms),
        ("upstreamStream", record.t_upstream_stream_ms),
        ("responseParse", record.t_resp_parse_ms),
        ("persistence", record.t_persist_ms),
    ] {
        record_perf_stage_sample(by_stage, stage, value);
    }
}

fn build_perf_stages(
    by_stage: BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
) -> Vec<PerfStageStats> {
    by_stage
        .into_iter()
        .filter_map(|(stage, (count, sum_ms, max_ms, histogram))| {
            (count > 0).then(|| PerfStageStats {
                stage,
                count,
                avg_ms: sum_ms / count as f64,
                p50_ms: approx_histogram_percentile_ms(&histogram, 0.50).unwrap_or(max_ms),
                p90_ms: approx_histogram_percentile_ms(&histogram, 0.90).unwrap_or(max_ms),
                p99_ms: approx_histogram_percentile_ms(&histogram, 0.99).unwrap_or(max_ms),
                max_ms,
            })
        })
        .collect()
}

pub(crate) async fn fetch_perf_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PerfQuery>,
) -> Result<Json<PerfStatsResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    if range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        return Ok(Json(
            fetch_historical_perf_stats(&state, range_window).await?,
        ));
    }
    let mut query = QueryBuilder::new(
        "SELECT \
            t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE source = ",
    );
    query
        .push_bind(SOURCE_PROXY)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_window.start))
        .push(" AND occurred_at <= ")
        .push_bind(db_occurred_at_lower_bound(range_window.display_end));
    let rows: Vec<PerfTimingRow> = query.build_query_as().fetch_all(&state.pool).await?;
    Ok(Json(build_live_perf_stats(rows, range_window)))
}

fn build_live_perf_stats(rows: Vec<PerfTimingRow>, range_window: RangeWindow) -> PerfStatsResponse {
    let stage_series: Vec<(&str, Vec<f64>)> = vec![
        (
            "total",
            rows.iter()
                .filter_map(|row| row.t_total_ms)
                .filter(|value| is_valid_perf_stage_sample("total", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "requestRead",
            rows.iter()
                .filter_map(|row| row.t_req_read_ms)
                .filter(|value| is_valid_perf_stage_sample("requestRead", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "requestParse",
            rows.iter()
                .filter_map(|row| row.t_req_parse_ms)
                .filter(|value| is_valid_perf_stage_sample("requestParse", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamConnect",
            rows.iter()
                .filter_map(|row| row.t_upstream_connect_ms)
                .filter(|value| is_valid_perf_stage_sample("upstreamConnect", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamFirstByte",
            rows.iter()
                .filter_map(|row| row.t_upstream_ttfb_ms)
                .filter(|value| is_valid_perf_stage_sample("upstreamFirstByte", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamStream",
            rows.iter()
                .filter_map(|row| row.t_upstream_stream_ms)
                .filter(|value| is_valid_perf_stage_sample("upstreamStream", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "responseParse",
            rows.iter()
                .filter_map(|row| row.t_resp_parse_ms)
                .filter(|value| is_valid_perf_stage_sample("responseParse", *value))
                .collect::<Vec<_>>(),
        ),
        (
            "persistence",
            rows.iter()
                .filter_map(|row| row.t_persist_ms)
                .filter(|value| is_valid_perf_stage_sample("persistence", *value))
                .collect::<Vec<_>>(),
        ),
    ];

    let mut stages = Vec::new();
    for (stage, mut values) in stage_series {
        if values.is_empty() {
            continue;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = values.len() as i64;
        let sum = values.iter().copied().sum::<f64>();
        let max_ms = values.last().copied().unwrap_or(0.0);
        stages.push(PerfStageStats {
            stage: stage.to_string(),
            count,
            avg_ms: sum / count as f64,
            p50_ms: percentile_sorted_f64(&values, 0.50),
            p90_ms: percentile_sorted_f64(&values, 0.90),
            p99_ms: percentile_sorted_f64(&values, 0.99),
            max_ms,
        });
    }

    PerfStatsResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        source: SOURCE_PROXY.to_string(),
        stages,
    }
}

pub(crate) async fn latest_quota_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QuotaSnapshotResponse>, ApiError> {
    let snapshot = QuotaSnapshotResponse::fetch_latest(&state.pool)
        .await?
        .unwrap_or_else(QuotaSnapshotResponse::degraded_default);
    Ok(Json(snapshot))
}

pub(crate) async fn broadcast_quota_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    snapshot: QuotaSnapshotResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    if broadcaster.receiver_count() == 0 {
        return Ok(false);
    }

    let mut cache = cache.lock().await;
    if cache
        .quota
        .as_ref()
        .is_some_and(|current| current == &snapshot)
    {
        return Ok(false);
    }

    match broadcaster.send(BroadcastPayload::Quota {
        snapshot: Box::new(snapshot.clone()),
    }) {
        Ok(_) => {
            cache.quota = Some(snapshot);
            Ok(true)
        }
        Err(_err) if broadcaster.receiver_count() == 0 => Ok(false),
        Err(err) => Err(err),
    }
}

pub(crate) async fn sse_stream(
    state: State<Arc<AppState>>,
    query: Query<SubscriptionStreamQuery>,
) -> Result<Response, ApiError> {
    topic_sse_stream(state, query).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionResponse {
    pub(crate) backend: String,
    pub(crate) frontend: String,
}
