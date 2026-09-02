//! Durable, compact source-change descriptors used to recover Summary projections.
//!
//! The journal deliberately stores identity and reconstruction metadata only.  Raw request or
//! response text, previews, and duplicate Summary rows never enter this table.  Terminal source
//! writes append a descriptor through the same SQLite transaction as the source row; readers can
//! therefore treat a committed descriptor as an exact durable tail.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite, SqliteConnection};

pub(crate) const SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES: usize = 10_000;
pub(crate) const SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummarySourceChangeEntry {
    pub(crate) row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) current_rank: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummarySourceChangeDescriptor {
    pub(crate) version: i64,
    pub(crate) source_kind: String,
    pub(crate) source_revision: u64,
    pub(crate) first_row_id: i64,
    pub(crate) last_row_id: i64,
    pub(crate) occurred_start: String,
    pub(crate) occurred_end: String,
    pub(crate) entries: Vec<SummarySourceChangeEntry>,
}

impl SummarySourceChangeDescriptor {
    pub(crate) fn terminal_batch(
        source_revision: u64,
        entries: Vec<SummarySourceChangeEntry>,
    ) -> Result<Self> {
        let first = entries
            .first()
            .ok_or_else(|| anyhow::anyhow!("summary source descriptor cannot be empty"))?;
        let last = entries.last().unwrap_or(first);
        let occurred_start = entries
            .iter()
            .map(|entry| entry.occurred_at.as_str())
            .min()
            .unwrap_or_default()
            .to_string();
        let occurred_end = entries
            .iter()
            .map(|entry| entry.occurred_at.as_str())
            .max()
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            version: SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION,
            source_kind: "terminal_batch".to_string(),
            source_revision,
            first_row_id: first.row_id,
            last_row_id: last.row_id,
            occurred_start,
            occurred_end,
            entries,
        })
    }

    pub(crate) fn encoded_len(&self) -> Result<usize> {
        Ok(serde_json::to_vec(self)?.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySourceChangeRecord {
    pub(crate) cursor: u64,
    pub(crate) descriptor: SummarySourceChangeDescriptor,
}

/// A compressed, normalized Summary page for one authoritative archive.  The caller is
/// responsible for encoding only fields needed by the Summary reducers; raw payloads and preview
/// text are intentionally rejected by convention and bounded by the page byte limit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryArchiveSnapshotPage {
    pub(crate) archive_batch_id: i64,
    pub(crate) manifest_sha256: String,
    pub(crate) page_index: u32,
    pub(crate) coverage_start: String,
    pub(crate) coverage_end: String,
    pub(crate) row_count: u32,
    pub(crate) payload: Vec<u8>,
}

impl SummaryArchiveSnapshotPage {
    pub(crate) fn snapshot_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.payload);
        format!("{:x}", hasher.finalize())
    }
}

