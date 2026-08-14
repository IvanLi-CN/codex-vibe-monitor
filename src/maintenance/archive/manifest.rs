use super::*;
use tracing::warn;

pub(crate) async fn refresh_archive_upstream_activity_manifest(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveManifestRefreshSummary> {
    let candidate_limit =
        super::super::retention::retention_candidate_limit(config, "archive_manifest_refresh");
    let mut batches = sqlx::query_as::<_, ArchiveManifestBatchRow>(
        r#"
        SELECT id, file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND upstream_activity_manifest_refreshed_at IS NULL
        ORDER BY month_key DESC, day_key DESC, part_key DESC, created_at DESC, id DESC
        LIMIT ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(candidate_limit.saturating_add(1) as i64)
    .fetch_all(pool)
    .await?;

    let candidate_remaining_hint = usize::from(batches.len() > candidate_limit);
    batches.truncate(candidate_limit);
    let mut summary = ArchiveManifestRefreshSummary {
        pending_batches: batches.len(),
        ..ArchiveManifestRefreshSummary::default()
    };

    for batch in batches {
        let archive_path = PathBuf::from(&batch.file_path);
        if !archive_path.exists() {
            summary.missing_files += 1;
            warn!(
                archive_batch_id = batch.id,
                file_path = %archive_path.display(),
                "archive upstream activity manifest rebuild skipped missing archive file and will retry later"
            );
            continue;
        }

        let values = match load_archive_upstream_activity_from_file(&archive_path).await {
            Ok(values) => values,
            Err(err) => {
                warn!(
                    archive_batch_id = batch.id,
                    file_path = %archive_path.display(),
                    error = %err,
                    "archive upstream activity manifest rebuild failed and will retry later"
                );
                continue;
            }
        };
        let deduped_values = dedupe_archive_upstream_last_activity(values);
        if dry_run {
            summary.refreshed_batches += 1;
            summary.account_rows_written += deduped_values.len();
            continue;
        }

        let Some(account_rows_written) = replace_archive_batch_upstream_activity_in_micro_batches(
            pool,
            config,
            batch.id,
            &deduped_values,
            candidate_remaining_hint.max(
                summary
                    .pending_batches
                    .saturating_sub(summary.refreshed_batches),
            ),
        )
        .await?
        else {
            break;
        };
        summary.refreshed_batches += 1;
        summary.account_rows_written += account_rows_written;
    }

    Ok(summary)
}

pub(crate) async fn replace_archive_batch_upstream_activity_in_micro_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    archive_batch_id: i64,
    values: &[(i64, String)],
    candidate_remaining_hint: usize,
) -> Result<Option<usize>> {
    let clear_batch_limit =
        super::super::retention::retention_micro_batch_limit(config, "archive_manifest_clear");
    let write_batch_limit =
        super::super::retention::retention_micro_batch_limit(config, "archive_manifest_refresh");

    loop {
        let staged_rowids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT rowid
            FROM archive_batch_upstream_activity
            WHERE archive_batch_id = ?1
            ORDER BY rowid ASC
            LIMIT ?2
            "#,
        )
        .bind(archive_batch_id)
        .bind(clear_batch_limit as i64)
        .fetch_all(pool)
        .await?;
        if staged_rowids.is_empty() {
            break;
        }

        let Some(admission) =
            super::super::retention::acquire_retention_write_admission("archive_manifest_clear")
                .await
        else {
            return Ok(None);
        };
        let execute_started = Instant::now();
        let mut tx = pool.begin().await?;
        let mut delete = QueryBuilder::<Sqlite>::new(
            "DELETE FROM archive_batch_upstream_activity WHERE rowid IN (",
        );
        {
            let mut separated = delete.separated(", ");
            for rowid in &staged_rowids {
                separated.push_bind(rowid);
            }
        }
        delete.push(")");
        delete.build().execute(tx.as_mut()).await?;
        let commit_started = Instant::now();
        tx.commit().await?;
        super::super::retention::retention_record_commit!(
            "archive_manifest_clear",
            admission.admission_mode(),
            staged_rowids.len(),
            staged_rowids.len().saturating_mul(96),
            Duration::ZERO,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count(),
            candidate_remaining_hint,
        );
    }

    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    if deduped_values.is_empty() {
        return mark_archive_batch_upstream_activity_refreshed(
            pool,
            archive_batch_id,
            candidate_remaining_hint,
        )
        .await
        .map(|completed| completed.then_some(0));
    }

    let chunk_count = deduped_values.chunks(write_batch_limit).len();
    for (index, chunk) in deduped_values.chunks(write_batch_limit).enumerate() {
        let Some(admission) =
            super::super::retention::acquire_retention_write_admission("archive_manifest_refresh")
                .await
        else {
            return Ok(None);
        };
        let execute_started = Instant::now();
        let mut tx = pool.begin().await?;
        let mut insert = QueryBuilder::<Sqlite>::new(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) ",
        );
        insert.push_values(chunk, |mut row, (account_id, last_activity_at)| {
            row.push_bind(archive_batch_id)
                .push_bind(account_id)
                .push_bind(last_activity_at);
        });
        insert.push(
            " ON CONFLICT(archive_batch_id, account_id) DO UPDATE SET last_activity_at = CASE \
             WHEN excluded.last_activity_at > last_activity_at THEN excluded.last_activity_at \
             ELSE last_activity_at END",
        );
        insert.build().execute(tx.as_mut()).await?;
        if index + 1 == chunk_count {
            sqlx::query(
                "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') WHERE id = ?1",
            )
            .bind(archive_batch_id)
            .execute(tx.as_mut())
            .await?;
        }
        let commit_started = Instant::now();
        tx.commit().await?;
        super::super::retention::retention_record_commit!(
            "archive_manifest_refresh",
            admission.admission_mode(),
            chunk.len(),
            chunk.len().saturating_mul(96),
            Duration::ZERO,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count(),
            candidate_remaining_hint,
        );
    }

    Ok(Some(deduped_values.len()))
}

