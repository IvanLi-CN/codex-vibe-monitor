#[allow(unused_imports)]
use super::*;

pub(crate) use super::*;

pub(crate) async fn insert_summary_archive_snapshot_proof(
    pool: &SqlitePool,
    archive_batch_id: i64,
    manifest_sha256: &str,
    coverage_start: &str,
    coverage_end: &str,
    row_count: u32,
) {
    let normalized = serde_json::to_vec(
        &(0..row_count)
            .map(|offset| SummaryArchiveSnapshotV2Record {
                id: i64::from(offset),
                invoke_id: format!("snapshot-proof-{offset}"),
                occurred_at: coverage_start.to_string(),
                source: "proxy".to_string(),
                model: None,
                response_model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_input_tokens: 0,
                reasoning_tokens: 0,
                reasoning_effort: None,
                total_tokens: 0,
                cost: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                status: "success".to_string(),
                error_message: None,
                failure_kind: None,
                failure_class: None,
                is_actionable: false,
                upstream_account_id: None,
            })
            .collect::<Vec<_>>(),
    )
    .expect("serialize Summary Snapshot proof records");
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id,
        manifest_sha256: manifest_sha256.to_string(),
        page_index: 0,
        coverage_start: coverage_start.to_string(),
        coverage_end: coverage_end.to_string(),
        row_count,
        payload: zstd::stream::encode_all(normalized.as_slice(), 1)
            .expect("compress Summary Snapshot proof records"),
    };
    let mut tx = pool
        .begin()
        .await
        .expect("begin Summary Snapshot proof transaction");
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
        .await
        .expect("store Summary Snapshot proof");
    tx.commit().await.expect("commit Summary Snapshot proof");
    ensure_summary_archive_snapshot_v2_final_proof(pool, archive_batch_id, manifest_sha256)
        .await
        .expect("commit Summary Snapshot final proof");
}

/// Hydrates the production Summary projection before exercising the memory-only handler.
///
/// `all` requires its separately reconciled exact coverage; rolling selections intentionally use
/// only Bootstrap so their tests retain the startup availability contract.
pub(crate) async fn fetch_summary_from_memory_snapshot(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    hydrate_summary_snapshots(state.as_ref())
        .await
        .map_err(ApiError::from)?;
    if matches!(params.window.as_deref(), Some("all" | "30d")) {
        // Historical authority is produced by the maintenance supervisor, never by the
        // request-side memory-only handler. Test fixtures that exercise `all` must drive the
        // same background owner before asserting the published response.
        SummaryCoverageRecoverySupervisor::run(state.as_ref())
            .await
            .map_err(ApiError::from)?;
        if params.window.as_deref() == Some("all") {
            refresh_summary_snapshots_with_mode(
                state.as_ref(),
                SummaryProjectionBuildMode::AllTime,
            )
            .await
            .map_err(ApiError::from)?;
        }
    }

    fetch_summary(State(state), Query(params)).await
}

mod invocation_query_filters_and_schema_migrations;
mod oauth_route_body_rewrite_and_timeout;
#[expect(
    clippy::type_complexity,
    reason = "Test fixture tuples mirror statistics row shapes."
)]
mod parallel_work_stats_and_timeseries;
mod pricing_catalog_and_models_passthrough;
#[expect(
    clippy::too_many_arguments,
    reason = "Test insertion helpers mirror persisted prompt-cache fields."
)]
mod prompt_cache_conversation_queries;
#[expect(
    clippy::too_many_arguments,
    reason = "Test insertion helpers mirror persisted rollup fields."
)]
mod proxy_backfill_and_cost_repairs;
mod proxy_broadcast_and_runtime_harness;
mod proxy_pool_roundtrip_and_retry_servers;
mod record_budget;
mod representative_scale_acceptance;
mod request_preparation_and_handshake_failures;
#[expect(
    clippy::await_holding_lock,
    reason = "Mock upstream attempt logs intentionally stay locked until async assertions observe requests."
)]
mod routing_failover_retry_budget;
#[expect(
    clippy::await_holding_lock,
    reason = "Mock upstream attempt logs intentionally stay locked until async assertions observe requests."
)]
mod routing_failover_terminal_reasoning;
#[expect(
    clippy::await_holding_lock,
    reason = "Mock upstream attempt logs intentionally stay locked until async assertions observe requests."
)]
mod routing_timeout_and_overload_failover;
mod runtime_overlay_and_group_rule_behaviors;
mod startup_rebuild_and_retention_basics;
mod system_status_and_account_roster;

pub(crate) use parallel_work_stats_and_timeseries::*;
pub(crate) use proxy_backfill_and_cost_repairs::*;
pub(crate) use proxy_pool_roundtrip_and_retry_servers::*;
pub(crate) use request_preparation_and_handshake_failures::*;
pub(crate) use routing_failover_terminal_reasoning::*;
pub(crate) use runtime_overlay_and_group_rule_behaviors::*;
pub(crate) use system_status_and_account_roster::*;
