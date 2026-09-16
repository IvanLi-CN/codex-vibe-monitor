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
    BudgetExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStartupLegacyDetailMirrorProof {
    Proven,
    NotMirror,
    BudgetExhausted,
    Unavailable,
}

async fn load_historical_rollup_startup_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            batches.id,
            batches.dataset,
            batches.file_path,
            batches.sha256,
            batches.row_count,
            batches.summary_source_kind,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.status = ?4
          AND batches.id > ?5
          AND (
                (batches.dataset = 'codex_invocations'
                    AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
                    AND (
                    batches.historical_rollups_materialized_at IS NULL
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?1
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?2
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?3
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                ))
                OR (batches.dataset = 'forward_proxy_attempts'
                    AND batches.historical_rollups_materialized_at IS NULL)
          )
        ORDER BY batches.id ASC
        LIMIT ?6
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(cursor_id)
    .bind(STARTUP_HISTORICAL_ROLLUP_CANDIDATE_LIMIT)
    .fetch_all(pool)
    .await
    .context("failed to load historical rollup startup keyset candidates")
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
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    if !inflate_gzip_sqlite_file_with_budget(
        archive_path,
        &temp_path,
        started_at,
        Some(max_elapsed),
    )? {
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
            if live_rows.len() != archive_rows.len() {
                return Ok(LegacyDetailMirrorProof::NotMirror);
            }
            let live_invoke_ids = live_rows.into_iter().collect::<HashMap<_, _>>();
            if archive_rows
                .iter()
                .any(|(id, invoke_id)| live_invoke_ids.get(id) != Some(invoke_id))
            {
                return Ok(LegacyDetailMirrorProof::NotMirror);
            }
            matched_rows += archive_rows.len() as i64;
        }
        Ok(LegacyDetailMirrorProof::Proven)
    }
    .await;
    archive_pool.close().await;
    let proof = proof_result?;
    if proof != LegacyDetailMirrorProof::Proven {
        return Ok(proof);
    }
    if matched_rows != candidate.row_count {
        return Ok(LegacyDetailMirrorProof::NotMirror);
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
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
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
          AND COALESCE(summary_source_kind, 'unknown') = ?2
          AND id > ?3
          AND (?4 IS NULL OR id <= ?4)
        ORDER BY id ASC
        LIMIT ?5
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
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
    )
    .await?;
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_legacy_detail_mirror_recovery_candidates(
            pool,
            0,
            None,
            LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT,
        )
        .await?;
        wrapped = !candidates.is_empty();
    }
    if candidates.is_empty() {
        return Ok(LegacyDetailMirrorRecoveryWindowResult {
            next_cursor_id: 0,
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            hit_budget: false,
            wrapped,
        });
    }

    let started_at = Instant::now();
    let mut next_cursor_id = cursor_id;
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;
    let mut proven_mirrors = Vec::new();
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
            LegacyDetailMirrorProof::BudgetExhausted => {
                hit_budget = true;
                break;
            }
        }
        inspected_path_count += 1;
        next_cursor_id = candidate.id;
    }

    let mut changed_path_count = 0_usize;
    if !proven_mirrors.is_empty() {
        let mut tx = pool.begin().await?;
        for candidate in proven_mirrors {
            changed_path_count += sqlx::query(
                "UPDATE archive_batches SET summary_source_kind = ?1 \
                 WHERE id = ?2 AND status = ?3 AND summary_source_kind = ?4 AND sha256 = ?5",
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
    }

    Ok(LegacyDetailMirrorRecoveryWindowResult {
        next_cursor_id,
        candidate_count: candidates.len(),
        inspected_path_count,
        changed_path_count,
        hit_budget,
        wrapped,
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
    let candidates = load_legacy_detail_mirror_recovery_candidates(
        pool,
        cursor_id,
        Some(high_watermark_id),
        SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
    )
    .await?;
    if candidates.is_empty() {
        return Ok(SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
            next_cursor_id: cursor_id,
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

    let mut next_cursor_id = cursor_id;
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
        next_cursor_id = candidate_id;
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

pub(crate) async fn count_historical_rollup_startup_pending_hint(
    pool: &Pool<Sqlite>,
) -> Result<HistoricalRollupStartupPendingHint> {
    let candidates = load_historical_rollup_startup_candidates(pool, 0).await?;
    let inspected_path_count = candidates.len();
    let pending_archive_batches = candidates
        .iter()
        .filter(|candidate| Path::new(&candidate.file_path).exists())
        .count();

    Ok(HistoricalRollupStartupPendingHint {
        pending_archive_batches,
        candidate_count: candidates.len(),
        inspected_path_count,
    })
}

pub(crate) async fn materialize_historical_rollups_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    max_elapsed: Duration,
) -> Result<HistoricalRollupStartupWindowResult> {
    let mut candidates = load_historical_rollup_startup_candidates(pool, cursor_id).await?;
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_historical_rollup_startup_candidates(pool, 0).await?;
        wrapped = !candidates.is_empty();
    }
    if candidates.is_empty() {
        return Ok(HistoricalRollupStartupWindowResult {
            summary: HistoricalRollupMaterializationSummary::default(),
            next_cursor_id: 0,
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            hit_budget: false,
            wrapped,
        });
    }

    let started_at = Instant::now();
    let mut tx = pool.begin().await?;
    let mut next_cursor_id = cursor_id;
    let mut scanned_archive_batches = 0_usize;
    let mut skipped_archive_batches = 0_usize;
    let mut materialized_archive_batches = 0_usize;
    let mut changed_path_count = 0_usize;
    let mut blocked_archive_batches = 0_usize;
    let mut materialized_invocation_batches = 0_usize;
    let mut materialized_forward_proxy_batches = 0_usize;
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;

    for candidate in candidates
        .iter()
        .take(STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT)
    {
        let candidate_summary = if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
                tx.as_mut(),
                started_at,
                Some(1),
                Some(max_elapsed),
                0,
                vec![candidate.archive_file()],
            )
            .await?
        } else {
            replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
                tx.as_mut(),
                started_at,
                Some(1),
                Some(max_elapsed),
                0,
                vec![candidate.archive_file()],
            )
            .await?
        };
        if candidate_summary.scanned_batches == 0 {
            hit_budget |= candidate_summary.hit_budget;
            break;
        }
        inspected_path_count += candidate_summary.scanned_batches as usize;
        scanned_archive_batches += candidate_summary.scanned_batches as usize;
        skipped_archive_batches += candidate_summary.skipped_batches as usize;
        materialized_archive_batches += candidate_summary.materialized_batches as usize;
        changed_path_count += candidate_summary.changed_batches as usize;
        blocked_archive_batches += candidate_summary.blocked_batches as usize;
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            materialized_invocation_batches += candidate_summary.materialized_batches as usize;
        } else {
            materialized_forward_proxy_batches += candidate_summary.materialized_batches as usize;
        }
        if candidate_summary.hit_budget {
            hit_budget = true;
            if candidate_summary.advance_cursor_after_unstarted_replay {
                next_cursor_id = candidate.id;
            }
            break;
        }
        next_cursor_id = candidate.id;
    }
    tx.commit().await?;

    Ok(HistoricalRollupStartupWindowResult {
        summary: HistoricalRollupMaterializationSummary {
            scanned_archive_batches,
            skipped_archive_batches,
            materialized_archive_batches,
            blocked_archive_batches,
            materialized_bucket_count: 0,
            materialized_invocation_batches,
            materialized_forward_proxy_batches,
            last_materialized_bucket_start_epoch: None,
        },
        next_cursor_id,
        candidate_count: candidates.len(),
        inspected_path_count,
        changed_path_count,
        hit_budget,
        wrapped,
    })
}

