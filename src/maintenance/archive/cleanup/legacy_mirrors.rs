use super::*;
use futures_util::{StreamExt, stream};

const LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT: i64 = 128;
const LEGACY_DETAIL_MIRROR_IDENTITY_PAGE_SIZE: i64 = 400;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT: i64 = 512;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_PROOF_CONCURRENCY: usize = 4;

#[derive(Debug)]
pub(crate) struct LegacyDetailMirrorRecoveryWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}

#[derive(Debug)]
pub(crate) struct SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) unavailable_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyDetailMirrorProof {
    Proven,
    NotMirror,
    Ambiguous,
    BudgetExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStartupLegacyDetailMirrorProof {
    Proven,
    NotMirror,
    BudgetExhausted,
    Unavailable,
}

async fn legacy_invocation_archive_is_live_detail_mirror(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<LegacyDetailMirrorProof> {
    if candidate.row_count < 0 {
        return Ok(LegacyDetailMirrorProof::NotMirror);
    }
    let archive_path = Path::new(&candidate.file_path);
    let Some(sha256_before_open) =
        legacy_detail_mirror_sha256_with_budget(archive_path, started_at, max_elapsed)?
    else {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    if sha256_before_open != candidate.sha256 {
        return Ok(LegacyDetailMirrorProof::NotMirror);
    }
    if started_at.elapsed() >= max_elapsed {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    }

    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    if !inflate_gzip_sqlite_file_with_budget(
        archive_path,
        &temp_path,
        started_at,
        Some(max_elapsed),
    )
    .await?
    {
        drop(temp_cleanup);
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    }
    if started_at.elapsed() >= max_elapsed {
        drop(temp_cleanup);
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| {
            format!(
                "failed to open legacy detail archive {}",
                archive_path.display()
            )
        })?;

    let mut after_id = i64::MIN;
    let mut matched_rows = 0_i64;
    let proof_result: Result<LegacyDetailMirrorProof> = async {
        loop {
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            let archive_rows = sqlx::query_as::<_, (i64, String)>(
                "SELECT id, invoke_id FROM codex_invocations WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
            )
            .bind(after_id)
            .bind(LEGACY_DETAIL_MIRROR_IDENTITY_PAGE_SIZE)
            .fetch_all(&archive_pool)
            .await
            .with_context(|| {
                format!(
                    "failed to read legacy detail archive identities for archive batch {}",
                    candidate.id
                )
            })?;
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            let Some(last_id) = archive_rows.last().map(|(id, _)| *id) else {
                break;
            };
            after_id = last_id;

            let mut live_query = QueryBuilder::<Sqlite>::new(
                "SELECT id, invoke_id FROM codex_invocations WHERE id IN (",
            );
            {
                let mut ids = live_query.separated(", ");
                for (id, _) in &archive_rows {
                    ids.push_bind(id);
                }
            }
            live_query.push(")");
            let live_rows = live_query
                .build_query_as::<(i64, String)>()
                .fetch_all(pool)
                .await
                .context("failed to read live identities for legacy detail mirror recovery")?;
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            let live_row_count = live_rows.len();
            let live_invoke_ids = live_rows.into_iter().collect::<HashMap<_, _>>();
            let matched_in_page = archive_rows
                .iter()
                .filter(|(id, invoke_id)| live_invoke_ids.get(id) == Some(invoke_id))
                .count() as i64;
            if live_row_count != archive_rows.len()
                || matched_in_page != archive_rows.len() as i64
            {
                return Ok(if matched_rows + matched_in_page > 0 {
                    LegacyDetailMirrorProof::Ambiguous
                } else {
                    LegacyDetailMirrorProof::NotMirror
                });
            }
            matched_rows += matched_in_page;
        }
        Ok(LegacyDetailMirrorProof::Proven)
    }
    .await;
    archive_pool.close().await;
    let proof = proof_result?;
    if !matches!(proof, LegacyDetailMirrorProof::Proven) {
        return Ok(proof);
    }
    if matched_rows != candidate.row_count {
        return Ok(if matched_rows > 0 {
            LegacyDetailMirrorProof::Ambiguous
        } else {
            LegacyDetailMirrorProof::NotMirror
        });
    }
    let Some(sha256_after_read) =
        legacy_detail_mirror_sha256_with_budget(archive_path, started_at, max_elapsed)?
    else {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    Ok(if sha256_after_read == candidate.sha256 {
        LegacyDetailMirrorProof::Proven
    } else {
        LegacyDetailMirrorProof::NotMirror
    })
}

fn legacy_detail_mirror_sha256_with_budget(
    path: &Path,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<Option<String>> {
    if started_at.elapsed() >= max_elapsed {
        return Ok(None);
    }
    let mut file = fs::File::open(path).with_context(|| {
        format!(
            "failed to open legacy detail archive for sha256 {}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).with_context(|| {
            format!(
                "failed to read legacy detail archive for sha256 {}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if started_at.elapsed() >= max_elapsed {
            return Ok(None);
        }
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

async fn load_legacy_detail_mirror_recovery_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    high_watermark_id: Option<i64>,
    candidate_limit: i64,
    include_authoritative_legacy_month: bool,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    let role_filter = if include_authoritative_legacy_month {
        "(COALESCE(summary_source_kind, 'unknown') = 'unknown' OR (summary_source_kind = 'authoritative' AND layout = 'legacy_month'))"
    } else {
        "COALESCE(summary_source_kind, 'unknown') = 'unknown'"
    };
    let query = format!(
        r#"
        SELECT
            id,
            dataset,
            file_path,
            sha256,
            row_count,
            summary_source_kind,
            coverage_start_at,
            coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
          AND {role_filter}
          AND id > ?2
          AND (?3 IS NULL OR id <= ?3)
        ORDER BY id ASC
        LIMIT ?4
        "#,
    );
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(&query)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(cursor_id)
        .bind(high_watermark_id)
        .bind(candidate_limit)
        .fetch_all(pool)
        .await
        .context("failed to load legacy detail mirror recovery candidates")
}

pub(crate) async fn summary_startup_legacy_detail_mirror_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = ?1 AND status = ?2 AND COALESCE(summary_source_kind, 'unknown') = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .fetch_one(pool)
    .await
    .context("failed to load summary startup legacy detail mirror high-watermark")
}

async fn update_summary_startup_proven_legacy_detail_mirrors(
    pool: &Pool<Sqlite>,
    proven_mirrors: Vec<HistoricalRollupStartupCandidateRow>,
) -> Result<usize> {
    if proven_mirrors.is_empty() {
        return Ok(0);
    }

    let Some(admission) = super::super::retention::acquire_retention_write_admission(
        "summary_legacy_detail_mirror_reconcile",
    )
    .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "summary_legacy_detail_mirror_reconcile",
        ));
    };
    let mut changed_path_count = 0_usize;
    let mut tx = pool.begin().await?;
    for candidate in proven_mirrors {
        changed_path_count += sqlx::query(
            "UPDATE archive_batches SET summary_source_kind = ?1 \
             WHERE id = ?2 AND status = ?3 \
               AND COALESCE(summary_source_kind, 'unknown') = ?4 AND sha256 = ?5",
        )
        .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
        .bind(candidate.id)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
        .bind(&candidate.sha256)
        .execute(tx.as_mut())
        .await?
        .rows_affected() as usize;
    }
    tx.commit().await?;
    drop(admission);
    Ok(changed_path_count)
}

pub(crate) async fn reconcile_legacy_detail_mirrors_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    max_elapsed: Duration,
) -> Result<LegacyDetailMirrorRecoveryWindowResult> {
    let mut candidates = load_legacy_detail_mirror_recovery_candidates(
        pool,
        cursor_id,
        None,
        LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT,
        true,
    )
    .await?;
    let skipped_cursor_id = candidates
        .iter()
        .filter(|candidate| {
            Path::new(&candidate.file_path)
                .parent()
                .is_none_or(|parent| !parent.exists())
        })
        .map(|candidate| candidate.id)
        .max();
    candidates.retain(|candidate| {
        Path::new(&candidate.file_path)
            .parent()
            .is_some_and(Path::exists)
    });
    if candidates.is_empty() {
        return Ok(LegacyDetailMirrorRecoveryWindowResult {
            // Keep the completed-cycle cursor so an idle pass does not immediately scan the
            // same archive identities again.
            next_cursor_id: skipped_cursor_id.unwrap_or(cursor_id),
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            hit_budget: false,
            wrapped: false,
        });
    }

    let started_at = Instant::now();
    let mut next_cursor_id = skipped_cursor_id.unwrap_or(cursor_id);
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;
    let mut proven_mirrors = Vec::new();
    let mut ambiguous_mirrors = Vec::new();
    for candidate in candidates.iter() {
        if started_at.elapsed() >= max_elapsed {
            hit_budget = true;
            break;
        }
        match legacy_invocation_archive_is_live_detail_mirror(
            pool,
            candidate,
            started_at,
            max_elapsed,
        )
        .await?
        {
            LegacyDetailMirrorProof::Proven => proven_mirrors.push(candidate),
            LegacyDetailMirrorProof::NotMirror => {}
            LegacyDetailMirrorProof::Ambiguous => ambiguous_mirrors.push(candidate),
            LegacyDetailMirrorProof::BudgetExhausted => {
                hit_budget = true;
                break;
            }
        }
        inspected_path_count += 1;
        next_cursor_id = next_cursor_id.max(candidate.id);
    }

    let mut changed_path_count = 0_usize;
    if !proven_mirrors.is_empty() || !ambiguous_mirrors.is_empty() {
        // The startup-backfill caller holds the P2 SQLite write permit before entering this stage.
        let mut tx = pool.begin().await?;
        for candidate in proven_mirrors {
            changed_path_count += sqlx::query(
                "UPDATE archive_batches SET summary_source_kind = ?1 \
                 WHERE id = ?2 AND status = ?3 \
                   AND (summary_source_kind = ?4 OR summary_source_kind = ?5) AND sha256 = ?6",
            )
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
            .bind(candidate.id)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE)
            .bind(&candidate.sha256)
            .execute(tx.as_mut())
            .await?
            .rows_affected() as usize;
        }
        for candidate in ambiguous_mirrors {
            changed_path_count += sqlx::query(
                "UPDATE archive_batches SET summary_source_kind = ?1 \
                 WHERE id = ?2 AND status = ?3 AND summary_source_kind = ?4 AND sha256 = ?5",
            )
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
            .bind(candidate.id)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE)
            .bind(&candidate.sha256)
            .execute(tx.as_mut())
            .await?
            .rows_affected() as usize;
        }
        tx.commit().await?;
    }

    Ok(LegacyDetailMirrorRecoveryWindowResult {
        next_cursor_id,
        candidate_count: candidates.len(),
        inspected_path_count,
        changed_path_count,
        hit_budget,
        wrapped: false,
    })
}

/// Reconciles one high-throughput, finite portion of the cold Summary startup source snapshot.
///
/// A failed identity read stays `unknown`: it remains conservative source evidence, but must not
/// stop later, independently provable mirrors from leaving Summary archive admission. A
/// budget-exhausted proof does not advance its cursor, so the next bounded window retries that
/// same record instead of silently skipping it.
pub(crate) async fn reconcile_legacy_detail_mirrors_for_summary_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    high_watermark_id: i64,
    max_elapsed: Duration,
) -> Result<SummaryStartupLegacyDetailMirrorRecoveryWindowResult> {
    let mut candidates = load_legacy_detail_mirror_recovery_candidates(
        pool,
        cursor_id,
        Some(high_watermark_id),
        SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
        false,
    )
    .await?;
    let skipped_cursor_id = candidates
        .iter()
        .filter(|candidate| {
            Path::new(&candidate.file_path)
                .parent()
                .is_none_or(|parent| !parent.exists())
        })
        .map(|candidate| candidate.id)
        .max();
    candidates.retain(|candidate| {
        Path::new(&candidate.file_path)
            .parent()
            .is_some_and(Path::exists)
    });
    if candidates.is_empty() {
        return Ok(SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
            next_cursor_id: skipped_cursor_id.unwrap_or(cursor_id),
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            unavailable_path_count: 0,
            hit_budget: false,
            completed: true,
        });
    }

    let started_at = Instant::now();
    let candidate_count = candidates.len();
    let mut proof_results = stream::iter(candidates.into_iter().enumerate().map(
        |(index, candidate)| async move {
            let proof = match legacy_invocation_archive_is_live_detail_mirror(
                pool,
                &candidate,
                started_at,
                max_elapsed,
            )
            .await
            {
                Ok(LegacyDetailMirrorProof::Proven) => {
                    SummaryStartupLegacyDetailMirrorProof::Proven
                }
                Ok(LegacyDetailMirrorProof::NotMirror) => {
                    SummaryStartupLegacyDetailMirrorProof::NotMirror
                }
                Ok(LegacyDetailMirrorProof::Ambiguous) => {
                    SummaryStartupLegacyDetailMirrorProof::NotMirror
                }
                Ok(LegacyDetailMirrorProof::BudgetExhausted) => {
                    SummaryStartupLegacyDetailMirrorProof::BudgetExhausted
                }
                // Missing, unreadable, corrupt, or concurrently replaced files remain
                // fail-closed unknown sources. The next exact Summary build will account for
                // their coverage rather than accepting a guessed mirror classification.
                Err(_) => SummaryStartupLegacyDetailMirrorProof::Unavailable,
            };
            (index, candidate, proof)
        },
    ))
    .buffer_unordered(SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_PROOF_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    proof_results.sort_unstable_by_key(|(index, _, _)| *index);

    let mut next_cursor_id = skipped_cursor_id.unwrap_or(cursor_id);
    let mut inspected_path_count = 0_usize;
    let mut unavailable_path_count = 0_usize;
    let mut hit_budget = false;
    let mut proven_mirrors = Vec::new();
    for (_, candidate, proof) in proof_results {
        let candidate_id = candidate.id;
        match proof {
            SummaryStartupLegacyDetailMirrorProof::Proven => proven_mirrors.push(candidate),
            SummaryStartupLegacyDetailMirrorProof::NotMirror => {}
            SummaryStartupLegacyDetailMirrorProof::Unavailable => unavailable_path_count += 1,
            SummaryStartupLegacyDetailMirrorProof::BudgetExhausted => {
                hit_budget = true;
                break;
            }
        }
        inspected_path_count += 1;
        next_cursor_id = next_cursor_id.max(candidate_id);
    }

    let changed_path_count =
        update_summary_startup_proven_legacy_detail_mirrors(pool, proven_mirrors).await?;
    Ok(SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
        next_cursor_id,
        candidate_count,
        inspected_path_count,
        changed_path_count,
        unavailable_path_count,
        hit_budget,
        completed: !hit_budget
            && candidate_count < SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT as usize,
    })
}