async fn mark_archive_batch_upstream_activity_refreshed(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    candidate_remaining_hint: usize,
) -> Result<bool> {
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("archive_manifest_refresh")
            .await
    else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    sqlx::query(
        "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') WHERE id = ?1",
    )
    .bind(archive_batch_id)
    .execute(pool)
    .await?;
    super::super::retention::retention_record_commit!(
        "archive_manifest_refresh",
        admission.admission_mode(),
        1,
        128,
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count(),
        candidate_remaining_hint,
    );
    Ok(true)
}

pub(crate) async fn load_archive_upstream_activity_from_file(
    archive_path: &Path,
) -> Result<Vec<(i64, String)>> {
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;

    let database_url = format!("sqlite://{}", temp_path.to_string_lossy());
    let connect_opts = build_sqlite_connect_options(
        &database_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_opts)
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;

    let rows = sqlx::query_as::<_, ArchivedAccountLastActivityRow>(
        r#"
        SELECT account_id, MAX(occurred_at) AS last_activity_at
        FROM (
            SELECT
                CASE
                    WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER)
                END AS account_id,
                occurred_at
            FROM codex_invocations
        )
        WHERE account_id IS NOT NULL
        GROUP BY account_id
        "#,
    )
    .fetch_all(&archive_pool)
    .await?;

    archive_pool.close().await;
    drop(temp_cleanup);

    Ok(dedupe_archive_upstream_last_activity(
        rows.into_iter()
            .map(|row| (row.account_id, row.last_activity_at)),
    ))
}

