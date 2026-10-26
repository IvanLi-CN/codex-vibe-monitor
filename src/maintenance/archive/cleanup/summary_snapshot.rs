use super::*;
use sha2_resumable::{
    Sha256 as ResumableSha256,
    digest::{
        Digest as ResumableDigest,
        common::hazmat::{SerializableState, SerializedState},
    },
};
use std::future::Future;

pub(super) const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE: &str = "summary-global";
const SUMMARY_ARCHIVE_SNAPSHOT_DUE_SCOPE: &str = "summary-global:due";
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT: i64 = 64;
const SUMMARY_ARCHIVE_SNAPSHOT_IDENTITY_WINDOW: i64 = 256;
const SUMMARY_ARCHIVE_SNAPSHOT_MAX_IDENTITY_WINDOWS: usize = 4;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_RETRY_SECS: i64 = 3_600;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION: i64 = 2;
pub(super) const SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION: i64 = 1;
pub(super) const SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM: &str = "sha256-0.11";
const SUMMARY_ARCHIVE_SNAPSHOT_HASH_BUFFER_BYTES: usize = 64 * 1024;
const SUMMARY_ARCHIVE_SNAPSHOT_SWEEP_ELIGIBILITY_SQL: &str = r#"
    NOT EXISTS (
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
            SELECT 1
            FROM summary_archive_snapshot_backfill_outcome AS outcome
            WHERE outcome.archive_batch_id = batches.id
              AND outcome.manifest_sha256 = batches.sha256
        )
        OR EXISTS (
            SELECT 1
            FROM summary_archive_snapshot_backfill_outcome AS outcome
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
"#;
const SUMMARY_ARCHIVE_SNAPSHOT_DUE_ELIGIBILITY_SQL: &str = r#"
    (
        (
            batches.coverage_end_at IS NOT NULL
            AND batches.coverage_end_at >= datetime('now', '-30 days')
            AND (batches.coverage_start_at IS NULL OR batches.coverage_start_at <= datetime('now'))
            AND
"#;
const SUMMARY_ARCHIVE_SNAPSHOT_DUE_ELIGIBILITY_SUFFIX_SQL: &str = r#"
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
"#;
const SUMMARY_ARCHIVE_SNAPSHOT_PROMOTION_ELIGIBILITY_SQL: &str = r#"
    EXISTS (
        SELECT 1
        FROM summary_archive_snapshot AS snapshot
        WHERE snapshot.archive_batch_id = batches.id
          AND snapshot.manifest_sha256 = batches.sha256
          AND snapshot.format_version = 2
    )
    AND NOT EXISTS (
        SELECT 1
        FROM summary_archive_snapshot_v2_proof AS proof
        WHERE proof.archive_batch_id = batches.id
          AND proof.manifest_sha256 = batches.sha256
    )
    AND NOT EXISTS (
        SELECT 1
        FROM summary_archive_snapshot_backfill_outcome AS outcome
        WHERE outcome.archive_batch_id = batches.id
          AND outcome.manifest_sha256 = batches.sha256
          AND (
              outcome.disposition = 'complete'
              OR (
                  outcome.disposition = 'unavailable'
                  AND outcome.failure_kind IN (
                      'verification_failed', 'manifest_sha_mismatch',
                      'invalid_timestamp', 'row_count_mismatch', 'empty_archive'
                  )
              )
          )
    )
"#;
// Phase one is deliberately only a fixed primary-key window. Proof, outcome and obligation
// predicates run in phase two against those bounded identities, so a sparse eligible set cannot
// turn LIMIT into an unbounded archive scan.
pub(super) const SUMMARY_ARCHIVE_SNAPSHOT_V2_PROMOTION_IDENTITY_SQL: &str = r#"
    SELECT batches.id, batches.sha256
    FROM archive_batches AS batches INDEXED BY idx_archive_batches_summary_rollup_repair_seek
    WHERE batches.dataset = 'codex_invocations'
      AND batches.status = 'completed'
      AND batches.sha256 IS NOT NULL
      AND TRIM(batches.sha256) <> ''
      AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
      AND batches.id > ?1
    ORDER BY batches.id ASC
    LIMIT ?2
    "#;

#[derive(Debug, Clone, FromRow)]
struct SummaryArchiveSnapshotCandidateIdentity {
    id: i64,
    sha256: String,
}

async fn load_summary_archive_snapshot_candidate_identities(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    limit: i64,
) -> Result<Vec<SummaryArchiveSnapshotCandidateIdentity>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT batches.id, batches.sha256 FROM archive_batches AS batches INDEXED BY idx_archive_batches_summary_rollup_repair_seek WHERE batches.dataset = 'codex_invocations' AND batches.status = 'completed'",
    );
    query
        .push(" AND batches.sha256 IS NOT NULL AND TRIM(batches.sha256) <> ''")
        .push(" AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'")
        .push(" AND batches.id > ")
        .push_bind(cursor_id.max(0));
    query.push(" ORDER BY batches.id ASC LIMIT ");
    query.push_bind(limit.max(1));
    crate::observability::contention::observe_query(
        crate::observability::contention::StatsQuery::SummarySnapshotCandidates,
        query
            .build_query_as::<SummaryArchiveSnapshotCandidateIdentity>()
            .fetch_all(pool),
    )
    .await
    .context("load bounded Summary Snapshot archive identities")
}

pub(crate) fn summary_archive_snapshot_backfill_due_identity_sql() -> String {
    SUMMARY_ARCHIVE_SNAPSHOT_V2_PROMOTION_IDENTITY_SQL.to_string()
}

async fn load_summary_archive_snapshot_backfill_candidate_rows(
    pool: &Pool<Sqlite>,
    identities: &[SummaryArchiveSnapshotCandidateIdentity],
    limit: i64,
    due_queue: bool,
    promotion: bool,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    if identities.is_empty() {
        return Ok(Vec::new());
    }

    crate::observability::contention::observe_query(
        crate::observability::contention::StatsQuery::SummarySnapshotCandidates,
        build_summary_archive_snapshot_backfill_candidate_query(
            identities, limit, due_queue, promotion, false,
        )
        .build_query_as::<HistoricalRollupStartupCandidateRow>()
        .fetch_all(pool),
    )
    .await
    .context("load bounded Summary Snapshot candidate facts")
}

fn build_summary_archive_snapshot_backfill_candidate_query<'args>(
    identities: &'args [SummaryArchiveSnapshotCandidateIdentity],
    limit: i64,
    due_queue: bool,
    promotion: bool,
    explain: bool,
) -> QueryBuilder<'args, Sqlite> {
    let mut query = if explain {
        QueryBuilder::<Sqlite>::new(
            "EXPLAIN QUERY PLAN WITH candidate(id, manifest_sha256) AS (VALUES ",
        )
    } else {
        QueryBuilder::<Sqlite>::new("WITH candidate(id, manifest_sha256) AS (VALUES ")
    };
    for (index, identity) in identities.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query
            .push("(")
            .push_bind(identity.id)
            .push(", ")
            .push_bind(&identity.sha256)
            .push(")");
    }
    query.push(") ");
    query.push(
        "SELECT batches.id, batches.dataset, batches.file_path, batches.sha256, batches.row_count, COALESCE(batches.summary_source_kind, 'unknown') AS summary_source_kind, batches.coverage_start_at, batches.coverage_end_at FROM candidate JOIN archive_batches AS batches ON batches.id = candidate.id AND batches.sha256 = candidate.manifest_sha256 WHERE ",
    );
    if promotion {
        query.push(SUMMARY_ARCHIVE_SNAPSHOT_PROMOTION_ELIGIBILITY_SQL);
    } else if due_queue {
        query
            .push(SUMMARY_ARCHIVE_SNAPSHOT_DUE_ELIGIBILITY_SQL)
            .push(SUMMARY_ARCHIVE_SNAPSHOT_SWEEP_ELIGIBILITY_SQL)
            .push(SUMMARY_ARCHIVE_SNAPSHOT_DUE_ELIGIBILITY_SUFFIX_SQL);
    } else {
        query.push(SUMMARY_ARCHIVE_SNAPSHOT_SWEEP_ELIGIBILITY_SQL);
    }
    if due_queue {
        query.push(" ORDER BY CASE WHEN batches.coverage_end_at IS NOT NULL AND batches.coverage_end_at >= datetime('now', '-30 days') AND (batches.coverage_start_at IS NULL OR batches.coverage_start_at <= datetime('now')) THEN 0 ELSE 1 END, batches.id ASC LIMIT ");
    } else {
        query.push(" ORDER BY batches.id ASC LIMIT ");
    }
    query.push_bind(limit.max(1));
    query
}

