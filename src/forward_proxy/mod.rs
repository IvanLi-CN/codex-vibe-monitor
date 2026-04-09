// Thin forward-proxy entry for reviewability; behavior is preserved via include! slices.

// DB storage and hourly aggregation queries.
include!("slices/storage_and_hourly_stats.rs");
// Settings/update/validation handlers and runtime sync helpers.
include!("slices/settings_validation_and_runtime_sync.rs");
// ForwardProxy manager core, xray wiring, response shapes, and in-module tests.
include!("slices/manager_xray_types_and_tests.rs");
