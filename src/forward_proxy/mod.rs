use super::*;

#[path = "slices/manager_xray_types_and_tests.rs"]
mod manager_xray_types_and_tests;
#[path = "slices/settings_validation_and_runtime_sync.rs"]
mod settings_validation_and_runtime_sync;
#[path = "slices/storage_and_hourly_stats.rs"]
#[expect(
    clippy::too_many_arguments,
    reason = "Forward-proxy statistics adapters preserve established call-site contracts."
)]
mod storage_and_hourly_stats;

pub(crate) use manager_xray_types_and_tests::*;
pub(crate) use settings_validation_and_runtime_sync::*;
pub(crate) use storage_and_hourly_stats::*;
