pub(crate) async fn cleanup_expired_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<usize> {
    if !dry_run {
        backfill_invocation_archive_expiries(pool, config).await?;
    }
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let candidates = sqlx::query_as::<_, ArchiveBatchCleanupCandidate>(
        r#"
        SELECT id, dataset, file_path, sha256, cleanup_state, historical_rollups_materialized_at, coverage_end_at
        FROM archive_batches
        WHERE status = ?1
          AND archive_expires_at IS NOT NULL
          AND archive_expires_at < ?2
        ORDER BY archive_expires_at ASC, id ASC
        LIMIT ?3
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&cutoff)
    .bind(super::super::retention::retention_candidate_limit(
        config,
        "archive_cleanup",
    ) as i64)
    .fetch_all(pool)
    .await?;
    let context = load_expired_archive_cleanup_context(pool, config).await?;
    let eligible_candidates =
        select_expired_archive_cleanup_candidates(pool, candidates, &context).await?;
    process_expired_archive_cleanup_candidates(pool, eligible_candidates, dry_run).await
}

struct ExpiredArchiveCleanupContext {
    materialized_pool_upstream_cache_files: HashSet<String>,
    materialized_pool_upstream_hourly_files: HashSet<String>,
    long_term_stats_status: Option<String>,
    long_term_stats_archive_files: HashSet<(String, String)>,
    long_term_stats_attempt_archive_files: HashSet<(String, String)>,
    invocation_archive_cutoff: String,
    owner_facing_node_health_window_cutoff: String,
}

async fn load_expired_archive_cleanup_context(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<ExpiredArchiveCleanupContext> {
    let materialized_pool_upstream_cache_files = load_archive_replay_paths(
        pool,
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        "pool_upstream_request_attempts",
    )
    .await?;
    let materialized_pool_upstream_hourly_files = load_archive_replay_paths(
        pool,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        "pool_upstream_request_attempts",
    )
    .await?;
    let long_term_stats_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let long_term_stats_ready = long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"));
    let long_term_stats_archive_files = if long_term_stats_ready {
        load_long_term_stats_archive_files(pool, "codex_invocations").await?
    } else {
        HashSet::new()
    };
    let long_term_stats_attempt_archive_files = if long_term_stats_ready {
        load_long_term_stats_archive_files(pool, "pool_upstream_request_attempts").await?
    } else {
        HashSet::new()
    };
    Ok(ExpiredArchiveCleanupContext {
        materialized_pool_upstream_cache_files,
        materialized_pool_upstream_hourly_files,
        long_term_stats_status,
        long_term_stats_archive_files,
        long_term_stats_attempt_archive_files,
        invocation_archive_cutoff: shanghai_local_cutoff_string(config.invocation_max_days),
        owner_facing_node_health_window_cutoff: shanghai_local_cutoff_string(7),
    })
}

async fn load_archive_replay_paths(
    pool: &Pool<Sqlite>,
    target: &str,
    dataset: &str,
) -> Result<HashSet<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2",
    )
    .bind(target)
    .bind(dataset)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect())
}

async fn load_long_term_stats_archive_files(
    pool: &Pool<Sqlite>,
    dataset: &str,
) -> Result<HashSet<(String, String)>> {
    Ok(sqlx::query_as::<_, (String, String)>(
        "SELECT replay.file_path, replay.archive_sha256
         FROM hourly_rollup_archive_replay replay
         INNER JOIN archive_batches batches
           ON batches.dataset = ?1
          AND batches.file_path = replay.file_path
          AND batches.sha256 = replay.archive_sha256
         WHERE replay.target = ?2 AND replay.dataset = ?1",
    )
    .bind(dataset)
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect())
}

async fn select_expired_archive_cleanup_candidates(
    pool: &Pool<Sqlite>,
    candidates: Vec<ArchiveBatchCleanupCandidate>,
    context: &ExpiredArchiveCleanupContext,
) -> Result<Vec<ArchiveBatchCleanupCandidate>> {
    let mut eligible_candidates = Vec::new();
    for candidate in candidates {
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
                && !summary_archive_snapshot_cleanup_gate_satisfied(
                    pool,
                    candidate.id,
                    &candidate.sha256,
                )
                .await?
            {
                continue;
            }
            eligible_candidates.push(candidate);
            continue;
        }
        if HISTORICAL_ROLLUP_ARCHIVE_DATASETS.contains(&candidate.dataset.as_str())
            && candidate.historical_rollups_materialized_at.is_none()
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && (candidate.historical_rollups_materialized_at.is_none()
                || !context
                    .materialized_pool_upstream_cache_files
                    .contains(&candidate.file_path)
                || !context
                    .materialized_pool_upstream_hourly_files
                    .contains(&candidate.file_path))
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && !context
                .long_term_stats_attempt_archive_files
                .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
        {
            continue;
        }
        if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) && archive_file_is_confirmed_missing(&candidate.file_path)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && (!context
                .long_term_stats_status
                .as_deref()
                .is_some_and(|status| matches!(status, "ready" | "empty"))
                || !context
                    .long_term_stats_archive_files
                    .contains(&(candidate.file_path.clone(), candidate.sha256.clone())))
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && !summary_archive_snapshot_cleanup_gate_satisfied(
                pool,
                candidate.id,
                &candidate.sha256,
            )
            .await?
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| {
                    coverage_end_at >= context.owner_facing_node_health_window_cutoff.as_str()
                })
                .unwrap_or(true)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| {
                    coverage_end_at >= context.invocation_archive_cutoff.as_str()
                })
                .unwrap_or(true)
        {
            continue;
        }
        eligible_candidates.push(candidate);
    }
    Ok(eligible_candidates)
}

