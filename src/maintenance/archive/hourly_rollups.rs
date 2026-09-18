use super::*;
use anyhow::anyhow;
use sqlx::FromRow;
use std::future::Future;
use tracing::warn;

#[path = "hourly_rollup_support.rs"]
mod archive_hourly_rollup_support;
pub(crate) use archive_hourly_rollup_support::*;

pub(crate) const PARALLEL_WORK_MINUTE_ROLLUP_RETAINED_COMPLETE_SHANGHAI_DAYS: i64 = 30;

include!("hourly_rollups/part-01.rs");
include!("hourly_rollups/part-02.rs");
include!("hourly_rollups/invocation_rollup_accumulation.rs");
include!("hourly_rollups/invocation_rollup_persistence_01.rs");
include!("hourly_rollups/invocation_rollup_persistence_02.rs");
include!("hourly_rollups/invocation_rollup_persistence_03.rs");
include!("hourly_rollups/invocation_rollup_persistence_04.rs");
include!("hourly_rollups/part-03.rs");
include!("hourly_rollups/part-04.rs");
include!("hourly_rollups/part-05.rs");
include!("hourly_rollups/part-06.rs");
include!("hourly_rollups/part-07.rs");
include!("hourly_rollups/part-08.rs");
#[cfg(test)]
mod upstream_host_network_minute_tests {
    include!("hourly_rollups/tests/upstream_host_network.rs");
}
#[cfg(test)]
mod retention_breakdown_materialization_tests {
    include!("hourly_rollups/tests/retention_breakdown.rs");
}
