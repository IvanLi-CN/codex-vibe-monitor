//! Keep the reservation permit with its native connection through settlement.
use super::*;

pub(crate) struct RangeTransaction {
    parts: Option<Parts>,
    started: Instant,
}

struct Parts {
    connection: sqlx::pool::PoolConnection<Sqlite>,
    permit: ProxySqliteWritePermit,
    active: bool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RANGE_FINISH_OWNER_READY: Arc<Notify>;
}

impl RangeTransaction {
    pub(crate) async fn begin(pool: &Pool<Sqlite>, permit: ProxySqliteWritePermit) -> Result<Self> {
        let started = Instant::now();
        let connection = pool.acquire().await?;
        observe_range_phase(InvocationRangePhase::Connection, started);
        let mut owner = Self {
            parts: Some(Parts {
                connection,
                permit,
                active: true,
            }),
            started: Instant::now(),
        };
        sqlx::query("BEGIN IMMEDIATE")
            .execute(owner.connection())
            .await?;
        Ok(owner)
    }

    pub(crate) fn connection(&mut self) -> &mut SqliteConnection {
        &mut self
            .parts
            .as_mut()
            .expect("reservation native owner")
            .connection
    }

    pub(crate) async fn finish(mut self, commit: bool) -> Result<()> {
        let mut parts = self.parts.take().expect("reservation native owner");
        let owner = tokio::spawn(async move {
            let result = sqlx::query(if commit { "COMMIT" } else { "ROLLBACK" })
                .execute(&mut *parts.connection)
                .await;
            parts.active = result.is_err();
            drain(parts).await;
            result.map(|_| ()).context("settle invocation reservation")
        });
        #[cfg(test)]
        let _ = RANGE_FINISH_OWNER_READY.try_with(|ready| ready.notify_one());
        let result = owner.await.context("join invocation reservation owner")?;
        observe_range_phase(InvocationRangePhase::Transaction, self.started);
        result
    }
}

async fn drain(mut parts: Parts) {
    if parts.active {
        let _ = sqlx::query("ROLLBACK")
            .execute(&mut *parts.connection)
            .await;
        if let Err(error) = parts.connection.close().await {
            warn!(%error, "invocation reservation native close failed");
        }
    } else {
        parts.connection.return_to_pool().await;
    }
    drop(parts.permit);
}

impl Drop for RangeTransaction {
    fn drop(&mut self) {
        let Some(parts) = self.parts.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(drain(parts)));
        } else {
            // No asynchronous drain acknowledgement is possible at runtime teardown.
            std::mem::forget(parts);
        }
    }
}
