//! SQLite-owned analytics backend control and process-local admission.
//!
//! The control row is the source-side authority for backend transitions. PostgreSQL keeps its
//! own owner fence, but it cannot make a transition safe by itself: a service restart must first
//! observe this row and re-establish admission only after the corresponding source/target proof
//! has been checked.

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use std::{
    fmt,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::{
    sync::Notify,
    time::{Duration, timeout},
};

use super::source::{SourceSnapshot, assert_source_snapshot_usable};

pub(crate) const ANALYTICS_ROLLBACK_GUARANTEE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub(crate) const ANALYTICS_OWNER_LEASE: Duration = Duration::from_secs(30);
const ANALYTICS_CONTROL_SOURCE_INSTANCE_ID: &str = super::LEGACY_SOURCE_INSTANCE_ID;

const ANALYTICS_CONTROL_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS analytics_backend_control (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    source_instance_id TEXT NOT NULL,
    owner_id TEXT,
    owner_epoch INTEGER NOT NULL CHECK (owner_epoch > 0),
    owner_lease_expires_at TEXT,
    desired_backend TEXT NOT NULL CHECK (desired_backend IN ('sqlite', 'postgres')),
    active_backend TEXT NOT NULL CHECK (active_backend IN ('sqlite', 'postgres')),
    phase TEXT NOT NULL CHECK (phase IN (
        'sqlite_active', 'pg_building', 'pg_prepared', 'switch_pending',
        'pg_active', 'rollback_building', 'sqlite_prepared'
    )),
    build_id TEXT,
    certificate_id TEXT,
    fence_generation INTEGER CHECK (fence_generation IS NULL OR fence_generation >= 0),
    fence_high_watermark_change_id INTEGER
        CHECK (fence_high_watermark_change_id IS NULL OR fence_high_watermark_change_id >= 0),
    fence_coverage_revision INTEGER
        CHECK (fence_coverage_revision IS NULL OR fence_coverage_revision >= 0),
    rollback_deadline_at TEXT,
    read_admission_open INTEGER NOT NULL DEFAULT 0 CHECK (read_admission_open IN (0, 1)),
    write_admission_open INTEGER NOT NULL DEFAULT 0 CHECK (write_admission_open IN (0, 1)),
    error_code TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
)
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsControlPhase {
    SqliteActive,
    PgBuilding,
    PgPrepared,
    SwitchPending,
    PgActive,
    RollbackBuilding,
    SqlitePrepared,
}

impl AnalyticsControlPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SqliteActive => "sqlite_active",
            Self::PgBuilding => "pg_building",
            Self::PgPrepared => "pg_prepared",
            Self::SwitchPending => "switch_pending",
            Self::PgActive => "pg_active",
            Self::RollbackBuilding => "rollback_building",
            Self::SqlitePrepared => "sqlite_prepared",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "sqlite_active" => Ok(Self::SqliteActive),
            "pg_building" => Ok(Self::PgBuilding),
            "pg_prepared" => Ok(Self::PgPrepared),
            "switch_pending" => Ok(Self::SwitchPending),
            "pg_active" => Ok(Self::PgActive),
            "rollback_building" => Ok(Self::RollbackBuilding),
            "sqlite_prepared" => Ok(Self::SqlitePrepared),
            _ => bail!("unknown analytics control phase: {raw}"),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::SqliteActive, Self::PgBuilding)
                | (Self::PgBuilding, Self::PgPrepared)
                | (Self::PgPrepared, Self::SwitchPending)
                | (Self::SwitchPending, Self::PgActive)
                | (Self::PgActive, Self::RollbackBuilding)
                | (Self::RollbackBuilding, Self::SqlitePrepared)
                | (Self::SqlitePrepared, Self::SqliteActive)
                // A failed or interrupted transition may be resumed from its durable phase.
                | (Self::PgBuilding, Self::PgBuilding)
                | (Self::PgPrepared, Self::PgPrepared)
                | (Self::SwitchPending, Self::SwitchPending)
                | (Self::PgActive, Self::PgActive)
                | (Self::RollbackBuilding, Self::RollbackBuilding)
                | (Self::SqlitePrepared, Self::SqlitePrepared)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsBackendControl {
    pub(crate) source_instance_id: String,
    pub(crate) owner_id: Option<String>,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_lease_expires_at: Option<String>,
    pub(crate) desired_backend: String,
    pub(crate) active_backend: String,
    pub(crate) phase: AnalyticsControlPhase,
    pub(crate) build_id: Option<String>,
    pub(crate) certificate_id: Option<String>,
    pub(crate) fence_generation: Option<i64>,
    pub(crate) fence_high_watermark_change_id: Option<i64>,
    pub(crate) fence_coverage_revision: Option<i64>,
    pub(crate) rollback_deadline_at: Option<String>,
    pub(crate) read_admission_open: bool,
    pub(crate) write_admission_open: bool,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, FromRow)]
