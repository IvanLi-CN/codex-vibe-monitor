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

async fn backfill_summary_archive_snapshot_v2_candidate(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<&'static str> {
    // A valid V2 page is already an exact authority and does not require reopening the raw
    // archive. Marking it complete here also upgrades V2 pages written by an older process into
    // the durable backfill outcome index.
    if summary_archive_snapshot_has_final_proof(pool, candidate.id, &candidate.sha256).await? {
        return Ok("complete");
    }
    if candidate.row_count <= 0 {
        return Ok("unavailable:empty_archive");
    }
    let archive_path = Path::new(&candidate.file_path);
    if !archive_path.exists() {
        return Ok("unavailable:missing_source");
    }
    let source_fingerprint = summary_archive_source_fingerprint(archive_path)?;
    let mut hash_progress =
        load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
    if hash_progress
        .source_fingerprint
        .as_deref()
        .is_some_and(|existing| existing != source_fingerprint)
    {
        // No incomplete V2 page may survive a source replacement. The manifest identity scopes
        // the outcome row, while this filesystem identity fences work that spans attempts.
        let mut tx = pool.begin().await?;
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
    }
    let actual_sha256 = if hash_progress.source_fingerprint.as_deref()
        == Some(source_fingerprint.as_str())
        && hash_progress.hash_complete_sha256.as_deref() == Some(candidate.sha256.as_str())
    {
        candidate.sha256.clone()
    } else {
        match resumable_summary_archive_sha256(
            archive_path,
            hash_progress,
            source_fingerprint.clone(),
            started_at,
            max_elapsed,
        )? {
            SummaryArchiveHashAdvance::Complete { sha256, progress } => {
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
    let archive_source =
        crate::stats::open_invocation_archive_batch_pool(&archive_row, "summary-snapshot-backfill")
            .await;
    let Some((archive_pool, temp_cleanup)) = (match archive_source {
        Ok(source) => source,
        Err(_) => return Ok("unavailable:source_open_failed"),
    }) else {
        return Ok("unavailable:source_open_failed");
    };

    let result = async {
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let query_sql = build_invocation_archive_rows_time_chunk_query(&archive_columns);
        let mut progress = load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
        if progress.cursor_version < SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION
            && (progress.next_row_id > 0 || progress.page_index > 0)
        {
            // A pre-V2 checkpoint only proves ID order. It cannot be combined with the
            // chronological V2 proof, so discard its unproven pages and restart this manifest.
            let mut tx = pool.begin().await?;
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
            let (rows, has_more) = load_invocation_archive_rows_time_chunk(
                &archive_pool,
                &query_sql,
                progress.next_occurred_at.as_deref(),
                progress.next_row_id,
            )
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
                bail!("Summary Snapshot source page is missing coverage");
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
                query
                    .build_query_as::<(i64, String)>()
                    .fetch_all(&archive_pool)
                    .await?
                    .into_iter()
                    .collect::<HashMap<_, _>>()
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
                row_count: u32::try_from(rows.len())
                    .context("Summary Snapshot V2 page row count overflow")?,
                payload,
            };
            if summary_archive_source_fingerprint(archive_path)? != source_fingerprint {
                bail!("Summary archive source identity changed before V2 page commit");
            }
            let mut tx = pool.begin().await?;
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
            total_rows = total_rows.saturating_add(i64::try_from(rows.len())?);
            progress.next_row_id = rows
                .last()
                .map(|row| row.id)
                .ok_or_else(|| anyhow!("Summary Snapshot page is missing row id"))?;
            progress.next_occurred_at = rows.last().map(|row| row.occurred_at.clone());
            progress.page_index = progress.page_index.saturating_add(1);
            if !has_more {
                break;
            }
        }
        if total_rows != candidate.row_count {
            bail!(
                "Summary Snapshot V2 row count mismatch: expected {}, got {}",
                candidate.row_count,
                total_rows
            );
        }
        if !ensure_summary_archive_snapshot_v2_final_proof(pool, candidate.id, &candidate.sha256)
            .await?
        {
            bail!("Summary Snapshot V2 proof validation failed after backfill");
        }
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
    let promoted_page_sets = promote_verified_summary_snapshot_page_sets(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
        started_at,
        max_elapsed,
    )
    .await?;
    let (mut cursor_id, mut high_watermark_id, checkpoint_completed) =
        load_summary_archive_snapshot_backfill_checkpoint(pool).await?;
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
    let mut candidates = load_summary_archive_snapshot_backfill_due_candidates(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
    )
    .await?;
    let candidates_from_due_queue = !candidates.is_empty();
    if candidates.is_empty() {
        candidates = load_summary_archive_snapshot_backfill_candidates(
            pool,
            cursor_id,
            SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
        )
        .await?;
    }
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
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
    let verified_proof_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof")
            .fetch_one(pool)
            .await
            .map(|count| usize::try_from(count.max(0)).unwrap_or(usize::MAX))?;
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
            record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                pool,
                &candidate,
                disposition,
                failure_kind,
                progress,
            )
            .await?;
            if summary_archive_backfill_failure_is_terminal(failure_kind) {
                record_summary_coverage_obligation_terminal_gap(pool, &candidate, failure_kind)
                    .await?;
            }
            result.unavailable_archive_batches += 1;
        } else {
            record_summary_archive_snapshot_backfill_outcome(
                pool, &candidate, "complete", "", 0, 0,
            )
            .await?;
            result.materialized_archive_batches += 1;
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
    if candidate_count == 0
        && !(checkpoint_completed && cursor_id == 0 && high_watermark_id == observed_high_watermark)
    {
        store_summary_archive_snapshot_backfill_checkpoint(pool, 0, high_watermark_id, true)
            .await?;
    }
    Ok(result)
}

#[derive(Debug)]
pub(crate) struct HistoricalRollupStartupWindowResult {
    pub(crate) summary: HistoricalRollupMaterializationSummary,
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}

#[derive(Debug)]
pub(crate) struct HistoricalRollupStartupPendingHint {
    pub(crate) pending_archive_batches: usize,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
}

#[derive(Debug)]
pub(crate) struct LegacyDetailMirrorRecoveryWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}
