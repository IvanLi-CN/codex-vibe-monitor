use super::{
    RetentionArchiveFileLock, RetentionWriteAdmission, RetentionWriteCommit,
    record_retention_write_commit,
};
use anyhow::{Result, anyhow};
use sqlx::{Sqlite, SqliteConnection, pool::PoolConnection};
#[cfg(test)]
use std::sync::{Arc, atomic::AtomicUsize};
use std::time::Duration;

pub(crate) struct RetentionTransactionResources {
    connection: Option<PoolConnection<Sqlite>>,
    admission: Option<RetentionWriteAdmission>,
    archive_lock: Option<RetentionArchiveFileLock>,
    transaction_started: bool,
    #[cfg(test)]
    return_probe_operation: Option<&'static str>,
}

struct RetentionCleanupParts {
    connection: PoolConnection<Sqlite>,
    admission: Option<RetentionWriteAdmission>,
    archive_lock: Option<RetentionArchiveFileLock>,
    transaction_started: bool,
    write_commit: Option<RetentionWriteCommit>,
}

struct RetentionCleanupResult {
    archive_lock: Option<RetentionArchiveFileLock>,
    write_hold_elapsed: Option<Duration>,
    write_commit_breached: Option<bool>,
}

#[derive(Clone, Copy)]
enum RetentionConnectionCleanup {
    RollbackAndClose,
    ReturnToPool,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct RetentionReturnToPoolProbe {
    pub(crate) operation: Option<&'static str>,
    pub(crate) returned: tokio::sync::Notify,
    pub(crate) resume: tokio::sync::Notify,
    pub(crate) pause_at_return: AtomicUsize,
    pub(crate) return_count: AtomicUsize,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_RETURN_TO_POOL: Arc<RetentionReturnToPoolProbe>;
    pub(crate) static RETENTION_CLEANUP_WAITER_READY: Arc<tokio::sync::Notify>;
}

impl RetentionTransactionResources {
    pub(crate) fn new(
        connection: PoolConnection<Sqlite>,
        admission: RetentionWriteAdmission,
        archive_lock: RetentionArchiveFileLock,
    ) -> Self {
        Self {
            connection: Some(connection),
            admission: Some(admission),
            archive_lock: Some(archive_lock),
            transaction_started: false,
            #[cfg(test)]
            return_probe_operation: None,
        }
    }

    pub(crate) fn without_archive_fence(
        connection: PoolConnection<Sqlite>,
        admission: RetentionWriteAdmission,
    ) -> Self {
        Self {
            connection: Some(connection),
            admission: Some(admission),
            archive_lock: None,
            transaction_started: false,
            #[cfg(test)]
            return_probe_operation: None,
        }
    }

    pub(crate) fn connection_mut(&mut self) -> &mut SqliteConnection {
        self.connection
            .as_mut()
            .expect("retention transaction connection is present")
    }

    #[cfg(test)]
    pub(crate) fn observe_return_for_test(&mut self, operation: &'static str) {
        self.return_probe_operation = Some(operation);
    }

    pub(crate) fn mark_transaction_started(&mut self) {
        self.transaction_started = true;
    }

    pub(crate) fn mark_transaction_finished(&mut self) {
        self.transaction_started = false;
    }

    fn release_archive_lock(&mut self) {
        drop(self.archive_lock.take());
    }

    fn release_admission(&mut self) {
        drop(self.admission.take());
    }

    fn release_resources(&mut self) {
        self.release_archive_lock();
        self.release_admission();
    }

    fn take_cleanup_parts(
        &mut self,
        write_commit: Option<RetentionWriteCommit>,
    ) -> Option<RetentionCleanupParts> {
        self.connection
            .take()
            .map(|connection| RetentionCleanupParts {
                connection,
                admission: self.admission.take(),
                archive_lock: self.archive_lock.take(),
                transaction_started: self.transaction_started,
                write_commit,
            })
    }

    async fn cleanup(
        mut self,
        mode: RetentionConnectionCleanup,
        retain_archive_lock: bool,
        write_commit: Option<RetentionWriteCommit>,
    ) -> Result<RetentionCleanupResult> {
        let Some(parts) = self.take_cleanup_parts(write_commit) else {
            self.release_resources();
            return Ok(RetentionCleanupResult {
                archive_lock: None,
                write_hold_elapsed: None,
                write_commit_breached: None,
            });
        };
        #[cfg(test)]
        let return_probe = RETENTION_TEST_RETURN_TO_POOL
            .try_with(Arc::clone)
            .ok()
            .filter(|probe| {
                probe.operation.is_none() || probe.operation == self.return_probe_operation
            });
        #[cfg(not(test))]
        let return_probe = None;
        // A cancelled waiter drops only the JoinHandle. This independent owner
        // retains the connection and both fences through native drain.
        let handle = tokio::spawn(cleanup_retention_transaction(
            parts,
            mode,
            return_probe,
            retain_archive_lock,
        ));
        #[cfg(test)]
        let _ = RETENTION_CLEANUP_WAITER_READY.try_with(|ready| ready.notify_one());
        handle
            .await
            .map_err(|error| anyhow!("retention transaction cleanup task failed: {error}"))?
    }

