use super::config::StorageConfig;
use super::core::{BackendKind, MigrationState, MigrationStatus, StorageModule};
use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};

const MIGRATION_ID: &str = "storage-migration";
const INSTALLATION_ID: &str = "local";
const SOURCE_IDENTITY: &str = "sqlite:local";
const TARGET_IDENTITY_CONFIGURED: &str = "postgres:configured";
const TARGET_IDENTITY_UNCONFIGURED: &str = "postgres:unconfigured";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationEndpoint {
    pub(crate) backend: StorageMigrationBackend,
    pub(crate) configured: bool,
    pub(crate) identity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationAuthority {
    pub(crate) backend: StorageMigrationBackend,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationDrainState {
    NotStarted,
    Draining,
    Drained,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationHeartbeat {
    pub(crate) at: String,
    pub(crate) healthy: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationIndexPreparation {
    pub(crate) prepared: Option<u64>,
    pub(crate) total: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
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

#[derive(Debug, Clone, Default, Serialize)]
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

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationBacklog {
    pub(crate) source_p1: Option<u64>,
    pub(crate) pending_keys: Option<u64>,
    pub(crate) oldest_change_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationVerification {
    pub(crate) dirty_chunks: Option<u64>,
    pub(crate) verified_chunks: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationTaskHolds {
    pub(crate) held: bool,
    pub(crate) preserved_task_keys: Vec<String>,
    pub(crate) reason: Option<String>,
    pub(crate) drain_state: StorageMigrationDrainState,
    pub(crate) last_safe_checkpoint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationSwitch {
    pub(crate) budget_ms: u64,
    pub(crate) attempt_count: u64,
    pub(crate) last_attempt_at: Option<String>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) result: Option<String>,
    pub(crate) failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationRecovery {
    pub(crate) required: bool,
    pub(crate) reason: Option<String>,
    pub(crate) authority_backend: Option<StorageMigrationBackend>,
    pub(crate) authority_generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationControls {
    pub(crate) permission: &'static str,
    pub(crate) can_start: bool,
    pub(crate) can_pause: bool,
    pub(crate) can_resume: bool,
    pub(crate) last_action: Option<String>,
    pub(crate) last_action_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationResponse {
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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageMigrationControlAction {
    Start,
    Pause,
    Resume,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationControlRequest {
    pub(crate) action: StorageMigrationControlAction,
}

#[derive(Debug, Clone)]
pub(crate) struct StorageMigrationEvent {
    pub(crate) kind: &'static str,
    pub(crate) status: StorageMigrationResponse,
}

#[derive(Debug, Clone)]
struct StorageMigrationRuntimeState {
    status: MigrationStatus,
    response: StorageMigrationResponse,
    target_configured: bool,
    last_action: Option<String>,
    last_action_at: Option<String>,
}

#[derive(Debug)]
pub(crate) struct StorageMigrationRuntime {
    state: Mutex<StorageMigrationRuntimeState>,
    events: broadcast::Sender<StorageMigrationEvent>,
}

impl Default for StorageMigrationRuntime {
    fn default() -> Self {
        Self::new(&StorageConfig::default())
    }
}

impl StorageMigrationRuntime {
    pub(crate) fn new(config: &StorageConfig) -> Self {
        let target_configured = config
            .postgres_database_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty());
        let target_identity = if target_configured {
            TARGET_IDENTITY_CONFIGURED
        } else {
            TARGET_IDENTITY_UNCONFIGURED
        };
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
        let (events, _) = broadcast::channel(32);
        Self {
            state: Mutex::new(StorageMigrationRuntimeState {
                status,
                response,
                target_configured,
                last_action: None,
                last_action_at: None,
            }),
            events,
        }
    }

    pub(crate) async fn snapshot(&self) -> StorageMigrationResponse {
        self.state.lock().await.response.clone()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<StorageMigrationEvent> {
        self.events.subscribe()
    }

    pub(crate) async fn apply_control(
        &self,
        request: StorageMigrationControlRequest,
    ) -> Result<StorageMigrationResponse> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        match request.action {
            StorageMigrationControlAction::Start => {
                if !state.target_configured {
                    bail!("PostgreSQL storage is not configured");
                }
                state
                    .status
                    .transition_to(MigrationState::Preparing, now.timestamp_millis())?;
            }
            StorageMigrationControlAction::Pause => {
                if state.status.state.is_terminal() {
                    bail!("terminal migration state cannot be paused");
                }
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

        let action = match request.action {
            StorageMigrationControlAction::Start => "start",
            StorageMigrationControlAction::Pause => "pause",
            StorageMigrationControlAction::Resume => "resume",
        };
        state.last_action = Some(action.to_string());
        state.last_action_at = Some(now.to_rfc3339());
        refresh_response(&mut state);
        let response = state.response.clone();
        drop(state);
        let _ = self.events.send(StorageMigrationEvent {
            kind: "status",
            status: response.clone(),
        });
        Ok(response)
    }

    /// Publish worker-observed progress without deriving an ETA from a non-converging backlog.
    pub(crate) async fn publish_progress(
        &self,
        mut progress: StorageMigrationProgress,
    ) -> Result<StorageMigrationResponse> {
        let mut state = self.state.lock().await;
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
        if state.status.state != next_state {
            state
                .status
                .transition_to(next_state, now.timestamp_millis())?;
        }
        state.response.progress = progress;
        state.response.backlog.pending_keys = state.response.progress.pending_keys;
        state.response.backlog.oldest_change_age_ms = state.response.progress.oldest_change_age_ms;
        state.response.pressure.copy_throughput_per_sec =
            state.response.progress.replay_throughput_per_sec;
        state.response.pressure.incoming_change_rate_per_sec =
            state.response.progress.incoming_rate_per_sec;
        state.response.pressure.non_convergent = state.response.progress.converged == Some(false);
        state.status.updated_at_epoch_ms =
            now.timestamp_millis().max(state.status.updated_at_epoch_ms);
        refresh_response(&mut state);
        let response = state.response.clone();
        drop(state);
        let _ = self.events.send(StorageMigrationEvent {
            kind: "progress",
            status: response.clone(),
        });
        Ok(response)
    }
}

fn build_response(
    status: &MigrationStatus,
    target_configured: bool,
    switch_budget_ms: u64,
    last_action: Option<String>,
    last_action_at: Option<String>,
) -> StorageMigrationResponse {
    let state = StorageMigrationState::from(status.state);
    StorageMigrationResponse {
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
            identity: SOURCE_IDENTITY.to_string(),
        },
        target: StorageMigrationEndpoint {
            backend: BackendKind::Postgres.into(),
            configured: target_configured,
            identity: status.target_identity.clone(),
        },
        authority: StorageMigrationAuthority {
            backend: BackendKind::Sqlite.into(),
            generation: status.authority_generation,
        },
        queued_modules: Vec::new(),
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
            task_hold: status.hold_reason.clone(),
            drain_state: StorageMigrationDrainState::Unknown,
            switch_attempt_count: 0,
            switch_duration_ms: None,
            switch_result: None,
            failure_reason: None,
        }],
        backlog: StorageMigrationBacklog::default(),
        verification: StorageMigrationVerification::default(),
        task_holds: StorageMigrationTaskHolds {
            held: status.hold_reason.is_some(),
            preserved_task_keys: Vec::new(),
            reason: status.hold_reason.clone(),
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
            authority_backend: Some(BackendKind::Sqlite.into()),
            authority_generation: Some(status.authority_generation),
        },
        controls: StorageMigrationControls {
            permission: "admin",
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
    let progress = state.response.progress.clone();
    let tables = state.response.tables.clone();
    let modules = state.response.modules.clone();
    let backlog = state.response.backlog.clone();
    let verification = state.response.verification.clone();
    let pressure = state.response.pressure.clone();
    let switch = state.response.switch.clone();
    let recovery = state.response.recovery.clone();
    let mut response = build_response(
        &state.status,
        state.target_configured,
        switch.budget_ms,
        state.last_action.clone(),
        state.last_action_at.clone(),
    );
    response.progress = progress;
    response.tables = tables;
    if !modules.is_empty() {
        response.modules = modules;
    }
    response.backlog = backlog;
    response.verification = verification;
    response.pressure = pressure;
    response.switch = switch;
    response.recovery = recovery;
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
