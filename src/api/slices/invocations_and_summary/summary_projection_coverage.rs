use super::*;
pub(crate) const SUMMARY_ALL_TIME_COVERAGE_SCOPE_GLOBAL: &str = "global";
pub(crate) const SUMMARY_ALL_TIME_COVERAGE_SCOPE_ACCOUNT: &str = "account";

#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub(crate) struct SummaryAllTimeCoverageCheckpointRow {
    pub(crate) manifest_high_watermark_id: i64,
    pub(crate) next_manifest_id: i64,
    pub(crate) completed: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryAllTimeCoverageProgress {
    pub(crate) global_complete: bool,
    pub(crate) account_complete: bool,
}

impl SummaryAllTimeCoverageProgress {
    pub(crate) const fn is_complete(self) -> bool {
        self.global_complete && self.account_complete
    }
}

pub(crate) async fn load_summary_all_time_coverage_checkpoint(
    pool: &Pool<Sqlite>,
    scope: &str,
) -> Result<Option<SummaryAllTimeCoverageCheckpointRow>> {
    sqlx::query_as::<_, SummaryAllTimeCoverageCheckpointRow>(
        "SELECT manifest_high_watermark_id, next_manifest_id, completed \
         FROM summary_all_time_coverage_checkpoint WHERE scope = ?1",
    )
    .bind(scope)
    .fetch_optional(pool)
    .await
    .context("summary all-time coverage checkpoint hydration failed")
}

pub(crate) async fn store_summary_all_time_coverage_checkpoint(
    pool: &Pool<Sqlite>,
    scope: &str,
    manifest_high_watermark_id: i64,
    next_manifest_id: i64,
    completed: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_all_time_coverage_checkpoint \
         (scope, manifest_high_watermark_id, next_manifest_id, completed, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
           manifest_high_watermark_id = excluded.manifest_high_watermark_id, \
           next_manifest_id = excluded.next_manifest_id, \
           completed = excluded.completed, updated_at = excluded.updated_at",
    )
    .bind(scope)
    .bind(manifest_high_watermark_id)
    .bind(next_manifest_id)
    .bind(i64::from(completed))
    .execute(pool)
    .await
    .context("summary all-time coverage checkpoint persistence failed")?;
    Ok(())
}

pub(crate) async fn summary_all_time_coverage_page_is_exact(
    pool: &Pool<Sqlite>,
    page: &SummaryProjectionBoundaryManifestPage,
) -> Result<(bool, bool)> {
    if page.archives.is_empty() {
        return Ok((true, true));
    }
    let paths = page
        .archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
    // Account manifest admission is an independent proof.  A high-cardinality account
    // manifest must only make account-scoped all-time unavailable; global replay/rollup proof
    // can still advance over the same immutable archive page.
    let account_manifest_overflow =
        match load_summary_projection_archive_manifest_account_sets(pool, &paths).await {
            Ok(account_ids_by_file) => Some(account_ids_by_file),
            Err(error)
                if error.to_string().starts_with(
                    "summary projection archive account manifest exceeded bounded row budget",
                ) =>
            {
                None
            }
            Err(error) => {
                return Err(error
                    .context("summary all-time coverage account manifest proof hydration failed"));
            }
        };
    // Probe the page for existing final-proof markers in one read.  Calling
    // `summary_archive_snapshot_path_has_proof` for every manifest would open a read transaction
    // and, when no marker exists, a failed `BEGIN IMMEDIATE` promotion attempt for each row.  A
    // large staged recovery can contain thousands of legacy manifests without any V2 pages; the
    // page-level probe keeps that common path bounded while marked identities still receive the
    // full semantic validation below.
    let proof_marked_paths = {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT DISTINCT batches.file_path \
             FROM archive_batches AS batches \
             INNER JOIN summary_archive_snapshot_v2_proof AS proof \
               ON proof.archive_batch_id = batches.id \
              AND proof.manifest_sha256 = batches.sha256 \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.file_path IN (",
        );
        let mut separated = query.separated(", ");
        for path in &paths {
            separated.push_bind(path);
        }
        separated.push_unseparated(")");
        query
            .build_query_scalar::<String>()
            .fetch_all(pool)
            .await
            .context("summary all-time coverage page proof marker lookup failed")?
            .into_iter()
            .collect::<HashSet<_>>()
    };
    let mut global_complete = true;
    let mut account_complete = true;
    for archive in &page.archives {
        let replay = replay_coverage
            .get(archive.file_path())
            .copied()
            .unwrap_or_default();
        let account_ids = account_manifest_overflow
            .as_ref()
            .and_then(|account_ids_by_file| account_ids_by_file.get(archive.file_path()));
        // A verified Snapshot V2 page set is a complete normalized Summary source. It can
        // replace both compact rollup and account-manifest proof when the raw archive is gone,
        // allowing the historical supervisor to advance without reopening the source file.
        if proof_marked_paths.contains(archive.file_path())
            && summary_archive_snapshot_path_has_proof(pool, archive.file_path()).await?
        {
            continue;
        }
        let (archive_global_complete, archive_account_complete) =
            if let Some(range) = summary_projection_archive_coverage_range(archive) {
                let rollup_range = (
                    align_bucket_epoch(range.start.timestamp(), 3_600, 0),
                    align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0)
                        .saturating_add(3_600),
                );
                let (hourly_rollup_totals, _) =
                    load_summary_projection_rollup_totals_in_range(pool, Some(rollup_range))
                        .await
                        .context("summary all-time coverage rollup key proof hydration failed")?;
                let (global_complete, mut account_complete) =
                    summary_projection_all_time_manifest_scope_coverage(
                        archive,
                        replay,
                        page.account_manifest_refreshed_paths
                            .contains(archive.file_path()),
                        account_ids.unwrap_or(&HashSet::new()),
                        &hourly_rollup_totals,
                    )?;
                if account_ids.is_none() {
                    account_complete = false;
                }
                (global_complete, account_complete)
            } else {
                // Legacy manifests have no finite range. A replay marker is still required, but
                // there are no concrete bucket keys to validate in this bounded proof page.
                (
                    // Summary's overall replay marker is the exact compact invocation proof
                    // produced by the bounded repair path, even before the broader historical
                    // rollup materialization flag is finalized.
                    replay.overall && replay.usage_breakdown,
                    archive.has_materialized_historical_rollups()
                        && account_ids.is_some()
                        && page
                            .account_manifest_refreshed_paths
                            .contains(archive.file_path())
                        && replay.account_stats
                        && replay.usage_breakdown,
                )
            };
        global_complete &= archive_global_complete;
        account_complete &= archive_account_complete;
        if !global_complete && !account_complete {
            break;
        }
    }
    Ok((global_complete, account_complete))
}

