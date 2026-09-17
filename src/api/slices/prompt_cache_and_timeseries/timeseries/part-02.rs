pub(crate) fn add_timeseries_terminal_delta_to_aggregate(
    entry: &mut BucketAggregate,
    delta: &TimeseriesTerminalDelta,
) {
    entry.total_count += 1;
    let classification = resolve_failure_classification(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    );
    let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        delta.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    let latency_status = is_success_like
        .then_some("success")
        .or(delta.status.as_deref());
    entry.record_total_latency_sample(delta.t_total_ms);
    entry.record_exact_ttfb_sample(latency_status, delta.t_upstream_ttfb_ms);
    entry.record_exact_first_response_byte_total_sample(
        delta.t_req_read_ms,
        delta.t_req_parse_ms,
        delta.t_upstream_connect_ms,
        delta.t_upstream_ttfb_ms,
    );
    entry.record_first_token_sample(delta.first_token_ms);
    add_optional_token_components(
        entry,
        delta.total_tokens,
        delta.input_tokens,
        delta.output_tokens,
        delta.cache_input_tokens,
        delta.reasoning_tokens,
    );
    let cost = delta.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    ) {
        entry.non_success_cost += cost;
    }
}

fn add_optional_token_components(
    entry: &mut BucketAggregate,
    total_tokens: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
) {
    entry.total_tokens += total_tokens.unwrap_or_default();
    entry.input_tokens += input_tokens.unwrap_or_default();
    entry.output_tokens += output_tokens.unwrap_or_default();
    entry.cache_input_tokens += cache_input_tokens.unwrap_or_default();
    entry.reasoning_tokens += reasoning_tokens.unwrap_or_default();

    if total_tokens.unwrap_or_default() > 0 {
        entry.token_components_observed = true;
        if input_tokens.is_none()
            || output_tokens.is_none()
            || cache_input_tokens.is_none()
            || reasoning_tokens.is_none()
        {
            entry.token_component_incomplete_count += 1;
        }
    }
}

fn subtract_optional_token_components(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    entry.total_tokens = entry
        .total_tokens
        .saturating_sub(record.total_tokens.unwrap_or_default());
    entry.input_tokens = entry
        .input_tokens
        .saturating_sub(record.input_tokens.unwrap_or_default());
    entry.output_tokens = entry
        .output_tokens
        .saturating_sub(record.output_tokens.unwrap_or_default());
    entry.cache_input_tokens = entry
        .cache_input_tokens
        .saturating_sub(record.cache_input_tokens.unwrap_or_default());
    entry.reasoning_tokens = entry
        .reasoning_tokens
        .saturating_sub(record.reasoning_tokens.unwrap_or_default());
    if record.total_tokens.unwrap_or_default() > 0
        && (record.input_tokens.is_none()
            || record.output_tokens.is_none()
            || record.cache_input_tokens.is_none()
            || record.reasoning_tokens.is_none())
    {
        entry.token_component_incomplete_count =
            entry.token_component_incomplete_count.saturating_sub(1);
    }
}

struct PendingTimeseriesDeltaOverlay<'a> {
    state: &'a AppState,
    aggregates: &'a mut BTreeMap<i64, BucketAggregate>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    max_row_id: Option<i64>,
    snapshot_id: i64,
}

