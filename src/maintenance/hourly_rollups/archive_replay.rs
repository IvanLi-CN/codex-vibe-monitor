use super::live_repair::{
    InvocationArchiveCoverageRow, InvocationArchiveRepairFence,
    LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
    LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING,
    USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS, archive_batch_has_completed_manifest_sha_tx,
    can_shortcut_legacy_materialized_upstream_account_targets,
    clear_invocation_rollup_rows_for_bucket_epochs_tx,
    clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx,
    forward_proxy_archive_has_stale_replay_marker_tx,
    invocation_archive_has_materialized_rollups_tx, invocation_archive_has_stale_replay_marker_tx,
    invocation_archive_has_unverified_replay_marker_tx,
    invocation_archive_is_missing_summary_projection_proof_tx,
    load_completed_invocation_archives_overlapping_usage_breakdown_buckets,
    load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx,
    reopen_replaced_materialized_forward_proxy_archive_tx,
    reopen_replaced_materialized_invocation_archive_tx, reset_invocation_archive_replay_state_tx,
    reset_invocation_archive_usage_breakdown_backfill_state_tx,
    usage_breakdown_repair_bucket_epochs_from_bounds,
};
use super::*;
use sha2::{Digest, Sha256};

pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE: i64 = BACKFILL_BATCH_SIZE;
pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoricalRollupArchiveReplayOutcome {
    Completed,
    HitBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoricalRollupArchiveReplayResult {
    pub(crate) outcome: HistoricalRollupArchiveReplayOutcome,
    pub(crate) cursor_id: i64,
}

pub(crate) fn historical_rollup_replay_made_progress(
    replay: HistoricalRollupArchiveReplayResult,
    initial_cursor_id: i64,
) -> bool {
    replay.cursor_id > initial_cursor_id
}

pub(crate) fn historical_rollup_candidate_changed(
    changed_in_prior_stage: bool,
    replay_progressed: bool,
    coverage_updated: bool,
) -> bool {
    changed_in_prior_stage || replay_progressed || coverage_updated
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HistoricalRollupArchiveReplaySummary {
    pub(crate) scanned_batches: u64,
    pub(crate) skipped_batches: u64,
    pub(crate) remaining_skip_batches: usize,
    pub(crate) budget_consumed_batches: u64,
    pub(crate) blocked_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) changed_batches: u64,
    pub(crate) hit_budget: bool,
    pub(crate) advance_cursor_after_unstarted_replay: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolUpstreamNodeHealthArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) cached_rows: u64,
    pub(crate) pending_batches: u64,
    pub(crate) hit_budget: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolUpstreamNodeHealthHourlyArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) materialized_rows: u64,
    pub(crate) pending_batches: u64,
    pub(crate) hit_budget: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct HistoricalRollupArchiveCoverageBoundsRow {
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) fn historical_rollup_elapsed_budget_reached(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> bool {
    crate::maintenance::migration_hold_signal_reached()
        || max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit)
}

pub(crate) fn historical_rollup_materialization_budget_reached(
    started_at: Instant,
    replayed: u64,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    max_archive_batches.is_some_and(|limit| replayed >= limit)
        || historical_rollup_elapsed_budget_reached(started_at, max_elapsed)
}

