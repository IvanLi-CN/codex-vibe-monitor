pub(crate) async fn archive_rows_into_segment_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    day_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive segment requires at least one row id");
    }
    if spec.dataset != "codex_invocations" {
        bail!("archive segment writer only supports codex_invocations");
    }
    let month_key = archive_month_key_from_day_key(day_key)?;
    let part_key = archive_segment_part_key_for_ids(ids)?;
    let final_path = archive_segment_file_path(
        config,
        spec.dataset,
        day_key,
        &part_key,
        config.invocation_archive_codec,
    )?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!(
        "{}.{}.partial.sqlite",
        final_path.display(),
        suffix
    ));
    let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", final_path.display(), suffix));
    let _temp_cleanup = TempSqliteCleanup(work_path.clone());
    let _gzip_cleanup = TempSqliteCleanup(temp_gzip_path.clone());
    ensure_attachable_archive_sqlite_path(&work_path)?;
    let row_count = async {
        let mut conn = pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ?1 AS archive_db")
            .bind(work_path.to_string_lossy().to_string())
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!("failed to attach archive database {}", work_path.display())
            })?;
        sqlx::query("PRAGMA archive_db.journal_mode=DELETE")
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!(
                    "failed to switch archive database {} to DELETE journal mode",
                    work_path.display()
                )
            })?;
        sqlx::query(spec.create_sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to ensure archive schema for {}", spec.dataset))?;
        ensure_codex_invocations_archive_schema(&mut conn).await?;

        let mut upstream_last_activity = Vec::new();
        for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS account_id, occurred_at FROM main.codex_invocations WHERE id IN (",
            );
            {
                let mut separated = query.separated(", ");
                for id in chunk {
                    separated.push_bind(id);
                }
            }
            query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
            upstream_last_activity.extend(
                query
                    .build_query_as::<ArchivedAccountLastActivityRow>()
                    .fetch_all(&mut *conn)
                    .await?,
            );
        }
        let upstream_last_activity = dedupe_archive_upstream_last_activity(
            upstream_last_activity
                .into_iter()
                .map(|row| (row.account_id, row.last_activity_at)),
        );

        let mut insert = QueryBuilder::<Sqlite>::new(format!(
            "INSERT OR IGNORE INTO archive_db.{} ({}) SELECT {} FROM main.{} WHERE id IN (",
            spec.dataset, spec.columns, spec.columns, spec.dataset
        ));
        {
            let mut separated = insert.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
        }
        insert.push(")");
        insert.build().execute(&mut *conn).await.with_context(|| {
            format!(
                "failed to copy rows into archive segment for {}",
                spec.dataset
            )
        })?;

        let count_query = format!("SELECT COUNT(*) FROM archive_db.{}", spec.dataset);
        let row_count = sqlx::query_scalar::<_, i64>(&count_query)
            .fetch_one(&mut *conn)
            .await
            .with_context(|| format!("failed to count archive rows for {}", spec.dataset))?;
        sqlx::query("DETACH DATABASE archive_db")
            .execute(&mut *conn)
            .await
            .context("failed to detach archive database")?;
        Ok::<(i64, Vec<(i64, String)>), anyhow::Error>((row_count, upstream_last_activity))
    }
    .await?;

    finalize_archive_sqlite_file(&work_path).await?;

    deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path)?;
    publish_prepared_archive_file(&temp_gzip_path, &final_path)?;

    let sha256 = sha256_hex_file(&final_path)?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key,
        day_key: Some(day_key.to_string()),
        part_key: Some(part_key),
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: row_count.0,
        upstream_last_activity: row_count.1,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
        layout: ARCHIVE_LAYOUT_SEGMENT_V1,
        codec: config.invocation_archive_codec.as_str(),
        writer_version: ARCHIVE_WRITER_VERSION_SEGMENT_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    })
}

