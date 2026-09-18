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

const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_DUE_CANDIDATES_SQL: &str = r#"
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
        "#;

async fn load_summary_archive_snapshot_backfill_due_candidates(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_DUE_CANDIDATES_SQL,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await
    .context("failed to load Summary Snapshot V2 backfill due candidates")
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

async fn record_summary_archive_snapshot_backfill_outcome(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    next_page_index: u32,
    next_row_id: i64,
) -> Result<()> {
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
    Ok(())
}

async fn record_summary_archive_snapshot_backfill_outcome_preserving_progress(
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

async fn record_summary_coverage_obligation_terminal_gap(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    reason: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO summary_coverage_obligation \
         (archive_batch_id, manifest_sha256, coverage_start, coverage_end, state) \
         VALUES (?1, ?2, ?3, ?4, 'pending')",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(candidate.coverage_start_at.as_deref())
    .bind(candidate.coverage_end_at.as_deref())
    .execute(tx.as_mut())
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
    .execute(tx.as_mut())
    .await
    .context("persist Summary coverage terminal gap")?;
    tx.commit().await?;
    Ok(())
}

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

fn classify_summary_archive_snapshot_backfill_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("busy") || message.contains("locked") {
        "deferred:sqlite_busy_or_locked"
    } else if message.contains("budget") || message.contains("deadline") {
        "deferred:budget"
    } else if message.contains("invalid")
        || message.contains("mismatch")
        || message.contains("proof")
        || message.contains("semantic")
        || message.contains("page")
    {
        "unavailable:verification_failed"
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

enum SummaryArchiveHashAdvance {
    Complete {
        sha256: String,
        progress: SummaryArchiveSnapshotBackfillProgress,
    },
    Deferred(SummaryArchiveSnapshotBackfillProgress),
}

fn summary_archive_source_fingerprint(path: &Path) -> Result<String> {
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

fn resumable_summary_archive_sha256(
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
        let serialized = <&SerializedState<ResumableSha256>>::try_from(state)
            .map_err(|_| anyhow!("invalid persisted Summary archive SHA-256 state"))?;
        ResumableSha256::deserialize(serialized)
            .map_err(|_| anyhow!("cannot deserialize persisted Summary archive SHA-256 state"))?
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
