use crate::oauth_bridge::oauth_codex_upstream_base_url;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use axum::{
    extract::{OriginalUri, Path as AxumPath, Query},
    http::{Uri, header},
    response::Html,
};
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use futures_util::FutureExt;
use rand::{Rng, RngCore, rngs::OsRng};
use sqlx::Transaction;
use std::{any::Any, collections::BTreeSet, panic::AssertUnwindSafe};
pub(crate) const ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET: &str =
    "UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "UPSTREAM_ACCOUNTS_OAUTH_ISSUER";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "UPSTREAM_ACCOUNTS_USAGE_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: &str =
    "UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: &str =
    "UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: &str =
    "UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: &str =
    "UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY: &str = "UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN";

pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "https://auth.openai.com";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: u64 = 10 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 5 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: u64 = 15 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: u64 = 30;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_SECONDARY_SYNC_INTERVAL_SECS: u64 = 30 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_PRIORITY_AVAILABLE_ACCOUNT_CAP: usize = 100;
const DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM: usize = 4;
const DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS: u64 = 60 * 60;
const DEFAULT_MANUAL_OAUTH_CALLBACK_PORT: u16 = 1455;
const MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 60;
const UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS: u64 = 60;
const UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS: i64 = 6 * 60 * 60;
const OAUTH_MAILBOX_SOURCE_GENERATED: &str = "generated";
const OAUTH_MAILBOX_SOURCE_ATTACHED: &str = "attached";

const UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX: &str = "oauth_codex";
const UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX: &str = "api_key_codex";
const UPSTREAM_ACCOUNT_PROVIDER_CODEX: &str = "codex";
const UPSTREAM_ACCOUNT_STATUS_ACTIVE: &str = "active";
const UPSTREAM_ACCOUNT_STATUS_SYNCING: &str = "syncing";
const UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH: &str = "needs_reauth";
const UPSTREAM_ACCOUNT_STATUS_ERROR: &str = "error";
const UPSTREAM_ACCOUNT_STATUS_DISABLED: &str = "disabled";
const UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED: &str = "enabled";
const UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED: &str = "disabled";
const UPSTREAM_ACCOUNT_WORK_STATUS_WORKING: &str = "working";
const UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED: &str = "degraded";
const UPSTREAM_ACCOUNT_WORK_STATUS_IDLE: &str = "idle";
const UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED: &str = "rate_limited";
const UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE: &str = "unavailable";
const UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL: &str = "normal";
const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE: &str = "upstream_unavailable";
const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED: &str = "upstream_rejected";
const UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER: &str = "error_other";
const UPSTREAM_ACCOUNT_SYNC_STATE_IDLE: &str = "idle";
const UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING: &str = "syncing";
const UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED: &str = "route_recovered";
const UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED: &str = "route_cooldown_started";
const UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE: &str = "route_retryable_failure";
const UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE: &str = "route_hard_unavailable";
const UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED: &str = "sync_succeeded";
const UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE: &str = "sync_hard_unavailable";
const UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED: &str = "sync_recovery_blocked";
const UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED: &str = "sync_failed";
const UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED: &str = "account_updated";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL: &str = "call";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL: &str = "sync_manual";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE: &str = "sync_maintenance";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE: &str = "sync_post_create";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT: &str = "oauth_import";
const UPSTREAM_ACCOUNT_ACTION_SOURCE_ACCOUNT_UPDATE: &str = "account_update";
const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK: &str = "sync_ok";
const UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED: &str = "account_updated";
const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR: &str = "sync_error";
const UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED: &str = "usage_snapshot_exhausted";
const UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED: &str = "quota_still_exhausted";
const UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED: &str =
    "recovery_unconfirmed_manual_required";
const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT: &str =
    "upstream_http_429_rate_limit";
const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED: &str =
    "upstream_http_429_quota_exhausted";
const UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE: &str = "transport_failure";
const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED: &str =
    "upstream_server_overloaded";
const UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED: &str = "reauth_required";
const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED: &str =
    "group_node_shunt_unassigned";
const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE: &str =
    "分组节点分流策略控制，未排节点";
const BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE: &str = "enable";
const BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE: &str = "disable";
const BULK_UPSTREAM_ACCOUNT_ACTION_DELETE: &str = "delete";
const BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP: &str = "set_group";
const BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS: &str = "add_tags";
const BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS: &str = "remove_tags";
const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING: &str = "pending";
const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED: &str = "succeeded";
const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED: &str = "failed";
const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED: &str = "skipped";
const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING: &str = "running";
const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED: &str = "completed";
const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED: &str = "failed";
const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED: &str = "cancelled";
const LOGIN_SESSION_STATUS_PENDING: &str = "pending";
const LOGIN_SESSION_STATUS_COMPLETED: &str = "completed";
const LOGIN_SESSION_STATUS_FAILED: &str = "failed";
const LOGIN_SESSION_STATUS_EXPIRED: &str = "expired";
const LOGIN_SESSION_BASE_UPDATED_AT_HEADER: &str = "x-codex-login-session-base-updated-at";
const IMPORT_VALIDATION_STATUS_OK: &str = "ok";
const IMPORT_VALIDATION_STATUS_OK_EXHAUSTED: &str = "ok_exhausted";
const IMPORT_VALIDATION_STATUS_INVALID: &str = "invalid";
const IMPORT_VALIDATION_STATUS_ERROR: &str = "error";
const IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT: &str = "duplicate_in_input";
const IMPORT_RESULT_STATUS_CREATED: &str = "created";
const IMPORT_RESULT_STATUS_UPDATED_EXISTING: &str = "updated_existing";
const IMPORT_RESULT_STATUS_FAILED: &str = "failed";
const DEFAULT_OAUTH_SCOPE: &str = "openid profile email offline_access";
const DEFAULT_OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
const DEFAULT_OAUTH_PROMPT: &str = "login";
const OAUTH_ORIGINATOR: &str = "Codex Desktop";
const DEFAULT_USAGE_LIMIT_ID: &str = "codex";
const DEFAULT_API_KEY_LIMIT_UNIT: &str = "requests";
const POOL_SETTINGS_SINGLETON_ID: i64 = 1;
const DEFAULT_STICKY_KEY_LIMIT: i64 = 50;
const STICKY_KEY_ACTIVITY_MODE_LIMIT: i64 = 50;
const DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE: usize = 20;
const UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS: [usize; 3] = [20, 50, 100];
const POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES: i64 = 5;
const POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD: i64 = 5;
const POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS: i64 = 30;
const POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS: i64 = 60;
pub(crate) const COMPACT_SUPPORT_STATUS_UNKNOWN: &str = "unknown";
pub(crate) const COMPACT_SUPPORT_STATUS_SUPPORTED: &str = "supported";
pub(crate) const COMPACT_SUPPORT_STATUS_UNSUPPORTED: &str = "unsupported";
const USAGE_PATH_STYLE_CHATGPT: &str = "/wham/usage";
const USAGE_PATH_STYLE_CODEX_API: &str = "/api/codex/usage";
const UPSTREAM_USAGE_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