pub(crate) async fn inflate_gzip_sqlite_file_with_budget(
    source: &Path,
    destination: &Path,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<bool> {
    inflate_gzip_sqlite_file_with_elapsed_budget(source, destination, started_at, max_elapsed).await
}

pub(crate) async fn open_historical_rollup_archive_pool(
    archive_path: &Path,
    temp_path: &Path,
    expected_sha256: &str,
) -> Result<Pool<Sqlite>> {
    let current_signature =
        match historical_rollup_archive_source_signature(archive_path, expected_sha256) {
            Ok(signature) => signature,
            Err(error) => {
                remove_temp_sqlite_artifacts(temp_path);
                return Err(error);
            }
        };
    let stale_temp = !temp_path.exists()
        || load_historical_rollup_temp_source_signature(temp_path).as_deref()
            != Some(current_signature.as_str());
    if stale_temp {
        remove_temp_sqlite_artifacts(temp_path);
        inflate_gzip_sqlite_file(archive_path, temp_path)?;
        persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
    }

    let connect = || async {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(temp_path))
            .await
    };

    match connect().await {
        Ok(pool) => Ok(pool),
        Err(first_err) => {
            remove_temp_sqlite_artifacts(temp_path);
            inflate_gzip_sqlite_file(archive_path, temp_path)?;
            persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
            connect().await.with_context(|| {
                format!(
                    "failed to reopen archive batch {} after resetting temp db (initial error: {first_err})",
                    archive_path.display()
                )
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationArchiveReadability {
    Readable,
    Rejected,
    BudgetExhausted,
}

#[derive(Debug)]
pub(super) enum InvocationArchiveReopenResult {
    Reopened(Vec<String>),
    Rejected,
    BudgetExhausted,
}

#[derive(Debug, Clone)]
struct InvocationArchiveReopenPlan {
    primary_file_path: String,
    primary_sha256: String,
    file_paths: Vec<String>,
    archive_rows: Vec<InvocationArchiveCoverageRow>,
    bucket_start_epochs: HashSet<i64>,
}

#[derive(Debug, Clone, Copy)]
enum InvocationArchiveReopenMode {
    ReplaceAllRollups,
    UsageBreakdown,
}

#[derive(Debug)]
enum InvocationArchiveReopenPreparation {
    Ready(InvocationArchiveReopenPlan),
    Rejected,
    BudgetExhausted,
}

async fn prepare_invocation_archive_reopen_plan(
    pool: &Pool<Sqlite>,
    file_path: &str,
    expected_sha256: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    include_initial_file: bool,
    fence: &mut InvocationArchiveRepairFence,
) -> Result<InvocationArchiveReopenPreparation> {
    let archive_path = Path::new(file_path);
    if !fence.try_fence(archive_path)? {
        return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
    }
    if historical_rollup_elapsed_budget_reached(fence.started_at, fence.max_elapsed) {
        return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
    }
    match invocation_archive_file_is_readable_with_budget(
        archive_path,
        expected_sha256,
        fence.started_at,
        fence.max_elapsed,
    )
    .await
    {
        InvocationArchiveReadability::Readable => {}
        InvocationArchiveReadability::Rejected => {
            return Ok(InvocationArchiveReopenPreparation::Rejected);
        }
        InvocationArchiveReadability::BudgetExhausted => {
            return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
        }
    }

    let mut bucket_start_epochs = HashSet::new();
    let mut reopened_file_path_set = HashSet::new();
    let mut archive_rows = Vec::new();
    if include_initial_file {
        reopened_file_path_set.insert(file_path.to_string());
        if let (Some(coverage_start_at), Some(coverage_end_at)) =
            (coverage_start_at, coverage_end_at)
        {
            let Some(initial_bucket_start_epochs) =
                usage_breakdown_repair_bucket_epochs_from_bounds(
                    coverage_start_at,
                    coverage_end_at,
                )?
            else {
                return Ok(InvocationArchiveReopenPreparation::Rejected);
            };
            bucket_start_epochs.extend(initial_bucket_start_epochs);
            archive_rows.push(InvocationArchiveCoverageRow {
                file_path: file_path.to_string(),
                sha256: Some(expected_sha256.to_string()),
                coverage_start_at: coverage_start_at.to_string(),
                coverage_end_at: coverage_end_at.to_string(),
            });
        }
    } else {
        let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
        else {
            return Ok(InvocationArchiveReopenPreparation::Rejected);
        };
        let Some(initial_bucket_start_epochs) =
            usage_breakdown_repair_bucket_epochs_from_bounds(coverage_start_at, coverage_end_at)?
        else {
            return Ok(InvocationArchiveReopenPreparation::Rejected);
        };
        bucket_start_epochs.extend(initial_bucket_start_epochs);
    }
    if bucket_start_epochs.len() > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS {
        return Ok(InvocationArchiveReopenPreparation::Rejected);
    }

    loop {
        let overlapping_archives =
            load_completed_invocation_archives_overlapping_usage_breakdown_buckets(
                pool,
                &bucket_start_epochs,
                &reopened_file_path_set,
            )
            .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            let Some(expected_overlap_sha256) = overlapping_archive
                .sha256
                .as_deref()
                .filter(|sha256| !sha256.trim().is_empty())
            else {
                return Ok(InvocationArchiveReopenPreparation::Rejected);
            };
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            if !fence.try_fence(Path::new(&overlapping_archive.file_path))? {
                return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
            }
            if historical_rollup_elapsed_budget_reached(fence.started_at, fence.max_elapsed) {
                return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
            }
            match invocation_archive_file_is_readable_with_budget(
                Path::new(&overlapping_archive.file_path),
                expected_overlap_sha256,
                fence.started_at,
                fence.max_elapsed,
            )
            .await
            {
                InvocationArchiveReadability::Readable => {}
                InvocationArchiveReadability::Rejected => {
                    return Ok(InvocationArchiveReopenPreparation::Rejected);
                }
                InvocationArchiveReadability::BudgetExhausted => {
                    return Ok(InvocationArchiveReopenPreparation::BudgetExhausted);
                }
            }
            let Some(overlap_bucket_start_epochs) =
                usage_breakdown_repair_bucket_epochs_from_bounds(
                    &overlapping_archive.coverage_start_at,
                    &overlapping_archive.coverage_end_at,
                )?
            else {
                return Ok(InvocationArchiveReopenPreparation::Rejected);
            };
            bucket_start_epochs.extend(overlap_bucket_start_epochs);
            if bucket_start_epochs.len() > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS {
                return Ok(InvocationArchiveReopenPreparation::Rejected);
            }
            archive_rows.push(overlapping_archive);
            expanded = true;
        }
        if !expanded {
            break;
        }
    }

    if !reopened_file_path_set.contains(file_path) {
        return Ok(InvocationArchiveReopenPreparation::Rejected);
    }
    let mut file_paths = reopened_file_path_set.into_iter().collect::<Vec<_>>();
    file_paths.sort_unstable();
    archive_rows.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(InvocationArchiveReopenPreparation::Ready(
        InvocationArchiveReopenPlan {
            primary_file_path: file_path.to_string(),
            primary_sha256: expected_sha256.to_string(),
            file_paths,
            archive_rows,
            bucket_start_epochs,
        },
    ))
}

async fn invocation_archive_reopen_plan_is_current(
    tx: &mut SqliteConnection,
    plan: &InvocationArchiveReopenPlan,
) -> Result<bool> {
    let current_primary_sha256 = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sha256
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
          AND status = ?3
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&plan.primary_file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_optional(&mut *tx)
    .await?;
    if current_primary_sha256.as_deref() != Some(plan.primary_sha256.as_str()) {
        return Ok(false);
    }
    for archive_row in &plan.archive_rows {
        let current_row = sqlx::query_as::<_, InvocationArchiveCoverageRow>(
            r#"
            SELECT file_path, sha256, coverage_start_at, coverage_end_at
            FROM archive_batches
            WHERE dataset = ?1
              AND file_path = ?2
              AND status = ?3
            LIMIT 1
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_row.file_path)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .fetch_optional(&mut *tx)
        .await?;
        if current_row.as_ref() != Some(archive_row) {
            return Ok(false);
        }
    }
    if plan.bucket_start_epochs.is_empty() {
        return Ok(true);
    }
    let mut current_rows = Vec::new();
    let mut current_bucket_start_epochs = plan.bucket_start_epochs.clone();
    let mut excluded_file_paths = HashSet::new();
    loop {
        let current_page =
            load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                tx,
                &current_bucket_start_epochs,
                &excluded_file_paths,
            )
            .await?;
        if current_page.is_empty() {
            break;
        }
        for current_row in current_page {
            if !excluded_file_paths.insert(current_row.file_path.clone()) {
                continue;
            }
            let Some(overlap_bucket_start_epochs) =
                usage_breakdown_repair_bucket_epochs_from_bounds(
                    &current_row.coverage_start_at,
                    &current_row.coverage_end_at,
                )?
            else {
                return Ok(false);
            };
            current_bucket_start_epochs.extend(overlap_bucket_start_epochs);
            if current_bucket_start_epochs.len() > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS {
                return Ok(false);
            }
            current_rows.push(current_row);
        }
    }
    current_rows.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(
        current_bucket_start_epochs == plan.bucket_start_epochs
            && current_rows == plan.archive_rows,
    )
}

async fn apply_invocation_archive_reopen_plan(
    pool: &Pool<Sqlite>,
    plan: InvocationArchiveReopenPlan,
    mode: InvocationArchiveReopenMode,
) -> Result<InvocationArchiveReopenResult> {
    let mut tx = pool.begin().await?;
    if !invocation_archive_reopen_plan_is_current(tx.as_mut(), &plan).await? {
        tx.rollback().await?;
        return Ok(InvocationArchiveReopenResult::Rejected);
    }
    match mode {
        InvocationArchiveReopenMode::ReplaceAllRollups => {
            clear_invocation_rollup_rows_for_bucket_epochs_tx(
                tx.as_mut(),
                &plan.bucket_start_epochs,
            )
            .await?;
            reset_invocation_archive_replay_state_tx(tx.as_mut(), &plan.file_paths).await?;
        }
        InvocationArchiveReopenMode::UsageBreakdown => {
            if !plan.bucket_start_epochs.is_empty() {
                clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx(
                    tx.as_mut(),
                    &plan.bucket_start_epochs,
                )
                .await?;
                let mut bucket_start_epochs =
                    plan.bucket_start_epochs.iter().copied().collect::<Vec<_>>();
                bucket_start_epochs.sort_unstable();
                let retained_live_rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(
                    tx.as_mut(),
                    &bucket_start_epochs,
                )
                .await?;
                upsert_invocation_hourly_rollups_tx(
                    tx.as_mut(),
                    &retained_live_rows,
                    &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
                )
                .await?;
            }
            for file_path in &plan.file_paths {
                reset_invocation_archive_usage_breakdown_backfill_state_tx(tx.as_mut(), file_path)
                    .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(InvocationArchiveReopenResult::Reopened(plan.file_paths))
}

pub(super) async fn reopen_replaced_materialized_invocation_archive_without_long_tx(
    pool: &Pool<Sqlite>,
    file_path: &str,
    expected_sha256: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    fence: &mut InvocationArchiveRepairFence,
) -> Result<InvocationArchiveReopenResult> {
    match prepare_invocation_archive_reopen_plan(
        pool,
        file_path,
        expected_sha256,
        coverage_start_at,
        coverage_end_at,
        false,
        fence,
    )
    .await?
    {
        InvocationArchiveReopenPreparation::Ready(plan) => {
            apply_invocation_archive_reopen_plan(
                pool,
                plan,
                InvocationArchiveReopenMode::ReplaceAllRollups,
            )
            .await
        }
        InvocationArchiveReopenPreparation::Rejected => Ok(InvocationArchiveReopenResult::Rejected),
        InvocationArchiveReopenPreparation::BudgetExhausted => {
            Ok(InvocationArchiveReopenResult::BudgetExhausted)
        }
    }
}

pub(super) async fn reopen_materialized_invocation_archive_usage_breakdown_without_long_tx(
    pool: &Pool<Sqlite>,
    file_path: &str,
    expected_sha256: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    fence: &mut InvocationArchiveRepairFence,
) -> Result<InvocationArchiveReopenResult> {
    match prepare_invocation_archive_reopen_plan(
        pool,
        file_path,
        expected_sha256,
        coverage_start_at,
        coverage_end_at,
        true,
        fence,
    )
    .await?
    {
        InvocationArchiveReopenPreparation::Ready(plan) => {
            apply_invocation_archive_reopen_plan(
                pool,
                plan,
                InvocationArchiveReopenMode::UsageBreakdown,
            )
            .await
        }
        InvocationArchiveReopenPreparation::Rejected => Ok(InvocationArchiveReopenResult::Rejected),
        InvocationArchiveReopenPreparation::BudgetExhausted => {
            Ok(InvocationArchiveReopenResult::BudgetExhausted)
        }
    }
}

pub(crate) fn invocation_archive_replay_temp_path(archive_path: &Path) -> PathBuf {
    let digest = Sha256::digest(archive_path.as_os_str().to_string_lossy().as_bytes());
    std::env::temp_dir().join(format!("codex-invocation-archive-check-{digest:x}.sqlite"))
}

pub(crate) fn forward_proxy_archive_replay_temp_path(archive_path: &Path) -> PathBuf {
    let digest = Sha256::digest(archive_path.as_os_str().to_string_lossy().as_bytes());
    std::env::temp_dir().join(format!(
        "codex-forward-proxy-archive-check-{digest:x}.sqlite"
    ))
}

pub(crate) async fn invocation_archive_file_is_readable_with_budget(
    archive_path: &Path,
    expected_sha256: &str,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> InvocationArchiveReadability {
    if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
        return InvocationArchiveReadability::BudgetExhausted;
    }
    let temp_path = invocation_archive_replay_temp_path(archive_path);
    if historical_rollup_archive_source_signature(archive_path, expected_sha256).is_err() {
        remove_temp_sqlite_artifacts(&temp_path);
        return InvocationArchiveReadability::Rejected;
    }
    let mut temp_cleanup = TempSqliteCleanup::with_reusable_copy(
        temp_path.clone(),
        temp_path.is_file()
            && load_historical_rollup_temp_source_signature(&temp_path).as_deref()
                == Some(expected_sha256),
    );
    let archive_pool = match open_historical_rollup_archive_pool_with_budget(
        archive_path,
        &temp_path,
        expected_sha256,
        started_at,
        None,
    )
    .await
    {
        Ok(Some(pool)) => pool,
        Ok(None) => {
            temp_cleanup.keep_reusable_copy();
            return InvocationArchiveReadability::BudgetExhausted;
        }
        Err(_) => {
            return InvocationArchiveReadability::Rejected;
        }
    };
    if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
        archive_pool.close().await;
        temp_cleanup.keep_complete_copy();
        return InvocationArchiveReadability::BudgetExhausted;
    }
    let readable = match load_sqlite_table_columns(&archive_pool, "codex_invocations").await {
        Ok(columns)
            if ["id", "invoke_id", "occurred_at", "raw_response"]
                .iter()
                .all(|column| columns.contains(*column)) =>
        {
            sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id ASC LIMIT 1")
                .fetch_optional(&archive_pool)
                .await
                .is_ok()
        }
        _ => false,
    };
    archive_pool.close().await;
    if !readable {
        return InvocationArchiveReadability::Rejected;
    }
    if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
        temp_cleanup.keep_complete_copy();
        return InvocationArchiveReadability::BudgetExhausted;
    }
    InvocationArchiveReadability::Readable
}

pub(crate) async fn open_historical_rollup_archive_pool_with_budget(
    archive_path: &Path,
    temp_path: &Path,
    expected_sha256: &str,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<Option<Pool<Sqlite>>> {
    #[cfg(test)]
    if HISTORICAL_ROLLUP_TEST_SKIP_ARCHIVE_OPEN
        .try_with(|_| ())
        .is_ok()
    {
        return Ok(None);
    }
    if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
        return Ok(None);
    }
    let current_signature =
        match historical_rollup_archive_source_signature(archive_path, expected_sha256) {
            Ok(signature) => signature,
            Err(error) => {
                remove_temp_sqlite_artifacts(temp_path);
                return Err(error);
            }
        };
    let stale_temp = !temp_path.exists()
        || load_historical_rollup_temp_source_signature(temp_path).as_deref()
            != Some(current_signature.as_str());
    if stale_temp {
        remove_temp_sqlite_artifacts(temp_path);
        // Once an archive has been selected, finish building its durable temp copy. The caller
        // still enforces the elapsed budget before the next archive, while the persisted source
        // signature prevents a later pass from repeating a partial gzip inflate.
        inflate_gzip_sqlite_file_with_budget(archive_path, temp_path, started_at, None).await?;
        persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
    }

    let connect = || async {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(temp_path))
            .await
    };

    match connect().await {
        Ok(pool) => Ok(Some(pool)),
        Err(first_err) => {
            remove_temp_sqlite_artifacts(temp_path);
            inflate_gzip_sqlite_file_with_budget(archive_path, temp_path, started_at, None).await?;
            persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
            connect().await.map(Some).with_context(|| {
                format!(
                    "failed to reopen archive batch {} after resetting temp db (initial error: {first_err})",
                    archive_path.display()
                )
            })
        }
    }
}

#[cfg(test)]
tokio::task_local! {
    // Model the elapsed deadline between candidate selection and opening the archive pool.
    pub(crate) static HISTORICAL_ROLLUP_TEST_SKIP_ARCHIVE_OPEN: ();
}

pub(crate) fn pool_upstream_node_health_archive_temp_path(archive_path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.pool-upstream-node-health.sqlite",
        archive_path.display()
    ))
}

pub(crate) fn historical_rollup_archive_source_signature(
    path: &Path,
    expected_sha256: &str,
) -> Result<String> {
    let actual_sha256 = crate::maintenance::sha256_hex_file_with_deadline(
        path,
        crate::maintenance::retention::retention_archive_file_operation_deadline(),
    )?;
    anyhow::ensure!(
        actual_sha256 == expected_sha256,
        "archive batch {} does not match manifest SHA-256",
        path.display()
    );
    Ok(expected_sha256.to_string())
}

pub(crate) fn load_historical_rollup_temp_source_signature(temp_path: &Path) -> Option<String> {
    fs::read_to_string(temp_sqlite_source_meta_path(temp_path))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn persist_historical_rollup_temp_source_signature(
    temp_path: &Path,
    signature: &str,
) -> Result<()> {
    fs::write(temp_sqlite_source_meta_path(temp_path), signature).with_context(|| {
        format!(
            "failed to persist archive temp source signature for {}",
            temp_path.display()
        )
    })
}

pub(crate) async fn load_pending_pool_upstream_node_health_archive_files(
    pool: &Pool<Sqlite>,
    range_start_at: Option<&str>,
    range_end_at: Option<&str>,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT id, file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'pool_upstream_request_attempts'
          AND batches.status = "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = "#,
    );
    query.push_bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET);
    query.push(
        r#"
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND batches.sha256 IS NOT NULL
                  AND TRIM(batches.sha256) <> ''
                  AND replay.archive_sha256 = batches.sha256
          )
        "#,
    );
    if let (Some(range_start_at), Some(range_end_at)) = (range_start_at, range_end_at) {
        query.push(
            r#"
          AND (
                batches.coverage_start_at IS NULL
                OR batches.coverage_end_at IS NULL
                OR (batches.coverage_end_at >= "#,
        );
        query.push_bind(range_start_at);
        query.push(" AND batches.coverage_start_at < ");
        query.push_bind(range_end_at);
        query.push(")\n          )");
    }
    query.push("\nORDER BY month_key ASC, created_at ASC, id ASC");
    query
        .build_query_as::<ArchiveBatchFileRow>()
        .fetch_all(pool)
        .await
        .context("failed to list pending pool upstream node health archive batches")
}

pub(crate) async fn pending_pool_upstream_node_health_archive_batches(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        load_pending_pool_upstream_node_health_archive_files(pool, None, None)
            .await?
            .len() as u64,
    )
}