fn add_pending_timeseries_deltas(
    request: PendingTimeseriesDeltaOverlay<'_>,
) -> Result<usize, ApiError> {
    let PendingTimeseriesDeltaOverlay {
        state,
        aggregates,
        source_scope,
        upstream_account_id,
        start,
        end,
        bucket_seconds,
        reporting_tz,
        max_row_id,
        snapshot_id,
    } = request;
    let mut applied = 0;
    let selection = TimeseriesProjectionSelection {
        source_scope: timeseries_projection_scope(source_scope),
        upstream_account_id,
    };
    for (_, row_id, delta) in state
        .terminal_projection_hub
        .pending_timeseries_deltas_for_selection(selection, 10_000)
    {
        if max_row_id.is_some_and(|cursor| row_id > cursor) {
            // Rows above the projection cursor are already represented by the SQL tail.
            continue;
        }
        if row_id > snapshot_id {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && delta.source != SOURCE_PROXY {
            continue;
        }
        if upstream_account_id
            .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
        {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
            continue;
        };
        if occurred < start || occurred >= end {
            continue;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_timeseries_terminal_delta_to_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            &delta,
        );
        applied += 1;
    }
    Ok(applied)
}

fn timeseries_topic_uses_hourly_rollup_baseline(
    _params: &TimeseriesQuery,
    reporting_tz: Tz,
    range_window: &RangeWindow,
    bucket_seconds: i64,
    invocation_max_days: u64,
) -> Result<bool, ApiError> {
    if bucket_seconds < 3_600 {
        return Ok(false);
    }
    let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, range_window);
    let needs_historical_rollups =
        range_window.start < shanghai_retention_cutoff(invocation_max_days);
    if !tz_is_hour_aligned && needs_historical_rollups {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
        )));
    }
    Ok(tz_is_hour_aligned)
}

/// DB-backed baseline for an open dashboard timeseries topic. Live publication only mutates
/// this state with terminal deltas and overlays the in-memory runtime snapshot.
#[derive(Debug)]
pub(crate) struct TimeseriesTopicMaterializedBase {
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    range_spec: String,
    bucket_selection: TimeseriesBucketSelection,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    snapshot_id: i64,
    terminal_sequence: u64,
    aggregates: BTreeMap<i64, BucketAggregate>,
    db_runtime_records: HashMap<(String, String), InvocationAggregateRecord>,
}

impl TimeseriesTopicMaterializedBase {
    pub(crate) async fn build(
        state: &AppState,
        params: &TimeseriesQuery,
    ) -> Result<Self, ApiError> {
        // Hold the terminal writer behind a SQLite write reservation while capturing both the
        // durable baseline and the in-memory terminal watermark. A row ID is not a version: a
        // terminal write can replace an older running row without advancing the row cursor.
        let reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
        let reconcile_guard = reconcile_gate.lock().await;
        let barrier = state.pool.begin_with("BEGIN IMMEDIATE").await?;
        drop(reconcile_guard);

        let build_result = Self::build_from_stable_persistence(state, params).await;
        let (pending_terminal_deltas, terminal_sequence) = {
            let cache = state.dashboard_activity_snapshot_cache.lock().await;
            (
                cache
                    .read_model
                    .pending_terminal_deltas
                    .iter()
                    .filter(|delta| delta.persisted_row_id.is_none())
                    .cloned()
                    .collect::<Vec<_>>(),
                cache.read_model.next_terminal_sequence,
            )
        };
        if build_result.is_ok() {
            barrier.commit().await?;
        } else {
            barrier.rollback().await?;
        }

        let mut base = build_result?;
        base.apply_terminal_slice(Some(&DashboardTerminalProjectionSlice {
            revision: 0,
            deltas: pending_terminal_deltas,
        }));
        // All durable terminal records in the baseline and every pending record replayed above
        // are covered by this watermark. Future terminal slices may therefore safely reconcile
        // an older persisted row exactly once.
        base.terminal_sequence = base.terminal_sequence.max(terminal_sequence);
        Ok(base)
    }

    async fn build_from_stable_persistence(
        state: &AppState,
        params: &TimeseriesQuery,
    ) -> Result<Self, ApiError> {
        let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
        let source_scope = resolve_default_source_scope(&state.pool).await?;
        let range_window = resolve_range_window(&params.range, reporting_tz)?;
        let bucket_selection = resolve_timeseries_bucket_selection(
            params,
            &range_window,
            state.config.invocation_max_days,
        )?;
        let bucket_seconds = bucket_selection.bucket_seconds;
        let (snapshot_id, mut aggregates) = build_timeseries_stable_aggregates(
            state,
            params,
            source_scope,
            reporting_tz,
            &range_window,
            &bucket_selection,
        )
        .await?;
        let in_flight_records = query_timeseries_topic_in_flight_records(
            &state.pool,
            ExactUtcRange {
                start: range_window.start,
                end: range_window.end,
            },
            source_scope,
            snapshot_id,
            params.upstream_account_id,
        )
        .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&in_flight_records);
        add_in_flight_timeseries_records(
            &mut aggregates,
            &in_flight_records,
            bucket_selection.bucket_seconds,
            reporting_tz,
        )?;
        fill_timeseries_buckets(
            &mut aggregates,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
        )?;
        Ok(Self {
            range_start: range_window.start,
            range_end: range_window.end,
            range_spec: params.range.clone(),
            bucket_selection,
            reporting_tz,
            source_scope,
            upstream_account_id: params.upstream_account_id,
            snapshot_id,
            terminal_sequence: 0,
            aggregates,
            db_runtime_records,
        })
    }
}

