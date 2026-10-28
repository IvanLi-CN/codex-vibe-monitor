use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Instant;

use anyhow::{Result, bail};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::{PgConnection, Pool, Postgres, QueryBuilder, Sqlite, SqliteConnection};

use super::core::{
    BackendKind, ModuleAuthority, ModuleGenerationGate, ModuleGenerationLease, StorageModule,
    StoragePools, TrafficArchiveTransaction, TypedStorageTransaction,
};
use super::migration::{SourceWriteFence, SourceWriteLease};

const SQLITE_TRAFFIC_AUTHORITY_GENERATION: u64 = 1;

/// The durable identity assigned to an invocation before its dependent traffic facts are
/// written. Attempts and source-change descriptors must retain this identity within the same
/// transaction; they must not independently resolve a different invocation row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrafficFactIdentity {
    pub(crate) invocation_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl TrafficFactIdentity {
    pub(crate) fn new(
        invocation_id: i64,
        invoke_id: impl Into<String>,
        occurred_at: impl Into<String>,
    ) -> Result<Self> {
        let identity = Self {
            invocation_id,
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.invocation_id <= 0 {
            bail!("traffic fact identity requires a positive invocation id");
        }
        if self.invoke_id.trim().is_empty() || self.occurred_at.trim().is_empty() {
            bail!("traffic fact identity requires invocation and timestamp keys");
        }
        Ok(())
    }
}

/// Runtime-owned traffic transactions. The generation lease is acquired for the short fact
/// commit only; request streams and terminal journals retain their own durable obligations and
/// never hold a database lease while waiting on an upstream or downstream peer.
pub(crate) struct TrafficFactStore {
    pools: StoragePools,
    gate: ModuleGenerationGate,
    source_fence: Option<SourceWriteFence>,
}

/// Authority and source-fence lease for SQLite writes that are not part of a traffic fact
/// transaction but still must not race a traffic-module authority switch.
pub(crate) struct TrafficWriteAuthorityLease {
    _generation_lease: ModuleGenerationLease,
    _source_write_lease: Option<SourceWriteLease>,
}

impl fmt::Debug for TrafficFactStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrafficFactStore")
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

impl TrafficFactStore {
    pub(crate) fn with_authority(pools: StoragePools, gate: ModuleGenerationGate) -> Self {
        Self {
            pools,
            gate,
            source_fence: None,
        }
    }

    pub(crate) fn with_authority_and_fence(
        pools: StoragePools,
        gate: ModuleGenerationGate,
        source_fence: SourceWriteFence,
    ) -> Self {
        Self {
            pools,
            gate,
            source_fence: Some(source_fence),
        }
    }

    pub(crate) fn sqlite(pool: Pool<Sqlite>) -> Self {
        Self::with_authority(
            StoragePools::sqlite_only(pool),
            ModuleGenerationGate::new(
                StorageModule::TrafficArchive,
                ModuleAuthority {
                    backend: BackendKind::Sqlite,
                    generation: SQLITE_TRAFFIC_AUTHORITY_GENERATION,
                },
            ),
        )
    }

    pub(crate) async fn begin(&self) -> Result<TrafficFactTransaction> {
        let (transaction, _) = self.begin_with_pool_acquisition_duration().await?;
        Ok(transaction)
    }

    pub(crate) async fn authority_backend(&self) -> BackendKind {
        self.gate.current_authority().await.backend
    }

    pub(crate) async fn acquire_write_authority(&self) -> Result<TrafficWriteAuthorityLease> {
        let generation_lease = self.gate.acquire(StorageModule::TrafficArchive).await?;
        let source_write_lease = if generation_lease.lease.authority.backend == BackendKind::Sqlite
        {
            match &self.source_fence {
                Some(source_fence) => Some(source_fence.begin_write().await?),
                None => None,
            }
        } else {
            None
        };
        Ok(TrafficWriteAuthorityLease {
            _generation_lease: generation_lease,
            _source_write_lease: source_write_lease,
        })
    }

    pub(crate) async fn begin_with_pool_acquisition_duration(
        &self,
    ) -> Result<(TrafficFactTransaction, std::time::Duration)> {
        let lease = self.gate.acquire(StorageModule::TrafficArchive).await?;
        let backend = lease.lease.authority.backend;
        let source_write_lease = if backend == BackendKind::Sqlite {
            if let Some(source_fence) = &self.source_fence {
                Some(source_fence.begin_write().await?)
            } else {
                None
            }
        } else {
            None
        };
        let pool_acquisition_started = Instant::now();
        let transaction = self
            .pools
            .begin_typed::<TrafficArchiveTransaction>(lease)
            .await?;
        Ok((
            TrafficFactTransaction {
                transaction,
                backend,
                _source_write_lease: source_write_lease,
            },
            pool_acquisition_started.elapsed(),
        ))
    }

    /// Read routing facts through the same module authority contract as writes. A runtime-owned
    /// store is constructed with the shared pools and generation gate; the SQLite constructor is
    /// the current pre-cutover adapter and does not perform authority migration.
    pub(crate) async fn load_transport_decode_sticky_escape_expiries(
        &self,
        account_ids: &[i64],
        now: DateTime<Utc>,
    ) -> Result<(HashMap<i64, DateTime<Utc>>, usize)> {
        if account_ids.is_empty() {
            return Ok((HashMap::new(), 0));
        }

        let mut seen_account_ids = HashSet::new();
        let unique_account_ids = account_ids
            .iter()
            .copied()
            .filter(|account_id| seen_account_ids.insert(*account_id))
            .collect::<Vec<_>>();
        let lease = self.gate.acquire(StorageModule::TrafficArchive).await?;
        let backend = lease.lease.authority.backend;
        let mut transaction = self
            .pools
            .begin_typed::<TrafficArchiveTransaction>(lease)
            .await?;
        let rows = match backend {
            BackendKind::Sqlite => {
                load_transport_decode_sticky_escape_rows_sqlite(
                    transaction.sqlite_transaction()?.as_mut(),
                    &unique_account_ids,
                )
                .await
            }
            BackendKind::Postgres => {
                load_transport_decode_sticky_escape_rows_postgres(
                    transaction.postgres_transaction()?.as_mut(),
                    &unique_account_ids,
                )
                .await
            }
        };
        let rollback_result = transaction.rollback().await;
        let rows = finish_authority_read(rows, rollback_result)?;
        Ok(evaluate_transport_decode_sticky_escape_rows(rows, now))
    }
}

pub(crate) struct TrafficFactTransaction {
    transaction: TypedStorageTransaction<TrafficArchiveTransaction>,
    backend: BackendKind,
    _source_write_lease: Option<SourceWriteLease>,
}

impl TrafficFactTransaction {
    pub(crate) fn backend_kind(&self) -> BackendKind {
        self.backend
    }

    pub(crate) fn sqlite_transaction(&mut self) -> Result<&mut sqlx::Transaction<'static, Sqlite>> {
        self.transaction.sqlite_transaction()
    }

    pub(crate) fn sqlite_connection(&mut self) -> Result<&mut SqliteConnection> {
        Ok(self.transaction.sqlite_transaction()?.as_mut())
    }

    pub(crate) fn postgres_transaction(
        &mut self,
    ) -> Result<&mut sqlx::Transaction<'static, Postgres>> {
        self.transaction.postgres_transaction()
    }

    pub(crate) fn postgres_connection(&mut self) -> Result<&mut PgConnection> {
        Ok(self.transaction.postgres_transaction()?.as_mut())
    }

    pub(crate) async fn commit(self) -> Result<()> {
        self.transaction.commit().await
    }

    pub(crate) async fn rollback(self) -> Result<()> {
        self.transaction.rollback().await
    }
}

