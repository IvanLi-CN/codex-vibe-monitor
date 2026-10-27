use anyhow::{Result, bail};

use crate::storage::analytics::{
    AnalyticsProjection, AnalyticsProjectionAdapter, AnalyticsProjectionCommit,
    AnalyticsProjectionOwner, AnalyticsProjectionStore, AnalyticsRollupRead, ProjectionBatch,
    ProjectionReadState, TrafficStatsAggregateRead, TrafficStatsAggregateRequest,
};

const AUTHORITY_READ_RETRIES: usize = 2;

/// API-facing authority contract for one rebuildable analytics consumer.
///
/// The runtime owner can inject this value into an HTTP, SSE, or background consumer without
/// exposing a `Pool<Sqlite>` to that call site. AppState construction and router replacement stay
/// outside this contract; pg08 owns that lifecycle wiring.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalyticsApiProjection<'a> {
    adapter: AnalyticsProjectionAdapter<'a>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalyticsApiProjectionSnapshot {
    pub(crate) projection: AnalyticsProjection,
    pub(crate) state: ProjectionReadState,
    pub(crate) rollup: Option<AnalyticsRollupRead>,
    pub(crate) live: Option<TrafficStatsAggregateRead>,
}

impl<'a> AnalyticsApiProjection<'a> {
    pub(crate) const fn from_adapter(adapter: AnalyticsProjectionAdapter<'a>) -> Self {
        Self { adapter }
    }

    pub(crate) const fn from_store(
        store: &'a AnalyticsProjectionStore,
        projection: AnalyticsProjection,
    ) -> Self {
        Self::from_adapter(AnalyticsProjectionAdapter::new(store, projection))
    }

    pub(crate) const fn projection(self) -> AnalyticsProjection {
        self.adapter.projection()
    }

    pub(crate) fn require_projection(self, expected: AnalyticsProjection) -> Result<Self> {
        if self.projection() != expected {
            bail!(
                "analytics API projection mismatch: expected {}, got {}",
                expected.name(),
                self.projection().name()
            );
        }
        Ok(self)
    }

    pub(crate) async fn initialize(self) -> Result<()> {
        self.adapter.initialize().await
    }

    pub(crate) async fn owner(self) -> Result<AnalyticsProjectionOwner> {
        self.adapter.owner().await
    }

    pub(crate) async fn projection_state(self) -> Result<ProjectionReadState> {
        self.adapter.projection_state().await
    }

    pub(crate) async fn read_state_snapshot(self) -> Result<AnalyticsApiProjectionSnapshot> {
        let state = self.projection_state().await?;
        Ok(AnalyticsApiProjectionSnapshot {
            projection: self.projection(),
            state,
            rollup: None,
            live: None,
        })
    }

    /// Read the analytics baseline and optional bounded traffic tail as one API contract.
    ///
    /// The analytics state and rollup use separate short transactions, so a cutover can happen
    /// between them. A generation mismatch is retried once and then surfaced; callers never
    /// combine a checkpoint from one analytics authority with totals from another.
    pub(crate) async fn read_historical_snapshot(
        self,
        start_epoch: i64,
        end_epoch: i64,
        source_scope: crate::InvocationSourceScope,
        live_request: Option<TrafficStatsAggregateRequest>,
    ) -> Result<AnalyticsApiProjectionSnapshot> {
        for attempt in 0..AUTHORITY_READ_RETRIES {
            let state = self.projection_state().await?;
            let rollup = self
                .adapter
                .rollup_totals(start_epoch, end_epoch, source_scope)
                .await?;
            if rollup.target_generation != state.owner.target_generation {
                if attempt + 1 < AUTHORITY_READ_RETRIES {
                    continue;
                }
                bail!(
                    "analytics authority changed during {} API read (state generation {}, rollup generation {})",
                    self.projection().name(),
                    state.owner.target_generation,
                    rollup.target_generation
                );
            }
            let live = match live_request.as_ref() {
                Some(request) => {
                    let live = self.adapter.bounded_live_totals(request.clone()).await?;
                    if let Some(cursor) = state.last_good_cursor
                        && (live.source_generation != cursor.source_generation
                            || live.source_watermark < cursor.source_watermark
                            || live.read_through_id < cursor.row_id)
                    {
                        if attempt + 1 < AUTHORITY_READ_RETRIES {
                            continue;
                        }
                        bail!(
                            "analytics {} baseline cannot be combined with traffic live tail: baseline source generation {}, row {}, watermark {}; live source generation {}, read-through {}, watermark {}",
                            self.projection().name(),
                            cursor.source_generation,
                            cursor.row_id,
                            cursor.source_watermark,
                            live.source_generation,
                            live.read_through_id,
                            live.source_watermark
                        );
                    }
                    Some(live)
                }
                None => None,
            };
            return Ok(AnalyticsApiProjectionSnapshot {
                projection: self.projection(),
                state,
                rollup: Some(rollup),
                live,
            });
        }
        unreachable!("analytics authority read retry loop always returns or errors")
    }

