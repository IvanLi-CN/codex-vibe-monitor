pub(crate) async fn cleanup_expired_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<usize> {
    if !dry_run {
        backfill_invocation_archive_expiries(pool, config).await?;
    }
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let owner_facing_node_health_window_cutoff = shanghai_local_cutoff_string(7);
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
    let materialized_pool_upstream_cache_files = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let materialized_pool_upstream_hourly_files = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let long_term_stats_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let long_term_stats_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'codex_invocations'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'codex_invocations'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let long_term_stats_attempt_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'pool_upstream_request_attempts'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'pool_upstream_request_attempts'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };

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
                // A pending deletion from an older process is still subject to the durable
                // Summary Snapshot gate.  Without a matching proof, retaining the manifest is
                // safer than deleting the only authoritative source.
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
                || !materialized_pool_upstream_cache_files.contains(&candidate.file_path)
                || !materialized_pool_upstream_hourly_files.contains(&candidate.file_path))
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && !long_term_stats_attempt_archive_files
                .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
        {
            continue;
        }
        // Only an already-staged deletion has evidence that the source archive was readable
        // when cleanup began. A missing invocation or attempt archive is source loss, even if
        // older replay markers exist, so retain its manifest for long-term reconciliation.
        if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) && archive_file_is_confirmed_missing(&candidate.file_path)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && (!long_term_stats_status
                .as_deref()
                .is_some_and(|status| matches!(status, "ready" | "empty"))
                || !long_term_stats_archive_files
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
            // Source cleanup is allowed only after a normalized Snapshot page and its manifest
            // identity have committed. Legacy archives are picked up by the background
            // Snapshot backfill instead of being retired here.
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| {
                    coverage_end_at >= owner_facing_node_health_window_cutoff.as_str()
                })
                .unwrap_or(true)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
        {
            continue;
        }
        eligible_candidates.push(candidate);
    }

    if dry_run {
        for candidate in &eligible_candidates {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "retention dry-run planned archive batch cleanup"
            );
        }
        return Ok(eligible_candidates.len());
    }

    let mut deleted = 0usize;
    for candidate in eligible_candidates {
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            if finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
            {
                deleted += 1;
            }
            continue;
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
                continue;
            }
        };
        if file_missing {
            // Eligibility and replay gates above already prove this expired manifest can retire.
            // Do not derive a source boundary from metadata when its source file is gone.
            if stage_archive_batch_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
                None,
            )
            .await?
                && finalize_archive_batch_file_deletion(
                    pool,
                    candidate.id,
                    &candidate.dataset,
                    &candidate.file_path,
                    &candidate.sha256,
                )
                .await?
            {
                deleted += 1;
            }
            continue;
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
                    continue;
                }
            }
        } else {
            None
        };
        let staged = stage_archive_batch_deletion(
            pool,
            candidate.id,
            &candidate.dataset,
            &candidate.file_path,
            &candidate.sha256,
            integrity_source_safe_start,
        )
        .await?;
        if staged
            && finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
        {
            deleted += 1;
        }
    }

    Ok(deleted)
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

