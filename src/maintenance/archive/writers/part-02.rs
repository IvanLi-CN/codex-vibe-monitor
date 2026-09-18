pub(crate) async fn upsert_archive_batch_manifest(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<()> {
    upsert_archive_batch_manifest_with_status(tx, batch, ARCHIVE_STATUS_COMPLETED).await
}

pub(crate) async fn stage_invocation_archive_batch_manifest(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<()> {
    if batch.dataset != HOURLY_ROLLUP_DATASET_INVOCATIONS
        || batch.summary_source_kind != SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
    {
        bail!(
            "only an authoritative codex_invocations archive can enter Summary publication staging"
        );
    }
    upsert_archive_batch_manifest_with_status(tx, batch, ARCHIVE_STATUS_MATERIALIZING).await
}

pub(crate) async fn finalize_invocation_archive_batch_publication_tx(
    tx: &mut sqlx::SqliteConnection,
    file_path: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE archive_batches
        SET status = ?1
        WHERE dataset = ?2
          AND file_path = ?3
          AND status = ?4
          AND summary_source_kind = ?5
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .bind(ARCHIVE_STATUS_MATERIALIZING)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        bail!("staged invocation archive was not available for publication: {file_path}");
    }
    Ok(())
}

async fn upsert_archive_batch_manifest_with_status(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
    status: &str,
) -> Result<()> {
    insert_archive_batch_manifest_row(tx, batch, status).await?;
    let deduped_upstream_last_activity =
        dedupe_archive_upstream_last_activity(batch.upstream_last_activity.iter().cloned());
    let archive_batch_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = ?1
          AND month_key = ?2
          AND file_path = ?3
        LIMIT 1
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(&batch.file_path)
    .fetch_one(&mut *tx)
    .await?;
    if batch.dataset == "codex_invocations" {
        write_archive_batch_upstream_activity(
            tx,
            archive_batch_id,
            &deduped_upstream_last_activity,
        )
        .await?;
        let descriptor = SummarySourceChangeDescriptor::coverage_change(
            u64::try_from(archive_batch_id).context("archive batch source revision overflow")?,
            vec![SummarySourceChangeEntry {
                row_id: archive_batch_id,
                invoke_id: format!("archive:{}", batch.sha256),
                occurred_at: batch
                    .coverage_start_at
                    .clone()
                    .unwrap_or_else(|| batch.month_key.clone()),
                upstream_account_id: None,
                current_rank: None,
            }],
        )?;
        append_summary_source_change_descriptor_tx(tx, &descriptor).await?;
    }
    if batch.dataset == "codex_invocations" && !deduped_upstream_last_activity.is_empty() {
        upsert_archived_upstream_last_activity(tx, &deduped_upstream_last_activity).await?;
    }
    Ok(())
}

async fn insert_archive_batch_manifest_row(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
    status: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, day_key, part_key, file_path, sha256, row_count, status,
            layout, codec, writer_version, cleanup_state, superseded_by, coverage_start_at,
            coverage_end_at, archive_expires_at, summary_source_kind, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'))
        ON CONFLICT(dataset, month_key, file_path) DO UPDATE SET
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            sha256 = excluded.sha256,
            row_count = excluded.row_count,
            status = excluded.status,
            layout = excluded.layout,
            codec = excluded.codec,
            writer_version = excluded.writer_version,
            cleanup_state = excluded.cleanup_state,
            cleanup_source_safe_start_date = NULL,
            superseded_by = excluded.superseded_by,
            coverage_start_at = CASE
                WHEN archive_batches.coverage_start_at IS NULL THEN excluded.coverage_start_at
                WHEN excluded.coverage_start_at IS NULL THEN archive_batches.coverage_start_at
                ELSE MIN(archive_batches.coverage_start_at, excluded.coverage_start_at)
            END,
            coverage_end_at = CASE
                WHEN archive_batches.coverage_end_at IS NULL THEN excluded.coverage_end_at
                WHEN excluded.coverage_end_at IS NULL THEN archive_batches.coverage_end_at
                ELSE MAX(archive_batches.coverage_end_at, excluded.coverage_end_at)
            END,
            archive_expires_at = excluded.archive_expires_at,
            summary_source_kind = excluded.summary_source_kind,
            created_at = datetime('now')
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(batch.day_key.as_deref())
    .bind(batch.part_key.as_deref())
    .bind(&batch.file_path)
    .bind(&batch.sha256)
    .bind(batch.row_count)
    .bind(status)
    .bind(batch.layout)
    .bind(batch.codec)
    .bind(batch.writer_version)
    .bind(batch.cleanup_state)
    .bind(batch.superseded_by)
    .bind(batch.coverage_start_at.as_deref())
    .bind(batch.coverage_end_at.as_deref())
    .bind(batch.archive_expires_at.as_deref())
    .bind(batch.summary_source_kind)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn load_archive_batch_id_for_file_tx(
    tx: &mut sqlx::SqliteConnection,
    dataset: &str,
    month_key: &str,
    file_path: &str,
) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = ?1
          AND month_key = ?2
          AND file_path = ?3
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(month_key)
    .bind(file_path)
    .fetch_one(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to load archive batch id for dataset={dataset} month_key={month_key} file_path={file_path}"
        )
    })
}

pub(crate) async fn write_archive_batch_upstream_activity(
    tx: &mut sqlx::SqliteConnection,
    archive_batch_id: i64,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
        .bind(archive_batch_id)
        .execute(&mut *tx)
        .await?;
    for chunk in deduped_values.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
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
        insert.build().execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') WHERE id = ?1",
    )
    .bind(archive_batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
