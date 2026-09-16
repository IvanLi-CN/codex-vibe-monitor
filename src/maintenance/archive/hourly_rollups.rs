#[path = "hourly_rollup_support.rs"]
mod archive_hourly_rollup_support;
pub(crate) use archive_hourly_rollup_support::*;

include!("hourly_rollups/part-00.rs");
include!("hourly_rollups/part-01.rs");
include!("hourly_rollups/part-02.rs");
include!("hourly_rollups/part-03.rs");
include!("hourly_rollups/part-04.rs");
include!("hourly_rollups/part-05.rs");
include!("hourly_rollups/part-06.rs");
include!("hourly_rollups/part-07.rs");
include!("hourly_rollups/part-08.rs");