async fn process_expired_archive_cleanup_candidates(
    pool: &Pool<Sqlite>,
    candidates: Vec<ArchiveBatchCleanupCandidate>,
    dry_run: bool,
) -> Result<usize> {
    if dry_run {
        for candidate in &candidates {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "retention dry-run planned archive batch cleanup"
            );
        }
        return Ok(candidates.len());
    }
    let mut deleted = 0usize;
    for candidate in candidates {
        if process_expired_archive_cleanup_candidate(pool, &candidate).await? {
            deleted += 1;
        }
    }
    Ok(deleted)
}

async fn process_expired_archive_cleanup_candidate(
    pool: &Pool<Sqlite>,
    candidate: &ArchiveBatchCleanupCandidate,
) -> Result<bool> {
    if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
        return finalize_archive_batch_file_deletion(
            pool,
            candidate.id,
            &candidate.dataset,
            &candidate.file_path,
            &candidate.sha256,
        )
        .await;
    }
    let file_missing = match fs::metadata(&candidate.file_path) {
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(error) => {
            warn!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                error = %error,
                "could not inspect expired archive file; retaining metadata for a later retry"
            );
            return Ok(false);
        }
    };
    if file_missing {
        return stage_and_finalize_expired_archive_cleanup_candidate(pool, candidate, None).await;
    }
    let integrity_source_safe_start = if matches!(
        candidate.dataset.as_str(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
    ) {
        match crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            pool,
            &candidate.dataset,
            &candidate.file_path,
            candidate.coverage_end_at.as_deref(),
        )
        .await
        {
            Ok(source_safe_start) => source_safe_start,
            Err(error) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %error,
                    "could not prove long-term source boundary; retaining expired archive batch"
                );
                return Ok(false);
            }
        }
    } else {
        None
    };
    stage_and_finalize_expired_archive_cleanup_candidate(
        pool,
        candidate,
        integrity_source_safe_start,
    )
    .await
}

async fn stage_and_finalize_expired_archive_cleanup_candidate(
    pool: &Pool<Sqlite>,
    candidate: &ArchiveBatchCleanupCandidate,
    source_safe_start: Option<NaiveDate>,
) -> Result<bool> {
    let staged = stage_archive_batch_deletion(
        pool,
        candidate.id,
        &candidate.dataset,
        &candidate.file_path,
        &candidate.sha256,
        source_safe_start,
    )
    .await?;
    if !staged {
        return Ok(false);
    }
    finalize_archive_batch_file_deletion(
        pool,
        candidate.id,
        &candidate.dataset,
        &candidate.file_path,
        &candidate.sha256,
    )
    .await
}

async fn summary_archive_snapshot_cleanup_gate_satisfied(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let v2_page_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND format_version = 2)",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_one(pool)
    .await?
        != 0;
    if !v2_page_exists {
        // A legacy archive without even an intermediate V2 page is still the only raw
        // authority.  Do not let retention cleanup race the recovery supervisor; raw deletion
        // is admitted only after a final V2 proof exists.
        return Ok(false);
    }
    summary_archive_snapshot_has_final_proof(pool, archive_batch_id, manifest_sha256).await
}

#[derive(Debug, FromRow)]
pub(crate) struct HistoricalRollupPendingArchiveBatchRow {
    dataset: String,
    month_key: String,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

const STARTUP_HISTORICAL_ROLLUP_CANDIDATE_LIMIT: i64 = 32;
const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: usize = 16;
const LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT: i64 = 128;
const LEGACY_DETAIL_MIRROR_IDENTITY_PAGE_SIZE: i64 = 400;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT: i64 = 512;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_PROOF_CONCURRENCY: usize = 4;

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
            coverage_start_at: self.coverage_start_at.clone(),
            coverage_end_at: self.coverage_end_at.clone(),
        }
    }
}