pub(crate) async fn load_pending_pool_upstream_node_health_hourly_archive_files(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT id, file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'pool_upstream_request_attempts'
          AND batches.status = "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = "#,
    );
    query.push_bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET);
    query.push(
        r#"
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND batches.sha256 IS NOT NULL
                  AND TRIM(batches.sha256) <> ''
                  AND replay.archive_sha256 = batches.sha256
          )
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    );
    query
        .build_query_as::<ArchiveBatchFileRow>()
        .fetch_all(pool)
        .await
        .context("failed to list pending pool upstream node health hourly archive batches")
}

pub(crate) async fn pending_pool_upstream_node_health_hourly_archive_batches(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        load_pending_pool_upstream_node_health_hourly_archive_files(pool)
            .await?
            .len() as u64,
    )
}

pub(crate) async fn load_invocation_archive_files_missing_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
) -> Result<Vec<ArchiveBatchFileRow>> {
    load_invocation_archive_files_missing_rollup_target_with_limit(executor, target, None).await
}

async fn load_invocation_archive_files_missing_rollup_target_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    limit: Option<i64>,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let limit_clause = limit.map(|_| " LIMIT ?3").unwrap_or_default();
    let query_sql = format!(
        r#"
        SELECT id, file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND replay.archive_sha256 = batches.sha256
          )
        ORDER BY month_key ASC, created_at ASC, id ASC{limit_clause}
        "#,
    );
    let query = sqlx::query_as::<_, ArchiveBatchFileRow>(&query_sql)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(target);
    let archive_files = (if let Some(limit) = limit {
        query.bind(limit).fetch_all(executor)
    } else {
        query.fetch_all(executor)
    })
    .await
    .context("failed to list invocation archive batches missing historical rollup target")?;
    // Keep missing sources in bounded candidate pages. The replay stage records them as blocked,
    // allowing its durable/keyset caller to advance past the page; filtering after LIMIT would
    // hide the scanned prefix and starve later recoverable archives.
    Ok(archive_files)
}

