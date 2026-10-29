use super::config::StorageConfig;
use super::core::{BackendKind, MigrationState, MigrationStatus, StorageModule};
use super::migration_capacity::MigrationCapacitySample;
use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify, watch},
    time::Instant,
};

const MIGRATION_ID: &str = "storage-migration";
const INSTALLATION_ID: &str = "local";
const SOURCE_IDENTITY: &str = "sqlite:local";
const STORAGE_MIGRATION_RUNTIME_TABLE: &str = "storage_migration_runtime";
const STORAGE_MIGRATION_PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

const MIGRATION_HELD_TASK_KEYS: &[&str] = &[
    "retention_archive",
    "pool_upstream_attempt_archive",
    "forward_proxy_attempt_archive",
    "invocation_detail_prune",
    "quota_snapshot_compaction",
    "model_route_expiry",
    "task_run_history_cleanup",
    "invocation_identity_cleanup",
    "raw_orphan_sweep",
    "raw_compression",
    "archive_upstream_activity_manifest",
    "materialize_historical_rollups",
    "verify_archive_storage",
    "prune_archive_batches",
    "prune_legacy_archive_batches",
    "prepare_performance_indexes",
    "startup_hourly_rollup_bootstrap",
    "startup_backfill",
    "prompt_cache_materialization",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationState {
    #[default]
    Pending,
    Preparing,
    Copying,
    CatchingUp,
    Verifying,
    Ready,
    Switching,
    Postgres,
    Paused,
    Blocked,
    Recovering,
    Failed,
}

impl From<BackendKind> for StorageMigrationBackend {
    fn from(value: BackendKind) -> Self {
        match value {
            BackendKind::Sqlite => Self::Sqlite,
            BackendKind::Postgres => Self::Postgres,
        }
    }
}

impl From<MigrationState> for StorageMigrationState {
    fn from(value: MigrationState) -> Self {
        match value {
            MigrationState::Pending => Self::Pending,
            MigrationState::Preparing => Self::Preparing,
            MigrationState::Copying => Self::Copying,
            MigrationState::CatchingUp => Self::CatchingUp,
            MigrationState::Verifying => Self::Verifying,
            MigrationState::Ready => Self::Ready,
            MigrationState::Switching => Self::Switching,
            MigrationState::Postgres => Self::Postgres,
            MigrationState::Paused => Self::Paused,
            MigrationState::Blocked => Self::Blocked,
            MigrationState::Recovering => Self::Recovering,
            MigrationState::Failed => Self::Failed,
        }
    }
}

impl From<StorageMigrationState> for MigrationState {
    fn from(value: StorageMigrationState) -> Self {
        match value {
            StorageMigrationState::Pending => Self::Pending,
            StorageMigrationState::Preparing => Self::Preparing,
            StorageMigrationState::Copying => Self::Copying,
            StorageMigrationState::CatchingUp => Self::CatchingUp,
            StorageMigrationState::Verifying => Self::Verifying,
            StorageMigrationState::Ready => Self::Ready,
            StorageMigrationState::Switching => Self::Switching,
            StorageMigrationState::Postgres => Self::Postgres,
            StorageMigrationState::Paused => Self::Paused,
            StorageMigrationState::Blocked => Self::Blocked,
            StorageMigrationState::Recovering => Self::Recovering,
            StorageMigrationState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationEndpoint {
    pub(crate) backend: StorageMigrationBackend,
    pub(crate) configured: bool,
    pub(crate) identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationAuthority {
    pub(crate) backend: StorageMigrationBackend,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationDrainState {
    NotStarted,
    Draining,
    Drained,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationHeartbeat {
    pub(crate) at: String,
    pub(crate) healthy: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationIndexPreparation {
    pub(crate) prepared: Option<u64>,
    pub(crate) total: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationProgress {
    pub(crate) committed_rows: Option<u64>,
    pub(crate) committed_bytes: Option<u64>,
    pub(crate) source_rows: Option<u64>,
    pub(crate) source_bytes: Option<u64>,
    pub(crate) scan_cursor: Option<String>,
    pub(crate) captured_through: Option<i64>,
    pub(crate) applied_through: Option<i64>,
    pub(crate) pending_keys: Option<u64>,
    pub(crate) oldest_change_age_ms: Option<u64>,
    pub(crate) replay_throughput_per_sec: Option<f64>,
    pub(crate) incoming_rate_per_sec: Option<f64>,
    pub(crate) converged: Option<bool>,
    pub(crate) eta_ms: Option<u64>,
    pub(crate) phase: StorageMigrationState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationTableProgress {
    pub(crate) table_name: String,
    pub(crate) state: StorageMigrationState,
    pub(crate) committed_rows: Option<u64>,
    pub(crate) committed_bytes: Option<u64>,
    pub(crate) source_rows: Option<u64>,
    pub(crate) source_bytes: Option<u64>,
    pub(crate) scan_cursor: Option<String>,
    pub(crate) scan_bound: Option<u64>,
    pub(crate) index_preparation: StorageMigrationIndexPreparation,
    pub(crate) captured_through: Option<i64>,
    pub(crate) applied_through: Option<i64>,
    pub(crate) dirty_chunks: Option<u64>,
    pub(crate) verified_chunks: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationBacklog {
    pub(crate) source_p1: Option<u64>,
    pub(crate) pending_keys: Option<u64>,
    pub(crate) oldest_change_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationVerification {
    pub(crate) dirty_chunks: Option<u64>,
    pub(crate) verified_chunks: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationTaskHolds {
    pub(crate) held: bool,
    pub(crate) preserved_task_keys: Vec<String>,
    pub(crate) reason: Option<String>,
    pub(crate) drain_state: StorageMigrationDrainState,
    pub(crate) last_safe_checkpoint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationPressure {
    pub(crate) source_backlog: Option<u64>,
    pub(crate) lock_wait_ms: Option<u64>,
    pub(crate) copy_throughput_per_sec: Option<f64>,
    pub(crate) incoming_change_rate_per_sec: Option<f64>,
    pub(crate) disk_headroom_bytes: Option<u64>,
    pub(crate) wal_headroom_bytes: Option<u64>,
    pub(crate) freshness_age_ms: Option<u64>,
    pub(crate) non_convergent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationSwitch {
    pub(crate) budget_ms: u64,
    pub(crate) attempt_count: u64,
    pub(crate) last_attempt_at: Option<String>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) result: Option<String>,
    pub(crate) failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationModuleProgress {
    pub(crate) module: String,
    pub(crate) state: StorageMigrationState,
    pub(crate) dependency_ready: Option<bool>,
    pub(crate) unapplied_changes: Option<u64>,
    pub(crate) final_proof_coverage: Option<f64>,
    pub(crate) task_hold: Option<String>,
    pub(crate) drain_state: StorageMigrationDrainState,
    pub(crate) switch_attempt_count: u64,
    pub(crate) switch_duration_ms: Option<u64>,
    pub(crate) switch_result: Option<String>,
    pub(crate) failure_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationRecovery {
    pub(crate) required: bool,
    pub(crate) reason: Option<String>,
    pub(crate) authority_backend: Option<StorageMigrationBackend>,
    pub(crate) authority_generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationControls {
    pub(crate) permission: String,
    pub(crate) can_start: bool,
    pub(crate) can_pause: bool,
    pub(crate) can_resume: bool,
    pub(crate) last_action: Option<String>,
    pub(crate) last_action_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationResponse {
    pub(crate) revision: u64,
    pub(crate) migration_id: Option<String>,
    pub(crate) active: bool,
    pub(crate) module: String,
    pub(crate) state: StorageMigrationState,
    pub(crate) source: StorageMigrationEndpoint,
    pub(crate) target: StorageMigrationEndpoint,
    pub(crate) authority: StorageMigrationAuthority,
    pub(crate) queued_modules: Vec<String>,
    pub(crate) completed_modules: Vec<String>,
    pub(crate) heartbeat: StorageMigrationHeartbeat,
    pub(crate) progress: StorageMigrationProgress,
    pub(crate) tables: Vec<StorageMigrationTableProgress>,
    pub(crate) modules: Vec<StorageMigrationModuleProgress>,
    pub(crate) backlog: StorageMigrationBacklog,
    pub(crate) verification: StorageMigrationVerification,
    pub(crate) task_holds: StorageMigrationTaskHolds,
    pub(crate) pressure: StorageMigrationPressure,
    pub(crate) switch: StorageMigrationSwitch,
    pub(crate) recovery: StorageMigrationRecovery,
    pub(crate) controls: StorageMigrationControls,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StorageMigrationControlAction {
    Start,
    Pause,
    Resume,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationControlRequest {
    pub(crate) paused: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StorageMigrationEvent {
    pub(crate) kind: &'static str,
    pub(crate) revision: u64,
    pub(crate) status: StorageMigrationResponse,
}

#[derive(Debug, Clone)]
struct StorageMigrationRuntimeState {
    status: MigrationStatus,
    response: StorageMigrationResponse,
    target_configured: bool,
    last_action: Option<String>,
    last_action_at: Option<String>,
    last_progress_published_at: Option<Instant>,
    progress_flush_deadline: Option<Instant>,
    progress_dirty: bool,
}

#[derive(Debug)]
struct ProgressFlushCoordinator {
    notify: Arc<Notify>,
    worker_started: AtomicBool,
}

#[derive(Debug)]
pub(crate) struct StorageMigrationRuntime {
    state: Arc<Mutex<StorageMigrationRuntimeState>>,
    persistence: Arc<Mutex<()>>,
    events: watch::Sender<StorageMigrationEvent>,
    event_receiver: watch::Receiver<StorageMigrationEvent>,
    pool: Option<Pool<Sqlite>>,
    target_identity: String,
    progress_flush: Arc<ProgressFlushCoordinator>,
    #[cfg(test)]
    progress_publish_count: Arc<AtomicUsize>,
    #[cfg(test)]
    progress_persisting: Arc<AtomicBool>,
}

pub(crate) async fn ensure_persistence_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {STORAGE_MIGRATION_RUNTIME_TABLE} (\
            migration_id TEXT PRIMARY KEY,\
            status_json TEXT NOT NULL,\
            response_json TEXT NOT NULL,\
            last_action TEXT,\
            last_action_at TEXT,\
            updated_at TEXT NOT NULL\
        )"
    ))
    .execute(pool)
    .await
    .map_err(|error| anyhow::anyhow!("failed to ensure storage migration persistence: {error}"))?;
    Ok(())
}

pub(crate) fn authorize_control_request(
    headers: &axum::http::HeaderMap,
    config: &StorageConfig,
) -> std::result::Result<(), (axum::http::StatusCode, String)> {
    if !crate::is_same_origin_settings_write(headers) {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "cross-origin storage migration controls are forbidden".to_string(),
        ));
    }
    let Some(expected) = config.migration_admin_token.as_deref() else {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "storage migration admin control is not configured".to_string(),
        ));
    };
    let Some(provided) = crate::bearer_token_from_headers(headers) else {
        return Err((
            axum::http::StatusCode::UNAUTHORIZED,
            "missing storage migration admin token".to_string(),
        ));
    };
    if !storage_migration_admin_token_matches(provided, expected) {
        return Err((
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid storage migration admin token".to_string(),
        ));
    }
    Ok(())
}

fn storage_migration_admin_token_matches(provided: &str, expected: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let Ok(mut expected_mac) = Hmac::<Sha256>::new_from_slice(expected.as_bytes()) else {
        return false;
    };
    expected_mac.update(b"cvm-storage-migration-admin-token");
    let signature = expected_mac.finalize().into_bytes();
    let Ok(mut provided_mac) = Hmac::<Sha256>::new_from_slice(provided.as_bytes()) else {
        return false;
    };
    provided_mac.update(b"cvm-storage-migration-admin-token");
    provided_mac.verify_slice(&signature).is_ok()
}

impl Default for StorageMigrationRuntime {
    fn default() -> Self {
        Self::new(&StorageConfig::default())
    }
}

impl StorageMigrationRuntime {
    pub(crate) fn new(config: &StorageConfig) -> Self {
        Self::from_parts(config, None)
    }

    pub(crate) async fn load(config: &StorageConfig, pool: Pool<Sqlite>) -> Result<Self> {
        let runtime = Self::from_parts(config, Some(pool.clone()));
        let row = sqlx::query(&format!(
            "SELECT status_json, response_json, last_action, last_action_at \
             FROM {STORAGE_MIGRATION_RUNTIME_TABLE} WHERE migration_id = ?1"
        ))
        .bind(MIGRATION_ID)
        .fetch_optional(&pool)
        .await
        .map_err(|error| anyhow::anyhow!("failed to load storage migration status: {error}"))?;

        if let Some(row) = row {
            let status: MigrationStatus =
                serde_json::from_str(row.try_get::<String, _>("status_json")?.as_str()).map_err(
                    |error| anyhow::anyhow!("invalid persisted storage migration status: {error}"),
                )?;
            status.validate()?;
            let response: StorageMigrationResponse = serde_json::from_str(
                row.try_get::<String, _>("response_json")?.as_str(),
            )
            .map_err(|error| {
                anyhow::anyhow!("invalid persisted storage migration response: {error}")
            })?;
            let last_action = row.try_get::<Option<String>, _>("last_action")?;
            let last_action_at = row.try_get::<Option<String>, _>("last_action_at")?;
            let mut state = StorageMigrationRuntimeState {
                status,
                response,
                target_configured: runtime.target_configured(config),
                last_action,
                last_action_at,
                last_progress_published_at: None,
                progress_flush_deadline: None,
                progress_dirty: false,
            };
            refresh_response(&mut state);
            let event = StorageMigrationEvent {
                kind: "snapshot",
                revision: state.response.revision,
                status: state.response.clone(),
            };
            runtime.events.send_replace(event);
            *runtime.state.lock().await = state;
        } else {
            let state = runtime.state.lock().await.clone();
            runtime.persist_state(&state).await?;
        }
        Ok(runtime)
    }

    fn from_parts(config: &StorageConfig, pool: Option<Pool<Sqlite>>) -> Self {
        let target_configured = config
            .postgres_database_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty());
        let target_identity = config.postgres_target_identity();
        let now = Utc::now();
        let status = MigrationStatus {
            migration_id: MIGRATION_ID.to_string(),
            installation_id: INSTALLATION_ID.to_string(),
            module: StorageModule::TrafficArchive,
            state: MigrationState::Pending,
            source_identity: SOURCE_IDENTITY.to_string(),
            target_identity: target_identity.to_string(),
            authority_generation: 1,
            capacity_bytes: 0,
            held_bytes: 0,
            hold_reason: None,
            updated_at_epoch_ms: now.timestamp_millis(),
        };
        status
            .validate()
            .expect("default storage migration status must be valid");
        let response = build_response(
            &status,
            target_configured,
            config.migration_switch_budget.as_millis() as u64,
            None,
            None,
        );
        let (events, event_receiver) = watch::channel(StorageMigrationEvent {
            kind: "snapshot",
            revision: response.revision,
            status: response.clone(),
        });
        Self {
            state: Arc::new(Mutex::new(StorageMigrationRuntimeState {
                status,
                response,
                target_configured,
                last_action: None,
                last_action_at: None,
                last_progress_published_at: None,
                progress_flush_deadline: None,
                progress_dirty: false,
            })),
            persistence: Arc::new(Mutex::new(())),
            events,
            event_receiver,
            pool,
            target_identity,
            progress_flush: Arc::new(ProgressFlushCoordinator {
                notify: Arc::new(Notify::new()),
                worker_started: AtomicBool::new(false),
            }),
            #[cfg(test)]
            progress_publish_count: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            progress_persisting: Arc::new(AtomicBool::new(false)),
        }
    }

    fn target_configured(&self, config: &StorageConfig) -> bool {
        config
            .postgres_database_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }

    async fn persist_state(&self, state: &StorageMigrationRuntimeState) -> Result<()> {
        persist_state_to_pool(self.pool.as_ref(), state).await
    }

    fn schedule_progress_flush(&self) {
        let notify = self.progress_flush.notify.clone();
        notify.notify_one();
        if self
            .progress_flush
            .worker_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let weak_state: Weak<Mutex<StorageMigrationRuntimeState>> = Arc::downgrade(&self.state);
        let events = self.events.clone();
        let pool = self.pool.clone();
        let persistence = self.persistence.clone();
        #[cfg(test)]
        let progress_publish_count = self.progress_publish_count.clone();
        #[cfg(test)]
        let progress_persisting = self.progress_persisting.clone();
        tokio::spawn(async move {
            while let Some(state) = weak_state.upgrade() {
                let deadline = state.lock().await.progress_flush_deadline;
                let Some(deadline) = deadline else {
                    tokio::select! {
                        _ = notify.notified() => {}
                        _ = tokio::time::sleep(STORAGE_MIGRATION_PROGRESS_INTERVAL) => {}
                    }
                    continue;
                };

                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => {}
                    _ = notify.notified() => continue,
                }

                let Some(state) = weak_state.upgrade() else {
                    break;
                };
                let snapshot = {
                    let mut state = state.lock().await;
                    if state.progress_flush_deadline != Some(deadline) {
                        continue;
                    }
                    if !state.progress_dirty {
                        state.progress_flush_deadline = None;
                        continue;
                    }
                    state.clone()
                };

                let persisted = persist_state_if_current(
                    &state,
                    persistence.as_ref(),
                    pool.as_ref(),
                    &snapshot,
                    #[cfg(test)]
                    progress_persisting.as_ref(),
                )
                .await
                .unwrap_or(false);

                let Some(state) = weak_state.upgrade() else {
                    break;
                };
                let mut state = state.lock().await;
                if state.response.revision != snapshot.response.revision
                    || state.progress_flush_deadline != Some(deadline)
                {
                    notify.notify_one();
                    continue;
                }
                if persisted {
                    state.progress_dirty = false;
                    state.progress_flush_deadline = None;
                    state.last_progress_published_at = Some(Instant::now());
                    let response = state.response.clone();
                    #[cfg(test)]
                    progress_publish_count.fetch_add(1, Ordering::Relaxed);
                    events.send_replace(StorageMigrationEvent {
                        kind: "progress",
                        revision: response.revision,
                        status: response,
                    });
                } else if state.progress_dirty {
                    state.progress_flush_deadline =
                        Some(Instant::now() + STORAGE_MIGRATION_PROGRESS_INTERVAL);
                }
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn progress_publish_count_for_test(&self) -> usize {
        self.progress_publish_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn progress_persistence_in_flight_for_test(&self) -> bool {
        self.progress_persisting.load(Ordering::Acquire)
    }

    pub(crate) async fn snapshot(&self) -> StorageMigrationResponse {
        self.state.lock().await.response.clone()
    }

    pub(crate) async fn validate_runtime_identity(&self) -> Result<()> {
        let state = self.state.lock().await;
        if state.status.migration_id != MIGRATION_ID
            || state.status.installation_id != INSTALLATION_ID
            || state.status.module != StorageModule::TrafficArchive
            || state.status.source_identity != SOURCE_IDENTITY
            || state.status.target_identity != self.target_identity
        {
            bail!("persisted storage migration status belongs to another runtime identity");
        }
        Ok(())
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<StorageMigrationEvent> {
        self.event_receiver.clone()
    }

    pub(crate) async fn migration_hold_reason(&self, task_key: &str) -> Option<&'static str> {
        let state = self.state.lock().await;
        (migration_hold_active(state.status.state) && is_migration_held_task(task_key))
            .then_some("migration_hold")
    }

    pub(crate) async fn held_task_keys(&self) -> Vec<String> {
        let state = self.state.lock().await;
        if !migration_hold_active(state.status.state) {
            return Vec::new();
        }
        MIGRATION_HELD_TASK_KEYS
            .iter()
            .map(|key| (*key).to_string())
            .collect()
    }

    pub(crate) async fn apply_control(
        &self,
        request: StorageMigrationControlRequest,
    ) -> Result<StorageMigrationResponse> {
        let (previous, snapshot) = {
            let mut state = self.state.lock().await;
            let previous = state.clone();
            let now = Utc::now();
            let action = if request.paused {
                if !can_pause_state(state.status.state) {
                    bail!("pause control is unavailable in {:?}", state.status.state);
                }
                StorageMigrationControlAction::Pause
            } else if matches!(state.status.state, MigrationState::Pending) {
                if !state.target_configured {
                    bail!("PostgreSQL storage is not configured");
                }
                StorageMigrationControlAction::Start
            } else if can_resume_state(state.status.state) {
                StorageMigrationControlAction::Resume
            } else {
                bail!("resume control is unavailable in {:?}", state.status.state);
            };
            match action {
                StorageMigrationControlAction::Start => {
                    state
                        .status
                        .transition_to(MigrationState::Preparing, now.timestamp_millis())?;
                }
                StorageMigrationControlAction::Pause => {
                    state.status.hold_reason = Some("operator_pause".to_string());
                    state
                        .status
                        .transition_to(MigrationState::Paused, now.timestamp_millis())?;
                }
                StorageMigrationControlAction::Resume => {
                    if !matches!(
                        state.status.state,
                        MigrationState::Paused | MigrationState::Blocked
                    ) {
                        bail!("migration is not paused or blocked");
                    }
                    state.status.hold_reason = None;
                    state
                        .status
                        .transition_to(MigrationState::Preparing, now.timestamp_millis())?;
                }
            }

            let action = match action {
                StorageMigrationControlAction::Start => "start",
                StorageMigrationControlAction::Pause => "pause",
                StorageMigrationControlAction::Resume => "resume",
            };
            state.last_action = Some(action.to_string());
            state.last_action_at = Some(now.to_rfc3339());
            advance_revision(&mut state.response)?;
            refresh_response(&mut state);
            let had_pending_progress = state.progress_dirty;
            state.progress_dirty = false;
            state.progress_flush_deadline = None;
            if had_pending_progress {
                state.last_progress_published_at = Some(Instant::now());
            }
            (previous, state.clone())
        };

        match persist_state_if_current(
            &self.state,
            self.persistence.as_ref(),
            self.pool.as_ref(),
            &snapshot,
            #[cfg(test)]
            self.progress_persisting.as_ref(),
        )
        .await
        {
            Ok(true) => {
                let state = self.state.lock().await;
                if state.response.revision != snapshot.response.revision {
                    return Ok(state.response.clone());
                }
                let response = state.response.clone();
                self.progress_flush.notify.notify_one();
                self.events.send_replace(StorageMigrationEvent {
                    kind: "status",
                    revision: response.revision,
                    status: response.clone(),
                });
                Ok(response)
            }
            Ok(false) => Ok(self.snapshot().await),
            Err(error) => {
                let mut state = self.state.lock().await;
                if state.response.revision == snapshot.response.revision {
                    *state = previous;
                    self.progress_flush.notify.notify_one();
                }
                Err(error)
            }
        }
    }

    /// Publish worker-observed progress without deriving an ETA from a non-converging backlog.
    pub(crate) async fn publish_progress(
        &self,
        progress: StorageMigrationProgress,
    ) -> Result<StorageMigrationResponse> {
        self.publish_progress_inner(progress, false).await
    }

    /// Publish a progress boundary immediately, bypassing the normal coalescing window.
    pub(crate) async fn publish_progress_durable(
        &self,
        progress: StorageMigrationProgress,
    ) -> Result<StorageMigrationResponse> {
        self.publish_progress_inner(progress, true).await
    }

    pub(crate) async fn publish_capacity_observation(
        &self,
        sample: MigrationCapacitySample,
        hold_reason: Option<&str>,
    ) -> Result<StorageMigrationResponse> {
        let (previous, snapshot) = {
            let mut state = self.state.lock().await;
            let previous = state.clone();
            let now = Utc::now();
            let previous_status = state.status.clone();
            let previous_disk_headroom = state.response.pressure.disk_headroom_bytes;
            let previous_recovery_reason = state.response.recovery.reason.clone();
            let previous_hold = state.status.hold_reason.clone();
            state.status.capacity_bytes = sample.capture_limit_bytes.max(1);
            state.status.held_bytes = sample.capture_bytes.min(state.status.capacity_bytes);
            state.response.pressure.disk_headroom_bytes = sample.source_free_bytes;

            if let Some(reason) = hold_reason {
                if !matches!(state.status.state, MigrationState::Paused) {
                    state.status.hold_reason = Some(reason.to_string());
                    if state.status.state != MigrationState::Blocked {
                        state
                            .status
                            .transition_to(MigrationState::Blocked, now.timestamp_millis())?;
                    }
                    state.response.recovery.reason = Some(reason.to_string());
                }
            } else if state.status.state == MigrationState::Preparing
                && (previous_hold
                    .as_deref()
                    .is_some_and(is_capacity_hold_reason)
                    || previous_recovery_reason
                        .as_deref()
                        .is_some_and(is_capacity_hold_reason))
            {
                state.status.hold_reason = None;
                state.response.recovery.reason = Some("capacity_recovered".to_string());
            }

            if state.status == previous_status
                && state.response.pressure.disk_headroom_bytes == previous_disk_headroom
                && state.response.recovery.reason == previous_recovery_reason
            {
                return Ok(previous.response);
            }

            state.status.updated_at_epoch_ms =
                now.timestamp_millis().max(state.status.updated_at_epoch_ms);
            refresh_response(&mut state);
            advance_revision(&mut state.response)?;
            (previous, state.clone())
        };

        match persist_state_if_current(
            &self.state,
            self.persistence.as_ref(),
            self.pool.as_ref(),
            &snapshot,
            #[cfg(test)]
            self.progress_persisting.as_ref(),
        )
        .await
        {
            Ok(true) => {
                let state = self.state.lock().await;
                if state.response.revision != snapshot.response.revision {
                    return Ok(state.response.clone());
                }
                let response = state.response.clone();
                self.events.send_replace(StorageMigrationEvent {
                    kind: "capacity",
                    revision: response.revision,
                    status: response.clone(),
                });
                Ok(response)
            }
            Ok(false) => Ok(self.snapshot().await),
            Err(error) => {
                let mut state = self.state.lock().await;
                if state.response.revision == snapshot.response.revision {
                    *state = previous;
                }
                Err(error)
            }
        }
    }

    async fn publish_progress_inner(
        &self,
        mut progress: StorageMigrationProgress,
        persist_immediately: bool,
    ) -> Result<StorageMigrationResponse> {
        if progress.phase == StorageMigrationState::Postgres {
            bail!(
                "PostgreSQL authority status must be published after the durable authority manifest"
            );
        }
        let (previous, snapshot, monotonic_now) = {
            let mut state = self.state.lock().await;
            let previous = state.clone();
            if matches!(
                state.status.state,
                MigrationState::Paused | MigrationState::Blocked
            ) {
                bail!("migration progress is held while the migration is paused or blocked");
            }
            if let (Some(replay), Some(incoming)) = (
                progress.replay_throughput_per_sec,
                progress.incoming_rate_per_sec,
            ) && replay <= incoming
            {
                progress.converged = Some(false);
                progress.eta_ms = None;
            }
            let now = Utc::now();
            let next_state = MigrationState::from(progress.phase);
            if state.status.state != next_state
                && let Err(error) = state
                    .status
                    .transition_to(next_state, now.timestamp_millis())
            {
                *state = previous;
                return Err(error);
            }
            state.response.progress = progress;
            state.response.backlog.pending_keys = state.response.progress.pending_keys;
            state.response.backlog.oldest_change_age_ms =
                state.response.progress.oldest_change_age_ms;
            state.response.pressure.copy_throughput_per_sec =
                state.response.progress.replay_throughput_per_sec;
            state.response.pressure.incoming_change_rate_per_sec =
                state.response.progress.incoming_rate_per_sec;
            state.response.pressure.non_convergent =
                state.response.progress.converged == Some(false);
            state.status.updated_at_epoch_ms =
                now.timestamp_millis().max(state.status.updated_at_epoch_ms);
            refresh_response(&mut state);
            if let Err(error) = advance_revision(&mut state.response) {
                *state = previous;
                return Err(error);
            }

            let monotonic_now = Instant::now();
            let publish_now = persist_immediately
                || state.last_progress_published_at.is_none_or(|last| {
                    monotonic_now.duration_since(last) >= STORAGE_MIGRATION_PROGRESS_INTERVAL
                });
            if !publish_now {
                state.progress_dirty = true;
                state.progress_flush_deadline = state
                    .last_progress_published_at
                    .map(|last| last + STORAGE_MIGRATION_PROGRESS_INTERVAL);
                let response = state.response.clone();
                drop(state);
                self.schedule_progress_flush();
                return Ok(response);
            }

            (previous, state.clone(), monotonic_now)
        };

        match persist_state_if_current(
            &self.state,
            self.persistence.as_ref(),
            self.pool.as_ref(),
            &snapshot,
            #[cfg(test)]
            self.progress_persisting.as_ref(),
        )
        .await
        {
            Ok(true) => {
                let mut state = self.state.lock().await;
                if state.response.revision != snapshot.response.revision {
                    return Ok(state.response.clone());
                }
                state.last_progress_published_at = Some(monotonic_now);
                state.progress_dirty = false;
                state.progress_flush_deadline = None;
                let response = state.response.clone();
                #[cfg(test)]
                self.progress_publish_count.fetch_add(1, Ordering::Relaxed);
                self.events.send_replace(StorageMigrationEvent {
                    kind: "progress",
                    revision: response.revision,
                    status: response.clone(),
                });
                Ok(response)
            }
            Ok(false) => Ok(self.snapshot().await),
            Err(error) => {
                let mut state = self.state.lock().await;
                if state.response.revision == snapshot.response.revision {
                    *state = previous;
                }
                Err(error)
            }
        }
    }

    pub(crate) async fn publish_worker_error(
        &self,
        error: String,
    ) -> Result<StorageMigrationResponse> {
        let (previous, snapshot) = {
            let mut state = self.state.lock().await;
            let previous = state.clone();
            let now = Utc::now();
            state.response.verification.last_error = Some(error.clone());
            state.response.switch.failure_reason = Some(error);
            state.response.heartbeat = StorageMigrationHeartbeat {
                at: now.to_rfc3339(),
                healthy: false,
            };
            state.status.updated_at_epoch_ms =
                now.timestamp_millis().max(state.status.updated_at_epoch_ms);
            refresh_response(&mut state);
            state.response.heartbeat.healthy = false;
            state.response.heartbeat.at = now.to_rfc3339();
            advance_revision(&mut state.response)?;
            (previous, state.clone())
        };

        match persist_state_if_current(
            &self.state,
            self.persistence.as_ref(),
            self.pool.as_ref(),
            &snapshot,
            #[cfg(test)]
            self.progress_persisting.as_ref(),
        )
        .await
        {
            Ok(true) => {
                let state = self.state.lock().await;
                if state.response.revision != snapshot.response.revision {
                    return Ok(state.response.clone());
                }
                let response = state.response.clone();
                self.events.send_replace(StorageMigrationEvent {
                    kind: "error",
                    revision: response.revision,
                    status: response.clone(),
                });
                Ok(response)
            }
            Ok(false) => Ok(self.snapshot().await),
            Err(error) => {
                let mut state = self.state.lock().await;
                if state.response.revision == snapshot.response.revision {
                    *state = previous;
                }
                Err(error)
            }
        }
    }

    pub(crate) async fn recover_postgres(
        &self,
        authority_generation: u64,
        source_watermark: i64,
    ) -> Result<StorageMigrationResponse> {
        self.publish_postgres_state(
            authority_generation,
            source_watermark,
            "recovered",
            "recovered",
        )
        .await
    }

    pub(crate) async fn publish_postgres_authority(
        &self,
        authority_generation: u64,
        source_watermark: i64,
    ) -> Result<StorageMigrationResponse> {
        self.publish_postgres_state(
            authority_generation,
            source_watermark,
            "published",
            "status",
        )
        .await
    }

    async fn publish_postgres_state(
        &self,
        authority_generation: u64,
        source_watermark: i64,
        switch_result: &'static str,
        event_kind: &'static str,
    ) -> Result<StorageMigrationResponse> {
        if authority_generation == 0 || source_watermark < 0 {
            bail!("PostgreSQL authority has invalid generation or watermark");
        }
        let (previous, snapshot) = {
            let mut state = self.state.lock().await;
            let previous = state.clone();
            if authority_generation < state.status.authority_generation {
                bail!(
                    "PostgreSQL authority generation regresses durable migration status from {} to {}",
                    state.status.authority_generation,
                    authority_generation
                );
            }
            let now = Utc::now();
            if state.status.state != MigrationState::Postgres {
                state
                    .status
                    .transition_to(MigrationState::Postgres, now.timestamp_millis())?;
            }
            state.status.authority_generation = authority_generation;
            state.response.progress.captured_through = Some(source_watermark);
            state.response.progress.applied_through = Some(source_watermark);
            state.response.progress.converged = Some(true);
            state.response.progress.phase = StorageMigrationState::Postgres;
            state.response.switch.result = Some(switch_result.to_string());
            state.response.switch.failure_reason = None;
            state.response.verification.last_error = None;
            state.response.heartbeat = StorageMigrationHeartbeat {
                at: now.to_rfc3339(),
                healthy: true,
            };
            refresh_response(&mut state);
            advance_revision(&mut state.response)?;
            (previous, state.clone())
        };

        match persist_state_if_current(
            &self.state,
            self.persistence.as_ref(),
            self.pool.as_ref(),
            &snapshot,
            #[cfg(test)]
            self.progress_persisting.as_ref(),
        )
        .await
        {
            Ok(true) => {
                let state = self.state.lock().await;
                if state.response.revision != snapshot.response.revision {
                    return Ok(state.response.clone());
                }
                let response = state.response.clone();
                self.events.send_replace(StorageMigrationEvent {
                    kind: event_kind,
                    revision: response.revision,
                    status: response.clone(),
                });
                Ok(response)
            }
            Ok(false) => Ok(self.snapshot().await),
            Err(error) => {
                let mut state = self.state.lock().await;
                if state.response.revision == snapshot.response.revision {
                    *state = previous;
                }
                Err(error)
            }
        }
    }
}

async fn persist_state_if_current(
    state: &Arc<Mutex<StorageMigrationRuntimeState>>,
    persistence: &Mutex<()>,
    pool: Option<&Pool<Sqlite>>,
    snapshot: &StorageMigrationRuntimeState,
    #[cfg(test)] progress_persisting: &AtomicBool,
) -> Result<bool> {
    let _persistence_guard = persistence.lock().await;
    let is_current = {
        let state = state.lock().await;
        state.response.revision == snapshot.response.revision
    };
    if !is_current {
        return Ok(false);
    }
    #[cfg(test)]
    progress_persisting.store(true, Ordering::Release);
    let result = persist_state_to_pool(pool, snapshot).await;
    #[cfg(test)]
    progress_persisting.store(false, Ordering::Release);
    result?;
    Ok(true)
}

async fn persist_state_to_pool(
    pool: Option<&Pool<Sqlite>>,
    state: &StorageMigrationRuntimeState,
) -> Result<()> {
    let Some(pool) = pool else {
        return Ok(());
    };
    let status_json = serde_json::to_string(&state.status)
        .map_err(|error| anyhow::anyhow!("failed to encode storage migration status: {error}"))?;
    let response_json = serde_json::to_string(&state.response)
        .map_err(|error| anyhow::anyhow!("failed to encode storage migration response: {error}"))?;
    sqlx::query(&format!(
        "INSERT INTO {STORAGE_MIGRATION_RUNTIME_TABLE} \
            (migration_id, status_json, response_json, last_action, last_action_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
         ON CONFLICT(migration_id) DO UPDATE SET \
            status_json = excluded.status_json, \
            response_json = excluded.response_json, \
            last_action = excluded.last_action, \
            last_action_at = excluded.last_action_at, \
            updated_at = excluded.updated_at"
    ))
    .bind(MIGRATION_ID)
    .bind(status_json)
    .bind(response_json)
    .bind(&state.last_action)
    .bind(&state.last_action_at)
    .execute(pool)
    .await
    .map_err(|error| anyhow::anyhow!("failed to persist storage migration status: {error}"))?;
    Ok(())
}

fn build_response(
    status: &MigrationStatus,
    target_configured: bool,
    switch_budget_ms: u64,
    last_action: Option<String>,
    last_action_at: Option<String>,
) -> StorageMigrationResponse {
    let state = StorageMigrationState::from(status.state);
    let authority_backend = authority_backend(status.state);
    let queued_modules = if target_configured && status.state == MigrationState::Pending {
        vec![module_name(status.module).to_string()]
    } else {
        Vec::new()
    };
    let task_hold_active = migration_hold_active(status.state);
    let task_hold_reason = if task_hold_active {
        Some(
            status
                .hold_reason
                .clone()
                .unwrap_or_else(|| "migration_hold".to_string()),
        )
    } else {
        status.hold_reason.clone()
    };
    StorageMigrationResponse {
        revision: 0,
        migration_id: Some(status.migration_id.clone()),
        active: !matches!(
            status.state,
            MigrationState::Pending | MigrationState::Postgres
        ),
        module: module_name(status.module).to_string(),
        state,
        source: StorageMigrationEndpoint {
            backend: BackendKind::Sqlite.into(),
            configured: true,
            identity: status.source_identity.clone(),
        },
        target: StorageMigrationEndpoint {
            backend: BackendKind::Postgres.into(),
            configured: target_configured,
            identity: status.target_identity.clone(),
        },
        authority: StorageMigrationAuthority {
            backend: authority_backend,
            generation: status.authority_generation,
        },
        queued_modules,
        completed_modules: if status.state == MigrationState::Postgres {
            vec![module_name(status.module).to_string()]
        } else {
            Vec::new()
        },
        heartbeat: StorageMigrationHeartbeat {
            at: epoch_ms_to_rfc3339(status.updated_at_epoch_ms),
            healthy: true,
        },
        progress: StorageMigrationProgress {
            phase: state,
            ..StorageMigrationProgress::default()
        },
        tables: Vec::new(),
        modules: vec![StorageMigrationModuleProgress {
            module: module_name(status.module).to_string(),
            state,
            dependency_ready: None,
            unapplied_changes: None,
            final_proof_coverage: None,
            task_hold: task_hold_reason.clone(),
            drain_state: StorageMigrationDrainState::Unknown,
            switch_attempt_count: 0,
            switch_duration_ms: None,
            switch_result: None,
            failure_reason: None,
        }],
        backlog: StorageMigrationBacklog::default(),
        verification: StorageMigrationVerification::default(),
        task_holds: StorageMigrationTaskHolds {
            held: task_hold_active,
            preserved_task_keys: Vec::new(),
            reason: task_hold_reason,
            drain_state: StorageMigrationDrainState::Unknown,
            last_safe_checkpoint: None,
        },
        pressure: StorageMigrationPressure::default(),
        switch: StorageMigrationSwitch {
            budget_ms: switch_budget_ms,
            attempt_count: 0,
            last_attempt_at: None,
            duration_ms: None,
            result: None,
            failure_reason: None,
        },
        recovery: StorageMigrationRecovery {
            required: matches!(status.state, MigrationState::Recovering),
            reason: None,
            authority_backend: Some(authority_backend),
            authority_generation: Some(status.authority_generation),
        },
        controls: StorageMigrationControls {
            permission: "admin".to_string(),
            can_start: target_configured && matches!(status.state, MigrationState::Pending),
            can_pause: matches!(
                status.state,
                MigrationState::Preparing
                    | MigrationState::Copying
                    | MigrationState::CatchingUp
                    | MigrationState::Verifying
                    | MigrationState::Ready
            ),
            can_resume: matches!(
                status.state,
                MigrationState::Paused | MigrationState::Blocked
            ),
            last_action,
            last_action_at,
        },
        updated_at: epoch_ms_to_rfc3339(status.updated_at_epoch_ms),
    }
}

fn refresh_response(state: &mut StorageMigrationRuntimeState) {
    let revision = state.response.revision;
    let progress = state.response.progress.clone();
    let tables = state.response.tables.clone();
    let modules = state.response.modules.clone();
    let backlog = state.response.backlog.clone();
    let verification = state.response.verification.clone();
    let pressure = state.response.pressure.clone();
    let switch = state.response.switch.clone();
    let recovery_reason = state.response.recovery.reason.clone();
    let mut response = build_response(
        &state.status,
        state.target_configured,
        switch.budget_ms,
        state.last_action.clone(),
        state.last_action_at.clone(),
    );
    response.revision = revision;
    response.progress = progress;
    response.progress.phase = response.state;
    response.tables = tables;
    if !modules.is_empty() {
        response.modules = modules;
    }
    response.backlog = backlog;
    response.verification = verification;
    response.pressure = pressure;
    response.switch = switch;
    response.recovery.reason = recovery_reason;
    response.heartbeat.at = response.updated_at.clone();
    let active_module = response.module.clone();
    if let Some(module) = response
        .modules
        .iter_mut()
        .find(|module| module.module == active_module)
    {
        module.state = response.state;
        module.task_hold = response.task_holds.reason.clone();
        module.drain_state = response.task_holds.drain_state;
        module.switch_attempt_count = response.switch.attempt_count;
        module.switch_duration_ms = response.switch.duration_ms;
        module.switch_result = response.switch.result.clone();
        module.failure_reason = response.switch.failure_reason.clone();
    }
    state.response = response;
}

fn advance_revision(response: &mut StorageMigrationResponse) -> Result<()> {
    response.revision = response
        .revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("storage migration revision overflowed"))?;
    Ok(())
}

fn can_pause_state(state: MigrationState) -> bool {
    matches!(
        state,
        MigrationState::Preparing
            | MigrationState::Copying
            | MigrationState::CatchingUp
            | MigrationState::Verifying
            | MigrationState::Ready
    )
}

fn can_resume_state(state: MigrationState) -> bool {
    matches!(state, MigrationState::Paused | MigrationState::Blocked)
}

fn is_capacity_hold_reason(reason: &str) -> bool {
    matches!(
        reason,
        "capacity_source_unavailable"
            | "capacity_source_free"
            | "capacity_capture"
            | "capture_continuity_lost"
    )
}

fn migration_hold_active(state: MigrationState) -> bool {
    matches!(
        state,
        MigrationState::Preparing
            | MigrationState::Copying
            | MigrationState::CatchingUp
            | MigrationState::Verifying
            | MigrationState::Ready
            | MigrationState::Switching
            | MigrationState::Recovering
    )
}

fn is_migration_held_task(task_key: &str) -> bool {
    task_key.starts_with("startup_backfill.")
        || MIGRATION_HELD_TASK_KEYS.contains(&task_key)
        || task_key == "startup_backfill"
        || task_key == "prompt_cache_materialization"
}

fn authority_backend(state: MigrationState) -> StorageMigrationBackend {
    match state {
        MigrationState::Postgres => BackendKind::Postgres.into(),
        _ => BackendKind::Sqlite.into(),
    }
}

fn module_name(module: StorageModule) -> &'static str {
    match module {
        StorageModule::TrafficArchive => "traffic_archive",
        StorageModule::Analytics => "analytics",
        StorageModule::AccountRouting => "account_routing",
        StorageModule::TaskOperations => "task_operations",
        StorageModule::Performance => "performance",
    }
}

fn epoch_ms_to_rfc3339(epoch_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(epoch_ms)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}