pub(crate) fn finish_authority_read<T>(
    result: Result<T>,
    rollback_result: Result<()>,
) -> Result<T> {
    match (result, rollback_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(rollback_error)) => Err(anyhow::anyhow!(
            "traffic authority read rollback failed: {rollback_error:#}"
        )),
        (Err(error), Err(rollback_error)) => Err(anyhow::anyhow!(
            "traffic authority read failed: {error:#}; rollback failed: {rollback_error:#}"
        )),
    }
}

type StickyEscapeRow = (i64, String, Option<String>);

async fn load_transport_decode_sticky_escape_rows_sqlite(
    connection: &mut SqliteConnection,
    account_ids: &[i64],
) -> Result<Vec<StickyEscapeRow>> {
    let mut rows = Vec::new();
    // SQLite limits compound SELECT terms; each account subquery stays capped at two rows.
    for account_id_batch in account_ids.chunks(400) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        for (index, account_id) in account_id_batch.iter().enumerate() {
            if index > 0 {
                query.push(" UNION ALL ");
            }
            query
                .push("SELECT ")
                .push_bind(*account_id)
                .push(" AS upstream_account_id, occurred_at, failure_kind FROM (")
                .push("SELECT occurred_at, failure_kind FROM pool_upstream_request_attempts ")
                .push("WHERE upstream_account_id = ")
                .push_bind(*account_id)
                .push(" AND route_mode = ")
                .push_bind(crate::INVOCATION_ROUTE_MODE_POOL)
                .push(" AND endpoint = '/v1/responses' AND phase IN (")
                .push_bind(crate::POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
                .push(", ")
                .push_bind(crate::POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
                .push(") ORDER BY occurred_at DESC, id DESC LIMIT 2) AS latest_attempts");
        }
        rows.extend(
            query
                .build_query_as::<StickyEscapeRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    Ok(rows)
}

async fn load_transport_decode_sticky_escape_rows_postgres(
    connection: &mut PgConnection,
    account_ids: &[i64],
) -> Result<Vec<StickyEscapeRow>> {
    let mut rows = Vec::new();
    for account_id_batch in account_ids.chunks(400) {
        let mut query = QueryBuilder::<Postgres>::new("");
        for (index, account_id) in account_id_batch.iter().enumerate() {
            if index > 0 {
                query.push(" UNION ALL ");
            }
            query
                .push("SELECT ")
                .push_bind(*account_id)
                .push(" AS upstream_account_id, occurred_at, failure_kind FROM (")
                .push("SELECT occurred_at, failure_kind FROM pool_upstream_request_attempts ")
                .push("WHERE upstream_account_id = ")
                .push_bind(*account_id)
                .push(" AND route_mode = ")
                .push_bind(crate::INVOCATION_ROUTE_MODE_POOL)
                .push(" AND endpoint = '/v1/responses' AND phase IN (")
                .push_bind(crate::POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
                .push(", ")
                .push_bind(crate::POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
                .push(") ORDER BY occurred_at DESC, id DESC LIMIT 2) AS latest_attempts");
        }
        rows.extend(
            query
                .build_query_as::<StickyEscapeRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    Ok(rows)
}

fn evaluate_transport_decode_sticky_escape_rows(
    rows: Vec<StickyEscapeRow>,
    now: DateTime<Utc>,
) -> (HashMap<i64, DateTime<Utc>>, usize) {
    let row_count = rows.len();
    let mut latest_failures = HashMap::<i64, Vec<(String, Option<String>)>>::new();
    for (account_id, occurred_at, failure_kind) in rows {
        latest_failures
            .entry(account_id)
            .or_default()
            .push((occurred_at, failure_kind));
    }
    let window = ChronoDuration::seconds(crate::POOL_ROUTE_BINDING_FAILURE_PENALTY_WINDOW_SECS);
    let states = latest_failures
        .into_iter()
        .filter_map(|(account_id, failures)| {
            if failures.len() != 2
                || !failures.iter().all(|(_, failure_kind)| {
                    failure_kind.as_deref() == Some(crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
                })
            {
                return None;
            }
            let latest_failure_at = failures
                .iter()
                .filter_map(|(occurred_at, _)| crate::parse_to_utc_datetime(occurred_at))
                .max()?;
            let oldest_failure_at = failures
                .iter()
                .filter_map(|(occurred_at, _)| crate::parse_to_utc_datetime(occurred_at))
                .min()?;
            if latest_failure_at > now || oldest_failure_at < now - window {
                return None;
            }
            let until = latest_failure_at + window;
            (now < until).then_some((account_id, until))
        })
        .collect();
    (states, row_count)
}
