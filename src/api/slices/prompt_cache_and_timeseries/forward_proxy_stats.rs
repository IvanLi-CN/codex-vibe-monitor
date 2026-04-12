use super::*;

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