#[derive(Debug, FromRow)]
pub(crate) struct LegacyArchivePruneCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    cleanup_state: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) fn estimate_historical_rollup_pending_bucket_count(
    row: &HistoricalRollupPendingArchiveBatchRow,
) -> u64 {
    if let (Some(start), Some(end)) = (&row.coverage_start_at, &row.coverage_end_at)
        && let (Ok(start_local), Ok(end_local)) = (
            parse_shanghai_local_naive(start),
            parse_shanghai_local_naive(end),
        )
    {
        let start_utc = local_naive_to_utc(start_local, Shanghai);
        let end_utc = local_naive_to_utc(end_local, Shanghai);
        let secs = (end_utc.timestamp() - start_utc.timestamp()).max(0);
        return ((secs + 3_599) / 3_600).max(1) as u64;
    }

    let Ok(start_date) = NaiveDate::parse_from_str(&format!("{}-01", row.month_key), "%Y-%m-%d")
    else {
        return 0;
    };
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let Some(next_month_date) = NaiveDate::from_ymd_opt(next_year, next_month, 1) else {
        return 0;
    };
    let Some(start_naive) = start_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let Some(end_naive) = next_month_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let start_utc = local_naive_to_utc(start_naive, Shanghai);
    let end_utc = local_naive_to_utc(end_naive, Shanghai);
    ((end_utc.timestamp() - start_utc.timestamp()).max(0) / 3_600) as u64
}

pub(crate) async fn count_historical_rollup_archive_batches(
    pool: &Pool<Sqlite>,
    pending_only: bool,
) -> Result<i64> {
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM archive_batches WHERE status = ");
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(")");
    if pending_only {
        query.push(" AND historical_rollups_materialized_at IS NULL");
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

pub(crate) fn historical_rollup_materialized_bucket_targets() -> [&'static str; 7] {
    [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
    ]
}

pub(crate) async fn load_latest_materialized_legacy_invocation_rollup_bucket_epoch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<Option<i64>> {
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let latest_coverage_end_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT MAX(coverage_end_at)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND historical_rollups_materialized_at IS NOT NULL
          AND coverage_end_at IS NOT NULL
          AND coverage_end_at < ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(invocation_archive_cutoff)
    .fetch_one(pool)
    .await?;

    Ok(latest_coverage_end_at.and_then(|coverage_end_at| {
        parse_shanghai_local_naive(&coverage_end_at)
            .ok()
            .and_then(|naive| {
                let bucket_start_epoch =
                    align_bucket_epoch(local_naive_to_utc(naive, Shanghai).timestamp(), 3_600, 0);
                Utc.timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .map(|_| bucket_start_epoch)
            })
    }))
}

pub(crate) async fn count_materialized_historical_rollup_buckets(
    pool: &Pool<Sqlite>,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target IN (",
    );
    {
        let mut separated = query.separated(", ");
        for target in historical_rollup_materialized_bucket_targets() {
            separated.push_bind(target);
        }
    }
    query.push(")");
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}
