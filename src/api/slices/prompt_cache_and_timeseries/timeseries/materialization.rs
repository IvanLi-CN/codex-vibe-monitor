use super::aggregation::{
    add_terminal_timeseries_records, add_timeseries_terminal_delta_to_aggregate,
    build_materialized_timeseries_response, fill_timeseries_buckets,
    fold_minute_projection_aggregates, overlay_runtime_timeseries_snapshot,
};
use super::minute_projection::{
    TimeseriesMinuteProjectionV2Load, TimeseriesMinuteProjectionWarmOutcome,
    complete_minute_bounds, evaluate_timeseries_minute_projection_eligibility,
    load_timeseries_minute_projection_v2,
    store_timeseries_minute_projection_v2_warm_with_eligibility_retry,
    timeseries_minute_projection_v2_snapshot_is_current,
};
use super::queries::{
    build_timeseries_account_hourly_rollup_baseline, build_timeseries_hourly_rollup_baseline,
    timeseries_topic_uses_hourly_rollup_baseline,
};
use super::*;
#[derive(Debug)]
pub(crate) struct TimeseriesTopicMaterializedBase {
    pub(crate) range_start: DateTime<Utc>,
    pub(crate) range_end: DateTime<Utc>,
    pub(crate) range_spec: String,
    pub(crate) bucket_selection: TimeseriesBucketSelection,
    pub(crate) reporting_tz: Tz,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) snapshot_id: i64,
    pub(crate) terminal_sequence: u64,
    pub(crate) aggregates: BTreeMap<i64, BucketAggregate>,
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
        let start = range_window.start;
        let end = range_window.end;
        let (snapshot_id, mut aggregates) = if timeseries_topic_uses_hourly_rollup_baseline(
            params,
            reporting_tz,
            &range_window,
            bucket_seconds,
            state.config.invocation_max_days,
        )? {
            let baseline = match params.upstream_account_id {
                Some(upstream_account_id) => {
                    build_timeseries_account_hourly_rollup_baseline(
                        state,
                        reporting_tz,
                        source_scope,
                        &range_window,
                        &bucket_selection,
                        upstream_account_id,
                    )
                    .await?
                }
                None => {
                    build_timeseries_hourly_rollup_baseline(
                        state,
                        reporting_tz,
                        source_scope,
                        &range_window,
                        &bucket_selection,
                        false,
                    )
                    .await?
                }
            };
            (baseline.snapshot_id, baseline.aggregates)
        } else {
            let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
            let projection_eligibility = evaluate_timeseries_minute_projection_eligibility(
                &state.pool,
                state.terminal_projection_hub.as_ref(),
                bucket_seconds < 3_600 && range_window.duration <= ChronoDuration::days(1),
                source_scope,
                params.upstream_account_id,
                start,
                end,
            )
            .await?;
            let use_minute_projection = projection_eligibility.can_load();
            let aggregates = if use_minute_projection {
                if let Some(TimeseriesMinuteProjectionV2Load {
                    aggregates: minute_aggregates,
                    cursor: projection_cursor,
                    snapshot_fence,
                }) = load_timeseries_minute_projection_v2(
                    &state.pool,
                    start,
                    end,
                    source_scope,
                    params.upstream_account_id,
                )
                .await?
                {
                    let mut aggregates = fold_minute_projection_aggregates(
                        minute_aggregates,
                        bucket_seconds,
                        reporting_tz,
                    )?;
                    let (full_minute_start_epoch, full_minute_end_epoch) =
                        complete_minute_bounds(start, end);
                    let full_minute_start = Utc
                        .timestamp_opt(full_minute_start_epoch, 0)
                        .single()
                        .ok_or_else(|| {
                            anyhow!("invalid materialized timeseries full-minute start")
                        })?;
                    let full_minute_end = Utc
                        .timestamp_opt(full_minute_end_epoch, 0)
                        .single()
                        .ok_or_else(|| {
                            anyhow!("invalid materialized timeseries full-minute end")
                        })?;
                    let mut records = query_timeseries_topic_baseline_records(
                        &state.pool,
                        ExactUtcRange {
                            start: full_minute_start,
                            end: full_minute_end,
                        },
                        source_scope,
                        Some(projection_cursor),
                        snapshot_id,
                        params.upstream_account_id,
                    )
                    .await?;
                    for (boundary_start, boundary_end) in [
                        (start, end.min(full_minute_start)),
                        (start.max(full_minute_end), end),
                    ] {
                        if let Some(range) = exact_utc_range(boundary_start, boundary_end)? {
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
                    add_terminal_timeseries_records(
                        &mut aggregates,
                        records,
                        bucket_seconds,
                        reporting_tz,
                    )?;
                    if timeseries_minute_projection_v2_snapshot_is_current(
                        &state.pool,
                        state.terminal_projection_hub.as_ref(),
                        start,
                        end,
                        source_scope,
                        params.upstream_account_id,
                        &snapshot_fence,
                    )
                    .await?
                    {
                        aggregates
                    } else {
                        debug!(
                            route = "timeseries_topic",
                            builder = "minute_projection_v2",
                            projection_cursor,
                            "minute projection coverage changed while building a hot topic; falling back to exact records"
                        );
                        build_exact_timeseries_topic_baseline(
                            state,
                            start,
                            end,
                            source_scope,
                            snapshot_id,
                            params.upstream_account_id,
                            bucket_seconds,
                            reporting_tz,
                            projection_eligibility.can_warm(),
                        )
                        .await?
                    }
                } else {
                    build_exact_timeseries_topic_baseline(
                        state,
                        start,
                        end,
                        source_scope,
                        snapshot_id,
                        params.upstream_account_id,
                        bucket_seconds,
                        reporting_tz,
                        true,
                    )
                    .await?
                }
            } else {
                if projection_eligibility.has_uncovered_terminal_delta() {
                    debug!(
                        route = "timeseries_topic",
                        builder = "minute_projection_v2",
                        response_source = "exact_fallback_pending_terminal_delta",
                        "minute projection deferred until an uncovered terminal delta is warmed"
                    );
                }
                build_exact_timeseries_topic_baseline(
                    state,
                    start,
                    end,
                    source_scope,
                    snapshot_id,
                    params.upstream_account_id,
                    bucket_seconds,
                    reporting_tz,
                    projection_eligibility.can_warm(),
                )
                .await?
            };
            (snapshot_id, aggregates)
        };

        fill_timeseries_buckets(&mut aggregates, start, end, bucket_seconds, reporting_tz)?;

        Ok(Self {
            range_start: start,
            range_end: end,
            range_spec: params.range.clone(),
            bucket_selection,
            reporting_tz,
            source_scope,
            upstream_account_id: params.upstream_account_id,
            snapshot_id,
            terminal_sequence: 0,
            aggregates,
        })
    }

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

    pub(crate) fn apply_terminal_delta(
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

    pub(crate) fn serialize(&self, runtime_records: &[ApiInvocation]) -> Result<Vec<u8>, ApiError> {
        let end = Utc::now().max(self.range_end);
        let mut runtime_overlay = BTreeMap::new();
        overlay_runtime_timeseries_snapshot(
            &mut runtime_overlay,
            runtime_records,
            self.source_scope,
            self.upstream_account_id,
            self.range_start,
            end,
            self.bucket_selection.bucket_seconds,
            self.reporting_tz,
        )?;
        let Json(response) = build_materialized_timeseries_response(
            self.range_start,
            end,
            self.bucket_selection.bucket_seconds,
            self.snapshot_id,
            &self.bucket_selection,
            &self.aggregates,
            &runtime_overlay,
            self.reporting_tz,
        )?;
        serde_json::to_vec(&response).map_err(ApiError::from)
    }
}

pub(crate) async fn query_timeseries_topic_baseline_records(
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

pub(crate) async fn build_exact_timeseries_topic_baseline(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    warm_minute_projection: bool,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
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
