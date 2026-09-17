#[derive(Debug)]
pub(crate) struct SummaryTopicTerminalConsistentBase {
    pub(crate) response: StatsResponse,
    pub(crate) pending_terminal_deltas: Vec<DashboardActivityTerminalDelta>,
    pub(crate) terminal_sequence: u64,
}

/// Builds an open summary topic base behind the terminal-writer barrier. The response and
/// terminal watermark must observe the same durable SQLite boundary, otherwise an ACK between
/// those reads can make a cached terminal slice appear already applied when it is absent.
pub(crate) async fn build_summary_topic_terminal_consistent_base(
    state: &AppState,
    params: &SummaryQuery,
) -> Result<SummaryTopicTerminalConsistentBase, ApiError> {
    let reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
    let reconcile_guard = reconcile_gate.lock().await;
    let barrier = begin_dashboard_activity_consistency_barrier(&state.pool).await?;
    // Once the SQLite write barrier is owned, the writer can retain new terminal work while the
    // base captures both the durable response and its pending in-memory overlay.
    drop(reconcile_guard);

    let response = load_summary_response_from_query(state, params, SummaryBuildRoute::Topic).await;
    let (pending_terminal_deltas, terminal_sequence) = {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        (
            cache
                .read_model
                .pending_terminal_deltas
                .iter()
                .filter(|delta| delta.persisted_row_id.is_none())
                .cloned()
                .collect(),
            cache.read_model.next_terminal_sequence,
        )
    };
    finish_dashboard_activity_consistency_barrier(barrier, response.is_ok()).await?;

    Ok(SummaryTopicTerminalConsistentBase {
        response: response?,
        pending_terminal_deltas,
        terminal_sequence,
    })
}

pub(crate) async fn load_summary_response_from_query(
    state: &AppState,
    params: &SummaryQuery,
    route: SummaryBuildRoute,
) -> Result<StatsResponse, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(params, default_limit)?;
    let summary_range_telemetry =
        SummaryRangeBuildTelemetry::new(route, &window, params.window.as_deref());
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let upstream_account_id = params.upstream_account_id;
    let now = Utc::now();

    if summary_range_telemetry.closed_window {
        let summary_delivery_mode = route.closed_window_delivery_mode();
        if matches!(route, SummaryBuildRoute::Topic) {
            tracing::warn!(
                route = route.telemetry_route(),
                summary_delivery_mode,
                closed_window = true,
                window_kind = summary_range_telemetry.window_kind,
                window_name = summary_range_telemetry.window_name,
                upstream_account_id,
                "closed-window summary topic request received; prefer exact http path"
            );
        } else {
            tracing::debug!(
                route = route.telemetry_route(),
                summary_delivery_mode,
                closed_window = true,
                window_kind = summary_range_telemetry.window_kind,
                window_name = summary_range_telemetry.window_name,
                upstream_account_id,
                "closed-window summary request using exact http path"
            );
        }
    }

    let Some(totals) = load_summary_window_totals(
        state,
        &window,
        reporting_tz,
        source_scope,
        upstream_account_id,
        now,
    )
    .await?
    else {
        return build_empty_summary_response(state, source_scope, upstream_account_id).await;
    };

    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    let range = summary_window_range(&window, reporting_tz, now)?;
    let range_telemetry = range.map(|_| summary_range_telemetry);
    if let Some((start, end)) = range {
        let telemetry = range_telemetry.expect("summary range telemetry should exist");
        response.usage_breakdown = load_usage_breakdown_for_range(
            state,
            source_scope,
            upstream_account_id,
            ExactUtcRange { start, end },
            telemetry,
        )
        .await?;
    }
    let policy = summary_live_augmentation_policy(&window, range, now);
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        upstream_account_id,
        range,
        policy,
        range_telemetry,
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    response.maintenance = Some(load_stats_maintenance_response(state).await?);
    Ok(response)
}

