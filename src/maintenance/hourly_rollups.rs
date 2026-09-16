#[path = "hourly_rollup_archive_support.rs"]
mod hourly_rollup_archive_support;
pub(crate) use hourly_rollup_archive_support::*;

include!("hourly_rollups/part-00.rs");
include!("hourly_rollups/part-01.rs");
include!("hourly_rollups/part-02.rs");
include!("hourly_rollups/part-03.rs");
include!("hourly_rollups/part-04.rs");
include!("hourly_rollups/part-05.rs");
