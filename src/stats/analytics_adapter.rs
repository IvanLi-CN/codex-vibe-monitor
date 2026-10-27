use anyhow::Result;

use crate::storage::analytics::{
    AnalyticsProjection, AnalyticsProjectionOwner, AnalyticsProjectionStore, ProjectionReadState,
    TrafficStatsAggregateRead, TrafficStatsAggregateRequest,
};

pub(crate) const STATS_ANALYTICS_PROJECTION: AnalyticsProjection = AnalyticsProjection::Summary;

/// Read-side adapter for the stats API. It keeps the HTTP/query layer independent of the
/// authority backend: historical reads stay on the analytics projection, while a bounded live
/// tail can be admitted from the authoritative traffic module during a staggered cutover.
pub(crate) struct StatsAnalyticsAdapter<'a> {
    store: &'a AnalyticsProjectionStore,
}

impl<'a> StatsAnalyticsAdapter<'a> {
    pub(crate) fn new(store: &'a AnalyticsProjectionStore) -> Self {
        Self { store }
    }

    pub(crate) async fn owner(&self) -> Result<AnalyticsProjectionOwner> {
        self.store
            .projection_owner(STATS_ANALYTICS_PROJECTION)
            .await
    }

    pub(crate) async fn bounded_live_totals(
        &self,
        request: TrafficStatsAggregateRequest,
    ) -> Result<TrafficStatsAggregateRead> {
        self.store.query_bounded_traffic_stats(request).await
    }

    pub(crate) async fn projection_state(&self) -> Result<ProjectionReadState> {
        self.store
            .load_projection_state(STATS_ANALYTICS_PROJECTION)
            .await
    }
}