async fn load_summary_window_totals(
    state: &AppState,
    window: &SummaryWindow,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Option<StatsTotals>, ApiError> {
    match window {
        SummaryWindow::All => {
            if let Some(account_id) = upstream_account_id {
                let start = Utc.timestamp_opt(0, 0).single().ok_or_else(|| {
                    ApiError::from(anyhow!("invalid account all-time summary start"))
                })?;
                query_hourly_backed_summary_range_for_account(
                    state,
                    start,
                    now,
                    source_scope,
                    account_id,
                )
                .await
                .map(Some)
            } else {
                Ok(
                    query_combined_totals(&state.pool, StatsFilter::All, source_scope)
                        .await
                        .map(Some)?,
                )
            }
        }
        SummaryWindow::Current(limit) => {
            if let Some(account_id) = upstream_account_id {
                Ok(crate::stats::query_upstream_account_stats_row(
                    &state.pool,
                    StatsFilter::RecentLimit(*limit),
                    source_scope,
                    account_id,
                )
                .await
                .map(StatsTotals::from)
                .map(Some)?)
            } else {
                Ok(query_combined_totals(
                    &state.pool,
                    StatsFilter::RecentLimit(*limit),
                    source_scope,
                )
                .await
                .map(Some)?)
            }
        }
        SummaryWindow::Duration(duration) => load_summary_range_totals(
            state,
            now - *duration,
            now,
            source_scope,
            upstream_account_id,
        )
        .await
        .map(Some),
        SummaryWindow::Calendar(spec) => {
            let range =
                resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            if range.start >= range.end {
                Ok(None)
            } else {
                load_summary_range_totals(
                    state,
                    range.start,
                    range.end,
                    source_scope,
                    upstream_account_id,
                )
                .await
                .map(Some)
            }
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(*day_count, now, reporting_tz)
                .ok_or_else(|| {
                    ApiError::bad_request(anyhow!("invalid previous full days window"))
                })?;
            load_summary_range_totals(state, start, end, source_scope, upstream_account_id)
                .await
                .map(Some)
        }
    }
}

async fn load_summary_range_totals(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<StatsTotals, ApiError> {
    if let Some(account_id) = upstream_account_id {
        query_hourly_backed_summary_range_for_account(state, start, end, source_scope, account_id)
            .await
    } else {
        query_hourly_backed_summary_range(state, start, end, source_scope).await
    }
}

pub(crate) fn summary_delta_gap_affects_selection(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> bool {
    if let SummaryWindow::Current(limit) = window {
        return gaps.iter().any(|gap| {
            (upstream_account_id.is_none()
                || gap.upstream_account_id.is_none()
                || gap.upstream_account_id == upstream_account_id)
                && projection.delta_gap_affects_current_selection(
                    gap,
                    (*limit).max(0) as usize,
                    upstream_account_id,
                    deltas,
                )
        });
    }
    let range = summary_window_range(window, reporting_tz, Utc::now())
        .ok()
        .flatten();
    gaps.iter().any(|gap| {
        if upstream_account_id.is_some()
            && gap.upstream_account_id.is_some()
            && gap.upstream_account_id != upstream_account_id
        {
            return false;
        }
        let Some((start, end)) = range else {
            return true;
        };
        parse_to_utc_datetime(&gap.occurred_at)
            .is_none_or(|occurred_at| occurred_at >= start && occurred_at < end)
    })
}

pub(crate) fn summary_delta_affects_selection(
    projection: &SummaryProjection,
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> bool {
    let account_matches = |delta: &DashboardActivityTerminalDelta| {
        upstream_account_id.is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
    };
    match window {
        SummaryWindow::All => deltas.iter().any(account_matches),
        SummaryWindow::Current(limit) => {
            let limit = (*limit).max(0) as usize;
            if limit == 0 {
                return false;
            }
            let cutoff = projection.current_selection_cutoff_for_scope(upstream_account_id, limit);
            deltas
                .iter()
                .filter(|delta| account_matches(delta))
                .any(|delta| {
                    let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
                        return true;
                    };
                    cutoff.is_none_or(|cutoff| occurred_at >= cutoff)
                })
        }
        _ => {
            let range = summary_window_range(window, reporting_tz, Utc::now())
                .ok()
                .flatten();
            deltas
                .iter()
                .filter(|delta| account_matches(delta))
                .any(|delta| {
                    let Some((start, end)) = range else {
                        return true;
                    };
                    parse_to_utc_datetime(&delta.occurred_at)
                        .is_none_or(|occurred_at| occurred_at >= start && occurred_at < end)
                })
        }
    }
}

pub(crate) async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    // Parse before touching the hub so invalid requests retain their former 400 contract without
    // creating cache state or touching SQLite/filesystem state.
    validate_summary_projection_window(&params, state.config.list_limit_max as i64)?;
    let all_time = matches!(
        parse_summary_window(&params, state.config.list_limit_max as i64)
            .map_err(ApiError::bad_request)?,
        SummaryWindow::All
    );
    let _key = SummarySnapshotKey::try_from_query(&params, state.config.list_limit_max as i64)
        .map_err(ApiError::bad_request)?;
    state
        .subscription_hub
        .note_summary_http_interest(all_time)
        .await;
    if all_time {
        return fetch_all_time_summary(&state, &params).await;
    }

    fetch_rolling_summary(&state, &params).await
}

async fn fetch_all_time_summary(
    state: &AppState,
    params: &SummaryQuery,
) -> Result<Json<StatsResponse>, ApiError> {
    let Some((projection, deltas, _, gaps)) = state
        .subscription_hub
        .summary_projection_with_terminal_overlay(true, params.upstream_account_id)
        .await?
    else {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection has not completed hydration"
        )));
    };
    let window = parse_summary_window(params, state.config.list_limit_max as i64)
        .map_err(ApiError::bad_request)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    if summary_delta_gap_affects_selection(
        projection.as_ref(),
        &gaps,
        &deltas,
        &window,
        reporting_tz,
        params.upstream_account_id,
    ) {
        return Err(ApiError::unavailable(anyhow!(
            "summary delta journal has an unproven change for the requested all-time selection"
        )));
    }
    let mut response = projection.response_for_query_with_rolling_delta(
        params,
        state.config.list_limit_max as i64,
        true,
    )?;
    let mut terminal_sequence = 0;
    apply_dashboard_terminal_slice_to_summary_response(
        &mut response,
        &mut terminal_sequence,
        &window,
        reporting_tz,
        InvocationSourceScope::All,
        params.upstream_account_id,
        &DashboardTerminalProjectionSlice {
            revision: 0,
            deltas,
        },
    );
    Ok(Json(response))
}