pub(crate) async fn summary_archive_snapshot_path_has_proof(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<bool> {
    let Some((archive_batch_id, manifest_sha256)) = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, sha256 FROM archive_batches
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
           AND file_path = ?1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(file_path)
    .fetch_optional(pool)
    .await
    .context("summary Snapshot V2 manifest identity lookup failed")?
    else {
        return Ok(false);
    };
    summary_archive_snapshot_has_final_proof_read_only(pool, archive_batch_id, &manifest_sha256)
        .await
}

/// A complete V2 proof set is stronger than the per-page replay/rollup proof used by the
/// legacy checkpoint. Once every manifest below this fence has that authority, walking the same
/// seek cursor again cannot discover new work. Complete the scope atomically instead of making
/// `all` wait for thousands of already-verified manifest pages.
pub(crate) async fn summary_all_time_manifest_v2_coverage_complete(
    pool: &Pool<Sqlite>,
    high_watermark_id: i64,
) -> Result<bool> {
    // Avoid the correlated page/row-count validation when the proof table cannot possibly cover
    // the admitted manifest set. This is the common staged-recovery path: a bounded aggregate
    // keeps the final empty-page check cheap, while the detailed validation below still rejects
    // forged markers once every manifest advertises a matching proof identity.
    let (manifest_count, proof_count) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE((
             SELECT COUNT(*)
             FROM summary_archive_snapshot_v2_proof AS proof
             INNER JOIN archive_batches AS proof_batches
               ON proof_batches.id = proof.archive_batch_id
              AND proof_batches.sha256 = proof.manifest_sha256
             WHERE proof_batches.dataset = 'codex_invocations'
               AND proof_batches.status = 'completed'
               AND COALESCE(proof_batches.summary_source_kind, 'unknown') <> 'live_mirror'
               AND proof_batches.id <= ?1
         ), 0)
         FROM archive_batches AS batches
         WHERE batches.dataset = 'codex_invocations'
           AND batches.status = 'completed'
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
           AND batches.id <= ?1",
    )
    .bind(high_watermark_id)
    .fetch_one(pool)
    .await
    .context("summary all-time Snapshot V2 coverage count check failed")?;
    if proof_count != manifest_count {
        return Ok(false);
    }
    let duplicate_paths = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT file_path FROM archive_batches
             WHERE dataset = 'codex_invocations' AND status = 'completed'
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
             GROUP BY file_path HAVING COUNT(*) > 1
         )",
    )
    .fetch_one(pool)
    .await
    .context("summary all-time Snapshot V2 duplicate manifest check failed")?;
    if duplicate_paths > 0 {
        return Ok(false);
    }
    // The marker/count aggregate keeps the normal recovery pass bounded. Only when every
    // candidate advertises a complete marker do we pay the one-time full semantic validation
    // needed to reject forged or stale markers.
    let incomplete_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
               WHERE proof.archive_batch_id = batches.id \
                 AND proof.manifest_sha256 = batches.sha256 \
                 AND proof.page_count > 0 \
                 AND proof.page_count = ( \
                     SELECT COUNT(*) FROM summary_archive_snapshot AS pages \
                     WHERE pages.archive_batch_id = batches.id \
                       AND pages.manifest_sha256 = batches.sha256 \
                 ) \
                 AND proof.row_count = ( \
                     SELECT COALESCE(SUM(pages.row_count), 0) \
                     FROM summary_archive_snapshot AS pages \
                     WHERE pages.archive_batch_id = batches.id \
                       AND pages.manifest_sha256 = batches.sha256 \
                 ) \
                 AND LENGTH(TRIM(proof.semantic_sha256)) > 0 \
           )",
    )
    .bind(high_watermark_id)
    .fetch_one(pool)
    .await
    .context("summary all-time Snapshot V2 coverage marker check failed")?;
    if incomplete_count > 0 {
        return Ok(false);
    }
    let manifests = sqlx::query_as::<_, (i64, String)>(
        "SELECT batches.id, batches.sha256 FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1",
    )
    .bind(high_watermark_id)
    .fetch_all(pool)
    .await
    .context("summary all-time Snapshot V2 coverage manifest lookup failed")?;
    for (archive_batch_id, manifest_sha256) in manifests {
        if !summary_archive_snapshot_has_final_proof_read_only(
            pool,
            archive_batch_id,
            &manifest_sha256,
        )
            .await
            .with_context(|| {
                format!(
                    "summary all-time Snapshot V2 final proof check failed for archive {archive_batch_id}"
                )
            })?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) async fn advance_summary_all_time_coverage_checkpoint_scope(
    pool: &Pool<Sqlite>,
    scope: &str,
    high_watermark_id: i64,
) -> Result<bool> {
    let checkpoint = load_summary_all_time_coverage_checkpoint(pool, scope).await?;
    let (next_manifest_id, completed) = match checkpoint {
        Some(checkpoint)
            if checkpoint.manifest_high_watermark_id == high_watermark_id
                && checkpoint.completed != 0 =>
        {
            return Ok(true);
        }
        Some(checkpoint) if checkpoint.manifest_high_watermark_id == high_watermark_id => {
            (checkpoint.next_manifest_id, false)
        }
        _ => (0, false),
    };
    if completed {
        return Ok(true);
    }
    let page = load_summary_projection_boundary_manifest_page(
        pool,
        None,
        (next_manifest_id > 0).then_some(next_manifest_id),
        high_watermark_id,
    )
    .await?;
    if page.archives.is_empty() {
        store_summary_all_time_coverage_checkpoint(
            pool,
            scope,
            high_watermark_id,
            next_manifest_id,
            true,
        )
        .await?;
        debug!(scope, "summary all-time coverage checkpoint completed");
        return Ok(true);
    }
    let (global_complete, account_complete) =
        summary_all_time_coverage_page_is_exact(pool, &page).await?;
    let page_complete = match scope {
        SUMMARY_ALL_TIME_COVERAGE_SCOPE_GLOBAL => global_complete,
        SUMMARY_ALL_TIME_COVERAGE_SCOPE_ACCOUNT => account_complete,
        _ => {
            return Err(anyhow!(
                "unknown summary all-time coverage checkpoint scope"
            ));
        }
    };
    if !page_complete {
        debug!(
            scope,
            manifest_count = page.archives.len(),
            "summary all-time coverage checkpoint waiting for exact page proof"
        );
        return Ok(false);
    }
    let next_after_id = page
        .next_after_id
        .expect("non-empty summary all-time coverage page has a seek cursor");
    store_summary_all_time_coverage_checkpoint(
        pool,
        scope,
        high_watermark_id,
        next_after_id,
        false,
    )
    .await?;
    debug!(
        scope,
        manifest_count = page.archives.len(),
        "summary all-time coverage checkpoint advanced"
    );
    Ok(false)
}

