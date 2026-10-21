use super::*;

pub(crate) struct RetentionArchiveCandidateScan {
    pub(crate) candidates: Vec<PathBuf>,
    pub(crate) progress: Option<String>,
}

impl RetentionArchiveCandidateScan {
    fn record_progress(&mut self, cursor_key: &str) {
        let should_replace = self
            .progress
            .as_ref()
            .is_none_or(|current| cursor_key > current.as_str());
        if should_replace {
            self.progress = Some(cursor_key.to_string());
        }
    }
}

pub(crate) fn collect_retention_archive_candidates_after_cursor(
    root: &Path,
    cursor: &str,
    limit: usize,
) -> Result<RetentionArchiveCandidateScan> {
    let mut scan = RetentionArchiveCandidateScan {
        candidates: Vec::with_capacity(limit.min(RETENTION_RECOVERY_LEGACY_SCAN_BATCH)),
        progress: None,
    };
    collect_retention_archive_candidates_after_cursor_inner(root, cursor, limit, &mut scan)?;
    // Filesystem enumeration order is not a cursor contract. Only sort the bounded candidate
    // set, never the complete contents of a visited directory.
    scan.candidates.sort_unstable();
    Ok(scan)
}

pub(crate) fn collect_retention_archive_candidates_after_cursor_inner(
    root: &Path,
    cursor: &str,
    limit: usize,
    scan: &mut RetentionArchiveCandidateScan,
) -> Result<bool> {
    if scan.candidates.len() >= limit {
        return Ok(true);
    }
    #[cfg(test)]
    retention_test_legacy_directory_traversal_event();
    if limit == 0 || !root.is_dir() {
        return Ok(false);
    }

    let mut directory_cursor = cursor.to_string();
    let mut skip_path: Option<PathBuf> = None;
    let mut pause_at: Option<String> = None;
    loop {
        let selection = collect_bounded_retention_archive_directory_entries(
            root,
            &directory_cursor,
            skip_path.as_deref(),
            limit,
        )?;
        let RetentionArchiveDirectorySelection { entries, truncated } = selection;
        if pause_at.is_none() && truncated {
            pause_at = entries.last().map(|entry| entry.cursor_key.clone());
        }
        let entries = entries.into_iter();
        let mut next_directory = None;
        for entry in entries {
            if entry.is_dir {
                next_directory = Some(entry);
                break;
            }
            let path = entry.path;
            let cursor_key = entry.cursor_key;
            let is_after_cursor = cursor_key.as_str() > directory_cursor.as_str();
            if is_after_cursor {
                scan.record_progress(&cursor_key);
                directory_cursor = cursor_key;
            }
            if entry.is_file
                && is_after_cursor
                && (path.to_string_lossy().ends_with(".sqlite.gz")
                    || path.to_string_lossy().ends_with(".sqlite.zst"))
            {
                scan.candidates.push(path);
            }
            if scan.candidates.len() >= limit {
                break;
            }
        }

        if scan.candidates.len() >= limit {
            return Ok(true);
        }
        let Some(entry) = next_directory else {
            if pause_at
                .as_deref()
                .is_some_and(|boundary| directory_cursor.as_str() >= boundary)
            {
                return Ok(true);
            }
            return Ok(truncated);
        };
        let RetentionArchiveDirectoryEntry {
            path,
            cursor_key,
            #[cfg(test)]
            heap_entry_guard,
            ..
        } = entry;
        #[cfg(test)]
        drop(heap_entry_guard);
        if cursor_key.as_str() > directory_cursor.as_str() {
            scan.record_progress(&cursor_key);
            directory_cursor = cursor_key;
        }
        let child_stopped = collect_retention_archive_candidates_after_cursor_inner(
            &path,
            &directory_cursor,
            limit,
            scan,
        )?;
        if child_stopped {
            return Ok(true);
        }
        if let Some(progress) = scan.progress.as_deref()
            && progress > directory_cursor.as_str()
        {
            directory_cursor = progress.to_string();
        }
        skip_path = Some(path);
        if truncated
            && pause_at
                .as_deref()
                .is_some_and(|boundary| directory_cursor.as_str() >= boundary)
        {
            return Ok(true);
        }
    }
}

pub(crate) fn retention_archive_cursor_key(path: &Path, is_dir: bool) -> String {
    let mut cursor_key = path.to_string_lossy().to_string();
    if is_dir {
        cursor_key.push('/');
    }
    cursor_key
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RetentionArchiveDirectoryEntry {
    pub(crate) path: PathBuf,
    pub(crate) cursor_key: String,
    pub(crate) is_dir: bool,
    pub(crate) is_file: bool,
    #[cfg(test)]
    pub(crate) heap_entry_guard: Option<RetentionTestLegacyDirectoryHeapEntryGuard>,
}

pub(crate) struct RetentionArchiveDirectorySelection {
    pub(crate) entries: Vec<RetentionArchiveDirectoryEntry>,
    pub(crate) truncated: bool,
}

impl Ord for RetentionArchiveDirectoryEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cursor_key.cmp(&other.cursor_key)
    }
}

