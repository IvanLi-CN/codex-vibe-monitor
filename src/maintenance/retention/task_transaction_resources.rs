use super::{RetentionArchiveFileLock, RetentionWriteAdmission};
use anyhow::{Result, anyhow};
use sqlx::{Sqlite, SqliteConnection, pool::PoolConnection};
#[cfg(test)]
use std::sync::{Arc, atomic::AtomicUsize};

pub(crate) struct RetentionTransactionResources {
    connection: Option<PoolConnection<Sqlite>>,
    admission: Option<RetentionWriteAdmission>,
    archive_lock: Option<RetentionArchiveFileLock>,
    transaction_started: bool,
}

struct RetentionCleanupParts {
    connection: PoolConnection<Sqlite>,
    admission: Option<RetentionWriteAdmission>,
    archive_lock: Option<RetentionArchiveFileLock>,
    transaction_started: bool,
}

#[derive(Clone, Copy)]
enum RetentionConnectionCleanup {
    RollbackAndClose,
    ReturnToPool,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct RetentionReturnToPoolProbe {
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
        }
    }

    pub(crate) fn connection_mut(&mut self) -> &mut SqliteConnection {
        self.connection
            .as_mut()
            .expect("retention transaction connection is present")
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

    fn take_cleanup_parts(&mut self) -> Option<RetentionCleanupParts> {
        self.connection
            .take()
            .map(|connection| RetentionCleanupParts {
                connection,
                admission: self.admission.take(),
                archive_lock: self.archive_lock.take(),
                transaction_started: self.transaction_started,
            })
    }

    async fn cleanup(
        mut self,
        mode: RetentionConnectionCleanup,
        retain_archive_lock: bool,
    ) -> Result<Option<RetentionArchiveFileLock>> {
        let Some(parts) = self.take_cleanup_parts() else {
            self.release_resources();
            return Ok(None);
        };
        #[cfg(test)]
        let return_probe = RETENTION_TEST_RETURN_TO_POOL.try_with(Arc::clone).ok();
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
        self.cleanup(RetentionConnectionCleanup::RollbackAndClose, false)
            .await
            .map(|_| ())
    }

    pub(crate) async fn return_to_pool(self) -> Result<()> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool, false)
            .await
            .map(|_| ())
    }

    pub(crate) async fn rollback_and_close_retaining_archive_fence(
        self,
    ) -> Result<RetentionArchiveFileLock> {
        self.cleanup(RetentionConnectionCleanup::RollbackAndClose, true)
            .await?
            .ok_or_else(|| anyhow!("archive publication lost its file fence"))
    }

    pub(crate) async fn return_to_pool_retaining_archive_fence(
        self,
    ) -> Result<RetentionArchiveFileLock> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool, true)
            .await?
            .ok_or_else(|| anyhow!("archive publication lost its file fence"))
    }
}

async fn cleanup_retention_transaction(
    mut parts: RetentionCleanupParts,
    mode: RetentionConnectionCleanup,
    #[cfg(test)] return_probe: Option<Arc<RetentionReturnToPoolProbe>>,
    #[cfg(not(test))] _return_probe: Option<()>,
    retain_archive_lock: bool,
) -> Result<Option<RetentionArchiveFileLock>> {
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
    drop(parts.admission);
    cleanup_error.map_or(Ok(archive_lock), Err)
}

impl Drop for RetentionTransactionResources {
    fn drop(&mut self) {
        let Some(parts) = self.take_cleanup_parts() else {
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
