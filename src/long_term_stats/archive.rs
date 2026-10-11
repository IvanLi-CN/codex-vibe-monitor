use super::*;

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermInvocationRow {
    pub(super) id: i64,
    pub(super) invoke_id: Option<String>,
    pub(super) occurred_at: String,
    pub(super) status: Option<String>,
    pub(super) model: Option<String>,
    pub(super) request_model: Option<String>,
    pub(super) response_model: Option<String>,
    pub(super) reasoning_effort: Option<String>,
    pub(super) upstream_account_id: Option<i64>,
    pub(super) upstream_account_kind: Option<String>,
    pub(super) upstream_account_name: Option<String>,
    pub(super) total_tokens: Option<i64>,
    pub(super) output_tokens: Option<i64>,
    pub(super) cost: Option<f64>,
    pub(super) t_total_ms: Option<f64>,
    pub(super) t_req_read_ms: Option<f64>,
    pub(super) t_req_parse_ms: Option<f64>,
    pub(super) t_upstream_connect_ms: Option<f64>,
    pub(super) t_upstream_ttfb_ms: Option<f64>,
    pub(super) t_upstream_stream_ms: Option<f64>,
    pub(super) error_message: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermArchiveAttemptRow {
    pub(super) invoke_id: String,
    pub(super) occurred_at: String,
    pub(super) upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermSourceTimingRow {
    pub(super) invoke_id: Option<String>,
    pub(super) occurred_at: String,
    pub(super) t_total_ms: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermAttemptArchivePath {
    pub(super) file_path: String,
    pub(super) sha256: Option<String>,
    pub(super) coverage_start_at: Option<String>,
    pub(super) coverage_end_at: Option<String>,
}

pub(super) async fn load_long_term_archive_attempt_accounts(
    pool: &Pool<Sqlite>,
    date_range: Option<(NaiveDate, NaiveDate)>,
) -> Result<(HashMap<(String, String), i64>, HashSet<(String, String)>)> {
    let has_manifest_sha = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('archive_batches') WHERE name = 'sha256')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or_default()
        != 0;
    let archive_query = if has_manifest_sha {
        "SELECT file_path, sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND status = ?1 AND sha256 IS NOT NULL AND TRIM(sha256) <> '' ORDER BY month_key ASC, created_at ASC, id ASC"
    } else {
        "SELECT file_path, NULL AS sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND status = ?1 ORDER BY month_key ASC, created_at ASC, id ASC"
    };
    let paths = match sqlx::query_as::<_, LongTermAttemptArchivePath>(archive_query)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .fetch_all(pool)
        .await
    {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => {
            return Ok((HashMap::new(), HashSet::new()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut accounts = HashMap::new();
    let mut consumed_archives = HashSet::new();
    for archive_path in paths.into_iter().filter(|path| {
        date_range.is_none_or(|(start, end)| {
            let Some(path_start) = path
                .coverage_start_at
                .as_deref()
                .and_then(long_term_archive_end_date)
            else {
                return true;
            };
            let path_end = path
                .coverage_end_at
                .as_deref()
                .and_then(long_term_archive_end_date)
                .unwrap_or(path_start);
            path_end >= start && path_start <= end
        })
    }) {
        let Some(manifest_sha) = archive_path.sha256.clone() else {
            continue;
        };
        ensure_long_term_archive_source_identity(
            pool,
            "pool_upstream_request_attempts",
            &archive_path.file_path,
            &manifest_sha,
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
        let attempt_archive = open_pool_upstream_request_attempt_archive_batch_pool(
            &ArchiveBatchPathRow::from_file_path(archive_path.file_path.clone()),
            "long-term-stats-attempt-fallback",
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
        let Some((archive_pool, cleanup)) = attempt_archive else {
            bail!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path,
            );
        };
        let mut query = String::from(
            r#"
            SELECT invoke_id, occurred_at, upstream_account_id
            FROM pool_upstream_request_attempts
            WHERE upstream_account_id IS NOT NULL
            "#,
        );
        if date_range.is_some() {
            query.push_str(" AND occurred_at >= ?1 AND occurred_at < ?2");
        }
        query.push_str(" ORDER BY id ASC");
        let mut statement = sqlx::query_as::<_, LongTermArchiveAttemptRow>(&query);
        if let Some((start, end)) = date_range {
            statement = statement
                .bind(format!("{start} 00:00:00"))
                .bind(format!("{} 00:00:00", end.succ_opt().unwrap_or(end)));
        }
        let rows = statement.fetch_all(&archive_pool).await;
        archive_pool.close().await;
        drop(cleanup);
        ensure_long_term_archive_source_identity(
            pool,
            "pool_upstream_request_attempts",
            &archive_path.file_path,
            &manifest_sha,
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
        match rows {
            Ok(rows) => {
                for row in rows {
                    if let Some(account_id) = row.upstream_account_id {
                        accounts.insert((row.invoke_id, row.occurred_at), account_id);
                    }
                }
                consumed_archives.insert((archive_path.file_path, manifest_sha.to_string()));
            }
            Err(error) if error.to_string().contains("no such table") => {
                consumed_archives.insert((archive_path.file_path, manifest_sha.to_string()));
            }
            Err(error) => {
                bail!(
                    "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}: {error}",
                    archive_path.file_path
                );
            }
        }
    }
    Ok((accounts, consumed_archives))
}

pub(super) fn hydrate_long_term_archive_attempt_account(
    row: &mut LongTermInvocationRow,
    attempt_accounts: &HashMap<(String, String), i64>,
) {
    if row.upstream_account_id.is_none()
        && let Some(invoke_id) = row.invoke_id.as_ref()
        && let Some(account_id) =
            attempt_accounts.get(&(invoke_id.clone(), row.occurred_at.clone()))
    {
        row.upstream_account_id = Some(*account_id);
    }
}

pub(super) async fn long_term_archive_invocation_query(pool: &Pool<Sqlite>) -> Result<String> {
    let query = long_term_archive_invocation_query_parts(pool).await?;
    Ok(format!(
        "{} WHERE {} ORDER BY occurred_at ASC, id ASC",
        query.select, query.terminal_filter
    ))
}

#[derive(Debug)]
pub(super) struct LongTermArchiveInvocationRangeQueries {
    pub(super) canonical: String,
    pub(super) crossing_text: String,
    pub(super) rfc3339: String,
    pub(super) parts: LongTermArchiveInvocationQueryParts,
}

#[derive(Debug)]
pub(super) struct LongTermArchiveInvocationQueryParts {
    pub(super) select: String,
    pub(super) terminal_filter: String,
    pub(super) status_column: String,
    pub(super) t_total_ms_column: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LongTermArchiveCompatibility {
    pub(super) has_legacy_crossing: bool,
    pub(super) legacy_max_duration_ms: Option<f64>,
    pub(super) legacy_min_occurred_at: Option<String>,
    pub(super) has_rfc3339: bool,
    pub(super) rfc3339_max_duration_ms: Option<f64>,
    pub(super) rfc3339_min_occurred_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct LongTermRfc3339Compatibility {
    pub(super) max_duration_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(super) struct LongTermArchiveCompatibilityRow {
    pub(super) id: i64,
    pub(super) occurred_at: String,
    pub(super) status: Option<String>,
    pub(super) t_total_ms: Option<f64>,
}

pub(super) async fn long_term_archive_invocation_query_for_range(
    pool: &Pool<Sqlite>,
) -> Result<LongTermArchiveInvocationRangeQueries> {
    let query = long_term_archive_invocation_query_parts(pool).await?;
    let canonical = format!(
        "{} WHERE {} AND instr(occurred_at, 'T') = 0 AND occurred_at >= ?1 AND occurred_at < ?2",
        query.select, query.terminal_filter
    );
    let crossing_text = format!(
        "{} WHERE {} AND occurred_at >= ?1 AND occurred_at < ?2 AND CASE WHEN instr(occurred_at, 'T') = 0 AND {} IS NOT NULL AND {} > 0 THEN julianday(occurred_at) + {} / 86400000.0 END >= julianday(?2)",
        query.select,
        query.terminal_filter,
        query.t_total_ms_column,
        query.t_total_ms_column,
        query.t_total_ms_column,
    );
    let rfc3339_epoch = long_term_rfc3339_whole_epoch_seconds_sql("occurred_at");
    let rfc3339_reaches_range_start =
        long_term_rfc3339_reaches_epoch_sql("occurred_at", &query.t_total_ms_column, "?3");
    let rfc3339 = format!(
        "{} WHERE {} AND instr(occurred_at, 'T') > 0 AND occurred_at >= ?1 AND occurred_at < ?2 AND {} < ?4 AND ({} >= ?3 OR ({} IS NOT NULL AND {} > 0 AND {}))",
        query.select,
        query.terminal_filter,
        rfc3339_epoch,
        rfc3339_epoch,
        query.t_total_ms_column,
        query.t_total_ms_column,
        rfc3339_reaches_range_start,
    );
    Ok(LongTermArchiveInvocationRangeQueries {
        canonical,
        crossing_text,
        rfc3339,
        parts: query,
    })
}

pub(super) fn long_term_projection_update_compatibility_duration(
    maximum: &mut Option<f64>,
    duration_ms: Option<f64>,
) {
    let Some(duration_ms) = duration_ms.filter(|value| *value > 0.0) else {
        return;
    };
    if maximum.is_none_or(|current| duration_ms > current) {
        *maximum = Some(duration_ms);
    }
}

pub(super) async fn inspect_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    query: &LongTermArchiveInvocationQueryParts,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermArchiveCompatibility> {
    // Page by raw primary-key range rather than terminal rows. An archive can contain an
    // arbitrarily long pending prefix, so filtering inside SQL would make one nominal 512-row
    // page scan the full file before it can observe cancellation.
    let first_compatibility_rows = format!(
        "SELECT id, occurred_at, {} AS status, {} AS t_total_ms FROM codex_invocations ORDER BY id ASC LIMIT ?1",
        query.status_column, query.t_total_ms_column,
    );
    let next_compatibility_rows = format!(
        "SELECT id, occurred_at, {} AS status, {} AS t_total_ms FROM codex_invocations WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        query.status_column, query.t_total_ms_column,
    );
    let mut cursor = None;
    let mut legacy_max_duration_ms = None;
    let mut legacy_min_occurred_at = None;
    let mut rfc3339_max_duration_ms = None;
    let mut rfc3339_min_occurred_at = None;
    loop {
        control.check()?;
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, LongTermArchiveCompatibilityRow>(&next_compatibility_rows)
                .bind(cursor)
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .fetch_all(pool)
                .await?
        } else {
            sqlx::query_as::<_, LongTermArchiveCompatibilityRow>(&first_compatibility_rows)
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .fetch_all(pool)
                .await?
        };
        let row_count = rows.len();
        for row in rows {
            cursor = Some(row.id);
            if row.status.as_deref().is_some_and(|status| {
                let status = status.trim();
                status.eq_ignore_ascii_case("running") || status.eq_ignore_ascii_case("pending")
            }) {
                continue;
            }
            if row.occurred_at.contains('T') {
                rfc3339_min_occurred_at = Some(
                    rfc3339_min_occurred_at.map_or(row.occurred_at.clone(), |current: String| {
                        current.min(row.occurred_at.clone())
                    }),
                );
                long_term_projection_update_compatibility_duration(
                    &mut rfc3339_max_duration_ms,
                    row.t_total_ms,
                );
            } else if row.t_total_ms.is_some_and(|duration_ms| duration_ms > 0.0) {
                legacy_min_occurred_at = Some(
                    legacy_min_occurred_at.map_or(row.occurred_at.clone(), |current: String| {
                        current.min(row.occurred_at.clone())
                    }),
                );
                long_term_projection_update_compatibility_duration(
                    &mut legacy_max_duration_ms,
                    row.t_total_ms,
                );
            }
        }
        control.complete_archive_compatibility_batch();
        if row_count < LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
            break;
        }
    }
    Ok(LongTermArchiveCompatibility {
        has_legacy_crossing: legacy_max_duration_ms.is_some(),
        legacy_max_duration_ms,
        legacy_min_occurred_at,
        has_rfc3339: rfc3339_min_occurred_at.is_some(),
        rfc3339_max_duration_ms,
        rfc3339_min_occurred_at,
    })
}

pub(super) async fn load_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    file_fingerprint: &str,
) -> Result<Option<LongTermArchiveCompatibility>> {
    let row = sqlx::query_as::<_, (i64, Option<f64>, Option<String>, i64, Option<f64>, Option<String>)>(
        "SELECT has_legacy_crossing, legacy_max_duration_ms, legacy_min_occurred_at, has_rfc3339, rfc3339_max_duration_ms, rfc3339_min_occurred_at FROM long_term_projection_archive_compatibility WHERE file_path = ?1 AND archive_sha256 = ?2 AND file_fingerprint = ?3",
    )
    .bind(file_path)
    .bind(archive_sha256)
    .bind(file_fingerprint)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(
        |(
            has_legacy_crossing,
            legacy_max_duration_ms,
            legacy_min_occurred_at,
            has_rfc3339,
            rfc3339_max_duration_ms,
            rfc3339_min_occurred_at,
        )| {
            if has_legacy_crossing != 0
                && (legacy_max_duration_ms.is_none() || legacy_min_occurred_at.is_none())
                || has_rfc3339 != 0
                    && (rfc3339_max_duration_ms.is_none() || rfc3339_min_occurred_at.is_none())
            {
                // This cache entry predates the bounded crossing metadata. Reinspect the
                // immutable archive once rather than retaining its old prefix scan.
                None
            } else {
                Some(LongTermArchiveCompatibility {
                    has_legacy_crossing: has_legacy_crossing != 0,
                    legacy_max_duration_ms,
                    legacy_min_occurred_at,
                    has_rfc3339: has_rfc3339 != 0,
                    rfc3339_max_duration_ms,
                    rfc3339_min_occurred_at,
                })
            }
        },
    ))
}

pub(super) async fn load_long_term_archive_sha256(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<Option<String>> {
    load_long_term_archive_sha256_for_dataset(pool, "codex_invocations", file_path).await
}

pub(super) async fn load_long_term_archive_sha256_for_dataset(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = ?1 AND status = 'completed' AND file_path = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(super) async fn persist_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    file_fingerprint: &str,
    compatibility: LongTermArchiveCompatibility,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT INTO long_term_projection_archive_compatibility (file_path, archive_sha256, file_fingerprint, has_legacy_crossing, legacy_max_duration_ms, legacy_min_occurred_at, has_rfc3339, rfc3339_max_duration_ms, rfc3339_min_occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(file_path) DO UPDATE SET archive_sha256 = excluded.archive_sha256, file_fingerprint = excluded.file_fingerprint, has_legacy_crossing = excluded.has_legacy_crossing, legacy_max_duration_ms = excluded.legacy_max_duration_ms, legacy_min_occurred_at = excluded.legacy_min_occurred_at, has_rfc3339 = excluded.has_rfc3339, rfc3339_max_duration_ms = excluded.rfc3339_max_duration_ms, rfc3339_min_occurred_at = excluded.rfc3339_min_occurred_at, updated_at = datetime('now')",
    )
    .bind(file_path)
    .bind(archive_sha256)
    .bind(file_fingerprint)
    .bind(compatibility.has_legacy_crossing)
    .bind(compatibility.legacy_max_duration_ms)
    .bind(&compatibility.legacy_min_occurred_at)
    .bind(compatibility.has_rfc3339)
    .bind(compatibility.rfc3339_max_duration_ms)
    .bind(&compatibility.rfc3339_min_occurred_at)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(super) async fn load_or_inspect_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    file_path: &str,
    file_fingerprint: &str,
    query: &LongTermArchiveInvocationQueryParts,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermArchiveCompatibility> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref()
        && let Some(compatibility) =
            load_long_term_archive_compatibility(pool, file_path, archive_sha256, file_fingerprint)
                .await?
    {
        return Ok(compatibility);
    }

    // Old archives can contain legacy or RFC3339 timestamps. Inspect once, then retain the
    // opened archive's checksum-and-file identity so ordinary canonical repairs stay range-seeked.
    let compatibility =
        inspect_long_term_archive_compatibility(archive_pool, query, control).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref() {
        persist_long_term_archive_compatibility(
            pool,
            file_path,
            archive_sha256,
            file_fingerprint,
            compatibility.clone(),
            control,
        )
        .await?;
    }
    Ok(compatibility)
}

pub(super) fn long_term_archive_file_fingerprint(file_path: &str) -> Result<String> {
    // Archive manifests are updated separately from file replacement. Hash the opened source
    // bytes rather than its mutable metadata so a stale manifest cannot reuse old capability.
    let mut file = std::fs::File::open(file_path).with_context(|| {
        format!("failed to open long-term archive {file_path} for fingerprinting")
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = std::io::Read::read(&mut file, &mut buffer)
            .with_context(|| format!("failed to fingerprint long-term archive {file_path}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn long_term_archive_scan_identity_matches_manifest(
    scanned_sha256: &str,
    current_file_sha256: Option<&str>,
    manifest_sha256: Option<&str>,
) -> bool {
    current_file_sha256 == Some(scanned_sha256) && manifest_sha256 == Some(scanned_sha256)
}

pub(super) fn long_term_archive_file_identity(file_path: &str) -> Result<String> {
    let metadata = std::fs::metadata(file_path)
        .with_context(|| format!("failed to stat long-term archive {file_path}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "unix:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let modified_at = metadata
            .modified()
            .with_context(|| format!("failed to read long-term archive timestamp {file_path}"))?
            .duration_since(std::time::UNIX_EPOCH)
            .with_context(|| {
                format!("long-term archive timestamp predates the epoch {file_path}")
            })?;
        Ok(format!(
            "portable:{}:{}:{}",
            metadata.len(),
            modified_at.as_secs(),
            modified_at.subsec_nanos()
        ))
    }
}

pub(super) async fn ensure_long_term_archive_source_identity(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
) -> Result<()> {
    let file_sha256 = crate::maintenance::sha256_hex_file(std::path::Path::new(file_path))?;
    let manifest_sha256 =
        load_long_term_archive_sha256_for_dataset(pool, dataset, file_path).await?;
    if long_term_archive_scan_identity_matches_manifest(
        expected_sha256,
        Some(&file_sha256),
        manifest_sha256.as_deref(),
    ) {
        Ok(())
    } else {
        bail!(
            "long-term archive source identity does not match its completed manifest: {file_path}"
        )
    }
}

pub(super) async fn long_term_archive_pool_fingerprint(pool: &Pool<Sqlite>) -> Result<String> {
    let databases = sqlx::query_as::<_, (i64, String, String)>("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .context("failed to resolve opened long-term archive database")?;
    let file_path = databases
        .into_iter()
        .find_map(|(_, name, file_path)| (name == "main").then_some(file_path))
        .filter(|file_path| !file_path.is_empty())
        .context("opened long-term archive does not expose a main database path")?;
    long_term_archive_file_fingerprint(&file_path)
}

pub(super) fn long_term_archive_legacy_crossing_start(
    start: &chrono::DateTime<chrono_tz::Tz>,
    max_duration_ms: f64,
) -> Option<String> {
    if !max_duration_ms.is_finite() || max_duration_ms <= 0.0 {
        return None;
    }
    let max_duration_ms = max_duration_ms.ceil();
    if max_duration_ms > (i64::MAX - 1_000) as f64 {
        return None;
    }
    start
        .checked_sub_signed(ChronoDuration::milliseconds(max_duration_ms as i64 + 1_000))
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub(super) fn long_term_rfc3339_text_bounds(
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
    compatibility: &LongTermRfc3339Compatibility,
) -> (String, String) {
    // RFC3339 input is accepted through -14:00 to +14:00. Relative to the +08:00 reporting
    // zone, the raw text for an instant can therefore be twenty-two hours earlier than its local
    // reporting time; leave one extra second for the exclusive lower boundary.
    const LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS: i64 = 22 * 60 * 60 + 1;
    let lower = compatibility
        .max_duration_ms
        .filter(|duration_ms| duration_ms.is_finite() && *duration_ms > 0.0)
        .and_then(|duration_ms| {
            let seconds = (duration_ms / 1000.0).ceil();
            (seconds <= (i64::MAX - LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS) as f64)
                .then_some(seconds as i64)
        })
        .and_then(|seconds| {
            start.checked_sub_signed(ChronoDuration::seconds(
                seconds + LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
            ))
        })
        .map(|value| value.format("%Y-%m-%dT%H:%M:%S").to_string())
        // Zero-duration rows cannot cross into the target date. Keep their candidate seek to the
        // RFC3339 offset window instead of reopening the range at the archive's first old row.
        .unwrap_or_else(|| {
            start
                .checked_sub_signed(ChronoDuration::seconds(
                    LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
                ))
                .unwrap_or(start)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        });
    let upper = end
        .checked_add_signed(ChronoDuration::hours(15))
        .unwrap_or(end)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    (lower, upper)
}

pub(super) async fn load_long_term_archive_invocation_rows_for_range(
    pool: &Pool<Sqlite>,
    queries: &LongTermArchiveInvocationRangeQueries,
    compatibility: LongTermArchiveCompatibility,
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
) -> Result<Vec<LongTermInvocationRow>> {
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.canonical)
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(pool)
        .await?;
    if compatibility.has_legacy_crossing {
        let crossing_start = compatibility
            .legacy_max_duration_ms
            .and_then(|max_duration_ms| {
                long_term_archive_legacy_crossing_start(&start, max_duration_ms)
            })
            .or_else(|| compatibility.legacy_min_occurred_at.clone())
            .context("long-term archive legacy compatibility cache is missing a bounded start")?;
        let crossing_rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.crossing_text)
            .bind(crossing_start)
            .bind(&start_text)
            .fetch_all(pool)
            .await?;
        rows.extend(crossing_rows);
    }
    if compatibility.has_rfc3339 {
        let rfc3339_compatibility = LongTermRfc3339Compatibility {
            max_duration_ms: compatibility.rfc3339_max_duration_ms,
        };
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        rows.extend(
            sqlx::query_as::<_, LongTermInvocationRow>(&queries.rfc3339)
                .bind(rfc3339_lower)
                .bind(rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(pool)
                .await?,
        );
    }
    // The canonical, legacy-crossing, and RFC3339 queries each preserve their own seek order.
    // Their union has no single text order across timestamp encodings, so retain a stable archive
    // row order without reinterpreting timestamps after the sargable range seeks.
    rows.sort_by_key(|row| row.id);
    Ok(rows)
}

pub(super) fn long_term_rfc3339_whole_second_sql(value: &str) -> String {
    // SQLite normalizes RFC3339 fractions to milliseconds before evaluating `strftime` or
    // `julianday`. Strip the fraction before finding the whole second, then retain its original
    // digits so both exact boundaries and sub-millisecond values preserve their true ordering.
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    let whole_second = format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN substr({value}, 1, 19) || substr({value}, 21 + ({fraction_end})) ELSE {value} END"
    );
    whole_second
}

pub(super) fn long_term_rfc3339_fraction_nanos_sql(value: &str) -> String {
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN CAST(substr(substr({value}, 21, {fraction_end}) || '000000000', 1, 9) AS INTEGER) ELSE 0 END"
    )
}

pub(super) fn long_term_rfc3339_whole_epoch_seconds_sql(value: &str) -> String {
    format!(
        "CAST(strftime('%s', {}) AS INTEGER)",
        long_term_rfc3339_whole_second_sql(value)
    )
}

pub(super) fn long_term_rfc3339_elapsed_nanos_sql(duration_ms: &str) -> String {
    // Timestamps retain the first nine fractional digits as nanoseconds. The elapsed value is a
    // persisted SQLite REAL, so normalize only that bounded duration before comparing it with the
    // exact timestamp digits; never turn the RFC3339 fraction itself into a REAL.
    format!("CAST(ROUND(MAX(COALESCE({duration_ms}, 0), 0) * 1000000.0) AS INTEGER)")
}

pub(super) fn long_term_rfc3339_fractional_carry_sql(value: &str, duration_ms: &str) -> String {
    let fraction_nanos = long_term_rfc3339_fraction_nanos_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let elapsed_fraction_nanos =
        format!("(({elapsed_nanos}) - ({elapsed_whole_seconds}) * 1000000000)");
    format!(
        "CASE WHEN ({fraction_nanos}) + ({elapsed_fraction_nanos}) >= 1000000000 THEN 1 ELSE 0 END"
    )
}

pub(super) fn long_term_rfc3339_reaches_epoch_sql(
    value: &str,
    duration_ms: &str,
    epoch: &str,
) -> String {
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, duration_ms);
    format!("(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}) >= {epoch})")
}

pub(super) fn long_term_rfc3339_shanghai_date_sql(
    value: &str,
    duration_ms: Option<&str>,
) -> String {
    // `date(..., 'unixepoch')` normalizes fractional seconds to milliseconds. Keep the raw
    // fractional tail outside date arithmetic, and carry it into the next exact second only
    // when a positive duration crosses that second.
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_seconds = duration_ms.unwrap_or("0");
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(elapsed_seconds);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, elapsed_seconds);
    format!(
        "date(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}), 'unixepoch', '+8 hours')"
    )
}

pub(super) async fn long_term_archive_invocation_query_parts(
    pool: &Pool<Sqlite>,
) -> Result<LongTermArchiveInvocationQueryParts> {
    let columns = load_archive_table_columns(pool, "codex_invocations").await?;
    let select = |column: &str| long_term_legacy_select_expr(&columns, column);
    let status_column = if columns.contains("status") {
        "status"
    } else {
        "NULL"
    };
    let payload = if columns.contains("payload") {
        "payload"
    } else {
        "NULL"
    };
    let request_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.requestModel') AS TEXT)), '') END"
    );
    let response_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.responseModel') AS TEXT)), '') END"
    );
    let reasoning_effort = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.reasoningEffort') AS TEXT)), '') END"
    );
    let payload_upstream_account_id = format!(
        "CASE WHEN json_valid({payload}) THEN CAST(json_extract({payload}, '$.upstreamAccountId') AS INTEGER) END"
    );
    let upstream_account_id = if columns.contains("upstream_account_id") {
        format!("COALESCE(upstream_account_id, {payload_upstream_account_id})")
    } else {
        payload_upstream_account_id
    };
    let t_total_ms_column = if columns.contains("t_total_ms") {
        "t_total_ms"
    } else {
        "NULL"
    };
    let select = format!(
        r#"
        SELECT
            id,
            {invoke_id},
            occurred_at,
            {status},
            {model},
            {request_model} AS request_model,
            {response_model} AS response_model,
            {reasoning_effort} AS reasoning_effort,
            {upstream_account_id} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            {total_tokens},
            {output_tokens},
            {cost},
            {t_total_ms},
            {t_req_read_ms},
            {t_req_parse_ms},
            {t_upstream_connect_ms},
            {t_upstream_ttfb_ms},
            {t_upstream_stream_ms},
            {error_message}
        FROM codex_invocations
        "#,
        invoke_id = select("invoke_id"),
        status = select("status"),
        model = select("model"),
        request_model = request_model,
        response_model = response_model,
        reasoning_effort = reasoning_effort,
        upstream_account_id = upstream_account_id,
        total_tokens = select("total_tokens"),
        output_tokens = select("output_tokens"),
        cost = select("cost"),
        t_total_ms = select("t_total_ms"),
        t_req_read_ms = select("t_req_read_ms"),
        t_req_parse_ms = select("t_req_parse_ms"),
        t_upstream_connect_ms = select("t_upstream_connect_ms"),
        t_upstream_ttfb_ms = select("t_upstream_ttfb_ms"),
        t_upstream_stream_ms = select("t_upstream_stream_ms"),
        error_message = select("error_message"),
    );
    Ok(LongTermArchiveInvocationQueryParts {
        select,
        terminal_filter: format!(
            "LOWER(TRIM(COALESCE({status_column}, ''))) NOT IN ('running', 'pending')"
        ),
        status_column: status_column.to_string(),
        t_total_ms_column: t_total_ms_column.to_string(),
    })
}

pub(super) fn long_term_legacy_select_expr(columns: &HashSet<String>, column: &str) -> String {
    format!(
        "{} AS {column}",
        long_term_legacy_column_expr(columns, column)
    )
}

pub(super) fn long_term_legacy_column_expr(columns: &HashSet<String>, column: &str) -> String {
    if columns.contains(column) {
        column.to_string()
    } else {
        "NULL".to_string()
    }
}
