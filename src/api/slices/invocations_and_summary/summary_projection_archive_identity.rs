#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryProjectionArchiveFileIdentity {
    Verified,
    Unavailable,
}

fn verify_summary_projection_archive_file_path_sha256(
    file_path: &str,
    expected_sha256: &str,
) -> Result<SummaryProjectionArchiveFileIdentity> {
    let actual_sha256 = match crate::maintenance::sha256_hex_file(Path::new(file_path)) {
        Ok(actual_sha256) => actual_sha256,
        // An unreadable archive contributes no raw data. Preserve the existing exact fallback:
        // materialized rollups remain usable, while an unmaterialized source is marked
        // unavailable by the strict archive readers below.
        Err(_) => return Ok(SummaryProjectionArchiveFileIdentity::Unavailable),
    };
    if actual_sha256 != expected_sha256 {
        if !summary_projection_archive_file_is_readable(file_path) {
            return Ok(SummaryProjectionArchiveFileIdentity::Unavailable);
        }
        return Err(anyhow!(
            "summary projection archive changed during hydration for {}",
            file_path
        ));
    }
    Ok(SummaryProjectionArchiveFileIdentity::Verified)
}

fn verify_summary_projection_archive_file_sha256(
    archive: &crate::stats::ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<SummaryProjectionArchiveFileIdentity> {
    verify_summary_projection_archive_file_path_sha256(archive.file_path(), expected_sha256)
}

fn require_summary_projection_archive_file_sha256(
    archive: &crate::stats::ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<()> {
    match verify_summary_projection_archive_file_sha256(archive, expected_sha256)? {
        SummaryProjectionArchiveFileIdentity::Verified => Ok(()),
        SummaryProjectionArchiveFileIdentity::Unavailable => Err(anyhow!(
            "summary projection archive became unavailable during hydration for {}",
            archive.file_path()
        )),
    }
}

fn verify_summary_projection_archive_file_paths_sha256(
    archive_paths: &[String],
    expected_sha256_by_file_path: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut verified_paths = Vec::with_capacity(archive_paths.len());
    for file_path in archive_paths {
        let expected_sha256 = expected_sha256_by_file_path.get(file_path).ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                file_path
            )
        })?;
        if verify_summary_projection_archive_file_path_sha256(file_path, expected_sha256)?
            == SummaryProjectionArchiveFileIdentity::Verified
        {
            verified_paths.push(file_path.clone());
        }
    }
    Ok(verified_paths)
}

fn require_summary_projection_archive_file_paths_sha256(
    archive_paths: &[String],
    expected_sha256_by_file_path: &HashMap<String, String>,
) -> Result<()> {
    for file_path in archive_paths {
        let expected_sha256 = expected_sha256_by_file_path.get(file_path).ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                file_path
            )
        })?;
        match verify_summary_projection_archive_file_path_sha256(file_path, expected_sha256)? {
            SummaryProjectionArchiveFileIdentity::Verified => {}
            SummaryProjectionArchiveFileIdentity::Unavailable => {
                return Err(anyhow!(
                    "summary projection archive became unavailable during hydration for {}",
                    file_path
                ));
            }
        }
    }
    Ok(())
}

struct SummaryProjectionAllTimeArchiveScanPaths {
    global: Vec<String>,
    account: Vec<String>,
    global_unmaterialized_count: usize,
    global_unmaterialized: Vec<String>,
}

async fn load_summary_projection_all_time_archive_scan_paths(
    pool: &Pool<Sqlite>,
) -> Result<SummaryProjectionAllTimeArchiveScanPaths> {
    ensure_summary_projection_archive_identity_is_unambiguous(pool).await?;
    let rows = query_summary_projection_all_time_archive_scan_rows(pool).await?;
    let mut global = Vec::new();
    let mut account = Vec::new();
    let mut global_unmaterialized_count = 0usize;
    let mut global_unmaterialized = Vec::new();
    for (file_path, needs_global_archive_scan, needs_account_archive_scan, is_unmaterialized) in
        rows
    {
        if needs_global_archive_scan != 0 {
            if is_unmaterialized != 0 {
                global_unmaterialized_count += 1;
                global_unmaterialized.push(file_path.clone());
            }
            global.push(file_path.clone());
        }
        if needs_account_archive_scan != 0 {
            account.push(file_path);
        }
    }
    Ok(SummaryProjectionAllTimeArchiveScanPaths {
        global,
        account,
        global_unmaterialized_count,
        global_unmaterialized,
    })
}

