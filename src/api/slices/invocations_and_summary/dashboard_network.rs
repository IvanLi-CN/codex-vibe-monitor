use super::*;
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardNetworkBucketAggregate {
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct DashboardNetworkBucketRow {
    pub(crate) bucket_start_epoch_second: i64,
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
}

pub(crate) fn validate_dashboard_network_timeseries_range(range: &str) -> Result<(), ApiError> {
    if matches!(range, "today" | "yesterday" | "1d") {
        return Ok(());
    }
    Err(ApiError::bad_request(anyhow!(
        "unsupported dashboard network range: {range}"
    )))
}

pub(crate) fn dashboard_network_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
             END, \
             {alias}.response_raw_size, \
             CAST(LENGTH({alias}.raw_response) AS INTEGER), \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
             END, \
             {alias}.response_raw_size, \
             CAST(LENGTH({alias}.raw_response) AS INTEGER), \
             0 \
           ) \
         END"
    )
}

pub(crate) fn dashboard_network_direct_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) \
         END"
    )
}

pub(crate) fn dashboard_network_direct_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             {}, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             {}, \
             0 \
           ) \
         END",
        dashboard_network_download_bytes_sql(alias),
        dashboard_network_download_bytes_sql(alias),
    )
}

pub(crate) fn dashboard_network_pool_attempt_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) \
         END"
    )
}

pub(crate) fn dashboard_network_pool_attempt_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) \
         END"
    )
}

pub(crate) fn invocation_payload_upstream_account_id_sql(alias: &str) -> String {
    format!(
        "CASE WHEN json_valid({alias}.payload) \
           THEN CAST(json_extract({alias}.payload, '$.upstreamAccountId') AS INTEGER) \
         END"
    )
}

pub(crate) async fn query_dashboard_network_bucket_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    _created_before: Option<&str>,
) -> Result<Vec<DashboardNetworkBucketRow>, ApiError> {
    if range.start >= range.end {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            ((bucket_start_epoch / 300) * 300) AS bucket_start_epoch_second,
            SUM(upload_bytes) AS upload_bytes,
            SUM(download_bytes) AS download_bytes
        FROM upstream_socket_network_minute
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(range.start.timestamp())
        .push(" AND bucket_start_epoch < ")
        .push_bind(range.end.timestamp());
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query.push(" AND ");
        if let Some(upstream_account_id) = upstream_account_id {
            query
                .push("upstream_account_id = ")
                .push_bind(upstream_account_id);
        } else {
            query.push("upstream_account_id IS NULL");
        }
    }
    query.push(" GROUP BY bucket_start_epoch_second ORDER BY bucket_start_epoch_second ASC");

    Ok(query
        .build_query_as::<DashboardNetworkBucketRow>()
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn query_dashboard_network_host_minute_bucket_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardNetworkBucketRow>, ApiError> {
    query_dashboard_network_bucket_rows(pool, source_scope, range, None, None).await
}

pub(crate) async fn load_dashboard_network_open_bucket_snapshot_for_scope(
    pool: &Pool<Sqlite>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    scope: DashboardNetworkScopeKey,
) -> Result<DashboardNetworkOpenBucketSnapshot, ApiError> {
    let read = dashboard_network_speed_cache.open_bucket_read_state(scope, range_end);
    if read.needs_seed {
        let seed_rows = query_dashboard_network_bucket_rows(
            pool,
            source_scope,
            ExactUtcRange {
                start: read.bucket_start,
                end: range_end.min(read.bucket_end),
            },
            scope.upstream_account_id(),
            None,
        )
        .await?;
        let seed_totals =
            seed_rows
                .into_iter()
                .fold(DashboardNetworkByteTotals::default(), |mut totals, row| {
                    totals.upload_bytes =
                        totals.upload_bytes.saturating_add(row.upload_bytes.max(0));
                    totals.download_bytes = totals
                        .download_bytes
                        .saturating_add(row.download_bytes.max(0));
                    totals
                });
        return Ok(dashboard_network_speed_cache.seed_open_bucket(
            scope,
            read.bucket_start,
            seed_totals,
            range_end,
        ));
    }

    Ok(dashboard_network_speed_cache.snapshot_open_bucket(scope, range_end))
}

pub(crate) async fn load_dashboard_network_open_bucket_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    upstream_account_id: Option<i64>,
) -> Result<DashboardNetworkOpenBucketSnapshot, ApiError> {
    load_dashboard_network_open_bucket_snapshot_for_scope(
        &state.pool,
        state.dashboard_network_speed_cache.as_ref(),
        source_scope,
        range_end,
        upstream_account_id
            .map(DashboardNetworkScopeKey::Account)
            .unwrap_or(DashboardNetworkScopeKey::Global),
    )
    .await
}

pub(crate) fn dashboard_network_bucket_rate(
    total_bytes: i64,
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
    range: ExactUtcRange,
) -> f64 {
    let effective_start = bucket_start.max(range.start);
    let effective_end = bucket_end.min(range.end);
    let effective_millis = effective_end
        .signed_duration_since(effective_start)
        .num_milliseconds()
        .max(1);
    total_bytes.max(0) as f64 / (effective_millis as f64 / 1000.0)
}

pub(crate) fn build_dashboard_network_realtime_rate_response(
    snapshot: DashboardNetworkRealtimeByteSnapshot,
) -> DashboardNetworkRealtimeRateResponse {
    DashboardNetworkRealtimeRateResponse {
        sample_start: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.sample_start_epoch_second, 0)
                .single()
                .expect("valid realtime sample start"),
        ),
        sample_end: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.sample_end_epoch_second, 0)
                .single()
                .expect("valid realtime sample end"),
        ),
        sample_seconds: snapshot.sample_seconds,
        upload_bytes_per_second: snapshot.totals.upload_bytes.max(0) as f64
            / snapshot.sample_seconds.max(1) as f64,
        download_bytes_per_second: snapshot.totals.download_bytes.max(0) as f64
            / snapshot.sample_seconds.max(1) as f64,
        upload_bytes: snapshot.totals.upload_bytes.max(0),
        download_bytes: snapshot.totals.download_bytes.max(0),
    }
}