struct AnalyticsBackendControlRow {
    source_instance_id: String,
    owner_id: Option<String>,
    owner_epoch: i64,
    owner_lease_expires_at: Option<String>,
    desired_backend: String,
    active_backend: String,
    phase: String,
    build_id: Option<String>,
    certificate_id: Option<String>,
    fence_generation: Option<i64>,
    fence_high_watermark_change_id: Option<i64>,
    fence_coverage_revision: Option<i64>,
    rollback_deadline_at: Option<String>,
    read_admission_open: i64,
    write_admission_open: i64,
    error_code: Option<String>,
}

impl TryFrom<AnalyticsBackendControlRow> for AnalyticsBackendControl {
    type Error = anyhow::Error;

    fn try_from(row: AnalyticsBackendControlRow) -> Result<Self> {
        ensure!(
            row.owner_epoch > 0,
            "analytics control owner epoch is invalid"
        );
        ensure!(
            (0..=1).contains(&row.read_admission_open)
                && (0..=1).contains(&row.write_admission_open),
            "analytics control admission flag is invalid"
        );
        Ok(Self {
            source_instance_id: row.source_instance_id,
            owner_id: row.owner_id,
            owner_epoch: row.owner_epoch,
            owner_lease_expires_at: row.owner_lease_expires_at,
            desired_backend: row.desired_backend,
            active_backend: row.active_backend,
            phase: AnalyticsControlPhase::parse(&row.phase)?,
            build_id: row.build_id,
            certificate_id: row.certificate_id,
            fence_generation: row.fence_generation,
            fence_high_watermark_change_id: row.fence_high_watermark_change_id,
            fence_coverage_revision: row.fence_coverage_revision,
            rollback_deadline_at: row.rollback_deadline_at,
            read_admission_open: row.read_admission_open == 1,
            write_admission_open: row.write_admission_open == 1,
            error_code: row.error_code,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsOwnerLease {
    pub(crate) owner_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) lease_expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsSourceFence {
    pub(crate) generation: i64,
    pub(crate) through_change_id: i64,
    pub(crate) coverage_revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsAdmissionSnapshot {
    pub(crate) epoch: i64,
    pub(crate) read_open: bool,
    pub(crate) write_open: bool,
    pub(crate) active_readers: usize,
    pub(crate) active_writers: usize,
    pub(crate) draining: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalyticsAdmissionError {
    Closed,
    EpochFenced,
    WrongBackend,
    Draining,
}

impl fmt::Display for AnalyticsAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Closed => "analytics admission is closed",
            Self::EpochFenced => "analytics admission epoch was fenced",
            Self::WrongBackend => {
                "analytics admission backend does not match the requested backend"
            }
            Self::Draining => "analytics admission is draining",
        })
    }
}

impl std::error::Error for AnalyticsAdmissionError {}

#[derive(Debug)]
struct AdmissionState {
    epoch: i64,
    backend: Option<String>,
    read_open: bool,
    write_open: bool,
    active_readers: usize,
    active_writers: usize,
    draining: bool,
}

/// Process-local admission is deliberately separate from the durable source row. It closes
/// immediately on a transition and permits already admitted operations to drain without
/// allowing a new operation to race the epoch change.
#[derive(Debug, Clone)]
pub(crate) struct AnalyticsAdmission {
    state: Arc<StdMutex<AdmissionState>>,
    notify: Arc<Notify>,
    #[cfg(test)]
    wait_started: Arc<Notify>,
}

impl Default for AnalyticsAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalyticsAdmission {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(StdMutex::new(AdmissionState {
                epoch: 0,
                backend: None,
                read_open: false,
                write_open: false,
                active_readers: 0,
                active_writers: 0,
                draining: false,
            })),
            notify: Arc::new(Notify::new()),
            #[cfg(test)]
            wait_started: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn snapshot(&self) -> AnalyticsAdmissionSnapshot {
        let state = self.state.lock().expect("analytics admission state");
        AnalyticsAdmissionSnapshot {
            epoch: state.epoch,
            read_open: state.read_open,
            write_open: state.write_open,
            active_readers: state.active_readers,
            active_writers: state.active_writers,
            draining: state.draining,
        }
    }

    pub(crate) fn open(&self, backend: &str, epoch: i64) {
        let mut state = self.state.lock().expect("analytics admission state");
        state.epoch = epoch;
        state.backend = Some(backend.to_string());
        state.read_open = true;
        state.write_open = true;
        state.draining = false;
        drop(state);
        self.notify.notify_waiters();
    }

    pub(crate) fn open_write_only(&self, backend: &str, epoch: i64) {
        let mut state = self.state.lock().expect("analytics admission state");
        state.epoch = epoch;
        state.backend = Some(backend.to_string());
        state.read_open = false;
        state.write_open = true;
        state.draining = false;
        drop(state);
        self.notify.notify_waiters();
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock().expect("analytics admission state");
        state.read_open = false;
        state.write_open = false;
        state.draining = true;
        drop(state);
        self.notify.notify_waiters();
    }

    pub(crate) fn try_read(
        &self,
        backend: &str,
        epoch: i64,
    ) -> std::result::Result<AnalyticsReadPermit, AnalyticsAdmissionError> {
        let mut state = self.state.lock().expect("analytics admission state");
        if state.epoch != epoch {
            return Err(AnalyticsAdmissionError::EpochFenced);
        }
        if state.backend.as_deref() != Some(backend) {
            return Err(AnalyticsAdmissionError::WrongBackend);
        }
        if state.draining {
            return Err(AnalyticsAdmissionError::Draining);
        }
        if !state.read_open {
            return Err(AnalyticsAdmissionError::Closed);
        }
        state.active_readers = state.active_readers.saturating_add(1);
        Ok(AnalyticsReadPermit {
            admission: self.clone(),
        })
    }

    pub(crate) fn try_write(
        &self,
        backend: &str,
        epoch: i64,
    ) -> std::result::Result<AnalyticsWritePermit, AnalyticsAdmissionError> {
        let mut state = self.state.lock().expect("analytics admission state");
        if state.epoch != epoch {
            return Err(AnalyticsAdmissionError::EpochFenced);
        }
        if state.backend.as_deref() != Some(backend) {
            return Err(AnalyticsAdmissionError::WrongBackend);
        }
        if state.draining {
            return Err(AnalyticsAdmissionError::Draining);
        }
        if !state.write_open {
            return Err(AnalyticsAdmissionError::Closed);
        }
        state.active_writers = state.active_writers.saturating_add(1);
        Ok(AnalyticsWritePermit {
            admission: self.clone(),
        })
    }

    pub(crate) fn try_sqlite_write(
        &self,
    ) -> std::result::Result<AnalyticsWritePermit, AnalyticsAdmissionError> {
        let epoch = self.snapshot().epoch;
        self.try_write("sqlite", epoch)
    }

    pub(crate) fn sqlite_write_open(&self) -> bool {
        let state = self.state.lock().expect("analytics admission state");
        state.backend.as_deref() == Some("sqlite") && state.write_open && !state.draining
    }

    /// Wait until a SQLite analytics writer can no longer be admitted. The state check is
    /// deliberately performed after arming `Notify::Notified` so a close racing this waiter
    /// cannot be lost between the check and the await.
    pub(crate) async fn wait_until_sqlite_write_closed(&self) {
        #[cfg(test)]
        self.wait_started.notify_waiters();
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.sqlite_write_open() {
                return;
            }
            notified.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn wait_started_for_test(&self) -> Arc<Notify> {
        self.wait_started.clone()
    }

    pub(crate) async fn drain(&self, wait: Duration) -> Result<()> {
        timeout(wait, async {
            loop {
                let notified = self.notify.notified();
                let idle = {
                    let state = self.state.lock().expect("analytics admission state");
                    state.active_readers == 0 && state.active_writers == 0
                };
                if idle {
                    return Ok::<(), anyhow::Error>(());
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| anyhow!("analytics admission drain exceeded {}ms", wait.as_millis()))??;
        Ok(())
    }

    fn release_reader(&self) {
        let mut state = self.state.lock().expect("analytics admission state");
        state.active_readers = state.active_readers.saturating_sub(1);
        drop(state);
        self.notify.notify_waiters();
    }

    fn release_writer(&self) {
        let mut state = self.state.lock().expect("analytics admission state");
        state.active_writers = state.active_writers.saturating_sub(1);
        drop(state);
        self.notify.notify_waiters();
    }
}

#[derive(Debug)]
pub(crate) struct AnalyticsReadPermit {
    admission: AnalyticsAdmission,
}

impl Drop for AnalyticsReadPermit {
    fn drop(&mut self) {
        self.admission.release_reader();
    }
}

#[derive(Debug)]
pub(crate) struct AnalyticsWritePermit {
    admission: AnalyticsAdmission,
}

impl Drop for AnalyticsWritePermit {
    fn drop(&mut self) {
        self.admission.release_writer();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AnalyticsControl {
    pool: Pool<Sqlite>,
    admission: Arc<AnalyticsAdmission>,
    owner_id: String,
    source_instance_id: String,
}

impl AnalyticsControl {
    pub(crate) fn new(pool: Pool<Sqlite>, owner_id: impl Into<String>) -> Self {
        Self::with_source_instance(pool, owner_id, ANALYTICS_CONTROL_SOURCE_INSTANCE_ID)
    }

    pub(crate) fn with_source_instance(
        pool: Pool<Sqlite>,
        owner_id: impl Into<String>,
        source_instance_id: impl Into<String>,
    ) -> Self {
        Self {
            pool,
            admission: Arc::new(AnalyticsAdmission::new()),
            owner_id: owner_id.into(),
            source_instance_id: source_instance_id.into(),
        }
    }

    pub(crate) fn admission(&self) -> &AnalyticsAdmission {
        &self.admission
    }

    pub(crate) fn admission_handle(&self) -> Arc<AnalyticsAdmission> {
        self.admission.clone()
    }

    pub(crate) async fn close_and_drain(&self, wait: Duration) -> Result<()> {
        self.admission.close();
        self.admission.drain(wait).await
    }

    pub(crate) async fn initialize(
        &self,
        desired_backend: &str,
    ) -> Result<AnalyticsBackendControl> {
        ensure!(
            matches!(desired_backend, "sqlite" | "postgres"),
            "unknown desired analytics backend"
        );
        ensure_analytics_backend_control_schema(
            &self.pool,
            &self.source_instance_id,
            desired_backend,
        )
        .await?;
        sqlx::query(
            "UPDATE analytics_backend_control
             SET desired_backend = ?1, updated_at = datetime('now') WHERE id = 1",
        )
        .bind(desired_backend)
        .execute(&self.pool)
        .await
        .context("record desired analytics backend")?;
        let state = self.load().await?;
        // A fresh process never trusts an old in-memory admission bit. SQLite can be reopened
        // only when it is still the durable active backend; PG requires a fresh target proof.
        if state.phase == AnalyticsControlPhase::SqliteActive
            && state.active_backend == "sqlite"
            && desired_backend == "sqlite"
        {
            self.admission.open("sqlite", state.owner_epoch);
        } else {
            self.admission.close();
        }
        Ok(state)
    }

    pub(crate) async fn load(&self) -> Result<AnalyticsBackendControl> {
        let row = sqlx::query_as::<_, AnalyticsBackendControlRow>(
            "SELECT source_instance_id, owner_id, owner_epoch, owner_lease_expires_at,
                    desired_backend, active_backend,
                    phase, build_id, certificate_id, fence_generation,
                    fence_high_watermark_change_id, fence_coverage_revision,
                    rollback_deadline_at, read_admission_open, write_admission_open, error_code
             FROM analytics_backend_control WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await
        .context("load analytics backend control")?;
        row.try_into()
    }

    pub(crate) async fn claim_owner(&self, expected_epoch: i64) -> Result<AnalyticsOwnerLease> {
        ensure!(
            !self.owner_id.trim().is_empty(),
            "analytics owner id must not be empty"
        );
        let expires_at = (Utc::now()
            + ChronoDuration::seconds(ANALYTICS_OWNER_LEASE.as_secs() as i64))
        .to_rfc3339();
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET owner_id = ?1, owner_epoch = owner_epoch + 1, owner_lease_expires_at = ?2,
                 read_admission_open = 0, write_admission_open = 0, updated_at = datetime('now')
             WHERE id = 1 AND owner_epoch = ?3
               AND (owner_id IS NULL OR owner_id = ?1
                    OR owner_lease_expires_at IS NULL
                    OR julianday(owner_lease_expires_at) <= julianday('now'))",
        )
        .bind(&self.owner_id)
        .bind(&expires_at)
        .bind(expected_epoch)
        .execute(&self.pool)
        .await
        .context("claim analytics backend owner")?;
        ensure!(
            result.rows_affected() == 1,
            "analytics backend owner CAS was rejected"
        );
        self.admission.close();
        let state = self.load().await?;
        Ok(AnalyticsOwnerLease {
            owner_id: self.owner_id.clone(),
            owner_epoch: state.owner_epoch,
            lease_expires_at: expires_at,
        })
    }

    pub(crate) async fn begin_pg_build(&self) -> Result<AnalyticsOwnerLease> {
        let state = self.load().await?;
        ensure!(
            state.desired_backend == "postgres",
            "PostgreSQL build is not the desired analytics backend"
        );
        let lease = self.claim_owner(state.owner_epoch).await?;
        match state.phase {
            AnalyticsControlPhase::SqliteActive => {
                self.transition(
                    &lease,
                    AnalyticsControlPhase::SqliteActive,
                    AnalyticsControlPhase::PgBuilding,
                )
                .await?;
            }
            AnalyticsControlPhase::PgBuilding | AnalyticsControlPhase::PgPrepared => {
                self.transition(&lease, state.phase, state.phase).await?;
            }
            AnalyticsControlPhase::SwitchPending | AnalyticsControlPhase::PgActive => {}
            AnalyticsControlPhase::RollbackBuilding | AnalyticsControlPhase::SqlitePrepared => {
                bail!(
                    "cannot start PostgreSQL build from phase {}",
                    state.phase.as_str()
                )
            }
        }
        Ok(lease)
    }

    pub(crate) async fn enter_switch_pending(
        &self,
        lease: &AnalyticsOwnerLease,
        wait: Duration,
    ) -> Result<AnalyticsBackendControl> {
        self.close_and_drain(wait).await?;
        self.transition(
            lease,
            AnalyticsControlPhase::PgPrepared,
            AnalyticsControlPhase::SwitchPending,
        )
        .await
    }

    pub(crate) async fn renew_owner(&self, lease: &AnalyticsOwnerLease) -> Result<()> {
        let expires_at = (Utc::now()
            + ChronoDuration::seconds(ANALYTICS_OWNER_LEASE.as_secs() as i64))
        .to_rfc3339();
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET owner_lease_expires_at = ?1, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?2 AND owner_epoch = ?3",
        )
        .bind(&expires_at)
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("renew analytics backend owner")?;
        ensure!(
            result.rows_affected() == 1,
            "analytics backend owner lease was fenced"
        );
        Ok(())
    }

    pub(crate) async fn release_owner(&self, lease: &AnalyticsOwnerLease) -> Result<()> {
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET owner_id = NULL, owner_lease_expires_at = NULL, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?1 AND owner_epoch = ?2",
        )
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("release analytics backend owner")?;
        ensure!(
            result.rows_affected() == 1,
            "analytics backend owner release was fenced"
        );
        Ok(())
    }

    pub(crate) async fn transition(
        &self,
        lease: &AnalyticsOwnerLease,
        expected_phase: AnalyticsControlPhase,
        next_phase: AnalyticsControlPhase,
    ) -> Result<AnalyticsBackendControl> {
        ensure!(
            expected_phase.can_transition_to(next_phase),
            "invalid analytics control transition {} -> {}",
            expected_phase.as_str(),
            next_phase.as_str()
        );
        let sqlite_shadow_open = matches!(
            next_phase,
            AnalyticsControlPhase::PgBuilding | AnalyticsControlPhase::PgPrepared
        );
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET phase = ?1, read_admission_open = ?5, write_admission_open = ?5,
                 updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?2 AND owner_epoch = ?3 AND phase = ?4",
        )
        .bind(next_phase.as_str())
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .bind(expected_phase.as_str())
        .bind(i64::from(sqlite_shadow_open))
        .execute(&self.pool)
        .await
        .context("transition analytics backend control")?;
        ensure!(
            result.rows_affected() == 1,
            "analytics backend control transition CAS was rejected"
        );
        if sqlite_shadow_open {
            self.admission.open("sqlite", lease.owner_epoch);
        } else {
            self.admission.close();
        }
        self.load().await
    }

    pub(crate) async fn publish_pg_active(
        &self,
        lease: &AnalyticsOwnerLease,
        build_id: &str,
        certificate_id: &str,
        fence: AnalyticsSourceFence,
    ) -> Result<AnalyticsBackendControl> {
        ensure!(
            !build_id.trim().is_empty(),
            "analytics build id must not be empty"
        );
        ensure!(
            !certificate_id.trim().is_empty(),
            "analytics certificate id must not be empty"
        );
        let deadline = (Utc::now()
            + ChronoDuration::seconds(ANALYTICS_ROLLBACK_GUARANTEE.as_secs() as i64))
        .to_rfc3339();
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET active_backend = 'postgres', phase = 'pg_active', build_id = ?1,
                 certificate_id = ?2, fence_generation = ?3,
                 fence_high_watermark_change_id = ?4, fence_coverage_revision = ?5,
                 rollback_deadline_at = ?6, read_admission_open = 1,
                 write_admission_open = 1, error_code = NULL, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?7 AND owner_epoch = ?8
               AND phase = 'switch_pending' AND desired_backend = 'postgres'",
        )
        .bind(build_id)
        .bind(certificate_id)
        .bind(fence.generation)
        .bind(fence.through_change_id)
        .bind(fence.coverage_revision)
        .bind(&deadline)
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("publish PostgreSQL analytics active state")?;
        ensure!(
            result.rows_affected() == 1,
            "PostgreSQL active CAS was rejected"
        );
        self.admission.open("postgres", lease.owner_epoch);
        self.load().await
    }

    pub(crate) async fn admit_pg_active(
        &self,
        lease: &AnalyticsOwnerLease,
    ) -> Result<AnalyticsBackendControl> {
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET read_admission_open = 1, write_admission_open = 1,
                 error_code = NULL, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?1 AND owner_epoch = ?2
               AND phase = 'pg_active' AND active_backend = 'postgres'
               AND desired_backend = 'postgres'",
        )
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("reopen PostgreSQL analytics admission")?;
        ensure!(
            result.rows_affected() == 1,
            "PostgreSQL active admission CAS was rejected"
        );
        self.admission.open("postgres", lease.owner_epoch);
        self.load().await
    }

    pub(crate) async fn begin_sqlite_rollback(
        &self,
        lease: &AnalyticsOwnerLease,
        expected_phase: AnalyticsControlPhase,
    ) -> Result<AnalyticsOwnerLease> {
        ensure!(
            expected_phase == AnalyticsControlPhase::PgActive,
            "SQLite rollback requires pg_active"
        );
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET desired_backend = 'sqlite', phase = 'rollback_building',
                 owner_epoch = owner_epoch + 1,
                 read_admission_open = 0, write_admission_open = 0,
                 updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?1 AND owner_epoch = ?2
               AND phase = 'pg_active' AND active_backend = 'postgres'",
        )
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("begin SQLite analytics rollback")?;
        ensure!(
            result.rows_affected() == 1,
            "SQLite rollback CAS was rejected"
        );
        self.admission.close();
        let state = self.load().await?;
        Ok(AnalyticsOwnerLease {
            owner_id: lease.owner_id.clone(),
            owner_epoch: state.owner_epoch,
            lease_expires_at: state
                .owner_lease_expires_at
                .context("SQLite rollback owner lease expiry is missing")?,
        })
    }

    pub(crate) async fn open_rollback_write_admission(
        &self,
        lease: &AnalyticsOwnerLease,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET write_admission_open = 1, read_admission_open = 0,
                 updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?1 AND owner_epoch = ?2
               AND phase = 'rollback_building' AND desired_backend = 'sqlite'",
        )
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("open SQLite rollback write admission")?;
        ensure!(
            result.rows_affected() == 1,
            "SQLite rollback write admission CAS was rejected"
        );
        self.admission.open_write_only("sqlite", lease.owner_epoch);
        Ok(())
    }

    pub(crate) async fn prepare_sqlite_active(
        &self,
        lease: &AnalyticsOwnerLease,
        build_id: &str,
        certificate_id: &str,
        fence: AnalyticsSourceFence,
    ) -> Result<AnalyticsBackendControl> {
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET phase = 'sqlite_prepared', build_id = ?1, certificate_id = ?2,
                 fence_generation = ?3, fence_high_watermark_change_id = ?4,
                 fence_coverage_revision = ?5, read_admission_open = 0,
                 write_admission_open = 0, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?6 AND owner_epoch = ?7
               AND phase = 'rollback_building' AND desired_backend = 'sqlite'",
        )
        .bind(build_id)
        .bind(certificate_id)
        .bind(fence.generation)
        .bind(fence.through_change_id)
        .bind(fence.coverage_revision)
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("publish SQLite rollback preparation")?;
        ensure!(
            result.rows_affected() == 1,
            "SQLite rollback preparation CAS was rejected"
        );
        self.admission.close();
        self.load().await
    }

