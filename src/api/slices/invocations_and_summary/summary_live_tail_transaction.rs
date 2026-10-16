use super::*;
use crate::db_pressure::DbBackgroundPermit;
use crate::proxy_sqlite_write_coordinator::{
    ProxySqliteWriteClass, ProxySqliteWritePermit, proxy_sqlite_write_coordinator,
};

/// A cancelled publication must not release logical admission ahead of the native worker.
pub(crate) struct SummaryLiveTailTransaction {
    parts: Option<SummaryLiveTailTransactionParts>,
}

struct SummaryLiveTailTransactionParts {
    connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    _write_permit: ProxySqliteWritePermit,
    _background_permit: Arc<DbBackgroundPermit>,
    transaction_started: bool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static SUMMARY_LIVE_TAIL_CLEANUP_READY: Arc<tokio::sync::Notify>;
}

impl SummaryLiveTailTransaction {
    pub(super) async fn try_acquire(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        background_permit: Arc<DbBackgroundPermit>,
    ) -> Result<Option<Self>> {
        // Acquire a connection before claiming the writer. Pool waits never reserve a writer;
        // writer admission itself cannot queue while this recovery holds a background slot.
        let connection = pool.acquire().await?;
        let Some(write_permit) =
            proxy_sqlite_write_coordinator().try_acquire(ProxySqliteWriteClass::P2Derived)
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            parts: Some(SummaryLiveTailTransactionParts {
                connection,
                _write_permit: write_permit,
                _background_permit: background_permit,
                transaction_started: false,
            }),
        }))
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
        write_permit: ProxySqliteWritePermit,
        background_permit: Arc<DbBackgroundPermit>,
    ) -> Self {
        Self {
            parts: Some(SummaryLiveTailTransactionParts {
                connection,
                _write_permit: write_permit,
                _background_permit: background_permit,
                transaction_started: false,
            }),
        }
    }

    pub(crate) fn connection_mut(&mut self) -> &mut SqliteConnection {
        &mut self.parts.as_mut().expect("live-tail resources").connection
    }

    pub(crate) async fn begin(&mut self) -> Result<()> {
        // BEGIN may reach SQLite even if its Rust future is cancelled before receiving ACK.
        self.parts
            .as_mut()
            .expect("live-tail resources")
            .transaction_started = true;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(self.connection_mut())
            .await
            .context("begin Summary live-tail publication transaction")?;
        Ok(())
    }

    pub(crate) async fn finish(mut self, commit: bool) -> Result<()> {
        let mut parts = self.parts.take().expect("live-tail resources");
        // Dropping this waiter cannot abort the independent native cleanup owner.
        let owner = tokio::spawn(async move {
            let result = sqlx::query(if commit { "COMMIT" } else { "ROLLBACK" })
                .execute(&mut *parts.connection)
                .await;
            if result.is_ok() {
                parts.transaction_started = false;
            }
            let cleanup = drain_live_tail_transaction(parts).await;
            result.context("finish Summary live-tail publication transaction")?;
            cleanup
        });
        #[cfg(test)]
        let _ = SUMMARY_LIVE_TAIL_CLEANUP_READY.try_with(|ready| ready.notify_one());
        owner
            .await
            .context("join Summary live-tail publication owner")?
    }
}

async fn drain_live_tail_transaction(mut parts: SummaryLiveTailTransactionParts) -> Result<()> {
    let cleanup = if parts.transaction_started {
        // Even an unsuccessful rollback is followed by close, which drains the native worker.
        let _ = sqlx::query("ROLLBACK")
            .execute(&mut *parts.connection)
            .await;
        parts
            .connection
            .close()
            .await
            .context("close cancelled Summary live-tail transaction")
    } else {
        parts.connection.return_to_pool().await;
        Ok(())
    };
    drop(parts._background_permit);
    drop(parts._write_permit);
    cleanup
}

impl Drop for SummaryLiveTailTransaction {
    fn drop(&mut self) {
        let Some(parts) = self.parts.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(async move {
                if let Err(error) = drain_live_tail_transaction(parts).await {
                    warn!(?error, "Summary live-tail native cleanup failed");
                }
            }));
        } else {
            // Runtime teardown cannot provide an asynchronous drain acknowledgement.
            std::mem::forget(parts);
        }
    }
}