pub(crate) async fn advance_summary_all_time_coverage_checkpoint(
    pool: &Pool<Sqlite>,
) -> Result<SummaryAllTimeCoverageProgress> {
    let manifest_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(pool)
    .await
    .context("summary all-time coverage checkpoint count failed")?;
    if manifest_count <= SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES as i64 {
        return Ok(SummaryAllTimeCoverageProgress {
            global_complete: true,
            account_complete: true,
        });
    }
    let Some(high_watermark_id) =
        summary_projection_completed_manifest_high_watermark(pool).await?
    else {
        return Ok(SummaryAllTimeCoverageProgress {
            global_complete: true,
            account_complete: true,
        });
    };
    let global_complete = advance_summary_all_time_coverage_checkpoint_scope(
        pool,
        SUMMARY_ALL_TIME_COVERAGE_SCOPE_GLOBAL,
        high_watermark_id,
    )
    .await?;
    let account_complete = advance_summary_all_time_coverage_checkpoint_scope(
        pool,
        SUMMARY_ALL_TIME_COVERAGE_SCOPE_ACCOUNT,
        high_watermark_id,
    )
    .await?;
    Ok(SummaryAllTimeCoverageProgress {
        global_complete,
        account_complete,
    })
}

pub(crate) const SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE: &str = "all";
pub(crate) const SUMMARY_PROJECTION_ALL_TIME_ROLLUP_PAGE_SIZE: i64 = 512;
// Coverage proof pages are deliberately smaller than the normal manifest hydration chunk. Each
// archive range is checked against its concrete compact rollup keys, keeping proof memory and
// query work bounded even when thousands of manifests share one retention horizon.
pub(crate) const SUMMARY_PROJECTION_ALL_TIME_MANIFEST_PROOF_PAGE_SIZE: usize = 32;

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(crate) struct SummaryAllTimeProjectionCheckpointRow {
    pub(crate) live_high_watermark_id: i64,
    pub(crate) rollup_live_cursor: i64,
    pub(crate) account_rollup_live_cursor: Option<i64>,
    pub(crate) manifest_high_watermark_id: Option<i64>,
    pub(crate) coverage_revision: i64,
    pub(crate) account_coverage_revision: i64,
    pub(crate) durable_terminal_sequence_watermark: i64,
    pub(crate) global_manifest_next_id: i64,
    pub(crate) account_manifest_next_id: i64,
    pub(crate) global_manifest_complete: i64,
    pub(crate) account_manifest_complete: i64,
    pub(crate) global_rollup_next_rowid: i64,
    pub(crate) account_rollup_next_rowid: i64,
    pub(crate) usage_rollup_next_rowid: i64,
    pub(crate) global_rollup_complete: i64,
    pub(crate) account_rollup_complete: i64,
    pub(crate) usage_rollup_complete: i64,
    pub(crate) account_unavailable: i64,
    pub(crate) global_usage_unavailable: i64,
    pub(crate) account_usage_unavailable: i64,
    pub(crate) global_total_count: i64,
    pub(crate) global_success_count: i64,
    pub(crate) global_failure_count: i64,
    pub(crate) global_total_tokens: i64,
    pub(crate) global_total_cost: f64,
    pub(crate) global_non_success_cost: f64,
}

impl SummaryAllTimeProjectionCheckpointRow {
    pub(crate) fn generation_fence(&self) -> SummaryProjectionGenerationFence {
        SummaryProjectionGenerationFence {
            live_high_watermark_id: self.live_high_watermark_id,
            rollup_live_cursor: self.rollup_live_cursor,
            account_rollup_live_cursor: self.account_rollup_live_cursor,
            completed_manifest_high_watermark_id: self.manifest_high_watermark_id,
            coverage_revision: self.coverage_revision,
            account_coverage_revision: self.account_coverage_revision,
            durable_terminal_sequence_watermark: self.durable_terminal_sequence_watermark.max(0)
                as u64,
        }
    }

    pub(crate) fn global_ready(&self) -> bool {
        self.global_manifest_complete != 0
            && self.global_rollup_complete != 0
            && self.usage_rollup_complete != 0
            && self.global_usage_unavailable == 0
    }

