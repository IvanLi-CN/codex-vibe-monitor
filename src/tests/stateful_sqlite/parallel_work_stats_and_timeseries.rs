#[allow(unused_imports)]
use super::*;
#[path = "parallel_work_stats_and_timeseries/pruned_summary_archives.rs"]
mod pruned_summary_archives;
use crate::api::DashboardTerminalProjectionSlice;
use serde_json::json;
use std::sync::{Arc, LazyLock};

#[path = "parallel_work_stats_and_timeseries/parallel_work_account_activity_v2_repairs.rs"]
mod parallel_work_account_activity_v2_repairs;
#[path = "parallel_work_stats_and_timeseries/parallel_work_dashboard_archive_fallback_and_recents.rs"]
mod parallel_work_dashboard_archive_fallback_and_recents;
#[path = "parallel_work_stats_and_timeseries/parallel_work_dashboard_subscriptions_and_live_slices.rs"]
mod parallel_work_dashboard_subscriptions_and_live_slices;
#[path = "parallel_work_stats_and_timeseries/parallel_work_dashboard_summary_and_cache.rs"]
mod parallel_work_dashboard_summary_and_cache;
#[path = "parallel_work_stats_and_timeseries/parallel_work_exact_archive_admission.rs"]
mod parallel_work_exact_archive_admission;
#[path = "parallel_work_stats_and_timeseries/parallel_work_exact_key_streaming.rs"]
mod parallel_work_exact_key_streaming;
#[path = "parallel_work_stats_and_timeseries/parallel_work_failure_archive_fallback.rs"]
mod parallel_work_failure_archive_fallback;
#[path = "parallel_work_stats_and_timeseries/parallel_work_failure_classification_and_runtime_overlays.rs"]
mod parallel_work_failure_classification_and_runtime_overlays;
#[path = "parallel_work_stats_and_timeseries/parallel_work_fixture_support_and_archive_seeding.rs"]
mod parallel_work_fixture_support_and_archive_seeding;
#[path = "parallel_work_stats_and_timeseries/parallel_work_historical_stats_and_timeseries.rs"]
mod parallel_work_historical_stats_and_timeseries;
#[path = "parallel_work_stats_and_timeseries/parallel_work_minute_projection_and_startup_recovery.rs"]
mod parallel_work_minute_projection_and_startup_recovery;
#[path = "parallel_work_stats_and_timeseries/parallel_work_minute_projection_bounded_recovery.rs"]
mod parallel_work_minute_projection_bounded_recovery;
#[path = "parallel_work_stats_and_timeseries/parallel_work_retention_and_minute_rollups.rs"]
mod parallel_work_retention_and_minute_rollups;
#[path = "parallel_work_stats_and_timeseries/parallel_work_rollup_repair_and_projection_fences.rs"]
mod parallel_work_rollup_repair_and_projection_fences;
#[path = "parallel_work_stats_and_timeseries/parallel_work_stats_windows_and_etags.rs"]
mod parallel_work_stats_windows_and_etags;
#[path = "parallel_work_stats_and_timeseries/parallel_work_summary_archive_replay.rs"]
mod parallel_work_summary_archive_replay;
#[path = "parallel_work_stats_and_timeseries/parallel_work_summary_usage_and_account_scopes.rs"]
mod parallel_work_summary_usage_and_account_scopes;
#[path = "parallel_work_stats_and_timeseries/parallel_work_timeseries_success_failure_metrics.rs"]
mod parallel_work_timeseries_success_failure_metrics;
#[path = "parallel_work_stats_and_timeseries/parallel_work_timeseries_windows_and_rollups.rs"]
mod parallel_work_timeseries_windows_and_rollups;

pub(crate) use parallel_work_fixture_support_and_archive_seeding::*;
