use super::*;

#[derive(Debug, Default)]
pub(crate) struct RawOrphanSweepPassResult {
    pub(crate) file_candidates_checked: usize,
    pub(crate) inspected_entries: usize,
    pub(crate) reconciliation_rows_checked: usize,
    pub(crate) reconciliation_has_more: bool,
    pub(crate) referenced_skipped: usize,
    pub(crate) quarantined: usize,
    pub(crate) removed: usize,
    pub(crate) removed_bytes: u64,
    pub(crate) failures: usize,
    pub(crate) reached_end: bool,
    pub(crate) complete: bool,
    pub(crate) deferred: bool,
    pub(crate) admission_stage: Option<String>,
    pub(crate) admission_cause: Option<String>,
}

pub(crate) async fn sweep_orphan_proxy_raw_files(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<usize> {
    let mut traversal = RetentionRawDirectoryTraversal::default();
    Ok(sweep_orphan_proxy_raw_files_slice(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        &mut traversal,
    )
    .await?
    .removed)
}

pub(crate) async fn sweep_orphan_proxy_raw_files_slice(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    traversal: &mut RetentionRawDirectoryTraversal,
) -> Result<RawOrphanSweepPassResult> {
    let context = match acquire_raw_orphan_sweep_pass_context().await {
        Ok(context) => context,
        Err(error) => return Err(error),
    };
    let mut result = RawOrphanSweepPassResult {
        complete: true,
        ..Default::default()
    };
    let raw_root = normalize_path_for_compare(&config.resolved_proxy_raw_dir());
    let effective_fallback_root = raw_path_fallback_root.or(config.database_path.parent());

    if traversal.root.as_deref() != Some(raw_root.as_path()) {
        traversal.root = Some(raw_root.clone());
        traversal.directory = None;
        traversal.pending_candidates.clear();
    }

    if !dry_run {
        match cleanup_missing_retention_raw_reconciliation_rows(
            pool,
            &context,
            &raw_root,
            RETENTION_RAW_RECONCILIATION_SCAN_BATCH,
        )
        .await
        {
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                return Ok(result);
            }
            Ok((examined, failures, has_more)) => {
                result.reconciliation_rows_checked = examined;
                result.reconciliation_has_more = has_more;
                result.failures += failures;
                if failures != 0 {
                    result.complete = false;
                }
            }
            Err(error) if is_retention_write_deferred(&error) => {
                result.complete = false;
                result.deferred = true;
                if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                    result.admission_stage = Some(stage.to_string());
                    result.admission_cause = Some(cause.to_string());
                }
                return Ok(result);
            }
            Err(_) => {
                result.complete = false;
                result.failures += 1;
            }
        }
    }

    if context.candidate_budget_expired() {
        result.complete = false;
        result.reached_end =
            traversal.directory.is_none() && traversal.pending_candidates.is_empty();
        return Ok(result);
    }

    let candidate_entries = if traversal.pending_candidates.is_empty() {
        let scan = match read_retention_raw_directory_slice_with_budget(
            &context,
            &raw_root,
            &mut traversal.directory,
        )
        .await
        {
            Ok(scan) => scan,
            Err(error) => return Err(error),
        };
        result.inspected_entries += scan.inspected_entries;
        result.failures += scan.failures;
        if scan.failures != 0 {
            result.complete = false;
        }
        traversal.directory = scan.directory;
        if context.candidate_budget_expired() && !scan.candidates.is_empty() {
            result.complete = false;
            traversal.pending_candidates.extend(scan.candidates);
            result.reached_end = false;
            return Ok(result);
        }
        scan.candidates
    } else {
        std::mem::take(&mut traversal.pending_candidates)
            .into_iter()
            .collect()
    };
    if candidate_entries.is_empty() {
        result.reached_end =
            traversal.directory.is_none() && traversal.pending_candidates.is_empty();
        if result.reconciliation_has_more || !result.reached_end {
            result.complete = false;
        }
        return Ok(result);
    }

    let candidate_paths = candidate_entries
        .iter()
        .map(|candidate| normalize_path_for_compare(&candidate.path))
        .collect::<Vec<_>>();
    let linked_paths = match context
        .run_with_candidate_budget(retention_raw_link_paths(pool, config, &candidate_paths))
        .await
    {
        Ok(paths) => paths,
        Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
            result.complete = false;
            traversal.pending_candidates.extend(candidate_entries);
            return Ok(result);
        }
        Err(_) => {
            result.failures += 1;
            result.complete = false;
            traversal.pending_candidates.extend(candidate_entries);
            return Ok(result);
        }
    };
    let referenced_candidates = match retention_raw_candidates_with_links_with_budget(
        &context,
        config,
        &candidate_paths,
        &linked_paths,
        effective_fallback_root,
    )
    .await
    {
        Ok(paths) => paths,
        Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
            result.complete = false;
            traversal.pending_candidates.extend(candidate_entries);
            return Ok(result);
        }
        Err(_) => {
            result.complete = false;
            result.failures += 1;
            traversal.pending_candidates.extend(candidate_entries);
            return Ok(result);
        }
    };

    for (candidate_index, path) in candidate_paths.iter().enumerate() {
        if context.candidate_budget_expired() {
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        result.file_candidates_checked += 1;
        let path = normalize_path_for_compare(path);
        let metadata = match retention_raw_file_metadata_with_budget(&context, &path).await {
            Ok(Ok(metadata)) if metadata.file_type().is_file() => metadata,
            Ok(Ok(_)) => continue,
            Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => continue,
            Ok(Err(_)) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if context.candidate_budget_expired() {
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        let raw_path = path.to_string_lossy().into_owned();
        let file_identity = retention_raw_file_identity(&metadata);
        let byte_size = match i64::try_from(metadata.len()) {
            Ok(size) => size,
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        let existing = match context
            .run_with_candidate_budget(load_retention_raw_reconciliation_row(pool, &raw_path))
            .await
        {
            Ok(existing) => existing,
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        // A replacement identity proves only that the old object is no longer at this
        // pathname; it does not prove that the failed unlink removed the old object.
        // Let observation reset the ledger without manufacturing removal evidence.
        if referenced_candidates.contains(&path) {
            if !dry_run
                && existing.is_some()
                && let Err(error) =
                    clear_retention_raw_reconciliation_row(pool, &context, &raw_path, None).await
            {
                result.complete = false;
                result.failures += 1;
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    result.failures = result.failures.saturating_sub(1);
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                if is_retention_write_deferred(&error) {
                    result.complete = false;
                    result.deferred = true;
                    if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                        result.admission_stage = Some(stage.to_string());
                        result.admission_cause = Some(cause.to_string());
                    }
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                continue;
            }
            result.referenced_skipped += 1;
            continue;
        }
        if existing
            .as_ref()
            .is_none_or(|row| row.file_identity != file_identity || row.byte_size != byte_size)
        {
            if !dry_run
                && let Err(error) = record_retention_raw_reconciliation_observation(
                    pool,
                    &context,
                    &raw_path,
                    &file_identity,
                    byte_size,
                )
                .await
            {
                result.complete = false;
                result.failures += 1;
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    result.failures = result.failures.saturating_sub(1);
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                if is_retention_write_deferred(&error) {
                    result.complete = false;
                    result.deferred = true;
                    if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                        result.admission_stage = Some(stage.to_string());
                        result.admission_cause = Some(cause.to_string());
                    }
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                continue;
            }
            if !dry_run {
                result.quarantined += 1;
            }
            continue;
        }
        let Some(existing) = existing else {
            continue;
        };
        if !retention_raw_quarantine_due(&existing.quarantined_at) {
            continue;
        }

        if dry_run {
            result.removed = result.removed.saturating_add(1);
            result.removed_bytes = result.removed_bytes.saturating_add(byte_size as u64);
            continue;
        }

        if context.candidate_budget_expired() {
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        let final_metadata = match retention_raw_file_metadata_with_budget(&context, &path).await {
            Ok(Ok(metadata)) if metadata.file_type().is_file() => metadata,
            Ok(Ok(_)) => continue,
            Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => {
                if let Err(error) = clear_retention_raw_reconciliation_row(
                    pool,
                    &context,
                    &raw_path,
                    Some(&existing.file_identity),
                )
                .await
                {
                    result.complete = false;
                    result.failures += 1;
                    if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                        result.failures = result.failures.saturating_sub(1);
                        traversal
                            .pending_candidates
                            .extend(candidate_entries.iter().skip(candidate_index).cloned());
                        break;
                    }
                    if is_retention_write_deferred(&error) {
                        result.complete = false;
                        result.deferred = true;
                        if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                            result.admission_stage = Some(stage.to_string());
                            result.admission_cause = Some(cause.to_string());
                        }
                        traversal
                            .pending_candidates
                            .extend(candidate_entries.iter().skip(candidate_index).cloned());
                        break;
                    }
                }
                continue;
            }
            Ok(Err(_)) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if context.candidate_budget_expired() {
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        let final_identity = retention_raw_file_identity(&final_metadata);
        let final_size = match i64::try_from(final_metadata.len()) {
            Ok(size) => size,
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        let current = match context
            .run_with_candidate_budget(load_retention_raw_reconciliation_row(pool, &raw_path))
            .await
        {
            Ok(Some(current)) => current,
            Ok(None) => continue,
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if current.file_identity != final_identity || current.byte_size != final_size {
            if let Err(error) = record_retention_raw_reconciliation_observation(
                pool,
                &context,
                &raw_path,
                &final_identity,
                final_size,
            )
            .await
            {
                result.complete = false;
                result.failures += 1;
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    result.failures = result.failures.saturating_sub(1);
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                if is_retention_write_deferred(&error) {
                    result.complete = false;
                    result.deferred = true;
                    if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                        result.admission_stage = Some(stage.to_string());
                        result.admission_cause = Some(cause.to_string());
                    }
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
            }
            continue;
        }
        if !retention_raw_quarantine_due(&current.quarantined_at) {
            continue;
        }
        let inventory_admission = match context.try_write("raw_reconciliation_release").await {
            Ok(admission) => admission,
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(error) => {
                result.complete = false;
                result.deferred = true;
                if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                    result.admission_stage = Some(stage.to_string());
                    result.admission_cause = Some(cause.to_string());
                }
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
        };
        let directory_lock = match retention_try_archive_locks_scope(async {
            retention_archive_file_try_lock(&path)
        })
        .await
        {
            Ok(lock) => lock,
            Err(error) if is_retention_write_deferred(&error) => {
                drop(inventory_admission);
                result.complete = false;
                result.deferred = true;
                result.admission_stage = Some("archive_file_phase".to_string());
                result.admission_cause = Some("archive_directory_lock_busy".to_string());
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        match retention_raw_candidate_is_referenced(
            pool,
            config,
            &path,
            effective_fallback_root,
            &context,
        )
        .await
        {
            Ok(true) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.referenced_skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                drop(directory_lock);
                drop(inventory_admission);
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    result.complete = false;
                    traversal
                        .pending_candidates
                        .extend(candidate_entries.iter().skip(candidate_index).cloned());
                    break;
                }
                result.complete = false;
                result.failures += 1;
                continue;
            }
        }
        if let Err(error) = context
            .run_with_candidate_budget(mark_retention_raw_inventory_reset_intent(pool))
            .await
        {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            result.failures += 1;
            continue;
        }
        let unlink_metadata = match try_retention_raw_file_metadata_with_file_phase(
            &path,
            context.candidate_budget_remaining(),
        )
        .await
        {
            Ok(RetentionRawFileMetadataProbe::FilePhaseBusy) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.deferred = true;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Ok(RetentionRawFileMetadataProbe::CandidateBudgetExpired) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Ok(metadata)))
                if metadata.file_type().is_file() =>
            {
                metadata
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Ok(_))) => {
                drop(directory_lock);
                drop(inventory_admission);
                continue;
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Err(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                drop(directory_lock);
                drop(inventory_admission);
                continue;
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Err(_))) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                continue;
            }
            Err(_) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if context.candidate_budget_expired() {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        let unlink_identity = retention_raw_file_identity(&unlink_metadata);
        let unlink_size = match i64::try_from(unlink_metadata.len()) {
            Ok(size) => size,
            Err(_) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if unlink_identity != final_identity
            || unlink_size != final_size
            || unlink_identity != current.file_identity
            || unlink_size != current.byte_size
        {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            result.failures += 1;
            traversal
                .pending_candidates
                .push_back(candidate_entries[candidate_index].clone());
            continue;
        }
        let release_pending_started_at = format_utc_iso(Utc::now());
        if let Err(error) = mark_retention_raw_reconciliation_release_pending(
            pool,
            &context,
            &raw_path,
            &unlink_identity,
            unlink_size,
            &release_pending_started_at,
        )
        .await
        {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            result.failures += 1;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        if context.candidate_budget_expired() {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        let final_unlink_metadata = match try_retention_raw_file_metadata_with_file_phase(
            &path,
            context.candidate_budget_remaining(),
        )
        .await
        {
            Ok(RetentionRawFileMetadataProbe::FilePhaseBusy) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.deferred = true;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Ok(RetentionRawFileMetadataProbe::CandidateBudgetExpired) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Ok(metadata)))
                if metadata.file_type().is_file() =>
            {
                metadata
            }
            Ok(RetentionRawFileMetadataProbe::Ready(Ok(_)))
            | Ok(RetentionRawFileMetadataProbe::Ready(Err(_))) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                traversal
                    .pending_candidates
                    .push_back(candidate_entries[candidate_index].clone());
                continue;
            }
            Err(_) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                traversal
                    .pending_candidates
                    .push_back(candidate_entries[candidate_index].clone());
                continue;
            }
        };
        let final_unlink_identity = retention_raw_file_identity(&final_unlink_metadata);
        let final_unlink_size = match i64::try_from(final_unlink_metadata.len()) {
            Ok(size) => size,
            Err(_) => {
                drop(directory_lock);
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                traversal
                    .pending_candidates
                    .push_back(candidate_entries[candidate_index].clone());
                continue;
            }
        };
        if final_unlink_identity != unlink_identity
            || final_unlink_size != unlink_size
            || context.candidate_budget_expired()
        {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            if final_unlink_identity != unlink_identity || final_unlink_size != unlink_size {
                result.failures += 1;
            }
            traversal
                .pending_candidates
                .push_back(candidate_entries[candidate_index].clone());
            if context.candidate_budget_expired() {
                break;
            }
            continue;
        }
        #[cfg(test)]
        if retention_test_raw_unlink_should_fail() {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            result.failures += 1;
            continue;
        }
        // Keep the directory fence alive while the blocking filesystem operation runs. If the
        // candidate budget expires, the blocking task may finish after this slice returns, but the
        // durable release-pending row and held fence let the next slice recover without a second
        // unlink attempt.
        let unlink_path = path.clone();
        let Some(file_phase) = try_acquire_file_phase_permit() else {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            result.deferred = true;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        };
        let unlink_operation = context
            .run_with_candidate_budget(async move {
                tokio::task::spawn_blocking(move || {
                    let _file_phase = file_phase;
                    let file_result = match fs::remove_file(&unlink_path) {
                        Ok(()) => (true, sync_retention_raw_parent(&unlink_path)),
                        Err(error) => (false, Err(error)),
                    };
                    (directory_lock, file_result)
                })
                .await
                .map_err(|error| anyhow!("raw orphan unlink worker failed: {error}"))
            })
            .await;
        let (directory_lock, (removed_file, sync_result)) = match unlink_operation {
            Ok(outcome) => outcome,
            Err(error) if is_raw_orphan_sweep_candidate_budget_expired(&error) => {
                drop(inventory_admission);
                result.complete = false;
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            Err(_) => {
                drop(inventory_admission);
                result.complete = false;
                result.failures += 1;
                continue;
            }
        };
        if !removed_file {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            result.failures += 1;
            continue;
        }
        if sync_result.is_err() {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            result.failures += 1;
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        result.removed += 1;
        result.removed_bytes = result
            .removed_bytes
            .saturating_add(final_unlink_metadata.len());
        RETENTION_RAW_CAPTURE_CIRCUIT
            .try_with(|circuit| {
                if let Some(circuit) = circuit.borrow().as_ref() {
                    circuit.record_deleted_bytes(final_unlink_metadata.len());
                }
            })
            .ok();
        if let Err(error) = persist_raw_orphan_sweep_removal_evidence_after_admission(
            pool,
            &context,
            &raw_path,
            &final_unlink_identity,
            result.removed,
            result.removed_bytes,
        )
        .await
        {
            drop(directory_lock);
            drop(inventory_admission);
            result.complete = false;
            if !is_raw_orphan_sweep_candidate_budget_expired(&error) {
                result.failures += 1;
            }
            traversal
                .pending_candidates
                .extend(candidate_entries.iter().skip(candidate_index).cloned());
            break;
        }
        drop(directory_lock);
        drop(inventory_admission);
        if let Err(error) = clear_retention_raw_reconciliation_row(
            pool,
            &context,
            &raw_path,
            Some(&final_unlink_identity),
        )
        .await
        {
            result.complete = false;
            result.failures += 1;
            if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                result.failures = result.failures.saturating_sub(1);
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
            if is_retention_write_deferred(&error) {
                result.complete = false;
                result.deferred = true;
                if let Some((stage, cause)) = raw_orphan_sweep_admission_details(&error) {
                    result.admission_stage = Some(stage.to_string());
                    result.admission_cause = Some(cause.to_string());
                }
                traversal
                    .pending_candidates
                    .extend(candidate_entries.iter().skip(candidate_index).cloned());
                break;
            }
        }
    }

    result.reached_end = traversal.directory.is_none() && traversal.pending_candidates.is_empty();
    if result.reconciliation_has_more || !result.reached_end {
        result.complete = false;
    }
    Ok(result)
}

#[derive(Debug)]
pub(crate) struct RawOrphanSweepSchedule {
    pub(crate) next_retry_at: Option<String>,
    pub(crate) consecutive_failure_count: u32,
    pub(crate) last_failure_fingerprint: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) last_progress_at: Option<String>,
    pub(crate) retry_after_secs: i64,
    pub(crate) admission_stage: Option<String>,
    pub(crate) admission_cause: Option<String>,
    pub(crate) last_settled_pass: Option<RawOrphanSweepSettledPassSnapshot>,
    pub(crate) last_nonzero_removal: Option<RawOrphanSweepRemovalSnapshot>,
}

pub(crate) async fn load_raw_orphan_sweep_schedule(
    pool: &Pool<Sqlite>,
) -> Result<RawOrphanSweepSchedule> {
    let row = sqlx::query(
        r#"SELECT next_retry_at, consecutive_failure_count,
                  last_failure_fingerprint, defer_reason, last_progress_at,
                  last_admission_stage, last_admission_cause,
                  last_settled_pass_at, last_settled_pass_complete,
                  last_settled_pass_inspected_entries,
                  last_settled_pass_referenced_skipped,
                  last_settled_pass_quarantined, last_settled_pass_removed,
                  last_settled_pass_removed_bytes,
                  last_nonzero_removal_at, last_nonzero_removal,
                  last_nonzero_removal_bytes,
                  CASE WHEN next_retry_at IS NULL THEN 0
                       ELSE MAX(0, CAST(strftime('%s', next_retry_at) - strftime('%s', 'now') AS INTEGER))
                  END AS retry_after_secs
           FROM retention_recovery_cursors WHERE scope = ?1"#,
    )
    .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(RawOrphanSweepSchedule {
            next_retry_at: None,
            consecutive_failure_count: 0,
            last_failure_fingerprint: None,
            defer_reason: None,
            last_progress_at: None,
            retry_after_secs: 0,
            admission_stage: None,
            admission_cause: None,
            last_settled_pass: None,
            last_nonzero_removal: None,
        });
    };

    let last_settled_pass_at = row.try_get::<Option<String>, _>("last_settled_pass_at")?;
    let last_settled_pass =
        last_settled_pass_at.map(|settled_at| RawOrphanSweepSettledPassSnapshot {
            settled_at,
            complete: row
                .try_get::<Option<i64>, _>("last_settled_pass_complete")
                .unwrap_or(None)
                .unwrap_or_default()
                != 0,
            inspected_entries: row
                .try_get::<Option<i64>, _>("last_settled_pass_inspected_entries")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as usize,
            referenced_skipped: row
                .try_get::<Option<i64>, _>("last_settled_pass_referenced_skipped")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as usize,
            quarantined: row
                .try_get::<Option<i64>, _>("last_settled_pass_quarantined")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as usize,
            removed: row
                .try_get::<Option<i64>, _>("last_settled_pass_removed")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as usize,
            removed_bytes: row
                .try_get::<Option<i64>, _>("last_settled_pass_removed_bytes")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as u64,
        });
    let last_nonzero_removal = row
        .try_get::<Option<String>, _>("last_nonzero_removal_at")?
        .map(|removed_at| RawOrphanSweepRemovalSnapshot {
            removed_at,
            removed: row
                .try_get::<Option<i64>, _>("last_nonzero_removal")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as usize,
            removed_bytes: row
                .try_get::<Option<i64>, _>("last_nonzero_removal_bytes")
                .unwrap_or(None)
                .unwrap_or_default()
                .max(0) as u64,
        });
    Ok(RawOrphanSweepSchedule {
        next_retry_at: row.try_get("next_retry_at")?,
        consecutive_failure_count: row.try_get::<i64, _>("consecutive_failure_count")?.max(0)
            as u32,
        last_failure_fingerprint: row.try_get("last_failure_fingerprint")?,
        defer_reason: row.try_get("defer_reason")?,
        last_progress_at: row.try_get("last_progress_at")?,
        retry_after_secs: row.try_get("retry_after_secs")?,
        admission_stage: row.try_get("last_admission_stage")?,
        admission_cause: row.try_get("last_admission_cause")?,
        last_settled_pass,
        last_nonzero_removal,
    })
}

#[derive(Default)]
pub(crate) struct RawOrphanSweepHealthUpdate<'a> {
    pub(crate) inspected_entries: Option<usize>,
    pub(crate) referenced_skipped: Option<usize>,
    pub(crate) quarantined: Option<usize>,
    pub(crate) removed: Option<usize>,
    pub(crate) removed_bytes: Option<u64>,
    pub(crate) schedule: Option<&'a RawOrphanSweepSchedule>,
    pub(crate) defer_reason: Option<&'a str>,
    pub(crate) failure_fingerprint: Option<String>,
    pub(crate) admission_stage: Option<&'a str>,
    pub(crate) admission_cause: Option<&'a str>,
    pub(crate) settled_pass: Option<RawOrphanSweepSettledPassSnapshot>,
    pub(crate) nonzero_removal: Option<RawOrphanSweepRemovalSnapshot>,
    pub(crate) clear_admission: bool,
    pub(crate) clear_defer_reason: bool,
}

pub(crate) fn raw_orphan_sweep_set_health(state: &str, update: RawOrphanSweepHealthUpdate<'_>) {
    let mut health = RAW_ORPHAN_SWEEP_HEALTH
        .lock()
        .expect("raw orphan sweep health");
    health.state = state.to_string();
    if let Some(inspected_entries) = update.inspected_entries {
        health.inspected_entries = Some(inspected_entries);
    }
    if let Some(referenced_skipped) = update.referenced_skipped {
        health.referenced_skipped = Some(referenced_skipped);
    }
    if let Some(quarantined) = update.quarantined {
        health.quarantined = Some(quarantined);
    }
    if let Some(removed) = update.removed {
        health.removed = Some(removed);
    }
    if let Some(removed_bytes) = update.removed_bytes {
        health.removed_bytes = Some(removed_bytes);
    }
    if let Some(defer_reason) = update.defer_reason {
        health.defer_reason = Some(defer_reason.to_string());
    }
    if update.clear_defer_reason {
        health.defer_reason = None;
    }
    if let Some(failure_fingerprint) = update.failure_fingerprint {
        health.failure_fingerprint = Some(failure_fingerprint);
    } else if state == "idle" || state == "scanning" {
        health.failure_fingerprint = None;
    }
    if let Some(admission_stage) = update.admission_stage {
        health.admission_stage = Some(admission_stage.to_string());
    }
    if let Some(admission_cause) = update.admission_cause {
        health.admission_cause = Some(admission_cause.to_string());
    }
    if update.clear_admission {
        health.admission_stage = None;
        health.admission_cause = None;
    }
    if let Some(settled_pass) = update.settled_pass {
        health.last_settled_pass = Some(settled_pass);
    }
    if let Some(nonzero_removal) = update.nonzero_removal {
        health.last_nonzero_removal = Some(nonzero_removal);
    }
    if let Some(schedule) = update.schedule {
        health.last_progress_at = schedule.last_progress_at.clone();
        health.next_retry_at = schedule.next_retry_at.clone();
        if health.last_settled_pass.is_none() {
            health.last_settled_pass = schedule.last_settled_pass.clone();
        }
        if let Some(schedule_removal) = schedule.last_nonzero_removal.as_ref()
            && health
                .last_nonzero_removal
                .as_ref()
                .is_none_or(|current| schedule_removal.removed_at > current.removed_at)
        {
            health.last_nonzero_removal = Some(schedule_removal.clone());
        }
        if state != "scanning" && health.admission_stage.is_none() {
            health.admission_stage = schedule.admission_stage.clone();
        }
        if state != "scanning" && health.admission_cause.is_none() {
            health.admission_cause = schedule.admission_cause.clone();
        }
        if state != "scanning" && health.defer_reason.is_none() {
            health.defer_reason = schedule.defer_reason.clone();
        }
        if state != "scanning" && health.failure_fingerprint.is_none() {
            health.failure_fingerprint = schedule.last_failure_fingerprint.clone();
        }
        if state == "scanning" {
            health.next_retry_at = None;
        }
    }
}

pub(crate) async fn persist_raw_orphan_sweep_schedule(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    transition: RawOrphanSweepScheduleTransition,
) -> Result<Option<u32>> {
    persist_raw_orphan_sweep_schedule_with_evidence(pool, retry_secs, transition, None, None).await
}

pub(crate) async fn persist_raw_orphan_sweep_schedule_with_evidence(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    transition: RawOrphanSweepScheduleTransition,
    pass: Option<&RawOrphanSweepPassResult>,
    admission_evidence: Option<(&str, &str)>,
) -> Result<Option<u32>> {
    persist_raw_orphan_sweep_schedule_with_evidence_and_removal(
        pool,
        retry_secs,
        transition,
        pass,
        pass,
        admission_evidence,
    )
    .await
}

pub(crate) async fn persist_raw_orphan_sweep_schedule_with_evidence_and_removal(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    transition: RawOrphanSweepScheduleTransition,
    settled_pass: Option<&RawOrphanSweepPassResult>,
    removal_pass: Option<&RawOrphanSweepPassResult>,
    admission_evidence: Option<(&str, &str)>,
) -> Result<Option<u32>> {
    if let Some(admission) = acquire_retention_write_admission("raw_reconciliation_schedule").await
    {
        let result = persist_raw_orphan_sweep_schedule_after_admission(
            pool,
            retry_secs,
            transition,
            settled_pass,
            removal_pass,
            admission_evidence,
        )
        .await;
        drop(admission);
        return result;
    }

    // A pressure-cooldown rejection still needs a durable short retry and fixed admission
    // evidence. The fallback intentionally takes only the coordinator write permit, so persisting
    // the cursor does not require the background slot that was just denied.
    let Some((write_permit, _)) =
        acquire_retention_write_coordinator("raw_reconciliation_schedule").await
    else {
        return Ok(None);
    };
    let result = persist_raw_orphan_sweep_schedule_after_admission(
        pool,
        retry_secs,
        transition,
        settled_pass,
        removal_pass,
        admission_evidence,
    )
    .await;
    drop(write_permit);
    result
}

pub(crate) async fn persist_raw_orphan_sweep_schedule_after_admission(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    transition: RawOrphanSweepScheduleTransition,
    settled_pass: Option<&RawOrphanSweepPassResult>,
    removal_pass: Option<&RawOrphanSweepPassResult>,
    admission_evidence: Option<(&str, &str)>,
) -> Result<Option<u32>> {
    sqlx::query("INSERT OR IGNORE INTO retention_recovery_cursors (scope, cursor) VALUES (?1, '')")
        .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
        .execute(pool)
        .await?;
    let current_failures: i64 = sqlx::query_scalar(
        "SELECT consecutive_failure_count FROM retention_recovery_cursors WHERE scope = ?1",
    )
    .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
    .fetch_one(pool)
    .await?;
    let next_failures = match &transition {
        RawOrphanSweepScheduleTransition::Failure { .. } => {
            current_failures.saturating_add(1).min(5)
        }
        RawOrphanSweepScheduleTransition::Success { .. } => 0,
        RawOrphanSweepScheduleTransition::Pressure => current_failures,
    };
    let (failure_count, defer_reason, fingerprint, progress) = match transition {
        RawOrphanSweepScheduleTransition::Failure {
            fingerprint,
            progressed,
        } => (
            next_failures,
            Some(RETENTION_RECOVERY_DEFER_RETRY_BACKOFF),
            Some(fingerprint),
            progressed,
        ),
        RawOrphanSweepScheduleTransition::Pressure => (
            next_failures,
            Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE),
            None,
            false,
        ),
        RawOrphanSweepScheduleTransition::Success { progressed } => (0, None, None, progressed),
    };
    let settled_pass_present = i64::from(settled_pass.is_some());
    let nonzero_removal_present = i64::from(removal_pass.is_some_and(|pass| pass.removed > 0));
    let (admission_stage, admission_cause) = admission_evidence
        .map(|(stage, cause)| (Some(stage), Some(cause)))
        .unwrap_or((None, None));
    sqlx::query(
        r#"UPDATE retention_recovery_cursors
           SET next_retry_at = CASE WHEN ?1 > 0 THEN datetime('now', printf('+%d seconds', ?1)) ELSE NULL END,
               consecutive_failure_count = ?2,
               last_failure_fingerprint = ?3,
               defer_reason = ?4,
               last_progress_at = CASE WHEN ?5 != 0 THEN datetime('now') ELSE last_progress_at END,
               last_admission_stage = ?7,
               last_admission_cause = ?8,
               last_settled_pass_at = CASE WHEN ?9 != 0 THEN datetime('now') ELSE last_settled_pass_at END,
               last_settled_pass_complete = CASE WHEN ?9 != 0 THEN ?10 ELSE last_settled_pass_complete END,
               last_settled_pass_inspected_entries = CASE WHEN ?9 != 0 THEN ?11 ELSE last_settled_pass_inspected_entries END,
               last_settled_pass_referenced_skipped = CASE WHEN ?9 != 0 THEN ?12 ELSE last_settled_pass_referenced_skipped END,
               last_settled_pass_quarantined = CASE WHEN ?9 != 0 THEN ?13 ELSE last_settled_pass_quarantined END,
               last_settled_pass_removed = CASE WHEN ?9 != 0 THEN ?14 ELSE last_settled_pass_removed END,
               last_settled_pass_removed_bytes = CASE WHEN ?9 != 0 THEN ?15 ELSE last_settled_pass_removed_bytes END,
               last_nonzero_removal_at = CASE WHEN ?16 != 0 THEN datetime('now') ELSE last_nonzero_removal_at END,
               last_nonzero_removal = CASE WHEN ?16 != 0 THEN ?17 ELSE last_nonzero_removal END,
               last_nonzero_removal_bytes = CASE WHEN ?16 != 0 THEN ?18 ELSE last_nonzero_removal_bytes END,
               updated_at = datetime('now')
           WHERE scope = ?6"#,
    )
    .bind(retry_secs)
    .bind(failure_count)
    .bind(fingerprint)
    .bind(defer_reason)
    .bind(i64::from(progress))
    .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
    .bind(admission_stage)
    .bind(admission_cause)
    .bind(settled_pass_present)
    .bind(settled_pass.map(|pass| i64::from(pass.complete)))
    .bind(settled_pass.map(|pass| pass.inspected_entries as i64))
    .bind(settled_pass.map(|pass| pass.referenced_skipped as i64))
    .bind(settled_pass.map(|pass| pass.quarantined as i64))
    .bind(settled_pass.map(|pass| pass.removed as i64))
    .bind(settled_pass.map(|pass| pass.removed_bytes.min(i64::MAX as u64) as i64))
    .bind(nonzero_removal_present)
    .bind(removal_pass.map(|pass| pass.removed as i64))
    .bind(removal_pass.map(|pass| pass.removed_bytes.min(i64::MAX as u64) as i64))
    .execute(pool)
    .await?;
    Ok(Some(next_failures as u32))
}

pub(crate) enum RawOrphanSweepScheduleTransition {
    Failure {
        fingerprint: String,
        progressed: bool,
    },
    Pressure,
    Success {
        progressed: bool,
    },
}

pub(crate) fn raw_orphan_sweep_failure_retry_secs(failure_count: u32) -> i64 {
    let index = failure_count
        .min((RETENTION_RAW_RECONCILIATION_FAILURE_BACKOFF_SECS.len() - 1) as u32)
        as usize;
    RETENTION_RAW_RECONCILIATION_FAILURE_BACKOFF_SECS[index]
}

pub(crate) fn raw_orphan_sweep_next_retry_secs(
    pass: &RawOrphanSweepPassResult,
    consecutive_failure_count: u32,
) -> i64 {
    if pass.failures > 0 {
        raw_orphan_sweep_failure_retry_secs(consecutive_failure_count)
    } else if !pass.complete || pass.reconciliation_has_more || !pass.reached_end {
        RETENTION_RAW_RECONCILIATION_PROGRESS_RETRY_SECS
    } else {
        RETENTION_RAW_RECONCILIATION_EMPTY_RETRY_SECS
    }
}

pub(crate) fn raw_orphan_sweep_settled_pass_snapshot(
    pass: &RawOrphanSweepPassResult,
) -> RawOrphanSweepSettledPassSnapshot {
    RawOrphanSweepSettledPassSnapshot {
        settled_at: format_utc_iso(Utc::now()),
        complete: pass.complete,
        inspected_entries: pass.inspected_entries,
        referenced_skipped: pass.referenced_skipped,
        quarantined: pass.quarantined,
        removed: pass.removed,
        removed_bytes: pass.removed_bytes,
    }
}

pub(crate) fn raw_orphan_sweep_removal_snapshot(
    pass: &RawOrphanSweepPassResult,
) -> Option<RawOrphanSweepRemovalSnapshot> {
    (pass.removed > 0).then(|| RawOrphanSweepRemovalSnapshot {
        removed_at: format_utc_iso(Utc::now()),
        removed: pass.removed,
        removed_bytes: pass.removed_bytes,
    })
}

/// The managed executor keeps this traversal across admitted requests. There is no
/// autonomous raw sweep loop: controls and scheduling belong to the maintenance store.
pub(crate) async fn run_managed_raw_orphan_sweep(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cancel: &CancellationToken,
    circuit: Arc<RawCaptureCircuitBreaker>,
    options: MaintenanceExecutionOptions,
    traversal: &mut RetentionRawDirectoryTraversal,
) -> Result<(RawOrphanSweepPassResult, i64)> {
    let dry_run = options.dry_run;
    let schedule = load_raw_orphan_sweep_schedule(pool).await?;
    if !options.manual
        && !dry_run
        && schedule.retry_after_secs > 0
        && matches!(
            schedule.defer_reason.as_deref(),
            Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE | RETENTION_RECOVERY_DEFER_RETRY_BACKOFF)
        )
    {
        return Ok((
            RawOrphanSweepPassResult {
                deferred: true,
                admission_stage: Some("safe_retry".to_string()),
                admission_cause: schedule.defer_reason.clone(),
                ..Default::default()
            },
            schedule.retry_after_secs,
        ));
    }
    let result = RETENTION_SHUTDOWN
        .scope(
            cancel.clone(),
            RETENTION_RAW_CAPTURE_CIRCUIT.scope(
                RefCell::new(Some(circuit)),
                sweep_orphan_proxy_raw_files_slice(
                    pool,
                    config,
                    config.database_path.parent(),
                    dry_run,
                    traversal,
                ),
            ),
        )
        .await;
    match result {
        Ok(pass) => {
            let progressed = pass.inspected_entries > 0
                || pass.reconciliation_rows_checked > 0
                || pass.removed > 0;
            let retry = if pass.deferred {
                RETENTION_RAW_RECONCILIATION_PRESSURE_RETRY_SECS
            } else {
                raw_orphan_sweep_next_retry_secs(&pass, schedule.consecutive_failure_count)
            };
            let transition = if pass.deferred {
                RawOrphanSweepScheduleTransition::Pressure
            } else if pass.failures > 0 {
                RawOrphanSweepScheduleTransition::Failure {
                    fingerprint: "raw_orphan_sweep_candidate_failure".to_string(),
                    progressed,
                }
            } else {
                RawOrphanSweepScheduleTransition::Success { progressed }
            };
            if !dry_run {
                let persisted = persist_raw_orphan_sweep_schedule_with_evidence_and_removal(
                    pool,
                    retry,
                    transition,
                    (!pass.deferred).then_some(&pass),
                    Some(&pass),
                    pass.admission_stage
                        .as_deref()
                        .zip(pass.admission_cause.as_deref()),
                )
                .await?;
                if persisted.is_none() {
                    bail!("raw orphan sweep settlement unavailable");
                }
            }
            let updated = load_raw_orphan_sweep_schedule(pool).await?;
            raw_orphan_sweep_set_health(
                if pass.deferred {
                    "deferred"
                } else if pass.failures > 0 {
                    "degraded"
                } else {
                    "idle"
                },
                RawOrphanSweepHealthUpdate {
                    inspected_entries: Some(pass.inspected_entries),
                    referenced_skipped: Some(pass.referenced_skipped),
                    quarantined: Some(pass.quarantined),
                    removed: (!dry_run).then_some(pass.removed),
                    removed_bytes: (!dry_run).then_some(pass.removed_bytes),
                    schedule: Some(&updated),
                    settled_pass: (!dry_run && !pass.deferred)
                        .then(|| raw_orphan_sweep_settled_pass_snapshot(&pass)),
                    nonzero_removal: (!dry_run)
                        .then(|| raw_orphan_sweep_removal_snapshot(&pass))
                        .flatten(),
                    admission_stage: pass.admission_stage.as_deref(),
                    admission_cause: pass.admission_cause.as_deref(),
                    clear_admission: !pass.deferred,
                    clear_defer_reason: !pass.deferred && pass.failures == 0,
                    ..Default::default()
                },
            );
            Ok((pass, retry))
        }
        Err(error) => {
            if !dry_run {
                let pressure = is_retention_write_deferred(&error);
                let retry = if pressure {
                    RETENTION_RAW_RECONCILIATION_PRESSURE_RETRY_SECS
                } else {
                    raw_orphan_sweep_failure_retry_secs(schedule.consecutive_failure_count)
                };
                let transition = if pressure {
                    RawOrphanSweepScheduleTransition::Pressure
                } else {
                    RawOrphanSweepScheduleTransition::Failure {
                        fingerprint: retention_error_fingerprint(&error),
                        progressed: false,
                    }
                };
                persist_raw_orphan_sweep_schedule_with_evidence(
                    pool,
                    retry,
                    transition,
                    None,
                    raw_orphan_sweep_admission_details(&error),
                )
                .await?;
            }
            Err(error)
        }
    }
}