async fn load_summary_archive_snapshot_backfill_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    // The monotonic ID cursor is a fairness sweep only. Eligibility and 30-day priority are
    // evaluated by the due queue below so a busy archive tail cannot starve older recent data.
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            batches.id,
            batches.dataset,
            batches.file_path,
            batches.sha256,
            batches.row_count,
            COALESCE(batches.summary_source_kind, 'unknown') AS summary_source_kind,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.id > ?2
          AND NOT EXISTS (
                SELECT 1
                FROM summary_archive_snapshot_v2_proof AS proof
                WHERE proof.archive_batch_id = batches.id
                  AND proof.manifest_sha256 = batches.sha256
          )
          AND (
                NOT EXISTS (
                    SELECT 1
                    FROM summary_coverage_obligation AS obligation
                    WHERE obligation.archive_batch_id = batches.id
                      AND obligation.manifest_sha256 = batches.sha256
                )
                OR EXISTS (
                    SELECT 1
                    FROM summary_coverage_obligation AS obligation
                    WHERE obligation.archive_batch_id = batches.id
                      AND obligation.manifest_sha256 = batches.sha256
                      AND obligation.state <> 'resolved'
                )
          )
          AND (
                NOT EXISTS (
                    SELECT 1 FROM summary_archive_snapshot_backfill_outcome AS outcome
                    WHERE outcome.archive_batch_id = batches.id
                      AND outcome.manifest_sha256 = batches.sha256
                )
                OR EXISTS (
                    SELECT 1 FROM summary_archive_snapshot_backfill_outcome AS outcome
                    WHERE outcome.archive_batch_id = batches.id
                      AND outcome.manifest_sha256 = batches.sha256
                      AND (
                          outcome.disposition = 'complete'
                          OR (
                              julianday(outcome.next_probe_at) <= julianday('now')
                              AND NOT (
                                  outcome.disposition = 'unavailable'
                                  AND outcome.failure_kind IN (
                                      'verification_failed', 'manifest_sha_mismatch',
                                      'invalid_timestamp', 'row_count_mismatch', 'empty_archive'
                                  )
                              )
                          )
                      )
                )
          )
        ORDER BY batches.id ASC
        LIMIT ?3
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(cursor_id.max(0))
    .bind(limit.max(1))
    .fetch_all(pool)
    .await
    .context("failed to load Summary Snapshot V2 backfill candidates")
}

