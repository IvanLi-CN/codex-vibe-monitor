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
    let actual_sha256 = match verify_summary_archive_snapshot_source(
        pool,
        candidate,
        archive_path,
        &source_fingerprint,
        started_at,
        max_elapsed,
    )
    .await?
    {
        SummaryArchiveSourceVerification::Verified(sha256) => sha256,
        SummaryArchiveSourceVerification::Deferred => return Ok("deferred:budget"),
        SummaryArchiveSourceVerification::ManifestMismatch => {
            return Ok("unavailable:manifest_sha_mismatch");
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

    let result = backfill_summary_archive_snapshot_v2_pages(
        pool,
        candidate,
        archive_path,
        &archive_pool,
        &source_fingerprint,
        started_at,
        max_elapsed,
    )
    .await;
    archive_pool.close().await;
    drop(temp_cleanup);
    result?;
    Ok("complete")
}

enum SummaryArchiveSourceVerification {
    Verified(String),
    Deferred,
    ManifestMismatch,
}

async fn verify_summary_archive_snapshot_source(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    archive_path: &Path,
    source_fingerprint: &str,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<SummaryArchiveSourceVerification> {
    let mut hash_progress =
        load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
    if hash_progress
        .source_fingerprint
        .as_deref()
        .is_some_and(|existing| existing != source_fingerprint)
    {
        hash_progress =
            reset_summary_archive_snapshot_source_identity(pool, candidate, source_fingerprint)
                .await?;
    }
    if hash_progress.source_fingerprint.as_deref() == Some(source_fingerprint)
        && hash_progress.hash_complete_sha256.as_deref() == Some(candidate.sha256.as_str())
    {
        return Ok(SummaryArchiveSourceVerification::Verified(
            candidate.sha256.clone(),
        ));
    }
    match resumable_summary_archive_sha256(
        archive_path,
        hash_progress,
        source_fingerprint.to_owned(),
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
            if sha256 == candidate.sha256 {
                Ok(SummaryArchiveSourceVerification::Verified(sha256))
            } else {
                Ok(SummaryArchiveSourceVerification::ManifestMismatch)
            }
        }
        SummaryArchiveHashAdvance::Deferred(progress) => {
            record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                pool,
                candidate,
                "in_progress",
                "hash_progress",
                progress,
            )
            .await?;
            Ok(SummaryArchiveSourceVerification::Deferred)
        }
    }
}

async fn reset_summary_archive_snapshot_source_identity(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    source_fingerprint: &str,
) -> Result<SummaryArchiveSnapshotBackfillProgress> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM summary_archive_snapshot
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .execute(tx.as_mut())
    .await?;
    let progress = SummaryArchiveSnapshotBackfillProgress {
        cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
        source_fingerprint: Some(source_fingerprint.to_owned()),
        ..SummaryArchiveSnapshotBackfillProgress::default()
    };
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        "in_progress",
        "source_identity_changed",
        &progress,
    )
    .await?;
    tx.commit().await?;
    Ok(progress)
}

