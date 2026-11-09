use super::*;
pub(crate) fn dashboard_account_to_upstream_account(
    account: DashboardActivityAccountResponse,
) -> Option<UpstreamAccountActivityAccountResponse> {
    let upstream_account_id = account.upstream_account_id?;
    Some(UpstreamAccountActivityAccountResponse {
        upstream_account_id,
        display_name: account.display_name,
        latest_conversation_created_at: account.latest_conversation_created_at,
        last_invocation_at: account.last_invocation_at,
        group_name: account.group_name,
        plan_type: account.plan_type,
        enabled: account.enabled.unwrap_or(true),
        display_status: account
            .display_status
            .unwrap_or_else(|| "active".to_string()),
        enable_status: account
            .enable_status
            .unwrap_or_else(|| "enabled".to_string()),
        work_status: account.work_status.unwrap_or_else(|| "idle".to_string()),
        health_status: account
            .health_status
            .unwrap_or_else(|| "normal".to_string()),
        sync_state: account.sync_state.unwrap_or_else(|| "idle".to_string()),
        last_error: account.last_error,
        last_action_reason_message: account.last_action_reason_message,
        request_count: account.request_count,
        success_count: account.success_count,
        failure_count: account.failure_count,
        non_success_count: account.non_success_count,
        total_tokens: account.total_tokens,
        success_tokens: account.success_tokens,
        non_success_tokens: account.non_success_tokens,
        failure_tokens: account.failure_tokens,
        failure_cost: account.failure_cost,
        total_cost: account.total_cost,
        usage_breakdown: account.usage_breakdown,
        cache_hit_rate: account.cache_hit_rate,
        tokens_per_minute: account.tokens_per_minute,
        spend_rate: account.spend_rate,
        first_byte_avg_ms: account.first_byte_avg_ms,
        first_response_byte_total_avg_ms: account.first_response_byte_total_avg_ms,
        first_token_avg_ms: account.first_token_avg_ms,
        avg_total_ms: account.avg_total_ms,
        current_first_token_avg_ms: account.current_first_token_avg_ms,
        current_first_response_byte_total_avg_ms: account.current_first_response_byte_total_avg_ms,
        current_avg_total_ms: account.current_avg_total_ms,
        current_avg_response_ms: account.current_avg_response_ms,
        in_progress_invocation_count: account.in_progress_invocation_count,
        in_progress_phase_counts: account.in_progress_phase_counts,
        retry_invocation_count: account.retry_invocation_count,
        upload_bytes_per_second: account.upload_bytes_per_second,
        download_bytes_per_second: account.download_bytes_per_second,
        effective_routing_rule: account
            .effective_routing_rule
            .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule),
        recent_invocations: account.recent_invocations,
    })
}

pub(crate) async fn build_dashboard_activity_topic_materialized_base(
    state: &AppState,
    params: &DashboardActivityQuery,
) -> Result<DashboardActivityTopicMaterializedBase, ApiError> {
    let recent_limit = validate_dashboard_activity_params(
        "dashboard-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let live = if params.range != "yesterday" {
        Some(capture_dashboard_activity_live_snapshot(state).await?)
    } else {
        None
    };
    let network_live_bucket = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_live_bucket.clone());
    let network_realtime_rate = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_realtime_rate.clone());
    let include_recent = params.include_recent.unwrap_or(true);
    let request_range =
        resolve_dashboard_activity_exact_range(params.range.as_str(), reporting_tz)?;
    let (mut snapshot, _) = load_dashboard_activity_snapshot_cached(
        state,
        params.range.as_str(),
        reporting_tz,
        recent_limit,
        params.include_accounts,
        include_recent,
        live.as_ref()
            .map(dashboard_live_snapshot_in_progress_counts),
    )
    .await?;
    snapshot.range_start = request_range.start;
    snapshot.range_end = request_range.end;
    let live_revision = live.as_ref().map_or(0, |snapshot| snapshot.revision);
    if let Some(live) = live {
        overlay_dashboard_activity_live_accounts(
            state,
            &mut snapshot,
            live,
            request_range,
            params.include_accounts,
            include_recent,
            recent_limit,
        )
        .await?;
    }
    let current_rate_window = if params.range == "yesterday" {
        dashboard_activity_last_complete_minute_window(ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        })
    } else {
        ExactUtcRange {
            start: snapshot.range_end
                - ChronoDuration::seconds(DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS),
            end: snapshot.range_end,
        }
    };
    let DashboardActivitySnapshot {
        range,
        range_start,
        range_end,
        terminal_sequence,
        accounts,
        summary,
        summary_model_performance_accumulator,
        account_model_performance_accumulators,
        account_latency_accumulators,
        model_performance_accumulator_ready,
        ..
    } = snapshot;
    Ok(DashboardActivityTopicMaterializedBase {
        response: DashboardActivityResponse {
            range,
            range_start: format_utc_iso_precise(range_start),
            range_end: format_utc_iso_precise(range_end),
            snapshot_id: range_end.timestamp_millis(),
            routing_state_version: current_routing_state_version(state),
            terminal_sequence,
            live_revision,
            rate_window: DashboardActivityRateWindowResponse {
                start: format_utc_iso_precise(current_rate_window.start),
                end: format_utc_iso_precise(current_rate_window.end),
                window_minutes: 1,
                mode: if params.range == "yesterday" {
                    "last_complete_1m_sma".to_string()
                } else {
                    "rolling_60s_live_mean".to_string()
                },
            },
            summary,
            network_live_bucket,
            network_realtime_rate,
            accounts: params.include_accounts.then_some(accounts),
        },
        summary_model_performance_accumulator,
        account_model_performance_accumulators,
        account_latency_accumulators,
        model_performance_accumulator_ready,
        recent_limit,
        include_recent,
    })
}