async fn build_timeseries_stable_aggregates(
    state: &AppState,
    params: &TimeseriesQuery,
    source_scope: InvocationSourceScope,
    reporting_tz: Tz,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
) -> Result<(i64, BTreeMap<i64, BucketAggregate>), ApiError> {
    if timeseries_topic_uses_hourly_rollup_baseline(
        params,
        reporting_tz,
        range_window,
        bucket_selection.bucket_seconds,
        state.config.invocation_max_days,
    )? {
        return build_timeseries_rollup_aggregates(
            state,
            params,
            source_scope,
            reporting_tz,
            range_window,
            bucket_selection,
        )
        .await;
    }
    build_timeseries_exact_or_projection_aggregates(
        state,
        params,
        source_scope,
        reporting_tz,
        range_window,
        bucket_selection,
    )
    .await
}

async fn build_timeseries_rollup_aggregates(
    state: &AppState,
    params: &TimeseriesQuery,
    source_scope: InvocationSourceScope,
    reporting_tz: Tz,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
) -> Result<(i64, BTreeMap<i64, BucketAggregate>), ApiError> {
    let baseline = match params.upstream_account_id {
        Some(upstream_account_id) => {
            build_timeseries_account_hourly_rollup_baseline(
                state,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
                upstream_account_id,
            )
            .await?
        }
        None => {
            build_timeseries_hourly_rollup_baseline(
                state,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
                false,
            )
            .await?
        }
    };
    Ok((baseline.snapshot_id, baseline.aggregates))
}

struct TimeseriesProjectionPlan {
    snapshot_id: i64,
    can_warm: bool,
    use_projection: bool,
}

async fn resolve_timeseries_projection_plan(
    state: &AppState,
    params: &TimeseriesQuery,
    source_scope: InvocationSourceScope,
    range_window: &RangeWindow,
    bucket_seconds: i64,
) -> Result<TimeseriesProjectionPlan, ApiError> {
    let candidate = bucket_seconds < 3_600 && range_window.duration <= ChronoDuration::days(1);
    let fence_installed = candidate
        && timeseries_minute_projection_non_proxy_terminal_replacement_fence_is_installed(
            &state.pool,
            source_scope,
        )
        .await?;
    let recovery_pending =
        candidate && timeseries_minute_projection_recovery_pending(&state.pool).await?;
    let uncovered_delta = candidate
        && timeseries_minute_projection_has_uncovered_terminal_delta(
            state.terminal_projection_hub.as_ref(),
            source_scope,
            params.upstream_account_id,
            range_window.start,
            range_window.end,
        );
    let warming_coverage = candidate
        && timeseries_minute_projection_v2_has_warming_coverage(
            &state.pool,
            range_window.start,
            range_window.end,
            source_scope,
            params.upstream_account_id,
        )
        .await?;
    let can_warm = candidate
        && fence_installed
        && !recovery_pending
        && state
            .terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_none()
        && !uncovered_delta;
    Ok(TimeseriesProjectionPlan {
        snapshot_id: resolve_invocation_snapshot_id(&state.pool, source_scope).await?,
        can_warm,
        use_projection: can_warm && !warming_coverage,
    })
}

