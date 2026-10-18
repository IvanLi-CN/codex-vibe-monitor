use super::*;
use sqlx::FromRow;

#[path = "cleanup/compression_backlog.rs"]
mod compression_backlog;
#[path = "cleanup/deletion.rs"]
mod deletion;
#[path = "cleanup/historical_rollups.rs"]
mod historical_rollups;
#[path = "cleanup/legacy_mirrors.rs"]
mod legacy_mirrors;
#[path = "cleanup/rollup_estimate.rs"]
mod rollup_estimate;
#[path = "cleanup/storage.rs"]
mod storage;
#[path = "cleanup/summary_snapshot.rs"]
mod summary_snapshot;
#[cfg(test)]
#[path = "cleanup/tests.rs"]
mod tests;

pub(crate) use compression_backlog::{
    load_raw_compression_backlog_snapshot, log_raw_compression_backlog_if_needed,
};
pub(crate) use deletion::{
    backfill_invocation_archive_expiries, cleanup_expired_archive_batches,
    prune_legacy_archive_batches,
};
pub(crate) use historical_rollups::{
    HistoricalRollupPendingArchiveBatchRow, count_historical_rollup_startup_pending_hint,
    invocation_status_counts_toward_terminal_totals, load_historical_rollup_backfill_snapshot,
    materialize_historical_rollups, materialize_historical_rollups_startup_window,
};
#[cfg(test)]
pub(crate) use historical_rollups::{
    materialize_historical_rollups_bounded, materialize_historical_rollups_bounded_from_skip,
    materialize_usage_breakdown_historical_rollups_bounded_from_skip,
};
pub(crate) use legacy_mirrors::reconcile_legacy_detail_mirrors_startup_window;
#[cfg(test)]
pub(crate) use legacy_mirrors::{
    reconcile_legacy_detail_mirrors_for_summary_startup_window,
    summary_startup_legacy_detail_mirror_high_watermark,
};
pub(crate) use rollup_estimate::estimate_historical_rollup_pending_bucket_count;
pub(crate) use storage::{cleanup_stale_archive_temp_files, verify_archive_storage};
pub(crate) use summary_snapshot::{
    SummaryArchiveSnapshotBackfillWindowResult, backfill_summary_archive_snapshots_v2_window,
};

#[derive(Debug, Clone, FromRow)]
struct HistoricalRollupStartupCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    row_count: i64,
    summary_source_kind: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

impl HistoricalRollupStartupCandidateRow {
    fn archive_file(&self) -> ArchiveBatchFileRow {
        ArchiveBatchFileRow {
            id: self.id,
            file_path: self.file_path.clone(),
            sha256: Some(self.sha256.clone()),
            coverage_start_at: self.coverage_start_at.clone(),
            coverage_end_at: self.coverage_end_at.clone(),
        }
    }
}

pub(crate) async fn prune_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveBatchPruneSummary> {
    let expired_archive_batches_deleted =
        cleanup_expired_archive_batches(pool, config, dry_run).await?;
    let legacy_summary = prune_legacy_archive_batches(pool, config, dry_run).await?;
    Ok(ArchiveBatchPruneSummary {
        expired_archive_batches_deleted,
        legacy_archive_batches_deleted: legacy_summary.deleted_archive_batches,
    })
}