pub(crate) async fn store_summary_archive_snapshot_page_tx(
    connection: &mut SqliteConnection,
    page: &SummaryArchiveSnapshotPage,
) -> Result<()> {
    if page.payload.len() > SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES {
        bail!("summary archive snapshot page byte budget exceeded");
    }
    if page.manifest_sha256.trim().is_empty()
        || page.coverage_start.trim().is_empty()
        || page.coverage_end.trim().is_empty()
    {
        bail!("summary archive snapshot page is missing identity or coverage proof");
    }
    sqlx::query(
        "INSERT OR REPLACE INTO summary_archive_snapshot \
         (archive_batch_id, manifest_sha256, page_index, coverage_start, coverage_end, \
          row_count, payload, payload_bytes, snapshot_sha256) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(page.archive_batch_id)
    .bind(&page.manifest_sha256)
    .bind(i64::from(page.page_index))
    .bind(&page.coverage_start)
    .bind(&page.coverage_end)
    .bind(i64::from(page.row_count))
    .bind(&page.payload)
    .bind(i64::try_from(page.payload.len()).context("snapshot byte count overflow")?)
    .bind(page.snapshot_sha256())
    .execute(&mut *connection)
    .await
    .context("store summary archive snapshot page")?;
    Ok(())
}

pub(crate) async fn summary_archive_snapshot_has_proof(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let row = sqlx::query(
        "SELECT snapshot_sha256, payload, coverage_start, coverage_end, payload_bytes \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND page_index = 0",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_optional(pool)
    .await
    .context("check summary archive snapshot proof")?;
    let Some(row) = row else {
        return Ok(false);
    };
    let payload = row.get::<Vec<u8>, _>("payload");
    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let computed = format!("{:x}", hasher.finalize());
    Ok(row.get::<String, _>("snapshot_sha256") == computed
        && row.get::<i64, _>("payload_bytes") == i64::try_from(payload.len()).unwrap_or(-1)
        && !row.get::<String, _>("coverage_start").trim().is_empty()
        && !row.get::<String, _>("coverage_end").trim().is_empty())
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
         (first_cursor, last_cursor, proof_kind, proof_json) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(first_cursor)
    .bind(last_cursor)
    .bind(&proof.proof_kind)
    .bind(proof_json)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        sqlx::query(
            "CREATE TABLE summary_source_change_journal (cursor INTEGER PRIMARY KEY AUTOINCREMENT, descriptor_version INTEGER NOT NULL, source_kind TEXT NOT NULL, source_revision INTEGER NOT NULL, first_row_id INTEGER NOT NULL, last_row_id INTEGER NOT NULL, occurred_start TEXT NOT NULL, occurred_end TEXT NOT NULL, descriptor_json TEXT NOT NULL, descriptor_bytes INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("journal table");
        sqlx::query(
            "CREATE TABLE summary_source_change_compaction_proof (id INTEGER PRIMARY KEY AUTOINCREMENT, first_cursor INTEGER NOT NULL, last_cursor INTEGER NOT NULL, proof_kind TEXT NOT NULL, proof_json TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("proof table");
        sqlx::query(
            "CREATE TABLE summary_archive_snapshot (archive_batch_id INTEGER NOT NULL, manifest_sha256 TEXT NOT NULL, page_index INTEGER NOT NULL, coverage_start TEXT NOT NULL, coverage_end TEXT NOT NULL, row_count INTEGER NOT NULL, payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL, snapshot_sha256 TEXT NOT NULL, PRIMARY KEY (archive_batch_id, manifest_sha256, page_index))",
        )
        .execute(&pool)
        .await
        .expect("snapshot table");
        pool
    }

    fn descriptor(row_id: i64) -> SummarySourceChangeDescriptor {
        SummarySourceChangeDescriptor::terminal_batch(
            1,
            vec![SummarySourceChangeEntry {
                row_id,
                invoke_id: format!("invoke-{row_id}"),
                occurred_at: "2026-08-09 12:00:00".to_string(),
                upstream_account_id: Some(7),
                current_rank: Some(1),
            }],
        )
        .expect("descriptor")
    }

    #[tokio::test]
    async fn descriptor_tail_round_trips_and_compacts_with_proof() {
        let pool = pool().await;
        let mut tx = pool.begin().await.expect("transaction");
        append_summary_source_change_descriptor_tx(tx.as_mut(), &descriptor(42))
            .await
            .expect("append descriptor");
        tx.commit().await.expect("commit descriptor");
        let tail = load_summary_source_change_tail(&pool, 0, 10)
            .await
            .expect("load tail");
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].descriptor.entries[0].row_id, 42);
        assert!(
            compact_summary_source_change_journal(&pool, 1, 2)
                .await
                .expect("compact tail")
        );
        let proofs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM summary_source_change_compaction_proof",
        )
        .fetch_one(&pool)
        .await
        .expect("count proof");
        assert_eq!(proofs, 1);
    }

    #[tokio::test]
    async fn archive_snapshot_cleanup_gate_requires_matching_sha() {
        let pool = pool().await;
        let page = SummaryArchiveSnapshotPage {
            archive_batch_id: 9,
            manifest_sha256: "manifest-9".to_string(),
            page_index: 0,
            coverage_start: "2026-08-01T00:00:00Z".to_string(),
            coverage_end: "2026-08-01T01:00:00Z".to_string(),
            row_count: 1,
            payload: b"normalized-summary".to_vec(),
        };
        let mut tx = pool.begin().await.expect("snapshot transaction");
        store_summary_archive_snapshot_page_tx(tx.as_mut(), &page)
            .await
            .expect("store snapshot");
        tx.commit().await.expect("commit snapshot");
        assert!(
            summary_archive_snapshot_has_proof(&pool, 9, "manifest-9")
                .await
                .expect("snapshot proof")
        );
        assert!(
            !summary_archive_snapshot_has_proof(&pool, 9, "other-manifest")
                .await
                .expect("mismatched proof")
        );
    }
}