pub(crate) async fn backfill_upstream_account_last_activity_from_archives(
    pool: &Pool<Sqlite>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<ArchiveBackfillSummary> {
    let total_pending_accounts = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64;
    if total_pending_accounts == 0 {
        return Ok(ArchiveBackfillSummary::default());
    }

    let pending_fetch_limit = scan_limit.unwrap_or(total_pending_accounts).max(1) as i64;
    let pending_account_ids = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        ORDER BY id ASC
        LIMIT ?1
        "#,
    )
    .bind(pending_fetch_limit)
    .fetch_all(pool)
    .await?;

    let pending_manifest_batches = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND upstream_activity_manifest_refreshed_at IS NULL
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?;
    if pending_manifest_batches > 0 {
        return Ok(ArchiveBackfillSummary {
            waiting_for_manifest_backfill: true,
            ..ArchiveBackfillSummary::default()
        });
    }

    let scanned_batches = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?
    .max(0) as u64;
    if scanned_batches == 0 {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_archive_backfill_completed = 1
            WHERE last_activity_at IS NULL
              AND last_activity_archive_backfill_completed = 0
            "#,
        )
        .execute(pool)
        .await?;
        return Ok(ArchiveBackfillSummary::default());
    }

    let pending = pending_account_ids.into_iter().collect::<HashSet<_>>();
    let mut recovered = HashMap::<i64, String>::new();
    let pending_chunks = pending_account_ids_chunks(&pending);
    let started_at = Instant::now();
    let mut processed_account_ids = HashSet::new();
    let mut hit_budget = total_pending_accounts > pending.len() as u64;

    for (chunk_idx, account_ids) in pending_chunks.iter().enumerate() {
        if startup_backfill_budget_reached(
            started_at,
            processed_account_ids.len() as u64,
            None,
            max_elapsed,
        ) {
            hit_budget = true;
            break;
        }
        for account_id in account_ids {
            processed_account_ids.insert(*account_id);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT account_id, MAX(last_activity_at) AS last_activity_at FROM archive_batch_upstream_activity WHERE account_id IN (",
        );
        {
            let mut separated = query.separated(", ");
            for account_id in account_ids {
                separated.push_bind(account_id);
            }
        }
        query.push(") GROUP BY account_id");
        for row in query
            .build_query_as::<ArchivedAccountLastActivityRow>()
            .fetch_all(pool)
            .await?
        {
            recovered
                .entry(row.account_id)
                .and_modify(|current| {
                    if *current < row.last_activity_at {
                        *current = row.last_activity_at.clone();
                    }
                })
                .or_insert(row.last_activity_at);
        }

        if chunk_idx + 1 < pending_chunks.len()
            && startup_backfill_budget_reached(
                started_at,
                processed_account_ids.len() as u64,
                None,
                max_elapsed,
            )
        {
            hit_budget = true;
            break;
        }
    }

    if recovered.is_empty() {
        let processed = processed_account_ids.iter().copied().collect::<Vec<_>>();
        if !processed.is_empty() {
            mark_archive_backfill_completed_for_accounts(pool, &processed).await?;
        }
        return Ok(ArchiveBackfillSummary {
            scanned_batches: processed_account_ids.len() as u64,
            updated_accounts: 0,
            hit_budget,
            waiting_for_manifest_backfill: false,
        });
    }

    let unresolved: Vec<i64> = processed_account_ids
        .iter()
        .copied()
        .filter(|account_id| !recovered.contains_key(account_id))
        .collect();
    let updated_accounts = recovered.len() as u64;
    let mut tx = pool.begin().await?;
    for (account_id, occurred_at) in recovered {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                    WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                    ELSE last_activity_at
                END,
                last_activity_archive_backfill_completed = 1
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(account_id)
        .execute(tx.as_mut())
        .await?;
    }
    if !unresolved.is_empty() {
        mark_archive_backfill_completed_for_accounts_tx(tx.as_mut(), &unresolved).await?;
    }
    tx.commit().await?;

    Ok(ArchiveBackfillSummary {
        scanned_batches: processed_account_ids.len() as u64,
        updated_accounts,
        hit_budget,
        waiting_for_manifest_backfill: false,
    })
}

pub(crate) fn pending_account_ids_chunks(pending: &HashSet<i64>) -> Vec<Vec<i64>> {
    pending
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

pub(crate) async fn mark_archive_backfill_completed_for_accounts(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<()> {
    if account_ids.is_empty() {
        return Ok(());
    }
    for account_chunk in account_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut update = QueryBuilder::<Sqlite>::new(
            "UPDATE pool_upstream_accounts SET last_activity_archive_backfill_completed = 1 WHERE id IN (",
        );
        {
            let mut separated = update.separated(", ");
            for account_id in account_chunk {
                separated.push_bind(account_id);
            }
        }
        update.push(")");
        update.build().execute(pool).await?;
    }
    Ok(())
}

pub(crate) async fn mark_archive_backfill_completed_for_accounts_tx(
    tx: &mut SqliteConnection,
    account_ids: &[i64],
) -> Result<()> {
    if account_ids.is_empty() {
        return Ok(());
    }
    for account_chunk in account_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut update = QueryBuilder::<Sqlite>::new(
            "UPDATE pool_upstream_accounts SET last_activity_archive_backfill_completed = 1 WHERE id IN (",
        );
        {
            let mut separated = update.separated(", ");
            for account_id in account_chunk {
                separated.push_bind(account_id);
            }
        }
        update.push(")");
        update.build().execute(&mut *tx).await?;
    }
    Ok(())
}

pub(crate) async fn count_upstream_accounts_missing_last_activity(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE deleted_at IS NULL
              AND last_activity_at IS NULL
              AND last_activity_archive_backfill_completed = 0
            "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64)
}

pub(crate) async fn count_upstream_accounts_missing_live_last_activity(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE deleted_at IS NULL
              AND last_activity_at IS NULL
              AND last_activity_live_backfill_completed = 0
            "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64)
}
