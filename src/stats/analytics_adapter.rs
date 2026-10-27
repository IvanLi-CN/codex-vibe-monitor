use anyhow::Result;

use crate::storage::analytics::{
    AnalyticsProjection, AnalyticsProjectionAdapter, AnalyticsProjectionOwner,
    AnalyticsProjectionStore, AnalyticsProjectionTransaction, AnalyticsRollupRead,
    ProjectionReadState, TrafficStatsAggregateRead, TrafficStatsAggregateRequest,
};

pub(crate) const STATS_SUMMARY_PROJECTION: AnalyticsProjection = AnalyticsProjection::Summary;
pub(crate) const STATS_TIMESERIES_PROJECTION: AnalyticsProjection = AnalyticsProjection::Timeseries;
pub(crate) const STATS_LONG_TERM_PROJECTION: AnalyticsProjection = AnalyticsProjection::LongTerm;
pub(crate) const STATS_PROMPT_CACHE_PROJECTION: AnalyticsProjection =
    AnalyticsProjection::PromptCache;
pub(crate) const STATS_SUBSCRIPTION_PROJECTION: AnalyticsProjection =
    AnalyticsProjection::Subscription;
pub(crate) const STATS_ANALYTICS_PROJECTION: AnalyticsProjection = STATS_SUMMARY_PROJECTION;

/// Read-side adapter for the stats API. It keeps the HTTP/query layer independent of the
/// authority backend: historical reads stay on the analytics projection, while a bounded live
/// tail can be admitted from the authoritative traffic module during a staggered cutover.
pub(crate) struct StatsAnalyticsAdapter<'a> {
    authority: AnalyticsProjectionAdapter<'a>,
}

impl<'a> StatsAnalyticsAdapter<'a> {
    pub(crate) fn new(store: &'a AnalyticsProjectionStore) -> Self {
        Self::for_projection(store, STATS_SUMMARY_PROJECTION)
    }

    pub(crate) fn for_projection(
        store: &'a AnalyticsProjectionStore,
        projection: AnalyticsProjection,
    ) -> Self {
        Self {
            authority: AnalyticsProjectionAdapter::new(store, projection),
        }
    }

    pub(crate) fn projection(&self) -> AnalyticsProjection {
        self.authority.projection()
    }

    pub(crate) async fn initialize(&self) -> Result<()> {
        self.authority.initialize().await
    }

    pub(crate) async fn owner(&self) -> Result<AnalyticsProjectionOwner> {
        self.authority.owner().await
    }

    pub(crate) async fn bounded_live_totals(
        &self,
        request: TrafficStatsAggregateRequest,
    ) -> Result<TrafficStatsAggregateRead> {
        self.authority.bounded_live_totals(request).await
    }

    pub(crate) async fn rollup_totals(
        &self,
        start_epoch: i64,
        end_epoch: i64,
        source_scope: crate::InvocationSourceScope,
    ) -> Result<AnalyticsRollupRead> {
        self.authority
            .rollup_totals(start_epoch, end_epoch, source_scope)
            .await
    }

    pub(crate) async fn projection_state(&self) -> Result<ProjectionReadState> {
        self.authority.projection_state().await
    }

    pub(crate) async fn begin(&self) -> Result<AnalyticsProjectionTransaction> {
        self.authority.begin().await
    }
}