impl PartialOrd for RetentionArchiveDirectoryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) fn collect_bounded_retention_archive_directory_entries(
    root: &Path,
    cursor: &str,
    skip_path: Option<&Path>,
    limit: usize,
) -> Result<RetentionArchiveDirectorySelection> {
    let directory_entries = fs::read_dir(root)
        .with_context(|| format!("failed to read archive directory {}", root.display()))?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let is_dir = file_type.is_dir();
            Ok(RetentionArchiveDirectoryEntry {
                cursor_key: retention_archive_cursor_key(&path, is_dir),
                path,
                is_dir,
                is_file: file_type.is_file(),
                #[cfg(test)]
                heap_entry_guard: None,
            })
        });
    select_bounded_retention_archive_directory_entries(directory_entries, cursor, skip_path, limit)
}

pub(crate) fn select_bounded_retention_archive_directory_entries(
    directory_entries: impl IntoIterator<Item = Result<RetentionArchiveDirectoryEntry>>,
    cursor: &str,
    skip_path: Option<&Path>,
    limit: usize,
) -> Result<RetentionArchiveDirectorySelection> {
    // Stream the directory through a fixed-size max heap. This keeps allocation and comparison
    // work per entry bounded by the scan batch without materializing or sorting the full listing.
    let mut entries = std::collections::BinaryHeap::with_capacity(limit);
    let mut truncated = false;
    for entry in directory_entries {
        let entry = entry?;
        #[cfg(test)]
        retention_test_legacy_directory_entry_event();
        let path = &entry.path;
        let eligible = if entry.is_dir {
            skip_path.is_none_or(|skipped| skipped != path)
                && (cursor.is_empty()
                    || entry.cursor_key.as_str() > cursor
                    || Path::new(cursor).starts_with(path))
        } else {
            entry.cursor_key.as_str() > cursor
        };
        if !eligible {
            continue;
        }
        if entries.len() < limit {
            #[cfg(test)]
            let candidate = {
                let mut candidate = entry;
                candidate.heap_entry_guard =
                    Some(RetentionTestLegacyDirectoryHeapEntryGuard::new());
                candidate
            };
            #[cfg(not(test))]
            let candidate = entry;
            entries.push(candidate);
        } else if entries
            .peek()
            .is_some_and(|largest| entry.cursor_key < largest.cursor_key)
        {
            truncated = true;
            entries.pop();
            #[cfg(test)]
            let candidate = {
                let mut candidate = entry;
                candidate.heap_entry_guard =
                    Some(RetentionTestLegacyDirectoryHeapEntryGuard::new());
                candidate
            };
            #[cfg(not(test))]
            let candidate = entry;
            entries.push(candidate);
        } else {
            truncated = true;
        }
    }
    let mut entries = entries.into_vec();
    entries.sort_unstable_by(|left, right| left.cursor_key.cmp(&right.cursor_key));
    Ok(RetentionArchiveDirectorySelection { entries, truncated })
}

