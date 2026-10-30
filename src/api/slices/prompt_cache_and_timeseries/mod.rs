use super::*;
use crate::storage::analytics::{
    AnalyticsProjection, AnalyticsProjectionAdapter, AnalyticsProjectionStore,
};

pub(crate) const SUMMARY_ANALYTICS_PROJECTION: AnalyticsProjection = AnalyticsProjection::Summary;
pub(crate) const TIMESERIES_ANALYTICS_PROJECTION: AnalyticsProjection =
    AnalyticsProjection::Timeseries;

pub(crate) fn summary_analytics_adapter<'a>(
    store: &'a AnalyticsProjectionStore,
) -> AnalyticsProjectionAdapter<'a> {
    AnalyticsProjectionAdapter::new(store, SUMMARY_ANALYTICS_PROJECTION)
}

pub(crate) fn timeseries_analytics_adapter<'a>(
    store: &'a AnalyticsProjectionStore,
) -> AnalyticsProjectionAdapter<'a> {
    AnalyticsProjectionAdapter::new(store, TIMESERIES_ANALYTICS_PROJECTION)
}

pub(crate) fn summary_analytics_api_projection<'a>(
    store: &'a AnalyticsProjectionStore,
) -> crate::api::AnalyticsApiProjection<'a> {
    crate::api::AnalyticsApiProjection::from_adapter(summary_analytics_adapter(store))
}

pub(crate) fn timeseries_analytics_api_projection<'a>(
    store: &'a AnalyticsProjectionStore,
) -> crate::api::AnalyticsApiProjection<'a> {
    crate::api::AnalyticsApiProjection::from_adapter(timeseries_analytics_adapter(store))
}

#[path = "shared.rs"]
pub(crate) mod prompt_cache_and_timeseries_shared;
pub(crate) use prompt_cache_and_timeseries_shared::*;
#[path = "summary_queries.rs"]
mod prompt_cache_and_timeseries_summary_queries;
pub(crate) use prompt_cache_and_timeseries_summary_queries::*;
#[path = "forward_proxy_stats.rs"]
mod prompt_cache_and_timeseries_forward_proxy_stats;
pub(crate) use prompt_cache_and_timeseries_forward_proxy_stats::*;
#[expect(
    clippy::too_many_arguments,
    reason = "Prompt-cache query and hydration adapters preserve established call-site contracts."
)]
mod prompt_cache_conversations;
pub(crate) use prompt_cache_conversations::*;
#[path = "timeseries.rs"]
#[expect(
    clippy::too_many_arguments,
    reason = "Timeseries aggregation adapters preserve established call-site contracts."
)]
pub(crate) mod prompt_cache_and_timeseries_timeseries;
pub(crate) use prompt_cache_and_timeseries_timeseries::*;
