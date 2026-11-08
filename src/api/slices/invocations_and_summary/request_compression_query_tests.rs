use super::*;
use crate::tests::{
    SeedInvocationArchiveBatchRow, assert_f64_close, seed_invocation_archive_batch,
    seed_invocation_archive_batch_with_details,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn hydrate_summary_projection_fixture(state: &Arc<AppState>) {
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-projection-fixture', datetime('now'), 'proxy', 'success', 17, 1.25, '{\"upstreamAccountId\":42}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert summary projection fixture");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection fixture");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("hydrate summary projection fixture all-time coverage");
}

#[path = "request_compression_query_tests/part_1.rs"]
mod part_1;
#[path = "request_compression_query_tests/part_2.rs"]
mod part_2;
#[path = "request_compression_query_tests/part_3.rs"]
mod part_3;