const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE: &str = "summary-global";
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT: i64 = 64;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_RETRY_SECS: i64 = 3_600;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION: i64 = 2;
const SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION: i64 = 1;
const SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM: &str = "sha256-0.11";
const SUMMARY_ARCHIVE_SNAPSHOT_HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryArchiveSnapshotBackfillDisposition {
    Complete,
    InProgress,
    Deferred,
    TransientFailure,
    Unrecoverable,
}

impl SummaryArchiveSnapshotBackfillDisposition {
    fn from_storage(disposition: &str, failure_kind: &str) -> Self {
        match disposition {
            "complete" => Self::Complete,
            "in_progress" => Self::InProgress,
            // A bounded page has already committed its cursor or hash state. It must be
            // eligible on the next supervisor turn, not delayed like a source failure.
            "deferred" if failure_kind == "budget" => Self::InProgress,
            "deferred" => Self::Deferred,
            "unavailable"
                if matches!(
                    failure_kind,
                    "verification_failed"
                        | "manifest_sha_mismatch"
                        | "invalid_timestamp"
                        | "row_count_mismatch"
                        | "empty_archive"
                ) =>
            {
                Self::Unrecoverable
            }
            "unavailable" => Self::TransientFailure,
            _ => Self::TransientFailure,
        }
    }

    fn next_probe_at(self, retry_attempt: i64) -> String {
        match self {
            Self::Complete | Self::Unrecoverable => {
                format_utc_iso(Utc::now() + ChronoDuration::days(365))
            }
            Self::InProgress => format_utc_iso(Utc::now()),
            Self::Deferred | Self::TransientFailure => {
                let exponent = u32::try_from(retry_attempt.clamp(0, 5)).unwrap_or(5);
                let delay_secs = SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_RETRY_SECS
                    .saturating_mul(1_i64 << exponent)
                    .min(24 * 3_600);
                format_utc_iso(Utc::now() + ChronoDuration::seconds(delay_secs))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SummaryArchiveSnapshotBackfillProgress {
    page_index: u32,
    next_occurred_at: Option<String>,
    next_row_id: i64,
    cursor_version: i64,
    retry_attempt: i64,
    hash_algorithm: Option<String>,
    hash_state_version: i64,
    hash_byte_offset: u64,
    hash_state: Option<Vec<u8>>,
    hash_complete_sha256: Option<String>,
    source_fingerprint: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SummaryArchiveSnapshotBackfillWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) recent_candidate_count: usize,
    pub(crate) scanned_archive_batches: usize,
    pub(crate) materialized_archive_batches: usize,
    pub(crate) unavailable_archive_batches: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
    pub(crate) pending_obligation_count: usize,
    pub(crate) terminal_gap_count: usize,
    pub(crate) verified_proof_count: usize,
}

fn summary_archive_snapshot_candidate_intersects_current_30d(
    candidate: &HistoricalRollupStartupCandidateRow,
    now: DateTime<Utc>,
) -> bool {
    let Some(coverage_end) = candidate
        .coverage_end_at
        .as_deref()
        .and_then(crate::stats::parse_to_utc_datetime)
    else {
        return false;
    };
    if coverage_end < now - ChronoDuration::days(30) {
        return false;
    }
    candidate
        .coverage_start_at
        .as_deref()
        .and_then(crate::stats::parse_to_utc_datetime)
        .is_none_or(|coverage_start| coverage_start <= now)
}

async fn load_summary_archive_snapshot_backfill_checkpoint(
    pool: &Pool<Sqlite>,
) -> Result<(i64, i64, bool)> {
    Ok(sqlx::query_as::<_, (i64, i64, bool)>(
        "SELECT next_archive_batch_id, manifest_high_watermark_id, completed \
         FROM summary_archive_snapshot_backfill_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
    .fetch_optional(pool)
    .await?
    .unwrap_or((0, 0, false)))
}

async fn store_summary_archive_snapshot_backfill_checkpoint(
    pool: &Pool<Sqlite>,
    next_archive_batch_id: i64,
    manifest_high_watermark_id: i64,
    completed: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_checkpoint \
         (scope, next_archive_batch_id, manifest_high_watermark_id, completed, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
           next_archive_batch_id = excluded.next_archive_batch_id, \
           manifest_high_watermark_id = excluded.manifest_high_watermark_id, \
           completed = excluded.completed, updated_at = excluded.updated_at",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
    .bind(next_archive_batch_id.max(0))
    .bind(manifest_high_watermark_id.max(0))
    .bind(i64::from(completed))
    .execute(pool)
    .await?;
    Ok(())
}