pub(crate) fn legacy_compatible_archive_select_expr(
    archive_columns: &HashSet<String>,
    column_name: &str,
) -> String {
    if archive_columns.contains(column_name) {
        column_name.to_string()
    } else {
        format!("NULL AS {column_name}")
    }
}

pub(crate) fn build_invocation_archive_rows_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    let input_tokens = legacy_compatible_archive_select_expr(archive_columns, "input_tokens");
    let output_tokens = legacy_compatible_archive_select_expr(archive_columns, "output_tokens");
    let cache_input_tokens =
        legacy_compatible_archive_select_expr(archive_columns, "cache_input_tokens");
    let reported_cache_write_tokens =
        legacy_compatible_archive_select_expr(archive_columns, "reported_cache_write_tokens");
    let model = legacy_compatible_archive_select_expr(archive_columns, "model");
    let cost_input = legacy_compatible_archive_select_expr(archive_columns, "cost_input");
    let cost_cache_write =
        legacy_compatible_archive_select_expr(archive_columns, "cost_cache_write");
    let cost_cache_read = legacy_compatible_archive_select_expr(archive_columns, "cost_cache_read");
    let cost_output = legacy_compatible_archive_select_expr(archive_columns, "cost_output");
    let cost_reasoning = legacy_compatible_archive_select_expr(archive_columns, "cost_reasoning");
    let first_token_ms = legacy_compatible_archive_select_expr(archive_columns, "first_token_ms");
    format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            {model},
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            {reported_cache_write_tokens},
            total_tokens,
            cost,
            {cost_input},
            {cost_cache_write},
            {cost_cache_read},
            {cost_output},
            {cost_reasoning},
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            {first_token_ms},
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#
    )
}

