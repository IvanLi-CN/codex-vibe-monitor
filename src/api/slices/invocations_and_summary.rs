use super::*;
use crate::observability::contention::{StatsQuery, observe_query};
use crate::runtime_mutation_bus::{RuntimeMutation, SequencedRuntimeMutation};
use crate::{
    EXPLICIT_BILLING_PRICE_VERSION_SUFFIX, ProxyPricingMode, REQUESTED_TIER_PRICE_VERSION_SUFFIX,
    RESPONSE_TIER_PRICE_VERSION_SUFFIX, estimate_proxy_cost_breakdown, has_billable_usage,
    proxy_price_version,
};
pub(crate) use anyhow::anyhow;
use chrono::{TimeZone, Timelike};
use chrono_tz::TZ_VARIANTS;
use flate2::read::GzDecoder;
use futures_util::{StreamExt, TryStreamExt};
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use serde_json::{Value, json};
use sha2::Digest;
pub(crate) use sqlx::{
    FromRow, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use std::{fs, io};
pub(crate) use tracing::{debug, info, warn};

mod dashboard_activity_archive_fallback;
mod dashboard_activity_endpoints;
mod dashboard_activity_preview;
mod dashboard_activity_query;
mod dashboard_activity_response;
mod dashboard_activity_runtime;
mod dashboard_activity_runtime_query;
mod dashboard_activity_snapshot_cache;
mod dashboard_activity_types;
mod dashboard_network;
mod invocation_history;
mod invocation_projection_sql;
mod invocation_query;
mod invocation_runtime_overlay;
#[path = "invocation_timeline.rs"]
mod invocation_timeline;
mod invocation_timing;
mod invocation_workflow_detail;
#[cfg(test)]
mod model_performance_duration_override_tests;
#[cfg(test)]
#[path = "../../tests/lightweight/statistics_observability.rs"]
mod statistics_observability;
mod summary_live_tail_transaction;
mod summary_observability;
mod summary_projection_archive;
mod summary_projection_boundary;
mod summary_projection_build;
mod summary_projection_core;
mod summary_projection_coverage;
#[cfg(test)]
#[path = "../../tests/lightweight/summary_fallback_contention.rs"]
mod summary_projection_fallback_contention;
mod summary_projection_lifecycle;
mod summary_projection_maintenance;
mod summary_projection_v2;
mod summary_recovery;
mod summary_topic_and_ranges;
#[cfg(test)]
pub(crate) use summary_live_tail_transaction::{
    SUMMARY_LIVE_TAIL_CLEANUP_READY, SummaryLiveTailTransaction,
};

pub(crate) use summary_observability::{
    fetch_summary, record_summary_projection_publication_metrics,
    summary_projection_pending_source_gaps,
};

pub(crate) use invocation_workflow_detail::{
    INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD, INVOCATION_COST_AUDIT_REASON_MODEL_MISSING,
    INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING,
    INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED,
    INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN,
    INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING,
    INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING,
    INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH, INVOCATION_COST_AUDIT_REASON_USAGE_MISSING,
    InvocationResponseBodyRow, InvocationWorkflowAttempt, InvocationWorkflowAttemptRow,
    InvocationWorkflowDetailResponse, InvocationWorkflowHero, InvocationWorkflowResponseBody,
    InvocationWorkflowTimelineEntry, apply_invocation_cost_audits,
    fetch_invocation_attempt_response_body, fetch_invocation_pool_attempts,
    fetch_invocation_record_detail, fetch_invocation_request_body, fetch_invocation_response_body,
    fetch_invocation_response_body_row_by_id, fetch_invocation_workflow_detail,
    hydrate_upstream_account_attempt_workflow_entries, is_abnormal_invocation_failure,
    raw_request_fallback_reason, raw_response_fallback_reason,
    resolve_invocation_cache_write_tokens, resolve_request_body_text_from_row,
    resolve_response_body_text_from_row, truncate_response_preview_text,
};

// Keep parent paths stable when a symbol is consumed only through a sibling glob or test module.
const _: () = {
    let _ = (
        fetch_invocation_attempt_response_body,
        fetch_invocation_pool_attempts,
        fetch_invocation_record_detail,
        fetch_invocation_request_body,
        fetch_invocation_response_body,
        fetch_invocation_response_body_row_by_id,
        fetch_invocation_workflow_detail,
        hydrate_upstream_account_attempt_workflow_entries,
        is_abnormal_invocation_failure,
        raw_request_fallback_reason,
        raw_response_fallback_reason,
        resolve_request_body_text_from_row,
        resolve_response_body_text_from_row,
        truncate_response_preview_text,
        latest_pool_attempt_phase_sql,
        latest_pool_attempt_phase_sql_at_snapshot,
    );
    let _: Option<InvocationResponseBodyRow> = None;
    let _: Option<InvocationWorkflowAttempt> = None;
    let _: Option<InvocationWorkflowDetailResponse> = None;
    let _: Option<InvocationWorkflowHero> = None;
    let _: Option<InvocationWorkflowResponseBody> = None;
    let _ = (
        INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD,
        INVOCATION_COST_AUDIT_REASON_MODEL_MISSING,
        INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING,
        INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED,
        INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN,
        INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING,
        INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING,
        INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH,
        INVOCATION_COST_AUDIT_REASON_USAGE_MISSING,
    );
};

#[cfg(test)]
pub(crate) use invocation_workflow_detail::{
    build_attempt_response_summary, build_invocation_cost_audit, build_invocation_usage_summary,
    build_response_body_response_with_source, build_workflow_attempt_from_row,
    fetch_invocation_attempt_response_body_row, final_real_attempt_row_id,
    last_success_like_attempt_row_id,
};

pub(crate) use summary_projection_lifecycle::{
    SummaryCoverageRecoveryNextTurn, SummaryCoverageRecoverySupervisor,
    hydrate_summary_snapshots_with_deadline, refresh_summary_snapshots,
    spawn_summary_coverage_recovery_maintenance,
};

pub(crate) use summary_recovery::{
    SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES,
    SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES, SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE, SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE,
    SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET,
    SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET, SUMMARY_RECOVERY_FAILED_CODE,
    SummaryCompactedSourceRecoveryState, SummaryDashboardRecoveryEntry,
    SummaryDashboardRecoverySnapshot, dashboard_recovery_response_if_ready,
    summary_source_gap_unavailable_error,
};

#[cfg(test)]
pub(crate) use summary_projection_lifecycle::{
    SummaryLiveTailReconciliationCheckpoint, explain_compacted_source_recovery_key_page_for_test,
    hydrate_summary_snapshots, load_summary_live_tail_reconciliation_checkpoint,
    note_summary_projection_boundary_archive_hydration,
    note_summary_projection_historical_identity_hydration, refresh_summary_snapshots_with_mode,
    refresh_summary_snapshots_with_test_setup_deadline,
    renew_summary_projection_freshness_if_generation_matches,
    renew_summary_projection_freshness_if_live_tail_matches,
    run_compacted_source_recovery_one_page_for_test,
    run_compacted_source_recovery_until_complete_for_test,
    store_summary_live_tail_reconciliation_checkpoint,
    summary_all_time_checkpoint_publication_required,
    summary_coverage_recovery_requires_second_v2_turn, summary_coverage_recovery_scheduler_delay,
};

pub(crate) use invocation_query::{
    InvocationAbnormalResponseBodyPreview, InvocationExceptionAggRow, InvocationExceptionSummary,
    InvocationHistoryOverviewResponse, InvocationListRequest, InvocationModelRerouteFilter,
    InvocationModelTarget, InvocationNetworkAggRow, InvocationNetworkSummary,
    InvocationNewRecordsCountResponse, InvocationRecordDetailResponse, InvocationRecordsFilters,
    InvocationResponseBodyResponse, InvocationSortBy, InvocationSortOrder,
    InvocationSuggestionBucket, InvocationSuggestionItem, InvocationSuggestionsResponse,
    InvocationSummaryAggRow, InvocationSummaryResponse, InvocationTokenSummary, SnapshotConstraint,
    append_invocation_order_clause, apply_invocation_records_filters,
    build_invocation_select_query, build_resolved_invocation_list_request, escape_sql_like,
    invocation_display_status_sql, invocation_model_sql_expr, normalize_query_text,
    resolve_invocation_snapshot_id, resolve_invocation_snapshot_id_on_connection,
};

pub(crate) use invocation_projection_sql::{
    INVOCATION_FAILURE_KIND_SQL, INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    INVOCATION_VALID_CACHE_INPUT_TOKENS_SQL, INVOCATION_VALID_CACHE_WRITE_TOKENS_SQL,
    invocation_resolved_failure_class_sql_for,
};
pub(crate) use invocation_query::build_invocation_filters;
pub(crate) use invocation_timeline::*;
pub(crate) use invocation_timing::{
    final_pool_attempt_first_token_ms_sql, final_pool_invocation_timing_sql,
    final_pool_invocation_timing_sql_at_snapshot, invocation_live_phase_sql,
    invocation_live_phase_sql_with_timing_sql,
    invocation_live_phase_sql_with_timing_sql_at_snapshot, invocation_timing_sql_for_source,
    latest_pool_attempt_phase_sql, latest_pool_attempt_phase_sql_at_snapshot,
};

#[cfg(test)]
#[path = "invocations_and_summary/attempt_response_body_query_tests.rs"]
mod attempt_response_body_query_tests;
#[cfg(test)]
#[path = "invocations_and_summary/dashboard_activity_read_model_tests.rs"]
mod dashboard_activity_read_model_tests;
#[cfg(test)]
#[path = "invocations_and_summary/dashboard_recent_network_window_response_tests.rs"]
mod dashboard_recent_network_window_response_tests;
#[cfg(test)]
#[path = "invocations_and_summary/invocation_cost_audit_tests.rs"]
mod invocation_cost_audit_tests;
#[cfg(test)]
#[path = "invocations_and_summary/request_compression_query_tests.rs"]
mod request_compression_query_tests;

pub(crate) use dashboard_activity_archive_fallback::*;
pub(crate) use dashboard_activity_endpoints::*;
pub(crate) use dashboard_activity_preview::*;
pub(crate) use dashboard_activity_query::*;
pub(crate) use dashboard_activity_response::*;
pub(crate) use dashboard_activity_runtime::*;
pub(crate) use dashboard_activity_runtime_query::*;
pub(crate) use dashboard_activity_snapshot_cache::*;
pub(crate) use dashboard_activity_types::*;
pub(crate) use dashboard_network::*;
pub(crate) use invocation_history::*;
pub(crate) use invocation_runtime_overlay::*;
pub(crate) use summary_projection_archive::*;
pub(crate) use summary_projection_boundary::*;
pub(crate) use summary_projection_build::*;
pub(crate) use summary_projection_core::*;
pub(crate) use summary_projection_coverage::*;
pub(crate) use summary_projection_maintenance::*;
pub(crate) use summary_projection_v2::*;
pub(crate) use summary_topic_and_ranges::*;