pub(crate) fn build_dashboard_recent_network_window_response(
    snapshot: crate::dashboard_network_speed::DashboardRecentNetworkWindowSnapshot,
) -> DashboardRecentNetworkWindowResponse {
    DashboardRecentNetworkWindowResponse {
        range_start: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.range_start_epoch_second, 0)
                .single()
                .expect("valid recent network range start"),
        ),
        range_end: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.range_end_epoch_second, 0)
                .single()
                .expect("valid recent network range end"),
        ),
        window_seconds: snapshot.window_seconds,
        sample_seconds: snapshot.sample_seconds,
        is_warming_up: snapshot.is_warming_up,
        points: snapshot
            .points
            .into_iter()
            .map(|point| DashboardRecentNetworkWindowPointResponse {
                sample_start: format_utc_iso_precise(
                    Utc.timestamp_opt(point.sample_start_epoch_second, 0)
                        .single()
                        .expect("valid recent network sample start"),
                ),
                sample_end: format_utc_iso_precise(
                    Utc.timestamp_opt(point.sample_end_epoch_second, 0)
                        .single()
                        .expect("valid recent network sample end"),
                ),
                upload_bytes_per_second: if point.is_available {
                    point.totals.upload_bytes.max(0) as f64 / snapshot.sample_seconds.max(1) as f64
                } else {
                    0.0
                },
                download_bytes_per_second: if point.is_available {
                    point.totals.download_bytes.max(0) as f64
                        / snapshot.sample_seconds.max(1) as f64
                } else {
                    0.0
                },
                upload_bytes: point.totals.upload_bytes.max(0),
                download_bytes: point.totals.download_bytes.max(0),
                is_available: point.is_available,
            })
            .collect(),
    }
}

pub(crate) fn build_dashboard_network_timeseries_point_response(
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
    totals: DashboardNetworkByteTotals,
    range: ExactUtcRange,
    is_live_bucket: bool,
) -> DashboardNetworkTimeseriesPointResponse {
    DashboardNetworkTimeseriesPointResponse {
        bucket_start: format_utc_iso_precise(bucket_start),
        bucket_end: format_utc_iso_precise(bucket_end),
        upload_bytes_per_second: dashboard_network_bucket_rate(
            totals.upload_bytes,
            bucket_start,
            bucket_end,
            range,
        ),
        download_bytes_per_second: dashboard_network_bucket_rate(
            totals.download_bytes,
            bucket_start,
            bucket_end,
            range,
        ),
        upload_bytes: totals.upload_bytes,
        download_bytes: totals.download_bytes,
        is_live_bucket,
    }
}

pub(crate) async fn load_dashboard_network_live_bucket_point(
    pool: &Pool<Sqlite>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    scope: DashboardNetworkScopeKey,
) -> Result<DashboardNetworkTimeseriesPointResponse, ApiError> {
    let snapshot = load_dashboard_network_open_bucket_snapshot_for_scope(
        pool,
        dashboard_network_speed_cache,
        source_scope,
        range_end,
        scope,
    )
    .await?;
    Ok(build_dashboard_network_timeseries_point_response(
        snapshot.bucket_start,
        snapshot.bucket_end,
        snapshot.totals,
        ExactUtcRange {
            start: snapshot.bucket_start,
            end: range_end.min(snapshot.bucket_end),
        },
        true,
    ))
}