pub(crate) async fn upsert_archived_upstream_last_activity(
    tx: &mut sqlx::SqliteConnection,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    for (account_id, occurred_at) in &deduped_values {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                    WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                    ELSE last_activity_at
                END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

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
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            day_key,
            part_key,
            file_path,
            sha256,
            row_count,
            status,
            layout,
            codec,
            writer_version,
            cleanup_state,
            superseded_by,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            summary_source_kind,
            created_at
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TempArchiveTestDirectory(std::path::PathBuf);

    impl Drop for TempArchiveTestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn segment_part_key_is_stable_for_one_prepared_identity() {
        let forward = archive_segment_part_key_for_ids(&[7, 3, 5]).expect("part key");
        let reordered = archive_segment_part_key_for_ids(&[5, 7, 3]).expect("part key");
        let distinct = archive_segment_part_key_for_ids(&[3, 5, 8]).expect("part key");

        assert_eq!(forward, reordered);
        assert_ne!(forward, distinct);
    }

    #[test]
    fn prepared_archive_publish_rejects_conflicting_existing_identity() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-publish-{}",
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create archive root");
        let temporary = root.join("prepared.sqlite.gz");
        let final_path = root.join("part.sqlite.gz");
        fs::write(&temporary, b"prepared").expect("write prepared archive");
        fs::write(&final_path, b"different").expect("write conflicting archive");

        let error = publish_prepared_archive_file(&temporary, &final_path)
            .expect_err("conflicting archive identity must fail");
        assert!(error.to_string().contains("identity collision"));
        assert!(temporary.exists());
        assert_eq!(
            fs::read(&final_path).expect("read final archive"),
            b"different"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepared_archive_publish_reuses_a_matching_identity() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-publish-{}",
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create archive root");
        let first_temporary = root.join("first.sqlite.gz");
        let retry_temporary = root.join("retry.sqlite.gz");
        let final_path = root.join("part.sqlite.gz");
        fs::write(&first_temporary, b"prepared").expect("write prepared archive");
        publish_prepared_archive_file(&first_temporary, &final_path)
            .expect("publish first prepared archive");
        fs::write(&retry_temporary, b"prepared").expect("write retry archive");
        publish_prepared_archive_file(&retry_temporary, &final_path)
            .expect("reuse matching prepared archive");

        assert!(!first_temporary.exists());
        assert!(!retry_temporary.exists());
        assert_eq!(
            fs::read(&final_path).expect("read final archive"),
            b"prepared"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn archive_finalization_skips_full_vacuum() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-archive-finalization-{}",
            retention_temp_suffix()
        ));
        let _root_cleanup = TempArchiveTestDirectory(root.clone());
        fs::create_dir_all(&root).expect("create archive root");
        let path = root.join("periodic-maintenance.sqlite");

        let mut connection = open_archive_sqlite_connection(&path)
            .await
            .expect("open archive sqlite file");
        sqlx::query("CREATE TABLE archive_payloads (payload BLOB NOT NULL)")
            .execute(&mut connection)
            .await
            .expect("create archive payload table");
        sqlx::query("INSERT INTO archive_payloads (payload) VALUES (?1)")
            .bind(vec![0_u8; 512 * 1024])
            .execute(&mut connection)
            .await
            .expect("insert archive payload");
        sqlx::query("DELETE FROM archive_payloads")
            .execute(&mut connection)
            .await
            .expect("delete archive payload");
        let free_pages_before: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut connection)
            .await
            .expect("read archive freelist before finalization");
        assert!(
            free_pages_before > 0,
            "fixture must contain reclaimable pages before archive finalization"
        );
        connection.close().await.expect("close archive sqlite file");

        finalize_archive_sqlite_file(&path)
            .await
            .expect("finalize archive sqlite file");

        let mut connection = open_archive_sqlite_connection(&path)
            .await
            .expect("reopen archive sqlite file");
        let free_pages_after: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut connection)
            .await
            .expect("read archive freelist after finalization");
        connection
            .close()
            .await
            .expect("close inspected archive sqlite file");
        let preserved_reclaimable_pages = free_pages_after > 0;

        assert!(
            preserved_reclaimable_pages,
            "periodic archive finalization must not run a full VACUUM"
        );
    }

    #[tokio::test]
    async fn legacy_archive_replacement_keeps_pending_cleanup_when_rename_fails() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let final_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-legacy-archive-replace-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_temp_path = final_path.with_extension("missing.tmp");
        fs::write(&final_path, b"old archive content").expect("write old archive file");
        let old_sha256 = sha256_hex_file(&final_path).expect("hash old archive file");
        let final_path_string = final_path.to_string_lossy().to_string();
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                cleanup_state,
                cleanup_source_safe_start_date,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', 'delete_pending', '2025-01-04', datetime('now'))
            "#,
        )
        .bind(&final_path_string)
        .bind(&old_sha256)
        .execute(&pool)
        .await
        .expect("insert pending archive manifest");

        replace_legacy_archive_file_with_cleanup_serialization(
            &pool,
            "codex_quota_snapshots",
            "2025-01",
            &missing_temp_path,
            &final_path,
        )
        .await
        .expect_err("missing replacement file must roll back pending reactivation");

        assert_eq!(
            fs::read(&final_path).expect("read original archive"),
            b"old archive content"
        );
        let manifest: (String, Option<String>) = sqlx::query_as(
            "SELECT cleanup_state, cleanup_source_safe_start_date FROM archive_batches WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load retained pending manifest");
        assert_eq!(manifest.0, ARCHIVE_CLEANUP_STATE_DELETE_PENDING);
        assert_eq!(manifest.1.as_deref(), Some("2025-01-04"));

        let _ = fs::remove_file(&final_path);
    }
}