    pub(crate) fn account_ready(&self) -> bool {
        self.account_unavailable == 0
            && self.account_manifest_complete != 0
            && self.account_rollup_complete != 0
            && self.usage_rollup_complete != 0
            && self.account_usage_unavailable == 0
    }

    // A terminal unavailable scope is still settled for the current source fence. It must stay
    // range-local unavailable, but it must not keep reopening the same durable checkpoint page.
    pub(crate) fn global_settled(&self) -> bool {
        self.global_ready()
            || (self.global_manifest_complete != 0
                && self.global_rollup_complete != 0
                && self.usage_rollup_complete != 0
                && self.global_usage_unavailable != 0)
    }

    pub(crate) fn account_settled(&self) -> bool {
        self.account_ready()
            || (self.account_manifest_complete != 0
                && self.account_rollup_complete != 0
                && self.usage_rollup_complete != 0
                && (self.account_unavailable != 0 || self.account_usage_unavailable != 0))
    }

    pub(crate) fn global_totals(&self) -> StatsTotals {
        StatsTotals {
            total_count: self.global_total_count,
            success_count: self.global_success_count,
            failure_count: self.global_failure_count,
            total_tokens: self.global_total_tokens,
            total_cost: self.global_total_cost,
            non_success_cost: self.global_non_success_cost,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryAllTimeCheckpointGlobalRollupRow {
    pub(crate) rowid: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
}

impl SummaryAllTimeCheckpointGlobalRollupRow {
    pub(crate) fn totals(&self) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_tokens: self.total_tokens,
            total_cost: self.total_cost,
            non_success_cost: self.non_success_cost,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryAllTimeCheckpointAccountRollupRow {
    pub(crate) rowid: i64,
    pub(crate) upstream_account_id: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
}

impl SummaryAllTimeCheckpointAccountRollupRow {
    pub(crate) fn totals(&self) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_tokens: self.total_tokens,
            total_cost: self.total_cost,
            non_success_cost: self.non_success_cost,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryAllTimeCheckpointAccountTotalsRow {
    pub(crate) upstream_account_id: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
}

impl SummaryAllTimeCheckpointAccountTotalsRow {
    pub(crate) fn totals(self) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_tokens: self.total_tokens,
            total_cost: self.total_cost,
            non_success_cost: self.non_success_cost,
        }
    }
}

pub(crate) async fn summary_projection_all_time_staged_recovery_required(
    pool: &Pool<Sqlite>,
) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(pool)
    .await
    .context("summary all-time staged recovery admission count failed")?;
    Ok(count > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES as i64)
}

pub(crate) async fn load_summary_all_time_projection_checkpoint(
    pool: &Pool<Sqlite>,
) -> Result<Option<SummaryAllTimeProjectionCheckpointRow>> {
    sqlx::query_as::<_, SummaryAllTimeProjectionCheckpointRow>(
        "SELECT live_high_watermark_id, rollup_live_cursor, account_rollup_live_cursor, \
                manifest_high_watermark_id, coverage_revision, account_coverage_revision, durable_terminal_sequence_watermark, \
                global_manifest_next_id, account_manifest_next_id, global_manifest_complete, \
                account_manifest_complete, global_rollup_next_rowid, account_rollup_next_rowid, \
                usage_rollup_next_rowid, global_rollup_complete, account_rollup_complete, \
                usage_rollup_complete, global_usage_unavailable, account_usage_unavailable, account_unavailable, \
                global_total_count, global_success_count, global_failure_count, global_total_tokens, \
                global_total_cost, global_non_success_cost \
         FROM summary_all_time_projection_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .fetch_optional(pool)
    .await
    .context("summary all-time projection checkpoint hydration failed")
}

pub(crate) async fn reset_summary_all_time_projection_checkpoint(
    pool: &Pool<Sqlite>,
    generation_fence: SummaryProjectionGenerationFence,
) -> Result<SummaryAllTimeProjectionCheckpointRow> {
    let mut transaction = pool
        .begin()
        .await
        .context("summary all-time projection checkpoint reset transaction failed")?;
    sqlx::query("DELETE FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1")
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account checkpoint reset failed")?;
    sqlx::query("DELETE FROM summary_all_time_projection_usage_checkpoint WHERE scope = ?1")
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection usage checkpoint reset failed")?;
    sqlx::query(
        "INSERT INTO summary_all_time_projection_checkpoint \
         (scope, live_high_watermark_id, rollup_live_cursor, account_rollup_live_cursor, \
          manifest_high_watermark_id, coverage_revision, account_coverage_revision, durable_terminal_sequence_watermark) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(scope) DO UPDATE SET \
           live_high_watermark_id = excluded.live_high_watermark_id, \
           rollup_live_cursor = excluded.rollup_live_cursor, \
           account_rollup_live_cursor = excluded.account_rollup_live_cursor, \
           manifest_high_watermark_id = excluded.manifest_high_watermark_id, \
           coverage_revision = excluded.coverage_revision, \
           account_coverage_revision = excluded.account_coverage_revision, \
           durable_terminal_sequence_watermark = excluded.durable_terminal_sequence_watermark, \
           global_manifest_next_id = 0, account_manifest_next_id = 0, \
           global_manifest_complete = 0, account_manifest_complete = 0, \
           global_rollup_next_rowid = 0, account_rollup_next_rowid = 0, \
           usage_rollup_next_rowid = 0, global_rollup_complete = 0, account_rollup_complete = 0, \
           usage_rollup_complete = 0, account_unavailable = 0, global_usage_unavailable = 0, \
           account_usage_unavailable = 0, global_non_success_tokens = 0, \
           global_total_count = 0, global_success_count = 0, global_failure_count = 0, \
           global_total_tokens = 0, global_total_cost = 0, global_non_success_cost = 0, \
           updated_at = datetime('now')",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .bind(generation_fence.live_high_watermark_id)
    .bind(generation_fence.rollup_live_cursor)
    .bind(generation_fence.account_rollup_live_cursor)
    .bind(generation_fence.completed_manifest_high_watermark_id)
    .bind(generation_fence.coverage_revision)
    .bind(generation_fence.account_coverage_revision)
    .bind(
        generation_fence
            .durable_terminal_sequence_watermark
            .min(i64::MAX as u64) as i64,
    )
    .execute(&mut *transaction)
    .await
    .context("summary all-time projection checkpoint reset failed")?;
    transaction
        .commit()
        .await
        .context("summary all-time projection checkpoint reset commit failed")?;
    load_summary_all_time_projection_checkpoint(pool)
        .await?
        .ok_or_else(|| anyhow!("summary all-time projection checkpoint disappeared after reset"))
}

pub(crate) async fn reset_summary_all_time_projection_account_scope(
    pool: &Pool<Sqlite>,
    generation_fence: SummaryProjectionGenerationFence,
) -> Result<SummaryAllTimeProjectionCheckpointRow> {
    let mut transaction = pool
        .begin()
        .await
        .context("summary all-time account checkpoint reset transaction failed")?;
    sqlx::query("DELETE FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1")
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time account checkpoint reset failed")?;
    sqlx::query("DELETE FROM summary_all_time_projection_usage_checkpoint WHERE scope = ?1")
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time account usage checkpoint reset failed")?;
    // The global checkpoint contains no account-rollup data. Preserve its committed manifest
    // and aggregate progress when only an account-scoped source changes.
    sqlx::query(
        "UPDATE summary_all_time_projection_checkpoint SET \
           account_coverage_revision = ?1, account_manifest_next_id = 0, \
           account_manifest_complete = 0, account_rollup_next_rowid = 0, \
           account_rollup_complete = 0, account_unavailable = 0, \
           usage_rollup_next_rowid = 0, usage_rollup_complete = 0, \
           global_usage_unavailable = 0, account_usage_unavailable = 0, \
           global_non_success_tokens = 0, updated_at = datetime('now') \
         WHERE scope = ?2",
    )
    .bind(generation_fence.account_coverage_revision)
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .execute(&mut *transaction)
    .await
    .context("summary all-time account checkpoint scope reset failed")?;
    transaction
        .commit()
        .await
        .context("summary all-time account checkpoint reset commit failed")?;
    load_summary_all_time_projection_checkpoint(pool)
        .await?
        .ok_or_else(|| anyhow!("summary all-time account checkpoint disappeared after reset"))
}

pub(crate) async fn advance_summary_all_time_projection_checkpoint_live_fence(
    pool: &Pool<Sqlite>,
    generation_fence: SummaryProjectionGenerationFence,
) -> Result<()> {
    // Record the live boundary only after the immutable Projection CAS succeeds. The update is
    // monotonic so a concurrent terminal or rollup repair cannot move a committed checkpoint
    // backwards between publication and this durable progress marker.
    sqlx::query(
        "UPDATE summary_all_time_projection_checkpoint SET \
           live_high_watermark_id = MAX(live_high_watermark_id, ?1), \
           rollup_live_cursor = MAX(rollup_live_cursor, ?2), \
           account_rollup_live_cursor = CASE \
             WHEN ?3 IS NULL THEN account_rollup_live_cursor \
             WHEN account_rollup_live_cursor IS NULL THEN ?3 \
             ELSE MAX(account_rollup_live_cursor, ?3) END, \
           durable_terminal_sequence_watermark = MAX(durable_terminal_sequence_watermark, ?4), \
           updated_at = datetime('now') \
         WHERE scope = ?5",
    )
    .bind(generation_fence.live_high_watermark_id)
    .bind(generation_fence.rollup_live_cursor)
    .bind(generation_fence.account_rollup_live_cursor)
    .bind(
        generation_fence
            .durable_terminal_sequence_watermark
            .min(i64::MAX as u64) as i64,
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .execute(pool)
    .await
    .context("summary all-time projection checkpoint live fence advance failed")?;
    Ok(())
}

pub(crate) async fn advance_summary_all_time_projection_manifest_scope(
    pool: &Pool<Sqlite>,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
    account_scope: bool,
) -> Result<()> {
    let revision_column = if account_scope {
        "account_coverage_revision"
    } else {
        "coverage_revision"
    };
    let expected_revision = if account_scope {
        checkpoint.account_coverage_revision
    } else {
        checkpoint.coverage_revision
    };
    let complete = if account_scope {
        checkpoint.account_manifest_complete != 0
    } else {
        checkpoint.global_manifest_complete != 0
    };
    if complete {
        return Ok(());
    }
    let Some(high_watermark_id) = checkpoint.manifest_high_watermark_id else {
        let column = if account_scope {
            "account_manifest_complete"
        } else {
            "global_manifest_complete"
        };
        let result = sqlx::query(&format!(
            "UPDATE summary_all_time_projection_checkpoint SET {column} = 1, updated_at = datetime('now') \
             WHERE scope = ?1 AND {revision_column} = ?2 AND manifest_high_watermark_id IS NULL"
        ))
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .bind(expected_revision)
        .execute(pool)
        .await
        .context("summary all-time projection empty manifest checkpoint advance failed")?;
        if result.rows_affected() == 0 {
            debug!(
                account_scope,
                expected_revision,
                "summary all-time projection empty manifest checkpoint was superseded"
            );
        }
        return Ok(());
    };
    let after_id = if account_scope {
        checkpoint.account_manifest_next_id
    } else {
        checkpoint.global_manifest_next_id
    };
    let page = load_summary_projection_boundary_manifest_page_with_limit(
        pool,
        None,
        (after_id > 0).then_some(after_id),
        high_watermark_id,
        SUMMARY_PROJECTION_ALL_TIME_MANIFEST_PROOF_PAGE_SIZE,
    )
    .await?;
    let column = if account_scope {
        "account_manifest_complete"
    } else {
        "global_manifest_complete"
    };
    let next_column = if account_scope {
        "account_manifest_next_id"
    } else {
        "global_manifest_next_id"
    };
    if page.archives.is_empty() {
        // Only the exhausted cursor needs the expensive final-proof sweep. Running this check
        // before every bounded page made recovery rescan every historical manifest (and then
        // validate it again) on each supervisor turn.
        if summary_all_time_manifest_v2_coverage_complete(pool, high_watermark_id).await? {
            let result = sqlx::query(&format!(
                "UPDATE summary_all_time_projection_checkpoint \
                 SET {column} = 1, {next_column} = ?1, updated_at = datetime('now') \
                 WHERE scope = ?2 AND {revision_column} = ?3 AND manifest_high_watermark_id = ?4"
            ))
            .bind(high_watermark_id)
            .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
            .bind(expected_revision)
            .bind(high_watermark_id)
            .execute(pool)
            .await
            .context("summary all-time projection Snapshot V2 manifest completion failed")?;
            if result.rows_affected() == 0 {
                debug!(
                    account_scope,
                    expected_revision,
                    high_watermark_id,
                    "summary all-time projection Snapshot V2 completion was superseded"
                );
            }
            return Ok(());
        }
        let result = sqlx::query(&format!(
            "UPDATE summary_all_time_projection_checkpoint SET {column} = 1, updated_at = datetime('now') \
             WHERE scope = ?1 AND {revision_column} = ?2 AND manifest_high_watermark_id = ?3"
        ))
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .bind(expected_revision)
        .bind(high_watermark_id)
        .execute(pool)
        .await
        .context("summary all-time projection manifest completion checkpoint failed")?;
        if result.rows_affected() == 0 {
            debug!(
                account_scope,
                expected_revision,
                high_watermark_id,
                "summary all-time projection manifest completion was superseded"
            );
        }
        return Ok(());
    }
    let (global_exact, account_exact) =
        summary_all_time_coverage_page_is_exact(pool, &page).await?;
    let scope_exact = if account_scope {
        account_exact
    } else {
        global_exact
    };
    if !scope_exact {
        debug!(
            account_scope,
            manifest_count = page.archives.len(),
            "summary all-time projection manifest page is not exact yet"
        );
        return Ok(());
    }
    let next_after_id = page
        .next_after_id
        .expect("non-empty summary all-time projection manifest page has a seek cursor");
    let result = sqlx::query(&format!(
        "UPDATE summary_all_time_projection_checkpoint \
         SET {next_column} = ?1, updated_at = datetime('now') \
         WHERE scope = ?2 AND {revision_column} = ?3 AND manifest_high_watermark_id = ?4"
    ))
    .bind(next_after_id)
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .bind(expected_revision)
    .bind(high_watermark_id)
    .execute(pool)
    .await
    .context("summary all-time projection manifest checkpoint advance failed")?;
    if result.rows_affected() == 0 {
        debug!(
            account_scope,
            expected_revision,
            high_watermark_id,
            next_after_id,
            "summary all-time projection manifest checkpoint advance was superseded"
        );
        return Ok(());
    }
    debug!(
        account_scope,
        manifest_count = page.archives.len(),
        "summary all-time projection manifest checkpoint advanced"
    );
    Ok(())
}

pub(crate) async fn advance_summary_all_time_projection_global_rollup(
    pool: &Pool<Sqlite>,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
) -> Result<()> {
    if checkpoint.global_rollup_complete != 0 {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, SummaryAllTimeCheckpointGlobalRollupRow>(
        "SELECT rowid, total_count, success_count, failure_count, total_tokens, total_cost, \
                COALESCE(non_success_cost, 0.0) AS non_success_cost \
         FROM invocation_rollup_hourly WHERE rowid > ?1 ORDER BY rowid ASC LIMIT ?2",
    )
    .bind(checkpoint.global_rollup_next_rowid)
    .bind(SUMMARY_PROJECTION_ALL_TIME_ROLLUP_PAGE_SIZE)
    .fetch_all(pool)
    .await
    .context("summary all-time projection global rollup page hydration failed")?;
    if rows.is_empty() {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint \
             SET global_rollup_complete = 1, updated_at = datetime('now') WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(pool)
        .await
        .context("summary all-time projection global rollup completion failed")?;
        return Ok(());
    }
    let totals = rows.iter().fold(StatsTotals::default(), |totals, row| {
        totals.add(row.totals())
    });
    let next_rowid = rows
        .last()
        .expect("non-empty global rollup page has a rowid")
        .rowid;
    sqlx::query(
        "UPDATE summary_all_time_projection_checkpoint SET \
           global_rollup_next_rowid = ?1, \
           global_total_count = global_total_count + ?2, \
           global_success_count = global_success_count + ?3, \
           global_failure_count = global_failure_count + ?4, \
           global_total_tokens = global_total_tokens + ?5, \
           global_total_cost = global_total_cost + ?6, \
           global_non_success_cost = global_non_success_cost + ?7, \
           updated_at = datetime('now') WHERE scope = ?8",
    )
    .bind(next_rowid)
    .bind(totals.total_count)
    .bind(totals.success_count)
    .bind(totals.failure_count)
    .bind(totals.total_tokens)
    .bind(totals.total_cost)
    .bind(totals.non_success_cost)
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .execute(pool)
    .await
    .context("summary all-time projection global rollup checkpoint advance failed")?;
    Ok(())
}

pub(crate) async fn advance_summary_all_time_projection_account_rollup(
    pool: &Pool<Sqlite>,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
) -> Result<()> {
    if checkpoint.account_rollup_complete != 0 || checkpoint.account_unavailable != 0 {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, SummaryAllTimeCheckpointAccountRollupRow>(
        "SELECT rowid, upstream_account_id, total_count, success_count, failure_count, \
                total_tokens, total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost \
         FROM upstream_account_stats_hourly \
         WHERE rowid > ?1 AND upstream_account_id > 0 ORDER BY rowid ASC LIMIT ?2",
    )
    .bind(checkpoint.account_rollup_next_rowid)
    .bind(SUMMARY_PROJECTION_ALL_TIME_ROLLUP_PAGE_SIZE)
    .fetch_all(pool)
    .await
    .context("summary all-time projection account rollup page hydration failed")?;
    let mut transaction = pool
        .begin()
        .await
        .context("summary all-time projection account rollup transaction failed")?;
    if rows.is_empty() {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint \
             SET account_rollup_complete = 1, updated_at = datetime('now') WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup completion failed")?;
        transaction
            .commit()
            .await
            .context("summary all-time projection account rollup completion commit failed")?;
        return Ok(());
    }
    for row in &rows {
        let totals = row.totals();
        sqlx::query(
            "INSERT INTO summary_all_time_projection_account_checkpoint \
             (scope, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(scope, upstream_account_id) DO UPDATE SET \
               total_count = total_count + excluded.total_count, \
               success_count = success_count + excluded.success_count, \
               failure_count = failure_count + excluded.failure_count, \
               total_tokens = total_tokens + excluded.total_tokens, \
               total_cost = total_cost + excluded.total_cost, \
               non_success_cost = non_success_cost + excluded.non_success_cost",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .bind(row.upstream_account_id)
        .bind(totals.total_count)
        .bind(totals.success_count)
        .bind(totals.failure_count)
        .bind(totals.total_tokens)
        .bind(totals.total_cost)
        .bind(totals.non_success_cost)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup accumulation failed")?;
    }
    let account_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .fetch_one(&mut *transaction)
    .await
    .context("summary all-time projection account checkpoint count failed")?;
    let next_rowid = rows
        .last()
        .expect("non-empty account rollup page has a rowid")
        .rowid;
    if account_count > SUMMARY_PROJECTION_MAX_ACCOUNTS as i64 {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             account_unavailable = 1, account_rollup_complete = 1, \
             updated_at = datetime('now') WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account cardinality checkpoint failed")?;
    } else {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             account_rollup_next_rowid = ?1, updated_at = datetime('now') WHERE scope = ?2",
        )
        .bind(next_rowid)
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup checkpoint advance failed")?;
    }
    transaction
        .commit()
        .await
        .context("summary all-time projection account rollup checkpoint commit failed")?;
    Ok(())
}

pub(crate) async fn load_summary_projection_generation_fence(
    state: &AppState,
) -> Result<SummaryProjectionGenerationFence> {
    let durable_terminal_sequence_watermark = state
        .dashboard_activity_snapshot_cache
        .lock()
        .await
        .read_model
        .settled_terminal_sequence;
    Ok(SummaryProjectionGenerationFence {
        live_high_watermark_id: load_summary_projection_live_high_watermark(&state.pool).await?,
        rollup_live_cursor: load_summary_projection_rollup_live_cursor(&state.pool).await?,
        account_rollup_live_cursor: load_summary_projection_account_rollup_live_cursor(&state.pool)
            .await?,
        completed_manifest_high_watermark_id: summary_projection_completed_manifest_high_watermark(
            &state.pool,
        )
        .await?,
        coverage_revision: summary_projection_coverage_revision(&state.pool).await?,
        account_coverage_revision: summary_projection_account_coverage_revision(&state.pool)
            .await?,
        durable_terminal_sequence_watermark,
    })
}

pub(crate) async fn load_summary_projection_generation_fence_tx(
    state: &AppState,
    connection: &mut SqliteConnection,
) -> Result<SummaryProjectionGenerationFence> {
    let durable_terminal_sequence_watermark = state
        .dashboard_activity_snapshot_cache
        .lock()
        .await
        .read_model
        .settled_terminal_sequence;
    let live_high_watermark_id =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(&mut *connection)
            .await
            .context("summary projection live high-watermark transaction hydration failed")?
            .unwrap_or_default();
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(connection)
        .await
        .context("summary projection live rollup cursor transaction hydration failed")?;
    let account_rollup_live_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress \
         WHERE dataset = 'invocation_account_activity_v2_repair_live_cursor'",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection account rollup cursor transaction hydration failed")?;
    let completed_manifest_high_watermark_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(&mut *connection)
    .await
    .context("summary projection completed manifest transaction hydration failed")?;
    let coverage_revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_coverage_revision WHERE id = 1",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection coverage revision transaction hydration failed")?
    .unwrap_or_default();
    let account_coverage_revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_account_coverage_revision WHERE id = 1",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection account coverage revision transaction hydration failed")?
    .unwrap_or_default();
    Ok(SummaryProjectionGenerationFence {
        live_high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        completed_manifest_high_watermark_id,
        coverage_revision,
        account_coverage_revision,
        durable_terminal_sequence_watermark,
    })
}

/// Publish a coverage-derived projection while holding the SQLite writer fence used to verify its
/// durable inputs. A caller may spend substantial time reducing pages before reaching this point;
/// the immediate transaction re-check ensures a manifest, rollup, or source-tail mutation cannot
/// land between the caller's last fence read and the in-memory CAS.
pub(crate) async fn publish_summary_projection_with_durable_fence(
    state: &AppState,
    projection: SummaryProjection,
    expected_revision: u64,
    expected_generation_fence: SummaryProjectionGenerationFence,
) -> Result<bool> {
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin Summary durable-fence publication transaction")?;
    let current_generation_fence =
        load_summary_projection_generation_fence_tx(state, transaction.as_mut()).await?;
    if current_generation_fence != expected_generation_fence {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-fence publication")?;
        return Ok(false);
    }
    let published = state
        .subscription_hub
        .store_summary_projection_if_revision_and_generation(
            projection,
            expected_revision,
            expected_generation_fence,
        )
        .await;
    if !published {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-fence projection CAS")?;
        return Ok(false);
    }
    transaction
        .commit()
        .await
        .context("commit Summary durable-fence publication transaction")?;
    Ok(true)
}

pub(crate) async fn publish_summary_projection_with_durable_coverage_fence(
    state: &AppState,
    projection: SummaryProjection,
    expected_revision: u64,
    expected_generation_fence: SummaryProjectionGenerationFence,
) -> Result<bool> {
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin Summary durable-coverage publication transaction")?;
    let current_generation_fence =
        load_summary_projection_generation_fence_tx(state, transaction.as_mut()).await?;
    if !current_generation_fence.coverage_sources_match(expected_generation_fence)
        || !current_generation_fence
            .live_tail_cursor()
            .at_or_behind(expected_generation_fence.live_tail_cursor())
    {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-coverage publication")?;
        return Ok(false);
    }
    let published = state
        .subscription_hub
        .store_summary_projection_if_revision_and_coverage_generation(
            projection,
            expected_revision,
            expected_generation_fence,
        )
        .await;
    if !published {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-coverage projection CAS")?;
        return Ok(false);
    }
    transaction
        .commit()
        .await
        .context("commit Summary durable-coverage publication transaction")?;
    Ok(true)
}

pub(crate) async fn advance_summary_all_time_projection_checkpoint(
    state: &AppState,
) -> Result<SummaryAllTimeProjectionCheckpointRow> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let mut checkpoint = match load_summary_all_time_projection_checkpoint(&state.pool).await? {
        Some(checkpoint)
            if checkpoint
                .generation_fence()
                .global_coverage_checkpoint_compatible(generation_fence) =>
        {
            if checkpoint
                .generation_fence()
                .account_coverage_checkpoint_compatible(generation_fence)
            {
                checkpoint
            } else {
                debug!(
                    "summary all-time account coverage revision changed; resetting only account recovery"
                );
                reset_summary_all_time_projection_account_scope(&state.pool, generation_fence)
                    .await?
            }
        }
        Some(_) => {
            debug!(
                "summary all-time projection checkpoint generation changed; restarting staged recovery"
            );
            reset_summary_all_time_projection_checkpoint(&state.pool, generation_fence).await?
        }
        None => reset_summary_all_time_projection_checkpoint(&state.pool, generation_fence).await?,
    };
    // Proof insertion is forward progress and therefore keeps the committed manifest/rollup
    // cursors.  Persist the newer coverage revisions on that same checkpoint before advancing
    // pages; otherwise finalization would retain the old fence and reject its own atomic swap.
    if checkpoint.coverage_revision != generation_fence.coverage_revision
        || checkpoint.account_coverage_revision != generation_fence.account_coverage_revision
    {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             coverage_revision = ?1, account_coverage_revision = ?2, updated_at = datetime('now') \
             WHERE scope = ?3",
        )
        .bind(generation_fence.coverage_revision)
        .bind(generation_fence.account_coverage_revision)
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&state.pool)
        .await
        .context("summary all-time projection checkpoint coverage revision adoption failed")?;
        checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
            .await?
            .expect(
                "summary all-time projection checkpoint exists after coverage revision adoption",
            );
    }
    advance_summary_all_time_projection_manifest_scope(&state.pool, &checkpoint, false).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after global manifest advance");
    advance_summary_all_time_projection_manifest_scope(&state.pool, &checkpoint, true).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after account manifest advance");
    advance_summary_all_time_projection_global_rollup(&state.pool, &checkpoint).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after global rollup advance");
    advance_summary_all_time_projection_account_rollup(&state.pool, &checkpoint).await?;
    if checkpoint.usage_rollup_complete == 0 {
        // Usage is independently bounded by the existing rollup reader. Mark it ready only
        // once; repeating this full read after completion made every idle supervisor pass scan
        // the complete usage rollup again.
        let usage_ready = match load_summary_projection_rollup_usage(&state.pool).await {
            Ok(_) => (1_i64, 0_i64, 0_i64),
            Err(error) if error.to_string().contains("budget") => (1_i64, 1_i64, 1_i64),
            Err(error) => return Err(error),
        };
        sqlx::query("UPDATE summary_all_time_projection_checkpoint SET usage_rollup_complete = ?1, global_usage_unavailable = ?2, account_usage_unavailable = ?3, updated_at = datetime('now') WHERE scope = ?4")
            .bind(usage_ready.0).bind(usage_ready.1).bind(usage_ready.2)
            .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE).execute(&state.pool).await?;
    }
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after account rollup advance");
    if !load_summary_projection_generation_fence(state)
        .await?
        .coverage_sources_match(generation_fence)
    {
        // The rolling projection may predate a newly verified Snapshot V2 page. Its recent
        // records remain exact; the finalizer can merge the historical totals and publish the
        // current fence atomically below. A concurrent live fence change is still checked before
        // the swap.
        debug!(
            "summary historical coverage finalization is merging into a projection with an older coverage fence"
        );
    }
    Ok(checkpoint)
}

pub(crate) async fn summary_all_time_projection_checkpoint_live_tail_count(
    pool: &Pool<Sqlite>,
    lower_bound_id: i64,
    upper_bound_id: i64,
    scope: &'static str,
) -> Result<()> {
    if upper_bound_id <= lower_bound_id {
        return Ok(());
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE id > ?1 AND id <= ?2",
    )
    .bind(lower_bound_id)
    .bind(upper_bound_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("summary all-time {scope} live-tail admission count failed"))?;
    if count > summary_projection_exact_record_limit() as i64 {
        return Err(anyhow!(
            "summary all-time {scope} live-tail admission exceeded bounded record budget ({count} > {})",
            summary_projection_exact_record_limit()
        ));
    }
    Ok(())
}
