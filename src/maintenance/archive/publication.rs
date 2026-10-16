use super::super::retention::{
    RetentionTransactionResources, acquire_retention_batch_write_connection_with_archive_fence,
};
use super::archive_writers::sync_published_archive_file;
use super::*;

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationTestPhase {
    Renamed,
    Committed,
    NativeDrained,
}

#[cfg(test)]
pub(crate) struct PublicationTestProbe {
    pub(crate) phase: PublicationTestPhase,
    pub(crate) reached: tokio::sync::Notify,
    pub(crate) resume: tokio::sync::Notify,
    pub(crate) fail: bool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_PUBLICATION_TEST_PROBE: Arc<PublicationTestProbe>;
}

#[cfg(test)]
async fn publication_test_phase(phase: PublicationTestPhase) -> Result<()> {
    if let Ok(probe) = RETENTION_PUBLICATION_TEST_PROBE.try_with(Arc::clone)
        && probe.phase == phase
    {
        probe.reached.notify_one();
        probe.resume.notified().await;
        if probe.fail {
            anyhow::bail!("injected archive publication failure");
        }
    }
    Ok(())
}

pub(crate) async fn replace_legacy_archive_file_with_cleanup_serialization(
    pool: &Pool<Sqlite>,
    dataset: &str,
    month_key: &str,
    temporary_file_path: &Path,
    final_file_path: &Path,
    expected_existing_sha256: Option<&str>,
    replacement_sha256: &str,
) -> Result<()> {
    const OPERATION: &str = "legacy_archive_file_publish";
    let prepared_bytes = temporary_file_path
        .metadata()
        .context("read prepared archive metadata before publication admission")?
        .len() as usize;
    let backup_path = expected_existing_sha256.map(|_| {
        PathBuf::from(format!(
            "{}.{}.restore",
            final_file_path.display(),
            retention_temp_suffix()
        ))
    });
    // This helper hashes the complete predecessor under the directory fence,
    // before trying the writer claim. It releases the fence before any retry wait.
    let Some((connection, admission, fence)) =
        acquire_retention_batch_write_connection_with_archive_fence(
            pool,
            OPERATION,
            final_file_path,
            expected_existing_sha256,
        )
        .await?
    else {
        return Err(super::super::retention::retention_write_deferred(OPERATION));
    };
    let admission_mode = admission.admission_mode();
    let lock_wait = admission.lock_wait();
    let p1_waiters = admission.p1_waiter_count();
    let mut resources = RetentionTransactionResources::new(connection, admission, fence);
    let mut backup_created = false;
    let mut renamed = false;
    let mut commit_attempted = false;
    let publication = async {
        ensure_no_pending_replacement(
            resources.connection_mut(),
            dataset,
            month_key,
            final_file_path,
        )
        .await?;
        if let Some(backup) = backup_path.as_deref() {
            // These durable pointers must precede any file mutation. A partial
            // staging failure remains identifiable by the existing recovery path.
            stage_replacement_journal(
                resources.connection_mut(),
                dataset,
                month_key,
                final_file_path,
                backup,
            )
            .await?;
        }
        let execute_started = Instant::now();
        // Mark before dispatch: cancellation can happen while SQLite begins.
        resources.mark_transaction_started();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(resources.connection_mut())
            .await?;
        if let Some(backup) = backup_path.as_deref() {
            fs::hard_link(final_file_path, backup).context("preserve archive rollback copy")?;
            backup_created = true;
            // The previous file was already durable. Persist its additional name
            // before replacing the final directory entry, so a crash before the
            // new file sync cannot erase the only durable predecessor reference.
            sync_archive_parent(backup)?;
        }
        sqlx::query(
            "UPDATE archive_batches
             SET cleanup_state = ?1, cleanup_source_safe_start_date = NULL, sha256 = ?2
             WHERE dataset = ?3 AND month_key = ?4 AND file_path = ?5
               AND cleanup_state IN (?6, ?7)",
        )
        .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
        .bind(replacement_sha256)
        .bind(dataset)
        .bind(month_key)
        .bind(final_file_path.to_string_lossy().to_string())
        .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
        .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
        .execute(resources.connection_mut())
        .await?;
        fs::rename(temporary_file_path, final_file_path).context("publish prepared archive")?;
        renamed = true;
        #[cfg(test)]
        publication_test_phase(PublicationTestPhase::Renamed).await?;
        // Keep the established rename -> file/directory sync -> manifest COMMIT
        // durability contract. Complete hashing and later cleanup are outside it.
        sync_published_archive_file(final_file_path)?;
        let commit_started = Instant::now();
        commit_attempted = true;
        sqlx::query("COMMIT")
            .execute(resources.connection_mut())
            .await?;
        resources.mark_transaction_finished();
        super::super::retention::retention_record_commit!(
            OPERATION,
            admission_mode,
            1,
            prepared_bytes,
            Duration::ZERO,
            lock_wait,
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            p1_waiters,
            0,
        );
        #[cfg(test)]
        publication_test_phase(PublicationTestPhase::Committed).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;

    match publication {
        Ok(()) => {
            let fence = resources.return_to_pool_retaining_archive_fence().await?;
            #[cfg(test)]
            publication_test_phase(PublicationTestPhase::NativeDrained).await?;
            // The manifest is durable and SQLite has returned its connection.
            // Keep only the directory fence while removing the rollback link.
            if let Some(backup) = backup_path.as_deref() {
                remove_backup_and_sync_parent(backup)?;
            }
            drop(fence);
            clear_replacement_journal_after_file_cleanup(
                pool,
                dataset,
                month_key,
                final_file_path,
                backup_path.as_deref(),
            )
            .await
        }
        Err(error) => {
            if commit_attempted {
                // A failed/cancelled COMMIT is not proof of rollback. Recovery
                // compares the durable manifest, final bytes and rollback link.
                let cleanup_error = resources.rollback_and_close().await;
                if cleanup_error.is_err() {
                    tracing::warn!(
                        "archive publication native cleanup reported an error after an ambiguous COMMIT; retaining recovery evidence"
                    );
                }
                return Err(error.context(
                    "archive replacement commit outcome is unknown; recovery will reconcile",
                ));
            }
            let fence = resources
                .rollback_and_close_retaining_archive_fence()
                .await?;
            compensate_uncommitted_publication(
                final_file_path,
                backup_path.as_deref(),
                backup_created,
                renamed,
            )
            .context("archive replacement rollback file compensation deferred")?;
            drop(fence);
            // No filesystem fence may be retained while waiting for another
            // database admission. Exact-path predicates cannot clear a newer retry.
            if let Err(cleanup_error) = clear_replacement_journal_after_file_cleanup(
                pool,
                dataset,
                month_key,
                final_file_path,
                backup_path.as_deref(),
            )
            .await
            {
                return Err(error.context(format!(
                    "archive replacement journal cleanup deferred: {cleanup_error}"
                )));
            }
            Err(error)
        }
    }
}

async fn ensure_no_pending_replacement(
    connection: &mut SqliteConnection,
    dataset: &str,
    month_key: &str,
    final_path: &Path,
) -> Result<()> {
    let prepared = match sqlx::query_scalar::<_, Option<String>>(
        "SELECT staged_file_path FROM retention_prepared_archives
         WHERE dataset = ?1 AND file_path = ?2 AND state = 'preparing'",
    )
    .bind(dataset)
    .bind(final_path.to_string_lossy().to_string())
    .fetch_optional(&mut *connection)
    .await
    {
        Ok(value) => value.flatten(),
        Err(error) if error.to_string().contains("no such table") => None,
        Err(error) => return Err(error.into()),
    };
    let manifest = sqlx::query_scalar::<_, Option<String>>(
        "SELECT replacement_staged_path FROM archive_batches
         WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3",
    )
    .bind(dataset)
    .bind(month_key)
    .bind(final_path.to_string_lossy().to_string())
    .fetch_optional(&mut *connection)
    .await?
    .flatten();
    if prepared.is_some() || manifest.is_some() {
        anyhow::bail!("archive replacement recovery is still pending");
    }
    Ok(())
}

async fn stage_replacement_journal(
    connection: &mut SqliteConnection,
    dataset: &str,
    month_key: &str,
    final_path: &Path,
    backup: &Path,
) -> Result<()> {
    sqlx::query(
        "UPDATE retention_prepared_archives
         SET staged_file_path = ?1, updated_at = datetime('now')
         WHERE dataset = ?2 AND file_path = ?3 AND state = 'preparing'",
    )
    .bind(backup.to_string_lossy().to_string())
    .bind(dataset)
    .bind(final_path.to_string_lossy().to_string())
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "UPDATE archive_batches SET replacement_staged_path = ?1
         WHERE dataset = ?2 AND month_key = ?3 AND file_path = ?4",
    )
    .bind(backup.to_string_lossy().to_string())
    .bind(dataset)
    .bind(month_key)
    .bind(final_path.to_string_lossy().to_string())
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn compensate_uncommitted_publication(
    final_path: &Path,
    backup: Option<&Path>,
    backup_created: bool,
    renamed: bool,
) -> Result<()> {
    if renamed {
        if let Some(backup) = backup {
            // Never unlink the final artifact before restoring its verified
            // predecessor. On failure retain the rollback link and journal.
            fs::rename(backup, final_path).context("restore archive predecessor atomically")?;
            sync_published_archive_file(final_path)?;
        } else {
            fs::remove_file(final_path).context("remove uncommitted first archive publication")?;
            sync_archive_parent(final_path)?;
        }
    } else if backup_created {
        remove_backup_and_sync_parent(backup.expect("created backup has a path"))?;
    }
    Ok(())
}

fn remove_backup_and_sync_parent(backup: &Path) -> Result<()> {
    match fs::remove_file(backup) {
        Ok(()) => sync_archive_parent(backup),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("archive rollback cleanup deferred"),
    }
}

fn sync_archive_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

async fn clear_replacement_journal_after_file_cleanup(
    pool: &Pool<Sqlite>,
    dataset: &str,
    month_key: &str,
    final_path: &Path,
    backup: Option<&Path>,
) -> Result<()> {
    let Some(backup) = backup else {
        return Ok(());
    };
    let operation = "legacy_archive_file_cleanup";
    let Some((connection, admission)) =
        super::super::retention::acquire_retention_batch_write_connection(pool, operation).await?
    else {
        return Err(super::super::retention::retention_write_deferred(operation));
    };
    let mut resources = RetentionTransactionResources::without_archive_fence(connection, admission);
    resources.mark_transaction_started();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(resources.connection_mut())
        .await?;
    sqlx::query(
        "UPDATE retention_prepared_archives
         SET staged_file_path = NULL, updated_at = datetime('now')
         WHERE dataset = ?1 AND file_path = ?2 AND state = 'preparing'
           AND staged_file_path = ?3",
    )
    .bind(dataset)
    .bind(final_path.to_string_lossy().to_string())
    .bind(backup.to_string_lossy().to_string())
    .execute(resources.connection_mut())
    .await?;
    sqlx::query(
        "UPDATE archive_batches SET replacement_staged_path = NULL
         WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3
           AND replacement_staged_path = ?4",
    )
    .bind(dataset)
    .bind(month_key)
    .bind(final_path.to_string_lossy().to_string())
    .bind(backup.to_string_lossy().to_string())
    .execute(resources.connection_mut())
    .await?;
    sqlx::query("COMMIT")
        .execute(resources.connection_mut())
        .await?;
    resources.mark_transaction_finished();
    resources
        .return_to_pool()
        .await
        .context("archive replacement journal cleanup did not drain")?;
    Ok(())
}