    pub(crate) async fn activate_sqlite(
        &self,
        lease: &AnalyticsOwnerLease,
    ) -> Result<AnalyticsBackendControl> {
        let result = sqlx::query(
            "UPDATE analytics_backend_control
             SET active_backend = 'sqlite', phase = 'sqlite_active',
                 read_admission_open = 1, write_admission_open = 1,
                 rollback_deadline_at = NULL, error_code = NULL, updated_at = datetime('now')
             WHERE id = 1 AND owner_id = ?1 AND owner_epoch = ?2
               AND phase = 'sqlite_prepared' AND desired_backend = 'sqlite'",
        )
        .bind(&lease.owner_id)
        .bind(lease.owner_epoch)
        .execute(&self.pool)
        .await
        .context("activate SQLite analytics backend")?;
        ensure!(
            result.rows_affected() == 1,
            "SQLite active CAS was rejected"
        );
        self.admission.open("sqlite", lease.owner_epoch);
        self.load().await
    }

    pub(crate) async fn assert_rollback_source(
        &self,
        build_id: &str,
        required_through_change_id: i64,
    ) -> Result<SourceSnapshot> {
        let state = self.load().await?;
        ensure!(
            state.phase == AnalyticsControlPhase::RollbackBuilding,
            "rollback source is not admitted in phase {}",
            state.phase.as_str()
        );
        let deadline = state
            .rollback_deadline_at
            .as_deref()
            .context("rollback guarantee deadline is unknown")?;
        let deadline = DateTime::parse_from_rfc3339(deadline)
            .context("rollback guarantee deadline is invalid")?;
        ensure!(
            deadline.with_timezone(&Utc) >= Utc::now(),
            "rollback guarantee window has expired; source proof is required"
        );
        assert_source_snapshot_usable(
            &self.pool,
            build_id,
            &state.source_instance_id,
            state
                .fence_generation
                .context("rollback source generation is unknown")?,
            required_through_change_id,
        )
        .await
    }

