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
          row_count, payload, payload_bytes, snapshot_sha256, format_version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_V1)
    .execute(&mut *connection)
    .await
    .context("store summary archive snapshot page")?;
    tracing::debug!(
        stage = "summary_archive_snapshot_page",
        page_index = page.page_index,
        row_count = page.row_count,
        payload_bytes = page.payload.len(),
        "stored normalized Summary Archive Snapshot page"
    );
    Ok(())
}

/// Store a page that has been decoded and validated against the V2 semantic Summary record
/// contract. Keeping this separate from the legacy writer makes it impossible for an old page
/// to accidentally become a cleanup authority during migration.
pub(crate) async fn store_summary_archive_snapshot_page_v2_tx(
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
    let records = decode_summary_archive_snapshot_v2_payload(&page.payload)?;
    if records.len() != page.row_count as usize
        || records.len() > SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS
    {
        bail!("V2 Summary Snapshot row count does not match payload");
    }
    let coverage_start = parse_snapshot_coverage_at(&page.coverage_start)
        .ok_or_else(|| anyhow::anyhow!("invalid V2 Summary Snapshot coverage start"))?;
    let coverage_end = parse_snapshot_coverage_at(&page.coverage_end)
        .ok_or_else(|| anyhow::anyhow!("invalid V2 Summary Snapshot coverage end"))?;
    // Archive manifests store the last row's timestamp as an inclusive endpoint. A page with a
    // single row therefore legitimately has equal start and end coverage markers.
    if coverage_start > coverage_end
        || records.iter().any(|record| {
            parse_snapshot_coverage_at(&record.occurred_at).is_none_or(|occurred_at| {
                occurred_at < coverage_start || occurred_at > coverage_end
            })
        })
    {
        bail!("V2 Summary Snapshot records exceed page coverage");
    }
    sqlx::query(
        "INSERT OR REPLACE INTO summary_archive_snapshot \
         (archive_batch_id, manifest_sha256, page_index, coverage_start, coverage_end, \
          row_count, payload, payload_bytes, snapshot_sha256, format_version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_V2)
    .execute(&mut *connection)
    .await
    .context("store V2 summary archive snapshot page")?;
    tracing::debug!(
        stage = "summary_archive_snapshot_v2_page",
        page_index = page.page_index,
        row_count = page.row_count,
        payload_bytes = page.payload.len(),
        "stored verified Summary Archive Snapshot V2 page"
    );
    Ok(())
}

pub(crate) async fn summary_archive_snapshot_has_proof(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let mut connection = pool.acquire().await?;
    summary_archive_snapshot_has_proof_tx(&mut connection, archive_batch_id, manifest_sha256).await
}

fn reject_summary_snapshot_proof(archive_batch_id: i64, reason: &'static str) -> Result<bool> {
    tracing::info!(
        stage = "summary_snapshot_v2_proof_rejected",
        archive_batch_id,
        reason,
        "Summary Snapshot V2 proof rejected"
    );
    Ok(false)
}

async fn load_summary_archive_snapshot_manifest(
    connection: &mut SqliteConnection,
    archive_batch_id: i64,
) -> Result<
    Option<(
        String,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    )>,
> {
    sqlx::query_as::<
        _,
        (
            String,
            String,
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        ),
    >(
        "SELECT dataset, sha256, row_count, status, COALESCE(summary_source_kind, 'unknown'), \
             coverage_start_at, coverage_end_at, file_path \
             FROM archive_batches WHERE id = ?1",
    )
    .bind(archive_batch_id)
    .fetch_optional(&mut *connection)
    .await
    .context("load Summary Snapshot archive manifest proof")
}

fn summary_snapshot_records_are_invalid(
    records: &[SummaryArchiveSnapshotV2Record],
    coverage_start: DateTime<Utc>,
    coverage_end: DateTime<Utc>,
    previous_record_key: &mut Option<(DateTime<Utc>, i64)>,
    seen_ids: &mut std::collections::HashSet<i64>,
) -> bool {
    records.iter().any(|record| {
        let Some(occurred_at) = parse_snapshot_coverage_at(&record.occurred_at) else {
            return true;
        };
        let key = (occurred_at, record.id);
        if previous_record_key.is_some_and(|previous| key < previous) {
            return true;
        }
        *previous_record_key = Some(key);
        !seen_ids.insert(record.id) || occurred_at < coverage_start || occurred_at > coverage_end
    })
}

async fn validate_summary_archive_snapshot_pages(
    connection: &mut SqliteConnection,
    archive_batch_id: i64,
    manifest_sha256: &str,
    manifest_row_count: i64,
    manifest_start: Option<DateTime<Utc>>,
    manifest_end: Option<DateTime<Utc>>,
) -> Result<bool> {
    let mut first_page_start = None;
    let mut last_page_end = None;
    let mut previous_page_end = None;
    let mut previous_record_key = None;
    let mut total_rows = 0_i64;
    let mut seen_ids = std::collections::HashSet::new();
    let mut validated_pages = 0_i64;
    let mut rows = sqlx::query(
        "SELECT page_index, snapshot_sha256, payload, coverage_start, coverage_end, payload_bytes, row_count, format_version \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch(&mut *connection);
    while let Some(row) = rows
        .try_next()
        .await
        .context("check summary archive snapshot proof")?
    {
        let expected_page = validated_pages;
        if row.get::<i64, _>("page_index") != expected_page {
            return reject_summary_snapshot_proof(archive_batch_id, "page_order");
        }
        if row.get::<i64, _>("format_version") != SUMMARY_ARCHIVE_SNAPSHOT_V2 {
            return reject_summary_snapshot_proof(archive_batch_id, "format_version");
        }
        let payload = row.get::<Vec<u8>, _>("payload");
        let row_count = row.get::<i64, _>("row_count");
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let computed = format!("{:x}", hasher.finalize());
        if row.get::<String, _>("snapshot_sha256") != computed
            || row.get::<i64, _>("payload_bytes") != i64::try_from(payload.len()).unwrap_or(-1)
            || row_count < 0
            || row.get::<String, _>("coverage_start").trim().is_empty()
            || row.get::<String, _>("coverage_end").trim().is_empty()
        {
            return reject_summary_snapshot_proof(archive_batch_id, "page_integrity_or_coverage");
        }
        let Some(coverage_start) = parse_snapshot_coverage_at(row.get("coverage_start")) else {
            return reject_summary_snapshot_proof(archive_batch_id, "page_coverage_start_parse");
        };
        let Some(coverage_end) = parse_snapshot_coverage_at(row.get("coverage_end")) else {
            return reject_summary_snapshot_proof(archive_batch_id, "page_coverage_end_parse");
        };
        if coverage_start > coverage_end {
            return reject_summary_snapshot_proof(archive_batch_id, "page_coverage_order");
        }
        if previous_page_end.is_some_and(|previous_end| coverage_start < previous_end) {
            return reject_summary_snapshot_proof(archive_batch_id, "page_coverage_overlap");
        }
        first_page_start.get_or_insert(coverage_start);
        last_page_end = Some(coverage_end);
        previous_page_end = Some(coverage_end);
        let records = match decode_summary_archive_snapshot_v2_payload(&payload) {
            Ok(records) => records,
            Err(_) => return reject_summary_snapshot_proof(archive_batch_id, "payload_decode"),
        };
        if i64::try_from(records.len()).unwrap_or(-1) != row_count {
            return reject_summary_snapshot_proof(archive_batch_id, "payload_row_count");
        }
        total_rows = total_rows.saturating_add(row_count);
        if summary_snapshot_records_are_invalid(
            &records,
            coverage_start,
            coverage_end,
            &mut previous_record_key,
            &mut seen_ids,
        ) {
            return reject_summary_snapshot_proof(
                archive_batch_id,
                "record_order_identity_or_coverage",
            );
        }
        validated_pages = validated_pages.saturating_add(1);
    }
    if validated_pages == 0 {
        return reject_summary_snapshot_proof(archive_batch_id, "no_snapshot_pages");
    }
    if total_rows != manifest_row_count
        || manifest_start.is_some_and(|start| first_page_start != Some(start))
        || manifest_end.is_some_and(|end| last_page_end != Some(end))
    {
        return reject_summary_snapshot_proof(archive_batch_id, "manifest_row_count_or_coverage");
    }
    Ok(true)
}

pub(crate) async fn summary_archive_snapshot_has_proof_tx(
    connection: &mut SqliteConnection,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let Some((
        dataset,
        manifest,
        manifest_row_count,
        status,
        source_kind,
        manifest_start,
        manifest_end,
        file_path,
    )) = load_summary_archive_snapshot_manifest(connection, archive_batch_id).await?
    else {
        return Ok(false);
    };
    // Retention validates and records the proof while the archive row is still staged. The
    // surrounding transaction promotes it to completed only after replay markers are written;
    // an error rolls both the proof and staged source back together.
    if dataset != "codex_invocations"
        || manifest != manifest_sha256
        || !matches!(status.as_str(), "completed" | "materializing")
        || source_kind == "live_mirror"
        || manifest_row_count < 0
    {
        return reject_summary_snapshot_proof(archive_batch_id, "manifest_identity_or_status");
    }
    let duplicate_path_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM archive_batches
         WHERE dataset = 'codex_invocations' AND status = 'completed'
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
           AND file_path = ?1 AND id <> ?2",
    )
    .bind(&file_path)
    .bind(archive_batch_id)
    .fetch_one(&mut *connection)
    .await
    .context("check Summary Snapshot manifest path uniqueness")?;
    if duplicate_path_count > 0 {
        return reject_summary_snapshot_proof(archive_batch_id, "duplicate_manifest_path");
    }
    let (page_count, payload_bytes) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_one(&mut *connection)
    .await
    .context("check Summary Snapshot V2 proof bounds")?;
    if page_count > SUMMARY_ARCHIVE_SNAPSHOT_MAX_PROOF_PAGES
        || payload_bytes > SUMMARY_ARCHIVE_SNAPSHOT_MAX_PROOF_PAYLOAD_BYTES
    {
        return reject_summary_snapshot_proof(archive_batch_id, "proof_budget");
    }
    let manifest_start = match manifest_start {
        Some(value) => match parse_snapshot_coverage_at(&value) {
            Some(parsed) => Some(parsed),
            None => {
                return reject_summary_snapshot_proof(
                    archive_batch_id,
                    "manifest_coverage_start_parse",
                );
            }
        },
        None => None,
    };
    let manifest_end = match manifest_end {
        Some(value) => match parse_snapshot_coverage_at(&value) {
            Some(parsed) => Some(parsed),
            None => {
                return reject_summary_snapshot_proof(
                    archive_batch_id,
                    "manifest_coverage_end_parse",
                );
            }
        },
        None => None,
    };
    validate_summary_archive_snapshot_pages(
        connection,
        archive_batch_id,
        manifest_sha256,
        manifest_row_count,
        manifest_start,
        manifest_end,
    )
    .await
}
