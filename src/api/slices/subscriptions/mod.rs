use super::*;
use crate::db_pressure::{DbPressureDenyReason, DbPressureGate};
use crate::observability::contention::spawn_performance;
use crate::storage::analytics::{
    AnalyticsProjection, AnalyticsProjectionAdapter, AnalyticsProjectionStore,
};
use axum::http::header;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde_json::json;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

pub(crate) const SUBSCRIPTION_ANALYTICS_PROJECTION: AnalyticsProjection =
    AnalyticsProjection::Subscription;

pub(crate) fn subscription_analytics_adapter<'a>(
    store: &'a AnalyticsProjectionStore,
) -> AnalyticsProjectionAdapter<'a> {
    AnalyticsProjectionAdapter::new(store, SUBSCRIPTION_ANALYTICS_PROJECTION)
}

pub(crate) fn subscription_analytics_api_projection<'a>(
    store: &'a AnalyticsProjectionStore,
) -> crate::api::AnalyticsApiProjection<'a> {
    crate::api::AnalyticsApiProjection::from_adapter(subscription_analytics_adapter(store))
}

/// SSE topic refreshes use this seam for the subscription checkpoint/quality marker. The runtime
/// supplies the shared analytics store through AppState while the topic payload remains local.
pub(crate) async fn read_subscription_analytics_projection(
    projection: crate::api::AnalyticsApiProjection<'_>,
) -> Result<crate::api::AnalyticsApiProjectionSnapshot, ApiError> {
    projection
        .require_projection(SUBSCRIPTION_ANALYTICS_PROJECTION)?
        .read_state_snapshot()
        .await
        .map_err(ApiError::from)
}

mod config;
mod hub_lifecycle;
mod hub_projection;
mod hub_projection_runtime;
mod hub_recovery;
mod materializers;
mod protocol;
mod query;
mod server_push;
mod state;
mod topic;

#[cfg(test)]
mod tests;

pub(crate) use config::*;
pub(crate) use hub_recovery::*;
pub(crate) use materializers::*;
pub(crate) use protocol::*;
pub(crate) use query::*;
pub(crate) use server_push::*;
pub(crate) use state::*;
pub(crate) use topic::*;

#[path = "topic_builders.rs"]
mod topic_builders;