#[cfg(test)]
pub(super) async fn explain_summary_archive_snapshot_backfill_candidate_query(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
    limit: i64,
    due_queue: bool,
    promotion: bool,
) -> Result<Vec<(i64, i64, i64, String)>> {
    let identity = SummaryArchiveSnapshotCandidateIdentity {
        id: archive_batch_id,
        sha256: manifest_sha256.to_owned(),
    };
    build_summary_archive_snapshot_backfill_candidate_query(
        std::slice::from_ref(&identity),
        limit,
        due_queue,
        promotion,
        true,
    )
    .build_query_as::<(i64, i64, i64, String)>()
    .fetch_all(pool)
    .await
    .context("explain bounded Summary Snapshot candidate facts")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SummaryArchiveSnapshotBackfillDisposition {
    Complete,
    InProgress,
    Deferred,
    TransientFailure,
    Unrecoverable,
}

impl SummaryArchiveSnapshotBackfillDisposition {
    pub(super) fn from_storage(disposition: &str, failure_kind: &str) -> Self {
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
pub(super) struct SummaryArchiveSnapshotBackfillProgress {
    pub(super) page_index: u32,
    pub(super) next_occurred_at: Option<String>,
    pub(super) next_row_id: i64,
    pub(super) cursor_version: i64,
    pub(super) retry_attempt: i64,
    pub(super) hash_algorithm: Option<String>,
    pub(super) hash_state_version: i64,
    pub(super) hash_byte_offset: u64,
    pub(super) hash_state: Option<Vec<u8>>,
    pub(super) hash_complete_sha256: Option<String>,
    pub(super) source_fingerprint: Option<String>,
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
    load_summary_archive_snapshot_backfill_checkpoint_for_scope(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE,
    )
    .await
}

async fn load_summary_archive_snapshot_backfill_checkpoint_for_scope(
    pool: &Pool<Sqlite>,
    scope: &str,
) -> Result<(i64, i64, bool)> {
    Ok(sqlx::query_as::<_, (i64, i64, bool)>(
        "SELECT next_archive_batch_id, manifest_high_watermark_id, completed \
         FROM summary_archive_snapshot_backfill_checkpoint WHERE scope = ?1",
    )
    .bind(scope)
    .fetch_optional(pool)
    .await?
    .unwrap_or((0, 0, false)))
}

pub(super) async fn store_summary_archive_snapshot_backfill_checkpoint(
    pool: &Pool<Sqlite>,
    next_archive_batch_id: i64,
    manifest_high_watermark_id: i64,
    completed: bool,
) -> Result<()> {
    store_summary_archive_snapshot_backfill_checkpoint_for_scope(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE,
        next_archive_batch_id,
        manifest_high_watermark_id,
        completed,
    )
    .await
}

async fn store_summary_archive_snapshot_backfill_checkpoint_for_scope(
    pool: &Pool<Sqlite>,
    scope: &str,
    next_archive_batch_id: i64,
    manifest_high_watermark_id: i64,
    completed: bool,
) -> Result<()> {
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("summary_snapshot_checkpoint")
            .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "summary_snapshot_checkpoint",
        ));
    };
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_checkpoint \
         (scope, next_archive_batch_id, manifest_high_watermark_id, completed, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
           next_archive_batch_id = excluded.next_archive_batch_id, \
           manifest_high_watermark_id = excluded.manifest_high_watermark_id, \
           completed = excluded.completed, updated_at = excluded.updated_at",
    )
    .bind(scope)
    .bind(next_archive_batch_id.max(0))
    .bind(manifest_high_watermark_id.max(0))
    .bind(i64::from(completed))
    .execute(pool)
    .await?;
    drop(admission);
    Ok(())
}

async fn load_summary_archive_snapshot_due_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(load_summary_archive_snapshot_backfill_checkpoint_for_scope(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_DUE_SCOPE,
    )
    .await?
    .0
    .max(0))
}

async fn store_summary_archive_snapshot_due_cursor(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
) -> Result<()> {
    store_summary_archive_snapshot_backfill_checkpoint_for_scope(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_DUE_SCOPE,
        cursor_id,
        0,
        false,
    )
    .await
}

pub(super) async fn load_summary_archive_snapshot_backfill_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    Ok(
        load_summary_archive_snapshot_backfill_candidates_with_cursor(pool, cursor_id, limit)
            .await?
            .0,
    )
}

async fn load_summary_archive_snapshot_backfill_candidates_with_cursor(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    limit: i64,
) -> Result<(Vec<HistoricalRollupStartupCandidateRow>, Option<i64>)> {
    let identities =
        load_summary_archive_snapshot_candidate_identities(pool, cursor_id, limit).await?;
    let next_cursor_id = identities.last().map(|identity| identity.id);
    let candidates = load_summary_archive_snapshot_backfill_candidate_rows(
        pool,
        &identities,
        limit,
        false,
        false,
    )
    .await?;
    Ok((candidates, next_cursor_id))
}

