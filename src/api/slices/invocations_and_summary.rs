use super::*;
use crate::runtime_mutation_bus::{RuntimeMutation, SequencedRuntimeMutation};
use crate::{
    EXPLICIT_BILLING_PRICE_VERSION_SUFFIX, ProxyPricingMode, REQUESTED_TIER_PRICE_VERSION_SUFFIX,
    RESPONSE_TIER_PRICE_VERSION_SUFFIX, estimate_proxy_cost_breakdown, has_billable_usage,
    proxy_price_version,
};
use anyhow::anyhow;
use chrono::{TimeZone, Timelike};
use chrono_tz::TZ_VARIANTS;
use flate2::read::GzDecoder;
use futures_util::{StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::{
    FromRow, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use std::{fs, io};
use tracing::{debug, info};

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

include!("invocations_and_summary/foundation.rs");
include!("invocations_and_summary/invocation_select.rs");
include!("invocations_and_summary/invocation_runtime.rs");
include!("invocations_and_summary/invocation_list.rs");
include!("invocations_and_summary/invocation_paging.rs");
include!("invocations_and_summary/invocation_locate.rs");
include!("invocations_and_summary/invocation_workflow.rs");
include!("invocations_and_summary/invocation_workflow_timeline.rs");
include!("invocations_and_summary/invocation_workflow_detail.rs");
include!("invocations_and_summary/invocation_bodies.rs");
include!("invocations_and_summary/stats_entrypoints.rs");

include!("invocations_and_summary/summary_projection_types.rs");
include!("invocations_and_summary/summary_projection_state.rs");
include!("invocations_and_summary/summary_projection_response.rs");
include!("invocations_and_summary/summary_projection_response_helpers.rs");
include!("invocations_and_summary/summary_projection_records.rs");
include!("invocations_and_summary/summary_projection_rollups.rs");
include!("invocations_and_summary/summary_projection_ranges.rs");
include!("invocations_and_summary/summary_projection_archive.rs");
include!("invocations_and_summary/summary_projection_archive_rows.rs");
include!("invocations_and_summary/summary_projection_maintenance.rs");
include!("invocations_and_summary/summary_projection_recovery.rs");
include!("invocations_and_summary/summary_projection_refresh.rs");
include!("invocations_and_summary/summary_projection_archive_identity.rs");
include!("invocations_and_summary/summary_projection_coverage.rs");
include!("invocations_and_summary/summary_projection_coverage_page.rs");
include!("invocations_and_summary/summary_projection_checkpoint.rs");
include!("invocations_and_summary/summary_projection_v2_totals.rs");
include!("invocations_and_summary/summary_projection_overlay.rs");
include!("invocations_and_summary/summary_projection_publication.rs");
include!("invocations_and_summary/summary_projection_publication_helpers.rs");
include!("invocations_and_summary/summary_projection_boundary.rs");
include!("invocations_and_summary/summary_projection_build_entry.rs");
include!("invocations_and_summary/summary_projection_build_runtime.rs");
include!("invocations_and_summary/summary_projection_build_current.rs");
include!("invocations_and_summary/summary_projection_build_archive.rs");
include!("invocations_and_summary/summary_projection_build_setup.rs");
include!("invocations_and_summary/summary_projection_build_paged_hydration.rs");
include!("invocations_and_summary/summary_projection_build_hydration.rs");
include!("invocations_and_summary/summary_projection_build_planning.rs");
include!("invocations_and_summary/summary_projection_build_live.rs");
include!("invocations_and_summary/summary_projection_build_current_archive.rs");
include!("invocations_and_summary/summary_projection_build_materialization.rs");
include!("invocations_and_summary/summary_projection_build_all_time.rs");
include!("invocations_and_summary/summary_projection_build_all_time_helpers.rs");
include!("invocations_and_summary/summary_projection_build_terminal.rs");
include!("invocations_and_summary/summary_projection_build_finalization.rs");
include!("invocations_and_summary/summary_projection_build_finalization_helpers.rs");
include!("invocations_and_summary/summary_projection_build_pipeline.rs");
include!("invocations_and_summary/summary_projection_build_pipeline_helpers.rs");
include!("invocations_and_summary/summary_projection_build_pipeline_state.rs");
include!("invocations_and_summary/summary_projection_build_pipeline_hydration.rs");
include!("invocations_and_summary/summary_projection_build.rs");

include!("invocations_and_summary/summary_snapshot.rs");
include!("invocations_and_summary/summary_snapshot_live.rs");
include!("invocations_and_summary/summary_live_tokens.rs");

include!("invocations_and_summary/dashboard_activity_types.rs");
include!("invocations_and_summary/dashboard_activity_rates.rs");
include!("invocations_and_summary/dashboard_activity_performance.rs");
include!("invocations_and_summary/dashboard_activity_range.rs");
include!("invocations_and_summary/dashboard_activity_preview.rs");
include!("invocations_and_summary/dashboard_activity_historical_live.rs");
include!("invocations_and_summary/dashboard_activity_runtime.rs");
include!("invocations_and_summary/dashboard_activity_usage.rs");
include!("invocations_and_summary/dashboard_activity_read_model.rs");
include!("invocations_and_summary/dashboard_activity_terminal.rs");
include!("invocations_and_summary/dashboard_activity_cache.rs");
include!("invocations_and_summary/dashboard_activity_selection.rs");
include!("invocations_and_summary/dashboard_activity_snapshot_build.rs");
include!("invocations_and_summary/dashboard_activity_account_build.rs");
include!("invocations_and_summary/dashboard_activity_snapshot_cache_flow.rs");
include!("invocations_and_summary/dashboard_activity_snapshot_cache_finalization.rs");
include!("invocations_and_summary/dashboard_activity_endpoints.rs");
include!("invocations_and_summary/dashboard_network.rs");
include!("invocations_and_summary/summary_endpoints.rs");

include!("invocations_and_summary/tests/attempt_response_body_query.rs");
include!("invocations_and_summary/tests/dashboard_activity_read_model.rs");
include!("invocations_and_summary/tests/dashboard_activity_routing.rs");
include!("invocations_and_summary/tests/dashboard_activity_ttft_fallback.rs");
include!("invocations_and_summary/tests/dashboard_network_timeseries.rs");
include!("invocations_and_summary/tests/dashboard_recent_network_window_response.rs");
include!("invocations_and_summary/tests/invocation_cost_audit.rs");
include!("invocations_and_summary/tests/invocation_live_phase.rs");
include!("invocations_and_summary/tests/model_performance_duration_override.rs");
include!("invocations_and_summary/tests/upstream_account_activity_rate.rs");
include!("invocations_and_summary/tests/request_compression_query.rs");