async fn build_timeseries_exact_or_projection_aggregates(
    state: &AppState,
    params: &TimeseriesQuery,
    source_scope: InvocationSourceScope,
    reporting_tz: Tz,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
) -> Result<(i64, BTreeMap<i64, BucketAggregate>), ApiError> {
    let plan = resolve_timeseries_projection_plan(
        state,
        params,
        source_scope,
        range_window,
        bucket_selection.bucket_seconds,
    )
    .await?;
    let projection_aggregates = if plan.use_projection {
        load_timeseries_projection_aggregates(TimeseriesProjectionAggregateRequest {
            state,
            params,
            source_scope,
            reporting_tz,
            range_window,
            bucket_selection,
            snapshot_id: plan.snapshot_id,
            can_warm: plan.can_warm,
        })
        .await?
    } else {
        None
    };
    if let Some(aggregates) = projection_aggregates {
        return Ok((plan.snapshot_id, aggregates));
    }
    let aggregates = build_exact_timeseries_topic_baseline(TimeseriesExactBaselineRequest {
        state,
        start: range_window.start,
        end: range_window.end,
        source_scope,
        snapshot_id: plan.snapshot_id,
        upstream_account_id: params.upstream_account_id,
        bucket_seconds: bucket_selection.bucket_seconds,
        reporting_tz,
        warm_minute_projection: plan.can_warm,
    })
    .await?;
    Ok((plan.snapshot_id, aggregates))
}

struct TimeseriesProjectionAggregateRequest<'a> {
    state: &'a AppState,
    params: &'a TimeseriesQuery,
    source_scope: InvocationSourceScope,
    reporting_tz: Tz,
    range_window: &'a RangeWindow,
    bucket_selection: &'a TimeseriesBucketSelection,
    snapshot_id: i64,
    can_warm: bool,
}