    pub(crate) async fn rollback_and_close(self) -> Result<()> {
        self.cleanup(RetentionConnectionCleanup::RollbackAndClose, false, None)
            .await
            .map(|_| ())
    }

    pub(crate) async fn return_to_pool(self) -> Result<()> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool, false, None)
            .await
            .map(|_| ())
    }

    pub(crate) async fn return_to_pool_with_hold(self) -> Result<Duration> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool, false, None)
            .await
            .map(|result| result.write_hold_elapsed.unwrap_or_default())
    }

    pub(crate) async fn return_to_pool_and_record_commit(
        self,
        write_commit: RetentionWriteCommit,
    ) -> Result<bool> {
        self.cleanup(
            RetentionConnectionCleanup::ReturnToPool,
            false,
            Some(write_commit),
        )
        .await
        .map(|result| result.write_commit_breached.unwrap_or(false))
    }

    pub(crate) async fn rollback_and_close_retaining_archive_fence(
        self,
    ) -> Result<RetentionArchiveFileLock> {
        self.cleanup(RetentionConnectionCleanup::RollbackAndClose, true, None)
            .await?
            .archive_lock
            .ok_or_else(|| anyhow!("archive publication lost its file fence"))
    }

    pub(crate) async fn return_to_pool_retaining_archive_fence(
        self,
    ) -> Result<RetentionArchiveFileLock> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool, true, None)
            .await?
            .archive_lock
            .ok_or_else(|| anyhow!("archive publication lost its file fence"))
    }
}

async fn cleanup_retention_transaction(
    mut parts: RetentionCleanupParts,
    mode: RetentionConnectionCleanup,
    #[cfg(test)] return_probe: Option<Arc<RetentionReturnToPoolProbe>>,
    #[cfg(not(test))] _return_probe: Option<()>,
    retain_archive_lock: bool,
) -> Result<RetentionCleanupResult> {
    let mut cleanup_error = None;
    if matches!(mode, RetentionConnectionCleanup::RollbackAndClose)
        && parts.transaction_started
        && let Err(error) = sqlx::query("ROLLBACK")
            .execute(&mut *parts.connection)
            .await
    {
        cleanup_error = Some(anyhow!("retention transaction rollback failed: {error}"));
    }
    if matches!(mode, RetentionConnectionCleanup::RollbackAndClose) {
        if let Err(error) = parts.connection.close().await {
            if let Some(previous) = cleanup_error.take() {
                cleanup_error = Some(
                    previous.context(format!("retention connection close also failed: {error}")),
                );
            } else {
                cleanup_error = Some(anyhow!(
                    "retention transaction connection close failed: {error}"
                ));
            }
        }
    } else {
        parts.connection.return_to_pool().await;
        #[cfg(test)]
        if let Some(probe) = return_probe {
            let return_count = probe
                .return_count
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                + 1;
            if probe
                .pause_at_return
                .load(std::sync::atomic::Ordering::Acquire)
                == return_count
            {
                probe.returned.notify_one();
                probe.resume.notified().await;
            }
        }
    }
    // Publication cleanup/compensation may retain just the directory fence;
    // its SQLite admission is released only after the native connection drains.
    let archive_lock = if retain_archive_lock {
        parts.archive_lock.take()
    } else {
        drop(parts.archive_lock.take());
        None
    };
    let write_hold_elapsed = parts
        .admission
        .as_ref()
        .map(RetentionWriteAdmission::write_hold_elapsed);
    let write_commit_breached = parts.write_commit.map(|mut commit| {
        commit.write_hold_elapsed = write_hold_elapsed;
        record_retention_write_commit(commit)
    });
    drop(parts.admission);
    cleanup_error.map_or(
        Ok(RetentionCleanupResult {
            archive_lock,
            write_hold_elapsed,
            write_commit_breached,
        }),
        Err,
    )
}

impl Drop for RetentionTransactionResources {
    fn drop(&mut self) {
        let Some(parts) = self.take_cleanup_parts(None) else {
            self.release_resources();
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            drop(handle.spawn(cleanup_retention_transaction(
                parts,
                RetentionConnectionCleanup::RollbackAndClose,
                None,
                false,
            )));
        } else {
            // Runtime teardown cannot acknowledge native drain. Keep every
            // ownership part through process exit instead of advertising release.
            std::mem::forget(parts);
        }
    }
}