/// Summary Snapshot V2 has a chronological proof contract, so its archive pager must use the
/// same deterministic key as the proof validator. Keep the generic historical-rollup pager
/// above ID-ordered because its checkpoint schema and replay semantics are independent.
pub(crate) fn build_invocation_archive_rows_time_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    build_invocation_archive_rows_chunk_query(archive_columns).replace(
        "WHERE id > ?1\n        ORDER BY id ASC\n        LIMIT ?2",
        "WHERE julianday(occurred_at) IS NOT NULL\n          AND (?1 IS NULL\n               OR julianday(occurred_at) > julianday(?1)\n               OR (julianday(occurred_at) = julianday(?1) AND id > ?2))\n        ORDER BY julianday(occurred_at) ASC, id ASC\n        LIMIT ?3",
    )
}

pub(crate) async fn load_invocation_archive_rows_time_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    cursor_occurred_at: Option<&str>,
    cursor_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(cursor_occurred_at)
        .bind(cursor_id.max(0))
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_invocation_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    start_after_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(start_after_id)
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_forward_proxy_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    start_after_id: i64,
) -> Result<(Vec<ForwardProxyAttemptHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(start_after_id)
    .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
    .fetch_all(archive_pool)
    .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_archive_coverage_bounds(
    archive_pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HistoricalRollupArchiveCoverageBoundsRow> {
    Ok(
        sqlx::query_as::<_, HistoricalRollupArchiveCoverageBoundsRow>(&format!(
            r#"
        SELECT
            MIN(occurred_at) AS coverage_start_at,
            MAX(occurred_at) AS coverage_end_at
        FROM {table_name}
        "#
        ))
        .fetch_one(archive_pool)
        .await?,
    )
}

pub(crate) async fn invocation_archive_has_pruned_success_details_in_db(
    archive_pool: &Pool<Sqlite>,
) -> Result<bool> {
    let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
    let query = format!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM codex_invocations
            WHERE detail_level != ?1
              AND {success_like_sql}
              AND COALESCE(NULLIF(LOWER(TRIM(COALESCE(failure_class, ''))), ''), 'none') = 'none'
            LIMIT 1
        )
        "#
    );
    let exists = sqlx::query_scalar::<_, i64>(&query)
        .bind(DETAIL_LEVEL_FULL)
        .fetch_one(archive_pool)
        .await?;
    Ok(exists != 0)
}

pub(crate) async fn replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let archive_columns = load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_chunk_query(&archive_columns);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_invocation_archive_rows_chunk(archive_pool, &query_sql, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

async fn replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    target_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let target_cursor_id = target_cursor_id.max(0);
    if start_after_id >= target_cursor_id {
        return Ok(HistoricalRollupArchiveReplayResult {
            outcome: HistoricalRollupArchiveReplayOutcome::Completed,
            cursor_id: start_after_id,
        });
    }

    let archive_columns = load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_chunk_query(&archive_columns);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_invocation_archive_rows_chunk(archive_pool, &query_sql, start_after_id).await?;
        let bounded_rows = rows
            .into_iter()
            .take_while(|row| row.id <= target_cursor_id)
            .collect::<Vec<_>>();
        if bounded_rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &bounded_rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &bounded_rows).await?;
        start_after_id = bounded_rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if start_after_id >= target_cursor_id || !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_forward_proxy_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
        mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing forward proxy archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_pool_upstream_node_health_archive_rows_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    archive_file_path: &str,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<(HistoricalRollupArchiveReplayResult, u64)> {
    let mut start_after_id = initial_cursor_id.max(0);
    let mut cached_rows = 0_u64;
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        let (rows, has_more) =
            load_pool_upstream_node_health_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        upsert_pool_upstream_node_health_archive_rows_tx(tx, archive_file_path, &rows).await?;
        cached_rows += rows.len() as u64;
        start_after_id = rows
            .last()
            .map(|row| row.archived_row_id)
            .ok_or_else(|| anyhow!("missing pool upstream node health archive row id"))?;

        if !has_more {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }
    }
}