async fn ensure_summary_projection_archive_identity_is_unambiguous(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    // Every downstream legacy fallback is keyed by file path. If a path has multiple completed
    // manifest identities, selecting the latest row could attach the wrong proof or snapshot to
    // an older archive. Refuse the ambiguous authority and leave the affected all-time scope
    // unavailable until recovery can establish an identity-bound V2 proof.
    if sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
         GROUP BY file_path \
         HAVING COUNT(DISTINCT COALESCE(NULLIF(TRIM(sha256), ''), '<missing>')) > 1 \
         ORDER BY file_path LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection archive identity ambiguity check failed")?
    .is_some()
    {
        return Err(anyhow!(
            "summary projection archive identity is ambiguous for a completed path"
        ));
    }
    if sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
         GROUP BY file_path, sha256 \
         HAVING COUNT(*) > 1 \
         ORDER BY file_path LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection duplicate archive identity check failed")?
    .is_some()
    {
        return Err(anyhow!(
            "summary projection archive identity is duplicated for a completed path"
        ));
    }
    Ok(())
}

async fn query_summary_projection_all_time_archive_scan_rows(
    pool: &Pool<Sqlite>,
) -> Result<Vec<(String, i64, i64, i64)>> {
    // These are precisely the completed archive paths that the two generic all-time
    // aggregators may inflate: the global pass handles missing invocation replay, and the
    // account pass additionally handles unmaterialized archives missing account replay.
    let rows = sqlx::query_as::<_, (String, i64, i64, i64)>(
        "SELECT batches.file_path, \
                MAX(NOT EXISTS ( \
                    SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                    WHERE replay.target = ?1 \
                      AND replay.dataset = 'codex_invocations' \
                      AND replay.file_path = batches.file_path \
                      AND batches.sha256 IS NOT NULL \
                      AND TRIM(batches.sha256) <> '' \
                      AND replay.archive_sha256 = batches.sha256 \
                ) OR ( \
                    batches.historical_rollups_materialized_at IS NULL \
                    AND ( \
                        NOT EXISTS ( \
                            SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                            WHERE replay.target = ?2 \
                              AND replay.dataset = 'codex_invocations' \
                              AND replay.file_path = batches.file_path \
                              AND batches.sha256 IS NOT NULL \
                              AND TRIM(batches.sha256) <> '' \
                              AND replay.archive_sha256 = batches.sha256 \
                        ) \
                        OR NOT EXISTS ( \
                            SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                            WHERE replay.target = ?3 \
                              AND replay.dataset = 'codex_invocations' \
                              AND replay.file_path = batches.file_path \
                              AND batches.sha256 IS NOT NULL \
                              AND TRIM(batches.sha256) <> '' \
                              AND replay.archive_sha256 = batches.sha256 \
                        ) \
                    ) \
                )) AS needs_global_archive_scan, \
                MAX(batches.historical_rollups_materialized_at IS NULL AND NOT EXISTS ( \
                    SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                    WHERE replay.target = ?2 \
                      AND replay.dataset = 'codex_invocations' \
                      AND replay.file_path = batches.file_path \
                      AND batches.sha256 IS NOT NULL \
                      AND TRIM(batches.sha256) <> '' \
                      AND replay.archive_sha256 = batches.sha256 \
                )) AS needs_account_archive_scan, \
                MAX(batches.historical_rollups_materialized_at IS NULL) AS is_unmaterialized \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
         AND batches.status = 'completed' \
         AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
         AND (NOT EXISTS ( \
                   SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                   WHERE replay.target = ?1 \
                     AND replay.dataset = 'codex_invocations' \
                     AND replay.file_path = batches.file_path \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND replay.archive_sha256 = batches.sha256 \
               ) OR ( \
                   batches.historical_rollups_materialized_at IS NULL \
                   AND (NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?2 \
                         AND replay.dataset = 'codex_invocations' \
                         AND replay.file_path = batches.file_path \
                         AND batches.sha256 IS NOT NULL \
                         AND TRIM(batches.sha256) <> '' \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?3 \
                         AND replay.dataset = 'codex_invocations' \
                         AND replay.file_path = batches.file_path \
                         AND batches.sha256 IS NOT NULL \
                         AND TRIM(batches.sha256) <> '' \
                         AND replay.archive_sha256 = batches.sha256 \
                   )) \
               )) \
         GROUP BY batches.file_path \
         ORDER BY MIN(batches.month_key) ASC, MIN(batches.created_at) ASC, MIN(batches.id) ASC \
         LIMIT ?4",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection all-time archive scan admission failed")?;
    if rows.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        return Err(anyhow!(
            "summary projection all-time archive scan cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES})"
        ));
    }
    Ok(rows)
}