async fn load_timeseries_projection_aggregates(
    request: TimeseriesProjectionAggregateRequest<'_>,
) -> Result<Option<BTreeMap<i64, BucketAggregate>>, ApiError> {
    let TimeseriesProjectionAggregateRequest {
        state,
        params,
        source_scope,
        reporting_tz,
        range_window,
        bucket_selection,
        snapshot_id,
        can_warm,
    } = request;
    let Some(TimeseriesMinuteProjectionV2Load {
        aggregates: minute_aggregates,
        cursor,
        coverage_rows,
    }) = load_timeseries_minute_projection_v2(
        &state.pool,
        range_window.start,
        range_window.end,
        source_scope,
        params.upstream_account_id,
    )
    .await?
    else {
        return Ok(None);
    };
    let mut aggregates = fold_minute_projection_aggregates(
        minute_aggregates,
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    let records = load_timeseries_projection_records(
        state,
        params,
        range_window,
        source_scope,
        cursor,
        snapshot_id,
    )
    .await?;
    add_terminal_timeseries_records(
        &mut aggregates,
        records,
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    if timeseries_minute_projection_v2_snapshot_is_current(
        &state.pool,
        state.terminal_projection_hub.as_ref(),
        range_window.start,
        range_window.end,
        source_scope,
        params.upstream_account_id,
        &coverage_rows,
    )
    .await?
    {
        return Ok(Some(aggregates));
    }
    debug!(
        route = "timeseries_topic",
        builder = "minute_projection_v2",
        cursor,
        "minute projection coverage changed while building a hot topic; using exact records"
    );
    let exact = build_exact_timeseries_topic_baseline(TimeseriesExactBaselineRequest {
        state,
        start: range_window.start,
        end: range_window.end,
        source_scope,
        snapshot_id,
        upstream_account_id: params.upstream_account_id,
        bucket_seconds: bucket_selection.bucket_seconds,
        reporting_tz,
        warm_minute_projection: can_warm,
    })
    .await?;
    Ok(Some(exact))
}

async fn load_timeseries_projection_records(
    state: &AppState,
    params: &TimeseriesQuery,
    range_window: &RangeWindow,
    source_scope: InvocationSourceScope,
    cursor: i64,
    snapshot_id: i64,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let (full_start_epoch, full_end_epoch) =
        complete_minute_bounds(range_window.start, range_window.end);
    let full_start = Utc
        .timestamp_opt(full_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid materialized timeseries full-minute start"))?;
    let full_end = Utc
        .timestamp_opt(full_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid materialized timeseries full-minute end"))?;
    let mut records = query_timeseries_topic_baseline_records(
        &state.pool,
        ExactUtcRange {
            start: full_start,
            end: full_end,
        },
        source_scope,
        Some(cursor),
        snapshot_id,
        params.upstream_account_id,
    )
    .await?;
    for (start, end) in [
        (range_window.start, range_window.end.min(full_start)),
        (range_window.start.max(full_end), range_window.end),
    ] {
        if let Some(range) = exact_utc_range(start, end)? {
            records.extend(
                query_timeseries_topic_baseline_records(
                    &state.pool,
                    range,
                    source_scope,
                    None,
                    snapshot_id,
                    params.upstream_account_id,
                )
                .await?,
            );
        }
    }
    Ok(records)
}

impl TimeseriesTopicMaterializedBase {
    pub(crate) fn requires_window_rebase(&self) -> bool {
        let Ok(current_range) = resolve_range_window(&self.range_spec, self.reporting_tz) else {
            return true;
        };
        if parse_duration_spec(&self.range_spec).is_ok() {
            return current_range.start < self.range_start
                || current_range.start - self.range_start
                    >= ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64);
        }
        current_range.start != self.range_start
    }

    #[cfg(test)]
    pub(crate) fn set_range_start_for_test(&mut self, range_start: DateTime<Utc>) {
        self.range_start = range_start;
    }

    pub(crate) fn apply_terminal_slice(
        &mut self,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) {
        let Some(terminal) = terminal else {
            return;
        };
        for delta in &terminal.deltas {
            self.apply_terminal_delta(
                delta.terminal_sequence,
                delta.persisted_row_id,
                &delta.timeseries,
            );
        }
    }

    fn apply_terminal_delta(
        &mut self,
        terminal_sequence: u64,
        persisted_row_id: Option<i64>,
        delta: &TimeseriesTerminalDelta,
    ) {
        if terminal_sequence <= self.terminal_sequence {
            return;
        }
        self.terminal_sequence = terminal_sequence;
        if self.source_scope == InvocationSourceScope::ProxyOnly && delta.source != SOURCE_PROXY {
            return;
        }
        if let Some(row_id) = persisted_row_id {
            self.snapshot_id = self.snapshot_id.max(row_id);
        }
        if self
            .upstream_account_id
            .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
        {
            return;
        }
        let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
            return;
        };
        if occurred < self.range_start || occurred >= Utc::now().max(self.range_end) {
            return;
        }
        let Ok(bucket_epoch) = align_reporting_bucket_epoch(
            occurred.timestamp(),
            self.bucket_selection.bucket_seconds,
            self.reporting_tz,
        ) else {
            return;
        };
        add_timeseries_terminal_delta_to_aggregate(
            self.aggregates.entry(bucket_epoch).or_default(),
            delta,
        );
    }

    pub(crate) fn response(
        &self,
        runtime_records: &[ApiInvocation],
    ) -> Result<Json<TimeseriesResponse>, ApiError> {
        let end = if parse_duration_spec(&self.range_spec).is_ok() {
            Utc::now().max(self.range_end)
        } else {
            self.range_end
        };
        let mut runtime_overlay = BTreeMap::new();
        overlay_runtime_timeseries_snapshot(RuntimeTimeseriesSnapshotOverlay {
            aggregates: &mut runtime_overlay,
            runtime_records,
            source_scope: self.source_scope,
            upstream_account_id: self.upstream_account_id,
            start: self.range_start,
            end,
            bucket_seconds: self.bucket_selection.bucket_seconds,
            reporting_tz: self.reporting_tz,
            db_runtime_records: &self.db_runtime_records,
        })?;
        let Json(response) =
            build_materialized_timeseries_response(MaterializedTimeseriesResponseInput {
                start_dt: self.range_start,
                end_dt: end,
                bucket_seconds: self.bucket_selection.bucket_seconds,
                snapshot_id: self.snapshot_id,
                bucket_selection: &self.bucket_selection,
                aggregates: &self.aggregates,
                runtime_overlay: &runtime_overlay,
                reporting_tz: self.reporting_tz,
            })?;
        Ok(Json(response))
    }

    pub(crate) fn serialize(&self, runtime_records: &[ApiInvocation]) -> Result<Vec<u8>, ApiError> {
        let Json(response) = self.response(runtime_records)?;
        serde_json::to_vec(&response).map_err(ApiError::from)
    }
}

async fn query_timeseries_topic_baseline_records(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    start_after_id: Option<i64>,
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    match upstream_account_id {
        Some(upstream_account_id) => {
            query_invocation_aggregate_records_from_live_range_for_account(
                pool,
                range,
                source_scope,
                start_after_id,
                Some(snapshot_id),
                upstream_account_id,
            )
            .await
        }
        None => {
            query_invocation_aggregate_records_from_live_range(
                pool,
                range,
                source_scope,
                start_after_id,
                Some(snapshot_id),
            )
            .await
        }
    }
}

struct TimeseriesExactBaselineRequest<'a> {
    state: &'a AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    warm_minute_projection: bool,
}

async fn build_exact_timeseries_topic_baseline(
    request: TimeseriesExactBaselineRequest<'_>,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
    let TimeseriesExactBaselineRequest {
        state,
        start,
        end,
        source_scope,
        snapshot_id,
        upstream_account_id,
        bucket_seconds,
        reporting_tz,
        warm_minute_projection,
    } = request;
    let should_warm_minute_projection = warm_minute_projection
        && state
            .terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_none();
    let records = query_timeseries_topic_baseline_records(
        &state.pool,
        ExactUtcRange { start, end },
        source_scope,
        None,
        snapshot_id,
        upstream_account_id,
    )
    .await?;
    if should_warm_minute_projection {
        let pool = state.pool.clone();
        let terminal_projection_hub = state.terminal_projection_hub.clone();
        tokio::spawn(async move {
            match store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
                &pool,
                start,
                end,
                source_scope,
                upstream_account_id,
                terminal_projection_hub.as_ref(),
                "topic_exact_fallback",
            )
            .await
            {
                Ok(TimeseriesMinuteProjectionWarmOutcome::Stored) => {}
                Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(_)) => {
                    debug!(
                        route = "timeseries_projection",
                        projection_store_outcome = "deferred",
                        "materialized timeseries minute projection warm write yielded to higher-priority work"
                    );
                }
                Err(error) => {
                    debug!(
                        ?error,
                        "materialized timeseries minute projection warm write failed"
                    );
                }
            }
        });
    }
    let mut aggregates = BTreeMap::new();
    add_terminal_timeseries_records(&mut aggregates, records, bucket_seconds, reporting_tz)?;
    Ok(aggregates)
}