pub(crate) async fn replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
    archive_files: Vec<ArchiveBatchFileRow>,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let mut summary = HistoricalRollupArchiveReplaySummary::default();
    let mut skip_remaining = skip_archive_batches;

    for archive_file in archive_files {
        if skip_remaining > 0 {
            skip_remaining -= 1;
            summary.scanned_batches += 1;
            summary.skipped_batches += 1;
            continue;
        }
        if historical_rollup_materialization_budget_reached(
            started_at,
            summary.budget_consumed_batches,
            max_archive_batches,
            max_elapsed,
        ) {
            summary.hit_budget = true;
            break;
        }
        summary.scanned_batches += 1;
        let archive_path = PathBuf::from(&archive_file.file_path);
        if archive_path.parent().is_none_or(|parent| !parent.exists()) {
            remove_temp_sqlite_artifacts(&invocation_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        let _archive_lock = match retention_try_archive_locks_scope(async {
            retention_archive_file_lock(&archive_path)
        })
        .await
        {
            Ok(lock) => lock,
            Err(error) if error.to_string().contains("archive directory lock busy") => {
                summary.blocked_batches += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if !archive_path.exists()
            || archive_file.sha256.as_deref().is_none_or(|expected| {
                sha256_hex_file_with_deadline(
                    &archive_path,
                    super::super::retention::retention_archive_file_operation_deadline(),
                )
                .ok()
                .as_deref()
                    != Some(expected)
            })
        {
            remove_temp_sqlite_artifacts(&invocation_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        if !archive_batch_has_completed_manifest_sha_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?
        {
            // A completed archive with no immutable manifest identity is intentionally
            // unverified. Do not clear state or additively replay it until it becomes
            // verifiable or a caller can perform a proven full rebuild.
            remove_temp_sqlite_artifacts(&invocation_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        if invocation_archive_has_unverified_replay_marker_tx(tx, &archive_file.file_path).await? {
            // A nullable or blank marker is not evidence that this archive's contributions are
            // represented by the current manifest. Keep it quarantined until an explicit,
            // proven rebuild can replace the unknown state.
            remove_temp_sqlite_artifacts(&invocation_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        let has_stale_replay_marker =
            invocation_archive_has_stale_replay_marker_tx(tx, &archive_file.file_path).await?;
        let materialized_summary_proof_missing =
            invocation_archive_has_materialized_rollups_tx(tx, &archive_file.file_path).await?
                && invocation_archive_is_missing_summary_projection_proof_tx(
                    tx,
                    &archive_file.file_path,
                )
                .await?;
        if has_stale_replay_marker || materialized_summary_proof_missing {
            // A stale marker or missing Summary proof on a materialized archive means a prior
            // contribution may remain. Reset the verified overlap closure before inspecting
            // pending targets so an incremental replay cannot double count old rows.
            let mut archive_fence = InvocationArchiveRepairFence::new(started_at, None);
            archive_fence.register_held_directory(&archive_path);
            let reopened = reopen_replaced_materialized_invocation_archive_tx(
                tx,
                &archive_file.file_path,
                archive_file
                    .sha256
                    .as_deref()
                    .expect("verified invocation archive manifest SHA"),
                archive_file.coverage_start_at.as_deref(),
                archive_file.coverage_end_at.as_deref(),
                &mut archive_fence,
            )
            .await?;
            match reopened {
                InvocationArchiveReopenResult::Reopened(_) => {}
                InvocationArchiveReopenResult::Rejected => {
                    delete_hourly_rollup_archive_progress_tx(
                        tx,
                        HOURLY_ROLLUP_DATASET_INVOCATIONS,
                        &archive_file.file_path,
                    )
                    .await?;
                    summary.blocked_batches += 1;
                    continue;
                }
                InvocationArchiveReopenResult::BudgetExhausted => {
                    summary.hit_budget = true;
                    break;
                }
            };
        }
        let mut pending_targets = Vec::new();
        let mut blocked_targets = Vec::new();
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROXY_PERF,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        ] {
            if !hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?
            {
                pending_targets.push(target);
            }
        }
        let account_activity_v2_pending =
            pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
        pending_targets
            .retain(|target| *target != HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
        if !account_activity_v2_pending
            && can_shortcut_legacy_materialized_upstream_account_targets(&pending_targets)
        {
            let legacy_markers_repaired =
                mark_materialized_upstream_account_archive_replayed_tx(tx, &archive_file.file_path)
                    .await?;
            if legacy_markers_repaired {
                mark_archive_batch_historical_rollups_materialized_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                )
                .await?;
                summary.changed_batches += 1;
                continue;
            }
        }
        if pending_targets.is_empty() && !account_activity_v2_pending {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.changed_batches += 1;
            continue;
        }
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        let usage_breakdown_pending =
            pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN);
        let mut usage_breakdown_cursor = if usage_breakdown_pending {
            load_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?
        } else {
            0
        };

        if !archive_path.exists() {
            remove_temp_sqlite_artifacts(&invocation_archive_replay_temp_path(&archive_path));
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            delete_hourly_rollup_archive_progress_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let temp_path = invocation_archive_replay_temp_path(&archive_path);
        let expected_sha256 = archive_file
            .sha256
            .as_deref()
            .expect("verified invocation archive manifest SHA");
        let mut temp_cleanup = TempSqliteCleanup::with_reusable_copy(
            temp_path.clone(),
            temp_path.is_file()
                && load_historical_rollup_temp_source_signature(&temp_path).as_deref()
                    == Some(expected_sha256),
        );
        let Some(archive_pool) = open_historical_rollup_archive_pool_with_budget(
            &archive_path,
            &temp_path,
            expected_sha256,
            started_at,
            max_elapsed,
        )
        .await?
        else {
            temp_cleanup.keep_reusable_copy();
            summary.hit_budget = true;
            summary.advance_cursor_after_unstarted_replay = true;
            break;
        };
        let has_pruned_success_details =
            invocation_archive_has_pruned_success_details_in_db(&archive_pool).await?;
        let mut changed_in_prior_stage = false;
        if has_pruned_success_details {
            let mut replayable_targets = Vec::with_capacity(pending_targets.len());
            let mut structured_rollup_targets = Vec::new();
            for target in pending_targets {
                if invocation_archive_target_needs_full_payload(target) {
                    blocked_targets.push(target);
                } else {
                    if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
                        structured_rollup_targets.push(target);
                    }
                    replayable_targets.push(target);
                }
            }
            pending_targets = replayable_targets;
            if !structured_rollup_targets.is_empty() {
                tracing::info!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = archive_file.file_path,
                    legacy_pruned_payload_mode =
                        LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING,
                    archive_replay_target = ?structured_rollup_targets,
                    pending_target_count = pending_targets.len(),
                    blocked_target_count = blocked_targets.len(),
                    materialized_batch_count = summary.materialized_batches,
                    "legacy archive batch contains pruned success details; replaying structured hourly rollup targets with unknown reasoning fallback"
                );
            }
        }

        if account_activity_v2_pending {
            let account_activity_v2_cursor = load_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            let replay = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
                tx,
                &archive_pool,
                account_activity_v2_cursor,
                &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
                started_at,
                max_elapsed,
            )
            .await?;
            if replay.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
                let replay_progressed =
                    historical_rollup_replay_made_progress(replay, account_activity_v2_cursor);
                if replay.cursor_id > account_activity_v2_cursor {
                    save_hourly_rollup_archive_progress_tx(
                        tx,
                        INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                        &archive_file.file_path,
                        replay.cursor_id,
                    )
                    .await?;
                }
                archive_pool.close().await;
                summary.budget_consumed_batches += 1;
                summary.hit_budget = true;
                if replay_progressed {
                    summary.changed_batches += 1;
                }
                temp_cleanup.keep_complete_copy();
                break;
            }
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            mark_hourly_rollup_archive_replayed_tx(
                tx,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            changed_in_prior_stage = true;
            tracing::debug!(
                archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                file_path = archive_file.file_path,
                materialized_batch_count = summary.materialized_batches,
                blocked_target_count = blocked_targets.len(),
                pending_target_count = pending_targets.len(),
                "materialized account activity v2 archive rollup"
            );
        }

        if pending_targets.is_empty() {
            archive_pool.close().await;
            drop(temp_cleanup);
            summary.budget_consumed_batches += 1;
            if blocked_targets.is_empty() {
                mark_archive_batch_historical_rollups_materialized_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                )
                .await?;
                summary.materialized_batches += 1;
            } else {
                summary.blocked_batches += 1;
                delete_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                )
                .await?;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = archive_file.file_path,
                    legacy_pruned_payload_mode =
                        LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
                    archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                    pending_target_count = 0_usize,
                    blocked_target_count = blocked_targets.len(),
                    blocked_targets = ?blocked_targets,
                    "legacy archive account activity replay completed; keyed targets remain blocked"
                );
            }
            if account_activity_v2_pending || blocked_targets.is_empty() {
                summary.changed_batches += 1;
            }
            continue;
        }

        let coverage_updated =
            archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none();
        if coverage_updated {
            let bounds = load_archive_coverage_bounds(&archive_pool, "codex_invocations").await?;
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                bounds.coverage_start_at.as_deref(),
                bounds.coverage_end_at.as_deref(),
            )
            .await?;
        }

        if usage_breakdown_pending && usage_breakdown_cursor < replay_cursor {
            let catch_up_outcome =
                replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
                    tx,
                    &archive_pool,
                    usage_breakdown_cursor,
                    replay_cursor,
                    &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
                    started_at,
                    max_elapsed,
                )
                .await?;
            let catch_up_progressed =
                historical_rollup_replay_made_progress(catch_up_outcome, usage_breakdown_cursor);
            if catch_up_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                    &archive_file.file_path,
                    catch_up_outcome.cursor_id,
                )
                .await?;
                usage_breakdown_cursor = catch_up_outcome.cursor_id;
                changed_in_prior_stage = true;
            }
            if catch_up_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget
                && usage_breakdown_cursor < replay_cursor
            {
                archive_pool.close().await;
                summary.budget_consumed_batches += 1;
                summary.hit_budget = true;
                if historical_rollup_candidate_changed(
                    changed_in_prior_stage,
                    false,
                    coverage_updated,
                ) {
                    summary.changed_batches += 1;
                }
                temp_cleanup.keep_complete_copy();
                break;
            }
        }

        let replay_outcome = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
            tx,
            &archive_pool,
            replay_cursor,
            &pending_targets,
            started_at,
            max_elapsed,
        )
        .await?;
        archive_pool.close().await;
        if replay_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
            let replay_progressed =
                historical_rollup_replay_made_progress(replay_outcome, replay_cursor);
            summary.budget_consumed_batches += 1;
            summary.hit_budget = true;
            if replay_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if usage_breakdown_pending && replay_outcome.cursor_id > usage_breakdown_cursor {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if historical_rollup_candidate_changed(
                changed_in_prior_stage,
                replay_progressed,
                coverage_updated,
            ) {
                summary.changed_batches += 1;
            }
            temp_cleanup.keep_complete_copy();
            break;
        }
        drop(temp_cleanup);
        summary.budget_consumed_batches += 1;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        if usage_breakdown_pending {
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
        }
        for target in pending_targets {
            mark_hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
        }
        if blocked_targets.is_empty() {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.materialized_batches += 1;
        } else {
            summary.blocked_batches += 1;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                legacy_pruned_payload_mode = LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
                archive_replay_target = "mixed",
                pending_target_count = 0_usize,
                blocked_target_count = blocked_targets.len(),
                materialized_batch_count = summary.materialized_batches,
                blocked_targets = ?blocked_targets,
                "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
            );
        }
        summary.changed_batches += 1;
    }

    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}

