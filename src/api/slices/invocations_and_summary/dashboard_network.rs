pub(crate) async fn fetch_dashboard_network_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardNetworkTimeseriesQuery>,
) -> Result<Json<DashboardNetworkTimeseriesResponse>, ApiError> {
    let started_at = Instant::now();
    validate_dashboard_network_timeseries_range(params.range.as_str())?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window =
        resolve_range_window(params.range.as_str(), reporting_tz).map_err(ApiError::from)?;
    let range = ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    };
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    flush_dashboard_network_socket_minute_rollups(
        &state.pool,
        state.dashboard_network_speed_cache.as_ref(),
        range.end,
    )
    .await?;
    let open_bucket_start_epoch_second = range.end.timestamp()
        - range
            .end
            .timestamp()
            .rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let open_bucket_start = Utc
        .timestamp_opt(open_bucket_start_epoch_second, 0)
        .single()
        .expect("valid dashboard network bucket start");
    let closed_range_end = if params.range == "yesterday" {
        range.end
    } else {
        open_bucket_start.min(range.end)
    };
    let mut aggregates = load_dashboard_network_closed_bucket_aggregates(
        &state.pool,
        source_scope,
        range,
        closed_range_end,
        params.upstream_account_id,
    )
    .await?;

    let live_bucket_epoch_second = open_bucket_start.timestamp();
    let include_live_bucket = params.range != "yesterday"
        && open_bucket_start < range.end
        && open_bucket_start + ChronoDuration::seconds(DASHBOARD_NETWORK_BUCKET_SECONDS)
            > range.start;
    if include_live_bucket {
        let live_bucket = load_dashboard_network_open_bucket_snapshot(
            state.as_ref(),
            source_scope,
            range.end,
            params.upstream_account_id,
        )
        .await?;
        aggregates.insert(
            live_bucket.bucket_start.timestamp(),
            DashboardNetworkBucketAggregate {
                upload_bytes: live_bucket.totals.upload_bytes.max(0),
                download_bytes: live_bucket.totals.download_bytes.max(0),
            },
        );
    }

    let points = build_dashboard_network_timeseries_points(
        aggregates,
        range,
        include_live_bucket,
        live_bucket_epoch_second,
    );

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    tracing::debug!(
        endpoint = "/api/stats/dashboard-network-timeseries",
        range = %params.range,
        upstream_account_id = params.upstream_account_id,
        bucket_count = points.len(),
        elapsed_ms,
        "dashboard network timeseries completed"
    );

    Ok(Json(DashboardNetworkTimeseriesResponse {
        range: params.range,
        range_start: format_utc_iso_precise(range.start),
        range_end: format_utc_iso_precise(range.end),
        snapshot_id: range.end.timestamp_millis(),
        bucket_seconds: DASHBOARD_NETWORK_BUCKET_SECONDS,
        points,
    }))
}

async fn load_dashboard_network_closed_bucket_aggregates(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    closed_range_end: DateTime<Utc>,
    upstream_account_id: Option<i64>,
) -> Result<HashMap<i64, DashboardNetworkBucketAggregate>, ApiError> {
    let closed_range = ExactUtcRange {
        start: range.start,
        end: closed_range_end,
    };
    let rows = if let Some(upstream_account_id) = upstream_account_id {
        query_dashboard_network_bucket_rows(
            pool,
            source_scope,
            closed_range,
            Some(Some(upstream_account_id)),
            None,
        )
        .await?
    } else {
        query_dashboard_network_host_minute_bucket_rows(pool, source_scope, closed_range).await?
    };
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.bucket_start_epoch_second,
                DashboardNetworkBucketAggregate {
                    upload_bytes: row.upload_bytes.max(0),
                    download_bytes: row.download_bytes.max(0),
                },
            )
        })
        .collect())
}