#[derive(Debug)]
pub(crate) struct UpstreamAccountsRuntime {
    pub(crate) crypto_key: Option<[u8; 32]>,
    account_ops: AccountOpCoordinator,
    validation_jobs: Arc<Mutex<HashMap<String, Arc<ImportedOauthValidationJob>>>>,
    bulk_sync_jobs: Arc<Mutex<HashMap<String, Arc<BulkUpstreamAccountSyncJob>>>>,
    bulk_sync_creation: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountCommand {
    UpdateAccount,
    ExternalOauthUpsert,
    DeleteAccount,
    ManualSync,
    MaintenanceSync,
    PersistOauthCallback,
    PersistImportedOauth,
    PostCreateSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncCause {
    Manual,
    Maintenance,
    PostCreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncSuccessRouteState {
    PreserveFailureState,
    ClearFailureState,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaintenanceDispatchOutcome {
    Executed,
    Skipped,
    Deduped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaintenanceQueueOutcome {
    Queued,
    Deduped,
}

struct MaintenancePendingGuard {
    flag: Arc<AtomicBool>,
}

impl MaintenancePendingGuard {
    fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

impl Drop for MaintenancePendingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
struct AccountActorHandle {
    serial: Arc<tokio::sync::Mutex<()>>,
    maintenance_pending: Arc<AtomicBool>,
}

impl fmt::Debug for AccountActorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountActorHandle")
            .field("serial_refs", &Arc::strong_count(&self.serial))
            .field(
                "maintenance_pending",
                &self.maintenance_pending.load(Ordering::Acquire),
            )
            .finish()
    }
}

#[derive(Clone)]
struct AccountOpCoordinator {
    actors: Arc<std::sync::Mutex<HashMap<i64, AccountActorHandle>>>,
    maintenance_slots: Arc<tokio::sync::Semaphore>,
    maintenance_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl Default for AccountOpCoordinator {
    fn default() -> Self {
        Self::new(DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM)
    }
}

impl fmt::Debug for AccountOpCoordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actor_count = self
            .actors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        f.debug_struct("AccountOpCoordinator")
            .field("actor_count", &actor_count)
            .field(
                "maintenance_slots_available",
                &self.maintenance_slots.available_permits(),
            )
            .field(
                "maintenance_handle_count",
                &self
                    .maintenance_handles
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .len(),
            )
            .finish()
    }
}

#[derive(Debug)]
enum AccountCommandDispatchError<E> {
    Command(E),
    ActorUnavailable(AccountCommand),
}

#[derive(Debug)]
enum AccountSubmitOutcome<T> {
    Completed(T),
    Deduped,
}

impl UpstreamAccountsRuntime {
    pub(crate) fn from_env() -> Result<Self> {
        let crypto_key = match env::var(ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET) {
            Ok(value) if !value.trim().is_empty() => Some(derive_secret_key(&value)),
            Ok(_) => {
                return Err(anyhow!(
                    "{} must not be empty when configured",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ));
            }
            Err(env::VarError::NotPresent) => None,
            Err(err) => {
                return Err(anyhow!(
                    "failed to read {}: {err}",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ));
            }
        };

        Ok(Self {
            crypto_key,
            account_ops: AccountOpCoordinator::default(),
            validation_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_creation: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn writes_enabled(&self) -> bool {
        self.crypto_key.is_some()
    }

    pub(crate) fn require_crypto_key(&self) -> Result<&[u8; 32], (StatusCode, String)> {
        self.crypto_key.as_ref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "account writes require {} to be configured",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn test_instance() -> Self {
        Self::test_instance_with_maintenance_parallelism(
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_instance_with_maintenance_parallelism(
        maintenance_parallelism: usize,
    ) -> Self {
        Self {
            crypto_key: Some(derive_secret_key("test-upstream-account-secret")),
            account_ops: AccountOpCoordinator::new(maintenance_parallelism),
            validation_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_creation: Arc::new(Mutex::new(())),
        }
    }

    async fn insert_validation_job(&self, job_id: String, job: Arc<ImportedOauthValidationJob>) {
        self.validation_jobs.lock().await.insert(job_id, job);
    }

    async fn get_validation_job(&self, job_id: &str) -> Option<Arc<ImportedOauthValidationJob>> {
        self.validation_jobs.lock().await.get(job_id).cloned()
    }

    async fn remove_validation_job(&self, job_id: &str) -> Option<Arc<ImportedOauthValidationJob>> {
        self.validation_jobs.lock().await.remove(job_id)
    }

    async fn insert_bulk_sync_job(&self, job_id: String, job: Arc<BulkUpstreamAccountSyncJob>) {
        self.bulk_sync_jobs.lock().await.insert(job_id, job);
    }

    async fn get_bulk_sync_job(&self, job_id: &str) -> Option<Arc<BulkUpstreamAccountSyncJob>> {
        self.bulk_sync_jobs.lock().await.get(job_id).cloned()
    }

    async fn get_running_bulk_sync_job(&self) -> Option<(String, Arc<BulkUpstreamAccountSyncJob>)> {
        let jobs = self.bulk_sync_jobs.lock().await;
        for (job_id, job) in jobs.iter() {
            if job.terminal_event.lock().await.is_none() {
                return Some((job_id.clone(), job.clone()));
            }
        }
        None
    }

    async fn remove_bulk_sync_job(&self, job_id: &str) -> Option<Arc<BulkUpstreamAccountSyncJob>> {
        self.bulk_sync_jobs.lock().await.remove(job_id)
    }

    pub(crate) async fn drain_background_tasks(&self) {
        self.account_ops.drain_maintenance_tasks().await;
    }
}

impl AccountOpCoordinator {
    fn new(maintenance_parallelism: usize) -> Self {
        Self {
            actors: Arc::new(std::sync::Mutex::new(HashMap::new())),
            maintenance_slots: Arc::new(tokio::sync::Semaphore::new(
                maintenance_parallelism.max(1),
            )),
            maintenance_handles: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn actor_handle(&self, account_id: i64) -> AccountActorHandle {
        let mut actors = self
            .actors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(handle) = actors.get(&account_id) {
            return handle.clone();
        }

        let maintenance_pending = Arc::new(AtomicBool::new(false));
        let handle = AccountActorHandle {
            serial: Arc::new(tokio::sync::Mutex::new(())),
            maintenance_pending,
        };
        let actor_handle = handle.clone();
        actors.insert(account_id, actor_handle.clone());
        actor_handle
    }

    fn remove_actor_if_idle(&self, account_id: i64, handle: &AccountActorHandle) {
        let mut actors = self
            .actors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = actors.get(&account_id) else {
            return;
        };
        if !Arc::ptr_eq(&current.serial, &handle.serial)
            || !Arc::ptr_eq(&current.maintenance_pending, &handle.maintenance_pending)
        {
            return;
        }

        // `2` means the only remaining owners are the map entry and this call frame.
        if Arc::strong_count(&handle.serial) == 2
            && Arc::strong_count(&handle.maintenance_pending) == 2
            && !handle.maintenance_pending.load(Ordering::Acquire)
        {
            actors.remove(&account_id);
        }
    }

    #[cfg(test)]
    fn actor_count(&self) -> usize {
        self.actors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    async fn run_command_with_handle<R, E, F, Fut>(
        &self,
        state: Arc<AppState>,
        account_id: i64,
        command: AccountCommand,
        handle: AccountActorHandle,
        job_factory: F,
    ) -> Result<R, AccountCommandDispatchError<E>>
    where
        R: Send + 'static,
        E: Send + 'static,
        F: FnOnce(Arc<AppState>, i64) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, E>> + Send + 'static,
    {
        let result = {
            let _serial_guard = handle.serial.lock().await;
            let maintenance_pending = handle.maintenance_pending.clone();
            let _reset_guard = (command == AccountCommand::MaintenanceSync)
                .then(|| MaintenancePendingGuard::new(maintenance_pending));
            AssertUnwindSafe(job_factory(state, account_id))
                .catch_unwind()
                .await
        };
        self.remove_actor_if_idle(account_id, &handle);

        match result {
            Ok(result) => result.map_err(AccountCommandDispatchError::Command),
            Err(panic) => {
                error!(
                    account_id,
                    panic = %describe_panic_payload(&panic),
                    "account actor job panicked"
                );
                Err(AccountCommandDispatchError::ActorUnavailable(command))
            }
        }
    }

    async fn drain_maintenance_tasks(&self) {
        let mut handles = {
            let mut guard = self
                .maintenance_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        handles.retain(|handle| !handle.is_finished());
        for handle in handles {
            if let Err(err) = handle.await {
                error!(?err, "queued maintenance task terminated unexpectedly");
            }
        }
    }

    async fn submit_command<R, E, F, Fut>(
        &self,
        state: Arc<AppState>,
        account_id: i64,
        command: AccountCommand,
        dedupe: bool,
        job_factory: F,
    ) -> Result<AccountSubmitOutcome<R>, AccountCommandDispatchError<E>>
    where
        R: Send + 'static,
        E: Send + 'static,
        F: FnOnce(Arc<AppState>, i64) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, E>> + Send + 'static,
    {
        let handle = self.actor_handle(account_id);
        if dedupe && handle.maintenance_pending.swap(true, Ordering::AcqRel) {
            return Ok(AccountSubmitOutcome::Deduped);
        }

        self.run_command_with_handle(state, account_id, command, handle, job_factory)
            .await
            .map(AccountSubmitOutcome::Completed)
    }

    async fn run_update_account(
        &self,
        state: Arc<AppState>,
        id: i64,
        payload: UpdateUpstreamAccountRequest,
    ) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::UpdateAccount,
            false,
            move |state, id| async move {
                update_upstream_account_inner(state.as_ref(), id, payload).await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::UpdateAccount)
    }

    async fn run_external_oauth_upsert(
        &self,
        state: Arc<AppState>,
        id: i64,
        identity: ExternalAccountIdentity,
        metadata: ExternalUpstreamAccountMetadataRequest,
        probe: ImportedOauthProbeOutcome,
    ) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::ExternalOauthUpsert,
            false,
            move |state, id| async move {
                persist_external_existing_oauth_upsert(
                    state.as_ref(),
                    &identity,
                    id,
                    &metadata,
                    probe,
                )
                .await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::ExternalOauthUpsert)
    }

    async fn run_delete_account(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<StatusCode, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::DeleteAccount,
            false,
            move |state, id| async move { delete_upstream_account_inner(state.as_ref(), id).await },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::DeleteAccount)
    }

    async fn run_manual_sync(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<UpstreamAccountDetail, anyhow::Error> {
        self.submit_command(
            state,
            id,
            AccountCommand::ManualSync,
            false,
            move |state, id| async move {
                sync_upstream_account_by_id(state.as_ref(), id, SyncCause::Manual).await
            },
        )
        .await
        .map_err(map_account_dispatch_anyhow)
        .and_then(|outcome| match outcome {
            AccountSubmitOutcome::Completed(Some(detail)) => Ok(detail),
            AccountSubmitOutcome::Completed(None) => Err(anyhow!("manual sync returned no detail")),
            AccountSubmitOutcome::Deduped => {
                Err(anyhow!("manual sync was unexpectedly deduplicated"))
            }
        })
    }

    async fn run_post_create_sync(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<UpstreamAccountDetail, anyhow::Error> {
        self.submit_command(
            state,
            id,
            AccountCommand::PostCreateSync,
            false,
            move |state, id| async move {
                sync_upstream_account_by_id(state.as_ref(), id, SyncCause::PostCreate).await
            },
        )
        .await
        .map_err(map_account_dispatch_anyhow)
        .and_then(|outcome| match outcome {
            AccountSubmitOutcome::Completed(Some(detail)) => Ok(detail),
            AccountSubmitOutcome::Completed(None) => {
                Err(anyhow!("post-create sync returned no detail"))
            }
            AccountSubmitOutcome::Deduped => {
                Err(anyhow!("post-create sync was unexpectedly deduplicated"))
            }
        })
    }

    #[cfg(test)]
    async fn run_maintenance_sync(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<MaintenanceDispatchOutcome, anyhow::Error> {
        match self
            .submit_command(
                state,
                id,
                AccountCommand::MaintenanceSync,
                true,
                move |state, id| async move {
                    sync_upstream_account_by_id(state.as_ref(), id, SyncCause::Maintenance).await
                },
            )
            .await
            .map_err(map_account_dispatch_anyhow)?
        {
            AccountSubmitOutcome::Completed(Some(_)) => Ok(MaintenanceDispatchOutcome::Executed),
            AccountSubmitOutcome::Completed(None) => Ok(MaintenanceDispatchOutcome::Skipped),
            AccountSubmitOutcome::Deduped => Ok(MaintenanceDispatchOutcome::Deduped),
        }
    }

    async fn run_persist_oauth_callback(
        &self,
        state: Arc<AppState>,
        id: i64,
        input: PersistOauthCallbackInput,
    ) -> Result<i64, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::PersistOauthCallback,
            false,
            move |state, _| async move {
                persist_existing_oauth_callback_inner(state.as_ref(), input).await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::PersistOauthCallback)
    }

    async fn run_persist_imported_oauth(
        &self,
        state: Arc<AppState>,
        id: i64,
        probe: ImportedOauthProbeOutcome,
    ) -> Result<Option<String>, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::PersistImportedOauth,
            false,
            move |state, id| async move {
                persist_imported_oauth_existing_inner(state.as_ref(), id, probe).await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::PersistImportedOauth)
    }

    fn dispatch_maintenance_sync(
        &self,
        state: Arc<AppState>,
        plan: MaintenanceDispatchPlan,
    ) -> Result<MaintenanceQueueOutcome, anyhow::Error> {
        let id = plan.account_id;
        let handle = self.actor_handle(id);
        if handle.maintenance_pending.swap(true, Ordering::AcqRel) {
            return Ok(MaintenanceQueueOutcome::Deduped);
        }

        let coordinator = self.clone();
        let handle = tokio::spawn(async move {
            let _permit = match coordinator.maintenance_slots.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(err) => {
                    warn!(
                        account_id = id,
                        error = %err,
                        "maintenance slots closed before sync started"
                    );
                    handle.maintenance_pending.store(false, Ordering::Release);
                    coordinator.remove_actor_if_idle(id, &handle);
                    return;
                }
            };
            match coordinator
                .run_command_with_handle(
                    state,
                    id,
                    AccountCommand::MaintenanceSync,
                    handle,
                    move |state, id| async move {
                        execute_queued_maintenance_sync(state.as_ref(), plan, id).await
                    },
                )
                .await
            {
                Ok(Some(_)) | Ok(None) => {}
                Err(AccountCommandDispatchError::Command(err)) => {
                    warn!(account_id = id, error = %err, "failed to maintain upstream OAuth account");
                }
                Err(AccountCommandDispatchError::ActorUnavailable(command)) => {
                    warn!(
                        account_id = id,
                        ?command,
                        "account actor became unavailable while executing maintenance"
                    );
                }
            }
        });
        let mut maintenance_handles = self
            .maintenance_handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        maintenance_handles.retain(|handle| !handle.is_finished());
        maintenance_handles.push(handle);

        Ok(MaintenanceQueueOutcome::Queued)
    }
}

impl<T> AccountSubmitOutcome<T> {
    fn expect_completed(self, command: AccountCommand) -> Result<T, (StatusCode, String)> {
        match self {
            Self::Completed(value) => Ok(value),
            Self::Deduped => Err(internal_error_tuple(anyhow!(
                "account command {:?} unexpectedly deduped",
                command
            ))),
        }
    }
}

fn describe_panic_payload(payload: &Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn map_account_dispatch_http(
    err: AccountCommandDispatchError<(StatusCode, String)>,
) -> (StatusCode, String) {
    match err {
        AccountCommandDispatchError::Command(err) => err,
        AccountCommandDispatchError::ActorUnavailable(command) => internal_error_tuple(anyhow!(
            "account actor became unavailable while executing {:?}",
            command
        )),
    }
}

fn map_account_dispatch_anyhow(err: AccountCommandDispatchError<anyhow::Error>) -> anyhow::Error {
    match err {
        AccountCommandDispatchError::Command(err) => err,
        AccountCommandDispatchError::ActorUnavailable(command) => anyhow!(
            "account actor became unavailable while executing {:?}",
            command
        ),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListMetrics {
    total: usize,
    oauth: usize,
    api_key: usize,
    attention: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListResponse {
    writes_enabled: bool,
    items: Vec<UpstreamAccountSummary>,
    total: usize,
    page: usize,
    page_size: usize,
    metrics: UpstreamAccountListMetrics,
    groups: Vec<UpstreamAccountGroupSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forward_proxy_nodes: Vec<ForwardProxyBindingNodeResponse>,
    has_ungrouped_accounts: bool,
    routing: PoolRoutingSettingsResponse,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListForwardProxyBindingNodesQuery {
    #[serde(default)]
    pub(crate) key: Vec<String>,
    #[serde(default)]
    pub(crate) include_current: bool,
    pub(crate) group_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsQuery {
    pub(crate) group_search: Option<String>,
    pub(crate) group_ungrouped: Option<bool>,
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) work_status: Vec<String>,
    #[serde(default)]
    pub(crate) enable_status: Vec<String>,
    #[serde(default)]
    pub(crate) health_status: Vec<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListUpstreamAccountsBaseQuery {
    group_search: Option<String>,
    group_ungrouped: Option<bool>,
    status: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DuplicateReason {
    SharedChatgptAccountId,
    SharedChatgptUserId,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TagPriorityTier {
    Fallback,
    Normal,
    Primary,
}

impl Default for TagPriorityTier {
    fn default() -> Self {
        Self::Normal
    }
}

impl TagPriorityTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fallback => "fallback",
            Self::Normal => "normal",
            Self::Primary => "primary",
        }
    }

    fn routing_rank(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Normal => 1,
            Self::Fallback => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TagFastModeRewriteMode {
    ForceRemove,
    #[default]
    KeepOriginal,
    FillMissing,
    ForceAdd,
}

impl TagFastModeRewriteMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ForceRemove => "force_remove",
            Self::KeepOriginal => "keep_original",
            Self::FillMissing => "fill_missing",
            Self::ForceAdd => "force_add",
        }
    }

    fn merge_rank(self) -> u8 {
        match self {
            Self::ForceRemove => 0,
            Self::ForceAdd => 1,
            Self::FillMissing => 2,
            Self::KeepOriginal => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DuplicateInfo {
    peer_account_ids: Vec<i64>,
    reasons: Vec<DuplicateReason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTagSummary {
    id: i64,
    name: String,
    routing_rule: TagRoutingRule,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveConversationGuard {
    tag_id: i64,
    tag_name: String,
    lookback_hours: i64,
    max_conversations: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveRoutingRule {
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    concurrency_limit: i64,
    source_tag_ids: Vec<i64>,
    source_tag_names: Vec<String>,
    guard_rules: Vec<EffectiveConversationGuard>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagRoutingRule {
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: TagPriorityTier,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    concurrency_limit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagSummary {
    id: i64,
    name: String,
    routing_rule: TagRoutingRule,
    account_count: i64,
    group_count: i64,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagDetail {
    #[serde(flatten)]
    summary: TagSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagListResponse {
    writes_enabled: bool,
    items: Vec<TagSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountGroupSummary {
    group_name: String,
    note: Option<String>,
    bound_proxy_keys: Vec<String>,
    node_shunt_enabled: bool,
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
    concurrency_limit: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamAccountGroupMetadata {
    note: Option<String>,
    bound_proxy_keys: Vec<String>,
    node_shunt_enabled: bool,
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
    concurrency_limit: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestedGroupMetadataChanges {
    note: Option<String>,
    note_was_requested: bool,
    bound_proxy_keys: Vec<String>,
    bound_proxy_keys_was_requested: bool,
    concurrency_limit: i64,
    concurrency_limit_was_requested: bool,
    node_shunt_enabled: bool,
    node_shunt_enabled_was_requested: bool,
}

impl RequestedGroupMetadataChanges {
    fn was_requested(&self) -> bool {
        self.note_was_requested
            || self.bound_proxy_keys_was_requested
            || self.concurrency_limit_was_requested
            || self.node_shunt_enabled_was_requested
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedRequiredGroupProxyBinding {
    pub(crate) group_name: String,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountSummary {
    id: i64,
    kind: String,
    provider: String,
    display_name: String,
    group_name: Option<String>,
    is_mother: bool,
    status: String,
    display_status: String,
    enabled: bool,
    work_status: String,
    enable_status: String,
    health_status: String,
    sync_state: String,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    plan_type: Option<String>,
    masked_api_key: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_activity_at: Option<String>,
    active_conversation_count: i64,
    last_error: Option<String>,
    last_error_at: Option<String>,
    last_action: Option<String>,
    last_action_source: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_reason_message: Option<String>,
    last_action_http_status: Option<u16>,
    last_action_invoke_id: Option<String>,
    last_action_at: Option<String>,
    routing_block_reason_code: Option<String>,
    routing_block_reason_message: Option<String>,
    token_expires_at: Option<String>,
    primary_window: Option<RateWindowSnapshot>,
    secondary_window: Option<RateWindowSnapshot>,
    credits: Option<CreditsSnapshot>,
    local_limits: Option<LocalLimitSnapshot>,
    compact_support: CompactSupportState,
    duplicate_info: Option<DuplicateInfo>,
    tags: Vec<AccountTagSummary>,
    effective_routing_rule: EffectiveRoutingRule,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountDetail {
    #[serde(flatten)]
    summary: UpstreamAccountSummary,
    note: Option<String>,
    upstream_base_url: Option<String>,
    chatgpt_user_id: Option<String>,
    last_refreshed_at: Option<String>,
    history: Vec<UpstreamAccountHistoryPoint>,
    recent_actions: Vec<UpstreamAccountActionEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActionEvent {
    id: i64,
    occurred_at: String,
    action: String,
    source: String,
    reason_code: Option<String>,
    reason_message: Option<String>,
    http_status: Option<u16>,
    failure_kind: Option<String>,
    invoke_id: Option<String>,
    sticky_key: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompactSupportState {
    status: String,
    observed_at: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingTimeoutSettingsResponse {
    pub(crate) responses_first_byte_timeout_secs: u64,
    pub(crate) compact_first_byte_timeout_secs: u64,
    pub(crate) responses_stream_timeout_secs: u64,
    pub(crate) compact_stream_timeout_secs: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolRoutingTimeoutSettingsResolved {
    pub(crate) default_first_byte_timeout: Duration,
    pub(crate) default_send_timeout: Duration,
    pub(crate) request_read_timeout: Duration,
    pub(crate) responses_first_byte_timeout: Duration,
    pub(crate) compact_first_byte_timeout: Duration,
    pub(crate) responses_stream_timeout: Duration,
    pub(crate) compact_stream_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSettingsResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) api_key_configured: bool,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) maintenance: PoolRoutingMaintenanceSettingsResponse,
    pub(crate) timeouts: PoolRoutingTimeoutSettingsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingMaintenanceSettingsResponse {
    pub(crate) primary_sync_interval_secs: u64,
    pub(crate) secondary_sync_interval_secs: u64,
    pub(crate) priority_available_account_cap: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingSettingsRequest {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) maintenance: Option<UpdatePoolRoutingMaintenanceSettingsRequest>,
    #[serde(default)]
    pub(crate) timeouts: Option<UpdatePoolRoutingTimeoutSettingsRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingMaintenanceSettingsRequest {
    #[serde(default)]
    pub(crate) primary_sync_interval_secs: Option<u64>,
    #[serde(default)]
    pub(crate) secondary_sync_interval_secs: Option<u64>,
    #[serde(default)]
    pub(crate) priority_available_account_cap: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingTimeoutSettingsRequest {
    #[serde(default)]
    pub(crate) responses_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    #[serde(alias = "compactUpstreamHandshakeTimeoutSecs")]
    pub(crate) compact_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) responses_stream_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) compact_stream_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeysResponse {
    range_start: String,
    range_end: String,
    selection_mode: AccountStickyKeySelectionMode,
    selected_limit: Option<i64>,
    selected_activity_hours: Option<i64>,
    implicit_filter: AccountStickyKeyImplicitFilter,
    conversations: Vec<AccountStickyKeyConversation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountStickyKeySelectionMode {
    Count,
    ActivityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountStickyKeySelection {
    Count(i64),
    ActivityWindow(i64),
}

impl AccountStickyKeySelection {
    fn selection_mode(self) -> AccountStickyKeySelectionMode {
        match self {
            Self::Count(_) => AccountStickyKeySelectionMode::Count,
            Self::ActivityWindow(_) => AccountStickyKeySelectionMode::ActivityWindow,
        }
    }

    fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindow(_) => None,
        }
    }

    fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindow(hours) => Some(hours),
        }
    }

    fn activity_window_hours(self) -> i64 {
        match self {
            Self::Count(_) => 24,
            Self::ActivityWindow(hours) => hours,
        }
    }

    fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindow(_) => STICKY_KEY_ACTIVITY_MODE_LIMIT,
        }
    }

    fn implicit_filter(
        self,
        filtered_counts: AccountStickyKeyFilteredCounts,
    ) -> AccountStickyKeyImplicitFilter {
        let (kind, filtered_count) = match self {
            Self::Count(_) => (None, 0),
            Self::ActivityWindow(_) if filtered_counts.capped_count > 0 => (
                Some(AccountStickyKeyImplicitFilterKind::CappedTo50),
                filtered_counts.capped_count,
            ),
            Self::ActivityWindow(_) if filtered_counts.inactive_count > 0 => (
                Some(AccountStickyKeyImplicitFilterKind::InactiveOutside24h),
                filtered_counts.inactive_count,
            ),
            Self::ActivityWindow(_) => (None, 0),
        };
        AccountStickyKeyImplicitFilter {
            kind,
            filtered_count: filtered_count.max(0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountStickyKeyFilteredCounts {
    inactive_count: i64,
    capped_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyImplicitFilter {
    kind: Option<AccountStickyKeyImplicitFilterKind>,
    filtered_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountStickyKeyImplicitFilterKind {
    InactiveOutside24h,
    CappedTo50,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyConversation {
    sticky_key: String,
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    last_activity_at: String,
    recent_invocations: Vec<crate::api::PromptCacheConversationInvocationPreviewResponse>,
    last24h_requests: Vec<AccountStickyKeyRequestPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyRequestPoint {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    occurred_at: String,
    status: String,
    is_success: bool,
    request_tokens: i64,
    cumulative_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountHistoryPoint {
    captured_at: String,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    credits_balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowSnapshot {
    used_percent: f64,
    used_text: String,
    limit_text: String,
    resets_at: Option<String>,
    window_duration_mins: i64,
    actual_usage: Option<RateWindowActualUsage>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowActualUsage {
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditsSnapshot {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalLimitSnapshot {
    primary_limit: Option<f64>,
    secondary_limit: Option<f64>,
    limit_unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginSessionStatusResponse {
    login_id: String,
    status: String,
    auth_url: Option<String>,
    redirect_uri: Option<String>,
    expires_at: String,
    updated_at: String,
    account_id: Option<i64>,
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_applied: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxSessionResponse {
    email_address: String,
    supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxCodeSummary {
    value: String,
    source: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthInviteSummary {
    subject: String,
    copy_value: String,
    copy_label: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatus {
    session_id: String,
    email_address: String,
    expires_at: String,
    latest_code: Option<OauthMailboxCodeSummary>,
    invite: Option<OauthInviteSummary>,
    invited: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusBatchResponse {
    items: Vec<OauthMailboxStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthLoginSessionRequest {
    display_name: Option<String>,
    group_name: Option<String>,
    #[serde(default)]
    group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    group_node_shunt_enabled: Option<bool>,
    note: Option<String>,
    group_note: Option<String>,
    concurrency_limit: Option<i64>,
    account_id: Option<i64>,
    #[serde(default)]
    tag_ids: Vec<i64>,
    is_mother: Option<bool>,
    mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteOauthLoginSessionRequest {
    callback_url: String,
    mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateOauthLoginSessionRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    display_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    group_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    group_bound_proxy_keys: OptionalField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    group_node_shunt_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    group_note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    concurrency_limit: OptionalField<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    tag_ids: OptionalField<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    is_mother: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    mailbox_session_id: OptionalField<String>,
    #[serde(
        default,
        alias = "generatedMailboxAddress",
        deserialize_with = "deserialize_optional_field"
    )]
    mailbox_address: OptionalField<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthMailboxSessionRequest {
    email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusRequest {
    #[serde(default)]
    session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApiKeyAccountRequest {
    display_name: String,
    group_name: Option<String>,
    #[serde(default)]
    group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    group_node_shunt_enabled: Option<bool>,
    note: Option<String>,
    group_note: Option<String>,
    concurrency_limit: Option<i64>,
    upstream_base_url: Option<String>,
    api_key: String,
    is_mother: Option<bool>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportOauthCredentialFileRequest {
    source_id: String,
    file_name: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateImportedOauthAccountsRequest {
    group_name: Option<String>,
    #[serde(default)]
    group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    items: Vec<ImportOauthCredentialFileRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportValidatedOauthAccountsRequest {
    #[serde(default)]
    items: Vec<ImportOauthCredentialFileRequest>,
    #[serde(default)]
    selected_source_ids: Vec<String>,
    #[serde(default)]
    validation_job_id: Option<String>,
    group_name: Option<String>,
    #[serde(default)]
    group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    group_node_shunt_enabled: Option<bool>,
    group_note: Option<String>,
    concurrency_limit: Option<i64>,
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthMatchSummary {
    account_id: i64,
    display_name: String,
    group_name: Option<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRow {
    source_id: String,
    file_name: String,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    display_name: Option<String>,
    token_expires_at: Option<String>,
    matched_account: Option<ImportedOauthMatchSummary>,
    status: String,
    detail: Option<String>,
    attempts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationResponse {
    input_files: usize,
    unique_in_input: usize,
    duplicate_in_input: usize,
    rows: Vec<ImportedOauthValidationRow>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationCounts {
    pending: usize,
    duplicate_in_input: usize,
    ok: usize,
    ok_exhausted: usize,
    invalid: usize,
    error: usize,
    checked: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationJobResponse {
    job_id: String,
    snapshot: ImportedOauthValidationResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationSnapshotEvent {
    snapshot: ImportedOauthValidationResponse,
    counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRowEvent {
    row: ImportedOauthValidationRow,
    counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationFailedEvent {
    snapshot: ImportedOauthValidationResponse,
    counts: ImportedOauthValidationCounts,
    error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ImportedOauthValidationTerminalEvent {
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug, Clone)]
enum ImportedOauthValidationJobEvent {
    Row(ImportedOauthValidationRowEvent),
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct ImportedOauthValidationJob {
    target_group_name: String,
    target_bound_proxy_keys: Vec<String>,
    target_node_shunt_enabled: bool,
    snapshot: Mutex<ImportedOauthValidationResponse>,
    validated_imports: Mutex<HashMap<String, ImportedOauthValidatedImportData>>,
    broadcaster: broadcast::Sender<ImportedOauthValidationJobEvent>,
    cancel: CancellationToken,
    terminal_event: Mutex<Option<ImportedOauthValidationTerminalEvent>>,
}

impl ImportedOauthValidationJob {
    fn new(
        snapshot: ImportedOauthValidationResponse,
        binding: &ResolvedRequiredGroupProxyBinding,
    ) -> Self {
        let (broadcaster, _rx) = broadcast::channel(256);
        Self {
            target_group_name: binding.group_name.clone(),
            target_bound_proxy_keys: binding.bound_proxy_keys.clone(),
            target_node_shunt_enabled: binding.node_shunt_enabled,
            snapshot: Mutex::new(snapshot),
            validated_imports: Mutex::new(HashMap::new()),
            broadcaster,
            cancel: CancellationToken::new(),
            terminal_event: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionRequest {
    account_ids: Vec<i64>,
    action: String,
    group_name: Option<String>,
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResponse {
    action: String,
    requested_count: usize,
    completed_count: usize,
    succeeded_count: usize,
    failed_count: usize,
    results: Vec<BulkUpstreamAccountActionResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResult {
    account_id: i64,
    display_name: Option<String>,
    status: String,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobRequest {
    account_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncCounts {
    total: usize,
    completed: usize,
    succeeded: usize,
    failed: usize,
    skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRow {
    account_id: i64,
    display_name: String,
    status: String,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshot {
    job_id: String,
    status: String,
    rows: Vec<BulkUpstreamAccountSyncRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobResponse {
    job_id: String,
    snapshot: BulkUpstreamAccountSyncSnapshot,
    counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRowEvent {
    row: BulkUpstreamAccountSyncRow,
    counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshotEvent {
    snapshot: BulkUpstreamAccountSyncSnapshot,
    counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncFailedEvent {
    snapshot: BulkUpstreamAccountSyncSnapshot,
    counts: BulkUpstreamAccountSyncCounts,
    error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum BulkUpstreamAccountSyncTerminalEvent {
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug, Clone)]
enum BulkUpstreamAccountSyncJobEvent {
    Row(BulkUpstreamAccountSyncRowEvent),
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct BulkUpstreamAccountSyncJob {
    snapshot: Mutex<BulkUpstreamAccountSyncSnapshot>,
    broadcaster: broadcast::Sender<BulkUpstreamAccountSyncJobEvent>,
    cancel: CancellationToken,
    terminal_event: Mutex<Option<BulkUpstreamAccountSyncTerminalEvent>>,
}

impl BulkUpstreamAccountSyncJob {
    fn new(snapshot: BulkUpstreamAccountSyncSnapshot) -> Self {
        let (broadcaster, _rx) = broadcast::channel(256);
        Self {
            snapshot: Mutex::new(snapshot),
            broadcaster,
            cancel: CancellationToken::new(),
            terminal_event: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportResult {
    source_id: String,
    file_name: String,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    account_id: Option<i64>,
    status: String,
    detail: Option<String>,
    matched_account: Option<ImportedOauthMatchSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportSummary {
    input_files: usize,
    selected_files: usize,
    created: usize,
    updated_existing: usize,
    failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportResponse {
    summary: ImportedOauthImportSummary,
    results: Vec<ImportedOauthImportResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountRequest {
    display_name: Option<String>,
    group_name: Option<String>,
    #[serde(default)]
    group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    group_node_shunt_enabled: Option<bool>,
    note: Option<String>,
    group_note: Option<String>,
    concurrency_limit: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    upstream_base_url: OptionalField<String>,
    enabled: Option<bool>,
    is_mother: Option<bool>,
    api_key: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
    tag_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExternalAccountIdentity {
    pub(crate) client_id: String,
    pub(crate) source_account_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalOauthCredentialsRequest {
    pub(crate) email: String,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) id_token: String,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    #[serde(default)]
    pub(crate) expired: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountMetadataRequest {
    pub(crate) display_name: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) enabled: Option<bool>,
    pub(crate) is_mother: Option<bool>,
    #[serde(default)]
    pub(crate) tag_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountUpsertRequest {
    #[serde(flatten)]
    pub(crate) metadata: ExternalUpstreamAccountMetadataRequest,
    pub(crate) oauth: ExternalOauthCredentialsRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountReloginRequest {
    pub(crate) oauth: ExternalOauthCredentialsRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTagRequest {
    name: String,
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<String>,
    fast_mode_rewrite_mode: Option<String>,
    concurrency_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateTagRequest {
    name: Option<String>,
    guard_enabled: Option<bool>,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: Option<bool>,
    allow_cut_in: Option<bool>,
    priority_tier: Option<String>,
    fast_mode_rewrite_mode: Option<String>,
    concurrency_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListTagsQuery {
    search: Option<String>,
    has_accounts: Option<bool>,
    guard_enabled: Option<bool>,
    allow_cut_in: Option<bool>,
    allow_cut_out: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeysQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) activity_hours: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountGroupRequest {
    note: Option<String>,
    #[serde(default)]
    bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    node_shunt_enabled: Option<bool>,
    #[serde(default)]
    upstream_429_retry_enabled: Option<bool>,
    #[serde(default)]
    upstream_429_max_retries: Option<u8>,
    concurrency_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OauthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredApiKeyCredentials {
    api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredOauthCredentials {
    access_token: String,
    refresh_token: String,
    id_token: String,
    token_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StoredCredentials {
    ApiKey(StoredApiKeyCredentials),
    Oauth(StoredOauthCredentials),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedCredentialsPayload {
    v: u8,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone)]
struct NormalizedUsageSnapshot {
    plan_type: Option<String>,
    limit_id: String,
    limit_name: Option<String>,
    primary: Option<NormalizedUsageWindow>,
    secondary: Option<NormalizedUsageWindow>,
    credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone)]
struct NormalizedUsageWindow {
    used_percent: f64,
    window_duration_mins: i64,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    expires_in: i64,
}

#[derive(Debug, Clone, Default)]
struct ChatgptJwtClaims {
    email: Option<String>,
    chatgpt_plan_type: Option<String>,
    chatgpt_user_id: Option<String>,
    chatgpt_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImportedOauthCredentialsFile {
    #[serde(rename = "type")]
    source_type: String,
    email: String,
    account_id: String,
    #[serde(default)]
    expired: Option<String>,
    access_token: String,
    refresh_token: String,
    id_token: String,
    #[serde(default)]
    #[serde(rename = "last_refresh")]
    _last_refresh: Option<serde_json::Value>,
    #[serde(default)]
    token_type: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedImportedOauthCredentials {
    source_id: String,
    file_name: String,
    email: String,
    display_name: String,
    chatgpt_account_id: String,
    token_expires_at: String,
    credentials: StoredOauthCredentials,
    claims: ChatgptJwtClaims,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthProbeOutcome {
    token_expires_at: String,
    credentials: StoredOauthCredentials,
    claims: ChatgptJwtClaims,
    usage_snapshot: Option<NormalizedUsageSnapshot>,
    exhausted: bool,
    usage_snapshot_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthValidatedImportData {
    normalized: NormalizedImportedOauthCredentials,
    probe: ImportedOauthProbeOutcome,
}

pub(crate) struct PersistOauthCallbackInput {
    session: OauthLoginSessionRow,
    display_name: String,
    claims: ChatgptJwtClaims,
    encrypted_credentials: String,
    token_expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtOuterClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<ChatgptJwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<ChatgptJwtAuthClaims>,
}

#[derive(Debug, Deserialize)]
struct JwtExpiryClaims {
    #[serde(default)]
    exp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtAuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}