pub(crate) async fn fetch_dashboard_network_recent(
    State(state): State<Arc<AppState>>,
    Query(_params): Query<DashboardRecentNetworkWindowQuery>,
) -> Result<Json<DashboardRecentNetworkWindowResponse>, ApiError> {
    let started_at = Instant::now();
    let snapshot = state
        .dashboard_network_speed_cache
        .snapshot_recent_global_window(Utc::now());
    let response = build_dashboard_recent_network_window_response(snapshot);
    tracing::debug!(
        endpoint = "/api/stats/dashboard-network-recent",
        window_seconds = response.window_seconds,
        sample_seconds = response.sample_seconds,
        point_count = response.points.len(),
        is_warming_up = response.is_warming_up,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "dashboard recent network window completed"
    );
    Ok(Json(response))
}

pub(crate) async fn fetch_dashboard_network_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardNetworkTimeseriesQuery>,
) -> Result<Json<DashboardNetworkTimeseriesResponse>, ApiError> {
    let started_at = Instant::now();
    let response =
        build_dashboard_network_timeseries_response_at(state.as_ref(), &params, Utc::now()).await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    tracing::debug!(
        endpoint = "/api/stats/dashboard-network-timeseries",
        range = %params.range,
        upstream_account_id = params.upstream_account_id,
        bucket_count = response.points.len(),
        elapsed_ms,
        "dashboard network timeseries completed"
    );

    Ok(Json(response))
}

pub(crate) async fn build_dashboard_network_timeseries_response_at(
    state: &AppState,
    params: &DashboardNetworkTimeseriesQuery,
    now: DateTime<Utc>,
) -> Result<DashboardNetworkTimeseriesResponse, ApiError> {
    validate_dashboard_network_timeseries_range(params.range.as_str())?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window_at(params.range.as_str(), reporting_tz, now)
        .map_err(ApiError::from)?;
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
    let bucket_rows = if let Some(upstream_account_id) = params.upstream_account_id {
        query_dashboard_network_bucket_rows(
            &state.pool,
            source_scope,
            ExactUtcRange {
                start: range.start,
                end: closed_range_end,
            },
            Some(Some(upstream_account_id)),
            None,
        )
        .await?
    } else {
        query_dashboard_network_host_minute_bucket_rows(
            &state.pool,
            source_scope,
            ExactUtcRange {
                start: range.start,
                end: closed_range_end,
            },
        )
        .await?
    };

    let mut aggregates = bucket_rows
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
        .collect::<HashMap<_, _>>();

    let live_bucket_epoch_second = open_bucket_start.timestamp();
    let include_live_bucket = params.range != "yesterday"
        && open_bucket_start < range.end
        && open_bucket_start + ChronoDuration::seconds(DASHBOARD_NETWORK_BUCKET_SECONDS)
            > range.start;
    if include_live_bucket {
        let live_bucket = load_dashboard_network_open_bucket_snapshot(
            state,
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

    let first_bucket_epoch_second = range.start.timestamp()
        - range
            .start
            .timestamp()
            .rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let mut points = Vec::new();
    let mut bucket_epoch_second = first_bucket_epoch_second;
    // Compare the complete bucket start with the subsecond range end so the current bucket is
    // emitted even during the first fractional second after a bucket boundary.
    loop {
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

    Ok(DashboardNetworkTimeseriesResponse {
        range: params.range.clone(),
        range_start: format_utc_iso_precise(range.start),
        range_end: format_utc_iso_precise(range.end),
        snapshot_id: range.end.timestamp_millis(),
        bucket_seconds: DASHBOARD_NETWORK_BUCKET_SECONDS,
        points,
    })
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

#[cfg(test)]
mod upstream_account_activity_rate_tests {
    use super::*;

    fn utc_at(epoch: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(epoch, 0)
            .single()
            .expect("valid test epoch")
    }

    #[test]
    fn rates_use_recent_five_minute_active_tail() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 600_000,
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 720_000,
                total_tokens: 0,
                total_cost: 0.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 840_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }

    #[test]
    fn rates_return_zero_when_recent_window_has_no_usage() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: 600_000,
            total_tokens: 10_000,
            total_cost: 10.0,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert_eq!(tokens_per_minute, Some(0.0));
        assert_eq!(spend_rate, Some(0.0));
    }

    #[test]
    fn rates_exclude_events_before_mid_minute_tail_window() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 30)
            .single()
            .expect("valid range end");
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 1)
                    .single()
                    .expect("valid excluded event time")
                    .timestamp_millis(),
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 31)
                    .single()
                    .expect("valid included event time")
                    .timestamp_millis(),
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (100.0 / 5.0)).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.10 / 5.0)).abs() < 1e-9);
    }

    #[test]
    fn rates_floor_first_active_event_to_minute_boundary() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 0)
            .single()
            .expect("valid range end");
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: Utc
                .with_ymd_and_hms(2026, 7, 1, 12, 4, 59)
                .single()
                .expect("valid late-minute event time")
                .timestamp_millis(),
            total_tokens: 600,
            total_cost: 0.60,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - 600.0).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - 0.60).abs() < 1e-9);
    }

    #[test]
    fn rates_use_earliest_active_event_when_events_are_newest_first() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 900_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }
}