fn add_terminal_timeseries_records(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    records: Vec<InvocationAggregateRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_exact_record_to_timeseries_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            &record,
        );
    }
    Ok(())
}

fn fill_timeseries_buckets(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(i64, i64), ApiError> {
    let fill_start_epoch =
        align_reporting_bucket_epoch(start.timestamp(), bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }
    Ok((fill_start_epoch, fill_end_epoch))
}

fn timeseries_projection_snapshot_records(
    source_records: &[InvocationAggregateRecord],
    pending_terminal_deltas: &[(u64, i64, TimeseriesTerminalDelta)],
) -> Vec<TimeseriesProjectionSnapshotRecord> {
    let terminal_source_rows = source_records
        .iter()
        .filter(|record| !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()))
        .map(|record| (record.id, (&record.invoke_id, &record.occurred_at)))
        .collect::<HashMap<_, _>>();

    pending_terminal_deltas
        .iter()
        .filter_map(|(_, row_id, delta)| {
            terminal_source_rows
                .get(row_id)
                .map(
                    |(invoke_id, occurred_at)| TimeseriesProjectionSnapshotRecord {
                        row_id: *row_id,
                        invoke_id: (*invoke_id).clone(),
                        occurred_at: (*occurred_at).clone(),
                        delta: delta.clone(),
                    },
                )
        })
        .collect()
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TimeseriesMinuteProjectionKey {
    minute_start_epoch: i64,
    source_scope: &'static str,
    upstream_account_key: i64,
}

struct TimeseriesMinuteProjectionPreparedWrite {
    key: TimeseriesMinuteProjectionKey,
    aggregate_json: String,
    total_latency_samples_json: String,
    first_byte_samples_json: String,
    first_response_byte_total_samples_json: String,
    first_token_samples_json: String,
    max_row_id: i64,
}
