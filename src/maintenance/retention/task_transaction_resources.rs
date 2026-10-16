use super::{RetentionArchiveFileLock, RetentionWriteAdmission};
use anyhow::{Context, Result, anyhow};
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

    async fn cleanup(mut self, mode: RetentionConnectionCleanup) -> Result<()> {
        let Some(parts) = self.take_cleanup_parts() else {
            self.release_resources();
            return Ok(());
        };
        #[cfg(test)]
        let return_probe = RETENTION_TEST_RETURN_TO_POOL.try_with(Arc::clone).ok();
        #[cfg(not(test))]
        let return_probe = None;
        let handle = tokio::spawn(cleanup_retention_transaction(parts, mode, return_probe));
        handle
            .await
            .map_err(|error| anyhow!("retention transaction cleanup task failed: {error}"))??;
        Ok(())
    }

    pub(crate) async fn rollback_and_close(self) -> Result<()> {
        self.cleanup(RetentionConnectionCleanup::RollbackAndClose)
            .await
    }

    pub(crate) async fn return_to_pool(self) -> Result<()> {
        self.cleanup(RetentionConnectionCleanup::ReturnToPool).await
    }
}

async fn cleanup_retention_transaction(
    mut parts: RetentionCleanupParts,
    mode: RetentionConnectionCleanup,
    #[cfg(test)] return_probe: Option<Arc<RetentionReturnToPoolProbe>>,
    #[cfg(not(test))] _return_probe: Option<()>,
) -> Result<()> {
    let mut cleanup_error = None;
    if matches!(mode, RetentionConnectionCleanup::RollbackAndClose) && parts.transaction_started {
        if let Err(error) = sqlx::query("ROLLBACK")
            .execute(&mut *parts.connection)
            .await
        {
            cleanup_error = Some(anyhow!("retention transaction rollback failed: {error}"));
        }
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
    drop(parts.archive_lock);
    drop(parts.admission);
    cleanup_error.map_or(Ok(()), Err)
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
            )));
        } else {
            // No asynchronous rollback can complete after the runtime has exited. Leak the
            // checked-out connection until process teardown instead of returning dirty state to
            // the pool; normal runtime cancellation always takes the detached drain above.
            let RetentionCleanupParts {
                connection,
                admission,
                archive_lock,
                ..
            } = parts;
            std::mem::forget(connection);
            drop(archive_lock);
            drop(admission);
        }
    }
}