#[cfg(test)]
mod dashboard_network_timeseries_tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn network_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                invoke_id TEXT NOT NULL DEFAULT '',
                occurred_at TEXT NOT NULL,
                payload TEXT,
                response_raw_size INTEGER,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_size INTEGER,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE pool_upstream_request_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                upstream_account_id INTEGER,
                upstream_request_header_bytes_approx INTEGER,
                upstream_request_transmitted_body_bytes INTEGER,
                upstream_response_header_bytes_approx INTEGER,
                upstream_response_body_bytes INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create pool_upstream_request_attempts table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_host_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_host_network_minute table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_socket_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upstream_account_id INTEGER,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_base_url_host,
                    upstream_account_id
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_socket_network_minute table");
        pool
    }

    #[tokio::test]
    async fn dashboard_network_bucket_rows_read_real_socket_minute_rows() {
        let pool = network_pool().await;
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(
            Utc.with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
                .single()
                .expect("valid bucket start")
                .timestamp(),
        )
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(42_i64)
        .bind(128_i64)
        .bind(256_i64)
        .execute(&pool)
        .await
        .expect("insert dashboard socket minute row");

        let rows = query_dashboard_network_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 34, 0)
                    .single()
                    .expect("valid range start"),
                end: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 40, 0)
                    .single()
                    .expect("valid range end"),
            },
            Some(Some(42)),
            None,
        )
        .await
        .expect("query dashboard socket minute rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upload_bytes, 128);
        assert_eq!(rows[0].download_bytes, 256);
    }

    #[tokio::test]
    async fn dashboard_network_host_minute_rows_roll_up_into_five_minute_buckets() {
        let pool = network_pool().await;
        let bucket_start_epoch = Utc
            .with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
            .single()
            .expect("valid bucket start")
            .timestamp();
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6), (?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(Some(42_i64))
        .bind(100_i64)
        .bind(200_i64)
        .bind(bucket_start_epoch + 60)
        .bind(SOURCE_PROXY)
        .bind("backup.openai.com")
        .bind(Some(77_i64))
        .bind(40_i64)
        .bind(80_i64)
        .execute(&pool)
        .await
        .expect("insert upstream socket minute rows");

        let rows = query_dashboard_network_host_minute_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .expect("valid start"),
                end: Utc
                    .timestamp_opt(bucket_start_epoch + 300, 0)
                    .single()
                    .expect("valid end"),
            },
        )
        .await
        .expect("query socket minute network rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bucket_start_epoch_second, bucket_start_epoch);
        assert_eq!(rows[0].upload_bytes, 140);
        assert_eq!(rows[0].download_bytes, 280);
    }

    #[test]
    fn dashboard_network_realtime_rate_response_serializes_complete_second_snapshot() {
        let response =
            build_dashboard_network_realtime_rate_response(DashboardNetworkRealtimeByteSnapshot {
                sample_start_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 3, 59)
                    .single()
                    .expect("valid sample start")
                    .timestamp(),
                sample_end_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 4, 0)
                    .single()
                    .expect("valid sample end")
                    .timestamp(),
                sample_seconds: 1,
                totals: DashboardNetworkByteTotals {
                    upload_bytes: 2048,
                    download_bytes: 4096,
                },
            });

        assert_eq!(response.sample_start, "2026-07-19T18:03:59Z");
        assert_eq!(response.sample_end, "2026-07-19T18:04:00Z");
        assert_eq!(response.sample_seconds, 1);
        assert_eq!(response.upload_bytes_per_second, 2048.0);
        assert_eq!(response.download_bytes_per_second, 4096.0);

        let payload =
            serde_json::to_value(&response).expect("serialize dashboard network realtime rate");
        assert_eq!(
            payload,
            json!({
                "sampleStart": "2026-07-19T18:03:59Z",
                "sampleEnd": "2026-07-19T18:04:00Z",
                "sampleSeconds": 1,
                "uploadBytesPerSecond": 2048.0,
                "downloadBytesPerSecond": 4096.0,
                "uploadBytes": 2048,
                "downloadBytes": 4096
            })
        );
    }
}