/// A prior attempt may have committed every V2 page and then failed before promoting the final
/// proof marker.  Retryable outcomes may still promote that page set, while typed terminal
/// outcomes remain quarantined until a new manifest identity produces a fresh page set.
pub(super) async fn load_summary_archive_snapshot_v2_promotion_candidates(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<(i64, String)>> {
    Ok(
        load_summary_archive_snapshot_v2_promotion_candidates_with_cursor(pool, limit, 0)
            .await?
            .0,
    )
}

pub(super) async fn load_summary_archive_snapshot_v2_promotion_candidates_with_cursor(
    pool: &Pool<Sqlite>,
    limit: i64,
    start_after_id: i64,
) -> Result<(Vec<(i64, String)>, i64)> {
    let limit = limit.max(1);
    let window_limit = SUMMARY_ARCHIVE_SNAPSHOT_IDENTITY_WINDOW.max(limit);
    let mut cursor_id = start_after_id.max(0);
    for _ in 0..SUMMARY_ARCHIVE_SNAPSHOT_MAX_IDENTITY_WINDOWS {
        let identities = crate::observability::contention::observe_query(
            crate::observability::contention::StatsQuery::SummarySnapshotCandidates,
            sqlx::query_as::<_, SummaryArchiveSnapshotCandidateIdentity>(
                SUMMARY_ARCHIVE_SNAPSHOT_V2_PROMOTION_IDENTITY_SQL,
            )
            .bind(cursor_id)
            .bind(window_limit)
            .fetch_all(pool),
        )
        .await
        .context("load bounded unpromoted Summary Snapshot identities")?;
        let window_start_cursor = cursor_id;
        let Some(last_identity) = identities.last() else {
            return Ok((Vec::new(), cursor_id));
        };
        let window_end_cursor = last_identity.id;
        let candidates = load_summary_archive_snapshot_backfill_candidate_rows(
            pool,
            &identities,
            limit,
            false,
            true,
        )
        .await?;
        if !candidates.is_empty() {
            // A full fact batch does not prove that the rest of this identity window was
            // ineligible. Keep the window cursor in place so the next turn can consume the
            // remaining candidates after this batch changes their durable outcome.
            let next_cursor_id = if candidates.len() >= usize::try_from(limit).unwrap_or(usize::MAX)
            {
                window_start_cursor
            } else {
                window_end_cursor
            };
            return Ok((
                candidates
                    .into_iter()
                    .map(|candidate| (candidate.id, candidate.sha256))
                    .collect(),
                next_cursor_id,
            ));
        }
        cursor_id = window_end_cursor;
        if identities.len() < usize::try_from(window_limit).unwrap_or(usize::MAX) {
            break;
        }
    }
    Ok((Vec::new(), cursor_id))
}

async fn promote_verified_summary_snapshot_page_sets(
    pool: &Pool<Sqlite>,
    limit: i64,
    start_after_id: i64,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<(usize, i64)> {
    let (candidates, next_cursor_id) =
        load_summary_archive_snapshot_v2_promotion_candidates_with_cursor(
            pool,
            limit,
            start_after_id,
        )
        .await?;
    let had_candidates = !candidates.is_empty();
    let mut promoted = 0;
    let mut processed_any = false;
    let mut processed_cursor_id = start_after_id.max(0);
    for (archive_batch_id, manifest_sha256) in candidates {
        if started_at.elapsed() >= max_elapsed {
            break;
        }
        let proof = match crate::prepare_verified_summary_archive_snapshot_if_valid(
            pool,
            archive_batch_id,
            &manifest_sha256,
        )
        .await
        {
            Ok(proof) => proof,
            Err(error) => return Err(error),
        };
        if let Some(proof) = proof {
            let Some(admission) = super::super::retention::acquire_retention_write_admission(
                "summary_snapshot_proof_promotion",
            )
            .await
            else {
                return Err(super::super::retention::retention_write_deferred(
                    "summary_snapshot_proof_promotion",
                ));
            };
            if crate::publish_verified_summary_archive_snapshot(pool, &proof).await? {
                crate::record_managed_task_processed_work(&["summary_coverage_recovery"], 1);
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
            drop(admission);
        }
        processed_any = true;
        processed_cursor_id = archive_batch_id;
    }
    if promoted > 0 {
        tracing::info!(
            stage = "summary_snapshot_v2_proof_promoted",
            promoted,
            "promoted verified Summary Snapshot V2 page sets before backfill selection"
        );
    }
    let cursor_id = if !had_candidates {
        next_cursor_id
    } else if !processed_any {
        start_after_id.max(0)
    } else if started_at.elapsed() >= max_elapsed {
        processed_cursor_id
    } else {
        next_cursor_id
    };
    Ok((promoted, cursor_id))
}

pub(super) async fn load_summary_archive_snapshot_backfill_due_candidates(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    Ok(
        load_summary_archive_snapshot_backfill_due_candidates_with_cursor(pool, limit, 0)
            .await?
            .0,
    )
}

pub(super) async fn load_summary_archive_snapshot_backfill_due_candidates_with_cursor(
    pool: &Pool<Sqlite>,
    limit: i64,
    start_after_id: i64,
) -> Result<(Vec<HistoricalRollupStartupCandidateRow>, i64)> {
    let limit = limit.max(1);
    let window_limit = SUMMARY_ARCHIVE_SNAPSHOT_IDENTITY_WINDOW.max(limit);
    let identity_sql = summary_archive_snapshot_backfill_due_identity_sql();
    let mut cursor_id = start_after_id.max(0);
    for _ in 0..SUMMARY_ARCHIVE_SNAPSHOT_MAX_IDENTITY_WINDOWS {
        let identities = crate::observability::contention::observe_query(
            crate::observability::contention::StatsQuery::SummarySnapshotCandidates,
            sqlx::query_as::<_, SummaryArchiveSnapshotCandidateIdentity>(&identity_sql)
                .bind(cursor_id)
                .bind(window_limit)
                .fetch_all(pool),
        )
        .await
        .context("load due Summary Snapshot archive identities")?;
        let window_start_cursor = cursor_id;
        let Some(last_identity) = identities.last() else {
            return Ok((Vec::new(), 0));
        };
        let window_end_cursor = last_identity.id;
        let candidates = load_summary_archive_snapshot_backfill_candidate_rows(
            pool,
            &identities,
            limit,
            true,
            false,
        )
        .await?;
        if !candidates.is_empty() {
            // Due candidates are ordered by recency before id. When the bounded fact query fills
            // its limit, it is not safe to advance past the identity window: older candidates
            // in the same window must remain eligible on the next turn.
            let next_cursor_id = if candidates.len() >= usize::try_from(limit).unwrap_or(usize::MAX)
            {
                window_start_cursor
            } else {
                window_end_cursor
            };
            return Ok((candidates, next_cursor_id));
        }
        cursor_id = window_end_cursor;
        if identities.len() < usize::try_from(window_limit).unwrap_or(usize::MAX) {
            break;
        }
    }
    Ok((Vec::new(), cursor_id))
}

async fn load_summary_archive_snapshot_backfill_progress(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
) -> Result<SummaryArchiveSnapshotBackfillProgress> {
    let progress = sqlx::query_as::<
        _,
        (
            i64,
            Option<String>,
            i64,
            i64,
            i64,
            Option<String>,
            i64,
            i64,
            Option<Vec<u8>>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT next_page_index, next_occurred_at, next_row_id, cursor_version, retry_attempt, \
                NULLIF(hash_algorithm, ''), hash_state_version, hash_byte_offset, hash_state, \
                hash_complete_sha256, source_fingerprint
         FROM summary_archive_snapshot_backfill_outcome
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .fetch_optional(pool)
    .await
    .context("load Summary Snapshot V2 backfill page progress")?;
    let Some((
        page_index,
        occurred_at,
        row_id,
        cursor_version,
        retry_attempt,
        hash_algorithm,
        hash_state_version,
        hash_byte_offset,
        hash_state,
        hash_complete_sha256,
        source_fingerprint,
    )) = progress
    else {
        return Ok(SummaryArchiveSnapshotBackfillProgress {
            page_index: 0,
            next_occurred_at: None,
            next_row_id: 0,
            cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
            retry_attempt: 0,
            ..SummaryArchiveSnapshotBackfillProgress::default()
        });
    };
    Ok(SummaryArchiveSnapshotBackfillProgress {
        page_index: u32::try_from(page_index.max(0)).unwrap_or(u32::MAX),
        next_occurred_at: occurred_at,
        next_row_id: row_id.max(0),
        cursor_version,
        retry_attempt: retry_attempt.max(0),
        hash_algorithm,
        hash_state_version: hash_state_version.max(0),
        hash_byte_offset: u64::try_from(hash_byte_offset.max(0)).unwrap_or(u64::MAX),
        hash_state,
        hash_complete_sha256,
        source_fingerprint,
    })
}

pub(super) async fn record_summary_archive_snapshot_backfill_outcome(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    next_page_index: u32,
    next_row_id: i64,
) -> Result<()> {
    let Some(admission) = super::super::retention::acquire_retention_write_admission(
        "summary_snapshot_backfill_outcome",
    )
    .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "summary_snapshot_backfill_outcome",
        ));
    };
    let mut tx = pool.begin().await?;
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        disposition,
        failure_kind,
        &SummaryArchiveSnapshotBackfillProgress {
            page_index: next_page_index,
            next_occurred_at: None,
            next_row_id,
            cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
            retry_attempt: 0,
            ..SummaryArchiveSnapshotBackfillProgress::default()
        },
    )
    .await?;
    tx.commit().await?;
    drop(admission);
    Ok(())
}