    /// Cleanup is a gate only. It returns a proof that no active transition/pin blocks a caller;
    /// it never deletes facts, Journal rows, terminal records, or source archives.
    pub(crate) async fn cleanup_allowed(&self) -> Result<bool> {
        let state = self.load().await?;
        if state.phase != AnalyticsControlPhase::SqliteActive
            && state.phase != AnalyticsControlPhase::PgActive
        {
            return Ok(false);
        }
        let active_pins: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM analytics_source_retention_pins
             WHERE state = 'active' AND julianday(expires_at) > julianday('now'))",
        )
        .fetch_one(&self.pool)
        .await
        .context("inspect analytics source retention pins")?;
        Ok(active_pins == 0)
    }
}

pub(crate) async fn ensure_analytics_backend_control_schema(
    pool: &Pool<Sqlite>,
    source_instance_id: &str,
    desired_backend: &str,
) -> Result<()> {
    ensure!(
        !source_instance_id.trim().is_empty(),
        "analytics source instance id must not be empty"
    );
    ensure!(
        matches!(desired_backend, "sqlite" | "postgres"),
        "unknown desired analytics backend"
    );
    sqlx::query(ANALYTICS_CONTROL_SCHEMA_SQL)
        .execute(pool)
        .await
        .context("ensure analytics backend control schema")?;
    sqlx::query(
        "INSERT INTO analytics_backend_control
            (id, source_instance_id, owner_epoch, desired_backend, active_backend, phase)
         VALUES (1, ?1, 1, ?2, 'sqlite', 'sqlite_active')
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(source_instance_id)
    .bind(desired_backend)
    .execute(pool)
    .await
    .context("seed analytics backend control")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn control() -> AnalyticsControl {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("control pool");
        let control = AnalyticsControl::with_source_instance(pool, "owner-a", "source-a");
        control
            .initialize("sqlite")
            .await
            .expect("initialize control");
        control
    }

    #[tokio::test]
    async fn sqlite_initialization_opens_only_the_sqlite_gate() {
        let control = control().await;
        let state = control.load().await.expect("load state");
        assert_eq!(state.phase, AnalyticsControlPhase::SqliteActive);
        assert!(
            control
                .admission()
                .try_read("sqlite", state.owner_epoch)
                .is_ok()
        );
        assert!(
            control
                .admission()
                .try_read("postgres", state.owner_epoch)
                .is_err()
        );
    }

    #[tokio::test]
    async fn closing_sqlite_admission_wakes_a_waiter_without_a_lost_notify() {
        let admission = AnalyticsAdmission::new();
        admission.open("sqlite", 7);
        let waiter_admission = admission.clone();
        let waiter = tokio::spawn(async move {
            waiter_admission.wait_until_sqlite_write_closed().await;
        });
        tokio::task::yield_now().await;
        admission.close();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("admission close wakes the waiter")
            .expect("admission waiter task");
        assert!(!admission.sqlite_write_open());
    }

    #[test]
    fn rollback_write_admission_keeps_reads_closed() {
        let admission = AnalyticsAdmission::new();
        admission.open_write_only("sqlite", 11);
        assert!(admission.try_read("sqlite", 11).is_err());
        let writer = admission
            .try_write("sqlite", 11)
            .expect("rollback writer is admitted");
        drop(writer);
        admission.close();
        assert!(!admission.sqlite_write_open());
    }

    #[tokio::test]
    async fn owner_cas_fences_stale_epochs_and_drains_existing_work() {
        let control = control().await;
        let initial = control.load().await.expect("load state");
        let reader = control
            .admission()
            .try_read("sqlite", initial.owner_epoch)
            .expect("reader admitted");
        let lease = control
            .claim_owner(initial.owner_epoch)
            .await
            .expect("claim owner");
        assert_eq!(lease.owner_epoch, initial.owner_epoch + 1);
        assert!(control.claim_owner(initial.owner_epoch).await.is_err());
        assert!(
            control
                .admission()
                .drain(Duration::from_millis(20))
                .await
                .is_err()
        );
        drop(reader);
        control
            .admission()
            .drain(Duration::from_secs(1))
            .await
            .expect("drain");
        control.release_owner(&lease).await.expect("release owner");
    }

    #[tokio::test]
    async fn transition_and_pg_publication_keep_the_epoch_and_set_bounded_rollback() {
        let control = control().await;
        sqlx::query(
            "UPDATE analytics_backend_control SET desired_backend = 'postgres' WHERE id = 1",
        )
        .execute(&control.pool)
        .await
        .expect("select postgres backend");
        let state = control.load().await.expect("load state");
        let lease = control
            .claim_owner(state.owner_epoch)
            .await
            .expect("claim owner");
        control
            .transition(
                &lease,
                AnalyticsControlPhase::SqliteActive,
                AnalyticsControlPhase::PgBuilding,
            )
            .await
            .expect("building");
        control
            .transition(
                &lease,
                AnalyticsControlPhase::PgBuilding,
                AnalyticsControlPhase::PgPrepared,
            )
            .await
            .expect("prepared");
        control
            .transition(
                &lease,
                AnalyticsControlPhase::PgPrepared,
                AnalyticsControlPhase::SwitchPending,
            )
            .await
            .expect("switch pending");
        let active = control
            .publish_pg_active(
                &lease,
                "build-1",
                "cert-1",
                AnalyticsSourceFence {
                    generation: 1,
                    through_change_id: 12,
                    coverage_revision: 13,
                },
            )
            .await
            .expect("pg active");
        assert_eq!(active.phase, AnalyticsControlPhase::PgActive);
        assert!(active.rollback_deadline_at.is_some());
        assert!(
            control
                .admission()
                .try_write("postgres", lease.owner_epoch)
                .is_ok()
        );
    }

    #[test]
    fn transitions_do_not_skip_the_drain_or_publication_fence() {
        assert!(
            !AnalyticsControlPhase::SqliteActive.can_transition_to(AnalyticsControlPhase::PgActive)
        );
        assert!(
            !AnalyticsControlPhase::PgActive.can_transition_to(AnalyticsControlPhase::SqliteActive)
        );
        assert!(
            AnalyticsControlPhase::SwitchPending.can_transition_to(AnalyticsControlPhase::PgActive)
        );
    }
}