fn build_dashboard_network_timeseries_points(
    aggregates: HashMap<i64, DashboardNetworkBucketAggregate>,
    range: ExactUtcRange,
    include_live_bucket: bool,
    live_bucket_epoch_second: i64,
) -> Vec<DashboardNetworkTimeseriesPointResponse> {
    let first_bucket_epoch_second = range.start.timestamp()
        - range
            .start
            .timestamp()
            .rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let mut points = Vec::new();
    let mut bucket_epoch_second = first_bucket_epoch_second;
    while bucket_epoch_second < range.end.timestamp() {
        let bucket_start = Utc
            .timestamp_opt(bucket_epoch_second, 0)
            .single()
            .expect("valid dashboard network bucket point start");
        let bucket_end = bucket_start + ChronoDuration::seconds(DASHBOARD_NETWORK_BUCKET_SECONDS);
        if bucket_end <= range.start {
            bucket_epoch_second =
                bucket_epoch_second.saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS);
            continue;
        }
        if bucket_start >= range.end {
            break;
        }
        let aggregate = aggregates
            .get(&bucket_epoch_second)
            .copied()
            .unwrap_or_default();
        points.push(build_dashboard_network_timeseries_point_response(
            bucket_start,
            bucket_end,
            DashboardNetworkByteTotals {
                upload_bytes: aggregate.upload_bytes,
                download_bytes: aggregate.download_bytes,
            },
            range,
            include_live_bucket && bucket_epoch_second == live_bucket_epoch_second,
        ));
        bucket_epoch_second = bucket_epoch_second.saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS);
    }
    points
}

pub(crate) async fn fetch_upstream_account_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UpstreamAccountActivityQuery>,
) -> Result<Json<UpstreamAccountActivityResponse>, ApiError> {
    let started_at = Instant::now();
    let recent_limit = validate_dashboard_activity_params(
        "upstream-account-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range = resolve_dashboard_activity_exact_range(params.range.as_str(), reporting_tz)?;
    let build = load_dashboard_activity_account_build_result(
        state.as_ref(),
        params.range.as_str(),
        range,
        recent_limit,
        true,
        None,
        DashboardActivityAccountBuilderKind::UpstreamAccount,
    )
    .await?;
    let accounts = build
        .accounts
        .into_iter()
        .filter_map(dashboard_account_to_upstream_account)
        .collect::<Vec<_>>();
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 250 {
        tracing::warn!(
            route = "upstream_account",
            builder = "upstream_account",
            endpoint = "/api/stats/upstream-account-activity",
            range = %params.range,
            recent_limit,
            account_count = accounts.len(),
            preview_read_mode = build.build_telemetry.preview_read_mode,
            candidate_preview_id_count = build.build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build.build_telemetry.hydrated_preview_row_count,
            aggregation_mode = if build.build_telemetry.account_aggregation.fallback_hour_count == 0 { "rollup_plus_boundary" } else { "partial_rollup_with_exact_fallback" },
            covered_hour_count = build.build_telemetry.account_aggregation.covered_hour_count,
            fallback_hour_count = build.build_telemetry.account_aggregation.fallback_hour_count,
            boundary_tail_count = build.build_telemetry.account_aggregation.boundary_tail_count,
            rollup_row_count = build.build_telemetry.account_aggregation.rollup_row_count,
            raw_fallback_range_count = build.build_telemetry.account_aggregation.raw_fallback_range_count,
            elapsed_ms,
            "upstream account activity exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route = "upstream_account",
            builder = "upstream_account",
            endpoint = "/api/stats/upstream-account-activity",
            range = %params.range,
            recent_limit,
            account_count = accounts.len(),
            preview_read_mode = build.build_telemetry.preview_read_mode,
            candidate_preview_id_count = build.build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build.build_telemetry.hydrated_preview_row_count,
            aggregation_mode = if build.build_telemetry.account_aggregation.fallback_hour_count == 0 { "rollup_plus_boundary" } else { "partial_rollup_with_exact_fallback" },
            covered_hour_count = build.build_telemetry.account_aggregation.covered_hour_count,
            fallback_hour_count = build.build_telemetry.account_aggregation.fallback_hour_count,
            boundary_tail_count = build.build_telemetry.account_aggregation.boundary_tail_count,
            rollup_row_count = build.build_telemetry.account_aggregation.rollup_row_count,
            raw_fallback_range_count = build.build_telemetry.account_aggregation.raw_fallback_range_count,
            elapsed_ms,
            "upstream account activity completed"
        );
    }

    Ok(Json(UpstreamAccountActivityResponse {
        range: params.range,
        range_start: format_utc_iso(range.start),
        range_end: format_utc_iso(range.end),
        routing_state_version: current_routing_state_version(state.as_ref()),
        accounts,
    }))
}