pub(crate) async fn fetch_dashboard_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardActivityQuery>,
) -> Result<Json<DashboardActivityResponse>, ApiError> {
    let started_at = Instant::now();
    let recent_limit = validate_dashboard_activity_params(
        "dashboard-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let live = if params.range != "yesterday" {
        Some(capture_dashboard_activity_live_snapshot(state.as_ref()).await?)
    } else {
        None
    };
    let network_live_bucket = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_live_bucket.clone());
    let network_realtime_rate = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_realtime_rate.clone());
    let include_recent = params.include_recent.unwrap_or(true);
    let request_range =
        resolve_dashboard_activity_exact_range(params.range.as_str(), reporting_tz)?;
    let (mut snapshot, cache_outcome) = load_dashboard_activity_snapshot_cached(
        state.as_ref(),
        params.range.as_str(),
        reporting_tz,
        recent_limit,
        params.include_accounts,
        include_recent,
        live.as_ref()
            .map(dashboard_live_snapshot_in_progress_counts),
    )
    .await?;
    snapshot.range_start = request_range.start;
    snapshot.range_end = request_range.end;
    let live_revision = live.as_ref().map_or(0, |snapshot| snapshot.revision);
    let live_overlay_started_at = Instant::now();
    if let Some(live) = live {
        overlay_dashboard_activity_live_accounts(
            state.as_ref(),
            &mut snapshot,
            live,
            request_range,
            params.include_accounts,
            include_recent,
            recent_limit,
        )
        .await?;
    }
    let build_telemetry = snapshot.build_telemetry;
    let live_overlay_elapsed_ms = live_overlay_started_at.elapsed().as_millis() as u64;
    let range_start = format_utc_iso_precise(snapshot.range_start);
    let range_end = format_utc_iso_precise(snapshot.range_end);
    let current_rate_window = if params.range == "yesterday" {
        dashboard_activity_last_complete_minute_window(ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        })
    } else {
        ExactUtcRange {
            start: snapshot.range_end
                - ChronoDuration::seconds(DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS),
            end: snapshot.range_end,
        }
    };
    let account_count = snapshot.accounts.len();
    let accounts = params.include_accounts.then_some(snapshot.accounts);
    let response = DashboardActivityResponse {
        range: snapshot.range,
        range_start: range_start.clone(),
        range_end: range_end.clone(),
        snapshot_id: snapshot.range_end.timestamp_millis(),
        routing_state_version: current_routing_state_version(&state),
        terminal_sequence: snapshot.terminal_sequence,
        live_revision,
        rate_window: DashboardActivityRateWindowResponse {
            start: format_utc_iso_precise(current_rate_window.start),
            end: format_utc_iso_precise(current_rate_window.end),
            window_minutes: 1,
            mode: if params.range == "yesterday" {
                "last_complete_1m_sma".to_string()
            } else {
                "rolling_60s_live_mean".to_string()
            },
        },
        summary: snapshot.summary,
        network_live_bucket,
        network_realtime_rate,
        accounts,
    };
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let active_subscriber_count = state
        .subscription_hub
        .active_topic_subscriber_count("dashboard.activity.current")
        .await;
    let builder = if params.include_accounts {
        "dashboard_full"
    } else {
        "summary_only"
    };
    if elapsed_ms >= 250 {
        tracing::warn!(
            route = "dashboard",
            builder,
            endpoint = "/api/stats/dashboard-activity",
            range = %params.range,
            include_accounts = params.include_accounts,
            include_recent,
            recent_limit,
            live_revision,
            active_subscriber_count,
            account_count,
            build_scope = dashboard_activity_build_scope(params.include_accounts, include_recent),
            cache_hit_or_miss = cache_outcome.cache_hit_or_miss,
            cache_bypass_reason = cache_outcome.cache_bypass_reason,
            coalesced_waiter_count = cache_outcome.coalesced_waiter_count,
            db_build_elapsed_ms = cache_outcome.db_build_elapsed_ms,
            cache_ttl_ms = cache_outcome.cache_ttl_ms,
            cache_entry_age_ms = cache_outcome.cache_entry_age_ms,
            cache_entry_count = cache_outcome.cache_entry_count,
            in_flight_count = cache_outcome.in_flight_count,
            refresh_reason = cache_outcome.refresh_reason,
            invalidation_reason = "none",
            selection_fingerprint = cache_outcome.selection_fingerprint,
            base_snapshot_age_ms = cache_outcome.cache_entry_age_ms,
            response_source = if cache_outcome.build_attempted { "fallback_db" } else { "memory" },
            read_model_state = dashboard_activity_read_model_state(cache_outcome),
            build_attempted = cache_outcome.build_attempted,
            build_source = if cache_outcome.build_attempted { "sqlite_baseline" } else { "none" },
            snapshot_origin = cache_outcome.snapshot_origin,
            baseline_cursor = cache_outcome.baseline_cursor,
            terminal_delta_count = cache_outcome.terminal_delta_count,
            duplicate_delta_count = cache_outcome.duplicate_delta_count,
            pending_delta_count = cache_outcome.pending_delta_count,
            pending_delta_estimated_bytes = cache_outcome.pending_delta_estimated_bytes,
            persisted_ack_pending_count = cache_outcome.persisted_ack_pending_count,
            delta_pruned_count = cache_outcome.delta_pruned_count,
            expiry_delta_count = cache_outcome.expiry_delta_count,
            hard_limit_reason = cache_outcome.hard_limit_reason,
            sequence_gap_count = cache_outcome.sequence_gap_count,
            reconcile_outcome = cache_outcome.cache_bypass_reason,
            live_overlay_elapsed_ms,
            preview_read_mode = build_telemetry.preview_read_mode,
            candidate_preview_id_count = build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build_telemetry.hydrated_preview_row_count,
            aggregation_mode = if build_telemetry.account_aggregation.fallback_hour_count == 0 { "rollup_plus_boundary" } else { "partial_rollup_with_exact_fallback" },
            covered_hour_count = build_telemetry.account_aggregation.covered_hour_count,
            fallback_hour_count = build_telemetry.account_aggregation.fallback_hour_count,
            boundary_tail_count = build_telemetry.account_aggregation.boundary_tail_count,
            rollup_row_count = build_telemetry.account_aggregation.rollup_row_count,
            raw_fallback_range_count = build_telemetry.account_aggregation.raw_fallback_range_count,
            model_performance_available = response.summary.model_performance.available,
            elapsed_ms,
            "dashboard activity snapshot exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route = "dashboard",
            builder,
            endpoint = "/api/stats/dashboard-activity",
            range = %params.range,
            include_accounts = params.include_accounts,
            include_recent,
            recent_limit,
            live_revision,
            active_subscriber_count,
            account_count,
            build_scope = dashboard_activity_build_scope(params.include_accounts, include_recent),
            cache_hit_or_miss = cache_outcome.cache_hit_or_miss,
            cache_bypass_reason = cache_outcome.cache_bypass_reason,
            coalesced_waiter_count = cache_outcome.coalesced_waiter_count,
            db_build_elapsed_ms = cache_outcome.db_build_elapsed_ms,
            cache_ttl_ms = cache_outcome.cache_ttl_ms,
            cache_entry_age_ms = cache_outcome.cache_entry_age_ms,
            cache_entry_count = cache_outcome.cache_entry_count,
            in_flight_count = cache_outcome.in_flight_count,
            refresh_reason = cache_outcome.refresh_reason,
            invalidation_reason = "none",
            selection_fingerprint = cache_outcome.selection_fingerprint,
            base_snapshot_age_ms = cache_outcome.cache_entry_age_ms,
            response_source = if cache_outcome.build_attempted { "fallback_db" } else { "memory" },
            read_model_state = dashboard_activity_read_model_state(cache_outcome),
            build_attempted = cache_outcome.build_attempted,
            build_source = if cache_outcome.build_attempted { "sqlite_baseline" } else { "none" },
            snapshot_origin = cache_outcome.snapshot_origin,
            baseline_cursor = cache_outcome.baseline_cursor,
            terminal_delta_count = cache_outcome.terminal_delta_count,
            duplicate_delta_count = cache_outcome.duplicate_delta_count,
            pending_delta_count = cache_outcome.pending_delta_count,
            pending_delta_estimated_bytes = cache_outcome.pending_delta_estimated_bytes,
            persisted_ack_pending_count = cache_outcome.persisted_ack_pending_count,
            delta_pruned_count = cache_outcome.delta_pruned_count,
            expiry_delta_count = cache_outcome.expiry_delta_count,
            hard_limit_reason = cache_outcome.hard_limit_reason,
            sequence_gap_count = cache_outcome.sequence_gap_count,
            reconcile_outcome = cache_outcome.cache_bypass_reason,
            live_overlay_elapsed_ms,
            preview_read_mode = build_telemetry.preview_read_mode,
            candidate_preview_id_count = build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build_telemetry.hydrated_preview_row_count,
            aggregation_mode = if build_telemetry.account_aggregation.fallback_hour_count == 0 { "rollup_plus_boundary" } else { "partial_rollup_with_exact_fallback" },
            covered_hour_count = build_telemetry.account_aggregation.covered_hour_count,
            fallback_hour_count = build_telemetry.account_aggregation.fallback_hour_count,
            boundary_tail_count = build_telemetry.account_aggregation.boundary_tail_count,
            rollup_row_count = build_telemetry.account_aggregation.rollup_row_count,
            raw_fallback_range_count = build_telemetry.account_aggregation.raw_fallback_range_count,
            model_performance_available = response.summary.model_performance.available,
            elapsed_ms,
            "dashboard activity snapshot completed"
        );
    }

    Ok(Json(response))
}