async fn fetch_rolling_summary(
    state: &AppState,
    params: &SummaryQuery,
) -> Result<Json<StatsResponse>, ApiError> {
    let SummaryRollingDeltaSnapshot {
        projection,
        entries: deltas,
        gaps,
    } = state
        .subscription_hub
        .summary_projection_with_rolling_delta()
        .await
        .ok_or_else(|| {
            ApiError::unavailable(anyhow!("summary projection has not completed hydration"))
        })?;
    let window = parse_summary_window(params, state.config.list_limit_max as i64)
        .map_err(ApiError::bad_request)?;
    let account_id = params.upstream_account_id;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    if summary_delta_gap_affects_selection(
        projection.as_ref(),
        &gaps,
        &deltas,
        &window,
        reporting_tz,
        account_id,
    ) {
        return Err(ApiError::unavailable(anyhow!(
            "summary delta journal has an unproven change for the requested selection"
        )));
    }
    let mut response = projection.response_for_query_with_rolling_delta(
        params,
        state.config.list_limit_max as i64,
        summary_delta_affects_selection(
            projection.as_ref(),
            &deltas,
            &window,
            reporting_tz,
            account_id,
        ),
    )?;
    if let SummaryWindow::Current(limit) = window {
        projection.apply_rolling_delta_to_current_response(
            &mut response,
            limit,
            account_id,
            &deltas,
        )?;
        return Ok(Json(response));
    }
    let mut terminal_sequence = 0;
    apply_dashboard_terminal_slice_to_summary_response(
        &mut response,
        &mut terminal_sequence,
        &window,
        reporting_tz,
        InvocationSourceScope::All,
        account_id,
        &DashboardTerminalProjectionSlice {
            revision: 0,
            deltas,
        },
    );
    Ok(Json(response))
}