pub(crate) async fn replay_invocation_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let limit = max_archive_batches.map(|limit| {
        limit
            .saturating_add(skip_archive_batches as u64)
            .min(i64::MAX as u64) as i64
    });
    let limit_clause = limit.map(|_| " LIMIT ?2").unwrap_or_default();
    let query_sql = format!(
        r#"
        SELECT id, file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC{limit_clause}
        "#,
    );
    let query = sqlx::query_as::<_, ArchiveBatchFileRow>(&query_sql).bind(ARCHIVE_STATUS_COMPLETED);
    let archive_files = if let Some(limit) = limit {
        query.bind(limit).fetch_all(&mut *tx).await?
    } else {
        query.fetch_all(&mut *tx).await?
    };
    replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
}

pub(crate) async fn replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let limit = max_archive_batches.map(|limit| {
        limit
            .saturating_add(skip_archive_batches as u64)
            .min(i64::MAX as u64) as i64
    });
    let archive_files = load_invocation_archive_files_missing_rollup_target_with_limit(
        &mut *tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        limit,
    )
    .await?;
    replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
}

pub(crate) async fn replay_invocation_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_invocation_archives_into_hourly_rollups_tx_with_limits(tx, Instant::now(), None, None, 0)
        .await
}

pub(crate) async fn replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let limit = max_archive_batches.map(|limit| {
        limit
            .saturating_add(skip_archive_batches as u64)
            .min(i64::MAX as u64) as i64
    });
    let limit_clause = limit.map(|_| " LIMIT ?3").unwrap_or_default();
    let query_sql = format!(
        r#"
        SELECT batches.id, batches.file_path, batches.sha256, batches.coverage_start_at, batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'forward_proxy_attempts'
          AND batches.status = ?1
          AND (
                batches.historical_rollups_materialized_at IS NULL
                OR EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                      AND batches.sha256 IS NOT NULL
                      AND TRIM(batches.sha256) <> ''
                      AND (
                            replay.archive_sha256 IS NULL
                            OR TRIM(replay.archive_sha256) = ''
                            OR replay.archive_sha256 <> batches.sha256
                      )
                )
          )
        ORDER BY month_key ASC, created_at ASC, id ASC{limit_clause}
        "#,
    );
    let query = sqlx::query_as::<_, ArchiveBatchFileRow>(&query_sql)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS);
    let archive_files = if let Some(limit) = limit {
        query.bind(limit).fetch_all(&mut *tx).await?
    } else {
        query.fetch_all(&mut *tx).await?
    };

    replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
}