async fn record_summary_archive_snapshot_backfill_outcome_preserving_progress(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    progress: SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let Some(admission) = super::super::retention::acquire_retention_write_admission(
        "summary_snapshot_backfill_outcome",
    )
    .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "summary_snapshot_backfill_outcome",
        ));
    };
    record_summary_archive_snapshot_backfill_outcome_preserving_progress_without_admission(
        pool,
        candidate,
        disposition,
        failure_kind,
        progress,
    )
    .await?;
    drop(admission);
    Ok(())
}

async fn record_summary_archive_snapshot_backfill_outcome_preserving_progress_without_admission(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    progress: SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        disposition,
        failure_kind,
        &SummaryArchiveSnapshotBackfillProgress {
            cursor_version: progress
                .cursor_version
                .max(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION),
            retry_attempt: progress.retry_attempt.saturating_add(1),
            ..progress
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn record_summary_coverage_obligation_terminal_gap_tx(
    connection: &mut SqliteConnection,
    candidate: &HistoricalRollupStartupCandidateRow,
    reason: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO summary_coverage_obligation \
         (archive_batch_id, manifest_sha256, coverage_start, coverage_end, state) \
         VALUES (?1, ?2, ?3, ?4, 'pending')",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(candidate.coverage_start_at.as_deref())
    .bind(candidate.coverage_end_at.as_deref())
    .execute(&mut *connection)
    .await
    .context("ensure Summary coverage obligation for terminal gap")?;
    sqlx::query(
        "UPDATE summary_coverage_obligation \
         SET state = 'terminal_gap', terminal_reason = ?3, resolved_at = NULL, updated_at = datetime('now') \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(reason)
    .execute(&mut *connection)
    .await
    .context("persist Summary coverage terminal gap")?;
    Ok(())
}

async fn record_summary_archive_snapshot_terminal_gap(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    reason: &str,
    progress: SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("summary_snapshot_terminal_gap")
            .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "summary_snapshot_terminal_gap",
        ));
    };
    let mut tx = pool.begin().await?;
    let progress = SummaryArchiveSnapshotBackfillProgress {
        cursor_version: progress
            .cursor_version
            .max(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION),
        retry_attempt: progress.retry_attempt.saturating_add(1),
        ..progress
    };
    // The durable outcome and its fail-closed obligation are one fact. Keeping them in the same
    // transaction prevents a crash from suppressing retries while leaving a pending obligation.
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        "unavailable",
        reason,
        &progress,
    )
    .await?;
    record_summary_coverage_obligation_terminal_gap_tx(tx.as_mut(), candidate, reason).await?;
    tx.commit().await?;
    drop(admission);
    Ok(())
}

#[derive(Debug)]
pub(super) struct SummaryArchiveSnapshotVerificationError {
    kind: &'static str,
    message: String,
}

impl SummaryArchiveSnapshotVerificationError {
    pub(super) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SummaryArchiveSnapshotVerificationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SummaryArchiveSnapshotVerificationError {}

fn summary_archive_backfill_failure_is_terminal(failure_kind: &str) -> bool {
    matches!(
        failure_kind,
        "verification_failed"
            | "manifest_sha_mismatch"
            | "invalid_timestamp"
            | "row_count_mismatch"
            | "empty_archive"
    )
}

pub(super) fn classify_summary_archive_snapshot_backfill_error(
    error: &anyhow::Error,
) -> &'static str {
    let is_sqlite_lock = error.chain().any(|cause| {
        let Some(sqlx_error) = cause.downcast_ref::<sqlx::Error>() else {
            return false;
        };
        let sqlx::Error::Database(database_error) = sqlx_error else {
            return false;
        };
        database_error.code().as_deref().is_some_and(|code| {
            matches!(code, "SQLITE_BUSY" | "SQLITE_LOCKED")
                || code
                    .parse::<i32>()
                    .is_ok_and(|code| matches!(code & 0xff, 5 | 6))
        })
    });
    if is_sqlite_lock {
        "deferred:sqlite_busy_or_locked"
    } else if let Some(error) = error.downcast_ref::<SummaryArchiveSnapshotVerificationError>() {
        match error.kind {
            "manifest_sha_mismatch" => "unavailable:manifest_sha_mismatch",
            "invalid_timestamp" => "unavailable:invalid_timestamp",
            "row_count_mismatch" => "unavailable:row_count_mismatch",
            "empty_archive" => "unavailable:empty_archive",
            _ => "unavailable:verification_failed",
        }
    } else {
        "unavailable:source_read_failed"
    }
}