#[cfg(test)]
pub(crate) fn retention_test_select_bounded_archive_paths(
    paths: Vec<PathBuf>,
    cursor: &str,
    limit: usize,
) -> Vec<PathBuf> {
    let entries = paths.into_iter().map(|path| {
        Ok(RetentionArchiveDirectoryEntry {
            cursor_key: retention_archive_cursor_key(&path, false),
            path,
            is_dir: false,
            is_file: true,
            heap_entry_guard: None,
        })
    });
    select_bounded_retention_archive_directory_entries(entries, cursor, None, limit)
        .expect("select bounded archive paths")
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

pub(crate) async fn verify_legacy_retention_archive_segment(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    archive_path: &Path,
    expected_sha256: &str,
) -> Result<Option<(RetentionPreparedArchiveDescriptor, String)>> {
    let file_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let Some(part_key) = file_name.strip_suffix(".sqlite.gz") else {
        return Ok(None);
    };
    if !part_key.starts_with("part-") {
        return Ok(None);
    }
    let Some(day) = archive_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let Some(month) = archive_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let Some(year) = archive_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let day_key = format!("{year}-{month}-{day}");
    let temp_path = PathBuf::from(format!(
        "{}.{}.recovery.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let _temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    inflate_gzip_sqlite_file_with_operation_budget(archive_path, &temp_path).await?;
    let mut archive_db = open_archive_sqlite_connection(&temp_path).await?;
    ensure_codex_invocations_archive_schema_direct(&mut archive_db).await?;
    let ids = sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id ASC")
        .fetch_all(&mut archive_db)
        .await?;
    if ids.is_empty() || archive_segment_part_key_for_ids(&ids).ok().as_deref() != Some(part_key) {
        return Ok(None);
    }
    let archive_identity = invocation_archive_source_identity_sha256(
        &mut archive_db,
        InvocationArchiveIdentityDatabase::Main,
        &ids,
    )
    .await?;
    archive_db.close().await?;
    let source_ids_json = serde_json::to_string(&ids)?;
    let mut source_connection = pool.acquire().await?;
    let source_identity = invocation_archive_source_identity_sha256(
        &mut source_connection,
        InvocationArchiveIdentityDatabase::Main,
        &ids,
    )
    .await?;
    drop(source_connection);
    if source_identity != archive_identity {
        return Ok(None);
    }
    let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
        r#"
        SELECT id, invoke_id, occurred_at, source, status, input_tokens, output_tokens,
               cache_input_tokens, reasoning_tokens, total_tokens, cost, first_token_ms,
               payload, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE id IN (SELECT value FROM json_each(?1))
        ORDER BY id ASC
        "#,
    )
    .bind(&source_ids_json)
    .fetch_all(pool)
    .await?;
    if candidates.len() != ids.len() {
        return Ok(None);
    }
    let descriptor = retention_prepared_archive_descriptor(
        config,
        "codex_invocations",
        &day_key,
        &candidates,
        source_identity,
        RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE,
    )?;
    if descriptor.file_path != archive_path.to_string_lossy()
        || descriptor.part_key.as_deref() != Some(part_key)
    {
        return Ok(None);
    }
    if sha256_hex_file_with_deadline(archive_path, retention_archive_file_operation_deadline())?
        != expected_sha256
    {
        return Ok(None);
    }
    Ok(Some((descriptor, expected_sha256.to_string())))
}

pub(crate) fn invocation_archive_candidate_to_hourly_source_record(
    candidate: &InvocationArchiveCandidate,
) -> InvocationHourlySourceRecord {
    InvocationHourlySourceRecord {
        id: candidate.id,
        occurred_at: candidate.occurred_at.clone(),
        source: candidate.source.clone(),
        status: candidate.status.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        model: None,
        input_tokens: candidate.input_tokens,
        output_tokens: candidate.output_tokens,
        cache_input_tokens: candidate.cache_input_tokens,
        reasoning_tokens: candidate.reasoning_tokens,
        total_tokens: candidate.total_tokens,
        cost: candidate.cost,
        upstream_account_id: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: candidate.payload.clone(),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: candidate.first_token_ms,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    }
}

pub(crate) fn encode_summary_archive_snapshot_v2_payload(
    rows: &[InvocationHourlySourceRecord],
    invoke_ids_by_row_id: &HashMap<i64, String>,
) -> Result<Vec<u8>> {
    let records = rows
        .iter()
        .map(|row| {
            let payload = row
                .payload
                .as_deref()
                .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok());
            SummaryArchiveSnapshotV2Record {
                id: row.id,
                // Preserve the durable invocation identity so a Snapshot fallback can deduplicate
                // against a live row that has not yet crossed the archive cleanup boundary.
                invoke_id: invoke_ids_by_row_id
                    .get(&row.id)
                    .cloned()
                    .or_else(|| {
                        payload
                            .as_ref()
                            .and_then(|value| value.get("invokeId"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| format!("archive-row-{}", row.id)),
                occurred_at: row.occurred_at.clone(),
                source: row.source.clone(),
                model: row.model.clone().or_else(|| {
                    payload
                        .as_ref()
                        .and_then(|value| value.get("model"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                }),
                response_model: payload
                    .as_ref()
                    .and_then(|value| value.get("responseModel"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                input_tokens: row.input_tokens.unwrap_or_default(),
                output_tokens: row.output_tokens.unwrap_or_default(),
                cache_input_tokens: row.cache_input_tokens.unwrap_or_default(),
                reasoning_tokens: row.reasoning_tokens.unwrap_or_default(),
                reasoning_effort: payload
                    .as_ref()
                    .and_then(|value| value.get("reasoningEffort"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                total_tokens: row.total_tokens.unwrap_or_default(),
                cost: row.cost,
                cost_input: row.cost_input,
                cost_cache_write: row.cost_cache_write,
                cost_cache_read: row.cost_cache_read,
                cost_output: row.cost_output,
                cost_reasoning: row.cost_reasoning,
                status: row.status.clone().unwrap_or_else(|| "unknown".to_string()),
                error_message: row.error_message.clone(),
                failure_kind: row.failure_kind.clone(),
                failure_class: row.failure_class.clone(),
                is_actionable: row.is_actionable.unwrap_or_default() != 0,
                upstream_account_id: row.resolved_upstream_account_id(),
            }
        })
        .collect::<Vec<_>>();
    let normalized =
        serde_json::to_vec(&records).context("encode normalized Summary Archive Snapshot page")?;
    zstd::stream::encode_all(normalized.as_slice(), 1)
        .context("compress normalized Summary Archive Snapshot page")
}
