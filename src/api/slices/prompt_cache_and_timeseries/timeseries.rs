use super::*;
use anyhow::anyhow;

#[path = "timeseries/aggregation.rs"]
mod aggregation;
#[path = "timeseries/materialization.rs"]
mod materialization;
#[path = "timeseries/minute_projection.rs"]
mod minute_projection;
#[path = "timeseries/parallel_work.rs"]
mod parallel_work;
#[path = "timeseries/queries.rs"]
mod queries;

pub(crate) use materialization::TimeseriesTopicMaterializedBase;

pub(crate) use parallel_work::{
    ParallelWorkProjectionBaseline, load_parallel_work_projection_baseline,
    load_parallel_work_stats_response,
};

pub(crate) use minute_projection::{
    flush_timeseries_minute_projection_managed, spawn_timeseries_minute_projection_supervisor,
};

#[cfg(test)]
pub(crate) use aggregation::{
    add_rollup_rows_to_timeseries_aggregates, fold_minute_projection_aggregates,
    timeseries_point_from_aggregate,
};
#[cfg(test)]
pub(crate) use minute_projection::{
    TimeseriesMinuteProjectionFlushOutcome, TimeseriesMinuteProjectionWarmOutcome,
    TimeseriesMinuteProjectionWriteAdmissionOutcome, load_timeseries_minute_projection_records,
    load_timeseries_minute_projection_v2, store_timeseries_minute_projection_records,
    store_timeseries_minute_projection_v2_for_test,
    timeseries_minute_projection_has_uncovered_terminal_delta,
    timeseries_minute_projection_pressure_deferred,
    timeseries_minute_projection_v2_snapshot_is_current,
    timeseries_projection_requires_exact_rebuild, try_acquire_timeseries_minute_projection_write,
};
#[cfg(test)]
pub(crate) use minute_projection::{
    flush_timeseries_minute_projection_with_coordinator,
    mark_timeseries_minute_projection_startup_recovery,
    prepare_timeseries_minute_projection_after_restart,
    store_timeseries_minute_projection_v2_warm_with_coordinator,
};
#[cfg(test)]
pub(crate) use parallel_work::load_parallel_work_stats_response_at;
#[cfg(test)]
pub(crate) use queries::{
    fetch_timeseries_from_hourly_rollups, timeseries_topic_uses_hourly_rollup_baseline,
};

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    queries::fetch_timeseries_query(state, params).await
}

#[cfg(test)]
#[path = "timeseries/tests/aggregation.rs"]
mod aggregation_tests;
#[cfg(test)]
#[path = "timeseries/tests/materialization.rs"]
mod materialization_tests;
#[cfg(test)]
#[path = "timeseries/tests/minute_projection.rs"]
mod minute_projection_tests;
#[cfg(test)]
#[path = "timeseries/test_support.rs"]
mod test_support;

#[cfg(test)]
pub(crate) use test_support::record;

#[cfg(test)]
pub(crate) async fn fetch_parallel_work_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    load_parallel_work_stats_response(&state, params)
        .await
        .map(Json)
}

#[cfg(test)]
pub(crate) async fn fetch_parallel_work_stats_at(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
    now: DateTime<Utc>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    load_parallel_work_stats_response_at(&state, params, now)
        .await
        .map(Json)
}

pub(crate) async fn fetch_parallel_work_stats_cached(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Response, ApiError> {
    let response = load_parallel_work_stats_response(&state, params).await?;
    let body = serde_json::to_vec(&response)
        .map_err(|err| ApiError::from(anyhow!("failed to serialize parallel-work stats: {err}")))?;
    let etag = parallel_work_stats_etag(&body);
    let mut response = if request_etag_matches(&headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response()
    };
    let etag_value = HeaderValue::from_str(&etag)
        .map_err(|err| ApiError::from(anyhow!("invalid parallel-work etag: {err}")))?;
    response
        .headers_mut()
        .insert(axum::http::header::ETAG, etag_value);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    Ok(response)
}

pub(crate) fn parallel_work_stats_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!("\"parallel-work-{digest:x}\"")
}

pub(crate) fn request_etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        })
        .unwrap_or(false)
}