pub(crate) async fn load_stats_maintenance_response(
    state: &AppState,
) -> Result<StatsMaintenanceResponse, ApiError> {
    {
        let cache = state.maintenance_stats_cache.lock().await;
        if let Some(response) = cache.fresh_response() {
            return Ok(response);
        }
    }

    let raw_backlog = load_raw_compression_backlog_snapshot(&state.pool, &state.config).await?;
    let startup_progress = load_startup_backfill_progress(
        &state.pool,
        StartupBackfillTask::UpstreamActivityArchives.name(),
    )
    .await?;
    let pending_accounts = count_upstream_accounts_missing_last_activity(&state.pool).await?;
    let historical_rollup_backfill =
        load_historical_rollup_backfill_snapshot(&state.pool, &state.config).await?;
    let response = StatsMaintenanceResponse {
        raw_compression_backlog: Some(RawCompressionBacklogResponse {
            oldest_uncompressed_age_secs: raw_backlog.oldest_uncompressed_age_secs,
            uncompressed_count: raw_backlog.uncompressed_count,
            uncompressed_bytes: raw_backlog.uncompressed_bytes,
            alert_level: raw_backlog.alert_level,
        }),
        startup_backfill: Some(StartupBackfillMaintenanceResponse {
            upstream_activity_archive_pending_accounts: pending_accounts,
            zero_update_streak: startup_progress.zero_update_streak,
            next_run_after: startup_progress.next_run_after,
        }),
        historical_rollup_backfill: Some(HistoricalRollupBackfillMaintenanceResponse {
            pending_buckets: historical_rollup_backfill.pending_buckets,
            legacy_archive_pending: historical_rollup_backfill.legacy_archive_pending,
            last_materialized_hour: historical_rollup_backfill.last_materialized_hour,
            alert_level: historical_rollup_backfill.alert_level,
        }),
    };
    let mut cache = state.maintenance_stats_cache.lock().await;
    if let Some(cached) = cache.fresh_response() {
        return Ok(cached);
    }
    cache.store(response.clone());
    Ok(response)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactUtcRange {
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub(crate) struct HourlyRollupExactRangePlan {
    pub(crate) full_hour_range: Option<(i64, i64)>,
    pub(crate) live_exact_ranges: Vec<ExactUtcRange>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub(crate) struct InvocationAggregateRecord {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    #[sqlx(default)]
    pub(crate) live_phase: Option<String>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
}

fn ceil_hour_epoch(epoch: i64) -> i64 {
    let floor = align_bucket_epoch(epoch, 3_600, 0);
    if floor < epoch { floor + 3_600 } else { floor }
}

pub(crate) fn exact_utc_range(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Option<ExactUtcRange>, ApiError> {
    if start >= end {
        return Ok(None);
    }
    Ok(Some(ExactUtcRange { start, end }))
}

pub(crate) fn push_exact_range(
    ranges: &mut Vec<ExactUtcRange>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(), ApiError> {
    let Some(range) = exact_utc_range(start, end)? else {
        return Ok(());
    };
    if ranges
        .iter()
        .any(|existing| existing.start == range.start && existing.end == range.end)
    {
        return Ok(());
    }
    ranges.push(range);
    Ok(())
}
