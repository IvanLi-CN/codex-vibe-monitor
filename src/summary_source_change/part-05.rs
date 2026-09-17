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
    tracing::debug!(
        stage = "summary_source_change_descriptor",
        source_kind = %descriptor.source_kind,
        entry_count = descriptor.entries.len(),
        descriptor_bytes,
        "stored durable Summary Source Change descriptor"
    );
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
             (first_cursor, last_cursor, proof_kind, proof_json, retained_after_cursor) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(first_cursor)
        .bind(last_cursor)
        .bind("bounded_source_tail_compaction")
        .bind(proof.to_string())
        .bind(last_cursor.saturating_add(1))
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

/// Return the last source-journal cursor that a process durably acknowledged after publishing
/// its reconstructed projection.  The checkpoint is deliberately allowed to lag the journal:
/// replaying an already-consumed descriptor is harmless because projection identity suppresses
/// duplicates, while advancing it before publication could lose a tail after a crash.
pub(crate) async fn load_summary_source_change_checkpoint(
    pool: &Pool<Sqlite>,
    scope: &str,
) -> Result<u64> {
    let cursor = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT cursor FROM summary_source_change_cursor WHERE scope = ?1",
    )
    .bind(scope)
    .fetch_optional(pool)
    .await
    .context("load summary source change checkpoint")?
    .flatten()
    .unwrap_or_default();
    u64::try_from(cursor).context("summary source checkpoint cursor is negative")
}

pub(crate) async fn store_summary_source_change_checkpoint(
    pool: &Pool<Sqlite>,
    scope: &str,
    cursor: u64,
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("begin summary source checkpoint")?;
    store_summary_source_change_checkpoint_tx(tx.as_mut(), scope, cursor).await?;
    tx.commit()
        .await
        .context("commit summary source checkpoint")?;
    Ok(())
}

pub(crate) async fn store_summary_source_change_checkpoint_tx(
    connection: &mut SqliteConnection,
    scope: &str,
    cursor: u64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_source_change_cursor (scope, cursor, updated_at) VALUES (?1, ?2, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET cursor = MAX(summary_source_change_cursor.cursor, excluded.cursor), updated_at = datetime('now')",
    )
    .bind(scope)
    .bind(i64::try_from(cursor).context("summary source checkpoint cursor overflow")?)
    .execute(&mut *connection)
    .await
    .context("store summary source change checkpoint")?;
    Ok(())
}

pub(crate) fn descriptor_occurred_at(entry: &SummarySourceChangeEntry) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&entry.occurred_at)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