pub(crate) async fn append_summary_source_change_descriptor_tx(
    connection: &mut SqliteConnection,
    descriptor: &SummarySourceChangeDescriptor,
) -> Result<u64> {
    if descriptor.version != SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION {
        bail!(
            "unsupported summary source descriptor version {}",
            descriptor.version
        );
    }
    if descriptor.entries.len() > SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES {
        bail!("summary source descriptor entry budget exceeded");
    }
    let descriptor_json = serde_json::to_string(descriptor)?;
    let descriptor_bytes = descriptor_json.len();
    if descriptor_bytes > SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES {
        bail!("summary source descriptor byte budget exceeded");
    }
    compact_source_change_tail_before_insert(connection, descriptor_bytes).await?;
    let result = sqlx::query(
        "INSERT INTO summary_source_change_journal \
         (descriptor_version, source_kind, source_revision, first_row_id, last_row_id, \
          occurred_start, occurred_end, descriptor_json, descriptor_bytes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(descriptor.version)
    .bind(&descriptor.source_kind)
    .bind(i64::try_from(descriptor.source_revision).context("source revision overflow")?)
    .bind(descriptor.first_row_id)
    .bind(descriptor.last_row_id)
    .bind(&descriptor.occurred_start)
    .bind(&descriptor.occurred_end)
    .bind(descriptor_json)
    .bind(i64::try_from(descriptor_bytes).context("descriptor byte count overflow")?)
    .execute(&mut *connection)
    .await
    .context("append summary source change descriptor")?;
    u64::try_from(result.last_insert_rowid()).context("summary source cursor overflow")
}

async fn compact_source_change_tail_before_insert(
    connection: &mut SqliteConnection,
    incoming_bytes: usize,
) -> Result<()> {
    let (mut count, mut bytes) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(descriptor_bytes), 0) \
         FROM summary_source_change_journal",
    )
    .fetch_one(&mut *connection)
    .await?;
    let mut first_cursor = None;
    let mut last_cursor = None;
    let mut removed = 0_i64;
    while count >= SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES as i64
        || bytes.saturating_add(i64::try_from(incoming_bytes).unwrap_or(i64::MAX))
            > SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES as i64
    {
        let Some((cursor, descriptor_bytes)) = sqlx::query_as::<_, (i64, i64)>(
            "SELECT cursor, descriptor_bytes FROM summary_source_change_journal \
             ORDER BY cursor ASC LIMIT 1",
        )
        .fetch_optional(&mut *connection)
        .await?
        else {
            break;
        };
        sqlx::query("DELETE FROM summary_source_change_journal WHERE cursor = ?1")
            .bind(cursor)
            .execute(&mut *connection)
            .await?;
        first_cursor.get_or_insert(cursor);
        last_cursor = Some(cursor);
        removed += 1;
        count = count.saturating_sub(1);
        bytes = bytes.saturating_sub(descriptor_bytes);
    }
    if removed > 0 {
        let first_cursor = first_cursor.expect("removed descriptor has first cursor");
        let last_cursor = last_cursor.expect("removed descriptor has last cursor");
        let proof = serde_json::json!({
            "firstCursor": first_cursor,
            "lastCursor": last_cursor,
            "retainedAfterCursor": last_cursor.saturating_add(1),
            "removedEntries": removed,
        });
        sqlx::query(
            "INSERT INTO summary_source_change_compaction_proof \
             (first_cursor, last_cursor, proof_kind, proof_json) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(first_cursor)
        .bind(last_cursor)
        .bind("bounded_source_tail_compaction")
        .bind(proof.to_string())
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

pub(crate) async fn load_summary_source_change_tail(
    pool: &Pool<Sqlite>,
    after_cursor: u64,
    limit: usize,
) -> Result<Vec<SummarySourceChangeRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT cursor, descriptor_json FROM summary_source_change_journal \
         WHERE cursor > ?1 ORDER BY cursor ASC LIMIT ?2",
    )
    .bind(i64::try_from(after_cursor).context("source cursor overflow")?)
    .bind(i64::try_from(limit).context("source journal limit overflow")?)
    .fetch_all(pool)
    .await
    .context("load summary source change tail")?;
    rows.into_iter()
        .map(|row| {
            let cursor = u64::try_from(row.get::<i64, _>("cursor"))
                .context("summary source cursor is negative")?;
            let descriptor = serde_json::from_str::<SummarySourceChangeDescriptor>(
                row.get::<String, _>("descriptor_json").as_str(),
            )?;
            Ok(SummarySourceChangeRecord { cursor, descriptor })
        })
        .collect()
}

pub(crate) async fn summary_source_change_cursor(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(cursor) FROM summary_source_change_journal",
    )
    .fetch_one(pool)
    .await
    .context("load summary source change cursor")?
    .unwrap_or_default();
    u64::try_from(cursor).context("summary source cursor is negative")
}

pub(crate) fn descriptor_occurred_at(entry: &SummarySourceChangeEntry) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&entry.occurred_at)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