async fn load_summary_projection_archive_manifest_accounts(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<Vec<(String, i64)>> {
    if archive_paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut accounts = Vec::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT batches.file_path, activity.account_id \
             FROM archive_batches AS batches \
             JOIN archive_batch_upstream_activity AS activity \
               ON activity.archive_batch_id = batches.id \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND activity.account_id > 0 \
               AND batches.file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        let remaining = summary_projection_exact_record_limit().saturating_sub(accounts.len());
        query
            .push(" LIMIT ")
            .push_bind((remaining.saturating_add(1)) as i64);
        let rows = query
            .build_query_as::<(String, i64)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive account manifest hydration failed")?;
        if rows.len() > remaining {
            return Err(anyhow!(
                "summary projection archive account manifest exceeded bounded row budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
            ));
        }
        accounts.extend(rows);
    }
    Ok(accounts)
}

async fn load_summary_projection_archive_manifest_refreshed_paths(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashSet<String>> {
    if archive_paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut refreshed_paths = HashSet::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
              AND upstream_activity_manifest_refreshed_at IS NOT NULL
              AND file_path IN (
            "#,
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        let rows = query
            .push(")")
            .build_query_scalar::<String>()
            .fetch_all(pool)
            .await
            .context("summary projection archive account manifest completion hydration failed")?;
        refreshed_paths.extend(rows);
    }
    Ok(refreshed_paths)
}

async fn load_summary_projection_archive_replay_coverage(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, SummaryProjectionArchiveReplayCoverage>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }
    let mut coverage = HashMap::<String, SummaryProjectionArchiveReplayCoverage>::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT replay.file_path, replay.target FROM hourly_rollup_archive_replay AS replay \
             INNER JOIN archive_batches AS batches \
               ON batches.dataset = replay.dataset \
              AND batches.file_path = replay.file_path \
              AND batches.status = 'completed' \
              AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
              AND batches.sha256 IS NOT NULL \
              AND TRIM(batches.sha256) <> '' \
              AND batches.sha256 = replay.archive_sha256 \
             WHERE replay.dataset = 'codex_invocations' AND replay.target IN (",
        );
        query
            .push_bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
            .push(", ")
            .push_bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
            .push(", ")
            .push_bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .push(") AND replay.file_path IN (");
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        let rows = query
            .build_query_as::<(String, String)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive replay coverage hydration failed")?;
        for (file_path, target) in rows {
            let entry = coverage.entry(file_path).or_default();
            if target == HOURLY_ROLLUP_TARGET_INVOCATIONS {
                entry.overall = true;
            }
            if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY {
                entry.account_stats = true;
            }
            if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
                entry.usage_breakdown = true;
            }
        }
    }
    Ok(coverage)
}

async fn summary_projection_completed_manifest_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(pool)
    .await
    .context("summary projection completed manifest high-watermark hydration failed")
}

async fn summary_projection_coverage_revision(pool: &Pool<Sqlite>) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_coverage_revision WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection coverage revision hydration failed")
    .map(|revision| revision.unwrap_or_default())
}

async fn summary_projection_account_coverage_revision(pool: &Pool<Sqlite>) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_account_coverage_revision WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection account coverage revision hydration failed")
    .map(|revision| revision.unwrap_or_default())
}