/// A prior attempt may have committed every V2 page and then failed before promoting the final
/// proof marker.  Retryable outcomes may still promote that page set, while typed terminal
/// outcomes remain quarantined until a new manifest identity produces a fresh page set.
async fn load_summary_archive_snapshot_v2_promotion_candidates(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<(i64, String)>> {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT snapshot.archive_batch_id, snapshot.manifest_sha256 \
         FROM summary_archive_snapshot AS snapshot \
         LEFT JOIN summary_archive_snapshot_v2_proof AS proof \
           ON proof.archive_batch_id = snapshot.archive_batch_id \
          AND proof.manifest_sha256 = snapshot.manifest_sha256 \
         LEFT JOIN summary_archive_snapshot_backfill_outcome AS outcome \
           ON outcome.archive_batch_id = snapshot.archive_batch_id \
          AND outcome.manifest_sha256 = snapshot.manifest_sha256 \
         WHERE snapshot.format_version = ?1 \
           AND proof.archive_batch_id IS NULL \
           AND (outcome.archive_batch_id IS NULL OR (
               outcome.disposition <> 'complete'
               AND NOT (
                   outcome.disposition = 'unavailable'
                   AND outcome.failure_kind IN (
                       'verification_failed', 'manifest_sha_mismatch',
                       'invalid_timestamp', 'row_count_mismatch', 'empty_archive'
                   )
               )
           )) \
         GROUP BY snapshot.archive_batch_id, snapshot.manifest_sha256 \
         ORDER BY snapshot.archive_batch_id ASC \
         LIMIT ?2",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_V2)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await
    .context("load unpromoted Summary Snapshot V2 page sets")
}

async fn promote_verified_summary_snapshot_page_sets(
    pool: &Pool<Sqlite>,
    limit: i64,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<usize> {
    let candidates = load_summary_archive_snapshot_v2_promotion_candidates(pool, limit).await?;
    let mut promoted = 0;
    for (archive_batch_id, manifest_sha256) in candidates {
        if started_at.elapsed() >= max_elapsed {
            break;
        }
        if ensure_summary_archive_snapshot_v2_final_proof(pool, archive_batch_id, &manifest_sha256)
            .await?
        {
            sqlx::query(
                "INSERT INTO summary_archive_snapshot_backfill_outcome \
                 (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at, \
                  next_page_index, next_row_id, cursor_version, retry_attempt) \
                 VALUES (?1, ?2, 'complete', '', datetime('now'), 0, 0, ?3, 0) \
                 ON CONFLICT(archive_batch_id, manifest_sha256) DO UPDATE SET \
                   disposition = 'complete', failure_kind = '', next_probe_at = datetime('now'), \
                   updated_at = datetime('now')",
            )
            .bind(archive_batch_id)
            .bind(&manifest_sha256)
            .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION)
            .execute(pool)
            .await
            .context("record promoted Summary Snapshot V2 outcome")?;
            promoted += 1;
        }
    }
    if promoted > 0 {
        tracing::info!(
            stage = "summary_snapshot_v2_proof_promoted",
            promoted,
            "promoted verified Summary Snapshot V2 page sets before backfill selection"
        );
    }
    Ok(promoted)
}

async fn load_summary_archive_snapshot_backfill_due_candidates(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            batches.id,
            batches.dataset,
            batches.file_path,
            batches.sha256,
            batches.row_count,
            COALESCE(batches.summary_source_kind, 'unknown') AS summary_source_kind,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND (
                (
                    batches.coverage_end_at IS NOT NULL
                    AND batches.coverage_end_at >= datetime('now', '-30 days')
                    AND (batches.coverage_start_at IS NULL OR batches.coverage_start_at <= datetime('now'))
                    AND NOT EXISTS (
                        SELECT 1
                        FROM summary_archive_snapshot_v2_proof AS proof
                        WHERE proof.archive_batch_id = batches.id
                          AND proof.manifest_sha256 = batches.sha256
                    )
                    AND (
                        NOT EXISTS (
                            SELECT 1 FROM summary_coverage_obligation AS obligation
                            WHERE obligation.archive_batch_id = batches.id
                              AND obligation.manifest_sha256 = batches.sha256
                        )
                        OR EXISTS (
                            SELECT 1 FROM summary_coverage_obligation AS obligation
                            WHERE obligation.archive_batch_id = batches.id
                              AND obligation.manifest_sha256 = batches.sha256
                              AND obligation.state <> 'resolved'
                        )
                    )
                    AND (
                        NOT EXISTS (
                            SELECT 1 FROM summary_archive_snapshot_backfill_outcome AS outcome
                            WHERE outcome.archive_batch_id = batches.id
                              AND outcome.manifest_sha256 = batches.sha256
                        )
                        OR EXISTS (
                            SELECT 1 FROM summary_archive_snapshot_backfill_outcome AS outcome
                            WHERE outcome.archive_batch_id = batches.id
                              AND outcome.manifest_sha256 = batches.sha256
                              AND (
                                  outcome.disposition = 'complete'
                                  OR (
                                      julianday(outcome.next_probe_at) <= julianday('now')
                                      AND NOT (
                                          outcome.disposition = 'unavailable'
                                          AND outcome.failure_kind IN (
                                              'verification_failed', 'manifest_sha_mismatch',
                                              'invalid_timestamp', 'row_count_mismatch', 'empty_archive'
                                          )
                                      )
                                  )
                              )
                        )
                    )
                )
                OR EXISTS (
                    SELECT 1
                    FROM summary_coverage_obligation AS obligation
                    LEFT JOIN summary_archive_snapshot_backfill_outcome AS outcome
                      ON outcome.archive_batch_id = obligation.archive_batch_id
                     AND outcome.manifest_sha256 = obligation.manifest_sha256
                    WHERE obligation.archive_batch_id = batches.id
                      AND obligation.manifest_sha256 = batches.sha256
                      AND obligation.state <> 'resolved'
                      AND (
                          outcome.archive_batch_id IS NULL
                          OR (
                              julianday(outcome.next_probe_at) <= julianday('now')
                              AND outcome.disposition <> 'complete'
                              AND NOT (
                                  outcome.disposition = 'unavailable'
                                  AND outcome.failure_kind IN (
                                      'verification_failed', 'manifest_sha_mismatch',
                                      'invalid_timestamp', 'row_count_mismatch', 'empty_archive'
                                  )
                              )
                          )
                      )
                  )
              )
        ORDER BY CASE
            WHEN batches.coverage_end_at IS NOT NULL
             AND batches.coverage_end_at >= datetime('now', '-30 days')
             AND (batches.coverage_start_at IS NULL OR batches.coverage_start_at <= datetime('now')) THEN 0
            ELSE 1
        END, batches.id ASC
        LIMIT ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await
    .context("failed to load Summary Snapshot V2 backfill due candidates")
}
