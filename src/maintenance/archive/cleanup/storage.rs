use super::*;
use tracing::warn;

pub(crate) fn archive_file_is_stale_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(is_archive_temp_file_name)
        .unwrap_or(false)
}

pub(crate) fn archive_temp_file_is_old_enough(path: &Path) -> bool {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs() >= DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS)
        .unwrap_or(false)
}

pub(crate) fn archive_file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or_default()
}

pub(crate) fn cleanup_stale_archive_temp_files(
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveTempCleanupSummary> {
    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    if let Err(error) = collect_archive_file_paths(&archive_root, &mut files) {
        if is_retention_write_deferred(&error) {
            // The janitor is auxiliary work. A busy file phase must not turn the
            // owner task into a deferred run; the next janitor pass can rescan
            // the same root after the owner releases its file-stage permit.
            tracing::debug!(error = %error, "archive temp janitor deferred its directory scan");
            return Ok(ArchiveTempCleanupSummary::default());
        }
        return Err(error);
    }
    let mut summary = ArchiveTempCleanupSummary::default();
    for file_path in files {
        if !archive_file_is_stale_temp(&file_path) || !archive_temp_file_is_old_enough(&file_path) {
            continue;
        }
        if dry_run
            && (crate::is_archive_file_operation_active(&file_path)
                || crate::task_work_artifact_is_active(&file_path))
        {
            continue;
        }
        let _task_work_fence = if dry_run {
            None
        } else {
            match crate::try_acquire_task_work_artifact_fence(&file_path) {
                Ok(crate::TaskWorkArtifactFence::Active) => continue,
                Ok(fence) => Some(fence),
                Err(err) => {
                    warn!(error = %err, "failed to inspect task-local archive work owner");
                    continue;
                }
            }
        };
        let archive_operation_lease = if dry_run {
            None
        } else {
            crate::try_acquire_archive_file_operation_lease(&file_path)
        };
        if !dry_run && archive_operation_lease.is_none() {
            continue;
        }
        let Some(_file_phase) = crate::try_acquire_file_phase_permit() else {
            continue;
        };
        let file_size = archive_file_size(&file_path);
        if dry_run {
            summary.stale_temp_files_removed += 1;
            summary.stale_temp_bytes_removed += file_size;
            continue;
        }
        match fs::remove_file(&file_path) {
            Ok(_) => {
                summary.stale_temp_files_removed += 1;
                summary.stale_temp_bytes_removed += file_size;
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(error = %err, "failed to remove stale archive temp file");
            }
        }
    }
    Ok(summary)
}

pub(crate) async fn verify_archive_storage(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<ArchiveStorageVerificationSummary> {
    let manifest_rows = sqlx::query_as::<_, ArchiveStorageManifestRow>(
        r#"
        SELECT id, dataset, layout, file_path
        FROM archive_batches
        WHERE status = ?1
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut summary = ArchiveStorageVerificationSummary {
        manifest_rows: manifest_rows.len(),
        ..ArchiveStorageVerificationSummary::default()
    };
    let referenced_paths = manifest_rows
        .iter()
        .map(|row| normalize_path_for_compare(Path::new(&row.file_path)))
        .collect::<HashSet<_>>();
    for row in &manifest_rows {
        if !Path::new(&row.file_path).exists() {
            summary.missing_files += 1;
            warn!(
                archive_batch_id = row.id,
                dataset = row.dataset,
                layout = row.layout,
                "archive manifest points to a missing file"
            );
        }
        crate::record_managed_task_processed_work(&["verify_archive_storage"], 1);
    }

    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    collect_archive_file_paths(&archive_root, &mut files)?;
    for file_path in files {
        let normalized = normalize_path_for_compare(&file_path);
        if archive_file_is_stale_temp(&file_path) {
            summary.stale_temp_files += 1;
            summary.stale_temp_bytes += archive_file_size(&file_path);
            continue;
        }
        if !referenced_paths.contains(&normalized) {
            summary.orphan_files += 1;
        }
    }
    Ok(summary)
}