async fn backfill_summary_archive_snapshot_v2_pages(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    archive_path: &Path,
    archive_pool: &Pool<Sqlite>,
    source_fingerprint: &str,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<&'static str> {
    let archive_columns = load_archive_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_time_chunk_query(&archive_columns);
    let mut progress = load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
    reset_legacy_summary_archive_snapshot_progress(pool, candidate, &mut progress).await?;
    let mut total_rows =
        load_summary_archive_snapshot_row_count(pool, candidate, &progress).await?;
    loop {
        if summary_archive_snapshot_backfill_budget_exhausted(started_at, max_elapsed) {
            return Ok("deferred:budget");
        }
        let (rows, has_more) = load_invocation_archive_rows_time_chunk(
            archive_pool,
            &query_sql,
            progress.next_occurred_at.as_deref(),
            progress.next_row_id,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        let (coverage_start, coverage_end) = summary_archive_snapshot_page_coverage(
            candidate,
            &rows,
            has_more,
            progress.page_index,
        )?;
        let invoke_ids_by_row_id =
            load_summary_archive_snapshot_invoke_ids(archive_pool, &archive_columns, &rows).await?;
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
        let next_progress =
            summary_archive_snapshot_page_progress(&progress, &rows, source_fingerprint)?;
        store_summary_archive_snapshot_v2_page(pool, candidate, &page, &next_progress).await?;
        total_rows = total_rows.saturating_add(i64::try_from(rows.len())?);
        progress = next_progress;
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

async fn reset_legacy_summary_archive_snapshot_progress(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    progress: &mut SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    if progress.cursor_version >= SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION
        || (progress.next_row_id == 0 && progress.page_index == 0)
    {
        return Ok(());
    }
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
    Ok(())
}

async fn load_summary_archive_snapshot_row_count(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    progress: &SummaryArchiveSnapshotBackfillProgress,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, Option<i64>>(
        "SELECT SUM(row_count) FROM summary_archive_snapshot
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND page_index < ?3",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(i64::from(progress.page_index))
    .fetch_one(pool)
    .await?
    .unwrap_or_default())
}

fn summary_archive_snapshot_page_coverage(
    candidate: &HistoricalRollupStartupCandidateRow,
    rows: &[InvocationHourlySourceRecord],
    has_more: bool,
    page_index: u32,
) -> Result<(String, String)> {
    let coverage_start = if page_index == 0 {
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
    match (coverage_start, coverage_end) {
        (Some(start), Some(end)) => Ok((start, end)),
        _ => bail!("Summary Snapshot source page is missing coverage"),
    }
}

async fn load_summary_archive_snapshot_invoke_ids(
    archive_pool: &Pool<Sqlite>,
    archive_columns: &HashSet<String>,
    rows: &[InvocationHourlySourceRecord],
) -> Result<HashMap<i64, String>> {
    if !archive_columns.contains("invoke_id") {
        return Ok(HashMap::new());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id, invoke_id FROM codex_invocations WHERE id IN (");
    let mut separated = query.separated(", ");
    for row in rows {
        separated.push_bind(row.id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build_query_as::<(i64, String)>()
        .fetch_all(archive_pool)
        .await?
        .into_iter()
        .collect())
}

fn summary_archive_snapshot_page_progress(
    progress: &SummaryArchiveSnapshotBackfillProgress,
    rows: &[InvocationHourlySourceRecord],
    source_fingerprint: &str,
) -> Result<SummaryArchiveSnapshotBackfillProgress> {
    let last_row = rows
        .last()
        .ok_or_else(|| anyhow!("Summary Snapshot page is missing row id"))?;
    Ok(SummaryArchiveSnapshotBackfillProgress {
        page_index: progress.page_index.saturating_add(1),
        next_occurred_at: Some(last_row.occurred_at.clone()),
        next_row_id: last_row.id,
        cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
        retry_attempt: 0,
        source_fingerprint: Some(source_fingerprint.to_owned()),
        ..progress.clone()
    })
}

async fn store_summary_archive_snapshot_v2_page(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    page: &SummaryArchiveSnapshotPage,
    progress: &SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), page).await?;
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        "in_progress",
        "",
        progress,
    )
    .await?;
    tx.commit().await?;
    Ok(())
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
    let context = load_summary_archive_snapshot_backfill_window_context(
        pool,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
    )
    .await?;
    let candidate_count = context.candidates.len();
    let mut result = SummaryArchiveSnapshotBackfillWindowResult {
        next_cursor_id: context.cursor_id,
        candidate_count,
        recent_candidate_count: context.recent_candidate_count,
        wrapped: context.wrapped,
        pending_obligation_count: context.pending_obligation_count,
        terminal_gap_count: context.terminal_gap_count,
        verified_proof_count: context.verified_proof_count,
        materialized_archive_batches: promoted_page_sets,
        ..SummaryArchiveSnapshotBackfillWindowResult::default()
    };
    process_summary_archive_snapshot_backfill_candidates(
        pool,
        context.candidates,
        context.candidates_from_due_queue,
        started_at,
        max_elapsed,
        context.high_watermark_id,
        &mut result,
    )
    .await?;
    if candidate_count == 0
        && !(context.checkpoint_completed
            && context.cursor_id == 0
            && context.high_watermark_id == context.observed_high_watermark)
    {
        store_summary_archive_snapshot_backfill_checkpoint(
            pool,
            0,
            context.high_watermark_id,
            true,
        )
        .await?;
    }
    Ok(result)
}

struct SummaryArchiveSnapshotBackfillWindowContext {
    candidates: Vec<HistoricalRollupStartupCandidateRow>,
    candidates_from_due_queue: bool,
    cursor_id: i64,
    high_watermark_id: i64,
    observed_high_watermark: i64,
    checkpoint_completed: bool,
    wrapped: bool,
    recent_candidate_count: usize,
    pending_obligation_count: usize,
    terminal_gap_count: usize,
    verified_proof_count: usize,
}

async fn load_summary_archive_snapshot_backfill_window_context(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<SummaryArchiveSnapshotBackfillWindowContext> {
    let (mut cursor_id, checkpoint_high_watermark_id, checkpoint_completed) =
        load_summary_archive_snapshot_backfill_checkpoint(pool).await?;
    let observed_high_watermark = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = ?1",
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?;
    let high_watermark_id = checkpoint_high_watermark_id.max(observed_high_watermark);
    let mut candidates = load_summary_archive_snapshot_backfill_due_candidates(pool, limit).await?;
    let candidates_from_due_queue = !candidates.is_empty();
    if candidates.is_empty() {
        candidates =
            load_summary_archive_snapshot_backfill_candidates(pool, cursor_id, limit).await?;
    }
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_summary_archive_snapshot_backfill_candidates(pool, 0, limit).await?;
        wrapped = !candidates.is_empty();
        if wrapped {
            cursor_id = 0;
        }
    }
    let recent_candidate_count = candidates
        .iter()
        .filter(|candidate| {
            summary_archive_snapshot_candidate_intersects_current_30d(candidate, Utc::now())
        })
        .count();
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
    Ok(SummaryArchiveSnapshotBackfillWindowContext {
        candidates,
        candidates_from_due_queue,
        cursor_id,
        high_watermark_id,
        observed_high_watermark,
        checkpoint_completed,
        wrapped,
        recent_candidate_count,
        pending_obligation_count,
        terminal_gap_count,
        verified_proof_count,
    })
}

async fn process_summary_archive_snapshot_backfill_candidates(
    pool: &Pool<Sqlite>,
    candidates: Vec<HistoricalRollupStartupCandidateRow>,
    candidates_from_due_queue: bool,
    started_at: Instant,
    max_elapsed: Duration,
    high_watermark_id: i64,
    result: &mut SummaryArchiveSnapshotBackfillWindowResult,
) -> Result<()> {
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
    Ok(())
}