pub(crate) async fn replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
    archive_files: Vec<ArchiveBatchFileRow>,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let mut summary = HistoricalRollupArchiveReplaySummary::default();
    let mut skip_remaining = skip_archive_batches;

    for archive_file in archive_files {
        if skip_remaining > 0 {
            skip_remaining -= 1;
            summary.scanned_batches += 1;
            summary.skipped_batches += 1;
            continue;
        }
        if historical_rollup_materialization_budget_reached(
            started_at,
            summary.budget_consumed_batches,
            max_archive_batches,
            max_elapsed,
        ) {
            summary.hit_budget = true;
            break;
        }
        summary.scanned_batches += 1;
        let archive_path = PathBuf::from(&archive_file.file_path);
        if archive_path.parent().is_none_or(|parent| !parent.exists()) {
            remove_temp_sqlite_artifacts(&forward_proxy_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        let _archive_lock = match retention_try_archive_locks_scope(async {
            retention_archive_file_lock(&archive_path)
        })
        .await
        {
            Ok(lock) => lock,
            Err(error) if error.to_string().contains("archive directory lock busy") => {
                summary.blocked_batches += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if !archive_path.exists()
            || archive_file.sha256.as_deref().is_none_or(|expected| {
                sha256_hex_file_with_deadline(
                    &archive_path,
                    super::super::retention::retention_archive_file_operation_deadline(),
                )
                .ok()
                .as_deref()
                    != Some(expected)
            })
        {
            remove_temp_sqlite_artifacts(&forward_proxy_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        if !archive_batch_has_completed_manifest_sha_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?
        {
            remove_temp_sqlite_artifacts(&forward_proxy_archive_replay_temp_path(&archive_path));
            summary.blocked_batches += 1;
            continue;
        }
        if forward_proxy_archive_has_stale_replay_marker_tx(tx, &archive_file.file_path).await? {
            let Some(_) = reopen_replaced_materialized_forward_proxy_archive_tx(
                tx,
                &archive_file.file_path,
                archive_file.coverage_start_at.as_deref(),
                archive_file.coverage_end_at.as_deref(),
            )
            .await?
            else {
                delete_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                    &archive_file.file_path,
                )
                .await?;
                summary.blocked_batches += 1;
                continue;
            };
        }
        if hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?
        {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                &archive_file.file_path,
            )
            .await?;
            summary.changed_batches += 1;
            continue;
        }
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;

        if !archive_path.exists() {
            remove_temp_sqlite_artifacts(&forward_proxy_archive_replay_temp_path(&archive_path));
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            delete_hourly_rollup_archive_progress_tx(
                tx,
                HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let temp_path = forward_proxy_archive_replay_temp_path(&archive_path);
        let expected_sha256 = archive_file
            .sha256
            .as_deref()
            .expect("verified forward proxy archive manifest SHA");
        let mut temp_cleanup = TempSqliteCleanup::with_reusable_copy(
            temp_path.clone(),
            temp_path.is_file()
                && load_historical_rollup_temp_source_signature(&temp_path).as_deref()
                    == Some(expected_sha256),
        );
        let Some(archive_pool) = open_historical_rollup_archive_pool_with_budget(
            &archive_path,
            &temp_path,
            expected_sha256,
            started_at,
            max_elapsed,
        )
        .await?
        else {
            temp_cleanup.keep_reusable_copy();
            summary.hit_budget = true;
            summary.advance_cursor_after_unstarted_replay = true;
            break;
        };

        let coverage_updated =
            archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none();
        if coverage_updated {
            let bounds =
                load_archive_coverage_bounds(&archive_pool, "forward_proxy_attempts").await?;
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                bounds.coverage_start_at.as_deref(),
                bounds.coverage_end_at.as_deref(),
            )
            .await?;
        }

        let replay_outcome = replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
            tx,
            &archive_pool,
            replay_cursor,
            started_at,
            max_elapsed,
        )
        .await?;
        archive_pool.close().await;
        if replay_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
            let replay_progressed =
                historical_rollup_replay_made_progress(replay_outcome, replay_cursor);
            summary.budget_consumed_batches += 1;
            summary.hit_budget = true;
            if replay_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if replay_progressed || coverage_updated {
                summary.changed_batches += 1;
            }
            temp_cleanup.keep_complete_copy();
            break;
        }
        drop(temp_cleanup);
        summary.budget_consumed_batches += 1;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        summary.materialized_batches += 1;
        summary.changed_batches += 1;
    }

    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}
