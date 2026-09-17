pub(crate) async fn store_summary_archive_snapshot_v2_final_proof_tx(
    connection: &mut SqliteConnection,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<()> {
    if !summary_archive_snapshot_has_proof_tx(connection, archive_batch_id, manifest_sha256).await?
    {
        bail!("Summary Snapshot V2 semantic proof validation failed");
    }
    let pages = sqlx::query_as::<_, (i64, String, i64, String, String)>(
        "SELECT page_index, snapshot_sha256, row_count, coverage_start, coverage_end \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 \
         ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_all(&mut *connection)
    .await
    .context("load Summary Snapshot V2 final proof metadata")?;
    if pages.is_empty()
        || pages.iter().enumerate().any(|(index, page)| {
            page.0 != i64::try_from(index).unwrap_or(-1)
                || page.2 < 0
                || page.3.trim().is_empty()
                || page.4.trim().is_empty()
        })
    {
        bail!("Summary Snapshot V2 final proof metadata is incomplete");
    }
    let mut semantic_hasher = Sha256::new();
    for (page_index, snapshot_sha256, row_count, coverage_start, coverage_end) in &pages {
        semantic_hasher.update(page_index.to_le_bytes());
        semantic_hasher.update(snapshot_sha256.as_bytes());
        semantic_hasher.update(row_count.to_le_bytes());
        semantic_hasher.update(coverage_start.as_bytes());
        semantic_hasher.update(coverage_end.as_bytes());
    }
    let semantic_sha256 = format!("{:x}", semantic_hasher.finalize());
    let page_count = i64::try_from(pages.len()).context("Summary Snapshot page count overflow")?;
    let row_count = pages.iter().map(|page| page.2).sum::<i64>();
    let coverage_start = pages.first().map(|page| page.3.clone());
    let coverage_end = pages.last().map(|page| page.4.clone());
    sqlx::query(
        "INSERT OR REPLACE INTO summary_archive_snapshot_v2_proof \
         (archive_batch_id, manifest_sha256, page_count, row_count, coverage_start, coverage_end, semantic_sha256, verified_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .bind(page_count)
    .bind(row_count)
    .bind(coverage_start.as_deref())
    .bind(coverage_end.as_deref())
    .bind(&semantic_sha256)
    .execute(&mut *connection)
    .await
    .context("store Summary Snapshot V2 final proof")?;
    sqlx::query(
        "INSERT OR IGNORE INTO summary_coverage_obligation \
         (archive_batch_id, manifest_sha256, coverage_start, coverage_end, state) \
         SELECT id, sha256, coverage_start_at, coverage_end_at, 'pending' \
         FROM archive_batches WHERE id = ?1 AND sha256 = ?2",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .execute(&mut *connection)
    .await
    .context("ensure Summary coverage obligation for final proof")?;
    sqlx::query(
        "UPDATE summary_coverage_obligation \
         SET state = 'resolved', terminal_reason = NULL, resolved_at = datetime('now'), updated_at = datetime('now') \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .execute(&mut *connection)
    .await
    .context("resolve Summary coverage obligation")?;
    Ok(())
}

/// Promote a structurally verified set of V2 pages to durable authority.  Pages are deliberately
/// not authority by themselves: the marker is invalidated by the schema page triggers whenever a
/// page is inserted, updated, or deleted.  This lazy promotion also upgrades pages written by an
/// older release without making an old outcome row authoritative.
pub(crate) async fn ensure_summary_archive_snapshot_v2_final_proof(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if !summary_archive_snapshot_has_proof_tx(tx.as_mut(), archive_batch_id, manifest_sha256)
        .await?
    {
        tx.rollback().await?;
        return Ok(false);
    }
    store_summary_archive_snapshot_v2_final_proof_tx(
        tx.as_mut(),
        archive_batch_id,
        manifest_sha256,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn summary_archive_snapshot_has_final_proof(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let marker = sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT page_count, row_count, semantic_sha256 \
         FROM summary_archive_snapshot_v2_proof \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 LIMIT 1",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_optional(pool)
    .await
    .context("load Summary Snapshot V2 final proof marker")?;
    let Some((marker_page_count, marker_row_count, marker_semantic_sha256)) = marker else {
        return ensure_summary_archive_snapshot_v2_final_proof(
            pool,
            archive_batch_id,
            manifest_sha256,
        )
        .await;
    };
    if !summary_archive_snapshot_has_proof(pool, archive_batch_id, manifest_sha256).await? {
        return Ok(false);
    }
    let pages = sqlx::query_as::<_, (i64, String, i64, String, String)>(
        "SELECT page_index, snapshot_sha256, row_count, coverage_start, coverage_end \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 \
         ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_all(pool)
    .await
    .context("load Summary Snapshot V2 final proof verification metadata")?;
    let mut semantic_hasher = Sha256::new();
    for (page_index, snapshot_sha256, row_count, coverage_start, coverage_end) in &pages {
        semantic_hasher.update(page_index.to_le_bytes());
        semantic_hasher.update(snapshot_sha256.as_bytes());
        semantic_hasher.update(row_count.to_le_bytes());
        semantic_hasher.update(coverage_start.as_bytes());
        semantic_hasher.update(coverage_end.as_bytes());
    }
    let semantic_sha256 = format!("{:x}", semantic_hasher.finalize());
    Ok(
        marker_page_count == i64::try_from(pages.len()).unwrap_or(-1)
            && marker_row_count == pages.iter().map(|page| page.2).sum::<i64>()
            && marker_semantic_sha256 == semantic_sha256,
    )
}

pub(crate) fn parse_snapshot_coverage_at(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|value| value.and_utc())
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummarySourceChangeCompactionProof {
    pub(crate) first_cursor: u64,
    pub(crate) last_cursor: u64,
    pub(crate) proof_kind: String,
    pub(crate) retained_after_cursor: u64,
}

pub(crate) async fn compact_summary_source_change_journal(
    pool: &Pool<Sqlite>,
    through_cursor: u64,
    retained_after_cursor: u64,
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let bounds = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT MIN(cursor), MAX(cursor) FROM summary_source_change_journal \
         WHERE cursor <= ?1 AND cursor < ?2",
    )
    .bind(i64::try_from(through_cursor).context("compaction cursor overflow")?)
    .bind(i64::try_from(retained_after_cursor).context("retained cursor overflow")?)
    .fetch_one(tx.as_mut())
    .await?;
    let (Some(first_cursor), Some(last_cursor)) = bounds else {
        tx.rollback().await?;
        return Ok(false);
    };
    let proof = SummarySourceChangeCompactionProof {
        first_cursor: u64::try_from(first_cursor)?,
        last_cursor: u64::try_from(last_cursor)?,
        proof_kind: "bounded_source_tail_compaction".to_string(),
        retained_after_cursor,
    };
    let proof_json = serde_json::to_string(&proof)?;
    sqlx::query(
        "INSERT INTO summary_source_change_compaction_proof \
         (first_cursor, last_cursor, proof_kind, proof_json, retained_after_cursor) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(first_cursor)
    .bind(last_cursor)
    .bind(&proof.proof_kind)
    .bind(proof_json)
    .bind(i64::try_from(retained_after_cursor).context("retained cursor overflow")?)
    .execute(tx.as_mut())
    .await?;
    sqlx::query("DELETE FROM summary_source_change_journal WHERE cursor <= ?1 AND cursor < ?2")
        .bind(i64::try_from(through_cursor).context("compaction cursor overflow")?)
        .bind(i64::try_from(retained_after_cursor).context("retained cursor overflow")?)
        .execute(tx.as_mut())
        .await?;
    tx.commit().await?;
    Ok(true)
}