async fn record_summary_archive_snapshot_backfill_outcome_tx(
    connection: &mut SqliteConnection,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    progress: &SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let disposition_kind =
        SummaryArchiveSnapshotBackfillDisposition::from_storage(disposition, failure_kind);
    let next_probe_at = disposition_kind.next_probe_at(progress.retry_attempt);
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_outcome \
         (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at, \
          next_page_index, next_occurred_at, next_row_id, cursor_version, retry_attempt, \
          hash_algorithm, hash_state_version, hash_byte_offset, hash_state, hash_complete_sha256, source_fingerprint, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now')) \
         ON CONFLICT(archive_batch_id, manifest_sha256) DO UPDATE SET \
           disposition = excluded.disposition, failure_kind = excluded.failure_kind, \
           next_probe_at = excluded.next_probe_at, next_page_index = excluded.next_page_index, \
           next_occurred_at = excluded.next_occurred_at, next_row_id = excluded.next_row_id, \
           cursor_version = excluded.cursor_version, retry_attempt = excluded.retry_attempt, \
           hash_algorithm = excluded.hash_algorithm, hash_state_version = excluded.hash_state_version, \
           hash_byte_offset = excluded.hash_byte_offset, hash_state = excluded.hash_state, \
           hash_complete_sha256 = excluded.hash_complete_sha256, \
           source_fingerprint = excluded.source_fingerprint, \
           updated_at = excluded.updated_at",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(disposition)
    .bind(failure_kind)
    .bind(next_probe_at)
    .bind(i64::from(progress.page_index))
    .bind(progress.next_occurred_at.as_deref())
    .bind(progress.next_row_id.max(0))
    .bind(progress.cursor_version.max(1))
    .bind(progress.retry_attempt.max(0))
    .bind(progress.hash_algorithm.as_deref().unwrap_or_default())
    .bind(progress.hash_state_version.max(0))
    .bind(i64::try_from(progress.hash_byte_offset).unwrap_or(i64::MAX))
    .bind(progress.hash_state.as_deref())
    .bind(progress.hash_complete_sha256.as_deref())
    .bind(progress.source_fingerprint.as_deref())
    .execute(&mut *connection)
    .await
    .context("failed to persist Summary Snapshot V2 backfill outcome")?;
    Ok(())
}

fn summary_archive_snapshot_backfill_budget_exhausted(
    started_at: Instant,
    max_elapsed: Duration,
) -> bool {
    started_at.elapsed() >= max_elapsed
}

async fn run_summary_archive_file_phase<F, T>(path: &Path, operation: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    let file_phase = crate::acquire_file_phase_permit("summary_archive_snapshot_backfill").await?;
    let archive_lock = super::super::retention::retention_archive_file_try_lock(path)?;
    crate::with_file_phase_permit(file_phase, async move {
        let result = operation.await;
        drop(archive_lock);
        result
    })
    .await
}

async fn run_summary_archive_temp_read_phase<F, T>(operation: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    let file_phase = crate::acquire_file_phase_permit("summary_archive_snapshot_backfill").await?;
    crate::with_file_phase_permit(file_phase, operation).await
}

pub(super) enum SummaryArchiveHashAdvance {
    Complete {
        sha256: String,
        progress: SummaryArchiveSnapshotBackfillProgress,
    },
    Deferred(SummaryArchiveSnapshotBackfillProgress),
}

pub(super) fn summary_archive_source_fingerprint(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat Summary archive source {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "unix:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let modified_at = metadata
            .modified()
            .context("read Summary archive source modified timestamp")?
            .duration_since(std::time::UNIX_EPOCH)
            .context("Summary archive source timestamp predates Unix epoch")?;
        Ok(format!(
            "portable:{}:{}:{}",
            metadata.len(),
            modified_at.as_secs(),
            modified_at.subsec_nanos()
        ))
    }
}

pub(super) fn resumable_summary_archive_sha256(
    path: &Path,
    mut progress: SummaryArchiveSnapshotBackfillProgress,
    source_fingerprint: String,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<SummaryArchiveHashAdvance> {
    let has_resumable_state = progress.hash_algorithm.as_deref()
        == Some(SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM)
        && progress.hash_state_version == SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION
        && progress.source_fingerprint.as_deref() == Some(source_fingerprint.as_str())
        && progress.hash_state.is_some();
    let mut hasher = if has_resumable_state {
        let state = progress
            .hash_state
            .as_deref()
            .expect("resumable hash state checked above");
        let serialized = <&SerializedState<ResumableSha256>>::try_from(state).map_err(|_| {
            SummaryArchiveSnapshotVerificationError::new(
                "verification_failed",
                "invalid persisted Summary archive SHA-256 state",
            )
        })?;
        ResumableSha256::deserialize(serialized).map_err(|_| {
            SummaryArchiveSnapshotVerificationError::new(
                "verification_failed",
                "cannot deserialize persisted Summary archive SHA-256 state",
            )
        })?
    } else {
        progress.hash_byte_offset = 0;
        ResumableSha256::new()
    };
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let source_length = file
        .metadata()
        .with_context(|| format!("failed to stat file for sha256 {}", path.display()))?
        .len();
    if progress.hash_byte_offset > source_length {
        progress.hash_byte_offset = 0;
        hasher = ResumableSha256::new();
    }
    file.seek(SeekFrom::Start(progress.hash_byte_offset))
        .with_context(|| format!("failed to seek Summary archive source {}", path.display()))?;
    let mut buffer = [0_u8; SUMMARY_ARCHIVE_SNAPSHOT_HASH_BUFFER_BYTES];
    loop {
        if summary_archive_snapshot_backfill_budget_exhausted(started_at, max_elapsed) {
            progress.hash_algorithm = Some(SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM.to_string());
            progress.hash_state_version = SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION;
            progress.hash_state = Some(hasher.serialize().to_vec());
            progress.source_fingerprint = Some(source_fingerprint);
            return Ok(SummaryArchiveHashAdvance::Deferred(progress));
        }
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read Summary archive source {}", path.display()))?;
        if read == 0 {
            let fingerprint_after = summary_archive_source_fingerprint(path)?;
            if fingerprint_after != source_fingerprint {
                bail!("Summary archive source identity changed during SHA-256 proof");
            }
            let digest = hasher.finalize();
            let digest = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            progress.hash_algorithm = Some(SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM.to_string());
            progress.hash_state_version = SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION;
            progress.hash_state = None;
            progress.hash_complete_sha256 = Some(digest.clone());
            progress.source_fingerprint = Some(source_fingerprint);
            return Ok(SummaryArchiveHashAdvance::Complete {
                sha256: digest,
                progress,
            });
        }
        hasher.update(&buffer[..read]);
        progress.hash_byte_offset = progress
            .hash_byte_offset
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
}

async fn backfill_summary_archive_snapshot_v2_candidate(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<&'static str> {
    // A valid V2 page is already an exact authority and does not require reopening the raw
    // archive. Marking it complete here also upgrades V2 pages written by an older process into
    // the durable backfill outcome index.
    let proof_marker_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_archive_snapshot_v2_proof \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2)",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .fetch_one(pool)
    .await?
        != 0;
    if proof_marker_exists
        && summary_archive_snapshot_has_final_proof_read_only(pool, candidate.id, &candidate.sha256)
            .await?
    {
        return Ok("complete");
    }
    if candidate.row_count <= 0 {
        return Ok("unavailable:empty_archive");
    }
    let archive_path = Path::new(&candidate.file_path);
    if !archive_path.exists() {
        return Ok("unavailable:missing_source");
    }
    let source_fingerprint = run_summary_archive_file_phase(archive_path, async {
        summary_archive_source_fingerprint(archive_path)
    })
    .await?;
    let mut hash_progress =
        load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
    if hash_progress
        .source_fingerprint
        .as_deref()
        .is_some_and(|existing| existing != source_fingerprint)
    {
        // No incomplete V2 page may survive a source replacement. The manifest identity scopes
        // the outcome row, while this filesystem identity fences work that spans attempts.
        let Some(admission) = super::super::retention::acquire_retention_write_admission(
            "summary_archive_snapshot_backfill",
        )
        .await
        else {
            return Err(super::super::retention::retention_write_deferred(
                "summary_archive_snapshot_backfill",
            ));
        };
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        if crate::summary_source_change::summary_archive_snapshot_final_proof_marker_exists_tx(
            tx.as_mut(),
            candidate.id,
            &candidate.sha256,
        )
        .await?
        {
            tx.rollback().await?;
            drop(admission);
            return Ok("complete");
        }
        sqlx::query(
            "DELETE FROM summary_archive_snapshot
             WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
        )
        .bind(candidate.id)
        .bind(&candidate.sha256)
        .execute(tx.as_mut())
        .await?;
        hash_progress = SummaryArchiveSnapshotBackfillProgress {
            cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
            source_fingerprint: Some(source_fingerprint.clone()),
            ..SummaryArchiveSnapshotBackfillProgress::default()
        };
        record_summary_archive_snapshot_backfill_outcome_tx(
            tx.as_mut(),
            candidate,
            "in_progress",
            "source_identity_changed",
            &hash_progress,
        )
        .await?;
        tx.commit().await?;
        drop(admission);
    }
    let actual_sha256 = if hash_progress.source_fingerprint.as_deref()
        == Some(source_fingerprint.as_str())
        && hash_progress.hash_complete_sha256.as_deref() == Some(candidate.sha256.as_str())
    {
        candidate.sha256.clone()
    } else {
        match run_summary_archive_file_phase(archive_path, async {
            resumable_summary_archive_sha256(
                archive_path,
                hash_progress,
                source_fingerprint.clone(),
                started_at,
                max_elapsed,
            )
        })
        .await?
        {
            SummaryArchiveHashAdvance::Complete { sha256, progress } => {
                let Some(admission) = super::super::retention::acquire_retention_write_admission(
                    "summary_archive_snapshot_backfill",
                )
                .await
                else {
                    return Err(super::super::retention::retention_write_deferred(
                        "summary_archive_snapshot_backfill",
                    ));
                };
                let mut tx = pool.begin().await?;
                record_summary_archive_snapshot_backfill_outcome_tx(
                    tx.as_mut(),
                    candidate,
                    "in_progress",
                    "hash_verified",
                    &progress,
                )
                .await?;
                tx.commit().await?;
                drop(admission);
                sha256
            }
            SummaryArchiveHashAdvance::Deferred(progress) => {
                // Hash state itself is committed progress. The outer scheduler records the
                // bounded defer separately, preserving this state and applying its backoff.
                record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                    pool,
                    candidate,
                    "in_progress",
                    "hash_progress",
                    progress,
                )
                .await?;
                return Ok("deferred:budget");
            }
        }
    };
    if actual_sha256 != candidate.sha256 {
        return Ok("unavailable:manifest_sha_mismatch");
    }
    let archive_row = crate::stats::ArchiveBatchPathRow::with_coverage(
        candidate.file_path.clone(),
        candidate.coverage_start_at.clone(),
        candidate.coverage_end_at.clone(),
    );
    let archive_source = run_summary_archive_file_phase(archive_path, async {
        if summary_archive_source_fingerprint(archive_path)? != source_fingerprint {
            bail!("Summary archive source identity changed before opening archive");
        }
        crate::stats::open_invocation_archive_batch_pool(&archive_row, "summary-snapshot-backfill")
            .await
    })
    .await;
    let Some((archive_pool, temp_cleanup)) = (match archive_source {
        Ok(source) => source,
        Err(_) => return Ok("unavailable:source_open_failed"),
    }) else {
        return Ok("unavailable:source_open_failed");
    };

    let result = async {
        let archive_columns = run_summary_archive_temp_read_phase(async {
            load_archive_table_columns(&archive_pool, "codex_invocations").await
        })
        .await?;
        let query_sql = build_invocation_archive_rows_time_chunk_query(&archive_columns);
        let mut progress = load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
        if progress.cursor_version < SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION
            && (progress.next_row_id > 0 || progress.page_index > 0)
        {
            // A pre-V2 checkpoint only proves ID order. It cannot be combined with the
            // chronological V2 proof, so discard its unproven pages and restart this manifest.
            let Some(admission) = super::super::retention::acquire_retention_write_admission(
                "summary_archive_snapshot_backfill",
            )
            .await
            else {
                return Err(super::super::retention::retention_write_deferred(
                    "summary_archive_snapshot_backfill",
                ));
            };
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
            if crate::summary_source_change::summary_archive_snapshot_final_proof_marker_exists_tx(
                tx.as_mut(),
                candidate.id,
                &candidate.sha256,
            )
            .await?
            {
                tx.rollback().await?;
                drop(admission);
                return Ok("complete");
            }
            sqlx::query(
                "DELETE FROM summary_archive_snapshot
                 WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
            )
            .bind(candidate.id)
            .bind(&candidate.sha256)
            .execute(tx.as_mut())
            .await?;
            record_summary_archive_snapshot_backfill_outcome_tx(
                tx.as_mut(),
                candidate,
                "in_progress",
                "legacy_cursor_reset",
                &SummaryArchiveSnapshotBackfillProgress {
                    page_index: 0,
                    next_occurred_at: None,
                    next_row_id: 0,
                    cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
                    retry_attempt: 0,
                    ..progress.clone()
                },
            )
            .await?;
            tx.commit().await?;
            drop(admission);
            progress.page_index = 0;
            progress.next_occurred_at = None;
            progress.next_row_id = 0;
            progress.cursor_version = SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION;
            progress.retry_attempt = 0;
        }
        let mut total_rows = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT SUM(row_count) FROM summary_archive_snapshot
             WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND page_index < ?3",
        )
        .bind(candidate.id)
        .bind(&candidate.sha256)
        .bind(i64::from(progress.page_index))
        .fetch_one(pool)
        .await?
        .unwrap_or_default();
        loop {
            if summary_archive_snapshot_backfill_budget_exhausted(started_at, max_elapsed) {
                return Ok("deferred:budget");
            }
            let (rows, has_more) = run_summary_archive_temp_read_phase(async {
                load_invocation_archive_rows_time_chunk(
                    &archive_pool,
                    &query_sql,
                    progress.next_occurred_at.as_deref(),
                    progress.next_row_id,
                )
                .await
            })
            .await?;
            if rows.is_empty() {
                break;
            }
            let coverage_start = if progress.page_index == 0 {
                candidate
                    .coverage_start_at
                    .clone()
                    .or_else(|| rows.first().map(|row| row.occurred_at.clone()))
            } else {
                rows.first().map(|row| row.occurred_at.clone())
            };
            let coverage_end = if !has_more {
                candidate
                    .coverage_end_at
                    .clone()
                    .or_else(|| rows.last().map(|row| row.occurred_at.clone()))
            } else {
                rows.last().map(|row| row.occurred_at.clone())
            };
            let (Some(coverage_start), Some(coverage_end)) = (coverage_start, coverage_end) else {
                return Err(anyhow::Error::new(
                    SummaryArchiveSnapshotVerificationError::new(
                        "verification_failed",
                        "Summary Snapshot source page is missing coverage",
                    ),
                ));
            };
            let invoke_ids_by_row_id = if archive_columns.contains("invoke_id") {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT id, invoke_id FROM codex_invocations WHERE id IN (",
                );
                let mut separated = query.separated(", ");
                for row in &rows {
                    separated.push_bind(row.id);
                }
                separated.push_unseparated(")");
                run_summary_archive_temp_read_phase(async {
                    Ok(query
                        .build_query_as::<(i64, String)>()
                        .fetch_all(&archive_pool)
                        .await?
                        .into_iter()
                        .collect::<HashMap<_, _>>())
                })
                .await?
            } else {
                HashMap::new()
            };
            let payload = super::super::retention::encode_summary_archive_snapshot_v2_payload(
                &rows,
                &invoke_ids_by_row_id,
            )?;
            let page = SummaryArchiveSnapshotPage {
                archive_batch_id: candidate.id,
                manifest_sha256: candidate.sha256.clone(),
                page_index: progress.page_index,
                coverage_start,
                coverage_end,
                row_count: u32::try_from(rows.len()).map_err(|_| {
                    anyhow::Error::new(SummaryArchiveSnapshotVerificationError::new(
                        "verification_failed",
                        "Summary Snapshot V2 page row count overflow",
                    ))
                })?,
                payload,
            };
            run_summary_archive_file_phase(archive_path, async {
                if summary_archive_source_fingerprint(archive_path)? != source_fingerprint {
                    bail!("Summary archive source identity changed before V2 page commit");
                }
                Ok(())
            })
            .await?;
            // Serialize the proof check and page write with the final-proof transaction. The
            // candidate query runs before this point and can become stale while a retention task
            // publishes its proof; a deferred transaction would then rewrite the page and fire
            // the trigger that revokes the newly published authority.
            let Some(admission) = super::super::retention::acquire_retention_write_admission(
                "summary_archive_snapshot_backfill",
            )
            .await
            else {
                return Err(super::super::retention::retention_write_deferred(
                    "summary_archive_snapshot_backfill",
                ));
            };
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
            if crate::summary_source_change::summary_archive_snapshot_final_proof_marker_exists_tx(
                tx.as_mut(),
                candidate.id,
                &candidate.sha256,
            )
            .await?
            {
                tx.rollback().await?;
                drop(admission);
                return Ok("complete");
            }
            store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page).await?;
            record_summary_archive_snapshot_backfill_outcome_tx(
                tx.as_mut(),
                candidate,
                "in_progress",
                "",
                &SummaryArchiveSnapshotBackfillProgress {
                    page_index: progress.page_index.saturating_add(1),
                    next_occurred_at: rows.last().map(|row| row.occurred_at.clone()),
                    next_row_id: rows
                        .last()
                        .map(|row| row.id)
                        .unwrap_or(progress.next_row_id),
                    cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
                    retry_attempt: 0,
                    source_fingerprint: Some(source_fingerprint.clone()),
                    ..progress.clone()
                },
            )
            .await?;
            tx.commit().await?;
            drop(admission);
            total_rows = total_rows.saturating_add(i64::try_from(rows.len())?);
            progress.next_row_id = rows.last().map(|row| row.id).ok_or_else(|| {
                anyhow::Error::new(SummaryArchiveSnapshotVerificationError::new(
                    "verification_failed",
                    "Summary Snapshot page is missing row id",
                ))
            })?;
            progress.next_occurred_at = rows.last().map(|row| row.occurred_at.clone());
            progress.page_index = progress.page_index.saturating_add(1);
            if !has_more {
                break;
            }
        }
        if total_rows != candidate.row_count {
            return Err(anyhow::Error::new(
                SummaryArchiveSnapshotVerificationError::new(
                    "row_count_mismatch",
                    format!(
                        "Summary Snapshot V2 row count mismatch: expected {}, got {}",
                        candidate.row_count, total_rows
                    ),
                ),
            ));
        }
        let proof = match crate::prepare_verified_summary_archive_snapshot_if_valid(
            pool,
            candidate.id,
            &candidate.sha256,
        )
        .await
        {
            Ok(Some(proof)) => proof,
            Ok(None) => {
                return Err(anyhow::Error::new(
                    SummaryArchiveSnapshotVerificationError::new(
                        "verification_failed",
                        "Summary Snapshot V2 proof validation failed after backfill",
                    ),
                ));
            }
            Err(error) => return Err(error),
        };
        let Some(admission) = super::super::retention::acquire_retention_write_admission(
            "summary_archive_snapshot_backfill",
        )
        .await
        else {
            return Err(super::super::retention::retention_write_deferred(
                "summary_archive_snapshot_backfill",
            ));
        };
        if !crate::publish_verified_summary_archive_snapshot(pool, &proof).await? {
            drop(admission);
            return Err(anyhow::Error::new(
                SummaryArchiveSnapshotVerificationError::new(
                    "verification_failed",
                    "Summary Snapshot V2 proof publication failed after backfill",
                ),
            ));
        }
        drop(admission);
        Ok("complete")
    }
    .await;
    archive_pool.close().await;
    drop(temp_cleanup);
    result?;
    Ok("complete")
}

