async fn load_summary_all_time_coverage_proof_marked_paths(
    pool: &Pool<Sqlite>,
    paths: &[String],
) -> Result<HashSet<String>> {
    // Probe all final-proof identities in one read so legacy manifests do not each open a
    // separate transaction during a staged recovery page.
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
    for path in paths {
        separated.push_bind(path);
    }
    separated.push_unseparated(")");
    Ok(query
        .build_query_scalar::<String>()
        .fetch_all(pool)
        .await
        .context("summary all-time coverage page proof marker lookup failed")?
        .into_iter()
        .collect())
}

async fn summary_all_time_coverage_archive_is_exact(
    pool: &Pool<Sqlite>,
    archive: &crate::stats::ArchiveBatchPathRow,
    replay: SummaryProjectionArchiveReplayCoverage,
    account_ids: Option<&HashSet<i64>>,
    account_manifest_refreshed: bool,
    proof_marked_paths: &HashSet<String>,
) -> Result<(bool, bool)> {
    // A verified Snapshot V2 page set is a complete normalized Summary source.
    if proof_marked_paths.contains(archive.file_path())
        && summary_archive_snapshot_path_has_proof(pool, archive.file_path()).await?
    {
        return Ok((true, true));
    }
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
        let empty_account_ids = HashSet::new();
        let (global_complete, mut account_complete) =
            summary_projection_all_time_manifest_scope_coverage(
                archive,
                replay,
                account_manifest_refreshed,
                account_ids.unwrap_or(&empty_account_ids),
                &hourly_rollup_totals,
            )?;
        if account_ids.is_none() {
            account_complete = false;
        }
        return Ok((global_complete, account_complete));
    }
    // Legacy manifests have no finite range, so replay markers are the bounded proof.
    Ok((
        replay.overall && replay.usage_breakdown,
        archive.has_materialized_historical_rollups()
            && account_ids.is_some()
            && account_manifest_refreshed
            && replay.account_stats
            && replay.usage_breakdown,
    ))
}