    /// Start an API-owned projection publication. Rows and the checkpoint are committed through
    /// the returned transaction; the API layer cannot acknowledge a cursor independently.
    pub(crate) async fn begin_projection_commit(
        self,
        expected: ProjectionReadState,
        batch: ProjectionBatch,
    ) -> Result<AnalyticsProjectionCommit> {
        let transaction = self.adapter.begin().await?;
        let decision = transaction.plan_projection_commit(&expected, batch)?;
        transaction.prepare_projection_commit(expected, decision)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

    use super::*;

    static SQLITE_TEST_DATABASE_ID: AtomicUsize = AtomicUsize::new(0);

    async fn fixture() -> Result<(AnalyticsProjectionStore, SqlitePool)> {
        let database_id = SQLITE_TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
        let database_url = format!(
            "sqlite:file:analytics-api-projection-seam-{database_id}?mode=memory&cache=shared"
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await?;
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, occurred_at TEXT, status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT, payload TEXT, total_tokens INTEGER, cost REAL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE invocation_rollup_hourly (bucket_start_epoch INTEGER, source TEXT, total_count INTEGER, success_count INTEGER, failure_count INTEGER, total_cost REAL, total_tokens INTEGER, non_success_cost REAL, terminal_proof_complete INTEGER)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly VALUES (100, 'proxy', 2, 1, 1, 1.5, 20, 0.5, 1)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO codex_invocations (id, source, occurred_at, status, error_message, failure_class, total_tokens, cost) VALUES (1, 'proxy', '2026-10-27 10:00:00', 'completed', '', 'none', 10, 0.5)",
        )
        .execute(&pool)
        .await?;
        Ok((AnalyticsProjectionStore::sqlite(pool.clone()), pool))
    }

    fn batch(quality: crate::storage::analytics::ProjectionDataQuality) -> ProjectionBatch {
        ProjectionBatch {
            source_generation: 1,
            source_watermark: 1,
            next_row_id: 1,
            has_more: false,
            quality,
        }
    }

    #[tokio::test]
    async fn route_and_consumer_seams_bind_all_projection_owners() -> Result<()> {
        let (store, _pool) = fixture().await?;
        let summary = crate::summary_analytics_api_projection(&store);
        let timeseries = crate::timeseries_analytics_api_projection(&store);
        let long_term = crate::long_term_analytics_api_projection(&store);
        let prompt_cache = crate::prompt_cache_analytics_api_projection(&store);
        let subscription = crate::subscription_analytics_api_projection(&store);

        let bindings = [
            (summary, AnalyticsProjection::Summary),
            (timeseries, AnalyticsProjection::Timeseries),
            (long_term, AnalyticsProjection::LongTerm),
            (prompt_cache, AnalyticsProjection::PromptCache),
            (subscription, AnalyticsProjection::Subscription),
        ];
        for (projection, expected) in bindings {
            projection.initialize().await?;
            assert_eq!(projection.projection(), expected);
            assert_eq!(projection.owner().await?.projection, expected);
            assert_eq!(
                projection
                    .read_state_snapshot()
                    .await?
                    .state
                    .owner
                    .projection,
                expected
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn summary_http_seam_keeps_rollup_quality_and_bounded_live_source_separate() -> Result<()>
    {
        let (store, _pool) = fixture().await?;
        let projection = crate::summary_analytics_api_projection(&store);
        let snapshot = crate::read_summary_analytics_projection(
            projection,
            0,
            200,
            crate::InvocationSourceScope::ProxyOnly,
            Some(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::ProxyOnly,
                upper_bound_id: 1,
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("summary API seam failed: {error:?}"))?;

        assert_eq!(snapshot.projection, AnalyticsProjection::Summary);
        assert_eq!(
            snapshot.state.quality,
            crate::storage::analytics::ProjectionDataQuality::Unavailable
        );
        assert_eq!(snapshot.rollup.unwrap().aggregate.total_count, 2);
        let live = snapshot.live.unwrap();
        assert_eq!(live.source_generation, 1);
        assert_eq!(live.read_through_id, 1);
        assert_eq!(live.aggregate.total_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn live_tail_rejects_source_generation_or_cursor_boundary_skew() -> Result<()> {
        let (store, _pool) = fixture().await?;
        let projection = crate::summary_analytics_api_projection(&store);
        let expected = projection.projection_state().await?;
        let mut commit = projection
            .begin_projection_commit(
                expected,
                ProjectionBatch {
                    source_generation: 2,
                    source_watermark: 2,
                    next_row_id: 2,
                    has_more: false,
                    quality: crate::storage::analytics::ProjectionDataQuality::Fresh,
                },
            )
            .await?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;

        let error = crate::read_summary_analytics_projection(
            projection,
            0,
            200,
            crate::InvocationSourceScope::ProxyOnly,
            Some(TrafficStatsAggregateRequest {
                filter: crate::stats::StatsFilter::All,
                source_scope: crate::InvocationSourceScope::ProxyOnly,
                upper_bound_id: 1,
            }),
        )
        .await
        .expect_err("a source-generation switch cannot combine old analytics with a live tail");
        assert!(format!("{error:?}").contains("baseline cannot be combined"));
        Ok(())
    }

    #[tokio::test]
    async fn projection_write_seam_publishes_cursor_and_retains_last_good_on_partial_input()
    -> Result<()> {
        let (store, _pool) = fixture().await?;
        let projection = crate::summary_analytics_api_projection(&store);
        let expected = projection.projection_state().await?;
        let mut commit = projection
            .begin_projection_commit(
                expected,
                batch(crate::storage::analytics::ProjectionDataQuality::Fresh),
            )
            .await?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;

        let fresh = projection.projection_state().await?;
        assert_eq!(
            fresh.quality,
            crate::storage::analytics::ProjectionDataQuality::Fresh
        );
        assert_eq!(fresh.last_good_cursor.unwrap().row_id, 1);

        let mut commit = projection
            .begin_projection_commit(
                fresh,
                batch(crate::storage::analytics::ProjectionDataQuality::Partial),
            )
            .await?;
        commit.persist_checkpoint().await?;
        commit.commit().await?;
        let partial = projection.projection_state().await?;
        assert_eq!(
            partial.quality,
            crate::storage::analytics::ProjectionDataQuality::Partial
        );
        assert_eq!(partial.last_good_cursor.unwrap().row_id, 1);
        Ok(())
    }

    #[tokio::test]
    async fn timeseries_sse_and_prompt_cache_subscription_seams_use_their_own_state() -> Result<()>
    {
        let (store, _pool) = fixture().await?;
        let timeseries = crate::read_timeseries_analytics_projection(
            crate::timeseries_analytics_api_projection(&store),
            0,
            200,
            crate::InvocationSourceScope::All,
        )
        .await
        .map_err(|error| anyhow::anyhow!("timeseries API seam failed: {error:?}"))?;
        assert_eq!(timeseries.projection, AnalyticsProjection::Timeseries);
        assert!(timeseries.rollup.is_some());

        let prompt_cache = crate::read_prompt_cache_analytics_projection(
            crate::prompt_cache_analytics_api_projection(&store),
        )
        .await
        .map_err(|error| anyhow::anyhow!("prompt-cache API seam failed: {error:?}"))?;
        assert_eq!(prompt_cache.projection, AnalyticsProjection::PromptCache);

        let subscription = crate::read_subscription_analytics_projection(
            crate::subscription_analytics_api_projection(&store),
        )
        .await
        .map_err(|error| anyhow::anyhow!("subscription API seam failed: {error:?}"))?;
        assert_eq!(subscription.projection, AnalyticsProjection::Subscription);

        let long_term = crate::read_long_term_analytics_projection(
            crate::long_term_analytics_api_projection(&store),
        )
        .await?;
        assert_eq!(long_term.projection, AnalyticsProjection::LongTerm);
        Ok(())
    }
}
