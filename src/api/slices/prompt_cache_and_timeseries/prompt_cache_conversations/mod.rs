use super::*;
use crate::observability::contention::{StatsQuery, observe_query};
use std::future::Future;

mod aggregate_queries;
mod bindings;
mod cache;
mod detail_queries;
mod hydration;
mod request;
mod response;

pub(crate) use aggregate_queries::*;
pub(crate) use bindings::*;
pub(crate) use cache::*;
pub(crate) use detail_queries::*;
pub(crate) use hydration::*;
pub(crate) use request::*;
pub(crate) use response::*;

pub(crate) async fn observe_prompt_cache_query<T, E>(
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    observe_query(StatsQuery::PromptCacheStatistics, future).await
}
