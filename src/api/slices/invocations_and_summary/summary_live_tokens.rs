struct SummaryNonSuccessTokensLiveSnapshot {
    live_tokens: i64,
    live_group_row_count: usize,
    covered_hour_count: usize,
    boundary_tail_count: usize,
    fallback_hour_count: usize,
    live_row_scan_mode: &'static str,
}

enum SummaryNonSuccessTokensLoad<T> {
    Value(T),
    Locked(&'static str),
}

fn classify_summary_non_success_tokens_query<T>(
    result: Result<T, ApiError>,
    lock_reason: &'static str,
) -> Result<SummaryNonSuccessTokensLoad<T>, ApiError> {
    match result {
        Ok(value) => Ok(SummaryNonSuccessTokensLoad::Value(value)),
        Err(ApiError::Internal(error)) if crate::is_sqlite_lock_error(&error) => {
            Ok(SummaryNonSuccessTokensLoad::Locked(lock_reason))
        }
        Err(error) => Err(error),
    }
}

async fn load_summary_non_success_tokens_historical(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    historical_live_end: DateTime<Utc>,
) -> Result<SummaryNonSuccessTokensLoad<i64>, ApiError> {
    if range.start >= historical_live_end {
        return Ok(SummaryNonSuccessTokensLoad::Value(0));
    }
    classify_summary_non_success_tokens_query(
        query_non_success_tokens_exact_tail(
            pool,
            source_scope,
            upstream_account_id,
            ExactUtcRange {
                start: range.start,
                end: historical_live_end,
            },
        )
        .await,
        "historical_exact_tail",
    )
}

async fn load_summary_non_success_tokens_exact_tails(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    ranges: Vec<ExactUtcRange>,
    boundary_tail_count: usize,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<SummaryNonSuccessTokensLoad<(i64, bool)>, ApiError> {
    let mut exact_tokens = 0_i64;
    let mut used_fallback = false;
    for (index, exact_range) in ranges.into_iter().enumerate() {
        let tokens = match classify_summary_non_success_tokens_query(
            query_non_success_tokens_exact_tail(
                &mut *tx,
                source_scope,
                upstream_account_id,
                exact_range,
            )
            .await,
            "exact_tail",
        )? {
            SummaryNonSuccessTokensLoad::Value(tokens) => tokens,
            SummaryNonSuccessTokensLoad::Locked(reason) => {
                return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
            }
        };
        exact_tokens += tokens;
        used_fallback = true;
        if index >= boundary_tail_count {
            tracing::debug!(
                route = telemetry.route_label(),
                builder = "summary_non_success_tokens",
                purpose = "coverage_hole",
                source_scope = ?source_scope,
                start = %exact_range.start,
                end = %exact_range.end,
                "summary non-success token exact fallback covered an hourly hole"
            );
        }
    }
    Ok(SummaryNonSuccessTokensLoad::Value((
        exact_tokens,
        used_fallback,
    )))
}

struct SummaryNonSuccessTokensRollup {
    tokens: i64,
    covered_hour_count: usize,
    boundary_tail_count: usize,
    fallback_hour_count: usize,
    exact_ranges: Vec<ExactUtcRange>,
}

async fn load_summary_non_success_tokens_rollup(
    state: &AppState,
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    live_range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<SummaryNonSuccessTokensLoad<SummaryNonSuccessTokensRollup>, ApiError> {
    let plan = match classify_summary_non_success_tokens_query(
        plan_account_activity_range_tx(state, &mut *tx, live_range).await,
        "coverage_planner",
    )? {
        SummaryNonSuccessTokensLoad::Value(plan) => plan,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    let covered_hour_count = plan.covered_hours.len();
    let fallback_hour_count = plan.uncovered_hours.len();
    let boundary_tail_count = plan.boundary_tail_count;
    let rollup_tokens = match classify_summary_non_success_tokens_query(
        query_account_activity_v2_non_success_tokens(
            &mut *tx,
            source_scope,
            upstream_account_id,
            &plan.covered_hours,
        )
        .await,
        "hourly_rollup",
    )? {
        SummaryNonSuccessTokensLoad::Value(tokens) => tokens,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    let repair_cursor = match load_hourly_rollup_live_progress_tx(
        &mut *tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await
    {
        Ok(cursor) => cursor,
        Err(error) if crate::is_sqlite_lock_error(&error) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked("v2_repair_cursor"));
        }
        Err(error) => return Err(error.into()),
    };
    let bucket_watermarks = match classify_summary_non_success_tokens_query(
        load_account_activity_v2_bucket_repair_watermarks(&mut *tx, &plan.covered_hours).await,
        "v2_bucket_watermarks",
    )? {
        SummaryNonSuccessTokensLoad::Value(watermarks) => watermarks,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    let unreplayed_live_tail = match classify_summary_non_success_tokens_query(
        query_non_success_tokens_unreplayed_live_tail(
            &mut *tx,
            source_scope,
            upstream_account_id,
            &plan.covered_hours,
            repair_cursor,
            &bucket_watermarks,
        )
        .await,
        "unreplayed_live_tail",
    )? {
        SummaryNonSuccessTokensLoad::Value(tokens) => tokens,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    if unreplayed_live_tail != 0 {
        tracing::debug!(
            route = telemetry.route_label(),
            builder = "summary_non_success_tokens",
            purpose = "unreplayed_live_tail",
            repair_cursor,
            covered_hour_count,
            "summary non-success token rollup included unreplayed live tail"
        );
    }
    Ok(SummaryNonSuccessTokensLoad::Value(
        SummaryNonSuccessTokensRollup {
            tokens: rollup_tokens + unreplayed_live_tail,
            covered_hour_count,
            boundary_tail_count,
            fallback_hour_count,
            exact_ranges: plan.exact_ranges,
        },
    ))
}

async fn load_summary_non_success_tokens_live_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    live_range: ExactUtcRange,
    mut live_tokens: i64,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<SummaryNonSuccessTokensLoad<SummaryNonSuccessTokensLiveSnapshot>, ApiError> {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) if crate::is_sqlite_lock_error(&anyhow!(error.to_string())) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked("live_snapshot"));
        }
        Err(error) => return Err(error.into()),
    };
    let rollup = match load_summary_non_success_tokens_rollup(
        state,
        tx.as_mut(),
        source_scope,
        upstream_account_id,
        live_range,
        telemetry,
    )
    .await?
    {
        SummaryNonSuccessTokensLoad::Value(rollup) => rollup,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    live_tokens += rollup.tokens;
    let (exact_tokens, used_fallback) = match load_summary_non_success_tokens_exact_tails(
        tx.as_mut(),
        source_scope,
        upstream_account_id,
        rollup.exact_ranges,
        rollup.boundary_tail_count,
        telemetry,
    )
    .await?
    {
        SummaryNonSuccessTokensLoad::Value(result) => result,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    live_tokens += exact_tokens;
    tx.commit().await?;
    Ok(SummaryNonSuccessTokensLoad::Value(
        SummaryNonSuccessTokensLiveSnapshot {
            live_tokens,
            live_group_row_count: rollup.covered_hour_count,
            covered_hour_count: rollup.covered_hour_count,
            boundary_tail_count: rollup.boundary_tail_count,
            fallback_hour_count: rollup.fallback_hour_count,
            live_row_scan_mode: if used_fallback { "fallback" } else { "none" },
        },
    ))
}

async fn load_summary_non_success_tokens_live_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<SummaryNonSuccessTokensLoad<SummaryNonSuccessTokensLiveSnapshot>, ApiError> {
    let historical_live_end = range.end.min(retention_cutoff);
    let historical_tokens = match load_summary_non_success_tokens_historical(
        &state.pool,
        source_scope,
        upstream_account_id,
        range,
        historical_live_end,
    )
    .await?
    {
        SummaryNonSuccessTokensLoad::Value(tokens) => tokens,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return Ok(SummaryNonSuccessTokensLoad::Locked(reason));
        }
    };
    let live_range = ExactUtcRange {
        start: range.start.max(retention_cutoff),
        end: range.end,
    };
    if live_range.start >= live_range.end {
        return Ok(SummaryNonSuccessTokensLoad::Value(
            SummaryNonSuccessTokensLiveSnapshot {
                live_tokens: historical_tokens,
                live_group_row_count: 0,
                covered_hour_count: 0,
                boundary_tail_count: 0,
                fallback_hour_count: 0,
                live_row_scan_mode: if historical_tokens == 0 {
                    "none"
                } else {
                    "fallback"
                },
            },
        ));
    }
    load_summary_non_success_tokens_live_range(
        state,
        source_scope,
        upstream_account_id,
        live_range,
        historical_tokens,
        telemetry,
    )
    .await
}

async fn load_summary_non_success_tokens_archive_overlap(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
    started_at: Instant,
    live_snapshot: &SummaryNonSuccessTokensLiveSnapshot,
) -> Result<Option<i64>, ApiError> {
    let archive_rows = match query_completed_invocation_archive_activity_aggregate_rows(
        &state.pool,
        source_scope,
        range,
    )
    .await
    {
        Ok(rows) => rows,
        Err(ApiError::Internal(error)) if crate::is_sqlite_lock_error(&error) => {
            tracing::warn!(
                route = telemetry.route_label(),
                builder = "summary_non_success_tokens",
                ?source_scope,
                upstream_account_id,
                window_kind = telemetry.window_kind,
                window_name = telemetry.window_name,
                aggregation_mode = "fallback",
                archive_overlap_strategy = "fallback",
                live_group_row_count = live_snapshot.live_group_row_count,
                archive_group_row_count = 0_usize,
                live_row_scan_mode = live_snapshot.live_row_scan_mode,
                live_id_overlap_scan_mode = "none",
                start = %range.start,
                end = %range.end,
                "summary archive aggregate overlap merge skipped because sqlite is locked"
            );
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let archive_group_row_count = archive_rows.aggregates.len();
    if !archive_rows.skipped_materialized_ranges.is_empty() {
        emit_summary_range_builder_telemetry(SummaryRangeBuilderTelemetryInput {
            telemetry,
            builder: "summary_non_success_tokens",
            source_scope,
            upstream_account_id,
            aggregation_mode: "fallback",
            archive_overlap_strategy: "fallback",
            live_group_row_count: live_snapshot.live_group_row_count,
            archive_group_row_count,
            live_row_scan_mode: live_snapshot.live_row_scan_mode,
            live_id_overlap_scan_mode: "none",
            elapsed_ms: started_at.elapsed().as_millis() as u64,
        });
        return Ok(None);
    }
    let archived_tokens = archive_rows
        .aggregates
        .into_iter()
        .filter(|row| {
            upstream_account_id.is_none_or(|account_id| row.upstream_account_id == Some(account_id))
        })
        .map(|row| row.non_success_tokens)
        .sum::<i64>();
    emit_summary_range_builder_telemetry(SummaryRangeBuilderTelemetryInput {
        telemetry,
        builder: "summary_non_success_tokens",
        source_scope,
        upstream_account_id,
        aggregation_mode: "rollup_plus_boundary",
        archive_overlap_strategy: "aggregate_merge",
        live_group_row_count: live_snapshot.live_group_row_count,
        archive_group_row_count,
        live_row_scan_mode: live_snapshot.live_row_scan_mode,
        live_id_overlap_scan_mode: "none",
        elapsed_ms: started_at.elapsed().as_millis() as u64,
    });
    Ok(Some(live_snapshot.live_tokens + archived_tokens))
}

pub(crate) async fn load_non_success_tokens_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<Option<i64>, ApiError> {
    let started_at = Instant::now();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let live_snapshot = match load_summary_non_success_tokens_live_snapshot(
        state,
        source_scope,
        upstream_account_id,
        range,
        retention_cutoff,
        telemetry,
    )
    .await?
    {
        SummaryNonSuccessTokensLoad::Value(snapshot) => snapshot,
        SummaryNonSuccessTokensLoad::Locked(reason) => {
            return summary_non_success_tokens_lock_fallback(
                telemetry,
                source_scope,
                upstream_account_id,
                range,
                reason,
            );
        }
    };

    if range.start >= retention_cutoff {
        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        tracing::debug!(
            route = telemetry.route_label(),
            builder = "summary_non_success_tokens",
            ?source_scope,
            upstream_account_id,
            window_kind = telemetry.window_kind,
            window_name = telemetry.window_name,
            aggregation_mode = "rollup_plus_boundary",
            archive_overlap_strategy = "aggregate_merge",
            covered_hour_count = live_snapshot.covered_hour_count,
            fallback_hour_count = live_snapshot.fallback_hour_count,
            boundary_tail_count = live_snapshot.boundary_tail_count,
            live_group_row_count = live_snapshot.live_group_row_count,
            archive_group_row_count = 0_usize,
            live_row_scan_mode = live_snapshot.live_row_scan_mode,
            live_id_overlap_scan_mode = "none",
            elapsed_ms,
            "summary non-success token rollup completed"
        );
        return Ok(Some(live_snapshot.live_tokens));
    }
    load_summary_non_success_tokens_archive_overlap(
        state,
        source_scope,
        upstream_account_id,
        range,
        telemetry,
        started_at,
        &live_snapshot,
    )
    .await
}

pub(crate) async fn build_empty_summary_response(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<StatsResponse, ApiError> {
    let mut response = StatsResponse {
        total_count: 0,
        success_count: 0,
        failure_count: 0,
        total_cost: 0.0,
        total_tokens: 0,
        usage_breakdown: None,
        in_progress_conversation_count: None,
        in_progress_retry_conversation_count: None,
        in_progress_avg_wait_ms: None,
        in_progress_phase_counts: None,
        non_success_cost: Some(0.0),
        non_success_tokens: None,
        maintenance: Some(load_stats_maintenance_response(state).await?),
    };
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        upstream_account_id,
        None,
        SummaryLiveAugmentationPolicy {
            include_in_progress: true,
            include_non_success_tokens: false,
        },
        None,
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    Ok(response)
}