pub(crate) async fn backfill_summary_archive_snapshots_v2_window(
    pool: &Pool<Sqlite>,
    max_elapsed: Duration,
) -> Result<SummaryArchiveSnapshotBackfillWindowResult> {
    let started_at = Instant::now();
    let (mut cursor_id, mut high_watermark_id, checkpoint_completed) =
        load_summary_archive_snapshot_backfill_checkpoint(pool).await?;
    let (promoted_page_sets, promotion_cursor_id) = promote_verified_summary_snapshot_page_sets(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
        cursor_id,
        started_at,
        max_elapsed,
    )
    .await?;
    cursor_id = promotion_cursor_id;
    let observed_high_watermark = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = ?1",
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?;
    high_watermark_id = high_watermark_id.max(observed_high_watermark);
    // Recent coverage and due retries must progress independently of the sweep cursor. The
    // sweep only services never-seen backlog entries after the due queue has had a bounded turn.
    let due_cursor_id = load_summary_archive_snapshot_due_cursor(pool).await?;
    let (mut candidates, due_cursor_after_scan) =
        load_summary_archive_snapshot_backfill_due_candidates_with_cursor(
            pool,
            SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
            due_cursor_id,
        )
        .await?;
    let candidates_from_due_queue = !candidates.is_empty();
    let mut sweep_cursor_after_scan = None;
    if candidates.is_empty() {
        let (sweep_candidates, next_cursor_id) =
            load_summary_archive_snapshot_backfill_candidates_with_cursor(
                pool,
                cursor_id,
                SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
            )
            .await?;
        candidates = sweep_candidates;
        sweep_cursor_after_scan = next_cursor_id;
    }
    let mut wrapped = false;
    let mut attempted_wrap = false;
    if candidates.is_empty() && cursor_id > 0 && sweep_cursor_after_scan.is_none() {
        attempted_wrap = true;
        candidates = load_summary_archive_snapshot_backfill_candidates(
            pool,
            0,
            SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
        )
        .await?;
        wrapped = !candidates.is_empty();
        if wrapped {
            cursor_id = 0;
        }
    }
    let candidate_count = candidates.len();
    let recent_candidate_count = candidates
        .iter()
        .filter(|candidate| {
            summary_archive_snapshot_candidate_intersects_current_30d(candidate, Utc::now())
        })
        .count();
    // Obligations are identity-bound to a manifest SHA.  A replacement archive deliberately
    // leaves the old identity available for audit, but it must not keep the supervisor alive
    // after candidate selection has moved to the replacement SHA.
    let obligation_counts = sqlx::query_as::<_, (String, i64)>(
        "SELECT obligation.state, COUNT(*)
         FROM summary_coverage_obligation AS obligation
         INNER JOIN archive_batches AS batches
           ON batches.id = obligation.archive_batch_id
          AND batches.sha256 = obligation.manifest_sha256
         GROUP BY obligation.state",
    )
    .fetch_all(pool)
    .await?;
    // Terminal gaps are durable fail-closed proofs, not work that should keep the
    // supervisor spinning.  They remain visible through `terminal_gap_count` and
    // availability overlays, but only unresolved/retryable obligations reserve another
    // recovery turn.
    let pending_obligation_count = obligation_counts
        .iter()
        .filter(|(state, _)| state != "resolved" && state != "terminal_gap")
        .map(|(_, count)| usize::try_from((*count).max(0)).unwrap_or(usize::MAX))
        .sum();
    let terminal_gap_count = obligation_counts
        .iter()
        .find(|(state, _)| state == "terminal_gap")
        .map(|(_, count)| usize::try_from((*count).max(0)).unwrap_or(usize::MAX))
        .unwrap_or_default();
    // Report only proofs promoted by this bounded turn. A cumulative table count would
    // repeatedly re-report historical work on every supervisor pass.
    let verified_proof_count = promoted_page_sets;
    let mut result = SummaryArchiveSnapshotBackfillWindowResult {
        next_cursor_id: cursor_id,
        candidate_count,
        recent_candidate_count,
        wrapped,
        pending_obligation_count,
        terminal_gap_count,
        verified_proof_count,
        materialized_archive_batches: promoted_page_sets,
        ..SummaryArchiveSnapshotBackfillWindowResult::default()
    };
    if candidates.is_empty()
        && !candidates_from_due_queue
        && let Some(next_cursor_id) = sweep_cursor_after_scan
    {
        result.next_cursor_id = next_cursor_id;
        let next_due_cursor_id = if due_cursor_after_scan == 0 && due_cursor_id > 0 {
            0
        } else {
            due_cursor_after_scan
        };
        store_summary_archive_snapshot_due_cursor(pool, next_due_cursor_id).await?;
        store_summary_archive_snapshot_backfill_checkpoint(
            pool,
            next_cursor_id,
            high_watermark_id,
            next_cursor_id >= high_watermark_id,
        )
        .await?;
        return Ok(result);
    }
    for candidate in candidates {
        if started_at.elapsed() >= max_elapsed {
            let progress =
                load_summary_archive_snapshot_backfill_progress(pool, &candidate).await?;
            record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                pool, &candidate, "deferred", "budget", progress,
            )
            .await?;
            result.hit_budget = true;
            break;
        }
        result.scanned_archive_batches += 1;
        let outcome = match backfill_summary_archive_snapshot_v2_candidate(
            pool,
            &candidate,
            started_at,
            max_elapsed,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => classify_summary_archive_snapshot_backfill_error(&error),
        };
        if let Some((disposition, failure_kind)) = outcome.split_once(':') {
            if disposition == "deferred" {
                let progress =
                    load_summary_archive_snapshot_backfill_progress(pool, &candidate).await?;
                record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                    pool,
                    &candidate,
                    disposition,
                    failure_kind,
                    progress,
                )
                .await?;
                result.hit_budget = true;
                break;
            }
            let progress =
                load_summary_archive_snapshot_backfill_progress(pool, &candidate).await?;
            if summary_archive_backfill_failure_is_terminal(failure_kind) {
                record_summary_archive_snapshot_terminal_gap(
                    pool,
                    &candidate,
                    failure_kind,
                    progress,
                )
                .await?;
            } else {
                record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                    pool,
                    &candidate,
                    disposition,
                    failure_kind,
                    progress,
                )
                .await?;
            }
            result.unavailable_archive_batches += 1;
        } else {
            crate::record_managed_task_processed_work(&["summary_coverage_recovery"], 1);
            record_summary_archive_snapshot_backfill_outcome(
                pool, &candidate, "complete", "", 0, 0,
            )
            .await?;
            result.materialized_archive_batches += 1;
            result.verified_proof_count = result.verified_proof_count.saturating_add(1);
        }
        if !candidates_from_due_queue {
            result.next_cursor_id = candidate.id;
            store_summary_archive_snapshot_backfill_checkpoint(
                pool,
                result.next_cursor_id,
                high_watermark_id,
                result.next_cursor_id >= high_watermark_id && !result.hit_budget,
            )
            .await?;
        }
    }
    if !result.hit_budget {
        let next_due_cursor_id = if due_cursor_after_scan == 0 && due_cursor_id > 0 {
            0
        } else {
            due_cursor_after_scan
        };
        store_summary_archive_snapshot_due_cursor(pool, next_due_cursor_id).await?;
    }
    if candidate_count == 0
        && !(checkpoint_completed && cursor_id == 0 && high_watermark_id == observed_high_watermark)
    {
        let checkpoint_cursor_id = if attempted_wrap { 0 } else { cursor_id };
        store_summary_archive_snapshot_backfill_checkpoint(
            pool,
            checkpoint_cursor_id,
            high_watermark_id,
            checkpoint_cursor_id == 0 && attempted_wrap,
        )
        .await?;
    } else if candidates_from_due_queue {
        store_summary_archive_snapshot_backfill_checkpoint(
            pool,
            cursor_id,
            high_watermark_id,
            false,
        )
        .await?;
    }
    Ok(result)
}