pub(crate) async fn fetch_dashboard_activity_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardActivityRecentQuery>,
) -> Result<Json<DashboardActivityRecentResponse>, ApiError> {
    let recent_limit = validate_dashboard_activity_params(
        "dashboard-activity/recent",
        "today",
        params.recent_limit,
    )?;
    let range_start = parse_to_utc_datetime(&params.range_start)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invalid rangeStart")))?;
    let range_end = parse_to_utc_datetime(&params.range_end)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invalid rangeEnd")))?;
    if range_start >= range_end
        || range_end - range_start > ChronoDuration::days(7)
        || params.snapshot_id != range_end.timestamp_millis()
    {
        return Err(ApiError::bad_request(anyhow!(
            "snapshotId must match rangeEnd and range must be between 0 and 7 days"
        )));
    }
    let range = ExactUtcRange {
        start: range_start,
        end: range_end,
    };
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let mut rows = query_live_upstream_account_activity_preview_rows_per_account_limit(
        &state.pool,
        source_scope,
        range,
        recent_limit,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard_recent",
            builder: "dashboard_recent",
            purpose: "bounded_per_account_recent",
        },
    )
    .await?;
    overlay_runtime_upstream_account_activity_preview_rows(
        state.as_ref(),
        &mut rows,
        source_scope,
        range,
    );
    overlay_runtime_terminal_upstream_account_activity_preview_rows(
        state.as_ref(),
        &mut rows,
        source_scope,
        range,
    )
    .await?;
    if range.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let live_ids = rows.iter().map(|row| row.id).collect::<HashSet<_>>();
        let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
            &state.pool,
            source_scope,
            range,
            Some(&live_ids),
        )
        .await?;
        let archived_row_ids = archived_rows.iter().map(|row| row.id).collect::<Vec<_>>();
        let persisted_live_ids = query_live_upstream_account_activity_existing_invocation_ids(
            &state.pool,
            source_scope,
            &archived_row_ids,
        )
        .await?;
        archived_rows.retain(|row| !persisted_live_ids.contains(&row.id));
        rows.extend(archived_rows);
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut grouped =
        HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
    for row in rows {
        let invocations = grouped.entry(row.upstream_account_id).or_default();
        if invocations.len() < recent_limit {
            invocations.push(upstream_account_invocation_preview_from_row(row));
        }
    }
    let mut accounts = grouped
        .into_iter()
        .map(
            |(account_id, recent_invocations)| DashboardActivityRecentAccountResponse {
                account_key: account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                recent_invocations,
            },
        )
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));

    Ok(Json(DashboardActivityRecentResponse {
        range_start: format_utc_iso_precise(range.start),
        range_end: format_utc_iso_precise(range.end),
        snapshot_id: params.snapshot_id,
        accounts,
    }))
}
